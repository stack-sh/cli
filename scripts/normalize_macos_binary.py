#!/usr/bin/env python3
import argparse
import hashlib
import os
from pathlib import Path
import stat
import struct
import subprocess
import uuid


MACH_HEADER_64 = struct.Struct("<IIIIIIII")
LOAD_COMMAND = struct.Struct("<II")
LC_UUID = 0x1B
LC_CODE_SIGNATURE = 0x1D
MH_MAGIC_64 = 0xFEEDFACF
MH_EXECUTE = 0x2
MAXIMUM_BINARY_BYTES = 256 * 1024 * 1024
TARGET_CPU_TYPES = {
    "aarch64-apple-darwin": 0x0100000C,
    "x86_64-apple-darwin": 0x01000007,
}


def require(condition, message):
    if not condition:
        raise ValueError(message)


def parse_macho(data, target):
    require(target in TARGET_CPU_TYPES, f"unsupported macOS target: {target}")
    require(len(data) >= MACH_HEADER_64.size, "Mach-O header is truncated")
    magic, cpu_type, _, file_type, command_count, command_bytes, _, _ = MACH_HEADER_64.unpack_from(data)
    require(magic == MH_MAGIC_64, "release binary must be a little-endian 64-bit Mach-O")
    require(cpu_type == TARGET_CPU_TYPES[target], f"Mach-O CPU type does not match {target}")
    require(file_type == MH_EXECUTE, "Mach-O release binary must be executable")
    require(0 < command_count <= 4096, "Mach-O load command count is invalid")
    require(command_bytes <= len(data) - MACH_HEADER_64.size, "Mach-O load commands are truncated")

    offset = MACH_HEADER_64.size
    command_end = offset + command_bytes
    uuid_offsets = []
    signature_commands = 0
    for _ in range(command_count):
        require(offset + LOAD_COMMAND.size <= command_end, "Mach-O load command header is truncated")
        command, command_size = LOAD_COMMAND.unpack_from(data, offset)
        require(command_size >= LOAD_COMMAND.size and command_size % 4 == 0, "Mach-O load command size is invalid")
        require(offset + command_size <= command_end, "Mach-O load command is truncated")
        if command == LC_UUID:
            require(command_size == 24, "Mach-O UUID load command size is invalid")
            uuid_offsets.append(offset + LOAD_COMMAND.size)
        elif command == LC_CODE_SIGNATURE:
            signature_commands += 1
        offset += command_size
    require(offset == command_end, "Mach-O load command size does not match its header")
    require(len(uuid_offsets) == 1, "Mach-O release binary must contain exactly one UUID")
    require(signature_commands <= 1, "Mach-O release binary has multiple code signatures")
    return {"uuid_offset": uuid_offsets[0], "has_signature": signature_commands == 1}


def content_uuid(data, uuid_offset):
    normalized = bytearray(data)
    normalized[uuid_offset : uuid_offset + 16] = bytes(16)
    digest = bytearray(hashlib.sha256(normalized).digest()[:16])
    digest[6] = (digest[6] & 0x0F) | 0x80
    digest[8] = (digest[8] & 0x3F) | 0x80
    return bytes(digest)


def run(arguments):
    completed = subprocess.run(
        [str(argument) for argument in arguments],
        check=False,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
    )
    if completed.returncode != 0:
        details = completed.stderr.decode("utf-8", errors="replace").strip()
        raise ValueError(f"command failed ({completed.returncode}): {' '.join(map(str, arguments))}: {details}")
    return completed


def normalize_binary(binary, target):
    binary_path = Path(binary).absolute()
    binary_stat = binary_path.lstat()
    require(stat.S_ISREG(binary_stat.st_mode), "macOS release binary must be a regular file, not a symlink")
    require(0 < binary_stat.st_size <= MAXIMUM_BINARY_BYTES, "macOS release binary size is invalid")
    require(os.access(binary_path, os.X_OK), "macOS release binary must be executable")

    data = binary_path.read_bytes()
    metadata = parse_macho(data, target)
    if metadata["has_signature"]:
        run(["codesign", "--remove-signature", binary_path])
        data = binary_path.read_bytes()
        metadata = parse_macho(data, target)
        require(not metadata["has_signature"], "existing Mach-O signature was not removed")

    normalized_uuid = content_uuid(data, metadata["uuid_offset"])
    with binary_path.open("r+b") as stream:
        stream.seek(metadata["uuid_offset"])
        stream.write(normalized_uuid)
        stream.flush()
        os.fsync(stream.fileno())

    run([
        "codesign",
        "--force",
        "--sign",
        "-",
        "--timestamp=none",
        "--identifier",
        "sh.stack.cli",
        binary_path,
    ])
    run(["codesign", "--verify", "--strict", binary_path])
    signature = run(["codesign", "--display", "--verbose=4", binary_path]).stderr.decode(
        "utf-8", errors="replace"
    )
    require("Identifier=sh.stack.cli\n" in signature, "normalized Mach-O signature has the wrong identifier")
    require("Signature=adhoc\n" in signature, "normalized Mach-O signature must be ad-hoc")

    final_data = binary_path.read_bytes()
    final_metadata = parse_macho(final_data, target)
    require(final_metadata["has_signature"], "normalized Mach-O binary must have an ad-hoc signature")
    require(
        final_data[final_metadata["uuid_offset"] : final_metadata["uuid_offset"] + 16] == normalized_uuid,
        "codesign changed the normalized Mach-O UUID",
    )
    return {"uuid": str(uuid.UUID(bytes=normalized_uuid)).upper(), "sha256": hashlib.sha256(final_data).hexdigest()}


def main():
    parser = argparse.ArgumentParser(description="Normalize and ad-hoc sign a reproducible Stack CLI Mach-O binary")
    parser.add_argument("--binary", required=True)
    parser.add_argument("--target", required=True)
    arguments = parser.parse_args()
    try:
        result = normalize_binary(arguments.binary, arguments.target)
        print(f"normalized Mach-O UUID {result['uuid']} (sha256:{result['sha256']})")
    except (OSError, ValueError) as error:
        raise SystemExit(str(error)) from error


if __name__ == "__main__":
    main()
