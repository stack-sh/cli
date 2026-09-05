import gzip
from pathlib import Path
import tarfile
import tempfile
import unittest

from scripts.package_release import create_archive, verify_archive


class PackageReleaseTest(unittest.TestCase):
    version = "0.3.0"
    target = "aarch64-apple-darwin"
    source_date_epoch = 1_788_566_400

    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix="stack-package-release-")
        self.root = Path(self.temporary.name)
        self.binary = self.root / "stack"
        self.binary.write_bytes(b"deterministic Stack executable fixture\n")
        self.binary.chmod(0o755)

    def tearDown(self):
        self.temporary.cleanup()

    def test_archive_is_reproducible_and_exact(self):
        first = create_archive(
            self.binary,
            self.target,
            self.version,
            self.source_date_epoch,
            self.root / "first",
        )
        second = create_archive(
            self.binary,
            self.target,
            self.version,
            self.source_date_epoch,
            self.root / "second",
        )

        self.assertEqual(first.read_bytes(), second.read_bytes())
        self.assertEqual(
            verify_archive(first, self.target, self.version, self.source_date_epoch, self.binary)["entries"],
            5,
        )
        with gzip.open(first, "rb") as stream:
            self.assertIn(b"deterministic Stack executable fixture", stream.read())

    def test_existing_archive_is_never_replaced(self):
        create_archive(
            self.binary,
            self.target,
            self.version,
            self.source_date_epoch,
            self.root,
        )
        with self.assertRaisesRegex(ValueError, "refusing to replace"):
            create_archive(
                self.binary,
                self.target,
                self.version,
                self.source_date_epoch,
                self.root,
            )

    def test_version_and_target_drift_are_rejected(self):
        with self.assertRaisesRegex(ValueError, "match Cargo.toml"):
            create_archive(self.binary, self.target, "0.4.0", self.source_date_epoch, self.root)
        with self.assertRaisesRegex(ValueError, "unsupported release target"):
            create_archive(self.binary, "x86_64-pc-windows-msvc", self.version, self.source_date_epoch, self.root)

    def test_symlink_binary_is_rejected(self):
        linked = self.root / "linked-stack"
        linked.symlink_to(self.binary)
        with self.assertRaisesRegex(ValueError, "not a symlink"):
            create_archive(linked, self.target, self.version, self.source_date_epoch, self.root)

    def test_modified_archive_is_rejected(self):
        archive = create_archive(
            self.binary,
            self.target,
            self.version,
            self.source_date_epoch,
            self.root,
        )
        contents = bytearray(archive.read_bytes())
        contents[len(contents) // 2] ^= 0xFF
        archive.write_bytes(contents)
        with self.assertRaises((gzip.BadGzipFile, tarfile.TarError, EOFError, OSError, ValueError)):
            verify_archive(archive, self.target, self.version, self.source_date_epoch, self.binary)


if __name__ == "__main__":
    unittest.main()
