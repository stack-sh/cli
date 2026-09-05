# Distribution contract

This document defines the shared release contract for the Stack CLI. It is normative for GitHub Releases, Homebrew, Cargo, Aqua, and `stack` self-update implementations. The machine-readable source is [`distribution/distribution-contract.json`](../distribution/distribution-contract.json).

[Stack CLI 0.3.0](https://github.com/stack-sh/cli/releases/tag/v0.3.0) is available as a supported GitHub Release for every target below, through the owner-maintained Homebrew tap for the hosts marked below, and through the checksum-locked owner Aqua registry. Cargo and self-update remain **planned** and have no supported install command yet.

## Supported platform matrix

The first supported binary matrix is intentionally narrow:

| Rust target | OS | Architecture | Runtime floor | Direct | Homebrew | Cargo | Aqua | Self-update |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| `aarch64-apple-darwin` | macOS | arm64 | macOS 13 | available | available | planned | available | planned |
| `x86_64-apple-darwin` | macOS | x86_64 | macOS 13 | available | — | planned | available | planned |
| `aarch64-unknown-linux-gnu` | Linux | arm64 | glibc 2.31 | available | available | planned | available | planned |
| `x86_64-unknown-linux-gnu` | Linux | x86_64 | glibc 2.31 | available | available | planned | available | planned |

Windows, musl-based Linux distributions such as Alpine, BSD, and 32-bit architectures are not supported release targets. A source build may happen to work elsewhere, but it is best-effort and does not block a release. Cargo installs on supported targets require Rust 1.85 or newer. Homebrew availability additionally follows [Homebrew's current tier-1 host requirements](https://docs.brew.sh/Support-Tiers); Stack does not label a host as supported when the package manager itself classifies it below tier 1.

`tier-1` means the release must build, verify, and smoke-test that target. Missing or failing evidence blocks the entire stable release; it is not acceptable to publish a partial stable matrix.

## Version and support policy

- Cargo `package.version`, CLI output, the Git tag `v{version}`, release title, archive names, and release manifest version must agree exactly.
- Stable versions use `MAJOR.MINOR.PATCH`. Release candidates use `MAJOR.MINOR.PATCH-rc.N`, are GitHub prereleases, and are never selected by default by package managers or self-update.
- Before 1.0, only the latest stable release is supported. Starting at 1.0, the latest two minor lines are supported.
- Each stable release manifest records `minimumSupportedCliVersion`. The self-update channel owns this compatibility floor, and release generation copies it forward instead of advancing it automatically with every release. This is the only input used by update clients and documentation to describe the minimum supported updater.
- A Cargo source version alone is not a supported distribution. Support starts only when a stable GitHub Release built from that exact source passes every activation check; changing a version does not reserve or silently publish it.

`.github/workflows/release.yaml` accepts a version-checked manual run from `main` without publication and an annotated `v{version}` tag for publication. A tag run is allowed only for a commit contained in `main`. The manual path must pass first for the same commit and version before a release tag is created.

## Release artifacts

For every supported Rust target, publish:

```text
stack-v{version}-{target}.tar.gz
stack-v{version}-{target}.spdx.json
stack-v{version}-{target}.provenance.sigstore.json
stack-v{version}-{target}.sbom.sigstore.json
```

Each archive contains one directory named `stack-v{version}-{target}` with `stack`, `LICENSE`, `NOTICE`, and `THIRD_PARTY_LICENSES.md`. Archives use bytewise path order, numeric uid/gid 0, `SOURCE_DATE_EPOCH` for entry times, and a gzip header without a source filename or wall-clock timestamp.

Release binaries use Rust 1.85.0 exactly. GNU/Linux builds run natively in the digest-pinned `rust:1.85.0-slim-bullseye` multi-architecture image and install their remaining release tools from the image's dated Debian snapshot, so their maximum glibc requirement is 2.31. macOS builds set `MACOSX_DEPLOYMENT_TARGET=13.0`, replace the path-dependent linker UUID with one derived from unsigned executable content, and apply a timestamp-free ad-hoc signature with a fixed identifier. Every target is built twice into isolated Cargo target directories on one GitHub-hosted native runner; the normalized binary bytes and independently packaged archive bytes must match before upload.

The embedded macOS ad-hoc signature makes the normalized Mach-O executable valid and reproducible; it is not an Apple Developer ID or proof of publisher identity. The release is not notarized. Verify publisher identity and artifact integrity with the keyless Sigstore and GitHub attestation procedure below before installation.

Every release also publishes:

```text
stack-v{version}-release-manifest.json
stack-v{version}-checksums.txt
stack-v{version}-checksums.txt.sigstore.json
```

The sorted checksum file uses SHA-256 and covers the release manifest, all archives, all SPDX SBOMs, and the per-target provenance and SBOM attestation bundles. A keyless Sigstore bundle signs the checksum file; GitHub's [artifact attestation model](https://docs.github.com/actions/concepts/security/artifact-attestations) is the trust baseline. The immutable GitHub Release asset is the canonical binary byte sequence; Homebrew and Aqua must reference its URL and digest instead of rebuilding or repacking it. Aqua uses its [`github_release` package mapping](https://aquaproj.github.io/docs/reference/registry-config/github-release-package) rather than a separate binary build.

The release manifest records the tag, commit, source version, `minimumSupportedCliVersion`, each target's artifact names and SHA-256 values, the build identity, and each channel whose own install smoke test passed. Its schema is [`distribution/release-manifest.schema.json`](../distribution/release-manifest.schema.json). Supply-chain generation and user verification are documented in the [supply-chain guide](./supply-chain.md).

## Homebrew installation

The owner-maintained [`stack-sh/homebrew-tap`](https://github.com/stack-sh/homebrew-tap) installs the canonical GitHub Release archive without rebuilding or repacking it. Homebrew is available on Apple Silicon macOS and glibc-based Linux on arm64 and x86_64 when the host meets Homebrew's current tier-1 requirements.

Install, upgrade, or uninstall with:

```sh
brew install stack-sh/tap/stack
brew upgrade stack-sh/tap/stack
brew uninstall stack-sh/tap/stack
```

The formula does not remove or replace Stack configuration and icon stores during an upgrade or uninstall. Formula updates verify release checksums, provenance, and SBOM attestations before changing the archive mapping. The fail-closed update and recovery procedure is maintained in the tap's [maintainer guide](https://github.com/stack-sh/homebrew-tap/blob/main/docs/maintaining.md).

Homebrew was activated after the immutable `v0.3.0` release assets were published. The release manifest therefore remains the publication-time record, while this contract and the tap CI record the later channel activation; release assets are not replaced to retrofit that state.

## Aqua installation

The owner registry is the [`aqua/registry.yaml`](../aqua/registry.yaml) file pinned to immutable commit `42702cda91a4156901b9a601bd143c43dcf05766`. Aqua maps `darwin/amd64`, `darwin/arm64`, `linux/amd64`, and `linux/arm64` to the four canonical GitHub Release archives, reads their SHA-256 values from the signed checksum asset, and verifies the checksum bundle against the tagged `release.yaml` workflow identity.

Add the following `aqua.yaml` to a Git repository:

```yaml
checksum:
  enabled: true
  require_checksum: true
registries:
  - name: stack-sh
    type: github_content
    repo_owner: stack-sh
    repo_name: cli
    ref: 42702cda91a4156901b9a601bd143c43dcf05766
    path: aqua/registry.yaml
packages:
  - name: stack-sh/cli@v0.3.0
    registry: stack-sh
```

Because Aqua denies non-standard registries by default, add and review this narrow `aqua-policy.yaml` rather than disabling policy:

```yaml
registries:
  - name: stack-sh
    type: github_content
    repo_owner: stack-sh
    repo_name: cli
    ref: 'Version == "42702cda91a4156901b9a601bd143c43dcf05766"'
    path: aqua/registry.yaml
packages:
  - name: stack-sh/cli
    registry: stack-sh
    version: semver(">= 0.3.0")
```

Allow the reviewed policy once, generate the checksum lock, and install:

```sh
aqua policy allow
aqua update-checksum
aqua install
stack --version
```

Commit `aqua-checksums.json` with the configuration. To upgrade after a new stable Stack release, run `aqua update`, review the version change, then run `aqua update-checksum` and `aqua install`. Aqua owns the replacement; `stack` self-update must refuse to overwrite it. The registry maintainer procedure and four-target test command are in [`aqua/README.md`](../aqua/README.md).

Aqua was activated after the immutable `v0.3.0` release assets were published. The release manifest remains the publication-time record; the pinned registry commit, generated checksum lock, contract, and CI runs are the later activation evidence. No release asset is replaced.

## Direct installation

Download [Stack CLI 0.3.0](https://github.com/stack-sh/cli/releases/tag/v0.3.0), select the archive whose target matches the supported platform table, and obtain all matching verification material. Complete the [supply-chain verification](./supply-chain.md), then extract and install the verified binary. Replace `{target}` with the exact release target:

```sh
tar -xzf "stack-v0.3.0-{target}.tar.gz"
mkdir -p "$HOME/.local/bin"
install -m 0755 "stack-v0.3.0-{target}/stack" "$HOME/.local/bin/stack"
"$HOME/.local/bin/stack" --version
```

Add `$HOME/.local/bin` to `PATH` if it is not already present. This manual installation has no self-update receipt. The source tree after 0.3.0 contains `stack update`, but the published 0.3.0 binary does not, and this installation cannot be claimed retroactively without risking a package-manager-owned binary. Self-update remains unavailable until a later release and verified direct installer separately activate the channel. The command and receipt contract are documented in the [self-update guide](./self-update.md).

## Channel ownership

| Channel | Owns | Must not do |
| --- | --- | --- |
| GitHub Releases | Canonical immutable archives, manifest, checksums, signature bundle, SBOMs, and provenance | Replace a tag or asset after publication |
| Homebrew | Formula metadata, archive URL/digest mapping, install, upgrade, and uninstall | Rebuild a different binary or delegate upgrades to `stack` |
| Cargo | A future unambiguous crates.io source package, its registry dependency graph, and installation of the `stack` binary | Claim binary-archive identity, promise the local `stack-cli` package name on crates.io, or publish while dependencies remain Git-only |
| Aqua | Registry metadata and version pinning mapped to canonical archives and digests | Repack an archive or select prereleases by default |
| `stack` self-update | Verified atomic replacement for direct installs with a Stack installation receipt | Replace a binary owned by Homebrew, Cargo, Aqua, or an unknown installer |

The direct installer must create an installation receipt that identifies the GitHub Release channel, installed version, target, source commit, archive digest, and final binary path and digest. Its public format is [`distribution/install-receipt.schema.json`](../distribution/install-receipt.schema.json). Self-update refuses to write when that receipt is absent or names another owner and prints detected or possible package-manager upgrade commands. Paths may improve guidance, but never authorize replacement. This keeps ownership deterministic instead of guessing from an executable path.

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
