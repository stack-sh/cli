# Contributing to Stack CLI

Thank you for helping improve the native Stack command-line experience.

## Before starting

- Search existing issues before opening a new one.
- Keep changes focused on native command behavior, host I/O, exit codes, configuration discovery, provider-pack import, or notice output.
- Propose language, compiler, theme, formatter, layout, or SVG-rendering changes in the repository that owns that contract.
- Do not include credentials, private data, signing material, or third-party assets without complete provenance and redistribution terms.

For a larger behavioral change, open an issue first so its repository and compatibility boundary can be agreed before implementation.

## Development checks

The minimum supported Rust version is 1.85. Run the same core checks used by CI:

```sh
cargo fmt --check
cargo test --locked
cargo clippy --all-targets --all-features --locked -- -D warnings
RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --locked
cargo build --release --locked
```

Formatter conformance also requires a checkout of the pinned Stack specification revision:

```sh
STACK_SPECIFICATION_DIR=../specification \
  cargo test --features conformance --test formatter-conformance --locked
```

## Pull requests

- Write source, comments, commits, issues, and pull requests in English.
- Add tests for behavior changes and keep standard output machine-readable.
- Explain compatibility, host-I/O, and license effects in the pull request.
- Keep generated output, local state, and build artifacts out of commits.

Unless explicitly stated otherwise, contributions intentionally submitted for inclusion are licensed under Apache-2.0 as described by Section 5 of the repository license.
