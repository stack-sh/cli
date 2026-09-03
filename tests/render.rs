use std::env;
use std::error::Error;
use std::ffi::OsStr;
use std::fs;
use std::path::PathBuf;
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};

use stack_engine::Engine;

static CASE_ID: AtomicU64 = AtomicU64::new(0);

struct TestDirectory {
    path: PathBuf,
}

impl TestDirectory {
    fn new(label: &str) -> Result<Self, Box<dyn Error>> {
        let case_id = CASE_ID.fetch_add(1, Ordering::Relaxed);
        let path = env::temp_dir().join(format!(
            "stack-cli-render-{}-{label}-{case_id}",
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

fn engine_svg(source: &[u8]) -> Result<String, Box<dyn Error>> {
    let output = Engine::bundled().render(source)?;
    output
        .svg
        .ok_or_else(|| format!("engine returned no SVG: {:?}", output.diagnostics).into())
}

#[test]
fn stdout_is_exactly_the_engine_svg() -> Result<(), Box<dyn Error>> {
    let directory = TestDirectory::new("stdout")?;
    let source = b"stack 1.0 diagram \"API\" { node web \"Web\" edge web -> api \"HTTPS\" node api \"API\" }";
    let path = directory.file("arch.stack", source)?;
    let expected = engine_svg(source)?;

    let output = stack([OsStr::new("render"), path.as_os_str()])?;

    assert_eq!(output.status.code(), Some(0));
    assert_eq!(output.stdout, expected.as_bytes());
    assert!(output.stderr.is_empty());
    assert_eq!(fs::read(&path)?, source);
    Ok(())
}

#[test]
fn warnings_stay_on_stderr_without_suppressing_svg() -> Result<(), Box<dyn Error>> {
    let directory = TestDirectory::new("warning")?;
    let source =
        b"stack 1.0 diagram \"Fallback\" { theme neon node api \"API\" { icon \"missing\" } }";
    let path = directory.file("warning.stack", source)?;
    let expected = engine_svg(source)?;

    let output = stack([OsStr::new("render"), path.as_os_str()])?;
    let stderr = String::from_utf8(output.stderr)?;

    assert_eq!(output.status.code(), Some(0));
    assert_eq!(output.stdout, expected.as_bytes());
    assert!(stderr.contains("warning[STK6001]"));
    assert!(stderr.contains("warning[STK5001]"));
    Ok(())
}

#[test]
fn output_file_is_replaced_atomically_with_exact_engine_bytes() -> Result<(), Box<dyn Error>> {
    let directory = TestDirectory::new("output")?;
    let source = b"stack 1.0 diagram \"Output\" { node api \"API\" }";
    let input = directory.file("arch.stack", source)?;
    let output_path = directory.file("arch.svg", b"sentinel")?;
    let expected = engine_svg(source)?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&output_path, fs::Permissions::from_mode(0o640))?;
    }

    let output = stack([
        OsStr::new("render"),
        input.as_os_str(),
        OsStr::new("-o"),
        output_path.as_os_str(),
    ])?;

    assert_eq!(output.status.code(), Some(0));
    assert!(output.stdout.is_empty());
    assert!(output.stderr.is_empty());
    assert_eq!(fs::read(&output_path)?, expected.as_bytes());
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            fs::metadata(&output_path)?.permissions().mode() & 0o777,
            0o640
        );
    }
    assert_eq!(fs::read_dir(&directory.path)?.count(), 2);

    fs::remove_file(&output_path)?;
    let created = stack([
        OsStr::new("render"),
        input.as_os_str(),
        OsStr::new("-o"),
        output_path.as_os_str(),
    ])?;
    assert_eq!(created.status.code(), Some(0));
    assert!(created.stdout.is_empty());
    assert!(created.stderr.is_empty());
    assert_eq!(fs::read(&output_path)?, expected.as_bytes());
    assert_eq!(fs::read_dir(&directory.path)?.count(), 2);
    Ok(())
}

#[test]
fn compiler_errors_never_create_or_replace_output() -> Result<(), Box<dyn Error>> {
    let directory = TestDirectory::new("source-error")?;
    let source = b"stack 1.0 diagram \"Incomplete\" {";
    let input = directory.file("invalid.stack", source)?;
    let output_path = directory.file("existing.svg", b"sentinel")?;

    let stdout = stack([OsStr::new("render"), input.as_os_str()])?;
    assert_eq!(stdout.status.code(), Some(1));
    assert!(stdout.stdout.is_empty());
    assert!(String::from_utf8(stdout.stderr)?.contains("error[STK2003]"));

    let file = stack([
        OsStr::new("render"),
        input.as_os_str(),
        OsStr::new("-o"),
        output_path.as_os_str(),
    ])?;
    assert_eq!(file.status.code(), Some(1));
    assert!(file.stdout.is_empty());
    assert!(String::from_utf8(file.stderr)?.contains("error[STK2003]"));
    assert_eq!(fs::read(&output_path)?, b"sentinel");
    assert_eq!(fs::read_dir(&directory.path)?.count(), 2);
    Ok(())
}

#[test]
fn missing_input_and_output_parent_are_host_failures() -> Result<(), Box<dyn Error>> {
    let directory = TestDirectory::new("host-failure")?;
    let missing = directory.path.join("missing.stack");
    let missing_output = stack([OsStr::new("render"), missing.as_os_str()])?;
    assert_eq!(missing_output.status.code(), Some(2));
    assert!(missing_output.stdout.is_empty());
    assert!(String::from_utf8(missing_output.stderr)?.contains("file not found"));

    let source = b"stack 1.0 diagram \"Output\" { node api \"API\" }";
    let input = directory.file("arch.stack", source)?;
    let output_path = directory.path.join("missing").join("arch.svg");
    let output = stack([
        OsStr::new("render"),
        input.as_os_str(),
        OsStr::new("-o"),
        output_path.as_os_str(),
    ])?;
    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
    assert!(String::from_utf8(output.stderr)?.contains("cannot write"));
    assert!(!output_path.exists());
    Ok(())
}

#[cfg(unix)]
#[test]
fn atomic_output_failure_leaves_no_partial_file() -> Result<(), Box<dyn Error>> {
    use std::os::unix::fs::PermissionsExt;

    let directory = TestDirectory::new("atomic-failure")?;
    let source = b"stack 1.0 diagram \"Output\" { node api \"API\" }";
    let input = directory.file("arch.stack", source)?;
    let output_path = directory.path.join("arch.svg");
    let original_permissions = fs::metadata(&directory.path)?.permissions();
    fs::set_permissions(&directory.path, fs::Permissions::from_mode(0o555))?;

    let output = stack([
        OsStr::new("render"),
        input.as_os_str(),
        OsStr::new("-o"),
        output_path.as_os_str(),
    ]);
    let restored = fs::set_permissions(&directory.path, original_permissions);
    restored?;
    let output = output?;

    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
    assert!(String::from_utf8(output.stderr)?.contains("cannot write"));
    assert!(!output_path.exists());
    assert_eq!(fs::read_dir(&directory.path)?.count(), 1);
    Ok(())
}
