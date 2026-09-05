#!/usr/bin/env python3
import argparse
import os
from pathlib import Path
import stat
import subprocess
import tempfile


ROOT = Path(__file__).resolve().parent.parent
DEFAULT_OUTPUT = ROOT / "distribution" / "generated"
ASSETS = {
    "share/bash-completion/completions/stack": ("completions", "bash"),
    "share/fish/vendor_completions.d/stack.fish": ("completions", "fish"),
    "share/man/man1/stack.1": ("manpage",),
    "share/zsh/site-functions/_stack": ("completions", "zsh"),
}
MAXIMUM_ASSET_BYTES = 1024 * 1024


def require(condition, message):
    if not condition:
        raise ValueError(message)


def generated_assets(binary):
    binary_path = Path(binary).absolute()
    metadata = binary_path.lstat()
    require(stat.S_ISREG(metadata.st_mode), "CLI binary must be a regular file, not a symlink")
    require(os.access(binary_path, os.X_OK), "CLI binary must be executable")

    generated = {}
    for relative_path, arguments in ASSETS.items():
        completed = subprocess.run(
            [binary_path, *arguments],
            check=False,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
        )
        require(completed.returncode == 0, f"asset generator failed: stack {' '.join(arguments)}")
        require(completed.stderr == b"", f"asset generator wrote diagnostics: stack {' '.join(arguments)}")
        require(
            0 < len(completed.stdout) <= MAXIMUM_ASSET_BYTES,
            f"generated asset size is invalid: {relative_path}",
        )
        require(completed.stdout.endswith(b"\n"), f"generated asset lacks final newline: {relative_path}")
        generated[relative_path] = completed.stdout
    return generated


def validate_inventory(output):
    if not output.exists():
        return
    actual = {
        path.relative_to(output).as_posix()
        for path in output.rglob("*")
        if path.is_file() or path.is_symlink()
    }
    require(actual == set(ASSETS), "generated asset inventory is incomplete or contains unexpected files")


def check_assets(binary, output=DEFAULT_OUTPUT):
    output_path = Path(output).absolute()
    validate_inventory(output_path)
    for relative_path, expected in generated_assets(binary).items():
        destination = output_path / relative_path
        metadata = destination.lstat()
        require(stat.S_ISREG(metadata.st_mode), f"generated asset must be a regular file: {relative_path}")
        require(destination.read_bytes() == expected, f"generated asset is stale: {relative_path}")
    return len(ASSETS)


def write_assets(binary, output=DEFAULT_OUTPUT):
    output_path = Path(output).absolute()
    generated = generated_assets(binary)
    for relative_path, contents in generated.items():
        destination = output_path / relative_path
        destination.parent.mkdir(parents=True, exist_ok=True)
        descriptor, temporary_name = tempfile.mkstemp(
            prefix=f".{destination.name}.", suffix=".tmp", dir=destination.parent
        )
        temporary = Path(temporary_name)
        try:
            with os.fdopen(descriptor, "wb") as stream:
                stream.write(contents)
                stream.flush()
                os.fsync(stream.fileno())
            os.chmod(temporary, 0o644)
            os.replace(temporary, destination)
        finally:
            temporary.unlink(missing_ok=True)
    validate_inventory(output_path)
    return len(generated)


def main():
    parser = argparse.ArgumentParser(description="Generate deterministic Stack CLI shell and manual assets")
    parser.add_argument("--binary", required=True)
    parser.add_argument("--output", default=DEFAULT_OUTPUT)
    parser.add_argument("--check", action="store_true")
    arguments = parser.parse_args()
    try:
        count = (
            check_assets(arguments.binary, arguments.output)
            if arguments.check
            else write_assets(arguments.binary, arguments.output)
        )
        print(f"{'Validated' if arguments.check else 'Generated'} {count} CLI assets.")
    except (OSError, ValueError) as error:
        raise SystemExit(str(error)) from error


if __name__ == "__main__":
    main()
