# Stack CLI

`stack-sh/cli` is the open-source native Rust `stack` command for Stack architecture diagrams.

The repository contains native validation, formatting, and rendering commands. The interface remains pre-release and no supported binary distribution is published yet.

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

The CLI links `stack-engine` as a native Rust dependency. It owns filesystem and standard-stream behavior, process exit codes, configuration discovery, provider-pack import, notice output, and command presentation. It must not duplicate compiler, formatter, layout, or SVG-rendering logic.

The bundled engine resolves the provider-neutral core icons `api`, `web`, `mobile`, `desktop`, `server`, `container`, `cluster`, `cloud`, `scheduler`, `webhook`, `identity`, and `observability`. Vendor assets are not bundled unless their exact terms permit software redistribution. Future provider-pack support will preserve upstream artwork and attach source and terms metadata.

## Development

The CLI requires Rust 1.85 or newer.

```sh
cargo run -- check arch.stack
cargo test --locked
cargo clippy --all-targets --all-features --locked -- -D warnings
```

CI validates formatting, unit and process-level integration tests, at least 90% line/region coverage and 95% function coverage, Clippy, documentation, a release build, `--help`, and `--version` on stable Rust. Tests and Clippy also run on Rust 1.85.

Canonical formatter behavior is checked against the pinned `stack-sh/specification` fixture revision recorded in `tests/specification-revision`.

See [CONTRIBUTING.md](./CONTRIBUTING.md) before opening a change. Please report security vulnerabilities through the process in [SECURITY.md](./SECURITY.md), not a public issue.

## Licensing

Repository-authored work is licensed under the [Apache License 2.0](./LICENSE) for personal and commercial use. Runtime and build dependency licenses are recorded in [THIRD_PARTY_LICENSES.md](./THIRD_PARTY_LICENSES.md). A future binary release must ship the applicable license and notice files described there.
