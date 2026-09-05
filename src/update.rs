//! Verified self-update for receipt-owned direct installations.

use std::collections::{BTreeMap, BTreeSet};
use std::env;
use std::ffi::OsStr;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

use semver::Version;
use serde::Deserialize;

use crate::config;

const REPOSITORY: &str = "stack-sh/cli";
const RELEASE_WORKFLOW: &str = "stack-sh/cli/.github/workflows/release.yaml";
const API_VERSION: &str = "2026-03-10";
const RECEIPT_SCHEMA_VERSION: u8 = 1;
const MAX_RELEASE_RESPONSE_BYTES: u64 = 1024 * 1024;
const MAX_MANIFEST_BYTES: u64 = 1024 * 1024;
const MAX_RECEIPT_BYTES: u64 = 64 * 1024;
const MAX_ARCHIVE_BYTES: u64 = 256 * 1024 * 1024;
const MAX_BINARY_BYTES: u64 = 256 * 1024 * 1024;
const SUPPORTED_TARGETS: [&str; 4] = [
    "aarch64-apple-darwin",
    "aarch64-unknown-linux-gnu",
    "x86_64-apple-darwin",
    "x86_64-unknown-linux-gnu",
];

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Options {
    pub(crate) check_only: bool,
    pub(crate) requested_version: Option<Version>,
}

pub(crate) fn parse_version(value: &OsStr) -> Result<Version, String> {
    let Some(value) = value.to_str() else {
        return Err("update version must be valid UTF-8".to_owned());
    };
    let version = match Version::parse(value) {
        Ok(version) => version,
        Err(_) => {
            return Err(
                "update version must be MAJOR.MINOR.PATCH or MAJOR.MINOR.PATCH-rc.N".to_owned(),
            );
        }
    };
    if !version.build.is_empty() {
        return Err("update version must not contain build metadata".to_owned());
    }
    if !version.pre.is_empty() {
        let prerelease = version.pre.as_str();
        let Some(sequence) = prerelease.strip_prefix("rc.") else {
            return Err(
                "only an exact MAJOR.MINOR.PATCH-rc.N prerelease can be requested".to_owned(),
            );
        };
        if sequence.is_empty()
            || !sequence.bytes().all(|byte| byte.is_ascii_digit())
            || sequence == "0"
        {
            return Err(
                "only an exact MAJOR.MINOR.PATCH-rc.N prerelease can be requested".to_owned(),
            );
        }
    }
    Ok(version)
}

pub(crate) fn run(options: Options, environment: &config::Environment) -> Result<String, String> {
    let current_version = match Version::parse(env!("CARGO_PKG_VERSION")) {
        Ok(version) => version,
        Err(_) => return Err("the running CLI has an invalid embedded version".to_owned()),
    };
    let current_executable = match env::current_exe().and_then(fs::canonicalize) {
        Ok(executable) => executable,
        Err(error) => {
            return Err(format!(
                "cannot resolve the running executable: {}",
                io_error(error)
            ));
        }
    };
    let receipt_path = config::installation_receipt_path(environment)?;
    let runtime = Runtime {
        current_version,
        current_executable,
        receipt_path,
        target: host_target()?,
    };
    execute(
        &options,
        &runtime,
        &ReleaseClient::production(),
        &GitHubAttestationVerifier::production(),
        &ExecutableVersionVerifier,
    )
}

#[cfg(test)]
pub(crate) fn run_integration_test(
    current_version: &str,
    current_executable: &Path,
    receipt_path: &Path,
    target: &str,
    server_base: &str,
    gh_command: &Path,
) -> Result<String, String> {
    let runtime = Runtime {
        current_version: parse_version(OsStr::new(current_version))?,
        current_executable: match fs::canonicalize(current_executable) {
            Ok(executable) => executable,
            Err(error) => {
                return Err(format!(
                    "cannot resolve test executable: {}",
                    io_error(error)
                ));
            }
        },
        receipt_path: receipt_path.to_owned(),
        target: target.to_owned(),
    };
    execute(
        &Options {
            check_only: false,
            requested_version: None,
        },
        &runtime,
        &ReleaseClient::for_debug(server_base),
        &GitHubAttestationVerifier {
            command: gh_command.to_owned(),
        },
        &ExecutableVersionVerifier,
    )
}

#[cfg(test)]
pub(crate) fn run_integration_noop() -> Result<String, String> {
    run(
        Options {
            check_only: true,
            requested_version: Some(match Version::parse(env!("CARGO_PKG_VERSION")) {
                Ok(version) => version,
                Err(_) => return Err("the test package version is invalid".to_owned()),
            }),
        },
        &config::Environment::capture(),
    )
}

#[derive(Clone, Debug)]
struct Runtime {
    current_version: Version,
    current_executable: PathBuf,
    receipt_path: PathBuf,
    target: String,
}

fn host_target() -> Result<String, String> {
    match (env::consts::OS, env::consts::ARCH) {
        ("macos", "aarch64") => Ok("aarch64-apple-darwin".to_owned()),
        ("macos", "x86_64") => Ok("x86_64-apple-darwin".to_owned()),
        ("linux", "aarch64") if cfg!(target_env = "gnu") => {
            Ok("aarch64-unknown-linux-gnu".to_owned())
        }
        ("linux", "x86_64") if cfg!(target_env = "gnu") => {
            Ok("x86_64-unknown-linux-gnu".to_owned())
        }
        _ => Err(format!(
            "self-update is unsupported on {}/{}; install with a supported package manager",
            env::consts::OS,
            env::consts::ARCH
        )),
    }
}

fn execute(
    options: &Options,
    runtime: &Runtime,
    client: &ReleaseClient,
    attestation_verifier: &dyn AttestationVerifier,
    binary_verifier: &dyn BinaryVerifier,
) -> Result<String, String> {
    if options
        .requested_version
        .as_ref()
        .is_some_and(|version| version == &runtime.current_version)
    {
        return Ok(format!(
            "stack {} is already installed; no files were changed.\n",
            runtime.current_version
        ));
    }
    if !options.check_only {
        read_and_validate_receipt(runtime)?;
    }

    let release = client.resolve_release(options.requested_version.as_ref())?;
    let comparison = release.version.cmp(&runtime.current_version);

    if comparison.is_eq() {
        return Ok(format!(
            "stack {} is already installed; no files were changed.\n",
            runtime.current_version
        ));
    }
    if options.requested_version.is_none() && comparison.is_lt() {
        return Ok(format!(
            "stack {} is newer than latest stable {}; no files were changed.\n",
            runtime.current_version, release.version
        ));
    }
    if options.check_only {
        let direction = if comparison.is_gt() {
            "Update available"
        } else {
            "Requested rollback available"
        };
        return Ok(format!(
            "{direction}: {} -> {} for {}. No files were changed.\n",
            runtime.current_version, release.version, runtime.target
        ));
    }

    let manifest_name = format!("stack-v{}-release-manifest.json", release.version);
    let archive_name = archive_name(&release.version, &runtime.target);
    let manifest_asset = release.asset(&manifest_name, MAX_MANIFEST_BYTES)?;
    let archive_asset = release.asset(&archive_name, MAX_ARCHIVE_BYTES)?;
    let manifest_bytes = client.download_asset(manifest_asset, MAX_MANIFEST_BYTES)?;
    let manifest = validate_manifest(
        &manifest_bytes,
        &release,
        runtime,
        &archive_name,
        &archive_asset.digest,
    )?;
    let executable_parent = parent_directory(&runtime.current_executable)?;
    let manifest_file =
        TemporaryFile::write(executable_parent, "update-manifest", &manifest_bytes, None)?;
    attestation_verifier.verify(
        manifest_file.path(),
        &release.version,
        &manifest.source.commit,
    )?;

    let archive_bytes = client.download_asset(archive_asset, MAX_ARCHIVE_BYTES)?;
    let archive_file =
        TemporaryFile::write(executable_parent, "update-archive", &archive_bytes, None)?;
    attestation_verifier.verify(
        archive_file.path(),
        &release.version,
        &manifest.source.commit,
    )?;

    let candidate = extract_binary(
        &archive_bytes,
        &release.version,
        &runtime.target,
        manifest.source_date_epoch,
    )?;
    let candidate_digest = sha256_bytes(&candidate);
    let new_receipt = InstallationReceipt::for_release(
        runtime,
        &release.version,
        &manifest.source.commit,
        &archive_name,
        &archive_asset.digest,
        &candidate_digest,
    )?;
    let warning = replace_binary_and_receipt(runtime, &candidate, &new_receipt, binary_verifier)?;

    let action = if comparison.is_gt() {
        "Updated"
    } else {
        "Rolled back"
    };
    let mut message = format!(
        "{action} stack {} -> {} for {}. Restart running language server processes.\n",
        runtime.current_version, release.version, runtime.target
    );
    if let Some(warning) = warning {
        message.push_str(&format!("Warning: {warning}\n"));
    }
    Ok(message)
}

#[derive(Clone)]
struct ReleaseClient {
    agent: ureq::Agent,
    api_base: String,
    asset_base: String,
}

impl ReleaseClient {
    fn production() -> Self {
        #[cfg(debug_assertions)]
        if let Some(value) = env::var_os("STACK_CLI_TEST_UPDATE_BASE_URL") {
            if let Ok(base) = value.into_string() {
                if base.starts_with("http://127.0.0.1:") || base.starts_with("http://[::1]:") {
                    return Self::for_debug(&base);
                }
            }
        }
        let config = ureq::Agent::config_builder()
            .https_only(true)
            .timeout_global(Some(Duration::from_secs(120)))
            .build();
        Self {
            agent: ureq::Agent::new_with_config(config),
            api_base: "https://api.github.com".to_owned(),
            asset_base: "https://github.com/stack-sh/cli/releases/download".to_owned(),
        }
    }

    #[cfg(debug_assertions)]
    fn for_debug(base: &str) -> Self {
        let config = ureq::Agent::config_builder()
            .https_only(false)
            .timeout_global(Some(Duration::from_secs(5)))
            .build();
        Self {
            agent: ureq::Agent::new_with_config(config),
            api_base: base.to_owned(),
            asset_base: format!("{base}/download"),
        }
    }

    #[cfg(test)]
    fn for_test(base: &str) -> Self {
        Self::for_debug(base)
    }

    fn resolve_release(&self, requested: Option<&Version>) -> Result<Release, String> {
        let endpoint = match requested {
            Some(version) => format!("/repos/{REPOSITORY}/releases/tags/v{version}"),
            None => format!("/repos/{REPOSITORY}/releases/latest"),
        };
        let bytes = self.get(
            &format!("{}{endpoint}", self.api_base),
            MAX_RELEASE_RESPONSE_BYTES,
            true,
        )?;
        let response: ApiRelease = match serde_json::from_slice(&bytes) {
            Ok(response) => response,
            Err(_) => return Err("GitHub returned invalid release metadata".to_owned()),
        };
        if response.draft {
            return Err("the selected GitHub release is still a draft".to_owned());
        }
        let Some(version_text) = response.tag_name.strip_prefix('v') else {
            return Err("the selected GitHub release tag is invalid".to_owned());
        };
        let version = parse_version(OsStr::new(version_text))?;
        if let Some(requested) = requested {
            if requested != &version {
                return Err("GitHub returned a different release version than requested".to_owned());
            }
        }
        if requested.is_none() && !version.pre.is_empty() {
            return Err("GitHub latest release unexpectedly selected a prerelease".to_owned());
        }
        if response.prerelease == version.pre.is_empty() {
            return Err("GitHub release prerelease metadata does not match its tag".to_owned());
        }

        let mut assets = BTreeMap::new();
        for asset in response.assets {
            let Some(digest) = asset.digest.strip_prefix("sha256:") else {
                return Err(format!(
                    "release asset '{}' has no SHA-256 digest",
                    asset.name
                ));
            };
            validate_digest(digest, &format!("release asset '{}'", asset.name))?;
            if asset.state != "uploaded" || asset.size == 0 {
                return Err(format!("release asset '{}' is not available", asset.name));
            }
            let expected_url = format!("{}/v{}/{}", self.asset_base, version, asset.name);
            if asset.browser_download_url != expected_url {
                return Err(format!(
                    "release asset '{}' has an unexpected download URL",
                    asset.name
                ));
            }
            let name = asset.name.clone();
            if assets
                .insert(
                    name.clone(),
                    ReleaseAsset {
                        name,
                        url: asset.browser_download_url,
                        size: asset.size,
                        digest: digest.to_owned(),
                    },
                )
                .is_some()
            {
                return Err("GitHub release metadata contains duplicate assets".to_owned());
            }
        }
        Ok(Release { version, assets })
    }

    fn download_asset(&self, asset: &ReleaseAsset, limit: u64) -> Result<Vec<u8>, String> {
        if asset.size > limit {
            return Err(format!(
                "release asset '{}' exceeds the {} byte limit",
                asset.name, limit
            ));
        }
        let bytes = self.get(&asset.url, limit, false)?;
        if bytes.len() as u64 != asset.size {
            return Err(format!(
                "release asset '{}' size differs from GitHub metadata",
                asset.name
            ));
        }
        if sha256_bytes(&bytes) != asset.digest {
            return Err(format!(
                "release asset '{}' failed SHA-256 verification",
                asset.name
            ));
        }
        Ok(bytes)
    }

    fn get(&self, url: &str, limit: u64, api: bool) -> Result<Vec<u8>, String> {
        let mut request = self
            .agent
            .get(url)
            .header(
                "User-Agent",
                concat!("stack-cli/", env!("CARGO_PKG_VERSION")),
            )
            .header("Accept", "application/vnd.github+json");
        if api {
            request = request.header("X-GitHub-Api-Version", API_VERSION);
        }
        let mut response = match request.call() {
            Ok(response) => response,
            Err(error) => {
                return Err(format!(
                    "cannot download release metadata or artifact: {error}"
                ));
            }
        };
        match response.body_mut().with_config().limit(limit).read_to_vec() {
            Ok(bytes) => Ok(bytes),
            Err(error) => Err(format!("cannot read release response: {error}")),
        }
    }
}

#[derive(Debug, Deserialize)]
struct ApiRelease {
    tag_name: String,
    draft: bool,
    prerelease: bool,
    assets: Vec<ApiAsset>,
}

#[derive(Debug, Deserialize)]
struct ApiAsset {
    name: String,
    state: String,
    size: u64,
    digest: String,
    browser_download_url: String,
}

#[derive(Debug)]
struct Release {
    version: Version,
    assets: BTreeMap<String, ReleaseAsset>,
}

impl Release {
    fn asset(&self, name: &str, limit: u64) -> Result<&ReleaseAsset, String> {
        let asset = match self.assets.get(name) {
            Some(asset) => asset,
            None => return Err(format!("GitHub release is missing required asset '{name}'")),
        };
        if asset.size > limit {
            return Err(format!(
                "release asset '{name}' exceeds the {limit} byte limit"
            ));
        }
        Ok(asset)
    }
}

#[derive(Debug)]
struct ReleaseAsset {
    name: String,
    url: String,
    size: u64,
    digest: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ReleaseManifest {
    #[serde(rename = "$schema")]
    schema: String,
    schema_version: u8,
    version: String,
    tag: String,
    source: ManifestSource,
    minimum_supported_cli_version: String,
    source_date_epoch: u64,
    builder_workflow: String,
    verified_channels: Vec<String>,
    targets: Vec<ManifestTarget>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ManifestSource {
    repository: String,
    commit: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ManifestTarget {
    target: String,
    archive: ManifestFile,
    sbom: ManifestFile,
    provenance: ManifestFile,
    sbom_attestation: ManifestFile,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ManifestFile {
    name: String,
    sha256: String,
}

fn validate_manifest(
    bytes: &[u8],
    release: &Release,
    runtime: &Runtime,
    selected_archive_name: &str,
    archive_digest: &str,
) -> Result<ReleaseManifest, String> {
    let manifest: ReleaseManifest = match serde_json::from_slice(bytes) {
        Ok(manifest) => manifest,
        Err(_) => {
            return Err("release manifest is invalid JSON or has unsupported fields".to_owned());
        }
    };
    if manifest.schema_version != 1
        || manifest.version != release.version.to_string()
        || manifest.tag != format!("v{}", release.version)
    {
        return Err("release manifest version metadata is inconsistent".to_owned());
    }
    if manifest.source.repository != REPOSITORY {
        return Err("release manifest names an unexpected source repository".to_owned());
    }
    validate_commit(&manifest.source.commit, "release manifest source commit")?;
    let expected_schema = format!(
        "https://raw.githubusercontent.com/{}/{}/distribution/release-manifest.schema.json",
        REPOSITORY, manifest.source.commit
    );
    if manifest.schema != expected_schema || manifest.builder_workflow != RELEASE_WORKFLOW {
        return Err("release manifest source or workflow evidence is inconsistent".to_owned());
    }
    let minimum = parse_version(OsStr::new(&manifest.minimum_supported_cli_version))?;
    if minimum > release.version {
        return Err("release minimum supported CLI version is newer than the release".to_owned());
    }
    if runtime.current_version < minimum {
        return Err(format!(
            "stack {} cannot self-update to {}; minimum supported updater is {}",
            runtime.current_version, release.version, minimum
        ));
    }
    let channels: BTreeSet<&str> = manifest
        .verified_channels
        .iter()
        .map(String::as_str)
        .collect();
    if channels.len() != manifest.verified_channels.len()
        || manifest
            .verified_channels
            .windows(2)
            .any(|pair| pair[0] >= pair[1])
        || channels.iter().any(|channel| {
            !matches!(
                *channel,
                "github-release" | "homebrew" | "cargo" | "aqua" | "self-update"
            )
        })
    {
        return Err("release manifest verified channels are invalid".to_owned());
    }
    if !channels.contains("github-release") || !channels.contains("self-update") {
        return Err("the selected release has not activated the self-update channel".to_owned());
    }
    let mut targets = BTreeSet::new();
    for target in &manifest.targets {
        if !targets.insert(target.target.as_str()) {
            return Err("release manifest contains duplicate targets".to_owned());
        }
        validate_manifest_file(
            &target.archive,
            "archive",
            &archive_name(&release.version, &target.target),
        )?;
        validate_manifest_file(
            &target.sbom,
            "SBOM",
            &format!("stack-v{}-{}.spdx.json", release.version, target.target),
        )?;
        validate_manifest_file(
            &target.provenance,
            "provenance",
            &format!(
                "stack-v{}-{}.provenance.sigstore.json",
                release.version, target.target
            ),
        )?;
        validate_manifest_file(
            &target.sbom_attestation,
            "SBOM attestation",
            &format!(
                "stack-v{}-{}.sbom.sigstore.json",
                release.version, target.target
            ),
        )?;
    }
    if targets != SUPPORTED_TARGETS.into_iter().collect() {
        return Err("release manifest must contain exactly the four supported targets".to_owned());
    }
    let mut selected_target = None;
    for target in &manifest.targets {
        if target.target == runtime.target {
            selected_target = Some(target);
            break;
        }
    }
    let target = match selected_target {
        Some(target) => target,
        None => return Err("release manifest does not support this host target".to_owned()),
    };
    if target.archive.name != selected_archive_name || target.archive.sha256 != archive_digest {
        return Err("release manifest archive identity differs from GitHub metadata".to_owned());
    }
    Ok(manifest)
}

fn validate_manifest_file(
    file: &ManifestFile,
    label: &str,
    expected_name: &str,
) -> Result<(), String> {
    if file.name != expected_name {
        return Err(format!("release manifest {label} name is invalid"));
    }
    validate_digest(&file.sha256, &format!("release manifest {label}"))
}

trait AttestationVerifier {
    fn verify(&self, archive: &Path, version: &Version, source_commit: &str) -> Result<(), String>;
}

struct GitHubAttestationVerifier {
    command: PathBuf,
}

impl GitHubAttestationVerifier {
    fn production() -> Self {
        Self {
            command: PathBuf::from("gh"),
        }
    }
}

impl AttestationVerifier for GitHubAttestationVerifier {
    fn verify(&self, archive: &Path, version: &Version, source_commit: &str) -> Result<(), String> {
        let tag_ref = format!("refs/tags/v{version}");
        let certificate_identity =
            format!("https://github.com/{REPOSITORY}/.github/workflows/release.yaml@{tag_ref}");
        let outcome = Command::new(&self.command)
            .arg("attestation")
            .arg("verify")
            .arg(archive)
            .arg("--repo")
            .arg(REPOSITORY)
            .arg("--cert-identity")
            .arg(certificate_identity)
            .arg("--cert-oidc-issuer")
            .arg("https://token.actions.githubusercontent.com")
            .arg("--deny-self-hosted-runners")
            .arg("--source-ref")
            .arg(tag_ref)
            .arg("--source-digest")
            .arg(source_commit)
            .arg("--predicate-type")
            .arg("https://slsa.dev/provenance/v1")
            .arg("--limit")
            .arg("5")
            .env("GH_HOST", "github.com")
            .env("GH_PROMPT_DISABLED", "1")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
        match outcome {
            Ok(status) if status.success() => Ok(()),
            Ok(_) => Err(
                "GitHub artifact attestation verification failed; the existing binary was not changed"
                    .to_owned(),
            ),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Err(
                "GitHub CLI with `gh attestation verify` is required for self-update".to_owned(),
            ),
            Err(error) => Err(format!(
                "cannot run GitHub artifact attestation verification: {}",
                io_error(error)
            )),
        }
    }
}

mod install;
use install::*;

#[cfg(test)]
#[path = "update/tests.rs"]
mod tests;
