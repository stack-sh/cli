#!/usr/bin/env python3
import argparse
import gzip
import hashlib
import os
from pathlib import Path
import re
import stat
import tarfile
import tempfile
import zlib


ROOT = Path(__file__).resolve().parent.parent
TARGETS = {
    "aarch64-apple-darwin",
    "x86_64-apple-darwin",
    "aarch64-unknown-linux-gnu",
    "x86_64-unknown-linux-gnu",
}
GENERATED_FILES = {
    "share/bash-completion/completions/stack": ROOT / "distribution/generated/share/bash-completion/completions/stack",
    "share/fish/vendor_completions.d/stack.fish": ROOT / "distribution/generated/share/fish/vendor_completions.d/stack.fish",
    "share/man/man1/stack.1": ROOT / "distribution/generated/share/man/man1/stack.1",
    "share/zsh/site-functions/_stack": ROOT / "distribution/generated/share/zsh/site-functions/_stack",
}
REQUIRED_FILES = (
    "LICENSE",
    "NOTICE",
    "THIRD_PARTY_LICENSES.md",
    *GENERATED_FILES,
    "stack",
)
VERSION_PATTERN = re.compile(r"^[0-9]+\.[0-9]+\.[0-9]+(?:-rc\.[1-9][0-9]*)?$")
MAXIMUM_FILE_BYTES = 256 * 1024 * 1024


def require(condition, message):
    if not condition:
        raise ValueError(message)


def cargo_version():
    cargo_toml = (ROOT / "Cargo.toml").read_text(encoding="utf-8")
    match = re.search(r'^version\s*=\s*"([^"]+)"', cargo_toml, re.MULTILINE)
    require(match is not None, "Cargo.toml package version is missing")
    return match.group(1)


def validate_inputs(binary, target, version, source_date_epoch):
    require(target in TARGETS, f"unsupported release target: {target}")
    require(VERSION_PATTERN.fullmatch(version) is not None, "invalid release version")
    require(version == cargo_version(), "release version must match Cargo.toml")
    require(0 <= source_date_epoch <= 0xFFFFFFFF, "SOURCE_DATE_EPOCH is outside the gzip timestamp range")

    binary_path = Path(binary).absolute()
    binary_stat = binary_path.lstat()
    require(stat.S_ISREG(binary_stat.st_mode), "release binary must be a regular file, not a symlink")
    require(0 < binary_stat.st_size <= MAXIMUM_FILE_BYTES, "release binary size is invalid")
    return binary_path


def archive_name(target, version):
    return f"stack-v{version}-{target}.tar.gz"


def archive_root(target, version):
    return f"stack-v{version}-{target}"


def sha256_stream(stream):
    digest = hashlib.sha256()
    while True:
        block = stream.read(1024 * 1024)
        if not block:
            return digest.hexdigest()
        digest.update(block)


def sha256_file(path):
    with Path(path).open("rb") as stream:
        return sha256_stream(stream)


def tar_info(name, mode, size, source_date_epoch, entry_type=tarfile.REGTYPE):
    info = tarfile.TarInfo(name)
    info.type = entry_type
    info.mode = mode
    info.uid = 0
    info.gid = 0
    info.uname = ""
    info.gname = ""
    info.mtime = source_date_epoch
    info.size = size
    return info


def archive_file_sources(binary_path):
    return {
        "LICENSE": ROOT / "LICENSE",
        "NOTICE": ROOT / "NOTICE",
        "THIRD_PARTY_LICENSES.md": ROOT / "THIRD_PARTY_LICENSES.md",
        **GENERATED_FILES,
        "stack": binary_path,
    }


def create_archive(binary, target, version, source_date_epoch, output_directory):
    binary_path = validate_inputs(binary, target, version, source_date_epoch)
    output = Path(output_directory).absolute()
    output.mkdir(parents=True, exist_ok=True)
    output_stat = output.lstat()
    require(stat.S_ISDIR(output_stat.st_mode), "output path must be a directory, not a symlink")

    destination = output / archive_name(target, version)
    require(not os.path.lexists(destination), f"refusing to replace release archive: {destination.name}")
    root_name = archive_root(target, version)

    file_sources = archive_file_sources(binary_path)
    for name, source in file_sources.items():
        source_stat = source.lstat()
        require(stat.S_ISREG(source_stat.st_mode), f"archive input must be a regular file: {name}")
        require(0 < source_stat.st_size <= MAXIMUM_FILE_BYTES, f"archive input size is invalid: {name}")

    descriptor, temporary_name = tempfile.mkstemp(prefix=f".{destination.name}.", suffix=".tmp", dir=output)
    temporary = Path(temporary_name)
    try:
        with os.fdopen(descriptor, "wb") as raw_stream:
            with gzip.GzipFile(
                filename="",
                mode="wb",
                compresslevel=9,
                fileobj=raw_stream,
                mtime=source_date_epoch,
            ) as gzip_stream:
                with tarfile.open(fileobj=gzip_stream, mode="w", format=tarfile.USTAR_FORMAT) as archive:
                    archive.addfile(tar_info(root_name, 0o755, 0, source_date_epoch, tarfile.DIRTYPE))
                    for name in sorted(REQUIRED_FILES):
                        source = file_sources[name]
                        mode = 0o755 if name == "stack" else 0o644
                        with source.open("rb") as source_stream:
                            archive.addfile(
                                tar_info(f"{root_name}/{name}", mode, source.stat().st_size, source_date_epoch),
                                source_stream,
                            )
        os.chmod(temporary, 0o644)
        os.link(temporary, destination)
    finally:
        temporary.unlink(missing_ok=True)

    verify_archive(destination, target, version, source_date_epoch, binary_path)
    return destination


def verify_gzip_header(archive, source_date_epoch):
    with archive.open("rb") as stream:
        header = stream.read(10)
    require(len(header) == 10 and header[0:2] == b"\x1f\x8b", "archive is not gzip data")
    require(header[2] == 8, "archive must use deflate compression")
    require(header[3] & 0x08 == 0, "gzip header must not contain a source filename")
    require(int.from_bytes(header[4:8], "little") == source_date_epoch, "gzip timestamp is not SOURCE_DATE_EPOCH")


def verify_archive(archive, target, version, source_date_epoch, expected_binary=None):
    require(target in TARGETS, f"unsupported release target: {target}")
    require(VERSION_PATTERN.fullmatch(version) is not None, "invalid release version")
    require(version == cargo_version(), "release version must match Cargo.toml")
    require(0 <= source_date_epoch <= 0xFFFFFFFF, "SOURCE_DATE_EPOCH is outside the gzip timestamp range")

    archive_path = Path(archive).absolute()
    archive_stat = archive_path.lstat()
    require(stat.S_ISREG(archive_stat.st_mode), "release archive must be a regular file, not a symlink")
    require(0 < archive_stat.st_size <= MAXIMUM_FILE_BYTES, "release archive size is invalid")
    require(archive_path.name == archive_name(target, version), "release archive filename is invalid")
    verify_gzip_header(archive_path, source_date_epoch)

    root_name = archive_root(target, version)
    expected_names = [root_name, *[f"{root_name}/{name}" for name in sorted(REQUIRED_FILES)]]
    with tarfile.open(archive_path, "r:gz") as release_tar:
        members = release_tar.getmembers()
        require([member.name for member in members] == expected_names, "archive entries must use exact bytewise order")
        require(members[0].isdir(), "archive root must be a directory")
        require(members[0].mode == 0o755, "archive root mode must be 0755")
        total_size = 0
        for member in members:
            require(member.uid == 0 and member.gid == 0, f"archive ownership is invalid: {member.name}")
            require(member.uname == "" and member.gname == "", f"archive owner names are invalid: {member.name}")
            require(member.mtime == source_date_epoch, f"archive timestamp is invalid: {member.name}")
            require(not member.issym() and not member.islnk(), f"archive links are forbidden: {member.name}")
            total_size += member.size
        require(total_size <= MAXIMUM_FILE_BYTES, "archive expands beyond the release size limit")

        for name in sorted(REQUIRED_FILES):
            member = release_tar.getmember(f"{root_name}/{name}")
            require(member.isfile(), f"archive entry must be a regular file: {name}")
            require(member.mode == (0o755 if name == "stack" else 0o644), f"archive mode is invalid: {name}")
            extracted = release_tar.extractfile(member)
            require(extracted is not None, f"archive entry cannot be read: {name}")
            if name == "stack" and expected_binary is not None:
                expected_digest = sha256_file(expected_binary)
                require(sha256_stream(extracted) == expected_digest, "archived binary differs from its build output")
            elif name != "stack":
                source = archive_file_sources(expected_binary)[name]
                require(extracted.read() == source.read_bytes(), f"archive file differs from source: {name}")

    return {"archive": archive_path.name, "entries": len(expected_names), "sha256": sha256_file(archive_path)}


def main():
    parser = argparse.ArgumentParser(description="Create and verify deterministic Stack CLI release archives")
    subparsers = parser.add_subparsers(dest="command", required=True)

    create = subparsers.add_parser("create")
    create.add_argument("--binary", required=True)
    create.add_argument("--target", required=True)
    create.add_argument("--version", required=True)
    create.add_argument("--source-date-epoch", required=True, type=int)
    create.add_argument("--output-directory", required=True)

    verify = subparsers.add_parser("verify")
    verify.add_argument("--archive", required=True)
    verify.add_argument("--binary")
    verify.add_argument("--target", required=True)
    verify.add_argument("--version", required=True)
    verify.add_argument("--source-date-epoch", required=True, type=int)

    arguments = parser.parse_args()
    try:
        if arguments.command == "create":
            result = create_archive(
                arguments.binary,
                arguments.target,
                arguments.version,
                arguments.source_date_epoch,
                arguments.output_directory,
            )
            print(result)
        else:
            result = verify_archive(
                arguments.archive,
                arguments.target,
                arguments.version,
                arguments.source_date_epoch,
                arguments.binary,
            )
            print(f"verified {result['archive']} ({result['entries']} entries, sha256:{result['sha256']})")
    except (OSError, tarfile.TarError, ValueError, zlib.error) as error:
        raise SystemExit(str(error)) from error


if __name__ == "__main__":
    main()
