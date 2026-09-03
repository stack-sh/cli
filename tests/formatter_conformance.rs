use std::env;
use std::error::Error;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

#[test]
fn canonical_formatter_fixtures_match_cli_output() -> Result<(), Box<dyn Error>> {
    let root = specification_root()?.join("conformance/formatter");
    let mut cases = fs::read_dir(&root)?.collect::<Result<Vec<_>, _>>()?;
    cases.sort_by_key(|entry| entry.file_name());
    if cases.is_empty() {
        return Err(format!("no formatter cases found in {}", root.display()).into());
    }

    for entry in cases {
        let case = entry.path();
        if !case.is_dir() {
            continue;
        }
        let input = fs::read(case.join("input.stack"))?;
        let expected = fs::read(case.join("expected.stack"))?;

        let formatted = stack_with_input(["fmt", "-"], &input)?;
        if formatted.status.code() != Some(0)
            || formatted.stdout != expected
            || !formatted.stderr.is_empty()
        {
            return Err(format!("{} did not match canonical output", case.display()).into());
        }

        let clean = stack_with_input(["fmt", "--check", "-"], &expected)?;
        if clean.status.code() != Some(0) || !clean.stdout.is_empty() || !clean.stderr.is_empty() {
            return Err(format!("{} expected output was not clean", case.display()).into());
        }

        let input_check = stack_with_input(["fmt", "--check", "-"], &input)?;
        let expected_status = if input == expected { Some(0) } else { Some(1) };
        if input_check.status.code() != expected_status
            || !input_check.stdout.is_empty()
            || !input_check.stderr.is_empty()
        {
            return Err(format!("{} input check result was incorrect", case.display()).into());
        }
    }
    Ok(())
}

fn stack_with_input(
    arguments: impl IntoIterator<Item = impl AsRef<std::ffi::OsStr>>,
    input: &[u8],
) -> Result<Output, Box<dyn Error>> {
    let mut child = Command::new(env!("CARGO_BIN_EXE_stack"))
        .args(arguments)
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

fn specification_root() -> Result<PathBuf, Box<dyn Error>> {
    let configured = env::var_os("STACK_SPECIFICATION_DIR")
        .ok_or("STACK_SPECIFICATION_DIR must point to stack-sh/specification")?;
    let root = Path::new(&configured);
    if !root.is_dir() {
        return Err(format!("{} is not a directory", root.display()).into());
    }
    Ok(root.to_path_buf())
}
