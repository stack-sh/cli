# Distribution contract

This document defines the shared release contract for the Stack CLI. It is normative for GitHub Releases, Homebrew, Cargo, and Aqua implementations. The machine-readable source is [`distribution/distribution-contract.json`](../distribution/distribution-contract.json).

[Stack CLI 0.5.2](https://github.com/stack-sh/cli/releases/tag/v0.5.2) is available as a supported GitHub Release for every target below, through the owner-maintained Homebrew tap for the hosts marked below, and through the checksum-locked owner Aqua registry. The registry-only Cargo source package is also available. Version 0.5.0 removes self-update; see the [upgrade guide](./self-update.md).

## Supported platform matrix

The first supported binary matrix is intentionally narrow:

| Rust target | OS | Architecture | Runtime floor | Direct | Homebrew | Cargo | Aqua |
| --- | --- | --- | --- | --- | --- | --- | --- |
| `aarch64-apple-darwin` | macOS | arm64 | macOS 13 | available | available | available | available |
| `x86_64-apple-darwin` | macOS | x86_64 | macOS 13 | available | — | available | available |
| `aarch64-unknown-linux-gnu` | Linux | arm64 | glibc 2.31 | available | available | available | available |
| `x86_64-unknown-linux-gnu` | Linux | x86_64 | glibc 2.31 | available | available | available | available |

Windows, musl-based Linux distributions such as Alpine, BSD, and 32-bit architectures are not supported release targets. A source build may happen to work elsewhere, but it is best-effort and does not block a release. Cargo installs on supported targets require Rust 1.85 or newer. Homebrew availability additionally follows [Homebrew's current tier-1 host requirements](https://docs.brew.sh/Support-Tiers); Stack does not label a host as supported when the package manager itself classifies it below tier 1.

`tier-1` means the release must build, verify, and smoke-test that target. Missing or failing evidence blocks the entire stable release; it is not acceptable to publish a partial stable matrix.

## Version and support policy

- Cargo `package.version`, CLI output, the Git tag `v{version}`, release title, archive names, and release manifest version must agree exactly.
- Stable versions use `MAJOR.MINOR.PATCH`. Release candidates use `MAJOR.MINOR.PATCH-rc.N`, are GitHub prereleases, and are never selected by default by package managers.
- Before 1.0, only the latest stable release is supported. Starting at 1.0, the latest two minor lines are supported.
- Release-manifest schema v1 retains `minimumSupportedCliVersion` for compatibility and sets it to the release version. It does not enable self-update: new manifests never include that channel. Distribution contract v3 declares the Cargo package name while retaining installer ownership. Historical v1/v2 schemas remain unchanged. Distribution contract v2 removed the updater channel, receipt requirement, and activation rules; the original v1 schema and receipt schema remain unchanged for historical consumers.
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

Each archive contains one directory named `stack-v{version}-{target}` with `stack`, `LICENSE`, `NOTICE`, `THIRD_PARTY_LICENSES.md`, and the generated files below. Archives use bytewise path order, numeric uid/gid 0, `SOURCE_DATE_EPOCH` for entry times, and a gzip header without a source filename or wall-clock timestamp.

```text
share/bash-completion/completions/stack
share/zsh/site-functions/_stack
share/fish/vendor_completions.d/stack.fish
share/man/man1/stack.1
```

The target binary must reproduce those exact bytes through `stack completions <SHELL>` and `stack manpage`. See the [shell completion and manual contract](./completions.md) for package-manager and user-owned installation paths.

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

## Cargo installation

Install the official source package with Rust 1.85 or newer and a working native linker:

```sh
cargo install stack-diagram-cli --version 0.5.2 --locked
stack --version
```

The package name is `stack-diagram-cli`; the executable name is `stack`. The unrelated `stack-cli` crate is not this project. The complete locked dependency graph comes from crates.io, including `stack-compiler 0.1.0`, `stack-theme 0.5.0`, `stack-formatter 0.1.0`, and `stack-engine 0.7.0`. The source package contains the embedded templates/catalogs and Apache-2.0 license, notice, and dependency attribution.

Cargo places the executable in its installation root (normally `$CARGO_HOME/bin`, defaulting to `$HOME/.cargo/bin`); ensure that directory is on `PATH`. macOS needs the Xcode Command Line Tools, and Linux needs a native C compiler/linker. Supported Cargo targets are the same four native targets above. The runtime floors in the table describe prebuilt GitHub archives: Cargo compiles on your host with your local toolchain, and its binary runtime requirements and bytes may differ. Cargo source installation is not a Sigstore-attested prebuilt archive.

Upgrade through Cargo by installing the desired published version with `--locked`, or use `cargo install stack-diagram-cli --locked` for the latest release. Uninstall with `cargo uninstall stack-diagram-cli`. Choose one installer for each binary location; do not use Cargo to replace a Homebrew- or Aqua-owned executable. Stack never replaces its own executable. Cargo does not place shell completions or manual pages automatically; use the generators described in the [shell integration guide](./completions.md). Configuration and imported icons remain outside Cargo’s binary installation root and are not removed on uninstall.

Initial publication verifies an isolated, registry-only installation on macOS and GNU/Linux arm64 / x86_64 with both Rust 1.85.0 and stable, then exercises version/help, templates, validation, rendering, JSON output, configuration, and generated shell assets. The [publication procedure](./cargo-releasing.md) records the bootstrap credential boundary.

## Homebrew installation

The owner-maintained [`stack-sh/homebrew-tap`](https://github.com/stack-sh/homebrew-tap) installs the canonical GitHub Release archive without rebuilding or repacking it. Homebrew is available on Apple Silicon macOS and glibc-based Linux on arm64 and x86_64 when the host meets Homebrew's current tier-1 requirements.

Install, upgrade, or uninstall with:

```sh
brew install stack-sh/tap/stack
brew upgrade stack-sh/tap/stack
brew uninstall stack-sh/tap/stack
```

For releases carrying the generated assets, the formula installs bash, zsh, and fish completions plus `stack.1` through Homebrew's standard path helpers. It does not edit shell startup files. The formula does not remove or replace Stack configuration and icon stores during an upgrade or uninstall. Formula updates verify release checksums, provenance, and SBOM attestations before changing the archive mapping. The fail-closed update and recovery procedure is maintained in the tap's [maintainer guide](https://github.com/stack-sh/homebrew-tap/blob/main/docs/maintaining.md).

The Homebrew v0.5.2 formula was activated after the immutable release assets were published. Its macOS ARM64, Linux ARM64, and Linux x86_64 lifecycle tests verify the archived completion and manual bytes during install, upgrade, and uninstall. The release manifest remains the publication-time record with only `github-release` in `verifiedChannels`; this contract and the tap CI record the later channel verification without replacing any release asset.

## Aqua installation

The owner registry is the [`aqua/registry.yaml`](../aqua/registry.yaml) file pinned to immutable commit `42702cda91a4156901b9a601bd143c43dcf05766`. Aqua maps `darwin/amd64`, `darwin/arm64`, `linux/amd64`, and `linux/arm64` to the four canonical GitHub Release archives, reads their SHA-256 values from the signed checksum asset, and verifies the checksum bundle against the tagged `release.yaml` workflow identity.

Add the following `aqua.yaml` at the root of a Git repository (the directory containing `.git`). Run `git init` first for a new project:

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
  - name: stack-sh/cli@v0.5.2
    registry: stack-sh
```

Because Aqua denies non-standard registries by default, add and review this narrow `aqua-policy.yaml` at the same repository root rather than disabling policy:

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

Commit `aqua-checksums.json` with the configuration. To upgrade after a new stable Stack release, run `aqua update`, review the version change, then run `aqua update-checksum` and `aqua install`. Aqua owns the replacement; Stack never replaces its own executable. The registry maintainer procedure and four-target test command are in [`aqua/README.md`](../aqua/README.md).

Aqua installs the executable declared by its registry mapping and does not own shell startup files or a global manual database. Stack CLI 0.5.2 includes the generators; use `stack completions` and `stack manpage` to write the desired user-owned files as documented in the [completion guide](./completions.md).

The Aqua v0.5.2 pin was activated after the immutable release assets were published. CI verifies all four target mappings without executing foreign binaries, then installs the native archive and compares all completion and manual generator bytes with the matching checksum-locked canonical release archive. This keeps the published release check independent from later source-tree command additions. The release manifest remains the publication-time record; the pinned registry commit, generated checksum lock, contract, and CI runs are the later verification evidence. No release asset is replaced.

## Direct installation

Download [Stack CLI 0.5.2](https://github.com/stack-sh/cli/releases/tag/v0.5.2), select the archive whose target matches the supported platform table, and obtain all matching verification material. Complete the [supply-chain verification](./supply-chain.md), then extract and install the verified binary. Replace `{target}` with the exact release target:

```sh
tar -xzf "stack-v0.5.2-{target}.tar.gz"
mkdir -p "$HOME/.local/bin"
install -m 0755 "stack-v0.5.2-{target}/stack" "$HOME/.local/bin/stack"
"$HOME/.local/bin/stack" --version
```

Add `$HOME/.local/bin` to `PATH` if it is not already present. Repeat the verified manual installation to update a directly downloaded binary; never overwrite a package-manager-owned binary. No receipt is created or required. See the [upgrade guide](./self-update.md).

The 0.5.2 archive carries completion and manual assets. Either copy its verified `share/` files into the matching system prefix or use the installed binary to generate user-owned files following the [completion guide](./completions.md). Do not copy these files from a different Stack version; CI and release verification require them to match the binary's command definition.

## Channel ownership

| Channel | Owns | Must not do |
| --- | --- | --- |
| GitHub Releases | Canonical immutable archives with generated completions and manual, manifest, checksums, signature bundle, SBOMs, and provenance | Replace a tag or asset after publication |
| Homebrew | Formula metadata, archive URL/digest mapping, standard completion/manual placement, install, upgrade, and uninstall | Rebuild a different binary or delegate upgrades to `stack` |
| Cargo | The `stack-diagram-cli` crates.io source package, registry-only locked dependencies, and installation of the `stack` binary | Claim binary-archive identity, claim the unrelated `stack-cli` package, or publish while dependencies remain Git-only |
| Aqua | Registry metadata and version pinning mapped to canonical archives and digests | Repack an archive or select prereleases by default |

Stack does not provide a self-updater or a receipt-writing installer. Update through the tool that installed the binary, or verify and manually install a new GitHub archive for a direct download. Existing receipts are neither read nor deleted.

The source and published Cargo package names are both `stack-diagram-cli`; the installed binary remains `stack`. Registry ownership and supported-target installation are verified before activating this channel.

## Release activation and rollback

### Continuous clean-install verification

[`Distribution smoke`](../.github/workflows/distribution-smoke.yaml) runs on pull requests and pushes to `main`, and accepts a manual exact stable version. Without an override it tests `currentReleaseVersion` from the distribution contract, not the possibly unpublished source version. It resolves the immutable release source and the official tap revision once before starting:

| Channel | Native installation cells |
| --- | --- |
| Direct archive | Four supported targets |
| Aqua 2.62.3 | Four supported targets, fresh Git project and Aqua store |
| Cargo | Four supported targets, each with Rust 1.85.0 and stable; fresh registry cache and build directory |
| Homebrew | Apple Silicon macOS, GNU/Linux arm64 and x86_64; fresh Stack formula prefix and download cache |

All 19 cells execute the installed binary on the matching native architecture. They check the exact version, help, configuration, doctor, templates, validation, formatting, SVG/JSON output, and completion/manual generation against the **published source**. Each also explicitly imports the audited Simple Icons catalog into a disposable store and renders an imported icon with its attribution. Only result metadata is uploaded; imported artwork and rendered provider examples are not redistributed as CI artifacts.

Direct, Aqua, and Homebrew binaries must byte-match the canonical archive after checksum, source-bound GitHub provenance, and archive-layout verification. Cargo must install the exact registry package with `--locked` and match its packaged source commit; it is not expected to reproduce prebuilt binary bytes. Homebrew additionally checks installed completion/manual files. Fresh installations do not use the repository's Cargo build output or an existing Stack configuration/icon store.

The `distribution smoke completion` job always evaluates the context and the whole requested matrix. A failure, cancellation, skip, missing artifact, wrong version, or mismatched digest prevents success. The job summary identifies the version and scope; GitHub Actions reports failure through its normal workflow notifications. Maintainers should watch **Actions** notifications for this repository and inspect the failed matrix cell before retrying; a retry is not a substitute for resolving a reproducible failure.

The reusable workflow also exposes `native` (Direct + Aqua, eight cells) and `cargo` (eight cells) scopes for publication integration. The Release workflow calls the native scope after stable publication; Cargo trusted publishing calls the Cargo scope after an actual upload. Cargo-only checks require the exact published source commit and do not assume the GitHub Release already exists. A scoped success is **not** all-channel activation. Full activation requires a successful `all` run for the same stable version after the tap and registry are available. These later results supplement the immutable publication-time manifest; they never rewrite a tag, release asset, or its `verifiedChannels` field.

Both publication workflows have an always-running completion job that rejects missing, failed, cancelled, or unexpectedly skipped required work. Native release builds exercise a real audited provider import/render before publication, including manual dry runs. A native dry run verifies newly built artifacts without requiring an unpublished version to exist in a package manager. Release candidates retain those build checks but skip stable package-manager installation; they do not activate stable channels. Cargo's no-upload OIDC verification similarly does not claim installation of a new version.

After an upload, a failing installation makes the workflow fail but does not undo publication. Inspect the named failed cell and registry/release state first, fix the cause, then re-run failed verification jobs only. Never rerun a successful upload job or overwrite release assets to make the run green. If the published package itself is broken, follow the withdrawal and new-patch procedure below.

Run the negative guards locally with:

```sh
node --test scripts/distribution-smoke-context.test.mjs scripts/distribution-smoke-workflow.test.mjs scripts/publication-completion.test.mjs
python3 -m unittest scripts/test_smoke_installed_cli.py
```

A channel becomes available only after all of its target builds and clean-install smoke tests pass. A stable GitHub release additionally requires matching tag/version metadata, complete archive contents, valid checksums and Sigstore bundle, inspectable SPDX SBOMs and provenance, exact generated completion/manual bytes, and successful `stack --version`, `help`, `init`, `check`, and `render` smoke tests on every tier-1 target.

Tags and assets are immutable. For a broken release, mark it as withdrawn, exclude it from default update resolution, restore package-manager metadata to the last verified release, and publish a new patch version. Do not overwrite the broken tag or assets. Cargo may yank a broken package version, but yanking is not deletion and the replacement still uses a new version.

## Contract validation

Run:

```sh
node scripts/validate-distribution-contract.mjs
node --test scripts/distribution-contract.test.mjs
```

The validator checks source-version and MSRV drift, the exact target and channel sets, target-to-channel references, artifact naming placeholders, archive requirements, package-manager ownership, unsupported-platform declarations, and release activation requirements.
