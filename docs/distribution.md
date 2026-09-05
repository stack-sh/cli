# Distribution contract

This document defines the shared release contract for the Stack CLI. It is normative for GitHub Releases, Homebrew, Cargo, Aqua, and `stack` self-update implementations. The machine-readable source is [`distribution/distribution-contract.json`](../distribution/distribution-contract.json).

No supported binary or package-manager release is published yet. Every target and channel below is **planned**, not currently available. A stable release changes availability only after its complete matrix passes the activation checks in this document.

## Supported platform matrix

The first supported binary matrix is intentionally narrow:

| Rust target | OS | Architecture | Runtime floor | Direct | Homebrew | Cargo | Aqua | Self-update |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| `aarch64-apple-darwin` | macOS | arm64 | macOS 13 | planned | planned | planned | planned | planned |
| `x86_64-apple-darwin` | macOS | x86_64 | macOS 13 | planned | — | planned | planned | planned |
| `aarch64-unknown-linux-gnu` | Linux | arm64 | glibc 2.31 | planned | planned | planned | planned | planned |
| `x86_64-unknown-linux-gnu` | Linux | x86_64 | glibc 2.31 | planned | planned | planned | planned | planned |

Windows, musl-based Linux distributions such as Alpine, BSD, and 32-bit architectures are not supported release targets. A source build may happen to work elsewhere, but it is best-effort and does not block a release. Cargo installs on supported targets require Rust 1.85 or newer. Homebrew availability additionally follows [Homebrew's current tier-1 host requirements](https://docs.brew.sh/Support-Tiers); Stack does not label a host as supported when the package manager itself classifies it below tier 1.

`tier-1` means the release must build, verify, and smoke-test that target. Missing or failing evidence blocks the entire stable release; it is not acceptable to publish a partial stable matrix.

## Version and support policy

- Cargo `package.version`, CLI output, the Git tag `v{version}`, release title, archive names, and release manifest version must agree exactly.
- Stable versions use `MAJOR.MINOR.PATCH`. Release candidates use `MAJOR.MINOR.PATCH-rc.N`, are GitHub prereleases, and are never selected by default by package managers or self-update.
- Before 1.0, only the latest stable release is supported. Starting at 1.0, the latest two minor lines are supported.
- Each stable release manifest records `minimumSupportedCliVersion`. This is the only input used by update clients and documentation to describe the minimum supported version.
- Existing 0.3.0 source is not a supported distribution. The first published version is selected by its release change; this contract does not reserve or silently publish one.

## Release artifacts

For every supported Rust target, publish:

```text
stack-v{version}-{target}.tar.gz
stack-v{version}-{target}.spdx.json
stack-v{version}-{target}.provenance.sigstore.json
stack-v{version}-{target}.sbom.sigstore.json
```

Each archive contains one directory named `stack-v{version}-{target}` with `stack`, `LICENSE`, `NOTICE`, and `THIRD_PARTY_LICENSES.md`. Archives use bytewise path order, numeric uid/gid 0, `SOURCE_DATE_EPOCH` for entry times, and a gzip header without a source filename or wall-clock timestamp.

Every release also publishes:

```text
stack-v{version}-release-manifest.json
stack-v{version}-checksums.txt
stack-v{version}-checksums.txt.sigstore.json
```

The sorted checksum file uses SHA-256 and covers the release manifest, all archives, all SPDX SBOMs, and the per-target provenance and SBOM attestation bundles. A keyless Sigstore bundle signs the checksum file; GitHub's [artifact attestation model](https://docs.github.com/actions/concepts/security/artifact-attestations) is the trust baseline. The immutable GitHub Release asset is the canonical binary byte sequence; Homebrew and Aqua must reference its URL and digest instead of rebuilding or repacking it. Aqua uses its [`github_release` package mapping](https://aquaproj.github.io/docs/reference/registry-config/github-release-package) rather than a separate binary build.

The release manifest records the tag, commit, source version, `minimumSupportedCliVersion`, each target's artifact names and SHA-256 values, the build identity, and each channel whose own install smoke test passed. Its schema is [`distribution/release-manifest.schema.json`](../distribution/release-manifest.schema.json). Supply-chain generation and user verification are documented in the [supply-chain guide](./supply-chain.md).

## Channel ownership

| Channel | Owns | Must not do |
| --- | --- | --- |
| GitHub Releases | Canonical immutable archives, manifest, checksums, signature bundle, SBOMs, and provenance | Replace a tag or asset after publication |
| Homebrew | Formula metadata, archive URL/digest mapping, install, upgrade, and uninstall | Rebuild a different binary or delegate upgrades to `stack` |
| Cargo | A future unambiguous crates.io source package, its registry dependency graph, and installation of the `stack` binary | Claim binary-archive identity, promise the local `stack-cli` package name on crates.io, or publish while dependencies remain Git-only |
| Aqua | Registry metadata and version pinning mapped to canonical archives and digests | Repack an archive or select prereleases by default |
| `stack` self-update | Verified atomic replacement for direct installs with a Stack installation receipt | Replace a binary owned by Homebrew, Cargo, Aqua, or an unknown installer |

The direct installer must create an installation receipt that identifies the GitHub Release channel, installed version, target, and artifact digest. Self-update refuses to write when that receipt is absent or names another owner and prints the appropriate package-manager upgrade command. This keeps ownership deterministic instead of guessing from an executable path.

The workspace currently uses `stack-cli` as its local Cargo package name, but that name is already occupied by an unrelated crates.io package. No public Cargo install command is supported yet. The Cargo channel must select and verify an unambiguous registry package name, while keeping the installed binary name `stack`, before changing its state to available.

## Release activation and rollback

A channel becomes available only after all of its target builds and clean-install smoke tests pass. A stable GitHub release additionally requires matching tag/version metadata, complete archive contents, valid checksums and Sigstore bundle, inspectable SPDX SBOMs and provenance, and successful `stack --version`, `help`, `init`, `check`, and `render` smoke tests on every tier-1 target.

Tags and assets are immutable. For a broken release, mark it as withdrawn, exclude it from default update resolution, restore package-manager metadata to the last verified release, and publish a new patch version. Do not overwrite the broken tag or assets. Cargo may yank a broken package version, but yanking is not deletion and the replacement still uses a new version.

## Contract validation

Run:

```sh
node scripts/validate-distribution-contract.mjs
node --test scripts/distribution-contract.test.mjs
```

The validator checks source-version and MSRV drift, the exact target and channel sets, target-to-channel references, artifact naming placeholders, archive requirements, package-manager ownership, unsupported-platform declarations, and release activation requirements.
