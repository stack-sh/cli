#!/usr/bin/env python3
"""Exercise the installed binary without reusing an existing config or icon store."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import tempfile

from .verify_release_binary import command, require, verify_architecture, verify_commands


def verify_checksum(archive, inventory):
    matches = []
    for line in inventory.read_text().splitlines():
        match = re.fullmatch(r"([0-9a-fA-F]{64}) [ *](.+)", line)
        require(match is not None, "Malformed checksum inventory")
        if match[2] == archive.name:
            matches.append(match[1].lower())
    require(len(matches) == 1, "Missing or duplicate archive checksum")
    require(hashlib.sha256(archive.read_bytes()).hexdigest() == matches[0], "Archive checksum mismatch")


def verify_import(binary, reference_root):
    with tempfile.TemporaryDirectory(prefix="stack-import-smoke-") as temporary:
        root = Path(temporary)
        environment = os.environ.copy()
        for name in ("XDG_CONFIG_HOME", "XDG_DATA_HOME", "XDG_CACHE_HOME"):
            environment[name] = str(root / name.lower())
        store = root / "icons"
        # Only the already-audited catalog is imported; no artwork is uploaded as CI evidence.
        command([binary, "icons", "import", "simple-icons", "--accept-terms", "-o", store], root, environment)
        pack = store / "simple-icons"
        manifest = json.loads((pack / "manifest.json").read_text())
        catalog = json.loads((reference_root / "catalogs/simple-icons.json").read_text())
        require(manifest["provider"]["id"] == "simple-icons", "Imported provider identity mismatch")
        require(len(manifest["icons"]) == len(catalog["icons"]), "Imported icon inventory mismatch")
        require((pack / "NOTICE.md").stat().st_size > 0, "Import omitted attribution")
        source = root / "provider.stack"
        source.write_text('stack 1.0\ndiagram "Import smoke" {\n  node rust "Rust" { kind service icon "simple-icons:rust" }\n}\n')
        output = root / "provider.svg"
        notice = root / "provider.NOTICE.md"
        command([binary, "render", source, "--provider-pack", store, "--notice", notice, "-o", output], root, environment)
        require(output.stat().st_size > 0, "Imported icon did not render")
        require('data-icon-id="simple-icons:rust"' in output.read_text(), "Imported icon fell back instead of resolving")
        require("simple-icons:rust" in notice.read_text(), "Rendered icon attribution is missing")
        return len(manifest["icons"])


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", required=True)
    parser.add_argument("--target", required=True)
    parser.add_argument("--version", required=True)
    parser.add_argument("--source-root", required=True, type=Path)
    parser.add_argument("--canonical-binary", type=Path)
    arguments = parser.parse_args()
    binary = Path(arguments.binary).resolve(strict=True)
    verify_architecture(binary, arguments.target)
    if arguments.canonical_binary:
        require(binary.read_bytes() == arguments.canonical_binary.read_bytes(), "Installed binary differs from the verified release archive")
    verify_commands(binary, arguments.version, arguments.source_root / "distribution/generated")
    count = verify_import(binary, arguments.source_root)
    print(json.dumps({"version": arguments.version, "target": arguments.target, "commands": "passed", "importedIcons": count, "render": "passed"}))


if __name__ == "__main__":
    main()
