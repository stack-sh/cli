import hashlib
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

from scripts.smoke_installed_cli import verify_checksum
from scripts.verify_release_binary import verify_commands
from scripts.verify_smoke_cargo_source import verify_source


class ChecksumTests(unittest.TestCase):
    def test_registry_source_must_match_one_clean_published_commit(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            source = "a" * 40
            with self.assertRaisesRegex(ValueError, "exactly one"):
                verify_source(root, "0.5.1", source)
            vcs = root / "registry/src/crates-io/stack-diagram-cli-0.5.1/.cargo_vcs_info.json"
            vcs.parent.mkdir(parents=True)
            vcs.write_text(json.dumps({"git": {"sha1": source}}))
            verify_source(root, "0.5.1", source)
            with self.assertRaisesRegex(ValueError, "differs"):
                verify_source(root, "0.5.1", "b" * 40)
            vcs.write_text(json.dumps({"git": {"sha1": source, "dirty": True}}))
            with self.assertRaisesRegex(ValueError, "dirty"):
                verify_source(root, "0.5.1", source)
            duplicate = root / "registry/src/other/stack-diagram-cli-0.5.1/.cargo_vcs_info.json"
            duplicate.parent.mkdir(parents=True)
            duplicate.write_text(vcs.read_text())
            with self.assertRaisesRegex(ValueError, "exactly one"):
                verify_source(root, "0.5.1", source)

    def test_wrong_installed_version_is_rejected_before_running_other_commands(self):
        with patch("scripts.verify_release_binary.command", return_value=b"stack 0.0.0\n") as run:
            with self.assertRaisesRegex(ValueError, "version output"):
                verify_commands(Path("unused"), "0.5.1")
            self.assertEqual(run.call_count, 1)

    def test_archive_must_exist_and_match_one_exact_checksum(self):
        with tempfile.TemporaryDirectory() as directory:
            archive = Path(directory) / "stack.tar.gz"
            inventory = Path(directory) / "checksums.txt"
            archive.write_bytes(b"immutable release bytes")
            digest = hashlib.sha256(archive.read_bytes()).hexdigest()
            inventory.write_text(f"{digest}  {archive.name}\n")
            verify_checksum(archive, inventory)
            archive.write_bytes(b"tampered")
            with self.assertRaisesRegex(ValueError, "mismatch"):
                verify_checksum(archive, inventory)
            inventory.write_text(f"{digest}  missing.tar.gz\n")
            with self.assertRaisesRegex(ValueError, "Missing"):
                verify_checksum(archive, inventory)
            inventory.write_text(f"{digest}  {archive.name}\n" * 2)
            with self.assertRaisesRegex(ValueError, "duplicate"):
                verify_checksum(archive, inventory)
            inventory.write_text("bad checksum\n")
            with self.assertRaisesRegex(ValueError, "Malformed"):
                verify_checksum(archive, inventory)


if __name__ == "__main__":
    unittest.main()
