# Stack CLI

`stack-sh/cli` is the private source repository for the native Rust `stack` command.

The repository contains native validation, formatting, and rendering commands. The CLI is not yet distributed as a supported external binary and its interface remains pre-release.

## Commands

```text
stack check arch.stack
stack fmt arch.stack
stack fmt --check arch.stack
stack fmt -
stack render arch.stack
stack render arch.stack -o arch.svg
```

`stack check` reads the file as bytes and runs the full compiler, theme, layout, and routing validation pipeline without changing the source. Diagnostics are written to standard error in source order. Standard output remains empty.

`stack fmt` uses the engine formatter and preserves comments. File mode replaces changed source atomically through a temporary file in the same directory; unchanged files are not replaced. Syntax, encoding, and host I/O failures leave the original file untouched. `stack fmt -` reads bytes from standard input and writes only canonical source to standard output. `--check` never writes source and exits with status `1` when formatting is required.

`stack render` uses the same engine pipeline to produce deterministic standalone SVG. Without `-o`, standard output contains only SVG. With `-o`, the output is written atomically in the destination directory. Diagnostics remain on standard error, warnings preserve SVG, and Stack errors never create or replace output.

| Result | Exit status |
| --- | ---: |
| No error diagnostics, including warning-only input | `0` |
| One or more Stack error diagnostics, or `fmt --check` finds a difference | `1` |
| Invalid arguments, host I/O failure, or engine operational failure | `2` |

The CLI will link `stack-engine` as a native Rust dependency. It owns filesystem and standard-stream behavior, process exit codes, configuration discovery, and command presentation. It must not duplicate compiler, formatter, layout, or SVG-rendering logic.

Future authenticated theme delivery may add a client for short-lived, scope-limited Stack tokens and entitlement-aware theme downloads. Credentials and downloaded paid-theme contents must never be committed to this repository.

## Development

The CLI requires Rust 1.85 or newer.

```sh
cargo run -- check arch.stack
cargo test --locked
cargo clippy --all-targets --all-features --locked -- -D warnings
```

CI validates formatting, unit and process-level integration tests, at least 90% line/region coverage and 95% function coverage, Clippy, documentation, a release build, `--help`, and `--version` on stable Rust. Tests and Clippy also run on Rust 1.85.

Canonical formatter behavior is checked against the pinned `stack-sh/specification` fixture revision recorded in `tests/specification-revision`.

## Licensing

The private source code in this repository is not currently offered under an open-source license. See [LICENSING.md](./LICENSING.md) for the decisions required before external distribution.
