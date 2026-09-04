# Repository publication audit

Audit date: 2026-09-04

This audit covers making the source repository public. It does not authorize or publish a binary release.

## History and private data

- Gitleaks `8.30.1` scanned all 11 commits and approximately 154 KB from every local and remote ref with redacted output; it found no secret.
- The complete path history contains only source, tests, fixtures, workflow, and repository documentation. No credential-like filename or blob larger than 100 KB is present.
- The repository has no issue, release, Actions artifact, fork, or non-default remote branch. Existing pull requests contain implementation discussion but no private product, customer, credential, or signing data.
- Commit and pull-request history is retained. Publication does not rewrite contributor history.

## License and dependency graph

- Repository-authored work is licensed under Apache-2.0.
- `cargo metadata --locked` reports license metadata for every resolved dependency after the repository license is applied.
- Runtime code consists of Apache-2.0 Stack repositories plus permissively licensed Rust dependencies listed in [`THIRD_PARTY_LICENSES.md`](../THIRD_PARTY_LICENSES.md).
- No vendor icon, font, credential, network client, telemetry client, updater, signing key, or customer data is bundled.

## Public repository surface

- The source remains pre-release; `Cargo.toml` keeps `publish = false` and there is no supported binary release.
- Contributions are accepted under the repository license boundary in [`CONTRIBUTING.md`](../CONTRIBUTING.md).
- Vulnerabilities must be submitted through GitHub private vulnerability reporting as documented in [`SECURITY.md`](../SECURITY.md).
- Public branch protection must require the existing `baseline` and `Minimum supported Rust` checks, reject force pushes and branch deletion, and require pull requests before the visibility change is complete.

## Reproduction

```sh
gitleaks detect --redact --source . --log-opts='--all'
cargo metadata --locked --format-version 1
cargo tree --locked
cargo fmt --check
cargo test --locked
cargo clippy --all-targets --all-features --locked -- -D warnings
RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --locked
cargo build --release --locked
```

The manager repository owns the declarative GitHub visibility, security, and ruleset change. Its `gh infra validate` and `gh infra plan` output must be reviewed and recorded before apply.
