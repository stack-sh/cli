# Stack CLI

`stack-sh/cli` is the private source repository for the native Rust `stack` command.

This repository currently contains only its repository foundation. It does not yet provide a distributable binary or a stable command-line interface.

## Planned commands

```text
stack render arch.stack -o arch.svg
stack check arch.stack
stack fmt arch.stack
stack fmt --check arch.stack
```

The CLI will link `stack-engine` as a native Rust dependency. It owns filesystem and standard-stream behavior, process exit codes, configuration discovery, and command presentation. It must not duplicate compiler, formatter, layout, or SVG-rendering logic.

Future authenticated theme delivery may add a client for short-lived, scope-limited Stack tokens and entitlement-aware theme downloads. Credentials and downloaded paid-theme contents must never be committed to this repository.

## Development

Repository checks currently validate the foundation files on every push and pull request. Rust formatting, linting, tests, and release builds will be added with the first CLI implementation.

## Licensing

The private source code in this repository is not currently offered under an open-source license. See [LICENSING.md](./LICENSING.md) for the decisions required before external distribution.
