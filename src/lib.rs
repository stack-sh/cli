//! Host boundary and command contract for the native Stack CLI.

#![forbid(unsafe_code)]

use std::ffi::{OsStr, OsString};
use std::fmt::Write as _;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

use stack_engine::{
    CheckOutput, Diagnostic, Engine, FormatOutput, OperationalError, RenderOutput, Severity,
};

mod provider;

/// Exit status used when a command completes without Stack error diagnostics.
pub const EXIT_SUCCESS: u8 = 0;
/// Exit status used when Stack source contains at least one error diagnostic.
pub const EXIT_STACK_ERROR: u8 = 1;
/// Exit status used for argument, host I/O, or engine operational failures.
pub const EXIT_USAGE_OR_IO: u8 = 2;

const GENERAL_HELP: &str = "Stack diagram toolchain\n\nUsage:\n  stack check <FILE>\n  stack fmt [--check] <FILE|->\n  stack render <FILE> [-o <OUTPUT>]\n  stack icons import <PROVIDER> <ARCHIVE> --accept-terms -o <DIRECTORY>\n  stack --help\n  stack --version\n\nCommands:\n  check     Validate a Stack source file without modifying it\n  fmt       Format a file in place or read from standard input\n  render    Render standalone SVG to standard output or a file\n  icons     Import local provider icon archives\n";
const CHECK_HELP: &str =
    "Validate a Stack source file without modifying it\n\nUsage:\n  stack check <FILE>\n";
const FORMAT_HELP: &str = "Format Stack source canonically\n\nUsage:\n  stack fmt <FILE>\n  stack fmt --check <FILE>\n  stack fmt -\n\nArguments:\n  <FILE>    Format the file atomically in place\n  -         Read from standard input and write to standard output\n\nOptions:\n  --check   Report whether formatting is required without writing output\n";
const RENDER_HELP: &str = "Render Stack source as standalone SVG\n\nUsage:\n  stack render <FILE>\n  stack render <FILE> -o <OUTPUT>\n\nArguments:\n  <FILE>      Read Stack source bytes from this file\n\nOptions:\n  -o <OUTPUT> Write SVG atomically instead of using standard output\n";
const ICONS_HELP: &str = "Manage local provider icon packs\n\nUsage:\n  stack icons import <PROVIDER> <ARCHIVE> --accept-terms -o <DIRECTORY>\n\nProviders:\n  aws       AWS Architecture Icons release 2026-07-31\n  gcp       Google Cloud core product icons from the May 2026 guide\n  azure     Azure Public Service Icons V24\n";
const ICONS_IMPORT_HELP: &str = "Import an audited official provider icon archive locally\n\nUsage:\n  stack icons import <PROVIDER> <ARCHIVE> --accept-terms -o <DIRECTORY>\n\nArguments:\n  <PROVIDER>   aws, gcp, or azure\n  <ARCHIVE>    Local official ZIP archive; Stack performs no download or upload\n\nOptions:\n  --accept-terms  Confirm that you reviewed and accept the provider terms\n  -o <DIRECTORY> Create a new local pack directory atomically\n";

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
        return run_icons(arguments, stdout, stderr);
    }

    argument_error(
        &format!("unknown command '{}'", command.to_string_lossy()),
        stderr,
    )
}

fn run_icons(
    mut arguments: impl Iterator<Item = OsString>,
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

fn run_icons_import(
    mut arguments: impl Iterator<Item = OsString>,
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
    let provider_name = provider.to_string_lossy();
    match provider::import_provider_pack(&provider_name, Path::new(&archive), &output) {
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

    let destination = match arguments.next() {
        None => RenderDestination::Stdout,
        Some(option) if option == OsStr::new("-o") => {
            let Some(output) = arguments.next() else {
                return argument_error("missing output file after '-o'", stderr);
            };
            if let Some(extra) = arguments.next() {
                return argument_error(
                    &format!("unexpected argument '{}'", extra.to_string_lossy()),
                    stderr,
                );
            }
            if output == source {
                return argument_error("input and output files must be different", stderr);
            }
            RenderDestination::File(PathBuf::from(output))
        }
        Some(extra) => {
            return argument_error(
                &format!("unexpected argument '{}'", extra.to_string_lossy()),
                stderr,
            );
        }
    };

    render_file(Path::new(&source), destination, stdout, stderr)
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
    stdout: &mut dyn Write,
    stderr: &mut dyn Write,
) -> u8 {
    render_file_with(
        path,
        destination,
        stdout,
        stderr,
        |source| Engine::bundled().render(source),
        atomic_write_output,
    )
}

fn render_file_with(
    path: &Path,
    destination: RenderDestination,
    stdout: &mut dyn Write,
    stderr: &mut dyn Write,
    render: impl FnOnce(&[u8]) -> Result<RenderOutput, OperationalError>,
    write_output: impl FnOnce(&Path, &[u8]) -> io::Result<()>,
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
    EXIT_SUCCESS
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
        assert_eq!(stdout, b"stack 0.1.0\n");
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
                OsString::from("file.stack"),
            ],
            vec![OsString::from("icons")],
            vec![OsString::from("icons"), OsString::from("unknown")],
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

        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
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
