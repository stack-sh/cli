use std::env;
use std::error::Error;
use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};

use serde::Deserialize;

static CASE_ID: AtomicU64 = AtomicU64::new(0);

#[derive(Deserialize)]
struct Catalog {
    examples: Vec<Example>,
}

#[derive(Deserialize)]
struct Example {
    id: String,
    providers: Vec<String>,
    source: String,
}

struct TestDirectory {
    path: PathBuf,
}

impl TestDirectory {
    fn new() -> Result<Self, Box<dyn Error>> {
        let case_id = CASE_ID.fetch_add(1, Ordering::Relaxed);
        let path = env::temp_dir().join(format!(
            "stack-cli-template-conformance-{}-{case_id}",
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

#[test]
fn curated_templates_match_the_pin_and_render() -> Result<(), Box<dyn Error>> {
    let repository_root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let catalog: Catalog =
        serde_json::from_slice(&fs::read(repository_root.join("templates/catalog.json"))?)?;
    let specification_root = specification_root()?;
    let directory = TestDirectory::new()?;
    assert_eq!(catalog.examples.len(), 9);

    for example in catalog.examples {
        let snapshot = repository_root
            .join("templates/sources")
            .join(&example.source);
        let canonical = specification_root.join("examples").join(&example.source);
        let expected = fs::read(&canonical)?;
        assert_eq!(
            fs::read(&snapshot)?,
            expected,
            "{} differs from {}",
            snapshot.display(),
            canonical.display()
        );

        let output_name = format!("{}.stack", example.id);
        let initialized = stack_in(
            &directory.path,
            [
                OsStr::new("init"),
                OsStr::new("--template"),
                OsStr::new(&example.id),
                OsStr::new("-o"),
                OsStr::new(&output_name),
            ],
        )?;
        assert_eq!(
            initialized.status.code(),
            Some(0),
            "template {}",
            example.id
        );
        assert!(initialized.stderr.is_empty(), "template {}", example.id);
        assert_eq!(fs::read(directory.path.join(&output_name))?, expected);
        let stdout = String::from_utf8(initialized.stdout)?;
        if example.providers.is_empty() {
            assert!(
                !stdout.contains("Provider icons:"),
                "template {}",
                example.id
            );
        } else {
            assert!(
                stdout.contains(&format!("Provider icons: {}", example.providers.join(", "))),
                "template {}",
                example.id
            );
        }
        for provider in &example.providers {
            assert!(
                stdout.contains(&format!("stack icons import {provider} --accept-terms")),
                "template {}",
                example.id
            );
        }

        let checked = stack_in(
            &directory.path,
            [OsStr::new("check"), OsStr::new(&output_name)],
        )?;
        assert_eq!(checked.status.code(), Some(0), "template {}", example.id);
        assert!(checked.stdout.is_empty(), "template {}", example.id);
        assert!(
            !String::from_utf8(checked.stderr)?.contains(" error["),
            "template {}",
            example.id
        );

        let svg_name = format!("{}.svg", example.id);
        let rendered = stack_in(
            &directory.path,
            [
                OsStr::new("render"),
                OsStr::new(&output_name),
                OsStr::new("-o"),
                OsStr::new(&svg_name),
            ],
        )?;
        assert_eq!(rendered.status.code(), Some(0), "template {}", example.id);
        assert!(rendered.stdout.is_empty(), "template {}", example.id);
        assert!(
            !String::from_utf8(rendered.stderr)?.contains(" error["),
            "template {}",
            example.id
        );
        assert!(fs::read_to_string(directory.path.join(svg_name))?.contains("<svg"));
    }
    Ok(())
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

fn specification_root() -> Result<PathBuf, Box<dyn Error>> {
    let configured = env::var_os("STACK_SPECIFICATION_DIR")
        .ok_or("STACK_SPECIFICATION_DIR must point to stack-sh/specification")?;
    let root = Path::new(&configured);
    if !root.is_dir() {
        return Err(format!("{} is not a directory", root.display()).into());
    }
    Ok(root.to_path_buf())
}
