import struct
import unittest

from scripts.normalize_macos_binary import LC_UUID, MACH_HEADER_64, MH_EXECUTE, MH_MAGIC_64, content_uuid, parse_macho


def macho_fixture(cpu_type=0x0100000C, uuid_bytes=bytes.fromhex("00112233445566778899aabbccddeeff"), duplicate=False):
    uuid_command = struct.pack("<II16s", LC_UUID, 24, uuid_bytes)
    commands = uuid_command + (uuid_command if duplicate else b"")
    header = MACH_HEADER_64.pack(
        MH_MAGIC_64,
        cpu_type,
        0,
        MH_EXECUTE,
        2 if duplicate else 1,
        len(commands),
        0,
        0,
    )
    return header + commands + b"deterministic executable content"


class NormalizeMacosBinaryTests(unittest.TestCase):
    def test_content_uuid_ignores_the_linker_uuid(self):
        first = macho_fixture(uuid_bytes=bytes(16))
        second = macho_fixture(uuid_bytes=bytes([0xFF]) * 16)
        first_metadata = parse_macho(first, "aarch64-apple-darwin")
        second_metadata = parse_macho(second, "aarch64-apple-darwin")
        first_uuid = content_uuid(first, first_metadata["uuid_offset"])
        second_uuid = content_uuid(second, second_metadata["uuid_offset"])
        self.assertEqual(first_uuid, second_uuid)
        self.assertEqual(first_uuid[6] >> 4, 8)
        self.assertEqual(first_uuid[8] >> 6, 2)

    def test_x86_64_cpu_type_is_accepted_for_its_target(self):
        metadata = parse_macho(macho_fixture(cpu_type=0x01000007), "x86_64-apple-darwin")
        self.assertEqual(metadata["uuid_offset"], MACH_HEADER_64.size + 8)
        self.assertFalse(metadata["has_signature"])

    def test_target_architecture_drift_is_rejected(self):
        with self.assertRaisesRegex(ValueError, "CPU type does not match"):
            parse_macho(macho_fixture(), "x86_64-apple-darwin")

    def test_duplicate_uuid_commands_are_rejected(self):
        with self.assertRaisesRegex(ValueError, "exactly one UUID"):
            parse_macho(macho_fixture(duplicate=True), "aarch64-apple-darwin")

    def test_truncated_load_commands_are_rejected(self):
        with self.assertRaisesRegex(ValueError, "truncated"):
            parse_macho(macho_fixture()[:40], "aarch64-apple-darwin")


if __name__ == "__main__":
    unittest.main()
