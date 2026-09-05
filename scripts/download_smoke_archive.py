"""Download and verify a published archive before any channel binary is executed."""
import argparse
from pathlib import Path
import subprocess
import sys

from .smoke_installed_cli import verify_checksum


def run(*arguments):
    subprocess.run([str(argument) for argument in arguments], check=True, timeout=180)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ("version", "target", "source-commit"):
        parser.add_argument(f"--{name}", required=True)
    parser.add_argument("--source-root", type=Path, required=True)
    parser.add_argument("--destination", type=Path, required=True)
    arguments = parser.parse_args()
    destination = arguments.destination.resolve()
    destination.mkdir(parents=True, exist_ok=False)
    archive = destination / f"stack-v{arguments.version}-{arguments.target}.tar.gz"
    inventory = destination / f"stack-v{arguments.version}-checksums.txt"
    run("gh", "release", "download", f"v{arguments.version}", "--repo", "stack-sh/cli",
        "--pattern", archive.name, "--pattern", inventory.name, "--dir", destination)
    verify_checksum(archive, inventory)
    run("gh", "attestation", "verify", archive, "--repo", "stack-sh/cli",
        "--signer-workflow", "stack-sh/cli/.github/workflows/release.yaml",
        "--source-ref", f"refs/tags/v{arguments.version}", "--source-digest", arguments.source_commit)
    timestamp = subprocess.check_output(
        ["git", "-C", str(arguments.source_root), "show", "-s", "--format=%ct"], text=True).strip()
    # Use the release's notice and generated assets, not an unreleased working tree.
    run(sys.executable, arguments.source_root / "scripts/package_release.py", "verify",
        "--archive", archive, "--version", arguments.version, "--target", arguments.target,
        "--source-date-epoch", timestamp)
    run("tar", "-xzf", archive, "-C", destination)


if __name__ == "__main__":
    main()
