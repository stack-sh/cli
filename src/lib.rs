//! Host boundary and command contract for the native Stack CLI.

#![forbid(unsafe_code)]

use std::collections::BTreeMap;
use std::ffi::{OsStr, OsString};
use std::fmt::Write as _;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Component, Path, PathBuf};

use stack_engine::{
    CheckOutput, Diagnostic, Engine, FormatOutput, OperationalError, ProviderAsset, ProviderNotice,
    ProviderPack, RenderOutput, Severity,
};

mod provider;
mod provider_catalog;

/// Exit status used when a command completes without Stack error diagnostics.
pub const EXIT_SUCCESS: u8 = 0;
/// Exit status used when Stack source contains at least one error diagnostic.
pub const EXIT_STACK_ERROR: u8 = 1;
/// Exit status used for argument, host I/O, or engine operational failures.
pub const EXIT_USAGE_OR_IO: u8 = 2;

const GENERAL_HELP: &str = "Stack diagram toolchain\n\nUsage:\n  stack check <FILE>\n  stack fmt [--check] <FILE|->\n  stack render <FILE> [--provider-pack <DIRECTORY>]... [-o <OUTPUT>] [--notice <NOTICE>]\n  stack icons list [PROVIDER] [QUERY]\n  stack icons import <PROVIDER> <ARCHIVE> [--source <ID>=<ARCHIVE>]... --accept-terms -o <DIRECTORY>\n  stack --help\n  stack --version\n\nCommands:\n  check     Validate a Stack source file without modifying it\n  fmt       Format a file in place or read from standard input\n  render    Render standalone SVG to standard output or a file\n  icons     List catalogs and import local provider icon archives\n";
const CHECK_HELP: &str =
    "Validate a Stack source file without modifying it\n\nUsage:\n  stack check <FILE>\n";
const FORMAT_HELP: &str = "Format Stack source canonically\n\nUsage:\n  stack fmt <FILE>\n  stack fmt --check <FILE>\n  stack fmt -\n\nArguments:\n  <FILE>    Format the file atomically in place\n  -         Read from standard input and write to standard output\n\nOptions:\n  --check   Report whether formatting is required without writing output\n";
const RENDER_HELP: &str = "Render Stack source as standalone SVG\n\nUsage:\n  stack render <FILE> [--provider-pack <DIRECTORY>]... [-o <OUTPUT>] [--notice <NOTICE>]\n\nArguments:\n  <FILE>                     Read Stack source bytes from this file\n\nOptions:\n  --provider-pack <DIRECTORY> Load one local imported provider pack; repeatable\n  -o <OUTPUT>                 Write SVG atomically instead of using standard output\n  --notice <NOTICE>           Write exact used-provider notices atomically\n";
const ICONS_HELP: &str = "Manage local provider icon packs\n\nUsage:\n  stack icons list [PROVIDER] [QUERY]\n  stack icons import <PROVIDER> <ARCHIVE> [--source <ID>=<ARCHIVE>]... --accept-terms -o <DIRECTORY>\n\nProviders:\n  aws            305 AWS Architecture Icons\n  gcp             45 Google Cloud product and category icons\n  azure          639 Azure Public Service Icons\n  simple-icons    62 curated developer and collaboration tools\n";
const ICONS_LIST_HELP: &str = "List searchable asset-free provider catalog metadata\n\nUsage:\n  stack icons list\n  stack icons list <PROVIDER> [QUERY]\n\nArguments:\n  <PROVIDER>  aws, gcp, azure, or simple-icons\n  [QUERY]     Case-insensitive ID, product, or category substring\n";
const ICONS_IMPORT_HELP: &str = "Import audited provider icon archives locally\n\nUsage:\n  stack icons import <PROVIDER> <ARCHIVE> [--source <ID>=<ARCHIVE>]... --accept-terms -o <DIRECTORY>\n\nArguments:\n  <PROVIDER>   aws, gcp, azure, or simple-icons\n  <ARCHIVE>    Local primary ZIP archive; Stack performs no download or upload\n\nOptions:\n  --source <ID>=<ARCHIVE>  Supply a required additional local archive; repeatable\n  --accept-terms           Confirm that you reviewed all provider and brand terms\n  -o <DIRECTORY>          Create a new local pack directory atomically\n";
const MAX_PROVIDER_MANIFEST_BYTES: usize = 1024 * 1024;
const MAX_PROVIDER_ASSET_BYTES: usize = 1024 * 1024;
const MAX_PROVIDER_PACKS: usize = 32;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FormatMode {
    Write,
    Check,
}

#[derive(Debug, PartialEq, Eq)]
enum RenderDestination {
    Stdout,
    File(PathBuf),
}

/// Runs the CLI with explicit streams and returns its process exit status.
pub fn run(
    arguments: impl IntoIterator<Item = OsString>,
    stdin: &mut dyn Read,
    stdout: &mut dyn Write,
    stderr: &mut dyn Write,
) -> u8 {
    let mut arguments = arguments.into_iter();
    let Some(command) = arguments.next() else {
        return argument_error("missing command", stderr);
    };

    if command == OsStr::new("--help") || command == OsStr::new("-h") {
        if let Some(extra) = arguments.next() {
            return argument_error(
                &format!("unexpected argument '{}'", extra.to_string_lossy()),
                stderr,
            );
        }
        return write_stdout(GENERAL_HELP, stdout, stderr);
    }
    if command == OsStr::new("--version") || command == OsStr::new("-V") {
        if let Some(extra) = arguments.next() {
            return argument_error(
                &format!("unexpected argument '{}'", extra.to_string_lossy()),
                stderr,
            );
        }
        return write_stdout(
            concat!("stack ", env!("CARGO_PKG_VERSION"), "\n"),
            stdout,
            stderr,
        );
    }
    if command == OsStr::new("check") {
        return run_check(arguments, stdout, stderr);
    }
    if command == OsStr::new("fmt") {
        return run_format(arguments, stdin, stdout, stderr);
    }
    if command == OsStr::new("render") {
        return run_render(arguments, stdout, stderr);
    }
    if command == OsStr::new("icons") {
        return run_icons(&mut arguments, stdout, stderr);
    }

    argument_error(
        &format!("unknown command '{}'", command.to_string_lossy()),
        stderr,
    )
}

fn run_icons(
    arguments: &mut dyn Iterator<Item = OsString>,
    stdout: &mut dyn Write,
    stderr: &mut dyn Write,
) -> u8 {
    let Some(command) = arguments.next() else {
        return argument_error("missing command for 'stack icons'", stderr);
    };
    if command == OsStr::new("--help") || command == OsStr::new("-h") {
        if let Some(extra) = arguments.next() {
            return argument_error(
                &format!("unexpected argument '{}'", extra.to_string_lossy()),
                stderr,
            );
        }
        return write_stdout(ICONS_HELP, stdout, stderr);
    }
    if command == OsStr::new("list") {
        return run_icons_list(arguments, stdout, stderr);
    }
    if command != OsStr::new("import") {
        return argument_error(
            &format!(
                "unknown command for 'stack icons': '{}'",
                command.to_string_lossy()
            ),
            stderr,
        );
    }
    run_icons_import(arguments, stdout, stderr)
}

fn run_icons_list(
    arguments: &mut dyn Iterator<Item = OsString>,
    stdout: &mut dyn Write,
    stderr: &mut dyn Write,
) -> u8 {
    let first = arguments.next();
    if first
        .as_ref()
        .is_some_and(|value| value == OsStr::new("--help") || value == OsStr::new("-h"))
    {
        if let Some(extra) = arguments.next() {
            return argument_error(
                &format!("unexpected argument '{}'", extra.to_string_lossy()),
                stderr,
            );
        }
        return write_stdout(ICONS_LIST_HELP, stdout, stderr);
    }
    let query = arguments.next();
    if let Some(extra) = arguments.next() {
        return argument_error(
            &format!("unexpected argument '{}'", extra.to_string_lossy()),
            stderr,
        );
    }
    let provider = first.as_ref().map(|value| value.to_string_lossy());
    let query = query.as_ref().map(|value| value.to_string_lossy());
    match provider::render_catalog_list(provider.as_deref(), query.as_deref()) {
        Ok(output) => write_stdout(&output, stdout, stderr),
        Err(error) => write_stderr_error(&format!("cannot list provider icons: {error}"), stderr),
    }
}

fn run_icons_import(
    arguments: &mut dyn Iterator<Item = OsString>,
    stdout: &mut dyn Write,
    stderr: &mut dyn Write,
) -> u8 {
    let Some(provider) = arguments.next() else {
        return argument_error("missing provider for 'stack icons import'", stderr);
    };
    if provider == OsStr::new("--help") || provider == OsStr::new("-h") {
        if let Some(extra) = arguments.next() {
            return argument_error(
                &format!("unexpected argument '{}'", extra.to_string_lossy()),
                stderr,
            );
        }
        return write_stdout(ICONS_IMPORT_HELP, stdout, stderr);
    }
    if provider.to_string_lossy().starts_with('-') {
        return argument_error(
            &format!("unknown option '{}'", provider.to_string_lossy()),
            stderr,
        );
    }
    let Some(archive) = arguments.next() else {
        return argument_error("missing archive for 'stack icons import'", stderr);
    };
    if archive.to_string_lossy().starts_with('-') {
        return argument_error(
            &format!("unknown option '{}'", archive.to_string_lossy()),
            stderr,
        );
    }

    let mut accepted_terms = false;
    let mut output = None;
    let mut additional_sources = BTreeMap::new();
    while let Some(option) = arguments.next() {
        if option == OsStr::new("--accept-terms") {
            if accepted_terms {
                return argument_error("duplicate '--accept-terms' option", stderr);
            }
            accepted_terms = true;
        } else if option == OsStr::new("-o") {
            if output.is_some() {
                return argument_error("duplicate '-o' option", stderr);
            }
            let Some(path) = arguments.next() else {
                return argument_error("missing output directory after '-o'", stderr);
            };
            output = Some(PathBuf::from(path));
        } else if option == OsStr::new("--source") {
            let Some(value) = arguments.next() else {
                return argument_error("missing <ID>=<ARCHIVE> after '--source'", stderr);
            };
            let Some(value) = value.to_str() else {
                return argument_error("source ID and archive must be valid UTF-8", stderr);
            };
            let Some((id, path)) = value.split_once('=') else {
                return argument_error("source must use <ID>=<ARCHIVE>", stderr);
            };
            if id.is_empty() || path.is_empty() {
                return argument_error("source must use non-empty <ID>=<ARCHIVE>", stderr);
            }
            if additional_sources
                .insert(id.to_owned(), PathBuf::from(path))
                .is_some()
            {
                return argument_error(&format!("duplicate source ID '{id}'"), stderr);
            }
        } else {
            return argument_error(
                &format!("unexpected argument '{}'", option.to_string_lossy()),
                stderr,
            );
        }
    }
    if !accepted_terms {
        return argument_error(
            "provider terms must be reviewed and accepted with '--accept-terms'",
            stderr,
        );
    }
    let Some(output) = output else {
        return argument_error("missing output directory after '-o'", stderr);
    };
    if output == Path::new(&archive) {
        return argument_error("archive and output directory must be different", stderr);
    }
    if additional_sources.values().any(|path| path == &output) {
        return argument_error(
            "source archive and output directory must be different",
            stderr,
        );
    }
    let provider_name = provider.to_string_lossy();
    match provider::import_provider_pack(
        &provider_name,
        Path::new(&archive),
        &additional_sources,
        &output,
    ) {
        Ok(summary) => write_stdout(
            &format!(
                "Imported {} {} icons to '{}'.\nManifest: {}\nNotice: {}\n",
                summary.icon_count,
                summary.provider_name,
                output.display(),
                summary.manifest_path.display(),
                summary.notice_path.display()
            ),
            stdout,
            stderr,
        ),
        Err(error) => write_stderr_error(&format!("cannot import provider icons: {error}"), stderr),
    }
}

fn run_render(
    mut arguments: impl Iterator<Item = OsString>,
    stdout: &mut dyn Write,
    stderr: &mut dyn Write,
) -> u8 {
    let Some(source) = arguments.next() else {
        return argument_error("missing file for 'stack render'", stderr);
    };
    if source == OsStr::new("--help") || source == OsStr::new("-h") {
        if let Some(extra) = arguments.next() {
            return argument_error(
                &format!("unexpected argument '{}'", extra.to_string_lossy()),
                stderr,
            );
        }
        return write_stdout(RENDER_HELP, stdout, stderr);
    }
    if source.to_string_lossy().starts_with('-') {
        return argument_error(
            &format!("unknown option '{}'", source.to_string_lossy()),
            stderr,
        );
    }

    let mut destination = None;
    let mut notice_path = None;
    let mut provider_pack_paths = Vec::new();
    while let Some(option) = arguments.next() {
        if option == OsStr::new("-o") {
            if destination.is_some() {
                return argument_error("duplicate '-o' option", stderr);
            }
            let Some(output) = arguments.next() else {
                return argument_error("missing output file after '-o'", stderr);
            };
            destination = Some(RenderDestination::File(PathBuf::from(output)));
        } else if option == OsStr::new("--notice") {
            if notice_path.is_some() {
                return argument_error("duplicate '--notice' option", stderr);
            }
            let Some(path) = arguments.next() else {
                return argument_error("missing notice file after '--notice'", stderr);
            };
            notice_path = Some(PathBuf::from(path));
        } else if option == OsStr::new("--provider-pack") {
            let Some(path) = arguments.next() else {
                return argument_error("missing provider pack directory", stderr);
            };
            provider_pack_paths.push(PathBuf::from(path));
            if provider_pack_paths.len() > MAX_PROVIDER_PACKS {
                return argument_error("at most 32 provider packs may be loaded", stderr);
            }
        } else {
            return argument_error(
                &format!("unexpected argument '{}'", option.to_string_lossy()),
                stderr,
            );
        }
    }

    let destination = destination.unwrap_or(RenderDestination::Stdout);
    if matches!(&destination, RenderDestination::File(path) if path.as_os_str() == source) {
        return argument_error("input and output files must be different", stderr);
    }
    if notice_path
        .as_ref()
        .is_some_and(|path| path.as_os_str() == source)
    {
        return argument_error("input and notice files must be different", stderr);
    }
    if matches!(&destination, RenderDestination::File(path) if notice_path.as_ref() == Some(path)) {
        return argument_error("output and notice files must be different", stderr);
    }

    render_file(
        Path::new(&source),
        destination,
        &provider_pack_paths,
        notice_path.as_deref(),
        stdout,
        stderr,
    )
}

fn run_format(
    mut arguments: impl Iterator<Item = OsString>,
    stdin: &mut dyn Read,
    stdout: &mut dyn Write,
    stderr: &mut dyn Write,
) -> u8 {
    let Some(first) = arguments.next() else {
        return argument_error("missing file for 'stack fmt'", stderr);
    };
    if first == OsStr::new("--help") || first == OsStr::new("-h") {
        if let Some(extra) = arguments.next() {
            return argument_error(
                &format!("unexpected argument '{}'", extra.to_string_lossy()),
                stderr,
            );
        }
        return write_stdout(FORMAT_HELP, stdout, stderr);
    }

    let (mode, input) = if first == OsStr::new("--check") {
        let Some(input) = arguments.next() else {
            return argument_error("missing file for 'stack fmt --check'", stderr);
        };
        (FormatMode::Check, input)
    } else {
        (FormatMode::Write, first)
    };
    if input != OsStr::new("-") && input.to_string_lossy().starts_with('-') {
        return argument_error(
            &format!("unknown option '{}'", input.to_string_lossy()),
            stderr,
        );
    }
    if let Some(extra) = arguments.next() {
        return argument_error(
            &format!("unexpected argument '{}'", extra.to_string_lossy()),
            stderr,
        );
    }

    if input == OsStr::new("-") {
        format_stdin(mode, stdin, stdout, stderr)
    } else {
        format_file(mode, Path::new(&input), stderr)
    }
}

fn run_check(
    mut arguments: impl Iterator<Item = OsString>,
    stdout: &mut dyn Write,
    stderr: &mut dyn Write,
) -> u8 {
    let Some(path) = arguments.next() else {
        return argument_error("missing file for 'stack check'", stderr);
    };

    if path == OsStr::new("--help") || path == OsStr::new("-h") {
        if let Some(extra) = arguments.next() {
            return argument_error(
                &format!("unexpected argument '{}'", extra.to_string_lossy()),
                stderr,
            );
        }
        return write_stdout(CHECK_HELP, stdout, stderr);
    }

    if let Some(extra) = arguments.next() {
        return argument_error(
            &format!("unexpected argument '{}'", extra.to_string_lossy()),
            stderr,
        );
    }

    check_file(Path::new(&path), stderr)
}

fn check_file(path: &Path, stderr: &mut dyn Write) -> u8 {
    check_file_with(path, stderr, |source| Engine::bundled().check(source))
}

fn check_file_with(
    path: &Path,
    stderr: &mut dyn Write,
    check: impl FnOnce(&[u8]) -> Result<CheckOutput, OperationalError>,
) -> u8 {
    let source = match fs::read(path) {
        Ok(source) => source,
        Err(error) => {
            return write_stderr_error(
                &format!(
                    "cannot read '{}': {}",
                    path.display(),
                    stable_io_error(error.kind())
                ),
                stderr,
            );
        }
    };

    let output = match check(&source) {
        Ok(output) => output,
        Err(error) => {
            return write_stderr_error(
                &format!("cannot check '{}': {error}", path.display()),
                stderr,
            );
        }
    };
    let has_errors = match write_diagnostics(path, &output.diagnostics, stderr) {
        Ok(has_errors) => has_errors,
        Err(()) => return EXIT_USAGE_OR_IO,
    };

    if has_errors {
        EXIT_STACK_ERROR
    } else {
        EXIT_SUCCESS
    }
}

fn render_file(
    path: &Path,
    destination: RenderDestination,
    provider_pack_paths: &[PathBuf],
    notice_path: Option<&Path>,
    stdout: &mut dyn Write,
    stderr: &mut dyn Write,
) -> u8 {
    let mut provider_packs = Vec::with_capacity(provider_pack_paths.len());
    for provider_pack_path in provider_pack_paths {
        let provider_pack = match load_provider_pack(provider_pack_path) {
            Ok(provider_pack) => provider_pack,
            Err(reason) => {
                return write_stderr_error(
                    &format!(
                        "cannot load provider pack '{}': {reason}",
                        provider_pack_path.display()
                    ),
                    stderr,
                );
            }
        };
        provider_packs.push(provider_pack);
    }
    let engine = match Engine::with_provider_packs(&provider_packs) {
        Ok(engine) => engine,
        Err(error) => {
            return write_stderr_error(&format!("cannot load provider packs: {error}"), stderr);
        }
    };
    render_file_with(
        path,
        destination,
        notice_path,
        stdout,
        stderr,
        |source| engine.render(source),
        atomic_write_output,
    )
}

fn render_file_with(
    path: &Path,
    destination: RenderDestination,
    notice_path: Option<&Path>,
    stdout: &mut dyn Write,
    stderr: &mut dyn Write,
    render: impl FnOnce(&[u8]) -> Result<RenderOutput, OperationalError>,
    mut write_output: impl FnMut(&Path, &[u8]) -> io::Result<()>,
) -> u8 {
    let source = match fs::read(path) {
        Ok(source) => source,
        Err(error) => {
            return write_stderr_error(
                &format!(
                    "cannot read '{}': {}",
                    path.display(),
                    stable_io_error(error.kind())
                ),
                stderr,
            );
        }
    };
    let output = match render(&source) {
        Ok(output) => output,
        Err(error) => {
            return write_stderr_error(
                &format!("cannot render '{}': {error}", path.display()),
                stderr,
            );
        }
    };
    let has_errors = match write_diagnostics(path, &output.diagnostics, stderr) {
        Ok(has_errors) => has_errors,
        Err(()) => return EXIT_USAGE_OR_IO,
    };
    if has_errors {
        return EXIT_STACK_ERROR;
    }
    let Some(svg) = output.svg else {
        return write_stderr_error("renderer produced no SVG or error diagnostic", stderr);
    };

    match destination {
        RenderDestination::Stdout => {
            if stdout.write_all(svg.as_bytes()).is_err() {
                return write_stderr_error("cannot write rendered SVG", stderr);
            }
        }
        RenderDestination::File(output_path) => {
            if let Err(error) = write_output(&output_path, svg.as_bytes()) {
                return write_stderr_error(
                    &format!(
                        "cannot write '{}': {}",
                        output_path.display(),
                        stable_io_error(error.kind())
                    ),
                    stderr,
                );
            }
        }
    }
    if let Some(notice_path) = notice_path {
        let notice = render_provider_notices(&output.provider_notices);
        if let Err(error) = write_output(notice_path, notice.as_bytes()) {
            return write_stderr_error(
                &format!(
                    "cannot write '{}': {}",
                    notice_path.display(),
                    stable_io_error(error.kind())
                ),
                stderr,
            );
        }
    }
    EXIT_SUCCESS
}

fn load_provider_pack(root: &Path) -> Result<ProviderPack, String> {
    let root_metadata =
        fs::symlink_metadata(root).map_err(|error| stable_io_error(error.kind()).to_owned())?;
    if root_metadata.file_type().is_symlink() || !root_metadata.is_dir() {
        return Err("pack path must be a real directory, not a symlink".to_owned());
    }

    let manifest_path = root.join("manifest.json");
    let manifest_bytes = read_bounded_regular_file(&manifest_path, MAX_PROVIDER_MANIFEST_BYTES)?;
    let manifest: stack_theme::ProviderPack = serde_json::from_slice(&manifest_bytes)
        .map_err(|_| "manifest.json is invalid".to_owned())?;
    let mut assets = Vec::with_capacity(manifest.icons.len());
    for icon in &manifest.icons {
        let relative = safe_provider_asset_path(&icon.asset.path)?;
        let path = root.join(relative);
        let svg = read_bounded_regular_file(&path, MAX_PROVIDER_ASSET_BYTES)?;
        let svg =
            String::from_utf8(svg).map_err(|_| format!("'{}' is not UTF-8 SVG", path.display()))?;
        assets.push(ProviderAsset::new(&icon.asset.path, svg));
    }
    ProviderPack::new(manifest, assets).map_err(|error| error.to_string())
}

fn read_bounded_regular_file(path: &Path, limit: usize) -> Result<Vec<u8>, String> {
    let metadata = fs::symlink_metadata(path).map_err(|error| {
        format!(
            "cannot read '{}': {}",
            path.display(),
            stable_io_error(error.kind())
        )
    })?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(format!(
            "'{}' must be a regular file, not a symlink",
            path.display()
        ));
    }
    if metadata.len() > limit as u64 {
        return Err(format!("'{}' exceeds the size limit", path.display()));
    }
    let mut contents = Vec::with_capacity(metadata.len() as usize);
    File::open(path)
        .and_then(|mut file| file.read_to_end(&mut contents))
        .map_err(|error| {
            format!(
                "cannot read '{}': {}",
                path.display(),
                stable_io_error(error.kind())
            )
        })?;
    if contents.len() > limit {
        return Err(format!("'{}' exceeds the size limit", path.display()));
    }
    Ok(contents)
}

fn safe_provider_asset_path(value: &str) -> Result<&Path, String> {
    let path = Path::new(value);
    let components = path.components().collect::<Vec<_>>();
    if components.len() != 2
        || components[0] != Component::Normal(OsStr::new("assets"))
        || !matches!(components[1], Component::Normal(_))
        || path.extension() != Some(OsStr::new("svg"))
    {
        return Err(format!("unsafe provider asset path '{value}'"));
    }
    Ok(path)
}

fn render_provider_notices(notices: &[ProviderNotice]) -> String {
    let mut output = String::from("# Stack provider icon notices\n");
    if notices.is_empty() {
        output.push_str("\nNo provider icons were embedded in this artifact.\n");
        return output;
    }
    for notice in notices {
        let _ = write!(
            output,
            "\n## {} (`{}`)\n\n- Pack version: {}\n- Pack revision: `{}`\n\n{}\n\n{}\n\n{}\n\nSources:\n",
            notice_text(&notice.provider_name),
            notice.provider_id,
            notice_text(&notice.pack_version),
            notice.pack_revision,
            notice_text(&notice.attribution),
            notice_text(&notice.terms_summary),
            notice_text(&notice.non_endorsement),
        );
        for source in &notice.sources {
            let _ = writeln!(
                output,
                "- `{}`: release {}; archive `{}`; terms {}",
                source.id,
                notice_text(&source.release),
                source.archive_sha256,
                notice_text(&source.terms_url),
            );
        }
        output.push_str("\nUsed icons:\n");
        for icon in &notice.icons {
            let _ = write!(
                output,
                "- `{}`: {} (source `{}`)",
                icon.id,
                notice_text(&icon.product_name),
                notice_text(&icon.source_id),
            );
            if let Some(url) = &icon.brand_source_url {
                let _ = write!(output, "; brand source {}", notice_text(url));
            }
            if let Some(url) = &icon.brand_guidelines_url {
                let _ = write!(output, "; brand guidelines {}", notice_text(url));
            }
            output.push('\n');
        }
    }
    output
}

fn notice_text(value: &str) -> String {
    value
        .chars()
        .map(|character| match character {
            '\r' | '\n' | '\t' => ' ',
            '<' | '>' => ' ',
            _ if character.is_control() => ' ',
            _ => character,
        })
        .collect()
}

fn format_file(mode: FormatMode, path: &Path, stderr: &mut dyn Write) -> u8 {
    format_file_with(
        mode,
        path,
        stderr,
        |source| Engine::bundled().format(source),
        atomic_replace,
    )
}

fn format_file_with(
    mode: FormatMode,
    path: &Path,
    stderr: &mut dyn Write,
    format: impl FnOnce(&[u8]) -> Result<FormatOutput, OperationalError>,
    replace: impl FnOnce(&Path, &[u8]) -> io::Result<()>,
) -> u8 {
    let source = match fs::read(path) {
        Ok(source) => source,
        Err(error) => {
            return write_stderr_error(
                &format!(
                    "cannot read '{}': {}",
                    path.display(),
                    stable_io_error(error.kind())
                ),
                stderr,
            );
        }
    };
    let output = match format(&source) {
        Ok(output) => output,
        Err(error) => {
            return write_stderr_error(
                &format!("cannot format '{}': {error}", path.display()),
                stderr,
            );
        }
    };
    let has_errors = match write_diagnostics(path, &output.diagnostics, stderr) {
        Ok(has_errors) => has_errors,
        Err(()) => return EXIT_USAGE_OR_IO,
    };
    let Some(formatted) = output.formatted_source else {
        return if has_errors {
            EXIT_STACK_ERROR
        } else {
            write_stderr_error("formatter produced no source or error diagnostic", stderr)
        };
    };
    let changed = formatted.as_bytes() != source;

    if mode == FormatMode::Check {
        return if changed || has_errors {
            EXIT_STACK_ERROR
        } else {
            EXIT_SUCCESS
        };
    }
    if changed {
        if let Err(error) = replace(path, formatted.as_bytes()) {
            return write_stderr_error(
                &format!(
                    "cannot replace '{}': {}",
                    path.display(),
                    stable_io_error(error.kind())
                ),
                stderr,
            );
        }
    }

    if has_errors {
        EXIT_STACK_ERROR
    } else {
        EXIT_SUCCESS
    }
}

fn format_stdin(
    mode: FormatMode,
    stdin: &mut dyn Read,
    stdout: &mut dyn Write,
    stderr: &mut dyn Write,
) -> u8 {
    let mut source = Vec::new();
    if stdin.read_to_end(&mut source).is_err() {
        return write_stderr_error("cannot read standard input", stderr);
    }
    format_stdin_with(mode, &source, stdout, stderr, |source| {
        Engine::bundled().format(source)
    })
}

fn format_stdin_with(
    mode: FormatMode,
    source: &[u8],
    stdout: &mut dyn Write,
    stderr: &mut dyn Write,
    format: impl FnOnce(&[u8]) -> Result<FormatOutput, OperationalError>,
) -> u8 {
    let output = match format(source) {
        Ok(output) => output,
        Err(error) => return write_stderr_error(&format!("cannot format stdin: {error}"), stderr),
    };
    let has_errors = match write_diagnostics(Path::new("<stdin>"), &output.diagnostics, stderr) {
        Ok(has_errors) => has_errors,
        Err(()) => return EXIT_USAGE_OR_IO,
    };
    let Some(formatted) = output.formatted_source else {
        return if has_errors {
            EXIT_STACK_ERROR
        } else {
            write_stderr_error("formatter produced no source or error diagnostic", stderr)
        };
    };
    let changed = formatted.as_bytes() != source;

    if mode == FormatMode::Check {
        return if changed || has_errors {
            EXIT_STACK_ERROR
        } else {
            EXIT_SUCCESS
        };
    }
    if stdout.write_all(formatted.as_bytes()).is_err() {
        return write_stderr_error("cannot write formatted source", stderr);
    }
    if has_errors {
        EXIT_STACK_ERROR
    } else {
        EXIT_SUCCESS
    }
}

fn atomic_replace(path: &Path, contents: &[u8]) -> io::Result<()> {
    let permissions = fs::metadata(path)?.permissions();
    atomic_write(path, contents, Some(permissions))
}

fn atomic_write_output(path: &Path, contents: &[u8]) -> io::Result<()> {
    let permissions = match fs::metadata(path) {
        Ok(metadata) => Some(metadata.permissions()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => None,
        Err(error) => return Err(error),
    };
    atomic_write(path, contents, permissions)
}

fn atomic_write(
    path: &Path,
    contents: &[u8],
    permissions: Option<fs::Permissions>,
) -> io::Result<()> {
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let (temporary_path, mut temporary_file) = create_temporary_file(parent)?;

    let prepared = temporary_file
        .write_all(contents)
        .and_then(|()| match permissions {
            Some(permissions) => temporary_file.set_permissions(permissions),
            None => Ok(()),
        })
        .and_then(|()| temporary_file.sync_all());
    drop(temporary_file);
    if let Err(error) = prepared {
        let _ = fs::remove_file(&temporary_path);
        return Err(error);
    }
    if let Err(error) = fs::rename(&temporary_path, path) {
        let _ = fs::remove_file(&temporary_path);
        return Err(error);
    }
    Ok(())
}

fn create_temporary_file(parent: &Path) -> io::Result<(PathBuf, File)> {
    for attempt in 0..128_u8 {
        let path = parent.join(format!(".stack-tmp-{}-{attempt}", std::process::id()));
        match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(file) => return Ok((path, file)),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error),
        }
    }
    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "could not reserve an atomic replacement file",
    ))
}

fn write_diagnostics(
    path: &Path,
    diagnostics: &[Diagnostic],
    stderr: &mut dyn Write,
) -> Result<bool, ()> {
    let has_errors = diagnostics
        .iter()
        .any(|diagnostic| diagnostic.severity == Severity::Error);
    let rendered = render_diagnostics(path, diagnostics);
    if !rendered.is_empty() && stderr.write_all(rendered.as_bytes()).is_err() {
        return Err(());
    }
    Ok(has_errors)
}

fn render_diagnostics(path: &Path, diagnostics: &[Diagnostic]) -> String {
    let mut rendered = String::new();
    for diagnostic in diagnostics {
        let severity = match diagnostic.severity {
            Severity::Error => "error",
            Severity::Warning => "warning",
        };
        let _ = writeln!(
            rendered,
            "{}:{}:{}: {severity}[{}]: {}",
            path.display(),
            diagnostic.range.start.line,
            diagnostic.range.start.column,
            diagnostic.code,
            diagnostic.message
        );
        if !diagnostic.expected.is_empty() {
            let _ = writeln!(rendered, "  expected: {}", diagnostic.expected.join(", "));
        }
        if let Some(help) = &diagnostic.help {
            let _ = writeln!(rendered, "  help: {help}");
        }
        for related in &diagnostic.related {
            let _ = writeln!(
                rendered,
                "{}:{}:{}: note: {}",
                path.display(),
                related.range.start.line,
                related.range.start.column,
                related.message
            );
        }
    }
    rendered
}

fn stable_io_error(kind: io::ErrorKind) -> &'static str {
    match kind {
        io::ErrorKind::NotFound => "file not found",
        io::ErrorKind::PermissionDenied => "permission denied",
        io::ErrorKind::InvalidData => "invalid data",
        _ => "I/O error",
    }
}

fn argument_error(message: &str, stderr: &mut dyn Write) -> u8 {
    let _ = writeln!(stderr, "error: {message}\n\n{GENERAL_HELP}");
    EXIT_USAGE_OR_IO
}

fn write_stderr_error(message: &str, stderr: &mut dyn Write) -> u8 {
    let _ = writeln!(stderr, "error: {message}");
    EXIT_USAGE_OR_IO
}

fn write_stdout(message: &str, stdout: &mut dyn Write, stderr: &mut dyn Write) -> u8 {
    if stdout.write_all(message.as_bytes()).is_ok() {
        EXIT_SUCCESS
    } else {
        write_stderr_error("cannot write command output", stderr)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct FailingWriter;

    struct FailingReader;

    impl Read for FailingReader {
        fn read(&mut self, _buffer: &mut [u8]) -> io::Result<usize> {
            Err(io::Error::other("test reader failure"))
        }
    }

    impl Write for FailingWriter {
        fn write(&mut self, _buffer: &[u8]) -> io::Result<usize> {
            Err(io::Error::other("test writer failure"))
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    fn run_without_input(
        arguments: impl IntoIterator<Item = OsString>,
        stdout: &mut dyn Write,
        stderr: &mut dyn Write,
    ) -> u8 {
        run(arguments, &mut io::empty(), stdout, stderr)
    }

    #[test]
    fn help_version_and_argument_errors_have_stable_streams() {
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        assert_eq!(
            run_without_input([OsString::from("--version")], &mut stdout, &mut stderr),
            EXIT_SUCCESS
        );
        assert_eq!(stdout, b"stack 0.2.0\n");
        assert!(stderr.is_empty());

        stdout.clear();
        assert_eq!(
            run_without_input([OsString::from("--help")], &mut stdout, &mut stderr),
            EXIT_SUCCESS
        );
        assert_eq!(stdout, GENERAL_HELP.as_bytes());

        stdout.clear();
        assert_eq!(
            run_without_input([], &mut stdout, &mut stderr),
            EXIT_USAGE_OR_IO
        );
        assert!(stdout.is_empty());
        assert!(String::from_utf8_lossy(&stderr).contains("error: missing command"));
    }

    #[test]
    fn command_aliases_and_invalid_arguments_are_stable() {
        for alias in ["-h", "-V"] {
            let mut stdout = Vec::new();
            let mut stderr = Vec::new();
            assert_eq!(
                run_without_input([OsString::from(alias)], &mut stdout, &mut stderr),
                EXIT_SUCCESS
            );
            assert!(!stdout.is_empty());
            assert!(stderr.is_empty());
        }

        for arguments in [
            vec![OsString::from("unknown")],
            vec![OsString::from("--help"), OsString::from("extra")],
            vec![OsString::from("--version"), OsString::from("extra")],
            vec![OsString::from("check")],
            vec![
                OsString::from("check"),
                OsString::from("--help"),
                OsString::from("extra"),
            ],
            vec![OsString::from("fmt")],
            vec![
                OsString::from("fmt"),
                OsString::from("--help"),
                OsString::from("extra"),
            ],
            vec![OsString::from("fmt"), OsString::from("--check")],
            vec![OsString::from("fmt"), OsString::from("--unknown")],
            vec![
                OsString::from("fmt"),
                OsString::from("file.stack"),
                OsString::from("extra"),
            ],
            vec![OsString::from("render")],
            vec![
                OsString::from("render"),
                OsString::from("--help"),
                OsString::from("extra"),
            ],
            vec![OsString::from("render"), OsString::from("--unknown")],
            vec![
                OsString::from("render"),
                OsString::from("file.stack"),
                OsString::from("-o"),
            ],
            vec![
                OsString::from("render"),
                OsString::from("file.stack"),
                OsString::from("unexpected"),
            ],
            vec![
                OsString::from("render"),
                OsString::from("file.stack"),
                OsString::from("-o"),
                OsString::from("out.svg"),
                OsString::from("extra"),
            ],
            vec![
                OsString::from("render"),
                OsString::from("file.stack"),
                OsString::from("-o"),
                OsString::from("one.svg"),
                OsString::from("-o"),
                OsString::from("two.svg"),
            ],
            vec![
                OsString::from("render"),
                OsString::from("file.stack"),
                OsString::from("--notice"),
            ],
            vec![
                OsString::from("render"),
                OsString::from("file.stack"),
                OsString::from("--notice"),
                OsString::from("one.md"),
                OsString::from("--notice"),
                OsString::from("two.md"),
            ],
            vec![
                OsString::from("render"),
                OsString::from("file.stack"),
                OsString::from("--provider-pack"),
            ],
            vec![
                OsString::from("render"),
                OsString::from("file.stack"),
                OsString::from("-o"),
                OsString::from("file.stack"),
            ],
            vec![
                OsString::from("render"),
                OsString::from("file.stack"),
                OsString::from("--notice"),
                OsString::from("file.stack"),
            ],
            vec![
                OsString::from("render"),
                OsString::from("file.stack"),
                OsString::from("-o"),
                OsString::from("artifact"),
                OsString::from("--notice"),
                OsString::from("artifact"),
            ],
            vec![OsString::from("icons")],
            vec![OsString::from("icons"), OsString::from("unknown")],
            vec![
                OsString::from("icons"),
                OsString::from("list"),
                OsString::from("aws"),
                OsString::from("s3"),
                OsString::from("extra"),
            ],
            vec![
                OsString::from("icons"),
                OsString::from("--help"),
                OsString::from("extra"),
            ],
            vec![OsString::from("icons"), OsString::from("import")],
            vec![
                OsString::from("icons"),
                OsString::from("import"),
                OsString::from("aws"),
            ],
            vec![
                OsString::from("icons"),
                OsString::from("import"),
                OsString::from("aws"),
                OsString::from("icons.zip"),
                OsString::from("-o"),
                OsString::from("pack"),
            ],
            vec![
                OsString::from("icons"),
                OsString::from("import"),
                OsString::from("aws"),
                OsString::from("icons.zip"),
                OsString::from("--accept-terms"),
                OsString::from("-o"),
            ],
            vec![
                OsString::from("icons"),
                OsString::from("import"),
                OsString::from("--unknown"),
            ],
            vec![
                OsString::from("icons"),
                OsString::from("import"),
                OsString::from("gcp"),
                OsString::from("core.zip"),
                OsString::from("--source"),
                OsString::from("invalid"),
                OsString::from("--accept-terms"),
                OsString::from("-o"),
                OsString::from("pack"),
            ],
            vec![
                OsString::from("check"),
                OsString::from("file.stack"),
                OsString::from("extra"),
            ],
        ] {
            let mut stdout = Vec::new();
            let mut stderr = Vec::new();
            assert_eq!(
                run_without_input(arguments, &mut stdout, &mut stderr),
                EXIT_USAGE_OR_IO
            );
            assert!(stdout.is_empty());
            assert!(String::from_utf8_lossy(&stderr).starts_with("error:"));
        }

        let mut too_many_packs = vec![OsString::from("render"), OsString::from("file.stack")];
        for index in 0..=MAX_PROVIDER_PACKS {
            too_many_packs.push(OsString::from("--provider-pack"));
            too_many_packs.push(OsString::from(format!("pack-{index}")));
        }
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        assert_eq!(
            run_without_input(too_many_packs, &mut stdout, &mut stderr),
            EXIT_USAGE_OR_IO
        );
        assert!(String::from_utf8_lossy(&stderr).contains("at most 32 provider packs"));

        stdout.clear();
        stderr.clear();
        assert_eq!(
            run_without_input(
                [OsString::from("check"), OsString::from("-h")],
                &mut stdout,
                &mut stderr,
            ),
            EXIT_SUCCESS
        );
        assert_eq!(stdout, CHECK_HELP.as_bytes());
        assert!(stderr.is_empty());

        stdout.clear();
        assert_eq!(
            run_without_input(
                [OsString::from("fmt"), OsString::from("-h")],
                &mut stdout,
                &mut stderr,
            ),
            EXIT_SUCCESS
        );
        assert_eq!(stdout, FORMAT_HELP.as_bytes());

        stdout.clear();
        assert_eq!(
            run_without_input(
                [OsString::from("render"), OsString::from("--help")],
                &mut stdout,
                &mut stderr,
            ),
            EXIT_SUCCESS
        );
        assert_eq!(stdout, RENDER_HELP.as_bytes());

        stdout.clear();
        assert_eq!(
            run_without_input(
                [OsString::from("icons"), OsString::from("--help")],
                &mut stdout,
                &mut stderr,
            ),
            EXIT_SUCCESS
        );
        assert_eq!(stdout, ICONS_HELP.as_bytes());

        stdout.clear();
        assert_eq!(
            run_without_input(
                [
                    OsString::from("icons"),
                    OsString::from("import"),
                    OsString::from("--help"),
                ],
                &mut stdout,
                &mut stderr,
            ),
            EXIT_SUCCESS
        );
        assert_eq!(stdout, ICONS_IMPORT_HELP.as_bytes());

        stdout.clear();
        assert_eq!(
            run_without_input(
                [
                    OsString::from("icons"),
                    OsString::from("list"),
                    OsString::from("--help"),
                ],
                &mut stdout,
                &mut stderr,
            ),
            EXIT_SUCCESS
        );
        assert_eq!(stdout, ICONS_LIST_HELP.as_bytes());
    }

    #[test]
    fn provider_catalog_listing_is_searchable_without_asset_bytes() {
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        assert_eq!(
            run_without_input(
                [OsString::from("icons"), OsString::from("list")],
                &mut stdout,
                &mut stderr,
            ),
            EXIT_SUCCESS
        );
        let listing = String::from_utf8_lossy(&stdout);
        assert!(listing.contains("aws\t305"));
        assert!(listing.contains("azure\t639"));
        assert!(listing.contains("simple-icons\t62"));

        stdout.clear();
        assert_eq!(
            run_without_input(
                [
                    OsString::from("icons"),
                    OsString::from("list"),
                    OsString::from("aws"),
                    OsString::from("s3"),
                ],
                &mut stdout,
                &mut stderr,
            ),
            EXIT_SUCCESS
        );
        let listing = String::from_utf8_lossy(&stdout);
        assert!(listing.contains("aws:s3\tAmazon Simple Storage Service"));
        assert!(listing.lines().count() >= 2);
        assert!(stderr.is_empty());
    }

    #[test]
    fn diagnostic_rendering_includes_related_locations() {
        let diagnostic = Diagnostic {
            code: "STKTEST".to_owned(),
            severity: Severity::Error,
            message: "primary".to_owned(),
            range: stack_engine::SourceRange {
                start: stack_engine::SourcePosition {
                    byte_offset: 0,
                    line: 2,
                    column: 3,
                },
                end: stack_engine::SourcePosition {
                    byte_offset: 1,
                    line: 2,
                    column: 4,
                },
            },
            expected: vec!["node".to_owned(), "group".to_owned()],
            help: Some("choose a declaration".to_owned()),
            related: vec![stack_engine::RelatedInformation {
                message: "related".to_owned(),
                range: stack_engine::SourceRange {
                    start: stack_engine::SourcePosition {
                        byte_offset: 2,
                        line: 4,
                        column: 5,
                    },
                    end: stack_engine::SourcePosition {
                        byte_offset: 3,
                        line: 4,
                        column: 6,
                    },
                },
            }],
        };

        assert_eq!(
            render_diagnostics(Path::new("arch.stack"), &[diagnostic]),
            "arch.stack:2:3: error[STKTEST]: primary\n  expected: node, group\n  help: choose a declaration\narch.stack:4:5: note: related\n"
        );
    }

    #[test]
    fn host_error_labels_and_stream_failures_are_stable() {
        assert_eq!(
            stable_io_error(io::ErrorKind::PermissionDenied),
            "permission denied"
        );
        assert_eq!(stable_io_error(io::ErrorKind::InvalidData), "invalid data");
        assert_eq!(stable_io_error(io::ErrorKind::Other), "I/O error");

        let mut failed_stdout = FailingWriter;
        let mut stderr = Vec::new();
        assert_eq!(
            write_stdout("help", &mut failed_stdout, &mut stderr),
            EXIT_USAGE_OR_IO
        );
        assert_eq!(stderr, b"error: cannot write command output\n");
        assert!(failed_stdout.flush().is_ok());

        let path = std::env::temp_dir().join(format!(
            "stack-cli-writer-failure-{}.stack",
            std::process::id()
        ));
        assert!(
            fs::write(
                &path,
                b"stack 1.0 diagram \"Fallback\" { theme neon node api \"API\" }",
            )
            .is_ok()
        );
        let mut failed_stderr = FailingWriter;
        let status = check_file(&path, &mut failed_stderr);
        let mut operational_stderr = Vec::new();
        let operational_status = check_file_with(&path, &mut operational_stderr, |_| {
            Err(OperationalError::InvalidIntermediateRepresentation {
                reason: "test failure",
            })
        });
        assert!(fs::remove_file(path).is_ok());
        assert_eq!(status, EXIT_USAGE_OR_IO);
        assert_eq!(operational_status, EXIT_USAGE_OR_IO);
        assert!(String::from_utf8_lossy(&operational_stderr).contains("error: cannot check"));
    }

    #[test]
    fn format_stream_failures_use_host_exit_status() {
        let source = b"stack 1.0 diagram \"Valid\" { node api \"API\" }";
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        assert_eq!(
            run(
                [OsString::from("fmt"), OsString::from("-")],
                &mut FailingReader,
                &mut stdout,
                &mut stderr,
            ),
            EXIT_USAGE_OR_IO
        );
        assert!(String::from_utf8_lossy(&stderr).contains("cannot read standard input"));

        stderr.clear();
        assert_eq!(
            format_stdin_with(FormatMode::Write, source, &mut stdout, &mut stderr, |_| {
                Err(OperationalError::InvalidIntermediateRepresentation {
                    reason: "test failure",
                })
            },),
            EXIT_USAGE_OR_IO
        );
        assert!(String::from_utf8_lossy(&stderr).contains("cannot format stdin"));

        let output = Engine::bundled().format(source);
        assert!(output.is_ok());
        let Ok(mut empty_output) = output else {
            return;
        };
        empty_output.formatted_source = None;
        empty_output.diagnostics.clear();
        stderr.clear();
        assert_eq!(
            format_stdin_with(FormatMode::Write, source, &mut stdout, &mut stderr, |_| Ok(
                empty_output
            ),),
            EXIT_USAGE_OR_IO
        );

        let syntax = b"stack 1.0 diagram \"Incomplete\" {";
        let mut failed_stderr = FailingWriter;
        assert_eq!(
            format_stdin_with(
                FormatMode::Write,
                syntax,
                &mut stdout,
                &mut failed_stderr,
                |source| Engine::bundled().format(source),
            ),
            EXIT_USAGE_OR_IO
        );

        stderr.clear();
        assert_eq!(
            format_stdin_with(
                FormatMode::Write,
                syntax,
                &mut stdout,
                &mut stderr,
                |source| Engine::bundled().format(source),
            ),
            EXIT_STACK_ERROR
        );

        let semantic = b"stack 1.0 diagram \"Invalid\" { node api \"A\" node api \"B\" }";
        stdout.clear();
        stderr.clear();
        assert_eq!(
            format_stdin_with(
                FormatMode::Write,
                semantic,
                &mut stdout,
                &mut stderr,
                |source| Engine::bundled().format(source),
            ),
            EXIT_STACK_ERROR
        );
        assert!(!stdout.is_empty());

        let mut failed_stdout = FailingWriter;
        stderr.clear();
        assert_eq!(
            format_stdin_with(
                FormatMode::Write,
                source,
                &mut failed_stdout,
                &mut stderr,
                |source| Engine::bundled().format(source),
            ),
            EXIT_USAGE_OR_IO
        );
    }

    #[test]
    fn format_file_failures_do_not_replace_the_source() {
        let path = std::env::temp_dir().join(format!(
            "stack-cli-format-failures-{}.stack",
            std::process::id()
        ));
        let source = b"stack 1.0 diagram \"Valid\"{node api \"API\"}";
        assert!(fs::write(&path, source).is_ok());

        let mut stderr = Vec::new();
        assert_eq!(
            format_file_with(
                FormatMode::Write,
                &path,
                &mut stderr,
                |_| {
                    Err(OperationalError::InvalidIntermediateRepresentation {
                        reason: "test failure",
                    })
                },
                atomic_replace,
            ),
            EXIT_USAGE_OR_IO
        );

        let syntax = b"stack 1.0 diagram \"Incomplete\" {";
        assert!(fs::write(&path, syntax).is_ok());
        let mut failed_stderr = FailingWriter;
        assert_eq!(
            format_file_with(
                FormatMode::Write,
                &path,
                &mut failed_stderr,
                |source| Engine::bundled().format(source),
                atomic_replace,
            ),
            EXIT_USAGE_OR_IO
        );
        assert_eq!(fs::read(&path).ok().as_deref(), Some(syntax.as_slice()));
        assert!(fs::write(&path, source).is_ok());

        let output = Engine::bundled().format(source);
        assert!(output.is_ok());
        let Ok(mut empty_output) = output else {
            return;
        };
        empty_output.formatted_source = None;
        empty_output.diagnostics.clear();
        stderr.clear();
        assert_eq!(
            format_file_with(
                FormatMode::Write,
                &path,
                &mut stderr,
                |_| Ok(empty_output),
                atomic_replace,
            ),
            EXIT_USAGE_OR_IO
        );

        stderr.clear();
        assert_eq!(
            format_file_with(
                FormatMode::Write,
                &path,
                &mut stderr,
                |source| Engine::bundled().format(source),
                |_, _| Err(io::Error::from(io::ErrorKind::PermissionDenied)),
            ),
            EXIT_USAGE_OR_IO
        );
        assert_eq!(fs::read(&path).ok().as_deref(), Some(source.as_slice()));
        assert!(fs::remove_file(path).is_ok());
    }

    #[test]
    fn atomic_replace_removes_its_temporary_file_after_rename_failure() {
        let root =
            std::env::temp_dir().join(format!("stack-cli-rename-failure-{}", std::process::id()));
        let target = root.join("target");
        assert!(fs::create_dir(&root).is_ok());
        assert!(fs::create_dir(&target).is_ok());
        assert!(fs::write(target.join("keep"), b"keep").is_ok());

        assert!(atomic_replace(&target, b"formatted").is_err());
        assert_eq!(
            fs::read_dir(&root).ok().map(|entries| entries.count()),
            Some(1)
        );
        assert!(target.join("keep").is_file());
        assert!(fs::remove_dir_all(root).is_ok());
    }

    #[test]
    fn temporary_file_creation_skips_collisions_and_is_bounded() {
        let parent =
            std::env::temp_dir().join(format!("stack-cli-temporary-files-{}", std::process::id()));
        assert!(fs::create_dir(&parent).is_ok());
        for attempt in 0..128_u8 {
            assert!(
                fs::write(
                    parent.join(format!(".stack-tmp-{}-{attempt}", std::process::id())),
                    b"collision",
                )
                .is_ok()
            );
        }
        let exhausted = create_temporary_file(&parent);
        assert_eq!(
            exhausted.err().map(|error| error.kind()),
            Some(io::ErrorKind::AlreadyExists)
        );
        assert!(fs::remove_dir_all(parent).is_ok());

        let missing =
            std::env::temp_dir().join(format!("stack-cli-missing-parent-{}", std::process::id()));
        assert_eq!(
            create_temporary_file(&missing)
                .err()
                .map(|error| error.kind()),
            Some(io::ErrorKind::NotFound)
        );
    }

    #[test]
    fn provider_pack_loader_rejects_untrusted_host_inputs() {
        let fixture = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("fixtures")
            .join("provider-pack");
        let pack = load_provider_pack(&fixture);
        assert_eq!(
            pack.ok().map(|pack| pack.manifest().provider.id.clone()),
            Some("example".to_owned())
        );

        assert!(safe_provider_asset_path("assets/icon.svg").is_ok());
        for path in [
            "icon.svg",
            "assets",
            "assets/nested/icon.svg",
            "assets/../icon.svg",
            "/assets/icon.svg",
            "assets/icon.png",
        ] {
            assert!(safe_provider_asset_path(path).is_err(), "accepted {path}");
        }

        let root =
            std::env::temp_dir().join(format!("stack-cli-provider-loader-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        assert!(load_provider_pack(&root).is_err());
        assert!(fs::write(&root, b"not a directory").is_ok());
        assert!(load_provider_pack(&root).is_err());
        assert!(fs::remove_file(&root).is_ok());
        assert!(fs::create_dir_all(root.join("assets")).is_ok());

        let manifest_path = root.join("manifest.json");
        assert!(fs::write(&manifest_path, b"{}").is_ok());
        assert!(load_provider_pack(&root).is_err());
        assert!(
            fs::write(
                &manifest_path,
                include_bytes!("../tests/fixtures/provider-pack/manifest.json")
            )
            .is_ok()
        );
        let asset_path = root.join("assets/storage.svg");
        assert!(fs::write(&asset_path, [0xff]).is_ok());
        assert!(load_provider_pack(&root).is_err());
        assert!(fs::write(&asset_path, b"<svg/>").is_ok());
        assert!(load_provider_pack(&root).is_err());

        let oversized = root.join("oversized");
        assert!(fs::write(&oversized, vec![b'x'; 5]).is_ok());
        assert!(read_bounded_regular_file(&oversized, 4).is_err());
        assert!(read_bounded_regular_file(&root, 4).is_err());
        assert_eq!(
            read_bounded_regular_file(&oversized, 5).ok(),
            Some(vec![b'x'; 5])
        );

        #[cfg(unix)]
        {
            use std::os::unix::fs::symlink;

            let linked = root.join("linked");
            assert!(symlink(&oversized, &linked).is_ok());
            assert!(read_bounded_regular_file(&linked, 5).is_err());
            assert!(load_provider_pack(&linked).is_err());
        }
        assert!(fs::remove_dir_all(root).is_ok());
    }

    #[test]
    fn provider_notice_text_is_stable_and_inert() {
        assert_eq!(
            render_provider_notices(&[]),
            "# Stack provider icon notices\n\nNo provider icons were embedded in this artifact.\n"
        );
        assert_eq!(notice_text("line\n<unsafe>\ttext"), "line  unsafe  text");
    }

    #[test]
    fn render_failures_do_not_emit_partial_artifacts() {
        let path = std::env::temp_dir().join(format!(
            "stack-cli-render-failures-{}.stack",
            std::process::id()
        ));
        let source = b"stack 1.0 diagram \"Valid\" { node api \"API\" }";
        assert!(fs::write(&path, source).is_ok());

        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        assert_eq!(
            render_file_with(
                &path,
                RenderDestination::Stdout,
                None,
                &mut stdout,
                &mut stderr,
                |_| {
                    Err(OperationalError::InvalidIntermediateRepresentation {
                        reason: "test failure",
                    })
                },
                atomic_write_output,
            ),
            EXIT_USAGE_OR_IO
        );
        assert!(stdout.is_empty());

        let output = Engine::bundled().render(source);
        assert!(output.is_ok());
        let Ok(mut empty_output) = output else {
            return;
        };
        empty_output.svg = None;
        empty_output.diagnostics.clear();
        stderr.clear();
        assert_eq!(
            render_file_with(
                &path,
                RenderDestination::Stdout,
                None,
                &mut stdout,
                &mut stderr,
                |_| Ok(empty_output),
                atomic_write_output,
            ),
            EXIT_USAGE_OR_IO
        );

        let mut failed_stdout = FailingWriter;
        stderr.clear();
        assert_eq!(
            render_file_with(
                &path,
                RenderDestination::Stdout,
                None,
                &mut failed_stdout,
                &mut stderr,
                |source| Engine::bundled().render(source),
                atomic_write_output,
            ),
            EXIT_USAGE_OR_IO
        );

        let output_path = path.with_extension("svg");
        stderr.clear();
        assert_eq!(
            render_file_with(
                &path,
                RenderDestination::File(output_path.clone()),
                None,
                &mut stdout,
                &mut stderr,
                |source| Engine::bundled().render(source),
                |_, _| Err(io::Error::from(io::ErrorKind::PermissionDenied)),
            ),
            EXIT_USAGE_OR_IO
        );
        assert!(!output_path.exists());

        let syntax = b"stack 1.0 diagram \"Incomplete\" {";
        assert!(fs::write(&path, syntax).is_ok());
        let mut failed_stderr = FailingWriter;
        assert_eq!(
            render_file_with(
                &path,
                RenderDestination::Stdout,
                None,
                &mut stdout,
                &mut failed_stderr,
                |source| Engine::bundled().render(source),
                atomic_write_output,
            ),
            EXIT_USAGE_OR_IO
        );
        assert!(stdout.is_empty());
        assert!(fs::remove_file(path).is_ok());
    }
}
