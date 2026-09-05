#!/usr/bin/env python3
import argparse
import os
from pathlib import Path
import re
import stat
import subprocess
import tempfile
import xml.etree.ElementTree as ET


ROOT = Path(__file__).resolve().parent.parent
TARGETS = {
    "aarch64-apple-darwin": ("Mach-O", "arm64"),
    "x86_64-apple-darwin": ("Mach-O", "x86_64"),
    "aarch64-unknown-linux-gnu": ("ELF", "ARM aarch64"),
    "x86_64-unknown-linux-gnu": ("ELF", "x86-64"),
}
MAXIMUM_BINARY_BYTES = 256 * 1024 * 1024
GENERATED_COMMANDS = {
    "share/bash-completion/completions/stack": ("completions", "bash"),
    "share/fish/vendor_completions.d/stack.fish": ("completions", "fish"),
    "share/man/man1/stack.1": ("manpage",),
    "share/zsh/site-functions/_stack": ("completions", "zsh"),
}


def require(condition, message):
    if not condition:
        raise ValueError(message)


def command(arguments, working_directory=None, environment=None, allow_stderr=False):
    completed = subprocess.run(
        [str(argument) for argument in arguments],
        cwd=working_directory,
        env=environment,
        check=False,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
    )
    if completed.returncode != 0:
        details = completed.stderr.decode("utf-8", errors="replace").strip()
        raise ValueError(f"command failed ({completed.returncode}): {' '.join(map(str, arguments))}: {details}")
    require(allow_stderr or completed.stderr == b"", f"command emitted unexpected diagnostics: {' '.join(map(str, arguments))}")
    return completed.stdout


def verify_architecture(binary, target):
    require(target in TARGETS, f"unsupported release target: {target}")
    description = command(["file", "-b", binary]).decode("utf-8")
    for expected in TARGETS[target]:
        require(expected in description, f"binary architecture mismatch for {target}: {description.strip()}")


def verify_linux_runtime(binary, target):
    version_info = command(["readelf", "--version-info", binary]).decode("utf-8")
    versions = {(int(major), int(minor)) for major, minor in re.findall(r"GLIBC_(\d+)\.(\d+)", version_info)}
    require(versions, "GNU/Linux binary does not declare any glibc requirements")
    require(max(versions) <= (2, 31), f"GNU/Linux binary requires glibc {max(versions)[0]}.{max(versions)[1]}")

    program_headers = command(["readelf", "--program-headers", binary]).decode("utf-8")
    interpreter = {
        "aarch64-unknown-linux-gnu": "/lib/ld-linux-aarch64.so.1",
        "x86_64-unknown-linux-gnu": "/lib64/ld-linux-x86-64.so.2",
    }[target]
    require(interpreter in program_headers, f"GNU/Linux interpreter mismatch: expected {interpreter}")


def verify_macos_runtime(binary):
    load_commands = command(["otool", "-l", binary]).decode("utf-8")
    minimums = re.findall(r"^\s+minos\s+(\d+)\.(\d+)(?:\.\d+)?$", load_commands, re.MULTILINE)
    if not minimums:
        minimums = re.findall(
            r"cmd LC_VERSION_MIN_MACOSX.*?^\s+version\s+(\d+)\.(\d+)(?:\.\d+)?$",
            load_commands,
            re.MULTILINE | re.DOTALL,
        )
    require(minimums and {(int(major), int(minor)) for major, minor in minimums} == {(13, 0)}, "macOS deployment target must be exactly 13.0")
    require(load_commands.count("cmd LC_UUID") == 1, "macOS release binary must contain exactly one UUID")
    command(["codesign", "--verify", "--strict", binary], allow_stderr=True)
    signature = subprocess.run(
        ["codesign", "--display", "--verbose=4", str(binary)],
        check=False,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
    )
    require(signature.returncode == 0, "macOS release signature metadata cannot be read")
    details = signature.stderr.decode("utf-8", errors="replace")
    require("Identifier=sh.stack.cli\n" in details, "macOS release signature identifier is invalid")
    require("Signature=adhoc\n" in details, "macOS release binary must use an ad-hoc signature")


def verify_commands(binary, version):
    expected_version = f"stack {version}\n".encode()
    require(command([binary, "--version"]) == expected_version, "--version output does not match Cargo version")
    require(command([binary, "version"]) == expected_version, "version command output does not match Cargo version")
    require(b"Usage:" in command([binary, "help"]), "help output is missing usage")
    require(
        b"stack lsp" in command([binary, "lsp", "--help"]),
        "LSP help output is missing usage",
    )
    require(
        b"stack update" in command([binary, "update", "--help"]),
        "update help output is missing usage",
    )
    for relative_path, arguments in GENERATED_COMMANDS.items():
        expected = (ROOT / "distribution/generated" / relative_path).read_bytes()
        require(
            command([binary, *arguments]) == expected,
            f"generated CLI asset differs from source: {relative_path}",
        )

    with tempfile.TemporaryDirectory(prefix="stack-release-smoke-") as temporary:
        working_directory = Path(temporary)
        environment = os.environ.copy()
        environment["XDG_CONFIG_HOME"] = str(working_directory / "config")
        command([binary, "init"], working_directory, environment)
        source = working_directory / "diagram.stack"
        require(source.is_file() and source.stat().st_size > 0, "stack init did not create diagram.stack")
        command([binary, "check", source], working_directory, environment)
        rendered = working_directory / "diagram.svg"
        command([binary, "render", source, "-o", rendered], working_directory, environment)
        require(rendered.is_file() and rendered.stat().st_size > 0, "stack render did not create SVG")
        svg = ET.fromstring(rendered.read_bytes())
        require(svg.tag == "{http://www.w3.org/2000/svg}svg", "rendered output is not an SVG root")
        require(svg.attrib.get("viewBox"), "rendered SVG has no viewBox")


def verify_release_binary(binary, target, version):
    binary_path = Path(binary).absolute()
    binary_stat = binary_path.lstat()
    require(stat.S_ISREG(binary_stat.st_mode), "release binary must be a regular file, not a symlink")
    require(0 < binary_stat.st_size <= MAXIMUM_BINARY_BYTES, "release binary size is invalid")
    require(os.access(binary_path, os.X_OK), "release binary must be executable")
    require(
        b"STACK_CLI_TEST_UPDATE_BASE_URL" not in binary_path.read_bytes(),
        "release binary contains the debug-only update endpoint override",
    )
    verify_architecture(binary_path, target)
    if target.endswith("linux-gnu"):
        verify_linux_runtime(binary_path, target)
    else:
        verify_macos_runtime(binary_path)
    verify_commands(binary_path, version)
    return binary_path


def main():
    parser = argparse.ArgumentParser(description="Verify a native Stack CLI release binary")
    parser.add_argument("--binary", required=True)
    parser.add_argument("--target", required=True)
    parser.add_argument("--version", required=True)
    arguments = parser.parse_args()
    try:
        binary = verify_release_binary(arguments.binary, arguments.target, arguments.version)
        print(f"validated native release binary {binary.name} for {arguments.target}")
    except (OSError, ValueError, ET.ParseError) as error:
        raise SystemExit(str(error)) from error


if __name__ == "__main__":
    main()
