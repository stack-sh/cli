use std::env;
use std::error::Error;
use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
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
        .output()?)
}

fn assert_unchanged(path: &Path, expected: &[u8]) -> Result<(), Box<dyn Error>> {
    assert_eq!(fs::read(path)?, expected);
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
        "{}:1:1: error[STK1001]: Input is not valid UTF-8.\n",
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
fn help_and_version_are_stdout_only() -> Result<(), Box<dyn Error>> {
    let help = stack(["--help"])?;
    assert_eq!(help.status.code(), Some(0));
    assert!(help.stderr.is_empty());
    assert!(String::from_utf8(help.stdout)?.contains("stack check <FILE>"));

    let version = stack(["--version"])?;
    assert_eq!(version.status.code(), Some(0));
    assert_eq!(version.stdout, b"stack 0.1.0\n");
    assert!(version.stderr.is_empty());
    Ok(())
}
