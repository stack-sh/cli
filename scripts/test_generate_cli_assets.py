from pathlib import Path
import tempfile
import unittest

from scripts.generate_cli_assets import ASSETS, check_assets, write_assets


class GenerateCliAssetsTest(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix="stack-cli-assets-")
        self.root = Path(self.temporary.name)
        self.binary = self.root / "stack"
        self.binary.write_text(
            "#!/bin/sh\n"
            "if [ \"$1\" = manpage ]; then\n"
            "  printf '.TH STACK 1\\n'\n"
            "else\n"
            "  printf '# %s completion\\n' \"$2\"\n"
            "fi\n",
            encoding="utf-8",
        )
        self.binary.chmod(0o755)
        self.output = self.root / "generated"

    def tearDown(self):
        self.temporary.cleanup()

    def test_write_and_check_exact_inventory(self):
        self.assertEqual(write_assets(self.binary, self.output), 4)
        self.assertEqual(check_assets(self.binary, self.output), 4)
        self.assertEqual(
            {
                path.relative_to(self.output).as_posix()
                for path in self.output.rglob("*")
                if path.is_file()
            },
            set(ASSETS),
        )

    def test_stale_and_unexpected_assets_fail(self):
        write_assets(self.binary, self.output)
        (self.output / "share/man/man1/stack.1").write_text("stale\n", encoding="utf-8")
        with self.assertRaisesRegex(ValueError, "stale"):
            check_assets(self.binary, self.output)

        write_assets(self.binary, self.output)
        unexpected = self.output / "unexpected"
        unexpected.write_text("unexpected\n", encoding="utf-8")
        with self.assertRaisesRegex(ValueError, "inventory"):
            check_assets(self.binary, self.output)


if __name__ == "__main__":
    unittest.main()
