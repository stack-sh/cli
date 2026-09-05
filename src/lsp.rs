use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::io::{self, BufRead, BufReader, BufWriter, Read, Write};
use std::sync::OnceLock;

use serde_json::{Map, Value, json};
use stack_compiler::diagnostic::{Diagnostic, Severity, SourcePosition, Span};
use stack_compiler::language_intelligence::{
    self, CompletionCatalog, CompletionCatalogEntry, CompletionItem, CompletionKind,
    DocumentSymbol, DocumentSymbolKind, Hover, IntelligenceError,
};
use stack_engine::Engine;

use crate::{EXIT_STACK_ERROR, EXIT_SUCCESS, EXIT_USAGE_OR_IO};

const MAX_MESSAGE_BYTES: usize = 8 * 1024 * 1024;
const MAX_HEADER_BYTES: usize = 32 * 1024;
const MAX_DOCUMENT_BYTES: usize = 4 * 1024 * 1024;
const MAX_OPEN_DOCUMENTS: usize = 64;
const MAX_REQUEST_IDS: usize = 1_024;
const MAX_URI_CHARS: usize = 4_096;

const PARSE_ERROR: i64 = -32_700;
const INVALID_REQUEST: i64 = -32_600;
const METHOD_NOT_FOUND: i64 = -32_601;
const INVALID_PARAMS: i64 = -32_602;
const INTERNAL_ERROR: i64 = -32_603;
const SERVER_NOT_INITIALIZED: i64 = -32_002;
const REQUEST_CANCELLED: i64 = -32_800;

pub(crate) fn run(stdin: &mut dyn Read, stdout: &mut dyn Write, stderr: &mut dyn Write) -> u8 {
    let mut reader = BufReader::new(stdin);
    let mut writer = BufWriter::new(stdout);
    let mut session = Session::new();

    loop {
        let payload = match read_frame(&mut reader) {
            Ok(Some(payload)) => payload,
            Ok(None) => {
                let _ = writeln!(
                    stderr,
                    "error: LSP input closed before the exit notification"
                );
                return EXIT_USAGE_OR_IO;
            }
            Err(error) => {
                let _ = writeln!(stderr, "error: invalid LSP frame: {error}");
                return EXIT_USAGE_OR_IO;
            }
        };
        let outcome = match serde_json::from_slice::<Value>(&payload) {
            Ok(message) => session.handle(message),
            Err(_) => Outcome::message(error_response(
                Value::Null,
                PARSE_ERROR,
                "invalid JSON payload",
            )),
        };
        for message in outcome.messages {
            if let Err(error) = write_frame(&mut writer, &message) {
                let _ = writeln!(stderr, "error: cannot write LSP response: {error}");
                return EXIT_USAGE_OR_IO;
            }
        }
        if let Err(error) = writer.flush() {
            let _ = writeln!(stderr, "error: cannot flush LSP response: {error}");
            return EXIT_USAGE_OR_IO;
        }
        if let Some(exit_code) = outcome.exit_code {
            return exit_code;
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Lifecycle {
    BeforeInitialize,
    Running,
    Shutdown,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PositionEncoding {
    Utf8,
    Utf16,
    Utf32,
}

impl PositionEncoding {
    const fn name(self) -> &'static str {
        match self {
            Self::Utf8 => "utf-8",
            Self::Utf16 => "utf-16",
            Self::Utf32 => "utf-32",
        }
    }

    fn units(self, text: &str) -> usize {
        match self {
            Self::Utf8 => text.len(),
            Self::Utf16 => text.chars().map(char::len_utf16).sum(),
            Self::Utf32 => text.chars().count(),
        }
    }

    fn byte_offset(self, text: &str, target_units: usize) -> Option<usize> {
        if target_units == 0 {
            return Some(0);
        }
        let mut units = 0_usize;
        for (byte_offset, scalar) in text.char_indices() {
            units += match self {
                Self::Utf8 => scalar.len_utf8(),
                Self::Utf16 => scalar.len_utf16(),
                Self::Utf32 => 1,
            };
            if units == target_units {
                return Some(byte_offset + scalar.len_utf8());
            }
            if units > target_units {
                return None;
            }
        }
        (units == target_units).then_some(text.len())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Document {
    text: String,
    version: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum RequestId {
    Number(i64),
    String(String),
}

impl RequestId {
    fn from_json(value: &Value) -> ProtocolResult<Self> {
        match value {
            Value::Number(number) => number
                .as_i64()
                .map(Self::Number)
                .ok_or_else(|| ProtocolError::invalid_request("request ID must be an integer")),
            Value::String(identifier) if identifier.chars().count() <= 256 => {
                Ok(Self::String(identifier.clone()))
            }
            Value::String(_) => Err(ProtocolError::invalid_request("request ID is too long")),
            _ => Err(ProtocolError::invalid_request(
                "request ID must be an integer or string",
            )),
        }
    }

    fn json(&self) -> Value {
        match self {
            Self::Number(identifier) => json!(identifier),
            Self::String(identifier) => json!(identifier),
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
struct ProtocolError {
    code: i64,
    message: String,
}

impl ProtocolError {
    fn invalid_request(message: &str) -> Self {
        Self {
            code: INVALID_REQUEST,
            message: message.to_owned(),
        }
    }

    fn invalid_params(message: &str) -> Self {
        Self {
            code: INVALID_PARAMS,
            message: message.to_owned(),
        }
    }
}

macro_rules! internal_error {
    ($message:expr) => {
        ProtocolError {
            code: INTERNAL_ERROR,
            message: ($message).into(),
        }
    };
}

type ProtocolResult<T> = Result<T, ProtocolError>;

#[derive(Debug)]
struct Outcome {
    messages: Vec<Value>,
    exit_code: Option<u8>,
}

impl Outcome {
    fn none() -> Self {
        Self {
            messages: Vec::new(),
            exit_code: None,
        }
    }

    fn message(message: Value) -> Self {
        Self {
            messages: vec![message],
            exit_code: None,
        }
    }

    fn exit(exit_code: u8) -> Self {
        Self {
            messages: Vec::new(),
            exit_code: Some(exit_code),
        }
    }
}

struct Session {
    lifecycle: Lifecycle,
    encoding: PositionEncoding,
    documents: BTreeMap<String, Document>,
    cancelled: BTreeSet<RequestId>,
    completed: BTreeSet<RequestId>,
}

impl Session {
    fn new() -> Self {
        Self {
            lifecycle: Lifecycle::BeforeInitialize,
            encoding: PositionEncoding::Utf16,
            documents: BTreeMap::new(),
            cancelled: BTreeSet::new(),
            completed: BTreeSet::new(),
        }
    }

    fn handle(&mut self, message: Value) -> Outcome {
        let Some(object) = message.as_object() else {
            return Outcome::message(error_response(
                Value::Null,
                INVALID_REQUEST,
                "JSON-RPC message must be an object",
            ));
        };
        if object.get("jsonrpc").and_then(Value::as_str) != Some("2.0") {
            return Outcome::message(error_response(
                Value::Null,
                INVALID_REQUEST,
                "jsonrpc must be 2.0",
            ));
        }
        let Some(method) = object.get("method").and_then(Value::as_str) else {
            return Outcome::message(error_response(
                Value::Null,
                INVALID_REQUEST,
                "client message must contain a method",
            ));
        };
        let params = object.get("params").cloned().unwrap_or(Value::Null);
        if let Some(identifier) = object.get("id") {
            let identifier = match RequestId::from_json(identifier) {
                Ok(identifier) => identifier,
                Err(error) => {
                    return Outcome::message(error_response(
                        Value::Null,
                        error.code,
                        &error.message,
                    ));
                }
            };
            self.handle_request(identifier, method, &params)
        } else {
            self.handle_notification(method, &params)
        }
    }

    fn handle_request(&mut self, identifier: RequestId, method: &str, params: &Value) -> Outcome {
        let response_identifier = identifier.json();
        if self.cancelled.remove(&identifier) {
            self.remember_completed(identifier);
            return Outcome::message(error_response(
                response_identifier,
                REQUEST_CANCELLED,
                "request cancelled",
            ));
        }

        let result = match self.lifecycle {
            Lifecycle::BeforeInitialize if method == "initialize" => self.initialize(params),
            Lifecycle::BeforeInitialize => Err(ProtocolError {
                code: SERVER_NOT_INITIALIZED,
                message: "server is not initialized".into(),
            }),
            Lifecycle::Running if method == "initialize" => Err(ProtocolError::invalid_request(
                "initialize may only be sent once",
            )),
            Lifecycle::Running if method == "shutdown" => {
                self.lifecycle = Lifecycle::Shutdown;
                Ok(Value::Null)
            }
            Lifecycle::Running => self.handle_feature_request(method, params),
            Lifecycle::Shutdown => Err(ProtocolError::invalid_request(
                "request received after shutdown",
            )),
        };
        self.remember_completed(identifier);
        Outcome::message(match result {
            Ok(result) => success_response(response_identifier, result),
            Err(error) => error_response(response_identifier, error.code, &error.message),
        })
    }

    fn handle_notification(&mut self, method: &str, params: &Value) -> Outcome {
        if method == "exit" {
            return Outcome::exit(if self.lifecycle == Lifecycle::Shutdown {
                EXIT_SUCCESS
            } else {
                EXIT_STACK_ERROR
            });
        }
        match self.lifecycle {
            Lifecycle::BeforeInitialize | Lifecycle::Shutdown => Outcome::none(),
            Lifecycle::Running if method == "initialized" || method == "$/setTrace" => {
                Outcome::none()
            }
            Lifecycle::Running if method == "$/cancelRequest" => {
                self.cancel_request(params);
                Outcome::none()
            }
            Lifecycle::Running if method == "textDocument/didOpen" => {
                let result = self.did_open(params);
                self.notification_result("didOpen", result)
            }
            Lifecycle::Running if method == "textDocument/didChange" => {
                let result = self.did_change(params);
                self.notification_result("didChange", result)
            }
            Lifecycle::Running if method == "textDocument/didClose" => {
                let result = self.did_close(params);
                self.notification_result("didClose", result)
            }
            Lifecycle::Running => Outcome::none(),
        }
    }

    fn initialize(&mut self, params: &Value) -> ProtocolResult<Value> {
        let params = required_object(params, "initialize params")?;
        self.encoding = negotiate_position_encoding(params);
        self.lifecycle = Lifecycle::Running;
        Ok(json!({
            "capabilities": {
                "positionEncoding": self.encoding.name(),
                "textDocumentSync": {
                    "openClose": true,
                    "change": 2
                },
                "completionProvider": {
                    "resolveProvider": false,
                    "triggerCharacters": [" ", "\"", ":"]
                },
                "hoverProvider": true,
                "documentSymbolProvider": true,
                "documentFormattingProvider": true
            },
            "serverInfo": {
                "name": "stack-lsp",
                "version": env!("CARGO_PKG_VERSION")
            }
        }))
    }

    fn did_open(&mut self, params: &Value) -> ProtocolResult<Value> {
        let document = required_object_field(params, "textDocument")?;
        let uri = validated_uri(required_string(document, "uri")?)?;
        let _language_id = required_string(document, "languageId")?;
        let version = required_version(document)?;
        let text = required_string(document, "text")?;
        validate_document_size(text)?;
        if !self.documents.contains_key(&uri) && self.documents.len() >= MAX_OPEN_DOCUMENTS {
            return Err(ProtocolError::invalid_params(
                "open document limit exceeded",
            ));
        }
        self.documents.insert(
            uri.clone(),
            Document {
                text: text.into(),
                version,
            },
        );
        publish_diagnostics(&uri, text, version, self.encoding)
    }

    fn did_change(&mut self, params: &Value) -> ProtocolResult<Value> {
        let identifier = required_object_field(params, "textDocument")?;
        let uri = validated_uri(required_string(identifier, "uri")?)?;
        let version = required_version(identifier)?;
        let changes = required_array(params, "contentChanges")?;
        let Some(current) = self.documents.get(&uri) else {
            return Err(ProtocolError::invalid_params("document is not open"));
        };
        if version <= current.version {
            return Err(ProtocolError::invalid_params(
                "document version must increase",
            ));
        }
        let mut updated = current.text.clone();
        for change in changes {
            apply_content_change(&mut updated, change, self.encoding)?;
        }
        validate_document_size(&updated)?;
        self.documents.insert(
            uri.clone(),
            Document {
                text: updated.clone(),
                version,
            },
        );
        publish_diagnostics(&uri, &updated, version, self.encoding)
    }

    fn did_close(&mut self, params: &Value) -> ProtocolResult<Value> {
        let identifier = required_object_field(params, "textDocument")?;
        let uri = validated_uri(required_string(identifier, "uri")?)?;
        let Some(document) = self.documents.remove(&uri) else {
            return Err(ProtocolError::invalid_params("document is not open"));
        };
        let version = document.version as i64;
        Ok(json!({
            "jsonrpc": "2.0",
            "method": "textDocument/publishDiagnostics",
            "params": {
                "uri": uri,
                "version": version,
                "diagnostics": []
            }
        }))
    }

    fn handle_feature_request(&self, method: &str, params: &Value) -> ProtocolResult<Value> {
        match method {
            "textDocument/completion" => self.completion(params),
            "textDocument/hover" => self.hover(params),
            "textDocument/documentSymbol" => self.document_symbols(params),
            "textDocument/formatting" => self.format_document(params),
            _ => Err(ProtocolError {
                code: METHOD_NOT_FOUND,
                message: format!("unsupported method: {method}"),
            }),
        }
    }

    fn completion(&self, params: &Value) -> ProtocolResult<Value> {
        let document = self.request_document(params)?;
        let position = compiler_position_from_lsp(
            &document.text,
            required_object_field(params, "position")?,
            self.encoding,
        )?;
        let output = match language_intelligence::completion(
            &document.text,
            document.version,
            position,
            completion_catalog(),
        ) {
            Ok(output) => output,
            Err(IntelligenceError::InvalidPosition) => {
                return Err(ProtocolError::invalid_params(
                    "position is outside the document",
                ));
            }
            Err(error) => {
                return Err(internal_error!(format!(
                    "completion catalog is invalid: {error}"
                )));
            }
        };
        let items = output
            .items
            .iter()
            .map(|item| completion_item_json(&document.text, item, self.encoding))
            .collect::<ProtocolResult<Vec<_>>>()?;
        Ok(json!({
            "isIncomplete": output.is_incomplete,
            "items": items
        }))
    }

    fn hover(&self, params: &Value) -> ProtocolResult<Value> {
        let document = self.request_document(params)?;
        let position = compiler_position_from_lsp(
            &document.text,
            required_object_field(params, "position")?,
            self.encoding,
        )?;
        let output = match language_intelligence::hover(&document.text, document.version, position)
        {
            Ok(output) => output,
            Err(IntelligenceError::InvalidPosition) => {
                return Err(ProtocolError::invalid_params(
                    "position is outside the document",
                ));
            }
            Err(error) => {
                return Err(internal_error!(format!(
                    "language intelligence failed: {error}"
                )));
            }
        };
        output
            .hover
            .as_ref()
            .map(|hover| hover_json(&document.text, hover, self.encoding))
            .transpose()
            .map(|hover| hover.unwrap_or(Value::Null))
    }

    fn document_symbols(&self, params: &Value) -> ProtocolResult<Value> {
        let document = self.request_document(params)?;
        let output = language_intelligence::document_symbols(&document.text, document.version);
        let symbols = output
            .symbols
            .iter()
            .map(|symbol| document_symbol_json(&document.text, symbol, self.encoding))
            .collect::<ProtocolResult<Vec<_>>>()?;
        Ok(Value::Array(symbols))
    }

    fn format_document(&self, params: &Value) -> ProtocolResult<Value> {
        let _options = required_object_field(params, "options")?;
        let document = self.request_document(params)?;
        let output = match Engine::bundled().format(document.text.as_bytes()) {
            Ok(output) => output,
            Err(error) => return Err(internal_error!(format!("formatter failed: {error}"))),
        };
        let Some(formatted) = output.formatted_source else {
            return Ok(Value::Array(Vec::new()));
        };
        if formatted == document.text {
            return Ok(Value::Array(Vec::new()));
        }
        let whole_document = Span {
            start: SourcePosition::start(),
            end: source_end_position(&document.text),
        };
        Ok(json!([{
            "range": lsp_range_json(&document.text, whole_document, self.encoding)?,
            "newText": formatted
        }]))
    }

    fn request_document(&self, params: &Value) -> ProtocolResult<&Document> {
        let identifier = required_object_field(params, "textDocument")?;
        let uri = validated_uri(required_string(identifier, "uri")?)?;
        self.documents
            .get(&uri)
            .ok_or_else(|| ProtocolError::invalid_params("document is not open"))
    }

    fn notification_result(&self, name: &str, result: ProtocolResult<Value>) -> Outcome {
        match result {
            Ok(notification) => Outcome::message(notification),
            Err(error) => Outcome::message(log_notification(
                1,
                &format!("ignored {name}: {}", error.message),
            )),
        }
    }

    fn cancel_request(&mut self, params: &Value) {
        let Some(identifier) = params
            .as_object()
            .and_then(|object| object.get("id"))
            .and_then(|value| RequestId::from_json(value).ok())
        else {
            return;
        };
        if self.completed.contains(&identifier) {
            return;
        }
        if self.cancelled.len() >= MAX_REQUEST_IDS {
            self.cancelled.clear();
        }
        self.cancelled.insert(identifier);
    }

    fn remember_completed(&mut self, identifier: RequestId) {
        if self.completed.len() >= MAX_REQUEST_IDS {
            self.completed.clear();
        }
        self.completed.insert(identifier);
    }
}

fn negotiate_position_encoding(params: &Map<String, Value>) -> PositionEncoding {
    let offered = params
        .get("capabilities")
        .and_then(Value::as_object)
        .and_then(|capabilities| capabilities.get("general"))
        .and_then(Value::as_object)
        .and_then(|general| general.get("positionEncodings"))
        .and_then(Value::as_array);
    if let Some(items) = offered {
        for item in items {
            match item.as_str() {
                Some("utf-8") => return PositionEncoding::Utf8,
                Some("utf-16") => return PositionEncoding::Utf16,
                Some("utf-32") => return PositionEncoding::Utf32,
                _ => {}
            }
        }
    }
    PositionEncoding::Utf16
}

fn apply_content_change(
    source: &mut String,
    change: &Value,
    encoding: PositionEncoding,
) -> ProtocolResult<()> {
    let change = required_object(change, "content change")?;
    let replacement = required_string(change, "text")?;
    let Some(range) = change.get("range") else {
        validate_document_size(replacement)?;
        source.clear();
        source.push_str(replacement);
        return Ok(());
    };
    let range = required_object(range, "change range")?;
    let start =
        lsp_position_to_offset(source, required_map_object_field(range, "start")?, encoding)?;
    let end = lsp_position_to_offset(source, required_map_object_field(range, "end")?, encoding)?;
    if start > end {
        return Err(ProtocolError::invalid_params(
            "change range start is after its end",
        ));
    }
    let replaced_bytes = end - start;
    let Some(updated_size) = source
        .len()
        .checked_sub(replaced_bytes)
        .and_then(|size| size.checked_add(replacement.len()))
    else {
        return Err(ProtocolError::invalid_params(
            "changed document size overflowed",
        ));
    };
    if updated_size > MAX_DOCUMENT_BYTES {
        return Err(ProtocolError::invalid_params("document is too large"));
    }
    source.replace_range(start..end, replacement);
    Ok(())
}

fn lsp_position_to_offset(
    source: &str,
    position: &Map<String, Value>,
    encoding: PositionEncoding,
) -> ProtocolResult<usize> {
    let line = required_usize(position, "line")?;
    let character = required_usize(position, "character")?;
    let Some((line_start, line_end)) = line_bounds(source, line) else {
        return Err(ProtocolError::invalid_params(
            "position line is outside the document",
        ));
    };
    let Some(line_text) = source.get(line_start..line_end) else {
        return Err(internal_error!("document line is not valid UTF-8"));
    };
    let Some(relative) = encoding.byte_offset(line_text, character) else {
        return Err(ProtocolError::invalid_params(
            "position character is outside a scalar boundary",
        ));
    };
    Ok(line_start + relative)
}

fn compiler_position_from_lsp(
    source: &str,
    position: &Map<String, Value>,
    encoding: PositionEncoding,
) -> ProtocolResult<SourcePosition> {
    let line = required_usize(position, "line")?;
    let byte_offset = lsp_position_to_offset(source, position, encoding)?;
    let Some((line_start, _)) = line_bounds(source, line) else {
        return Err(ProtocolError::invalid_params(
            "position line is outside the document",
        ));
    };
    let Some(prefix) = source.get(line_start..byte_offset) else {
        return Err(internal_error!("position is not a UTF-8 boundary"));
    };
    Ok(SourcePosition {
        byte_offset,
        line: line + 1,
        column: prefix.chars().count() + 1,
    })
}

fn line_bounds(source: &str, requested_line: usize) -> Option<(usize, usize)> {
    let mut line = 0_usize;
    let mut start = 0_usize;
    let bytes = source.as_bytes();
    let mut index = 0_usize;
    while index < bytes.len() {
        let separator_bytes = match bytes[index] {
            b'\r' if bytes.get(index + 1) == Some(&b'\n') => 2,
            b'\r' | b'\n' => 1,
            _ => {
                index += 1;
                continue;
            }
        };
        if line == requested_line {
            return Some((start, index));
        }
        line += 1;
        index += separator_bytes;
        start = index;
    }
    (line == requested_line).then_some((start, source.len()))
}

fn lsp_range_json(source: &str, span: Span, encoding: PositionEncoding) -> ProtocolResult<Value> {
    Ok(json!({
        "start": lsp_position_json(source, span.start, encoding)?,
        "end": lsp_position_json(source, span.end, encoding)?,
    }))
}

fn lsp_position_json(
    source: &str,
    position: SourcePosition,
    encoding: PositionEncoding,
) -> ProtocolResult<Value> {
    let Some(line) = position.line.checked_sub(1) else {
        return Err(internal_error!(
            "compiler returned a zero-based source line"
        ));
    };
    let Some((line_start, line_end)) = line_bounds(source, line) else {
        return Err(internal_error!(
            "compiler position line is outside the source"
        ));
    };
    if position.byte_offset < line_start || position.byte_offset > line_end {
        return Err(internal_error!(
            "compiler position byte offset is outside its line"
        ));
    }
    let Some(prefix) = source.get(line_start..position.byte_offset) else {
        return Err(internal_error!("compiler position is not a UTF-8 boundary"));
    };
    Ok(json!({
        "line": line,
        "character": encoding.units(prefix),
    }))
}

fn publish_diagnostics(
    uri: &str,
    source: &str,
    version: u64,
    encoding: PositionEncoding,
) -> ProtocolResult<Value> {
    let output = language_intelligence::diagnostics(source, version);
    let diagnostics = output
        .diagnostics
        .iter()
        .map(|diagnostic| diagnostic_json(uri, source, diagnostic, encoding))
        .collect::<ProtocolResult<Vec<_>>>()?;
    let version = output.document_version as i64;
    Ok(json!({
        "jsonrpc": "2.0",
        "method": "textDocument/publishDiagnostics",
        "params": {
            "uri": uri,
            "version": version,
            "diagnostics": diagnostics
        }
    }))
}

fn completion_catalog() -> &'static CompletionCatalog {
    static CATALOG: OnceLock<CompletionCatalog> = OnceLock::new();
    CATALOG.get_or_init(|| {
        let mut icons = BTreeMap::new();
        for theme in &stack_theme::catalog().themes {
            for icon in &theme.icons {
                icons
                    .entry(icon.id.clone())
                    .or_insert_with(|| CompletionCatalogEntry {
                        id: icon.id.clone(),
                        label: icon.subject.clone(),
                        detail: Some("Stack core icon".into()),
                        documentation: icon.description.clone(),
                    });
            }
        }
        CompletionCatalog {
            icons: icons.into_values().collect(),
        }
    })
}

fn completion_item_json(
    source: &str,
    item: &CompletionItem,
    encoding: PositionEncoding,
) -> ProtocolResult<Value> {
    let mut value = Map::new();
    value.insert("label".into(), json!(item.label));
    value.insert(
        "kind".into(),
        json!(match item.kind {
            CompletionKind::Keyword => 14,
            CompletionKind::Property => 10,
            CompletionKind::EnumValue => 20,
            CompletionKind::Identifier => 18,
            CompletionKind::Icon => 12,
        }),
    );
    if let Some(detail) = &item.detail {
        value.insert("detail".into(), json!(detail));
    }
    if let Some(documentation) = &item.documentation {
        value.insert("documentation".into(), json!(documentation));
    }
    value.insert("filterText".into(), json!(item.filter_text));
    value.insert("sortText".into(), json!(item.sort_text));
    value.insert("insertTextFormat".into(), json!(1));
    value.insert(
        "textEdit".into(),
        json!({
            "range": lsp_range_json(source, item.edit.range, encoding)?,
            "newText": item.edit.new_text
        }),
    );
    Ok(Value::Object(value))
}

fn hover_json(source: &str, hover: &Hover, encoding: PositionEncoding) -> ProtocolResult<Value> {
    let mut sections = vec![hover.label.clone()];
    if let Some(detail) = &hover.detail {
        sections.push(detail.clone());
    }
    if let Some(documentation) = &hover.documentation {
        sections.push(documentation.clone());
    }
    Ok(json!({
        "contents": {
            "kind": "plaintext",
            "value": sections.join("\n\n")
        },
        "range": lsp_range_json(source, hover.range, encoding)?
    }))
}

fn document_symbol_json(
    source: &str,
    symbol: &DocumentSymbol,
    encoding: PositionEncoding,
) -> ProtocolResult<Value> {
    let children = symbol
        .children
        .iter()
        .map(|child| document_symbol_json(source, child, encoding))
        .collect::<ProtocolResult<Vec<_>>>()?;
    let mut value = Map::new();
    value.insert("name".into(), json!(symbol.name));
    if let Some(detail) = &symbol.detail {
        value.insert("detail".into(), json!(detail));
    }
    value.insert(
        "kind".into(),
        json!(match symbol.kind {
            DocumentSymbolKind::Diagram => 2,
            DocumentSymbolKind::Group => 3,
            DocumentSymbolKind::Node => 19,
            DocumentSymbolKind::Edge => 25,
        }),
    );
    value.insert(
        "range".into(),
        lsp_range_json(source, symbol.range, encoding)?,
    );
    value.insert(
        "selectionRange".into(),
        lsp_range_json(source, symbol.selection_range, encoding)?,
    );
    value.insert("children".into(), Value::Array(children));
    Ok(Value::Object(value))
}

fn source_end_position(source: &str) -> SourcePosition {
    let bytes = source.as_bytes();
    let mut line = 1_usize;
    let mut line_start = 0_usize;
    let mut index = 0_usize;
    while index < bytes.len() {
        let separator_bytes = match bytes[index] {
            b'\r' if bytes.get(index + 1) == Some(&b'\n') => 2,
            b'\r' | b'\n' => 1,
            _ => {
                index += 1;
                continue;
            }
        };
        line += 1;
        index += separator_bytes;
        line_start = index;
    }
    SourcePosition {
        byte_offset: source.len(),
        line,
        column: source[line_start..].chars().count() + 1,
    }
}

fn diagnostic_json(
    uri: &str,
    source: &str,
    diagnostic: &Diagnostic,
    encoding: PositionEncoding,
) -> ProtocolResult<Value> {
    let mut related = Vec::new();
    for information in &diagnostic.related {
        related.push(json!({
            "location": {
                "uri": uri,
                "range": lsp_range_json(source, information.span, encoding)?
            },
            "message": information.message
        }));
    }
    Ok(json!({
        "range": lsp_range_json(source, diagnostic.span, encoding)?,
        "severity": match diagnostic.severity {
            Severity::Error => 1,
            Severity::Warning => 2,
        },
        "code": diagnostic.code,
        "source": "stack",
        "message": diagnostic.message,
        "relatedInformation": related,
        "data": {
            "schemaVersion": language_intelligence::SCHEMA_VERSION,
            "expected": diagnostic.expected,
            "help": diagnostic.help
        }
    }))
}

fn validate_document_size(source: &str) -> ProtocolResult<()> {
    if source.len() > MAX_DOCUMENT_BYTES {
        Err(ProtocolError::invalid_params("document is too large"))
    } else {
        Ok(())
    }
}

fn validated_uri(uri: &str) -> ProtocolResult<String> {
    let length = uri.chars().count();
    if (1..=MAX_URI_CHARS).contains(&length) {
        Ok(uri.into())
    } else {
        Err(ProtocolError::invalid_params("document URI is invalid"))
    }
}

fn required_object<'value>(
    value: &'value Value,
    name: &str,
) -> ProtocolResult<&'value Map<String, Value>> {
    match value.as_object() {
        Some(object) => Ok(object),
        None => Err(ProtocolError::invalid_params(&format!(
            "{name} must be an object"
        ))),
    }
}

fn required_object_field<'value>(
    value: &'value Value,
    field: &str,
) -> ProtocolResult<&'value Map<String, Value>> {
    match required_object(value, "params")?
        .get(field)
        .and_then(Value::as_object)
    {
        Some(object) => Ok(object),
        None => Err(ProtocolError::invalid_params(&format!(
            "{field} must be an object"
        ))),
    }
}

fn required_map_object_field<'value>(
    value: &'value Map<String, Value>,
    field: &str,
) -> ProtocolResult<&'value Map<String, Value>> {
    match value.get(field).and_then(Value::as_object) {
        Some(object) => Ok(object),
        None => Err(ProtocolError::invalid_params(&format!(
            "{field} must be an object"
        ))),
    }
}

fn required_array<'value>(value: &'value Value, field: &str) -> ProtocolResult<&'value Vec<Value>> {
    match required_object(value, "params")?
        .get(field)
        .and_then(Value::as_array)
    {
        Some(items) => Ok(items),
        None => Err(ProtocolError::invalid_params(&format!(
            "{field} must be an array"
        ))),
    }
}

fn required_string<'value>(
    value: &'value Map<String, Value>,
    field: &str,
) -> ProtocolResult<&'value str> {
    match value.get(field).and_then(Value::as_str) {
        Some(text) => Ok(text),
        None => Err(ProtocolError::invalid_params(&format!(
            "{field} must be a string"
        ))),
    }
}

fn required_usize(value: &Map<String, Value>, field: &str) -> ProtocolResult<usize> {
    let Some(number) = value.get(field).and_then(Value::as_u64) else {
        return Err(ProtocolError::invalid_params(&format!(
            "{field} must be an unsigned integer"
        )));
    };
    match usize::try_from(number) {
        Ok(number) => Ok(number),
        Err(_) => Err(ProtocolError::invalid_params(&format!(
            "{field} must fit the platform index range"
        ))),
    }
}

fn required_version(value: &Map<String, Value>) -> ProtocolResult<u64> {
    let version = value
        .get("version")
        .and_then(Value::as_i64)
        .filter(|version| *version >= 0 && *version <= i64::from(i32::MAX));
    match version {
        Some(version) => Ok(version as u64),
        None => Err(ProtocolError::invalid_params(
            "version must be a non-negative LSP integer",
        )),
    }
}

fn success_response(identifier: Value, result: Value) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": identifier,
        "result": result
    })
}

fn error_response(identifier: Value, code: i64, message: &str) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": identifier,
        "error": {
            "code": code,
            "message": message
        }
    })
}

fn log_notification(message_type: u8, message: &str) -> Value {
    json!({
        "jsonrpc": "2.0",
        "method": "window/logMessage",
        "params": {
            "type": message_type,
            "message": message
        }
    })
}

#[derive(Debug, PartialEq, Eq)]
enum FrameError {
    Io(io::ErrorKind),
    HeaderTooLarge,
    InvalidHeader,
    MissingContentLength,
    DuplicateContentLength,
    InvalidContentLength,
    MessageTooLarge,
    UnsupportedEncoding,
    TruncatedBody,
}

impl fmt::Display for FrameError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let message = match self {
            Self::Io(_) => "I/O failure",
            Self::HeaderTooLarge => "header is too large",
            Self::InvalidHeader => "header is invalid",
            Self::MissingContentLength => "Content-Length is missing",
            Self::DuplicateContentLength => "Content-Length is duplicated",
            Self::InvalidContentLength => "Content-Length is invalid",
            Self::MessageTooLarge => "message exceeds the size limit",
            Self::UnsupportedEncoding => "message charset is not UTF-8",
            Self::TruncatedBody => "message body is truncated",
        };
        formatter.write_str(message)
    }
}

fn read_frame(reader: &mut dyn BufRead) -> Result<Option<Vec<u8>>, FrameError> {
    let mut content_length = None;
    let mut total_header_bytes = 0_usize;
    loop {
        let mut line = Vec::new();
        let read = match reader.read_until(b'\n', &mut line) {
            Ok(read) => read,
            Err(error) => return Err(FrameError::Io(error.kind())),
        };
        if read == 0 {
            return if total_header_bytes == 0 {
                Ok(None)
            } else {
                Err(FrameError::InvalidHeader)
            };
        }
        total_header_bytes = total_header_bytes
            .checked_add(read)
            .ok_or(FrameError::HeaderTooLarge)?;
        if total_header_bytes > MAX_HEADER_BYTES {
            return Err(FrameError::HeaderTooLarge);
        }
        if line == b"\r\n" || line == b"\n" {
            break;
        }
        if !line.ends_with(b"\n") || !line.is_ascii() {
            return Err(FrameError::InvalidHeader);
        }
        let line = match std::str::from_utf8(&line) {
            Ok(line) => line,
            Err(_) => return Err(FrameError::InvalidHeader),
        };
        let line = line.trim_end_matches(['\r', '\n']);
        let Some((name, value)) = line.split_once(':') else {
            return Err(FrameError::InvalidHeader);
        };
        let value = value.trim();
        if name.eq_ignore_ascii_case("Content-Length") {
            if content_length.is_some() {
                return Err(FrameError::DuplicateContentLength);
            }
            let length = match value.parse::<usize>() {
                Ok(length) => length,
                Err(_) => return Err(FrameError::InvalidContentLength),
            };
            if length > MAX_MESSAGE_BYTES {
                return Err(FrameError::MessageTooLarge);
            }
            content_length = Some(length);
        } else if name.eq_ignore_ascii_case("Content-Type") {
            let normalized = value.to_ascii_lowercase();
            if normalized.contains("charset=")
                && !normalized.contains("charset=utf-8")
                && !normalized.contains("charset=utf8")
            {
                return Err(FrameError::UnsupportedEncoding);
            }
        }
    }
    let length = content_length.ok_or(FrameError::MissingContentLength)?;
    let mut payload = vec![0_u8; length];
    if let Err(error) = reader.read_exact(&mut payload) {
        return if error.kind() == io::ErrorKind::UnexpectedEof {
            Err(FrameError::TruncatedBody)
        } else {
            Err(FrameError::Io(error.kind()))
        };
    }
    Ok(Some(payload))
}

fn write_frame(writer: &mut dyn Write, message: &Value) -> io::Result<()> {
    let payload = serde_json::to_vec(message).map_err(io::Error::other)?;
    write!(writer, "Content-Length: {}\r\n\r\n", payload.len())?;
    writer.write_all(&payload)
}

#[cfg(test)]
mod tests {
    use std::io::{BufReader, Cursor};

    use serde_json::{Value, json};

    use super::{
        Document, FrameError, Lifecycle, MAX_DOCUMENT_BYTES, MAX_MESSAGE_BYTES, MAX_OPEN_DOCUMENTS,
        PositionEncoding, Session, apply_content_change, compiler_position_from_lsp, line_bounds,
        lsp_position_json, read_frame, run, source_end_position, validate_document_size,
        write_frame,
    };
    use crate::{EXIT_STACK_ERROR, EXIT_SUCCESS, EXIT_USAGE_OR_IO};
    use stack_compiler::diagnostic::SourcePosition;

    fn initialized_session(source: &str) -> Session {
        let mut session = Session::new();
        let initialized = session.handle(json!({
            "jsonrpc": "2.0",
            "id": "initialize",
            "method": "initialize",
            "params": {
                "capabilities": { "general": { "positionEncodings": ["utf-16"] } }
            }
        }));
        assert_eq!(initialized.messages[0]["id"], "initialize");
        let opened = session.handle(json!({
            "jsonrpc": "2.0",
            "method": "textDocument/didOpen",
            "params": {
                "textDocument": {
                    "uri": "file:///workspace/diagram.stack",
                    "languageId": "stack",
                    "version": 7,
                    "text": source
                }
            }
        }));
        assert_eq!(
            opened.messages[0]["method"],
            "textDocument/publishDiagnostics"
        );
        session
    }

    #[test]
    fn position_encodings_reject_split_scalars_and_round_trip_compiler_positions()
    -> Result<(), &'static str> {
        let source = "a😀\r\nβz";
        let utf8 = json!({ "line": 0, "character": 5 });
        let utf16 = json!({ "line": 0, "character": 3 });
        let utf32 = json!({ "line": 0, "character": 2 });
        for (position, encoding) in [
            (&utf8, PositionEncoding::Utf8),
            (&utf16, PositionEncoding::Utf16),
            (&utf32, PositionEncoding::Utf32),
        ] {
            let object = position
                .as_object()
                .ok_or("test position must be an object")?;
            let converted = compiler_position_from_lsp(source, object, encoding);
            assert_eq!(
                converted,
                Ok(SourcePosition {
                    byte_offset: 5,
                    line: 1,
                    column: 3,
                })
            );
            let encoded = lsp_position_json(
                source,
                SourcePosition {
                    byte_offset: 5,
                    line: 1,
                    column: 3,
                },
                encoding,
            );
            assert_eq!(encoded, Ok(position.clone()));
        }

        for (character, encoding) in [
            (2, PositionEncoding::Utf8),
            (2, PositionEncoding::Utf16),
            (3, PositionEncoding::Utf32),
        ] {
            let invalid = json!({ "line": 0, "character": character });
            let object = invalid
                .as_object()
                .ok_or("test position must be an object")?;
            assert!(compiler_position_from_lsp(source, object, encoding).is_err());
        }
        assert_eq!(line_bounds(source, 1), Some((7, 10)));
        assert_eq!(line_bounds(source, 2), None);
        assert_eq!(line_bounds("a\rb", 1), Some((2, 3)));
        assert_eq!(line_bounds("a\r\nb", 1), Some((3, 4)));
        assert_eq!(
            source_end_position("a\rb\r\nc\n"),
            SourcePosition {
                byte_offset: 7,
                line: 4,
                column: 1,
            }
        );
        Ok(())
    }

    #[test]
    fn incremental_changes_apply_in_order_and_atomically_reject_bad_ranges() {
        let mut source = "a😀\r\nβz".to_owned();
        let replace_emoji = json!({
            "range": {
                "start": { "line": 0, "character": 1 },
                "end": { "line": 0, "character": 3 }
            },
            "text": "X"
        });
        assert_eq!(
            apply_content_change(&mut source, &replace_emoji, PositionEncoding::Utf16),
            Ok(())
        );
        let replace_second_line = json!({
            "range": {
                "start": { "line": 1, "character": 0 },
                "end": { "line": 1, "character": 1 }
            },
            "text": "B"
        });
        assert_eq!(
            apply_content_change(&mut source, &replace_second_line, PositionEncoding::Utf16),
            Ok(())
        );
        assert_eq!(source, "aX\r\nBz");

        let original = source.clone();
        let backwards = json!({
            "range": {
                "start": { "line": 1, "character": 2 },
                "end": { "line": 0, "character": 0 }
            },
            "text": "ignored"
        });
        assert!(apply_content_change(&mut source, &backwards, PositionEncoding::Utf16).is_err());
        assert_eq!(source, original);

        let whole = json!({ "text": "replacement" });
        assert_eq!(
            apply_content_change(&mut source, &whole, PositionEncoding::Utf8),
            Ok(())
        );
        assert_eq!(source, "replacement");
    }

    #[test]
    fn lifecycle_negotiates_encoding_tracks_versions_and_clears_diagnostics() {
        let mut session = Session::new();
        let before = session.handle(json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "shutdown"
        }));
        assert_eq!(before.messages[0]["error"]["code"], -32_002);

        let initialized = session.handle(json!({
            "jsonrpc": "2.0",
            "id": "init",
            "method": "initialize",
            "params": {
                "capabilities": { "general": { "positionEncodings": ["utf-8", "utf-16"] } }
            }
        }));
        assert_eq!(session.lifecycle, Lifecycle::Running);
        assert_eq!(session.encoding, PositionEncoding::Utf8);
        assert_eq!(
            initialized.messages[0]["result"]["capabilities"]["positionEncoding"],
            "utf-8"
        );

        let opened = session.handle(json!({
            "jsonrpc": "2.0",
            "method": "textDocument/didOpen",
            "params": {
                "textDocument": {
                    "uri": "file:///diagram.stack",
                    "languageId": "stack",
                    "version": 1,
                    "text": "stack 1.0 diagram \"D\" { node api }"
                }
            }
        }));
        assert_eq!(
            opened.messages[0]["method"],
            "textDocument/publishDiagnostics"
        );
        assert_eq!(opened.messages[0]["params"]["version"], 1);
        assert!(opened.messages[0]["params"]["diagnostics"].is_array());

        let changed = session.handle(json!({
            "jsonrpc": "2.0",
            "method": "textDocument/didChange",
            "params": {
                "textDocument": { "uri": "file:///diagram.stack", "version": 2 },
                "contentChanges": [{ "text": "stack 1.0 diagram \"D\" { node api \"API\" }" }]
            }
        }));
        assert_eq!(changed.messages[0]["params"]["version"], 2);
        assert_eq!(changed.messages[0]["params"]["diagnostics"], json!([]));

        let stale = session.handle(json!({
            "jsonrpc": "2.0",
            "method": "textDocument/didChange",
            "params": {
                "textDocument": { "uri": "file:///diagram.stack", "version": 2 },
                "contentChanges": [{ "text": "ignored" }]
            }
        }));
        assert_eq!(stale.messages[0]["method"], "window/logMessage");

        let closed = session.handle(json!({
            "jsonrpc": "2.0",
            "method": "textDocument/didClose",
            "params": { "textDocument": { "uri": "file:///diagram.stack" } }
        }));
        assert_eq!(closed.messages[0]["params"]["diagnostics"], json!([]));
        assert!(session.documents.is_empty());

        let shutdown = session.handle(json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "shutdown"
        }));
        assert_eq!(shutdown.messages[0]["result"], Value::Null);
        let exit = session.handle(json!({ "jsonrpc": "2.0", "method": "exit" }));
        assert_eq!(exit.exit_code, Some(EXIT_SUCCESS));

        let mut preferred = Session::new();
        let initialized = preferred.handle(json!({
            "jsonrpc": "2.0",
            "id": 3,
            "method": "initialize",
            "params": {
                "capabilities": { "general": { "positionEncodings": ["utf-32", "utf-16"] } }
            }
        }));
        assert_eq!(preferred.encoding, PositionEncoding::Utf32);
        assert_eq!(
            initialized.messages[0]["result"]["capabilities"]["positionEncoding"],
            "utf-32"
        );
    }

    #[test]
    fn core_requests_map_compiler_semantics_and_formatter_edits() {
        let source = concat!(
            "stack 1.0\n\n",
            "diagram \"😀 Checkout\" {\n",
            "  node api \"API\" {\n",
            "    kind service\n",
            "    icon \"ser\"\n",
            "  }\n",
            "  node db \"Database\" { kind database }\n",
            "  edge api -> db\n",
            "}\n",
        );
        let mut session = initialized_session(source);

        let completion = session.handle(json!({
            "jsonrpc": "2.0",
            "id": 20,
            "method": "textDocument/completion",
            "params": {
                "textDocument": { "uri": "file:///workspace/diagram.stack" },
                "position": { "line": 5, "character": 13 }
            }
        }));
        assert_eq!(completion.messages[0]["id"], 20);
        let items = completion.messages[0]["result"]["items"].as_array();
        assert!(
            items.is_some_and(|items| items.iter().any(|item| {
                item["filterText"] == "server"
                    && item["label"] == "Server host"
                    && item["kind"] == 12
                    && item["textEdit"]["newText"] == "server"
            })),
            "{:?}",
            completion.messages[0]
        );

        let hover = session.handle(json!({
            "jsonrpc": "2.0",
            "id": 21,
            "method": "textDocument/hover",
            "params": {
                "textDocument": { "uri": "file:///workspace/diagram.stack" },
                "position": { "line": 8, "character": 8 }
            }
        }));
        assert_eq!(hover.messages[0]["result"]["contents"]["kind"], "plaintext");
        assert!(
            hover.messages[0]["result"]["contents"]["value"]
                .as_str()
                .is_some_and(|value| value.contains("API") && value.contains("node api"))
        );
        assert_eq!(hover.messages[0]["result"]["range"]["start"]["line"], 8);

        let symbols = session.handle(json!({
            "jsonrpc": "2.0",
            "id": 22,
            "method": "textDocument/documentSymbol",
            "params": {
                "textDocument": { "uri": "file:///workspace/diagram.stack" }
            }
        }));
        assert_eq!(symbols.messages[0]["result"][0]["name"], "😀 Checkout");
        assert_eq!(symbols.messages[0]["result"][0]["kind"], 2);
        assert_eq!(
            symbols.messages[0]["result"][0]["children"][0]["name"],
            "API"
        );
        assert_eq!(symbols.messages[0]["result"][0]["children"][2]["kind"], 25);

        let formatting = session.handle(json!({
            "jsonrpc": "2.0",
            "id": 23,
            "method": "textDocument/formatting",
            "params": {
                "textDocument": { "uri": "file:///workspace/diagram.stack" },
                "options": { "tabSize": 2, "insertSpaces": true }
            }
        }));
        assert_eq!(
            formatting.messages[0]["result"][0]["range"]["start"]["line"],
            0
        );
        assert_eq!(
            formatting.messages[0]["result"][0]["range"]["end"]["line"],
            10
        );
        assert!(
            formatting.messages[0]["result"][0]["newText"]
                .as_str()
                .is_some_and(|text| text.contains("diagram \"😀 Checkout\""))
        );
    }

    #[test]
    fn diagnostic_ranges_match_utf16_editor_coordinates() {
        let source = "stack 1.0 diagram \"😀\" { node api \"API\" { kind nope } }";
        let mut session = Session::new();
        let _initialized = session.handle(json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": { "capabilities": {} }
        }));
        let opened = session.handle(json!({
            "jsonrpc": "2.0",
            "method": "textDocument/didOpen",
            "params": {
                "textDocument": {
                    "uri": "file:///workspace/range.stack",
                    "languageId": "stack",
                    "version": 1,
                    "text": source
                }
            }
        }));
        let diagnostic = &opened.messages[0]["params"]["diagnostics"][0];
        assert_eq!(diagnostic["code"], "STK2002");
        assert_eq!(
            diagnostic["range"]["start"],
            json!({ "line": 0, "character": 47 })
        );
        assert_eq!(
            diagnostic["range"]["end"],
            json!({ "line": 0, "character": 51 })
        );
    }

    #[test]
    fn invalid_feature_requests_return_empty_or_actionable_results() {
        let mut session = initialized_session("stack 1.0 diagram \"Partial\" { node api");
        let hover = session.handle(json!({
            "jsonrpc": "2.0",
            "id": 30,
            "method": "textDocument/hover",
            "params": {
                "textDocument": { "uri": "file:///workspace/diagram.stack" },
                "position": { "line": 0, "character": 42 }
            }
        }));
        assert_eq!(hover.messages[0]["result"], Value::Null);

        let symbols = session.handle(json!({
            "jsonrpc": "2.0",
            "id": 31,
            "method": "textDocument/documentSymbol",
            "params": {
                "textDocument": { "uri": "file:///workspace/diagram.stack" }
            }
        }));
        assert_eq!(symbols.messages[0]["result"], json!([]));

        let formatting = session.handle(json!({
            "jsonrpc": "2.0",
            "id": 32,
            "method": "textDocument/formatting",
            "params": {
                "textDocument": { "uri": "file:///workspace/diagram.stack" },
                "options": { "tabSize": 2, "insertSpaces": true }
            }
        }));
        assert_eq!(formatting.messages[0]["result"], json!([]));

        for (identifier, params) in [
            (
                33,
                json!({
                    "textDocument": { "uri": "file:///workspace/missing.stack" },
                    "position": { "line": 0, "character": 0 }
                }),
            ),
            (
                34,
                json!({
                    "textDocument": { "uri": "file:///workspace/diagram.stack" },
                    "position": { "line": 99, "character": 0 }
                }),
            ),
        ] {
            let response = session.handle(json!({
                "jsonrpc": "2.0",
                "id": identifier,
                "method": "textDocument/hover",
                "params": params
            }));
            assert_eq!(response.messages[0]["error"]["code"], -32_602);
        }
    }

    #[test]
    fn cancellation_and_invalid_messages_return_protocol_errors() {
        let mut session = Session::new();
        assert_eq!(
            session
                .handle(json!({ "jsonrpc": "2.0", "method": "exit" }))
                .exit_code,
            Some(EXIT_STACK_ERROR)
        );
        for message in [
            json!([]),
            json!({ "method": "initialize", "id": 1 }),
            json!({ "jsonrpc": "2.0", "id": null, "method": "initialize" }),
            json!({ "jsonrpc": "2.0", "id": 1.5, "method": "initialize" }),
        ] {
            let outcome = session.handle(message);
            assert_eq!(outcome.messages[0]["error"]["code"], -32_600);
        }

        let _ = session.handle(json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": { "capabilities": {} }
        }));
        session.handle(json!({
            "jsonrpc": "2.0",
            "method": "$/cancelRequest",
            "params": { "id": 9 }
        }));
        let cancelled = session.handle(json!({
            "jsonrpc": "2.0",
            "id": 9,
            "method": "textDocument/hover",
            "params": {}
        }));
        assert_eq!(cancelled.messages[0]["error"]["code"], -32_800);

        let repeated = session.handle(json!({
            "jsonrpc": "2.0",
            "id": 10,
            "method": "initialize",
            "params": {}
        }));
        assert_eq!(repeated.messages[0]["error"]["code"], -32_600);
        let unsupported = session.handle(json!({
            "jsonrpc": "2.0",
            "id": 11,
            "method": "workspace/symbol",
            "params": {}
        }));
        assert_eq!(unsupported.messages[0]["error"]["code"], -32_601);
    }

    #[test]
    fn framing_round_trips_and_rejects_unsafe_headers() -> Result<(), &'static str> {
        let message = json!({ "jsonrpc": "2.0", "method": "initialized", "params": {} });
        let mut bytes = Vec::new();
        assert!(write_frame(&mut bytes, &message).is_ok());
        let decoded = read_frame(&mut BufReader::new(Cursor::new(bytes)));
        let payload = decoded
            .map_err(|_| "written frame must decode")?
            .ok_or("written frame must contain a payload")?;
        let parsed = serde_json::from_slice::<Value>(&payload);
        assert_eq!(parsed.as_ref().ok(), Some(&message));

        for (frame, expected) in [
            (b"\r\n".as_slice(), FrameError::MissingContentLength),
            (
                b"Content-Length: 1\r\nContent-Length: 1\r\n\r\nx".as_slice(),
                FrameError::DuplicateContentLength,
            ),
            (
                b"Content-Length: nope\r\n\r\n".as_slice(),
                FrameError::InvalidContentLength,
            ),
            (
                b"Content-Length: 4\r\nContent-Type: application/vscode-jsonrpc; charset=latin1\r\n\r\nnull".as_slice(),
                FrameError::UnsupportedEncoding,
            ),
            (
                b"Content-Length: 4\r\n\r\n{}".as_slice(),
                FrameError::TruncatedBody,
            ),
        ] {
            assert_eq!(
                read_frame(&mut BufReader::new(Cursor::new(frame))),
                Err(expected)
            );
        }
        Ok(())
    }

    #[test]
    fn resource_limits_reject_oversized_messages_and_documents() {
        let oversized_document = "x".repeat(MAX_DOCUMENT_BYTES + 1);
        assert!(validate_document_size(&oversized_document).is_err());

        let oversized_header = format!("Content-Length: {}\r\n\r\n", MAX_MESSAGE_BYTES + 1);
        assert_eq!(
            read_frame(&mut BufReader::new(Cursor::new(oversized_header))),
            Err(FrameError::MessageTooLarge)
        );

        let mut session = initialized_session("stack 1.0 diagram \"Open\" { node a \"A\" }");
        for index in 1..MAX_OPEN_DOCUMENTS {
            session.documents.insert(
                format!("file:///workspace/{index}.stack"),
                Document {
                    text: String::new(),
                    version: 1,
                },
            );
        }
        let rejected = session.handle(json!({
            "jsonrpc": "2.0",
            "method": "textDocument/didOpen",
            "params": {
                "textDocument": {
                    "uri": "file:///workspace/overflow.stack",
                    "languageId": "stack",
                    "version": 1,
                    "text": "stack 1.0 diagram \"Overflow\" { node a \"A\" }"
                }
            }
        }));
        assert_eq!(rejected.messages[0]["method"], "window/logMessage");
        assert!(
            rejected.messages[0]["params"]["message"]
                .as_str()
                .is_some_and(|message| message.contains("open document limit exceeded"))
        );
        assert!(
            !session
                .documents
                .contains_key("file:///workspace/overflow.stack")
        );
    }

    #[test]
    fn stdio_transcript_recovers_from_json_errors_and_exits_cleanly() {
        let messages = [
            None,
            Some(json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": "initialize",
                "params": { "capabilities": {} }
            })),
            Some(json!({ "jsonrpc": "2.0", "id": 2, "method": "shutdown" })),
            Some(json!({ "jsonrpc": "2.0", "method": "exit" })),
        ];
        let mut input = Vec::new();
        for message in messages {
            if let Some(message) = message {
                assert!(write_frame(&mut input, &message).is_ok());
            } else {
                input.extend_from_slice(b"Content-Length: 1\r\n\r\n{");
            }
        }
        let mut output = Vec::new();
        let mut stderr = Vec::new();
        assert_eq!(
            run(&mut Cursor::new(input), &mut output, &mut stderr),
            EXIT_SUCCESS
        );
        assert!(stderr.is_empty());

        let mut reader = BufReader::new(Cursor::new(output));
        let mut responses = Vec::new();
        while let Ok(Some(payload)) = read_frame(&mut reader) {
            if let Ok(value) = serde_json::from_slice::<Value>(&payload) {
                responses.push(value);
            }
        }
        assert_eq!(responses.len(), 3);
        assert_eq!(responses[0]["error"]["code"], -32_700);
        assert_eq!(responses[1]["id"], 1);
        assert_eq!(responses[2]["id"], 2);

        let mut empty_output = Vec::new();
        let mut empty_error = Vec::new();
        assert_eq!(
            run(
                &mut Cursor::new(Vec::<u8>::new()),
                &mut empty_output,
                &mut empty_error,
            ),
            EXIT_USAGE_OR_IO
        );
        assert!(!empty_error.is_empty());
    }
}
