use std::env;
use std::error::Error;
use std::ffi::OsStr;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};

static CASE_ID: AtomicU64 = AtomicU64::new(0);

struct TestDirectory {
    path: PathBuf,
}

impl TestDirectory {
    fn new(label: &str) -> Result<Self, Box<dyn Error>> {
        let case_id = CASE_ID.fetch_add(1, Ordering::Relaxed);
        let path = env::temp_dir().join(format!(
            "stack-cli-{}-{label}-{case_id}",
            std::process::id()
        ));
        fs::create_dir(&path)?;
        Ok(Self { path })
    }

    fn file(&self, name: &str, bytes: &[u8]) -> Result<PathBuf, Box<dyn Error>> {
        let path = self.path.join(name);
        fs::write(&path, bytes)?;
        Ok(path)
    }
}

impl Drop for TestDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

fn stack(arguments: impl IntoIterator<Item = impl AsRef<OsStr>>) -> Result<Output, Box<dyn Error>> {
    Ok(Command::new(env!("CARGO_BIN_EXE_stack"))
        .args(arguments)
        .env(
            "XDG_CONFIG_HOME",
            env::temp_dir().join(format!("stack-cli-empty-config-{}", std::process::id())),
        )
        .output()?)
}

fn stack_with_input(
    arguments: impl IntoIterator<Item = impl AsRef<OsStr>>,
    input: &[u8],
) -> Result<Output, Box<dyn Error>> {
    let mut child = Command::new(env!("CARGO_BIN_EXE_stack"))
        .args(arguments)
        .env(
            "XDG_CONFIG_HOME",
            env::temp_dir().join(format!("stack-cli-empty-config-{}", std::process::id())),
        )
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let Some(mut stdin) = child.stdin.take() else {
        return Err("missing child standard input".into());
    };
    stdin.write_all(input)?;
    drop(stdin);
    Ok(child.wait_with_output()?)
}

fn assert_unchanged(path: &Path, expected: &[u8]) -> Result<(), Box<dyn Error>> {
    assert_eq!(fs::read(path)?, expected);
    Ok(())
}

fn assert_stdout_only(arguments: &[&str], expected: &[u8]) -> Result<(), Box<dyn Error>> {
    let output = stack(arguments.iter().copied())?;
    assert_eq!(output.status.code(), Some(0), "arguments: {arguments:?}");
    assert_eq!(output.stdout, expected, "arguments: {arguments:?}");
    assert!(output.stderr.is_empty(), "arguments: {arguments:?}");
    Ok(())
}

#[test]
fn valid_source_is_silent_and_unchanged() -> Result<(), Box<dyn Error>> {
    let directory = TestDirectory::new("valid")?;
    let source = b"stack 1.0 diagram \"Valid\" { node api \"API\" }";
    let path = directory.file("valid.stack", source)?;

    let output = stack([OsStr::new("check"), path.as_os_str()])?;

    assert_eq!(output.status.code(), Some(0));
    assert!(output.stdout.is_empty());
    assert!(output.stderr.is_empty());
    assert_unchanged(&path, source)
}

#[test]
fn warning_source_exits_zero_and_writes_only_stderr() -> Result<(), Box<dyn Error>> {
    let directory = TestDirectory::new("warning")?;
    let source = b"stack 1.0 diagram \"Fallback\" { theme neon node api \"API\" }";
    let path = directory.file("warning.stack", source)?;
    let warning_column = source
        .windows(b"neon".len())
        .position(|window| window == b"neon")
        .ok_or("missing warning position")?
        + 1;

    let output = stack([OsStr::new("check"), path.as_os_str()])?;
    let expected = format!(
        "{}:1:{warning_column}: warning[STK6001]: theme 'neon' is unavailable; default theme was used\n  help: Install the requested theme or select an available theme.\n",
        path.display()
    );

    assert_eq!(output.status.code(), Some(0));
    assert!(output.stdout.is_empty());
    assert_eq!(String::from_utf8(output.stderr)?, expected);
    assert_unchanged(&path, source)
}

#[test]
fn invalid_utf8_exits_one_and_writes_only_stderr() -> Result<(), Box<dyn Error>> {
    let directory = TestDirectory::new("invalid-utf8")?;
    let source = [0xff];
    let path = directory.file("invalid.stack", &source)?;

    let output = stack([OsStr::new("check"), path.as_os_str()])?;
    let expected = format!(
        "{}:1:1: error[STK1001]: Input is not valid UTF-8.\n  help: Save the source as UTF-8 and replace the invalid byte sequence.\n",
        path.display()
    );

    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    assert_eq!(String::from_utf8(output.stderr)?, expected);
    assert_unchanged(&path, &source)
}

#[test]
fn missing_file_exits_two_with_a_stable_host_error() -> Result<(), Box<dyn Error>> {
    let directory = TestDirectory::new("missing")?;
    let path = directory.path.join("missing.stack");

    let output = stack([OsStr::new("check"), path.as_os_str()])?;
    let expected = format!("error: cannot read '{}': file not found\n", path.display());

    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
    assert_eq!(String::from_utf8(output.stderr)?, expected);
    assert!(!path.exists());
    Ok(())
}

#[test]
fn help_snapshots_and_aliases_are_stdout_only() -> Result<(), Box<dyn Error>> {
    let cases: &[(&[&str], &[u8])] = &[
        (&["--help"], include_bytes!("snapshots/help.txt")),
        (&["-h"], include_bytes!("snapshots/help.txt")),
        (&["help"], include_bytes!("snapshots/help.txt")),
        (
            &["check", "--help"],
            include_bytes!("snapshots/check-help.txt"),
        ),
        (
            &["help", "check"],
            include_bytes!("snapshots/check-help.txt"),
        ),
        (&["fmt", "--help"], include_bytes!("snapshots/fmt-help.txt")),
        (&["help", "fmt"], include_bytes!("snapshots/fmt-help.txt")),
        (
            &["render", "--help"],
            include_bytes!("snapshots/render-help.txt"),
        ),
        (
            &["help", "render"],
            include_bytes!("snapshots/render-help.txt"),
        ),
        (
            &["icons", "--help"],
            include_bytes!("snapshots/icons-help.txt"),
        ),
        (
            &["icons", "help"],
            include_bytes!("snapshots/icons-help.txt"),
        ),
        (
            &["help", "icons"],
            include_bytes!("snapshots/icons-help.txt"),
        ),
        (
            &["icons", "list", "--help"],
            include_bytes!("snapshots/icons-list-help.txt"),
        ),
        (
            &["icons", "help", "list"],
            include_bytes!("snapshots/icons-list-help.txt"),
        ),
        (
            &["help", "icons", "list"],
            include_bytes!("snapshots/icons-list-help.txt"),
        ),
        (
            &["icons", "import", "--help"],
            include_bytes!("snapshots/icons-import-help.txt"),
        ),
        (
            &["icons", "help", "import"],
            include_bytes!("snapshots/icons-import-help.txt"),
        ),
        (
            &["help", "icons", "import"],
            include_bytes!("snapshots/icons-import-help.txt"),
        ),
        (&["help", "help"], include_bytes!("snapshots/help-help.txt")),
        (
            &["help", "--help"],
            include_bytes!("snapshots/help-help.txt"),
        ),
        (
            &["version", "--help"],
            include_bytes!("snapshots/version-help.txt"),
        ),
        (
            &["help", "version"],
            include_bytes!("snapshots/version-help.txt"),
        ),
    ];

    for (arguments, expected) in cases {
        assert_stdout_only(arguments, expected)?;
    }

    let expected_version = format!("stack {}\n", env!("CARGO_PKG_VERSION"));
    for arguments in [["version"], ["-v"], ["-V"], ["--version"]] {
        assert_stdout_only(&arguments, expected_version.as_bytes())?;
    }
    Ok(())
}

#[test]
fn command_typos_are_actionable_and_stderr_only() -> Result<(), Box<dyn Error>> {
    let cases: &[(&[&str], &str)] = &[
        (
            &["chekc"],
            "error: unknown command 'chekc'\n\nDid you mean 'check'?\n\nFor more information, try 'stack help'.\n",
        ),
        (
            &["icons", "lst"],
            "error: unknown command for 'stack icons': 'lst'\n\nDid you mean 'list'?\n\nFor more information, try 'stack help icons'.\n",
        ),
        (
            &["help", "rennder"],
            "error: unknown command for 'stack help': 'rennder'\n\nDid you mean 'render'?\n\nFor more information, try 'stack help'.\n",
        ),
        (
            &["definitely-unknown"],
            "error: unknown command 'definitely-unknown'\n\nFor more information, try 'stack help'.\n",
        ),
    ];

    for (arguments, expected) in cases {
        let output = stack(arguments.iter().copied())?;
        assert_eq!(output.status.code(), Some(2), "arguments: {arguments:?}");
        assert!(output.stdout.is_empty(), "arguments: {arguments:?}");
        assert_eq!(
            output.stderr,
            expected.as_bytes(),
            "arguments: {arguments:?}"
        );
    }
    Ok(())
}

#[test]
fn provider_catalog_commands_are_available_from_the_binary() -> Result<(), Box<dyn Error>> {
    let listed = stack(["icons", "list", "simple-icons", "linear"])?;
    assert_eq!(listed.status.code(), Some(0));
    assert!(listed.stderr.is_empty());
    let listing = String::from_utf8(listed.stdout)?;
    assert!(listing.contains("simple-icons:linear"));
    assert!(listing.contains("Linear"));
    assert!(!listing.contains("<svg"));

    let unknown_list = stack(["icons", "list", "unknown"])?;
    assert_eq!(unknown_list.status.code(), Some(2));
    assert!(unknown_list.stdout.is_empty());
    assert!(String::from_utf8(unknown_list.stderr)?.contains("unknown provider 'unknown'"));

    let directory = TestDirectory::new("provider-import")?;
    let output = directory.path.join("pack");
    let unknown_import = stack([
        OsStr::new("icons"),
        OsStr::new("import"),
        OsStr::new("unknown"),
        OsStr::new("missing.zip"),
        OsStr::new("--accept-terms"),
        OsStr::new("-o"),
        output.as_os_str(),
    ])?;
    assert_eq!(unknown_import.status.code(), Some(2));
    assert!(unknown_import.stdout.is_empty());
    assert!(String::from_utf8(unknown_import.stderr)?.contains("unknown provider 'unknown'"));
    assert!(!output.exists());
    Ok(())
}

#[test]
fn format_replaces_only_changed_files_and_is_idempotent() -> Result<(), Box<dyn Error>> {
    let directory = TestDirectory::new("format-in-place")?;
    let source = b"// kept\nstack 1 . 0 diagram \"Valid\"{node api \"API\"}";
    let expected = b"// kept\nstack 1.0\n\ndiagram \"Valid\" {\n  node api \"API\"\n}\n";
    let path = directory.file("format.stack", source)?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o640))?;
    }

    let first = stack([OsStr::new("fmt"), path.as_os_str()])?;
    assert_eq!(first.status.code(), Some(0));
    assert!(first.stdout.is_empty());
    assert!(first.stderr.is_empty());
    assert_eq!(fs::read(&path)?, expected);
    #[cfg(unix)]
    {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        let metadata = fs::metadata(&path)?;
        assert_eq!(metadata.permissions().mode() & 0o777, 0o640);
        assert_ne!(metadata.ino(), 0);
    }

    #[cfg(unix)]
    let formatted_inode = {
        use std::os::unix::fs::MetadataExt;
        fs::metadata(&path)?.ino()
    };

    let second = stack([OsStr::new("fmt"), path.as_os_str()])?;
    assert_eq!(second.status.code(), Some(0));
    assert!(second.stdout.is_empty());
    assert!(second.stderr.is_empty());
    assert_eq!(fs::read(&path)?, expected);
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        assert_eq!(fs::metadata(&path)?.ino(), formatted_inode);
    }
    assert_eq!(fs::read_dir(&directory.path)?.count(), 1);
    Ok(())
}

#[test]
fn format_check_reports_differences_without_writing() -> Result<(), Box<dyn Error>> {
    let directory = TestDirectory::new("format-check")?;
    let unformatted = b"stack 1.0 diagram \"Check\"{node api \"API\"}";
    let formatted = b"stack 1.0\n\ndiagram \"Check\" {\n  node api \"API\"\n}\n";
    let path = directory.file("check.stack", unformatted)?;

    let different = stack([OsStr::new("fmt"), OsStr::new("--check"), path.as_os_str()])?;
    assert_eq!(different.status.code(), Some(1));
    assert!(different.stdout.is_empty());
    assert!(different.stderr.is_empty());
    assert_unchanged(&path, unformatted)?;

    fs::write(&path, formatted)?;
    let clean = stack([OsStr::new("fmt"), OsStr::new("--check"), path.as_os_str()])?;
    assert_eq!(clean.status.code(), Some(0));
    assert!(clean.stdout.is_empty());
    assert!(clean.stderr.is_empty());
    assert_unchanged(&path, formatted)
}

#[test]
fn semantic_errors_are_formatted_but_exit_one() -> Result<(), Box<dyn Error>> {
    let directory = TestDirectory::new("format-semantic-error")?;
    let source = b"// preserved\nstack 1.0 diagram \"Invalid\"{node api \"First\" node api \"Second\" edge api->missing}";
    let path = directory.file("semantic.stack", source)?;

    let checked = stack([OsStr::new("fmt"), OsStr::new("--check"), path.as_os_str()])?;
    assert_eq!(checked.status.code(), Some(1));
    assert!(checked.stdout.is_empty());
    assert!(String::from_utf8(checked.stderr)?.contains("error[STK3002]"));
    assert_unchanged(&path, source)?;

    let output = stack([OsStr::new("fmt"), path.as_os_str()])?;
    let formatted = fs::read_to_string(&path)?;

    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    assert!(String::from_utf8(output.stderr)?.contains("error[STK3002]"));
    assert!(formatted.starts_with("// preserved\nstack 1.0\n"));
    assert!(formatted.contains("node api \"First\"\n"));
    assert!(formatted.contains("edge api -> missing\n"));
    assert_ne!(formatted.as_bytes(), source);
    Ok(())
}

#[test]
fn syntax_errors_never_modify_files() -> Result<(), Box<dyn Error>> {
    let directory = TestDirectory::new("format-syntax-error")?;
    let source = b"stack 1.0 diagram \"Incomplete\" {";
    let path = directory.file("syntax.stack", source)?;

    for arguments in [
        vec![OsStr::new("fmt"), path.as_os_str()],
        vec![OsStr::new("fmt"), OsStr::new("--check"), path.as_os_str()],
    ] {
        let output = stack(arguments)?;
        assert_eq!(output.status.code(), Some(1));
        assert!(output.stdout.is_empty());
        assert!(String::from_utf8(output.stderr)?.contains("error[STK2003]"));
        assert_unchanged(&path, source)?;
    }
    assert_eq!(fs::read_dir(&directory.path)?.count(), 1);
    Ok(())
}

#[test]
fn stdin_formatting_has_explicit_stdout_and_check_semantics() -> Result<(), Box<dyn Error>> {
    let source = b"stack 1.0 diagram \"Stdin\"{node api \"API\"}";
    let expected = b"stack 1.0\n\ndiagram \"Stdin\" {\n  node api \"API\"\n}\n";

    let formatted = stack_with_input(["fmt", "-"], source)?;
    assert_eq!(formatted.status.code(), Some(0));
    assert_eq!(formatted.stdout, expected);
    assert!(formatted.stderr.is_empty());

    let check_clean = stack_with_input(["fmt", "--check", "-"], expected)?;
    assert_eq!(check_clean.status.code(), Some(0));
    assert!(check_clean.stdout.is_empty());
    assert!(check_clean.stderr.is_empty());

    let check_different = stack_with_input(["fmt", "--check", "-"], source)?;
    assert_eq!(check_different.status.code(), Some(1));
    assert!(check_different.stdout.is_empty());
    assert!(check_different.stderr.is_empty());
    Ok(())
}

#[test]
fn format_missing_file_is_a_host_failure() -> Result<(), Box<dyn Error>> {
    let directory = TestDirectory::new("format-missing")?;
    let path = directory.path.join("missing.stack");

    let output = stack([OsStr::new("fmt"), path.as_os_str()])?;
    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
    assert_eq!(
        String::from_utf8(output.stderr)?,
        format!("error: cannot read '{}': file not found\n", path.display())
    );
    assert!(!path.exists());
    Ok(())
}

#[cfg(unix)]
#[test]
fn atomic_io_failure_keeps_the_original_file() -> Result<(), Box<dyn Error>> {
    use std::os::unix::fs::PermissionsExt;

    let directory = TestDirectory::new("format-atomic-failure")?;
    let source = b"stack 1.0 diagram \"Valid\"{node api \"API\"}";
    let path = directory.file("readonly.stack", source)?;
    let original_permissions = fs::metadata(&directory.path)?.permissions();
    fs::set_permissions(&directory.path, fs::Permissions::from_mode(0o555))?;

    let output = stack([OsStr::new("fmt"), path.as_os_str()]);
    let restored = fs::set_permissions(&directory.path, original_permissions);
    restored?;
    let output = output?;

    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
    assert!(String::from_utf8(output.stderr)?.contains("cannot replace"));
    assert_unchanged(&path, source)?;
    assert_eq!(fs::read_dir(&directory.path)?.count(), 1);
    Ok(())
}
