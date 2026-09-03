//! Host boundary and command contract for the native Stack CLI.

#![forbid(unsafe_code)]

use std::ffi::{OsStr, OsString};
use std::fmt::Write as _;
use std::fs;
use std::io::{self, Write};
use std::path::Path;

use stack_engine::{CheckOutput, Diagnostic, Engine, OperationalError, Severity};

/// Exit status used when a command completes without Stack error diagnostics.
pub const EXIT_SUCCESS: u8 = 0;
/// Exit status used when Stack source contains at least one error diagnostic.
pub const EXIT_STACK_ERROR: u8 = 1;
/// Exit status used for argument, host I/O, or engine operational failures.
pub const EXIT_USAGE_OR_IO: u8 = 2;

const GENERAL_HELP: &str = "Stack diagram toolchain\n\nUsage:\n  stack check <FILE>\n  stack --help\n  stack --version\n\nCommands:\n  check    Validate a Stack source file without modifying it\n";
const CHECK_HELP: &str =
    "Validate a Stack source file without modifying it\n\nUsage:\n  stack check <FILE>\n";

/// Runs the CLI with explicit streams and returns its process exit status.
pub fn run(
    arguments: impl IntoIterator<Item = OsString>,
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

    argument_error(
        &format!("unknown command '{}'", command.to_string_lossy()),
        stderr,
    )
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
    let has_errors = output
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.severity == Severity::Error);
    let rendered = render_diagnostics(path, &output.diagnostics);
    if !rendered.is_empty() && stderr.write_all(rendered.as_bytes()).is_err() {
        return EXIT_USAGE_OR_IO;
    }

    if has_errors {
        EXIT_STACK_ERROR
    } else {
        EXIT_SUCCESS
    }
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

    impl Write for FailingWriter {
        fn write(&mut self, _buffer: &[u8]) -> io::Result<usize> {
            Err(io::Error::other("test writer failure"))
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn help_version_and_argument_errors_have_stable_streams() {
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        assert_eq!(
            run([OsString::from("--version")], &mut stdout, &mut stderr),
            EXIT_SUCCESS
        );
        assert_eq!(stdout, b"stack 0.1.0\n");
        assert!(stderr.is_empty());

        stdout.clear();
        assert_eq!(
            run([OsString::from("--help")], &mut stdout, &mut stderr),
            EXIT_SUCCESS
        );
        assert_eq!(stdout, GENERAL_HELP.as_bytes());

        stdout.clear();
        assert_eq!(run([], &mut stdout, &mut stderr), EXIT_USAGE_OR_IO);
        assert!(stdout.is_empty());
        assert!(String::from_utf8_lossy(&stderr).contains("error: missing command"));
    }

    #[test]
    fn command_aliases_and_invalid_arguments_are_stable() {
        for alias in ["-h", "-V"] {
            let mut stdout = Vec::new();
            let mut stderr = Vec::new();
            assert_eq!(
                run([OsString::from(alias)], &mut stdout, &mut stderr),
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
            vec![
                OsString::from("check"),
                OsString::from("file.stack"),
                OsString::from("extra"),
            ],
        ] {
            let mut stdout = Vec::new();
            let mut stderr = Vec::new();
            assert_eq!(run(arguments, &mut stdout, &mut stderr), EXIT_USAGE_OR_IO);
            assert!(stdout.is_empty());
            assert!(String::from_utf8_lossy(&stderr).starts_with("error:"));
        }

        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        assert_eq!(
            run(
                [OsString::from("check"), OsString::from("-h")],
                &mut stdout,
                &mut stderr,
            ),
            EXIT_SUCCESS
        );
        assert_eq!(stdout, CHECK_HELP.as_bytes());
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
            help: None,
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
            "arch.stack:2:3: error[STKTEST]: primary\narch.stack:4:5: note: related\n"
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
}
