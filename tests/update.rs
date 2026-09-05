use std::collections::BTreeMap;
use std::env;
use std::error::Error;
use std::ffi::OsStr;
use std::fs;
use std::io::{self, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::process::{Command, Output};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use flate2::Compression;
use flate2::write::GzEncoder;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

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
fn assert_stdout_only(arguments: &[&str], expected: &[u8]) -> Result<(), Box<dyn Error>> {
    let output = stack(arguments.iter().copied())?;
    assert_eq!(output.status.code(), Some(0), "arguments: {arguments:?}");
    assert_eq!(output.stdout, expected, "arguments: {arguments:?}");
    assert!(output.stderr.is_empty(), "arguments: {arguments:?}");
    Ok(())
}

fn sha256(bytes: &[u8]) -> String {
    let mut output = String::with_capacity(64);
    for byte in Sha256::digest(bytes) {
        let _ = std::fmt::Write::write_fmt(&mut output, format_args!("{byte:02x}"));
    }
    output
}

fn append_tar_entry(
    builder: &mut tar::Builder<GzEncoder<Vec<u8>>>,
    name: &str,
    bytes: &[u8],
    mode: u32,
    kind: tar::EntryType,
    epoch: u64,
) -> Result<(), Box<dyn Error>> {
    let mut header = tar::Header::new_ustar();
    header.set_size(bytes.len() as u64);
    header.set_mode(mode);
    header.set_uid(0);
    header.set_gid(0);
    header.set_mtime(epoch);
    header.set_entry_type(kind);
    header.set_cksum();
    builder.append_data(&mut header, name, bytes)?;
    Ok(())
}

fn update_archive(
    version: &str,
    target: &str,
    candidate: &[u8],
    epoch: u64,
) -> Result<Vec<u8>, Box<dyn Error>> {
    let encoder = GzEncoder::new(Vec::new(), Compression::default());
    let mut builder = tar::Builder::new(encoder);
    let root = format!("stack-v{version}-{target}");
    append_tar_entry(
        &mut builder,
        &root,
        &[],
        0o755,
        tar::EntryType::Directory,
        epoch,
    )?;
    for (name, bytes) in [
        ("LICENSE", b"license".as_slice()),
        ("NOTICE", b"notice".as_slice()),
        ("THIRD_PARTY_LICENSES.md", b"third party".as_slice()),
    ] {
        append_tar_entry(
            &mut builder,
            &format!("{root}/{name}"),
            bytes,
            0o644,
            tar::EntryType::Regular,
            epoch,
        )?;
    }
    append_tar_entry(
        &mut builder,
        &format!("{root}/stack"),
        candidate,
        0o755,
        tar::EntryType::Regular,
        epoch,
    )?;
    let encoder = builder.into_inner()?;
    Ok(encoder.finish()?)
}

fn update_target() -> Result<&'static str, Box<dyn Error>> {
    match (env::consts::OS, env::consts::ARCH) {
        ("macos", "aarch64") => Ok("aarch64-apple-darwin"),
        ("macos", "x86_64") => Ok("x86_64-apple-darwin"),
        ("linux", "aarch64") => Ok("aarch64-unknown-linux-gnu"),
        ("linux", "x86_64") => Ok("x86_64-unknown-linux-gnu"),
        _ => Err("unsupported update integration-test host".into()),
    }
}

struct UpdateServer {
    base: String,
    hits: Arc<Mutex<Vec<String>>>,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl UpdateServer {
    fn start(
        routes: impl FnOnce(&str) -> BTreeMap<String, Vec<u8>>,
    ) -> Result<Self, Box<dyn Error>> {
        let listener = TcpListener::bind("127.0.0.1:0")?;
        listener.set_nonblocking(true)?;
        let base = format!("http://{}", listener.local_addr()?);
        let routes = Arc::new(routes(&base));
        let hits = Arc::new(Mutex::new(Vec::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let server_hits = Arc::clone(&hits);
        let server_stop = Arc::clone(&stop);
        let thread = thread::spawn(move || {
            while !server_stop.load(Ordering::Relaxed) {
                match listener.accept() {
                    Ok((stream, _)) => serve_update_route(stream, &routes, &server_hits),
                    Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(2));
                    }
                    Err(_) => break,
                }
            }
        });
        Ok(Self {
            base,
            hits,
            stop,
            thread: Some(thread),
        })
    }

    fn hits(&self) -> Vec<String> {
        match self.hits.lock() {
            Ok(hits) => hits.clone(),
            Err(_) => Vec::new(),
        }
    }
}

impl Drop for UpdateServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn serve_update_route(
    mut stream: TcpStream,
    routes: &BTreeMap<String, Vec<u8>>,
    hits: &Mutex<Vec<String>>,
) {
    let _ = stream.set_nonblocking(false);
    let _ = stream.set_read_timeout(Some(Duration::from_secs(1)));
    let mut request = Vec::new();
    let mut buffer = [0_u8; 1024];
    while request.len() < 8192 && !request.windows(4).any(|bytes| bytes == b"\r\n\r\n") {
        match stream.read(&mut buffer) {
            Ok(0) | Err(_) => break,
            Ok(read) => request.extend_from_slice(&buffer[..read]),
        }
    }
    let route = std::str::from_utf8(&request)
        .ok()
        .and_then(|request| request.lines().next())
        .and_then(|line| line.split_whitespace().nth(1))
        .unwrap_or("/")
        .to_owned();
    if let Ok(mut hits) = hits.lock() {
        hits.push(route.clone());
    }
    let (status, body) = match routes.get(&route) {
        Some(body) => ("200 OK", body.as_slice()),
        None => ("404 Not Found", b"missing".as_slice()),
    };
    let header = format!(
        "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    let _ = stream.write_all(header.as_bytes());
    let _ = stream.write_all(body);
}
#[test]
fn update_argument_contract_is_local_and_fail_closed() -> Result<(), Box<dyn Error>> {
    assert_stdout_only(
        &["update", "--check", "--version", env!("CARGO_PKG_VERSION")],
        format!(
            "stack {} is already installed; no files were changed.\n",
            env!("CARGO_PKG_VERSION")
        )
        .as_bytes(),
    )?;

    let cases: &[(&[&str], &str)] = &[
        (
            &["update", "--check", "--check"],
            "duplicate '--check' option",
        ),
        (
            &["update", "--version", "1.0.0", "--version", "1.0.1"],
            "duplicate '--version' option",
        ),
        (
            &["update", "--version"],
            "missing version after '--version'",
        ),
        (
            &["update", "--version", "--check"],
            "missing version after '--version'",
        ),
        (&["update", "--version", "1.0"], "update version must be"),
        (&["update", "--version", "1.0.0-beta.1"], "only an exact"),
        (&["update", "extra"], "unexpected argument 'extra'"),
        (
            &["update", "--help", "extra"],
            "unexpected argument 'extra'",
        ),
    ];
    for (arguments, expected) in cases {
        let output = stack(arguments.iter().copied())?;
        assert_eq!(output.status.code(), Some(2), "arguments: {arguments:?}");
        assert!(output.stdout.is_empty(), "arguments: {arguments:?}");
        assert!(
            String::from_utf8(output.stderr)?.contains(expected),
            "arguments: {arguments:?}"
        );
    }
    Ok(())
}

#[cfg(unix)]
#[test]
fn update_binary_integrates_local_release_verification_and_atomic_replacement()
-> Result<(), Box<dyn Error>> {
    use std::os::unix::fs::PermissionsExt;

    let directory = TestDirectory::new("update-process")?;
    let install_directory = directory.path.join("install");
    let tool_directory = directory.path.join("tools");
    let config_directory = directory.path.join("config");
    fs::create_dir_all(&install_directory)?;
    fs::create_dir_all(&tool_directory)?;
    fs::create_dir_all(config_directory.join("stack"))?;

    let installed = install_directory.join("stack");
    fs::copy(env!("CARGO_BIN_EXE_stack"), &installed)?;
    fs::set_permissions(&installed, fs::Permissions::from_mode(0o755))?;
    let installed = installed.canonicalize()?;
    let current_bytes = fs::read(&installed)?;
    let target = update_target()?;
    let current_version = env!("CARGO_PKG_VERSION");
    let update_version = "0.3.1";
    let epoch = 1_788_566_400_u64;
    let current_commit = "1111111111111111111111111111111111111111";
    let release_commit = "2222222222222222222222222222222222222222";

    let receipt_path = config_directory.join("stack/install-receipt.json");
    fs::write(
        &receipt_path,
        serde_json::to_vec_pretty(&json!({
            "$schema": format!(
                "https://raw.githubusercontent.com/stack-sh/cli/{current_commit}/distribution/install-receipt.schema.json"
            ),
            "schemaVersion": 1,
            "owner": "github-release",
            "repository": "stack-sh/cli",
            "version": current_version,
            "target": target,
            "sourceCommit": current_commit,
            "archive": {
                "name": format!("stack-v{current_version}-{target}.tar.gz"),
                "sha256": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
            },
            "binary": {
                "path": installed.to_str().ok_or("installed path is not UTF-8")?,
                "sha256": sha256(&current_bytes)
            }
        }))?,
    )?;

    let candidate = b"#!/bin/sh\nif [ \"$1\" = \"--version\" ]; then printf 'stack 0.3.1\\n'; exit 0; fi\nexit 2\n";
    let archive = update_archive(update_version, target, candidate, epoch)?;
    let archive_digest = sha256(&archive);
    let supported_targets = [
        "aarch64-apple-darwin",
        "aarch64-unknown-linux-gnu",
        "x86_64-apple-darwin",
        "x86_64-unknown-linux-gnu",
    ];
    let targets: Vec<Value> = supported_targets
        .iter()
        .map(|release_target| {
            json!({
                "target": release_target,
                "archive": {
                    "name": format!("stack-v{update_version}-{release_target}.tar.gz"),
                    "sha256": if *release_target == target {
                        archive_digest.clone()
                    } else {
                        "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_owned()
                    }
                },
                "sbom": {
                    "name": format!("stack-v{update_version}-{release_target}.spdx.json"),
                    "sha256": "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
                },
                "provenance": {
                    "name": format!("stack-v{update_version}-{release_target}.provenance.sigstore.json"),
                    "sha256": "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc"
                },
                "sbomAttestation": {
                    "name": format!("stack-v{update_version}-{release_target}.sbom.sigstore.json"),
                    "sha256": "dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd"
                }
            })
        })
        .collect();
    let manifest = serde_json::to_vec(&json!({
        "$schema": format!(
            "https://raw.githubusercontent.com/stack-sh/cli/{release_commit}/distribution/release-manifest.schema.json"
        ),
        "schemaVersion": 1,
        "version": update_version,
        "tag": format!("v{update_version}"),
        "source": { "repository": "stack-sh/cli", "commit": release_commit },
        "minimumSupportedCliVersion": current_version,
        "sourceDateEpoch": epoch,
        "builderWorkflow": "stack-sh/cli/.github/workflows/release.yaml",
        "verifiedChannels": ["github-release", "self-update"],
        "targets": targets
    }))?;
    let manifest_name = format!("stack-v{update_version}-release-manifest.json");
    let archive_name = format!("stack-v{update_version}-{target}.tar.gz");
    let server = UpdateServer::start(|base| {
        let response = serde_json::to_vec(&json!({
            "tag_name": format!("v{update_version}"),
            "draft": false,
            "prerelease": false,
            "assets": [
                {
                    "name": manifest_name,
                    "state": "uploaded",
                    "size": manifest.len(),
                    "digest": format!("sha256:{}", sha256(&manifest)),
                    "browser_download_url": format!("{base}/download/v{update_version}/{manifest_name}")
                },
                {
                    "name": archive_name,
                    "state": "uploaded",
                    "size": archive.len(),
                    "digest": format!("sha256:{archive_digest}"),
                    "browser_download_url": format!("{base}/download/v{update_version}/{archive_name}")
                }
            ]
        }))
        .unwrap_or_default();
        BTreeMap::from([
            ("/repos/stack-sh/cli/releases/latest".to_owned(), response),
            (
                format!("/download/v{update_version}/{manifest_name}"),
                manifest,
            ),
            (
                format!("/download/v{update_version}/{archive_name}"),
                archive,
            ),
        ])
    })?;

    let gh_log = directory.path.join("gh-calls.txt");
    let gh = tool_directory.join("gh");
    fs::write(
        &gh,
        format!(
            "#!/bin/sh\nprintf 'verified\\n' >> '{}'\n",
            gh_log.display()
        ),
    )?;
    fs::set_permissions(&gh, fs::Permissions::from_mode(0o755))?;
    let existing_paths = env::var_os("PATH")
        .map(|value| env::split_paths(&value).collect::<Vec<_>>())
        .unwrap_or_default();
    let command_paths = env::join_paths(std::iter::once(tool_directory).chain(existing_paths))?;

    let output = Command::new(&installed)
        .arg("update")
        .env("XDG_CONFIG_HOME", &config_directory)
        .env("STACK_CLI_TEST_UPDATE_BASE_URL", &server.base)
        .env("PATH", command_paths)
        .output()?;
    assert_eq!(output.status.code(), Some(0));
    assert!(
        output.stderr.is_empty(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8(output.stdout)?.contains("Updated stack 0.3.0 -> 0.3.1"));
    assert_eq!(fs::read(&installed)?, candidate);
    assert_eq!(fs::read_to_string(&gh_log)?, "verified\nverified\n");
    assert_eq!(server.hits().len(), 3);

    let version = Command::new(&installed).arg("--version").output()?;
    assert_eq!(version.status.code(), Some(0));
    assert_eq!(version.stdout, b"stack 0.3.1\n");
    let receipt: Value = serde_json::from_slice(&fs::read(receipt_path)?)?;
    assert_eq!(receipt["version"], "0.3.1");
    assert_eq!(receipt["sourceCommit"], release_commit);
    assert_eq!(receipt["binary"]["sha256"], sha256(candidate));
    Ok(())
}
