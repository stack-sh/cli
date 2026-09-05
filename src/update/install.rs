//! Receipt, archive, and local replacement boundaries for self-update.

use std::ffi::OsStr;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Cursor, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use flate2::read::GzDecoder;
use semver::Version;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::{
    MAX_ARCHIVE_BYTES, MAX_BINARY_BYTES, MAX_RECEIPT_BYTES, RECEIPT_SCHEMA_VERSION, REPOSITORY,
    Runtime, parse_version,
};

pub(super) trait BinaryVerifier {
    fn verify(&self, candidate: &Path, version: &Version) -> Result<(), String>;
}

pub(super) struct ExecutableVersionVerifier;

impl BinaryVerifier for ExecutableVersionVerifier {
    fn verify(&self, candidate: &Path, version: &Version) -> Result<(), String> {
        let outcome = Command::new(candidate)
            .arg("--version")
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .output();
        let output = match outcome {
            Ok(output) => output,
            Err(error) => {
                return Err(format!(
                    "cannot execute verified update candidate: {}",
                    io_error(error)
                ));
            }
        };
        if !output.status.success() || output.stdout != format!("stack {version}\n").as_bytes() {
            return Err(
                "verified update candidate did not report the selected version; the existing binary was not changed"
                    .to_owned(),
            );
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct InstallationReceipt {
    #[serde(rename = "$schema")]
    pub(super) schema: String,
    pub(super) schema_version: u8,
    pub(super) owner: String,
    pub(super) repository: String,
    pub(super) version: String,
    pub(super) target: String,
    pub(super) source_commit: String,
    pub(super) archive: ReceiptArtifact,
    pub(super) binary: ReceiptBinary,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ReceiptArtifact {
    pub(super) name: String,
    pub(super) sha256: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ReceiptBinary {
    pub(super) path: String,
    pub(super) sha256: String,
}

impl InstallationReceipt {
    pub(super) fn for_release(
        runtime: &Runtime,
        version: &Version,
        source_commit: &str,
        archive_name: &str,
        archive_digest: &str,
        binary_digest: &str,
    ) -> Result<Self, String> {
        let Some(executable_path) = runtime.current_executable.to_str() else {
            return Err("the executable path is not valid UTF-8".to_owned());
        };
        Ok(Self {
            schema: format!(
                "https://raw.githubusercontent.com/{REPOSITORY}/{source_commit}/distribution/install-receipt.schema.json"
            ),
            schema_version: RECEIPT_SCHEMA_VERSION,
            owner: "github-release".to_owned(),
            repository: REPOSITORY.to_owned(),
            version: version.to_string(),
            target: runtime.target.clone(),
            source_commit: source_commit.to_owned(),
            archive: ReceiptArtifact {
                name: archive_name.to_owned(),
                sha256: archive_digest.to_owned(),
            },
            binary: ReceiptBinary {
                path: executable_path.to_owned(),
                sha256: binary_digest.to_owned(),
            },
        })
    }
}

pub(super) fn read_and_validate_receipt(runtime: &Runtime) -> Result<InstallationReceipt, String> {
    let metadata = match fs::symlink_metadata(&runtime.receipt_path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return Err(missing_receipt_guidance(runtime));
        }
        Err(error) => {
            return Err(format!(
                "cannot read install receipt '{}': {}",
                runtime.receipt_path.display(),
                io_error(error)
            ));
        }
    };
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(format!(
            "install receipt '{}' must be a regular file, not a symlink",
            runtime.receipt_path.display()
        ));
    }
    if metadata.len() == 0 || metadata.len() > MAX_RECEIPT_BYTES {
        return Err(format!(
            "install receipt '{}' must be between 1 byte and 64 KiB",
            runtime.receipt_path.display()
        ));
    }
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    let mut file = match File::open(&runtime.receipt_path) {
        Ok(file) => file,
        Err(error) => {
            return Err(format!(
                "cannot read install receipt '{}': {}",
                runtime.receipt_path.display(),
                io_error(error)
            ));
        }
    };
    if let Err(error) = file.read_to_end(&mut bytes) {
        return Err(format!(
            "cannot read install receipt '{}': {}",
            runtime.receipt_path.display(),
            io_error(error)
        ));
    }
    let receipt: InstallationReceipt = match serde_json::from_slice(&bytes) {
        Ok(receipt) => receipt,
        Err(_) => {
            return Err(format!(
                "install receipt '{}' is invalid JSON or has unsupported fields",
                runtime.receipt_path.display()
            ));
        }
    };
    validate_receipt(runtime, &receipt)?;
    Ok(receipt)
}

pub(super) fn validate_receipt(
    runtime: &Runtime,
    receipt: &InstallationReceipt,
) -> Result<(), String> {
    if let Some(owner) = managed_path_owner(&runtime.current_executable) {
        return Err(owner_guidance(owner));
    }
    if receipt.owner != "github-release" {
        return Err(owner_guidance(&receipt.owner));
    }
    if receipt.schema_version != RECEIPT_SCHEMA_VERSION || receipt.repository != REPOSITORY {
        return Err("install receipt has an unsupported schema or repository owner".to_owned());
    }
    let version = parse_version(OsStr::new(&receipt.version))?;
    if version != runtime.current_version {
        return Err(format!(
            "install receipt records stack {}, but the running binary is stack {}; no files were changed",
            version, runtime.current_version
        ));
    }
    if receipt.target != runtime.target {
        return Err("install receipt target does not match the running binary".to_owned());
    }
    validate_commit(&receipt.source_commit, "install receipt source commit")?;
    let expected_schema = format!(
        "https://raw.githubusercontent.com/{REPOSITORY}/{}/distribution/install-receipt.schema.json",
        receipt.source_commit
    );
    if receipt.schema != expected_schema {
        return Err("install receipt schema URL does not match its source commit".to_owned());
    }
    if receipt.archive.name != archive_name(&version, &runtime.target) {
        return Err("install receipt archive name is inconsistent".to_owned());
    }
    validate_digest(&receipt.archive.sha256, "install receipt archive")?;
    validate_digest(&receipt.binary.sha256, "install receipt binary")?;
    let Some(expected_path) = runtime.current_executable.to_str() else {
        return Err("the executable path is not valid UTF-8".to_owned());
    };
    if receipt.binary.path != expected_path {
        return Err(format!(
            "install receipt belongs to '{}', not the running executable; no files were changed",
            receipt.binary.path
        ));
    }
    let actual_digest = sha256_file(&runtime.current_executable, MAX_BINARY_BYTES)?;
    if actual_digest != receipt.binary.sha256 {
        return Err(
            "the running executable differs from its direct-install receipt; no files were changed"
                .to_owned(),
        );
    }
    Ok(())
}

pub(super) fn missing_receipt_guidance(runtime: &Runtime) -> String {
    let guidance = match managed_path_owner(&runtime.current_executable) {
        Some("homebrew") => {
            "This path appears to be owned by Homebrew; run `brew upgrade stack-sh/tap/stack`."
        }
        Some("aqua") => {
            "This path appears to be owned by Aqua; update the version and checksum lock, then run `aqua install`."
        }
        Some("cargo") => {
            "This path appears to be owned by Cargo; reinstall it through the Cargo package that installed `stack`."
        }
        _ => {
            "Use the verified direct installer to create a receipt. Homebrew users should run `brew upgrade stack-sh/tap/stack`; Aqua users should update their version and checksum lock, then run `aqua install`; Cargo users should reinstall through the owning package."
        }
    };
    format!(
        "no eligible direct-install receipt exists at '{}'; no files were changed. {guidance}",
        runtime.receipt_path.display()
    )
}

pub(super) fn managed_path_owner(executable: &Path) -> Option<&'static str> {
    let executable = executable.to_string_lossy();
    if executable.contains("/Cellar/")
        || executable.contains("/homebrew/")
        || executable.contains("/linuxbrew/")
    {
        Some("homebrew")
    } else if executable.contains("/aquaproj-aqua/") || executable.contains("/aqua/pkgs/") {
        Some("aqua")
    } else if executable.contains("/.cargo/bin/") {
        Some("cargo")
    } else {
        None
    }
}

pub(super) fn owner_guidance(owner: &str) -> String {
    match owner {
        "homebrew" => {
            "this executable is owned by Homebrew; run `brew upgrade stack-sh/tap/stack`"
                .to_owned()
        }
        "aqua" => "this executable is owned by Aqua; update the version and checksum lock, then run `aqua install`".to_owned(),
        "cargo" => "this executable is owned by Cargo; reinstall it through the Cargo package that created the receipt".to_owned(),
        _ => "the install receipt names an unsupported owner; no files were changed".to_owned(),
    }
}

pub(super) fn archive_name(version: &Version, target: &str) -> String {
    format!("stack-v{version}-{target}.tar.gz")
}

pub(super) fn extract_binary(
    archive_bytes: &[u8],
    version: &Version,
    target: &str,
    source_date_epoch: u64,
) -> Result<Vec<u8>, String> {
    let root = format!("stack-v{version}-{target}");
    let expected = [
        root.clone(),
        format!("{root}/LICENSE"),
        format!("{root}/NOTICE"),
        format!("{root}/THIRD_PARTY_LICENSES.md"),
        format!("{root}/stack"),
    ];
    let decoder = GzDecoder::new(Cursor::new(archive_bytes));
    let mut archive = tar::Archive::new(decoder);
    let entries = match archive.entries() {
        Ok(entries) => entries,
        Err(_) => return Err("release archive cannot be read".to_owned()),
    };
    let mut names = Vec::new();
    let mut binary = None;
    let mut expanded_bytes = 0_u64;

    for entry in entries {
        let mut entry = match entry {
            Ok(entry) => entry,
            Err(_) => return Err("release archive contains an invalid entry".to_owned()),
        };
        let entry_path = match entry.path() {
            Ok(path) => path,
            Err(_) => return Err("release archive contains an invalid path".to_owned()),
        };
        let Some(name) = entry_path.to_str() else {
            return Err("release archive path is not valid UTF-8".to_owned());
        };
        names.push(name.to_owned());
        let size = entry.size();
        expanded_bytes = match expanded_bytes.checked_add(size) {
            Some(total) => total,
            None => return Err("release archive expanded size overflowed".to_owned()),
        };
        if expanded_bytes > MAX_ARCHIVE_BYTES {
            return Err("release archive expands beyond the 256 MiB limit".to_owned());
        }
        let header = entry.header();
        let mode = match header.mode() {
            Ok(mode) => mode,
            Err(_) => return Err("release archive entry mode is invalid".to_owned()),
        };
        let uid = match header.uid() {
            Ok(uid) => uid,
            Err(_) => return Err("release archive entry owner is invalid".to_owned()),
        };
        let gid = match header.gid() {
            Ok(gid) => gid,
            Err(_) => return Err("release archive entry group is invalid".to_owned()),
        };
        let mtime = match header.mtime() {
            Ok(mtime) => mtime,
            Err(_) => return Err("release archive entry timestamp is invalid".to_owned()),
        };
        if uid != 0 || gid != 0 || mtime != source_date_epoch {
            return Err("release archive ownership or timestamp is invalid".to_owned());
        }
        if name == root {
            if !header.entry_type().is_dir() || mode != 0o755 || size != 0 {
                return Err("release archive root metadata is invalid".to_owned());
            }
            continue;
        }
        if !header.entry_type().is_file() {
            return Err("release archive links and special files are forbidden".to_owned());
        }
        let expected_mode = if name == format!("{root}/stack") {
            0o755
        } else {
            0o644
        };
        if mode != expected_mode {
            return Err("release archive entry mode is invalid".to_owned());
        }
        if name == format!("{root}/stack") {
            if size == 0 || size > MAX_BINARY_BYTES {
                return Err("release archive binary size is invalid".to_owned());
            }
            let mut bytes = Vec::with_capacity(size as usize);
            if entry.read_to_end(&mut bytes).is_err() {
                return Err("release archive binary cannot be read".to_owned());
            }
            if bytes.len() as u64 != size {
                return Err("release archive binary is truncated".to_owned());
            }
            binary = Some(bytes);
        }
    }
    if names != expected {
        return Err("release archive entries or bytewise order are invalid".to_owned());
    }
    match binary {
        Some(binary) => Ok(binary),
        None => Err("release archive does not contain the stack binary".to_owned()),
    }
}

pub(super) fn replace_binary_and_receipt(
    runtime: &Runtime,
    candidate_bytes: &[u8],
    new_receipt: &InstallationReceipt,
    binary_verifier: &dyn BinaryVerifier,
) -> Result<Option<String>, String> {
    let current_metadata = match fs::symlink_metadata(&runtime.current_executable) {
        Ok(metadata) => metadata,
        Err(error) => {
            return Err(format!(
                "cannot inspect running executable '{}': {}",
                runtime.current_executable.display(),
                io_error(error)
            ));
        }
    };
    if current_metadata.file_type().is_symlink() || !current_metadata.is_file() {
        return Err("the running executable must be a regular file, not a symlink".to_owned());
    }
    let receipt_metadata = match fs::symlink_metadata(&runtime.receipt_path) {
        Ok(metadata) => metadata,
        Err(error) => {
            return Err(format!(
                "cannot inspect install receipt '{}': {}",
                runtime.receipt_path.display(),
                io_error(error)
            ));
        }
    };
    if receipt_metadata.file_type().is_symlink() || !receipt_metadata.is_file() {
        return Err("the install receipt must remain a regular file during replacement".to_owned());
    }
    let executable_parent = parent_directory(&runtime.current_executable)?;
    let receipt_parent = parent_directory(&runtime.receipt_path)?;
    let candidate = TemporaryFile::write(
        executable_parent,
        "update-binary",
        candidate_bytes,
        Some(current_metadata.permissions()),
    )?;
    binary_verifier.verify(
        candidate.path(),
        &parse_version(OsStr::new(&new_receipt.version))?,
    )?;

    let mut receipt_bytes = match serde_json::to_vec_pretty(new_receipt) {
        Ok(bytes) => bytes,
        Err(_) => return Err("cannot serialize the updated install receipt".to_owned()),
    };
    receipt_bytes.push(b'\n');
    let receipt = TemporaryFile::write(
        receipt_parent,
        "update-receipt",
        &receipt_bytes,
        Some(receipt_metadata.permissions()),
    )?;
    let backup = create_backup_link(executable_parent, &runtime.current_executable)?;

    if let Err(error) = fs::rename(candidate.path(), &runtime.current_executable) {
        let _ = fs::remove_file(&backup);
        return Err(format!(
            "cannot replace the running executable: {}; the existing binary was not changed",
            io_error(error)
        ));
    }
    if let Err(error) = fs::rename(receipt.path(), &runtime.receipt_path) {
        let rollback = fs::rename(&backup, &runtime.current_executable);
        return match rollback {
            Ok(()) => Err(format!(
                "cannot commit the updated install receipt: {}; the original binary was restored",
                io_error(error)
            )),
            Err(rollback_error) => Err(format!(
                "cannot commit the updated install receipt ({}) or restore the original binary ({}); backup remains at '{}'",
                io_error(error),
                io_error(rollback_error),
                backup.display()
            )),
        };
    }

    let warning = match fs::remove_file(&backup) {
        Ok(()) => None,
        Err(error) => Some(format!(
            "the update succeeded but backup '{}' could not be removed: {}",
            backup.display(),
            io_error(error)
        )),
    };
    Ok(warning)
}

pub(super) fn create_backup_link(parent: &Path, executable: &Path) -> Result<PathBuf, String> {
    for attempt in 0..128_u8 {
        let candidate = parent.join(format!(
            ".stack-update-backup-{}-{attempt}",
            std::process::id()
        ));
        match fs::hard_link(executable, &candidate) {
            Ok(()) => return Ok(candidate),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
            Err(error) => {
                return Err(format!(
                    "cannot create an update rollback link: {}; the existing binary was not changed",
                    io_error(error)
                ));
            }
        }
    }
    Err("cannot reserve an update rollback path; the existing binary was not changed".to_owned())
}

pub(super) struct TemporaryFile {
    path: PathBuf,
}

impl TemporaryFile {
    pub(super) fn write(
        parent: &Path,
        label: &str,
        bytes: &[u8],
        permissions: Option<fs::Permissions>,
    ) -> Result<Self, String> {
        for attempt in 0..128_u8 {
            let candidate = parent.join(format!(".stack-{label}-{}-{attempt}", std::process::id()));
            match OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&candidate)
            {
                Ok(mut file) => {
                    let prepared = file
                        .write_all(bytes)
                        .and_then(|()| match permissions {
                            Some(permissions) => file.set_permissions(permissions),
                            None => Ok(()),
                        })
                        .and_then(|()| file.sync_all());
                    drop(file);
                    if let Err(error) = prepared {
                        let _ = fs::remove_file(&candidate);
                        return Err(format!(
                            "cannot prepare verified update material: {}",
                            io_error(error)
                        ));
                    }
                    return Ok(Self { path: candidate });
                }
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
                Err(error) => {
                    return Err(format!(
                        "cannot create update material in '{}': {}",
                        parent.display(),
                        io_error(error)
                    ));
                }
            }
        }
        Err("cannot reserve a temporary update file".to_owned())
    }

    pub(super) fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TemporaryFile {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

pub(super) fn parent_directory(file: &Path) -> Result<&Path, String> {
    match file.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => Ok(parent),
        _ => Err(format!("'{}' has no parent directory", file.display())),
    }
}

pub(super) fn sha256_bytes(bytes: &[u8]) -> String {
    digest_hex(&Sha256::digest(bytes))
}

pub(super) fn sha256_file(file: &Path, limit: u64) -> Result<String, String> {
    let metadata = match fs::symlink_metadata(file) {
        Ok(metadata) => metadata,
        Err(error) => {
            return Err(format!(
                "cannot inspect '{}': {}",
                file.display(),
                io_error(error)
            ));
        }
    };
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(format!("'{}' must be a regular file", file.display()));
    }
    if metadata.len() == 0 || metadata.len() > limit {
        return Err(format!("'{}' has an invalid size", file.display()));
    }
    let mut input = match File::open(file) {
        Ok(input) => input,
        Err(error) => {
            return Err(format!(
                "cannot read '{}': {}",
                file.display(),
                io_error(error)
            ));
        }
    };
    let mut hash = Sha256::new();
    let mut buffer = [0_u8; 1024 * 1024];
    let mut total = 0_u64;
    loop {
        let read = match input.read(&mut buffer) {
            Ok(read) => read,
            Err(error) => {
                return Err(format!(
                    "cannot read '{}': {}",
                    file.display(),
                    io_error(error)
                ));
            }
        };
        if read == 0 {
            break;
        }
        total = match total.checked_add(read as u64) {
            Some(total) if total <= limit => total,
            _ => return Err(format!("'{}' exceeds the size limit", file.display())),
        };
        hash.update(&buffer[..read]);
    }
    Ok(digest_hex(&hash.finalize()))
}

pub(super) fn digest_hex(digest: &[u8]) -> String {
    let mut output = String::with_capacity(digest.len() * 2);
    for byte in digest {
        let _ = std::fmt::Write::write_fmt(&mut output, format_args!("{byte:02x}"));
    }
    output
}

pub(super) fn validate_digest(digest: &str, label: &str) -> Result<(), String> {
    if digest.len() == 64
        && digest
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        Ok(())
    } else {
        Err(format!("{label} SHA-256 digest is invalid"))
    }
}

pub(super) fn validate_commit(commit: &str, label: &str) -> Result<(), String> {
    if commit.len() == 40
        && commit
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        Ok(())
    } else {
        Err(format!("{label} is invalid"))
    }
}

pub(super) fn io_error(error: io::Error) -> &'static str {
    match error.kind() {
        io::ErrorKind::NotFound => "file not found",
        io::ErrorKind::PermissionDenied => "permission denied",
        io::ErrorKind::AlreadyExists => "already exists",
        io::ErrorKind::InvalidInput => "invalid input",
        _ => "I/O error",
    }
}
