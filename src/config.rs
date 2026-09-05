//! User-level Stack configuration discovery.

use std::env;
use std::ffi::OsString;
use std::fs::{self, File};
use std::io::Read;
use std::path::{Path, PathBuf};

use serde::Deserialize;

const MAX_CONFIG_BYTES: usize = 64 * 1024;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Environment {
    xdg_config_home: Option<OsString>,
    home: Option<OsString>,
}

impl Environment {
    pub(crate) fn capture() -> Self {
        Self {
            xdg_config_home: env::var_os("XDG_CONFIG_HOME"),
            home: env::var_os("HOME"),
        }
    }

    #[cfg(test)]
    fn new(xdg_config_home: Option<&Path>, home: Option<&Path>) -> Self {
        Self {
            xdg_config_home: xdg_config_home.map(Path::as_os_str).map(OsString::from),
            home: home.map(Path::as_os_str).map(OsString::from),
        }
    }
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct StackConfig {
    default_icons_path: Option<PathBuf>,
}

pub(crate) fn icon_store_root(
    explicit_root: Option<&Path>,
    environment: &Environment,
) -> Result<PathBuf, String> {
    if let Some(root) = explicit_root {
        return Ok(root.to_owned());
    }

    let config_root = config_root(environment)?;
    let stack_root = config_root.join("stack");
    let config_path = stack_root.join("config.yaml");
    let config = read_config(&config_path)?;
    if let Some(default_icons_path) = config.default_icons_path {
        if !default_icons_path.is_absolute() {
            return Err(format!(
                "config '{}' must set 'default_icons_path' to an absolute path",
                config_path.display()
            ));
        }
        return Ok(default_icons_path);
    }
    Ok(stack_root.join("icons"))
}

pub(crate) fn installation_receipt_path(environment: &Environment) -> Result<PathBuf, String> {
    Ok(config_root(environment)?.join("stack/install-receipt.json"))
}

fn config_root(environment: &Environment) -> Result<PathBuf, String> {
    if let Some(value) = &environment.xdg_config_home {
        if !value.is_empty() {
            let path = PathBuf::from(value);
            if path.is_absolute() {
                return Ok(path);
            }
        }
    }

    if let Some(value) = &environment.home {
        if !value.is_empty() {
            let home = PathBuf::from(value);
            if home.is_absolute() {
                return Ok(home.join(".config"));
            }
        }
    }
    Err(
        "cannot determine the Stack config directory; set XDG_CONFIG_HOME or HOME to an absolute path"
            .to_owned(),
    )
}

fn read_config(path: &Path) -> Result<StackConfig, String> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(StackConfig::default());
        }
        Err(error) => {
            return Err(format!(
                "cannot read config '{}': {}",
                path.display(),
                stable_io_error(error.kind())
            ));
        }
    };
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(format!(
            "config '{}' must be a regular file, not a symlink",
            path.display()
        ));
    }
    if metadata.len() > MAX_CONFIG_BYTES as u64 {
        return Err(format!(
            "config '{}' exceeds the 64 KiB limit",
            path.display()
        ));
    }

    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    let read_result = File::open(path).and_then(|mut file| file.read_to_end(&mut bytes));
    if let Err(error) = read_result {
        return Err(format!(
            "cannot read config '{}': {}",
            path.display(),
            stable_io_error(error.kind())
        ));
    }
    if bytes.len() > MAX_CONFIG_BYTES {
        return Err(format!(
            "config '{}' exceeds the 64 KiB limit",
            path.display()
        ));
    }
    if bytes.iter().all(u8::is_ascii_whitespace) {
        return Ok(StackConfig::default());
    }
    match serde_yaml_ng::from_slice(&bytes) {
        Ok(config) => Ok(config),
        Err(_) => Err(format!("config '{}' is invalid YAML", path.display())),
    }
}

fn stable_io_error(kind: std::io::ErrorKind) -> &'static str {
    match kind {
        std::io::ErrorKind::NotFound => "file not found",
        std::io::ErrorKind::PermissionDenied => "permission denied",
        std::io::ErrorKind::AlreadyExists => "already exists",
        std::io::ErrorKind::InvalidInput => "invalid input",
        _ => "I/O error",
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicU64, Ordering};

    use super::*;

    static CASE_ID: AtomicU64 = AtomicU64::new(0);

    struct TestDirectory {
        path: PathBuf,
    }

    impl TestDirectory {
        fn new(label: &str) -> Self {
            let case_id = CASE_ID.fetch_add(1, Ordering::Relaxed);
            let path = env::temp_dir().join(format!(
                "stack-cli-config-{}-{label}-{case_id}",
                std::process::id()
            ));
            assert!(fs::create_dir(&path).is_ok());
            Self { path }
        }
    }

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }

    #[test]
    fn xdg_and_home_defaults_are_deterministic() {
        let directory = TestDirectory::new("defaults");
        let xdg = directory.path.join("xdg");
        let home = directory.path.join("home");
        assert!(matches!(
            icon_store_root(None, &Environment::new(Some(&xdg), Some(&home))),
            Ok(path) if path == xdg.join("stack/icons")
        ));
        assert!(matches!(
            icon_store_root(
                None,
                &Environment::new(Some(Path::new("relative")), Some(&home))
            ),
            Ok(path) if path == home.join(".config/stack/icons")
        ));
        assert!(matches!(
            icon_store_root(None, &Environment::new(None, Some(&home))),
            Ok(path) if path == home.join(".config/stack/icons")
        ));
        assert!(icon_store_root(None, &Environment::new(None, None)).is_err());
        assert!(matches!(
            installation_receipt_path(&Environment::new(Some(&xdg), Some(&home))),
            Ok(path) if path == xdg.join("stack/install-receipt.json")
        ));
        assert!(matches!(
            installation_receipt_path(&Environment::new(None, Some(&home))),
            Ok(path) if path == home.join(".config/stack/install-receipt.json")
        ));
    }

    #[test]
    fn config_override_is_absolute_and_strict() {
        let directory = TestDirectory::new("override");
        let xdg = directory.path.join("xdg");
        let stack_root = xdg.join("stack");
        assert!(fs::create_dir_all(&stack_root).is_ok());
        let custom = directory.path.join("shared-icons");
        assert!(
            fs::write(
                stack_root.join("config.yaml"),
                format!("default_icons_path: {}\n", custom.display()),
            )
            .is_ok()
        );
        assert!(matches!(
            icon_store_root(None, &Environment::new(Some(&xdg), None)),
            Ok(path) if path == custom
        ));

        assert!(
            fs::write(
                stack_root.join("config.yaml"),
                "default_icons_path: ./relative\n",
            )
            .is_ok()
        );
        assert!(matches!(
            icon_store_root(None, &Environment::new(Some(&xdg), None)),
            Err(error) if error.contains("absolute path")
        ));
        assert!(fs::write(stack_root.join("config.yaml"), "unknown: true\n").is_ok());
        assert!(matches!(
            icon_store_root(None, &Environment::new(Some(&xdg), None)),
            Err(error) if error.contains("invalid YAML")
        ));
    }

    #[test]
    fn explicit_root_has_highest_precedence() {
        let explicit = Path::new(".stack-icons");
        assert!(matches!(
            icon_store_root(Some(explicit), &Environment::new(None, None)),
            Ok(path) if path == explicit
        ));
    }

    #[test]
    fn config_files_are_bounded_regular_files() {
        let directory = TestDirectory::new("file-boundary");
        let xdg = directory.path.join("xdg");
        let stack_root = xdg.join("stack");
        let config_path = stack_root.join("config.yaml");
        assert!(fs::create_dir_all(&stack_root).is_ok());

        assert!(fs::write(&config_path, b" \n\t").is_ok());
        assert!(matches!(
            icon_store_root(None, &Environment::new(Some(&xdg), None)),
            Ok(path) if path == stack_root.join("icons")
        ));

        assert!(fs::remove_file(&config_path).is_ok());
        assert!(fs::create_dir(&config_path).is_ok());
        assert!(matches!(
            icon_store_root(None, &Environment::new(Some(&xdg), None)),
            Err(error) if error.contains("regular file")
        ));
        assert!(fs::remove_dir(&config_path).is_ok());

        let oversized = File::create(&config_path);
        assert!(oversized.is_ok());
        if let Ok(oversized) = oversized {
            assert!(oversized.set_len(MAX_CONFIG_BYTES as u64 + 1).is_ok());
        }
        assert!(matches!(
            icon_store_root(None, &Environment::new(Some(&xdg), None)),
            Err(error) if error.contains("64 KiB")
        ));

        #[cfg(unix)]
        {
            use std::os::unix::fs::symlink;

            assert!(fs::remove_file(&config_path).is_ok());
            let target = stack_root.join("target.yaml");
            assert!(fs::write(&target, b"default_icons_path: /tmp/icons\n").is_ok());
            assert!(symlink(&target, &config_path).is_ok());
            assert!(matches!(
                icon_store_root(None, &Environment::new(Some(&xdg), None)),
                Err(error) if error.contains("symlink")
            ));
        }

        assert_eq!(
            stable_io_error(std::io::ErrorKind::NotFound),
            "file not found"
        );
        assert_eq!(
            stable_io_error(std::io::ErrorKind::PermissionDenied),
            "permission denied"
        );
        assert_eq!(
            stable_io_error(std::io::ErrorKind::AlreadyExists),
            "already exists"
        );
        assert_eq!(
            stable_io_error(std::io::ErrorKind::InvalidInput),
            "invalid input"
        );
        assert_eq!(stable_io_error(std::io::ErrorKind::Other), "I/O error");
        let _ = Environment::capture();
    }
}
