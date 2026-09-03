# Stack CLI

`stack-sh/cli` is the private source repository for the native Rust `stack` command.

The repository now contains the first native command: `stack check`. The CLI is not yet distributed as a supported external binary and its interface remains pre-release.

## Commands

```text
stack check arch.stack
```

`stack check` reads the file as bytes and runs the full compiler, theme, layout, and routing validation pipeline without changing the source. Diagnostics are written to standard error in source order. Standard output remains empty.

| Result | Exit status |
| --- | ---: |
| No error diagnostics, including warning-only input | `0` |
| One or more Stack error diagnostics | `1` |
| Invalid arguments, host I/O failure, or engine operational failure | `2` |

The remaining planned commands are:

```text
stack render arch.stack -o arch.svg
stack fmt arch.stack
stack fmt --check arch.stack
```

The CLI will link `stack-engine` as a native Rust dependency. It owns filesystem and standard-stream behavior, process exit codes, configuration discovery, and command presentation. It must not duplicate compiler, formatter, layout, or SVG-rendering logic.

Future authenticated theme delivery may add a client for short-lived, scope-limited Stack tokens and entitlement-aware theme downloads. Credentials and downloaded paid-theme contents must never be committed to this repository.

## Development

The CLI requires Rust 1.85 or newer.

```sh
cargo run -- check arch.stack
cargo test --locked
cargo clippy --all-targets --locked -- -D warnings
```

CI validates formatting, unit and process-level integration tests, at least 90% line/region coverage and 95% function coverage, Clippy, documentation, a release build, `--help`, and `--version` on stable Rust. Tests and Clippy also run on Rust 1.85.

## Licensing

The private source code in this repository is not currently offered under an open-source license. See [LICENSING.md](./LICENSING.md) for the decisions required before external distribution.
