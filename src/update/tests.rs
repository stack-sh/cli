use super::*;
use std::error::Error;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use flate2::Compression;
use flate2::write::GzEncoder;
use serde_json::{Value, json};

type TestResult<T = ()> = Result<T, Box<dyn Error>>;

static CASE_ID: AtomicU64 = AtomicU64::new(0);
const CURRENT_COMMIT: &str = "1111111111111111111111111111111111111111";
const RELEASE_COMMIT: &str = "2222222222222222222222222222222222222222";
const TARGET: &str = "x86_64-unknown-linux-gnu";
const EPOCH: u64 = 1_788_566_400;

fn boxed(message: String) -> Box<dyn Error> {
    Box::new(io::Error::other(message))
}

fn result_error<T>(result: Result<T, String>) -> TestResult<String> {
    match result {
        Ok(_) => Err(io::Error::other("operation unexpectedly succeeded").into()),
        Err(error) => Ok(error),
    }
}

struct TestDirectory {
    root: PathBuf,
}

impl TestDirectory {
    fn new(label: &str) -> io::Result<Self> {
        let id = CASE_ID.fetch_add(1, Ordering::Relaxed);
        let root = env::temp_dir().join(format!(
            "stack-update-test-{}-{id}-{label}",
            std::process::id()
        ));
        fs::create_dir(&root)?;
        Ok(Self { root })
    }

    fn path(&self, relative: &str) -> PathBuf {
        self.root.join(relative)
    }
}

impl Drop for TestDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

struct LocalServer {
    base: String,
    hits: Arc<Mutex<Vec<String>>>,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl LocalServer {
    fn start(routes: impl FnOnce(&str) -> BTreeMap<String, Vec<u8>>) -> io::Result<Self> {
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
                    Ok((stream, _)) => serve(stream, &routes, &server_hits),
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

    fn base(&self) -> &str {
        &self.base
    }

    fn hits(&self) -> Vec<String> {
        match self.hits.lock() {
            Ok(hits) => hits.clone(),
            Err(_) => Vec::new(),
        }
    }
}

impl Drop for LocalServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn serve(mut stream: TcpStream, routes: &BTreeMap<String, Vec<u8>>, hits: &Mutex<Vec<String>>) {
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
    let response = format!(
        "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    let _ = stream.write_all(response.as_bytes());
    let _ = stream.write_all(body);
}

fn add_archive_entry(
    builder: &mut tar::Builder<GzEncoder<Vec<u8>>>,
    name: &str,
    bytes: &[u8],
    mode: u32,
    kind: tar::EntryType,
) -> io::Result<()> {
    let mut header = tar::Header::new_ustar();
    header.set_size(bytes.len() as u64);
    header.set_mode(mode);
    header.set_uid(0);
    header.set_gid(0);
    header.set_mtime(EPOCH);
    header.set_entry_type(kind);
    header.set_cksum();
    builder.append_data(&mut header, name, bytes)
}

fn release_archive(version: &Version, candidate: &[u8]) -> TestResult<Vec<u8>> {
    let encoder = GzEncoder::new(Vec::new(), Compression::default());
    let mut builder = tar::Builder::new(encoder);
    let root = format!("stack-v{version}-{TARGET}");
    add_archive_entry(&mut builder, &root, &[], 0o755, tar::EntryType::Directory)?;
    for (name, bytes) in [
        ("LICENSE", b"license".as_slice()),
        ("NOTICE", b"notice".as_slice()),
        ("THIRD_PARTY_LICENSES.md", b"third party".as_slice()),
    ] {
        add_archive_entry(
            &mut builder,
            &format!("{root}/{name}"),
            bytes,
            0o644,
            tar::EntryType::Regular,
        )?;
    }
    add_archive_entry(
        &mut builder,
        &format!("{root}/stack"),
        candidate,
        0o755,
        tar::EntryType::Regular,
    )?;
    let encoder = builder.into_inner()?;
    Ok(encoder.finish()?)
}

fn manifest_value(version: &Version, archive_digest: &str) -> Value {
    let targets: Vec<Value> = SUPPORTED_TARGETS
        .iter()
        .map(|target| {
            let archive_digest = if *target == TARGET {
                archive_digest.to_owned()
            } else {
                "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_owned()
            };
            json!({
                "target": target,
                "archive": {
                    "name": archive_name(version, target),
                    "sha256": archive_digest,
                },
                "sbom": {
                    "name": format!("stack-v{version}-{target}.spdx.json"),
                    "sha256": "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
                },
                "provenance": {
                    "name": format!("stack-v{version}-{target}.provenance.sigstore.json"),
                    "sha256": "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc",
                },
                "sbomAttestation": {
                    "name": format!("stack-v{version}-{target}.sbom.sigstore.json"),
                    "sha256": "dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd",
                },
            })
        })
        .collect();
    json!({
        "$schema": format!(
            "https://raw.githubusercontent.com/{REPOSITORY}/{RELEASE_COMMIT}/distribution/release-manifest.schema.json"
        ),
        "schemaVersion": 1,
        "version": version.to_string(),
        "tag": format!("v{version}"),
        "source": { "repository": REPOSITORY, "commit": RELEASE_COMMIT },
        "minimumSupportedCliVersion": "1.0.0",
        "sourceDateEpoch": EPOCH,
        "builderWorkflow": RELEASE_WORKFLOW,
        "verifiedChannels": ["github-release", "self-update"],
        "targets": targets,
    })
}

fn release_response(
    base: &str,
    version: &Version,
    manifest: &[u8],
    archive_name: &str,
    archive_size: usize,
    archive_digest: &str,
) -> Vec<u8> {
    let manifest_name = format!("stack-v{version}-release-manifest.json");
    serde_json::to_vec(&json!({
        "tag_name": format!("v{version}"),
        "draft": false,
        "prerelease": !version.pre.is_empty(),
        "assets": [
            {
                "name": manifest_name,
                "state": "uploaded",
                "size": manifest.len(),
                "digest": format!("sha256:{}", sha256_bytes(manifest)),
                "browser_download_url": format!("{base}/download/v{version}/{manifest_name}"),
            },
            {
                "name": archive_name,
                "state": "uploaded",
                "size": archive_size,
                "digest": format!("sha256:{archive_digest}"),
                "browser_download_url": format!("{base}/download/v{version}/{archive_name}"),
            }
        ]
    }))
    .unwrap_or_default()
}

fn write_receipt(runtime: &Runtime) -> TestResult<InstallationReceipt> {
    let receipt = InstallationReceipt::for_release(
        runtime,
        &runtime.current_version,
        CURRENT_COMMIT,
        &archive_name(&runtime.current_version, &runtime.target),
        "eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee",
        &sha256_file(&runtime.current_executable, MAX_BINARY_BYTES).map_err(boxed)?,
    )
    .map_err(boxed)?;
    let parent = parent_directory(&runtime.receipt_path).map_err(boxed)?;
    fs::create_dir_all(parent)?;
    fs::write(&runtime.receipt_path, serde_json::to_vec_pretty(&receipt)?)?;
    Ok(receipt)
}

struct UpdateFixture {
    _directory: TestDirectory,
    runtime: Runtime,
    client: ReleaseClient,
    server: LocalServer,
    candidate: Vec<u8>,
    manifest: Vec<u8>,
    archive: Vec<u8>,
}

impl UpdateFixture {
    fn new(tampered_archive: bool) -> TestResult<Self> {
        let directory = TestDirectory::new("fixture")?;
        let binary = directory.path("bin/stack");
        let receipt = directory.path("config/stack/install-receipt.json");
        let binary_parent = binary
            .parent()
            .ok_or_else(|| io::Error::other("binary parent is missing"))?;
        fs::create_dir_all(binary_parent)?;
        fs::write(&binary, b"old binary")?;
        #[cfg(unix)]
        fs::set_permissions(&binary, fs::Permissions::from_mode(0o755))?;
        let runtime = Runtime {
            current_version: Version::parse("1.0.0")?,
            current_executable: fs::canonicalize(binary)?,
            receipt_path: receipt,
            target: TARGET.to_owned(),
        };
        write_receipt(&runtime)?;

        let release_version = Version::parse("1.1.0")?;
        let candidate = b"new verified binary".to_vec();
        let archive = release_archive(&release_version, &candidate)?;
        let archive_digest = sha256_bytes(&archive);
        let manifest = serde_json::to_vec(&manifest_value(&release_version, &archive_digest))?;
        let archive_name = archive_name(&release_version, TARGET);
        let mut served_archive = archive.clone();
        if tampered_archive {
            served_archive.extend_from_slice(b"tampered");
        }
        let response_archive_size = served_archive.len();
        let server = LocalServer::start(|base| {
            let response = release_response(
                base,
                &release_version,
                &manifest,
                &archive_name,
                response_archive_size,
                &archive_digest,
            );
            BTreeMap::from([
                (
                    format!("/repos/{REPOSITORY}/releases/latest"),
                    response.clone(),
                ),
                (
                    format!("/repos/{REPOSITORY}/releases/tags/v{release_version}"),
                    response,
                ),
                (
                    format!(
                        "/download/v{release_version}/stack-v{release_version}-release-manifest.json"
                    ),
                    manifest.clone(),
                ),
                (
                    format!("/download/v{release_version}/{archive_name}"),
                    served_archive,
                ),
            ])
        })?;
        let client = ReleaseClient::for_test(server.base());
        Ok(Self {
            _directory: directory,
            runtime,
            client,
            server,
            candidate,
            manifest,
            archive,
        })
    }
}

struct RecordingAttestation {
    fail_on: Option<usize>,
    subjects: Mutex<Vec<Vec<u8>>>,
}

impl RecordingAttestation {
    fn passing() -> Self {
        Self {
            fail_on: None,
            subjects: Mutex::new(Vec::new()),
        }
    }

    fn subjects(&self) -> Vec<Vec<u8>> {
        match self.subjects.lock() {
            Ok(subjects) => subjects.clone(),
            Err(_) => Vec::new(),
        }
    }
}

impl AttestationVerifier for RecordingAttestation {
    fn verify(
        &self,
        subject: &Path,
        _version: &Version,
        _source_commit: &str,
    ) -> Result<(), String> {
        let bytes = fs::read(subject)
            .map_err(|error| format!("cannot read test subject: {}", io_error(error)))?;
        let index = match self.subjects.lock() {
            Ok(mut subjects) => {
                let index = subjects.len();
                subjects.push(bytes);
                index
            }
            Err(_) => return Err("test attestation recorder is unavailable".to_owned()),
        };
        if self.fail_on == Some(index) {
            Err("test attestation rejected the subject".to_owned())
        } else {
            Ok(())
        }
    }
}

struct AcceptBinary(Vec<u8>);

impl BinaryVerifier for AcceptBinary {
    fn verify(&self, candidate: &Path, version: &Version) -> Result<(), String> {
        if version != &Version::new(1, 1, 0) {
            return Err("test candidate version differs".to_owned());
        }
        let actual = fs::read(candidate)
            .map_err(|error| format!("cannot read test candidate: {}", io_error(error)))?;
        if actual != self.0 {
            return Err("test candidate bytes differ".to_owned());
        }
        Ok(())
    }
}

#[test]
fn version_policy_accepts_stable_and_exact_release_candidates() -> TestResult {
    for value in ["0.0.0", "1.2.3", "1.2.3-rc.1", "1.2.3-rc.42"] {
        assert_eq!(
            parse_version(OsStr::new(value)).map_err(boxed)?.to_string(),
            value
        );
    }
    for value in [
        "1.2",
        "v1.2.3",
        "1.2.3+build",
        "1.2.3-beta.1",
        "1.2.3-rc.0",
        "1.2.3-rc.01",
        "1.2.3-rc.x",
    ] {
        assert!(parse_version(OsStr::new(value)).is_err(), "{value}");
    }
    Ok(())
}

#[test]
fn local_server_update_replaces_binary_and_receipt() -> TestResult {
    let fixture = UpdateFixture::new(false)?;
    let attestation = RecordingAttestation::passing();
    let output = execute(
        &Options {
            check_only: false,
            requested_version: None,
        },
        &fixture.runtime,
        &fixture.client,
        &attestation,
        &AcceptBinary(fixture.candidate.clone()),
    )
    .map_err(boxed)?;

    assert!(output.contains("Updated stack 1.0.0 -> 1.1.0"));
    assert_eq!(
        fs::read(&fixture.runtime.current_executable)?,
        fixture.candidate
    );
    let receipt: InstallationReceipt =
        serde_json::from_slice(&fs::read(&fixture.runtime.receipt_path)?)?;
    assert_eq!(receipt.version, "1.1.0");
    assert_eq!(receipt.source_commit, RELEASE_COMMIT);
    assert_eq!(receipt.binary.sha256, sha256_bytes(&fixture.candidate));
    assert_eq!(
        attestation.subjects(),
        vec![fixture.manifest, fixture.archive]
    );
    assert_eq!(fixture.server.hits().len(), 3);
    assert_eq!(
        fs::read_dir(parent_directory(&fixture.runtime.current_executable).map_err(boxed)?)?
            .count(),
        1
    );
    Ok(())
}

#[test]
fn check_only_resolves_metadata_without_receipt_or_download() -> TestResult {
    let fixture = UpdateFixture::new(false)?;
    fs::remove_file(&fixture.runtime.receipt_path)?;
    let output = execute(
        &Options {
            check_only: true,
            requested_version: None,
        },
        &fixture.runtime,
        &fixture.client,
        &RecordingAttestation::passing(),
        &AcceptBinary(fixture.candidate.clone()),
    )
    .map_err(boxed)?;
    assert!(output.contains("Update available: 1.0.0 -> 1.1.0"));
    assert_eq!(
        fs::read(&fixture.runtime.current_executable)?,
        b"old binary"
    );
    assert_eq!(fixture.server.hits().len(), 1);
    Ok(())
}

#[test]
fn release_metadata_rejects_untrusted_api_values() -> TestResult {
    let valid_digest = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    let cases = [
        (b"{".to_vec(), "invalid release metadata"),
        (
            serde_json::to_vec(&json!({
                "tag_name": "v1.1.0",
                "draft": true,
                "prerelease": false,
                "assets": []
            }))?,
            "still a draft",
        ),
        (
            serde_json::to_vec(&json!({
                "tag_name": "1.1.0",
                "draft": false,
                "prerelease": false,
                "assets": []
            }))?,
            "tag is invalid",
        ),
        (
            serde_json::to_vec(&json!({
                "tag_name": "v1.1.0-rc.1",
                "draft": false,
                "prerelease": true,
                "assets": []
            }))?,
            "unexpectedly selected a prerelease",
        ),
        (
            serde_json::to_vec(&json!({
                "tag_name": "v1.1.0",
                "draft": false,
                "prerelease": true,
                "assets": []
            }))?,
            "prerelease metadata does not match",
        ),
        (
            serde_json::to_vec(&json!({
                "tag_name": "v1.1.0",
                "draft": false,
                "prerelease": false,
                "assets": [{
                    "name": "artifact",
                    "state": "uploaded",
                    "size": 1,
                    "digest": "not-sha256",
                    "browser_download_url": "https://example.com/artifact"
                }]
            }))?,
            "has no SHA-256 digest",
        ),
        (
            serde_json::to_vec(&json!({
                "tag_name": "v1.1.0",
                "draft": false,
                "prerelease": false,
                "assets": [{
                    "name": "artifact",
                    "state": "new",
                    "size": 1,
                    "digest": format!("sha256:{valid_digest}"),
                    "browser_download_url": "https://example.com/artifact"
                }]
            }))?,
            "is not available",
        ),
        (
            serde_json::to_vec(&json!({
                "tag_name": "v1.1.0",
                "draft": false,
                "prerelease": false,
                "assets": [{
                    "name": "artifact",
                    "state": "uploaded",
                    "size": 1,
                    "digest": format!("sha256:{valid_digest}"),
                    "browser_download_url": "https://example.com/artifact"
                }]
            }))?,
            "unexpected download URL",
        ),
    ];

    for (body, expected) in cases {
        let server = LocalServer::start(move |_| {
            BTreeMap::from([(format!("/repos/{REPOSITORY}/releases/latest"), body)])
        })?;
        let error = result_error(ReleaseClient::for_test(server.base()).resolve_release(None))?;
        assert!(error.contains(expected), "{expected}: {error}");
        assert_eq!(server.hits().len(), 1);
    }
    Ok(())
}

#[test]
fn release_resolution_handles_same_newer_and_explicit_rollback_directions() -> TestResult {
    let fixture = UpdateFixture::new(false)?;
    for (release_version, requested_version, check_only, expected) in [
        ("1.0.0", None, false, "already installed"),
        ("0.9.0", None, false, "newer than latest stable"),
        (
            "0.9.0",
            Some(Version::new(0, 9, 0)),
            true,
            "Requested rollback available",
        ),
    ] {
        let release_version = Version::parse(release_version)?;
        let response_version = release_version.clone();
        let server = LocalServer::start(move |base| {
            let response = serde_json::to_vec(&json!({
                "tag_name": format!("v{response_version}"),
                "draft": false,
                "prerelease": false,
                "assets": []
            }))
            .unwrap_or_default();
            BTreeMap::from([
                (
                    format!("/repos/{REPOSITORY}/releases/latest"),
                    response.clone(),
                ),
                (
                    format!("/repos/{REPOSITORY}/releases/tags/v{response_version}"),
                    response,
                ),
                (format!("{base}/unused"), Vec::new()),
            ])
        })?;
        let output = execute(
            &Options {
                check_only,
                requested_version,
            },
            &fixture.runtime,
            &ReleaseClient::for_test(server.base()),
            &RecordingAttestation::passing(),
            &AcceptBinary(fixture.candidate.clone()),
        )
        .map_err(boxed)?;
        assert!(output.contains(expected), "{release_version}: {output}");
        assert_eq!(server.hits().len(), 1);
    }
    Ok(())
}

#[test]
fn tampered_archive_and_failed_attestations_preserve_the_binary() -> TestResult {
    let tampered = UpdateFixture::new(true)?;
    let error = result_error(execute(
        &Options {
            check_only: false,
            requested_version: None,
        },
        &tampered.runtime,
        &tampered.client,
        &RecordingAttestation::passing(),
        &AcceptBinary(tampered.candidate.clone()),
    ))?;
    assert!(
        error.contains("failed SHA-256 verification"),
        "{error}; hits: {:?}",
        tampered.server.hits()
    );
    assert_eq!(
        fs::read(&tampered.runtime.current_executable)?,
        b"old binary"
    );

    for fail_on in [0, 1] {
        let fixture = UpdateFixture::new(false)?;
        let error = result_error(execute(
            &Options {
                check_only: false,
                requested_version: None,
            },
            &fixture.runtime,
            &fixture.client,
            &RecordingAttestation {
                fail_on: Some(fail_on),
                subjects: Mutex::new(Vec::new()),
            },
            &AcceptBinary(fixture.candidate.clone()),
        ))?;
        assert!(error.contains("test attestation rejected"));
        assert_eq!(
            fs::read(&fixture.runtime.current_executable)?,
            b"old binary"
        );
    }
    Ok(())
}

#[test]
fn package_manager_receipts_refuse_before_network_access() -> TestResult {
    for (owner, guidance) in [
        ("homebrew", "brew upgrade"),
        ("aqua", "aqua install"),
        ("cargo", "Cargo"),
        ("unknown", "unsupported owner"),
    ] {
        let fixture = UpdateFixture::new(false)?;
        let mut receipt: InstallationReceipt =
            serde_json::from_slice(&fs::read(&fixture.runtime.receipt_path)?)?;
        receipt.owner = owner.to_owned();
        fs::write(&fixture.runtime.receipt_path, serde_json::to_vec(&receipt)?)?;
        let error = result_error(execute(
            &Options {
                check_only: false,
                requested_version: None,
            },
            &fixture.runtime,
            &fixture.client,
            &RecordingAttestation::passing(),
            &AcceptBinary(fixture.candidate.clone()),
        ))?;
        assert!(error.contains(guidance), "{owner}: {error}");
        assert!(fixture.server.hits().is_empty());
        assert_eq!(
            fs::read(&fixture.runtime.current_executable)?,
            b"old binary"
        );
    }

    for (binary, owner, guidance) in [
        ("Cellar/stack/1.0.0/bin/stack", "homebrew", "brew upgrade"),
        ("aquaproj-aqua/pkgs/stack", "aqua", "aqua install"),
        ("user/.cargo/bin/stack", "cargo", "Cargo"),
    ] {
        let mut fixture = UpdateFixture::new(false)?;
        let managed_binary = fixture._directory.path(binary);
        let parent = parent_directory(&managed_binary).map_err(boxed)?;
        fs::create_dir_all(parent)?;
        fs::rename(&fixture.runtime.current_executable, &managed_binary)?;
        fixture.runtime.current_executable = managed_binary;
        let mut receipt: InstallationReceipt =
            serde_json::from_slice(&fs::read(&fixture.runtime.receipt_path)?)?;
        receipt.owner = "github-release".to_owned();
        receipt.binary.path = fixture
            .runtime
            .current_executable
            .to_str()
            .ok_or("managed test path is not UTF-8")?
            .to_owned();
        fs::write(&fixture.runtime.receipt_path, serde_json::to_vec(&receipt)?)?;

        let error = result_error(execute(
            &Options {
                check_only: false,
                requested_version: None,
            },
            &fixture.runtime,
            &fixture.client,
            &RecordingAttestation::passing(),
            &AcceptBinary(fixture.candidate.clone()),
        ))?;
        assert!(error.contains(guidance), "{owner}: {error}");
        assert!(fixture.server.hits().is_empty());
        assert_eq!(
            fs::read(&fixture.runtime.current_executable)?,
            b"old binary"
        );
    }
    Ok(())
}

struct ReplaceReceiptWithDirectory(PathBuf);

impl BinaryVerifier for ReplaceReceiptWithDirectory {
    fn verify(&self, _candidate: &Path, _version: &Version) -> Result<(), String> {
        fs::remove_file(&self.0).map_err(|error| io_error(error).to_owned())?;
        fs::create_dir(&self.0).map_err(|error| io_error(error).to_owned())
    }
}

#[test]
fn receipt_commit_failure_rolls_back_the_original_binary() -> TestResult {
    let fixture = UpdateFixture::new(false)?;
    let new_receipt = InstallationReceipt::for_release(
        &fixture.runtime,
        &Version::new(1, 1, 0),
        RELEASE_COMMIT,
        &archive_name(&Version::new(1, 1, 0), TARGET),
        &sha256_bytes(&fixture.archive),
        &sha256_bytes(&fixture.candidate),
    )
    .map_err(boxed)?;
    let error = result_error(replace_binary_and_receipt(
        &fixture.runtime,
        &fixture.candidate,
        &new_receipt,
        &ReplaceReceiptWithDirectory(fixture.runtime.receipt_path.clone()),
    ))?;
    assert!(error.contains("original binary was restored"));
    assert_eq!(
        fs::read(&fixture.runtime.current_executable)?,
        b"old binary"
    );
    Ok(())
}

#[cfg(unix)]
#[test]
fn permission_failure_preserves_binary_and_receipt() -> TestResult {
    let fixture = UpdateFixture::new(false)?;
    let old_receipt = fs::read(&fixture.runtime.receipt_path)?;
    let binary_parent = parent_directory(&fixture.runtime.current_executable).map_err(boxed)?;
    let original_permissions = fs::metadata(binary_parent)?.permissions();
    fs::set_permissions(binary_parent, fs::Permissions::from_mode(0o555))?;
    let new_receipt = InstallationReceipt::for_release(
        &fixture.runtime,
        &Version::new(1, 1, 0),
        RELEASE_COMMIT,
        &archive_name(&Version::new(1, 1, 0), TARGET),
        &sha256_bytes(&fixture.archive),
        &sha256_bytes(&fixture.candidate),
    )
    .map_err(boxed)?;
    let result = replace_binary_and_receipt(
        &fixture.runtime,
        &fixture.candidate,
        &new_receipt,
        &AcceptBinary(fixture.candidate.clone()),
    );
    fs::set_permissions(binary_parent, original_permissions)?;
    let error = result_error(result)?;
    assert!(error.contains("permission denied"));
    assert_eq!(
        fs::read(&fixture.runtime.current_executable)?,
        b"old binary"
    );
    assert_eq!(fs::read(&fixture.runtime.receipt_path)?, old_receipt);
    Ok(())
}

#[test]
fn receipt_validation_rejects_tampering_and_improves_missing_guidance() -> TestResult {
    let fixture = UpdateFixture::new(false)?;
    let receipt = read_and_validate_receipt(&fixture.runtime).map_err(boxed)?;
    assert_eq!(receipt.version, "1.0.0");

    let mut cases = Vec::new();
    let mut candidate = receipt.clone();
    candidate.schema_version = 2;
    cases.push((candidate, "unsupported schema"));
    let mut candidate = receipt.clone();
    candidate.version = "1.0.1".to_owned();
    cases.push((candidate, "running binary is stack 1.0.0"));
    let mut candidate = receipt.clone();
    candidate.target = "aarch64-apple-darwin".to_owned();
    cases.push((candidate, "target does not match"));
    let mut candidate = receipt.clone();
    candidate.source_commit = "bad".to_owned();
    cases.push((candidate, "source commit is invalid"));
    let mut candidate = receipt.clone();
    candidate.schema = "https://example.com/schema.json".to_owned();
    cases.push((candidate, "schema URL does not match"));
    let mut candidate = receipt.clone();
    candidate.archive.name = "wrong.tar.gz".to_owned();
    cases.push((candidate, "archive name is inconsistent"));
    let mut candidate = receipt.clone();
    candidate.archive.sha256 = "bad".to_owned();
    cases.push((candidate, "archive SHA-256 digest is invalid"));
    let mut candidate = receipt.clone();
    candidate.binary.sha256 = "bad".to_owned();
    cases.push((candidate, "binary SHA-256 digest is invalid"));
    let mut candidate = receipt.clone();
    candidate.binary.path = "/other/stack".to_owned();
    cases.push((candidate, "not the running executable"));
    for (candidate, expected) in cases {
        let error = result_error(validate_receipt(&fixture.runtime, &candidate))?;
        assert!(error.contains(expected), "{expected}: {error}");
    }

    fs::write(&fixture.runtime.current_executable, b"modified")?;
    let error = result_error(read_and_validate_receipt(&fixture.runtime))?;
    assert!(error.contains("differs from its direct-install receipt"));

    let missing_runtime = Runtime {
        current_version: Version::new(1, 0, 0),
        current_executable: PathBuf::from("/opt/homebrew/Cellar/stack/1.0.0/bin/stack"),
        receipt_path: fixture._directory.path("missing.json"),
        target: TARGET.to_owned(),
    };
    assert!(result_error(read_and_validate_receipt(&missing_runtime))?.contains("brew upgrade"));
    for (binary, guidance) in [
        ("/tmp/aquaproj-aqua/pkgs/stack", "aqua install"),
        ("/tmp/user/.cargo/bin/stack", "Cargo"),
        ("/tmp/custom/stack", "verified direct installer"),
    ] {
        let runtime = Runtime {
            current_executable: PathBuf::from(binary),
            ..missing_runtime.clone()
        };
        assert!(
            result_error(read_and_validate_receipt(&runtime))?.contains(guidance),
            "{binary}"
        );
    }
    Ok(())
}

#[test]
fn manifest_validation_is_strict_and_target_complete() -> TestResult {
    let fixture = UpdateFixture::new(false)?;
    let version = Version::new(1, 1, 0);
    let archive_digest = sha256_bytes(&fixture.archive);
    let release = Release {
        version: version.clone(),
        assets: BTreeMap::new(),
    };
    let validate = |value: &Value, runtime: &Runtime| {
        let bytes = serde_json::to_vec(value).map_err(|error| error.to_string())?;
        validate_manifest(
            &bytes,
            &release,
            runtime,
            &archive_name(&version, TARGET),
            &archive_digest,
        )
        .map(|_| ())
    };
    let valid = manifest_value(&version, &archive_digest);
    validate(&valid, &fixture.runtime).map_err(boxed)?;

    let mut cases = Vec::new();
    let mut value = valid.clone();
    value["source"]["repository"] = json!("other/repository");
    cases.push((value, "unexpected source repository"));
    let mut value = valid.clone();
    value["source"]["commit"] = json!("bad");
    cases.push((value, "source commit is invalid"));
    let mut value = valid.clone();
    value["builderWorkflow"] = json!("other.yaml");
    cases.push((value, "source or workflow evidence"));
    let mut value = valid.clone();
    value["minimumSupportedCliVersion"] = json!("2.0.0");
    cases.push((value, "newer than the release"));
    let mut value = valid.clone();
    value["verifiedChannels"] = json!(["self-update"]);
    cases.push((value, "has not activated"));
    let mut value = valid.clone();
    value["verifiedChannels"] = json!(["self-update", "github-release"]);
    cases.push((value, "verified channels are invalid"));
    let mut value = valid.clone();
    value["verifiedChannels"] = json!(["github-release", "unknown"]);
    cases.push((value, "verified channels are invalid"));
    let mut value = valid.clone();
    value["targets"][0]["target"] = json!("unsupported-target");
    cases.push((value, "archive name is invalid"));
    let mut value = valid.clone();
    value["targets"][0]["archive"]["sha256"] = json!("bad");
    cases.push((value, "archive SHA-256 digest is invalid"));
    let mut value = valid.clone();
    value["targets"][0]["sbom"]["name"] = json!("wrong.json");
    cases.push((value, "SBOM name is invalid"));
    let mut value = valid.clone();
    let first = value["targets"][0].clone();
    value["targets"][1] = first;
    cases.push((value, "duplicate targets"));
    let mut value = valid.clone();
    value["unexpected"] = json!(true);
    cases.push((value, "unsupported fields"));

    for (value, expected) in cases {
        let error = result_error(validate(&value, &fixture.runtime))?;
        assert!(error.contains(expected), "{expected}: {error}");
    }

    let mut too_old = fixture.runtime.clone();
    too_old.current_version = Version::new(0, 9, 0);
    assert!(result_error(validate(&valid, &too_old))?.contains("minimum supported updater"));
    Ok(())
}

#[test]
fn archive_validation_checks_metadata_and_exact_layout() -> TestResult {
    let version = Version::new(1, 1, 0);
    let candidate = b"candidate";
    let archive = release_archive(&version, candidate)?;
    assert_eq!(
        extract_binary(&archive, &version, TARGET, EPOCH).map_err(boxed)?,
        candidate
    );
    assert!(
        result_error(extract_binary(&archive, &version, TARGET, EPOCH + 1))?.contains("timestamp")
    );
    let mut damaged = archive;
    damaged.truncate(damaged.len() / 2);
    assert!(extract_binary(&damaged, &version, TARGET, EPOCH).is_err());
    Ok(())
}

#[cfg(unix)]
#[test]
fn command_verifiers_enforce_identity_and_embedded_version() -> TestResult {
    let directory = TestDirectory::new("commands")?;
    let log = directory.path("arguments.txt");
    let gh = directory.path("gh");
    fs::write(
        &gh,
        format!(
            "#!/bin/sh\nprintf 'GH_HOST=%s\\nGH_PROMPT_DISABLED=%s\\n' \"$GH_HOST\" \"$GH_PROMPT_DISABLED\" > '{}'\nprintf '%s\\n' \"$@\" >> '{}'\n",
            log.display(),
            log.display()
        ),
    )?;
    fs::set_permissions(&gh, fs::Permissions::from_mode(0o755))?;
    let subject = directory.path("subject");
    fs::write(&subject, b"subject")?;
    GitHubAttestationVerifier {
        command: gh.clone(),
    }
    .verify(&subject, &Version::new(1, 2, 3), RELEASE_COMMIT)
    .map_err(boxed)?;
    let arguments = fs::read_to_string(log)?;
    assert_eq!(
        arguments,
        format!(
            "GH_HOST=github.com\nGH_PROMPT_DISABLED=1\nattestation\nverify\n{}\n--repo\nstack-sh/cli\n--cert-identity\nhttps://github.com/stack-sh/cli/.github/workflows/release.yaml@refs/tags/v1.2.3\n--cert-oidc-issuer\nhttps://token.actions.githubusercontent.com\n--deny-self-hosted-runners\n--source-ref\nrefs/tags/v1.2.3\n--source-digest\n{RELEASE_COMMIT}\n--predicate-type\nhttps://slsa.dev/provenance/v1\n--limit\n5\n",
            subject.display()
        )
    );

    let missing = GitHubAttestationVerifier {
        command: directory.path("missing-gh"),
    };
    assert!(
        result_error(missing.verify(&subject, &Version::new(1, 2, 3), RELEASE_COMMIT))?
            .contains("GitHub CLI")
    );

    let binary = directory.path("candidate");
    fs::write(&binary, b"#!/bin/sh\nprintf 'stack 1.2.3\\n'\n")?;
    fs::set_permissions(&binary, fs::Permissions::from_mode(0o755))?;
    ExecutableVersionVerifier
        .verify(&binary, &Version::new(1, 2, 3))
        .map_err(boxed)?;
    assert!(
        result_error(ExecutableVersionVerifier.verify(&binary, &Version::new(1, 2, 4)))?
            .contains("did not report")
    );

    let fixture = UpdateFixture::new(false)?;
    let error = result_error(run_integration_test(
        "1.0.0",
        &fixture.runtime.current_executable,
        &fixture.runtime.receipt_path,
        TARGET,
        fixture.server.base(),
        &gh,
    ))?;
    assert!(error.contains("verified update candidate"), "{error}");
    Ok(())
}

#[test]
fn exact_current_version_is_a_network_free_noop() -> TestResult {
    let fixture = UpdateFixture::new(false)?;
    let output = execute(
        &Options {
            check_only: false,
            requested_version: Some(fixture.runtime.current_version.clone()),
        },
        &fixture.runtime,
        &fixture.client,
        &RecordingAttestation::passing(),
        &AcceptBinary(fixture.candidate.clone()),
    )
    .map_err(boxed)?;
    assert!(output.contains("already installed"));
    assert!(fixture.server.hits().is_empty());
    assert!(
        run_integration_noop()
            .map_err(boxed)?
            .contains("already installed")
    );
    Ok(())
}

#[test]
fn helper_validation_is_fail_closed() -> TestResult {
    assert!(validate_digest(&"a".repeat(64), "test").is_ok());
    assert!(validate_digest(&"A".repeat(64), "test").is_err());
    assert!(validate_commit(&"a".repeat(40), "test").is_ok());
    assert!(validate_commit(&"g".repeat(40), "test").is_err());
    assert_eq!(digest_hex(&[0, 15, 255]), "000fff");
    assert_eq!(
        io_error(io::Error::from(io::ErrorKind::NotFound)),
        "file not found"
    );
    assert_eq!(
        io_error(io::Error::from(io::ErrorKind::PermissionDenied)),
        "permission denied"
    );
    assert_eq!(
        io_error(io::Error::from(io::ErrorKind::AlreadyExists)),
        "already exists"
    );
    assert_eq!(
        io_error(io::Error::from(io::ErrorKind::InvalidInput)),
        "invalid input"
    );
    assert_eq!(io_error(io::Error::other("private")), "I/O error");
    assert_eq!(
        managed_path_owner(Path::new("/opt/homebrew/Cellar/stack/1/bin/stack")),
        Some("homebrew")
    );
    assert_eq!(
        managed_path_owner(Path::new("/tmp/aquaproj-aqua/pkgs/stack")),
        Some("aqua")
    );
    assert_eq!(
        managed_path_owner(Path::new("/tmp/user/.cargo/bin/stack")),
        Some("cargo")
    );
    assert_eq!(managed_path_owner(Path::new("/opt/stack/bin/stack")), None);
    Ok(())
}
