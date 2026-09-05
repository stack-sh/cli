use std::env;
use std::error::Error;
use std::ffi::OsStr;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use serde_json::{Value, json};

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

fn stack_in(
    directory: &Path,
    arguments: impl IntoIterator<Item = impl AsRef<OsStr>>,
) -> Result<Output, Box<dyn Error>> {
    Ok(Command::new(env!("CARGO_BIN_EXE_stack"))
        .args(arguments)
        .current_dir(directory)
        .env("XDG_CONFIG_HOME", directory.join(".config"))
        .output()?)
}

fn stack_with_config_environment(
    xdg_root: Option<&Path>,
    home_root: Option<&Path>,
    arguments: impl IntoIterator<Item = impl AsRef<OsStr>>,
) -> Result<Output, Box<dyn Error>> {
    let mut command = Command::new(env!("CARGO_BIN_EXE_stack"));
    command
        .args(arguments)
        .env_remove("XDG_CONFIG_HOME")
        .env_remove("HOME");
    if let Some(root) = xdg_root {
        command.env("XDG_CONFIG_HOME", root);
    }
    if let Some(root) = home_root {
        command.env("HOME", root);
    }
    Ok(command.output()?)
}

fn create_provider_store(root: &Path) -> Result<(), Box<dyn Error>> {
    let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/provider-pack");
    let pack = root.join("simple-icons");
    fs::create_dir_all(pack.join("assets"))?;
    let manifest = fs::read_to_string(fixture.join("manifest.json"))?
        .replace("\"example\"", "\"simple-icons\"")
        .replace("example:storage", "simple-icons:storage")
        .replace("Example Cloud", "Simple Icons Fixture");
    fs::write(pack.join("manifest.json"), manifest)?;
    fs::copy(
        fixture.join("assets/storage.svg"),
        pack.join("assets/storage.svg"),
    )?;
    Ok(())
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

fn lsp_frame(message: &Value) -> Result<Vec<u8>, Box<dyn Error>> {
    let payload = serde_json::to_vec(message)?;
    let mut frame = format!("Content-Length: {}\r\n\r\n", payload.len()).into_bytes();
    frame.extend_from_slice(&payload);
    Ok(frame)
}

fn lsp_transcript(messages: &[Value]) -> Result<Vec<u8>, Box<dyn Error>> {
    let mut transcript = Vec::new();
    for message in messages {
        transcript.extend_from_slice(&lsp_frame(message)?);
    }
    Ok(transcript)
}

fn lsp_messages(bytes: &[u8]) -> Result<Vec<Value>, Box<dyn Error>> {
    let mut remaining = bytes;
    let mut messages = Vec::new();
    while !remaining.is_empty() {
        let header_offset = remaining
            .windows(4)
            .position(|window| window == b"\r\n\r\n")
            .ok_or("missing LSP header terminator")?;
        let header = std::str::from_utf8(&remaining[..header_offset])?;
        let content_length = header
            .lines()
            .find_map(|line| line.strip_prefix("Content-Length: "))
            .ok_or("missing Content-Length")?
            .parse::<usize>()?;
        let body_start = header_offset + 4;
        let body_end = body_start
            .checked_add(content_length)
            .ok_or("LSP body length overflow")?;
        let body = remaining
            .get(body_start..body_end)
            .ok_or("truncated LSP body")?;
        messages.push(serde_json::from_slice(body)?);
        remaining = remaining.get(body_end..).ok_or("invalid LSP body end")?;
    }
    Ok(messages)
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
            &["init", "--help"],
            include_bytes!("snapshots/init-help.txt"),
        ),
        (&["help", "init"], include_bytes!("snapshots/init-help.txt")),
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
            &["update", "--help"],
            include_bytes!("snapshots/update-help.txt"),
        ),
        (
            &["help", "update"],
            include_bytes!("snapshots/update-help.txt"),
        ),
        (&["lsp", "--help"], include_bytes!("snapshots/lsp-help.txt")),
        (&["help", "lsp"], include_bytes!("snapshots/lsp-help.txt")),
        (
            &["doctor", "--help"],
            include_bytes!("snapshots/doctor-help.txt"),
        ),
        (
            &["help", "doctor"],
            include_bytes!("snapshots/doctor-help.txt"),
        ),
        (
            &["config", "--help"],
            include_bytes!("snapshots/config-help.txt"),
        ),
        (
            &["config", "help"],
            include_bytes!("snapshots/config-help.txt"),
        ),
        (
            &["help", "config"],
            include_bytes!("snapshots/config-help.txt"),
        ),
        (
            &["config", "path", "--help"],
            include_bytes!("snapshots/config-path-help.txt"),
        ),
        (
            &["config", "help", "path"],
            include_bytes!("snapshots/config-path-help.txt"),
        ),
        (
            &["help", "config", "path"],
            include_bytes!("snapshots/config-path-help.txt"),
        ),
        (
            &["config", "get", "--help"],
            include_bytes!("snapshots/config-get-help.txt"),
        ),
        (
            &["config", "help", "get"],
            include_bytes!("snapshots/config-get-help.txt"),
        ),
        (
            &["help", "config", "get"],
            include_bytes!("snapshots/config-get-help.txt"),
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
        (
            &["completions", "--help"],
            include_bytes!("snapshots/completions-help.txt"),
        ),
        (
            &["help", "completions"],
            include_bytes!("snapshots/completions-help.txt"),
        ),
        (
            &["manpage", "--help"],
            include_bytes!("snapshots/manpage-help.txt"),
        ),
        (
            &["help", "manpage"],
            include_bytes!("snapshots/manpage-help.txt"),
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
fn config_discovery_and_doctor_are_read_only_and_report_sources() -> Result<(), Box<dyn Error>> {
    let directory = TestDirectory::new("config-discovery")?;
    let xdg_root = directory.path.join("xdg");
    let config_path = xdg_root.join("stack/config.yaml");
    let default_store = xdg_root.join("stack/icons");

    let output = stack_with_config_environment(Some(&xdg_root), None, ["config", "path"])?;
    assert_eq!(output.status.code(), Some(0));
    assert_eq!(
        output.stdout,
        format!("{}\n", config_path.display()).as_bytes()
    );
    assert!(output.stderr.is_empty());
    assert!(!xdg_root.exists());

    let output = stack_with_config_environment(
        Some(&xdg_root),
        None,
        ["config", "get", "default_icons_path"],
    )?;
    assert_eq!(output.status.code(), Some(0));
    assert_eq!(
        output.stdout,
        format!("{}\n", default_store.display()).as_bytes()
    );
    assert!(output.stderr.is_empty());
    assert!(!xdg_root.exists());

    let output = stack_with_config_environment(Some(&xdg_root), None, ["doctor"])?;
    assert_eq!(output.status.code(), Some(0));
    assert!(output.stderr.is_empty());
    let report = String::from_utf8(output.stdout)?;
    assert!(report.contains(&format!(
        "[ok] config path: {} (source: XDG_CONFIG_HOME)",
        config_path.display()
    )));
    assert!(report.contains("[ok] config file: missing; defaults apply"));
    assert!(report.contains(&format!(
        "[ok] icon store: {} (source: default)",
        default_store.display()
    )));
    assert!(report.contains("[ok] provider packs: store is missing; 0 packs installed"));
    assert!(report.ends_with("Result: healthy\n"));
    let normalized_report = report
        .replace(env!("CARGO_PKG_VERSION"), "<VERSION>")
        .replace(
            directory.path.to_str().ok_or("non-UTF-8 test path")?,
            "<ROOT>",
        );
    assert_eq!(
        normalized_report.as_bytes(),
        include_bytes!("snapshots/doctor-report.txt")
    );
    assert!(!xdg_root.exists());

    let home_root = directory.path.join("home");
    let output = stack_with_config_environment(
        Some(Path::new("relative-xdg")),
        Some(&home_root),
        ["config", "path"],
    )?;
    assert_eq!(output.status.code(), Some(0));
    assert_eq!(
        output.stdout,
        format!(
            "{}\n",
            home_root.join(".config/stack/config.yaml").display()
        )
        .as_bytes()
    );
    assert!(output.stderr.is_empty());
    assert!(!home_root.exists());

    fs::create_dir_all(config_path.parent().ok_or("missing config parent")?)?;
    let custom_store = directory.path.join("shared-icons");
    create_provider_store(&custom_store)?;
    let config = format!("default_icons_path: {}\n", custom_store.display());
    fs::write(&config_path, &config)?;

    let output = stack_with_config_environment(
        Some(&xdg_root),
        None,
        ["config", "get", "default_icons_path"],
    )?;
    assert_eq!(output.status.code(), Some(0));
    assert_eq!(
        output.stdout,
        format!("{}\n", custom_store.display()).as_bytes()
    );
    assert!(output.stderr.is_empty());

    let output = stack_with_config_environment(Some(&xdg_root), None, ["doctor"])?;
    assert_eq!(output.status.code(), Some(0));
    assert!(output.stderr.is_empty());
    let report = String::from_utf8(output.stdout)?;
    assert!(report.contains("[ok] config file: loaded"));
    assert!(report.contains("(source: config default_icons_path)"));
    assert!(report.contains("[ok] provider packs: 1 valid known-provider pack"));
    assert!(report.ends_with("Result: healthy\n"));
    assert_eq!(fs::read_to_string(&config_path)?, config);

    let output = stack_with_config_environment(None, None, ["doctor"])?;
    assert_eq!(output.status.code(), Some(2));
    assert!(output.stderr.is_empty());
    let report = String::from_utf8(output.stdout)?;
    assert!(report.contains("[error] config path: unavailable"));
    assert!(report.contains("[blocked] provider packs: not checked"));
    Ok(())
}

#[test]
fn doctor_distinguishes_warnings_errors_and_redacts_untrusted_values() -> Result<(), Box<dyn Error>>
{
    const SECRET: &str = "stack-test-secret-do-not-print";

    let directory = TestDirectory::new("doctor-diagnostics")?;
    let xdg_root = directory.path.join("xdg");
    let stack_root = xdg_root.join("stack");
    let config_path = stack_root.join("config.yaml");
    let missing_store = directory.path.join("configured-but-missing");
    fs::create_dir_all(&stack_root)?;
    fs::write(
        &config_path,
        format!("default_icons_path: {}\n", missing_store.display()),
    )?;

    let output = stack_with_config_environment(Some(&xdg_root), None, ["doctor"])?;
    assert_eq!(output.status.code(), Some(0));
    assert!(output.stderr.is_empty());
    let report = String::from_utf8(output.stdout)?;
    assert!(report.contains("[warn] provider packs: configured store is missing"));
    assert!(report.ends_with("Result: healthy with 1 warning\n"));

    let output = stack_with_config_environment(
        Some(&xdg_root),
        None,
        [
            "doctor",
            "--provider-pack",
            missing_store.to_str().ok_or("non-UTF-8 test path")?,
        ],
    )?;
    assert_eq!(output.status.code(), Some(2));
    assert!(output.stderr.is_empty());
    let report = String::from_utf8(output.stdout)?;
    assert!(report.contains("[error] provider packs: explicit store is missing"));
    assert!(report.ends_with("Result: 1 problem found\n"));

    let invalid_config = format!("default_icons_path: [\nprivate_token: {SECRET}\n");
    fs::write(&config_path, &invalid_config)?;
    let output = stack_with_config_environment(
        Some(&xdg_root),
        None,
        ["config", "get", "default_icons_path"],
    )?;
    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
    let diagnostic = String::from_utf8(output.stderr)?;
    assert!(diagnostic.contains("invalid YAML"));
    assert!(!diagnostic.contains(SECRET));

    let output = Command::new(env!("CARGO_BIN_EXE_stack"))
        .arg("doctor")
        .env("XDG_CONFIG_HOME", &xdg_root)
        .env("STACK_PRIVATE_TOKEN", SECRET)
        .output()?;
    assert_eq!(output.status.code(), Some(2));
    assert!(output.stderr.is_empty());
    let report = String::from_utf8(output.stdout)?;
    assert!(report.contains("[error] config file: invalid YAML"));
    assert!(report.contains("[blocked] icon store"));
    assert!(!report.contains(SECRET));
    assert_eq!(fs::read_to_string(&config_path)?, invalid_config);

    fs::remove_file(&config_path)?;
    let invalid_store = directory.path.join("invalid-store");
    fs::create_dir_all(invalid_store.join("aws"))?;
    fs::write(
        invalid_store.join("aws/manifest.json"),
        format!("{{\"private_token\":\"{SECRET}\"}}"),
    )?;
    let output = stack_with_config_environment(
        Some(&xdg_root),
        None,
        [
            "doctor",
            "--provider-pack",
            invalid_store.to_str().ok_or("non-UTF-8 test path")?,
        ],
    )?;
    assert_eq!(output.status.code(), Some(2));
    assert!(output.stderr.is_empty());
    let report = String::from_utf8(output.stdout)?;
    assert!(report.contains("[error] provider packs: a known-provider pack is invalid"));
    assert!(!report.contains(SECRET));

    let invalid_store_path = directory.file("provider-store-file", SECRET.as_bytes())?;
    let output = stack_with_config_environment(
        Some(&xdg_root),
        None,
        [
            "doctor",
            "--provider-pack",
            invalid_store_path.to_str().ok_or("non-UTF-8 test path")?,
        ],
    )?;
    assert_eq!(output.status.code(), Some(2));
    assert!(output.stderr.is_empty());
    let report = String::from_utf8(output.stdout)?;
    assert!(report.contains("store must be a real directory, not a file or symlink"));
    assert!(!report.contains(SECRET));
    Ok(())
}

#[cfg(unix)]
#[test]
fn doctor_reports_permission_failures_without_exposing_file_contents() -> Result<(), Box<dyn Error>>
{
    use std::os::unix::fs::PermissionsExt;
    use std::os::unix::fs::symlink;

    const SECRET: &str = "permission-test-secret-do-not-print";

    let directory = TestDirectory::new("doctor-permissions")?;
    let xdg_root = directory.path.join("xdg");
    let stack_root = xdg_root.join("stack");
    let config_path = stack_root.join("config.yaml");
    fs::create_dir_all(&stack_root)?;
    fs::write(&config_path, format!("private_token: {SECRET}\n"))?;
    fs::set_permissions(&config_path, fs::Permissions::from_mode(0o000))?;

    if fs::read(&config_path).is_err() {
        let output = stack_with_config_environment(Some(&xdg_root), None, ["doctor"])?;
        assert_eq!(output.status.code(), Some(2));
        assert!(output.stderr.is_empty());
        let report = String::from_utf8(output.stdout)?;
        assert!(report.contains("[error] config file: permission denied"));
        assert!(!report.contains(SECRET));
    }
    fs::set_permissions(&config_path, fs::Permissions::from_mode(0o600))?;
    fs::remove_file(&config_path)?;

    let provider_store = directory.path.join("provider-store");
    create_provider_store(&provider_store)?;
    let manifest = provider_store.join("simple-icons/manifest.json");
    fs::set_permissions(&manifest, fs::Permissions::from_mode(0o000))?;
    if fs::read(&manifest).is_err() {
        let output = stack_with_config_environment(
            Some(&xdg_root),
            None,
            [
                "doctor",
                "--provider-pack",
                provider_store.to_str().ok_or("non-UTF-8 test path")?,
            ],
        )?;
        assert_eq!(output.status.code(), Some(2));
        assert!(output.stderr.is_empty());
        let report = String::from_utf8(output.stdout)?;
        assert!(report.contains("store or pack is unreadable"));
        assert!(!report.contains(SECRET));
    }
    fs::set_permissions(&manifest, fs::Permissions::from_mode(0o600))?;

    let provider_store_link = directory.path.join("provider-store-link");
    symlink(&provider_store, &provider_store_link)?;
    let output = stack_with_config_environment(
        Some(&xdg_root),
        None,
        [
            "doctor",
            "--provider-pack",
            provider_store_link.to_str().ok_or("non-UTF-8 test path")?,
        ],
    )?;
    assert_eq!(output.status.code(), Some(2));
    assert!(output.stderr.is_empty());
    let report = String::from_utf8(output.stdout)?;
    assert!(report.contains("store must be a real directory, not a file or symlink"));
    Ok(())
}

#[test]
fn generated_shell_and_manual_assets_are_exact_and_usable() -> Result<(), Box<dyn Error>> {
    for (shell, expected) in [
        (
            "bash",
            include_bytes!("../distribution/generated/share/bash-completion/completions/stack")
                .as_slice(),
        ),
        (
            "zsh",
            include_bytes!("../distribution/generated/share/zsh/site-functions/_stack").as_slice(),
        ),
        (
            "fish",
            include_bytes!("../distribution/generated/share/fish/vendor_completions.d/stack.fish")
                .as_slice(),
        ),
    ] {
        assert_stdout_only(&["completions", shell], expected)?;
    }
    assert_stdout_only(
        &["manpage"],
        include_bytes!("../distribution/generated/share/man/man1/stack.1"),
    )?;

    let directory = TestDirectory::new("completion-smoke")?;
    let bash_completion = directory.file(
        "stack.bash",
        include_bytes!("../distribution/generated/share/bash-completion/completions/stack"),
    )?;
    let bash = Command::new("bash")
        .args([
            "-c",
            "source \"$1\"; COMP_WORDS=(stack r); COMP_CWORD=1; _stack_completion; printf '%s\\n' \"${COMPREPLY[@]}\"; COMP_WORDS=(stack icons li); COMP_CWORD=2; _stack_completion; printf '%s\\n' \"${COMPREPLY[@]}\"; COMP_WORDS=(stack icons list a); COMP_CWORD=3; _stack_completion; printf '%s\\n' \"${COMPREPLY[@]}\"",
            "stack-completion-test",
        ])
        .arg(&bash_completion)
        .output()?;
    assert!(bash.status.success());
    assert_eq!(bash.stdout, b"render\nlist\naws\nazure\n");
    assert!(bash.stderr.is_empty());

    for (program, relative_path) in [
        (
            "zsh",
            "distribution/generated/share/zsh/site-functions/_stack",
        ),
        (
            "fish",
            "distribution/generated/share/fish/vendor_completions.d/stack.fish",
        ),
    ] {
        match Command::new(program)
            .arg("-n")
            .arg(Path::new(env!("CARGO_MANIFEST_DIR")).join(relative_path))
            .output()
        {
            Ok(output) => {
                assert!(output.status.success(), "{program} syntax check failed");
                assert!(
                    output.stderr.is_empty(),
                    "{program} syntax check emitted diagnostics"
                );
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
    }

    let unsupported = stack(["completions", "powershell"])?;
    assert_eq!(unsupported.status.code(), Some(2));
    assert!(unsupported.stdout.is_empty());
    let diagnostics = String::from_utf8(unsupported.stderr)?;
    assert!(
        diagnostics.starts_with(
            "error: unsupported shell 'powershell'; supported shells: bash, zsh, fish\n"
        )
    );
    assert!(diagnostics.contains("Usage:\n  stack <COMMAND>"));
    Ok(())
}

#[test]
fn command_typos_are_actionable_and_stderr_only() -> Result<(), Box<dyn Error>> {
    let cases: &[(&[&str], &str)] = &[
        (
            &["inti"],
            "error: unknown command 'inti'\n\nDid you mean 'init'?\n\nFor more information, try 'stack help'.\n",
        ),
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
            &["lspp"],
            "error: unknown command 'lspp'\n\nDid you mean 'lsp'?\n\nFor more information, try 'stack help'.\n",
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
fn lsp_binary_serves_core_features_and_recovers_from_invalid_json() -> Result<(), Box<dyn Error>> {
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
    let messages = [
        json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "capabilities": { "general": { "positionEncodings": ["utf-16"] } }
            }
        }),
        json!({
            "jsonrpc": "2.0",
            "method": "textDocument/didOpen",
            "params": {
                "textDocument": {
                    "uri": "file:///workspace/diagram.stack",
                    "languageId": "stack",
                    "version": 1,
                    "text": source
                }
            }
        }),
        json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "textDocument/completion",
            "params": {
                "textDocument": { "uri": "file:///workspace/diagram.stack" },
                "position": { "line": 5, "character": 13 }
            }
        }),
        json!({
            "jsonrpc": "2.0",
            "id": 3,
            "method": "textDocument/hover",
            "params": {
                "textDocument": { "uri": "file:///workspace/diagram.stack" },
                "position": { "line": 8, "character": 8 }
            }
        }),
        json!({
            "jsonrpc": "2.0",
            "id": 4,
            "method": "textDocument/documentSymbol",
            "params": {
                "textDocument": { "uri": "file:///workspace/diagram.stack" }
            }
        }),
        json!({
            "jsonrpc": "2.0",
            "id": 5,
            "method": "textDocument/formatting",
            "params": {
                "textDocument": { "uri": "file:///workspace/diagram.stack" },
                "options": { "tabSize": 2, "insertSpaces": true }
            }
        }),
        json!({ "jsonrpc": "2.0", "id": 6, "method": "shutdown" }),
        json!({ "jsonrpc": "2.0", "method": "exit" }),
    ];
    let mut transcript = b"Content-Length: 1\r\n\r\n{".to_vec();
    transcript.extend_from_slice(&lsp_transcript(&messages)?);

    let output = stack_with_input(["lsp"], &transcript)?;
    assert_eq!(output.status.code(), Some(0));
    assert!(output.stderr.is_empty());
    let responses = lsp_messages(&output.stdout)?;
    assert_eq!(responses.len(), 8);
    assert_eq!(responses[0]["error"]["code"], -32_700);
    assert_eq!(
        responses[1]["result"]["capabilities"]["positionEncoding"],
        "utf-16"
    );
    assert_eq!(responses[2]["method"], "textDocument/publishDiagnostics");

    let completion = responses
        .iter()
        .find(|message| message["id"] == 2)
        .ok_or("missing completion response")?;
    assert!(
        completion["result"]["items"]
            .as_array()
            .is_some_and(|items| items.iter().any(|item| item["filterText"] == "server"))
    );
    let hover = responses
        .iter()
        .find(|message| message["id"] == 3)
        .ok_or("missing hover response")?;
    assert!(
        hover["result"]["contents"]["value"]
            .as_str()
            .is_some_and(|value| value.contains("node api"))
    );
    let symbols = responses
        .iter()
        .find(|message| message["id"] == 4)
        .ok_or("missing symbols response")?;
    assert_eq!(symbols["result"][0]["name"], "😀 Checkout");
    let formatting = responses
        .iter()
        .find(|message| message["id"] == 5)
        .ok_or("missing formatting response")?;
    assert!(
        formatting["result"]
            .as_array()
            .is_some_and(|edits| !edits.is_empty())
    );
    assert_eq!(
        responses.last().and_then(|message| message.get("id")),
        Some(&json!(6))
    );
    Ok(())
}

#[test]
fn lsp_large_invalid_document_has_bounded_latency_and_no_crash() -> Result<(), Box<dyn Error>> {
    let padding = "x".repeat(1024 * 1024);
    let source = format!("// {padding}\nstack 1.0 diagram \"Incomplete\" {{ node api");
    let messages = [
        json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": { "capabilities": {} }
        }),
        json!({
            "jsonrpc": "2.0",
            "method": "textDocument/didOpen",
            "params": {
                "textDocument": {
                    "uri": "file:///workspace/large.stack",
                    "languageId": "stack",
                    "version": 1,
                    "text": source
                }
            }
        }),
        json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "textDocument/documentSymbol",
            "params": { "textDocument": { "uri": "file:///workspace/large.stack" } }
        }),
        json!({ "jsonrpc": "2.0", "id": 3, "method": "shutdown" }),
        json!({ "jsonrpc": "2.0", "method": "exit" }),
    ];
    let transcript = lsp_transcript(&messages)?;
    let started = Instant::now();
    let output = stack_with_input(["lsp"], &transcript)?;
    let elapsed = started.elapsed();

    assert_eq!(output.status.code(), Some(0));
    assert!(output.stderr.is_empty());
    assert!(
        elapsed < Duration::from_secs(5),
        "large document took {elapsed:?}"
    );
    let responses = lsp_messages(&output.stdout)?;
    let diagnostics = responses
        .iter()
        .find(|message| message["method"] == "textDocument/publishDiagnostics")
        .ok_or("missing diagnostics")?;
    assert!(
        diagnostics["params"]["diagnostics"]
            .as_array()
            .is_some_and(|items| !items.is_empty())
    );
    let symbols = responses
        .iter()
        .find(|message| message["id"] == 2)
        .ok_or("missing symbols response")?;
    assert_eq!(symbols["result"], json!([]));
    Ok(())
}

#[test]
fn lsp_rejects_untrusted_framing_before_allocating_the_body() -> Result<(), Box<dyn Error>> {
    let input = b"Content-Length: 8388609\r\n\r\n";
    let output = stack_with_input(["lsp"], input)?;

    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
    assert_eq!(
        output.stderr,
        b"error: invalid LSP frame: message exceeds the size limit\n"
    );
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
fn init_creates_a_valid_renderable_default_project() -> Result<(), Box<dyn Error>> {
    let directory = TestDirectory::new("init-default")?;
    let initialized = stack_in(&directory.path, ["init"])?;
    let source_path = directory.path.join("diagram.stack");

    assert_eq!(initialized.status.code(), Some(0));
    assert!(initialized.stderr.is_empty());
    assert!(String::from_utf8(initialized.stdout)?.contains("template 'hello-stack'"));
    assert_eq!(
        fs::read(&source_path)?,
        include_bytes!("../templates/sources/01-minimal.stack")
    );

    let checked = stack_in(&directory.path, ["check", "diagram.stack"])?;
    assert_eq!(checked.status.code(), Some(0));
    assert!(checked.stdout.is_empty());
    assert!(checked.stderr.is_empty());

    let rendered = stack_in(
        &directory.path,
        ["render", "diagram.stack", "-o", "diagram.svg"],
    )?;
    assert_eq!(rendered.status.code(), Some(0));
    assert!(rendered.stdout.is_empty());
    assert!(rendered.stderr.is_empty());
    assert!(fs::read_to_string(directory.path.join("diagram.svg"))?.contains("<svg"));
    Ok(())
}

#[test]
fn init_selects_templates_protects_files_and_explains_provider_icons() -> Result<(), Box<dyn Error>>
{
    let directory = TestDirectory::new("init-options")?;
    let source_path = directory.path.join("architecture.stack");
    let initialized = stack_in(
        &directory.path,
        [
            "init",
            "--template",
            "aws-serverless-checkout",
            "--output",
            "architecture.stack",
        ],
    )?;

    assert_eq!(initialized.status.code(), Some(0));
    assert!(initialized.stderr.is_empty());
    let output = String::from_utf8(initialized.stdout)?;
    assert!(output.contains("Provider icons: aws"));
    assert!(output.contains("stack icons import aws --accept-terms"));
    assert_eq!(
        fs::read(&source_path)?,
        include_bytes!("../templates/sources/05-aws-serverless.stack")
    );

    let protected = stack_in(
        &directory.path,
        [
            "init",
            "--template",
            "application-and-data",
            "-o",
            "architecture.stack",
        ],
    )?;
    assert_eq!(protected.status.code(), Some(2));
    assert!(protected.stdout.is_empty());
    assert!(String::from_utf8(protected.stderr)?.contains("pass '--force'"));
    assert_eq!(
        fs::read(&source_path)?,
        include_bytes!("../templates/sources/05-aws-serverless.stack")
    );

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&source_path, fs::Permissions::from_mode(0o640))?;
    }
    let replaced = stack_in(
        &directory.path,
        [
            "init",
            "--template",
            "application-and-data",
            "-o",
            "architecture.stack",
            "--force",
        ],
    )?;
    assert_eq!(replaced.status.code(), Some(0));
    assert!(replaced.stderr.is_empty());
    assert_eq!(
        fs::read(&source_path)?,
        include_bytes!("../templates/sources/02-node-semantics.stack")
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            fs::metadata(&source_path)?.permissions().mode() & 0o777,
            0o640
        );
    }

    let missing_parent = stack_in(&directory.path, ["init", "-o", "missing/diagram.stack"])?;
    assert_eq!(missing_parent.status.code(), Some(2));
    assert!(missing_parent.stdout.is_empty());
    assert!(String::from_utf8(missing_parent.stderr)?.contains("file not found"));
    assert!(!directory.path.join("missing").exists());
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
