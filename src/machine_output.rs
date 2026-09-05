//! Stable machine-readable output owned by the native CLI.

use std::path::Path;

use serde::Serialize;
use stack_engine::{Diagnostic, Severity, SourceRange};

pub(crate) const SCHEMA_URI: &str =
    "https://raw.githubusercontent.com/stack-sh/cli/main/schemas/cli-output-v1.schema.json";
pub(crate) const SCHEMA_VERSION: u8 = 1;

pub(crate) const ARGUMENT_ERROR: &str = "CLI1001";
pub(crate) const IO_ERROR: &str = "CLI1002";
pub(crate) const CONFIGURATION_ERROR: &str = "CLI1003";
pub(crate) const ENGINE_ERROR: &str = "CLI1004";
pub(crate) const INTERNAL_ERROR: &str = "CLI1005";

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum Command {
    Check,
    Fmt,
    Render,
}

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum Outcome {
    Success,
    ChangesRequired,
    StackError,
    OperationalError,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Envelope {
    #[serde(rename = "$schema")]
    schema: &'static str,
    schema_version: u8,
    command: Command,
    outcome: Outcome,
    exit_status: u8,
    diagnostics: Vec<MachineDiagnostic>,
    artifacts: Vec<Artifact>,
    error: Option<MachineError>,
}

impl Envelope {
    pub(crate) fn result(
        command: Command,
        outcome: Outcome,
        exit_status: u8,
        source_path: &Path,
        diagnostics: &[Diagnostic],
        artifacts: Vec<Artifact>,
    ) -> Self {
        Self {
            schema: SCHEMA_URI,
            schema_version: SCHEMA_VERSION,
            command,
            outcome,
            exit_status,
            diagnostics: diagnostics
                .iter()
                .map(|diagnostic| MachineDiagnostic::new(source_path, diagnostic))
                .collect(),
            artifacts,
            error: None,
        }
    }

    pub(crate) fn operational_error(
        command: Command,
        code: &'static str,
        message: String,
        diagnostics_path: &Path,
        diagnostics: &[Diagnostic],
        artifacts: Vec<Artifact>,
    ) -> Self {
        Self {
            schema: SCHEMA_URI,
            schema_version: SCHEMA_VERSION,
            command,
            outcome: Outcome::OperationalError,
            exit_status: super::EXIT_USAGE_OR_IO,
            diagnostics: diagnostics
                .iter()
                .map(|diagnostic| MachineDiagnostic::new(diagnostics_path, diagnostic))
                .collect(),
            artifacts,
            error: Some(MachineError { code, message }),
        }
    }

    pub(crate) const fn exit_status(&self) -> u8 {
        self.exit_status
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MachineDiagnostic {
    code: String,
    severity: MachineSeverity,
    message: String,
    path: String,
    range: MachineRange,
    expected: Vec<String>,
    help: Option<String>,
    related: Vec<MachineRelatedInformation>,
}

impl MachineDiagnostic {
    fn new(path: &Path, diagnostic: &Diagnostic) -> Self {
        let path = path.to_string_lossy().into_owned();
        Self {
            code: diagnostic.code.clone(),
            severity: diagnostic.severity.into(),
            message: diagnostic.message.clone(),
            path: path.clone(),
            range: diagnostic.range.into(),
            expected: diagnostic.expected.clone(),
            help: diagnostic.help.clone(),
            related: diagnostic
                .related
                .iter()
                .map(|related| MachineRelatedInformation {
                    message: related.message.clone(),
                    path: path.clone(),
                    range: related.range.into(),
                })
                .collect(),
        }
    }
}

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "lowercase")]
enum MachineSeverity {
    Error,
    Warning,
}

impl From<Severity> for MachineSeverity {
    fn from(severity: Severity) -> Self {
        match severity {
            Severity::Error => Self::Error,
            Severity::Warning => Self::Warning,
        }
    }
}

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct MachineRange {
    start: MachinePosition,
    end: MachinePosition,
}

impl From<SourceRange> for MachineRange {
    fn from(range: SourceRange) -> Self {
        Self {
            start: range.start.into(),
            end: range.end.into(),
        }
    }
}

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct MachinePosition {
    byte_offset: u64,
    line: u64,
    column: u64,
}

impl From<stack_engine::SourcePosition> for MachinePosition {
    fn from(position: stack_engine::SourcePosition) -> Self {
        Self {
            byte_offset: position.byte_offset,
            line: position.line,
            column: position.column,
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct MachineRelatedInformation {
    message: String,
    path: String,
    range: MachineRange,
}

#[derive(Debug, Serialize)]
pub(crate) struct Artifact {
    kind: ArtifactKind,
    path: Option<String>,
    #[serde(rename = "mediaType")]
    media_type: &'static str,
    content: Option<String>,
}

impl Artifact {
    pub(crate) fn formatted_source(path: Option<&Path>, content: Option<String>) -> Self {
        Self {
            kind: ArtifactKind::FormattedSource,
            path: path.map(|path| path.to_string_lossy().into_owned()),
            media_type: "text/vnd.stack",
            content,
        }
    }

    pub(crate) fn rendered_svg(path: Option<&Path>, content: Option<String>) -> Self {
        Self {
            kind: ArtifactKind::RenderedSvg,
            path: path.map(|path| path.to_string_lossy().into_owned()),
            media_type: "image/svg+xml",
            content,
        }
    }

    pub(crate) fn provider_notice(path: &Path) -> Self {
        Self {
            kind: ArtifactKind::ProviderNotice,
            path: Some(path.to_string_lossy().into_owned()),
            media_type: "text/markdown",
            content: None,
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "kebab-case")]
enum ArtifactKind {
    FormattedSource,
    RenderedSvg,
    ProviderNotice,
}

#[derive(Debug, Serialize)]
struct MachineError {
    code: &'static str,
    message: String,
}
