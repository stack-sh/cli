"""Reject a registry install that does not come from the expected release source."""
import argparse
import json
from pathlib import Path

from .verify_release_binary import require


def verify_source(cargo_home, version, source_commit):
    candidates = list(cargo_home.glob(f"registry/src/*/stack-diagram-cli-{version}/.cargo_vcs_info.json"))
    require(len(candidates) == 1, "Expected exactly one fresh registry source package")
    vcs = json.loads(candidates[0].read_text())
    require(vcs["git"]["sha1"] == source_commit, "Registry source commit differs from the expected release")
    require(vcs["git"].get("dirty", False) is False, "Registry source was packaged from a dirty tree")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--cargo-home", type=Path, required=True)
    parser.add_argument("--version", required=True)
    parser.add_argument("--source-commit", required=True)
    arguments = parser.parse_args()
    verify_source(arguments.cargo_home, arguments.version, arguments.source_commit)
