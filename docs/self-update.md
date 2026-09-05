# Verified self-update

`stack update` updates only a direct GitHub Release installation that has a matching Stack installation receipt. It never claims an unreceipted binary and never replaces an installation owned by Homebrew, Aqua, Cargo, or an unknown installer.

The command is implemented in the source tree after Stack CLI 0.3.0. The published 0.3.0 binary does not contain it, and the documented 0.3.0 manual installation does not create a receipt. The self-update channel therefore remains **planned** until a later release both activates `self-update` in its authenticated release manifest and has a verified direct installer that creates the receipt. Do not describe 0.3.0 as self-updatable.

## Commands

```sh
stack update --check
stack update
stack update --version 0.4.0
stack update --version 0.4.0-rc.1
```

With no version, GitHub's latest stable release is selected. `--version` accepts an exact stable version or `MAJOR.MINOR.PATCH-rc.N`; release candidates are never selected by default. `--check` resolves release metadata only and does not require a receipt, download artifacts, invoke a verifier, or change files. An exact request for the already-running version is also a local no-op.

Actual replacement requires [GitHub CLI](https://cli.github.com/manual/gh_attestation_verify) with `gh attestation verify`. The verifier constrains both the release manifest and target archive to:

- repository `stack-sh/cli`;
- `.github/workflows/release.yaml` at the exact release tag;
- GitHub's OIDC issuer and a GitHub-hosted runner;
- the manifest's exact source commit;
- SLSA provenance.

The manifest must explicitly list both `github-release` and `self-update` in `verifiedChannels`. HTTPS release metadata supplies the asset name, size, URL, and SHA-256; the authenticated manifest independently binds the source, target archive digest, minimum updater version, and channel activation. The updater compatibility floor is owned by the distribution contract and copied into each release manifest; it does not automatically advance to the new release version.

## Installation receipt

The direct installer owns `$XDG_CONFIG_HOME/stack/install-receipt.json`, falling back to `$HOME/.config/stack/install-receipt.json`. Its format is [`distribution/install-receipt.schema.json`](../distribution/install-receipt.schema.json). A receipt records the owner, repository, exact version and target, source commit, archive name and digest, and the absolute installed-binary path and digest.

Before any network request or write, an actual update requires all receipt fields to match the running executable and verifies its complete SHA-256. A missing, malformed, symlinked, oversized, mismatched, or non-`github-release` receipt fails closed. A canonicalized path recognized as Homebrew, Aqua, or Cargo managed also fails closed even if a forged receipt claims `github-release` ownership. Recognized package-manager ownership produces the corresponding upgrade guidance; an unrecognized path lists the safe alternatives without guessing ownership.

## Replacement and recovery

After both attestations pass, the updater checks the archive's exact root, bytewise entry order, regular-file types, uid/gid, `SOURCE_DATE_EPOCH`, modes, expanded-size limit, and target binary. It writes the candidate next to the installed executable, preserves permissions, syncs it, and runs `--version` before changing the live path.

The live executable is replaced with a same-filesystem rename while a hard-linked rollback copy remains. The new receipt is prepared and synced in its own directory before replacement. If the executable rename fails, the old binary remains at its original path. If the receipt commit fails, the updater restores the original binary from the rollback link. Failure diagnostics never claim success and identify any retained recovery path if automatic rollback itself fails.

An operating-system or power interruption can occur between the two file renames because the binary and configuration directory may be on different filesystems. In that case the old receipt's binary digest will reject another update. Preserve any `.stack-update-backup-*` file beside the executable and restore it before retrying; do not delete the receipt or bypass its digest check.

## Maintainer activation

Self-update is activated only after all of these are true:

1. The release workflow attests the release manifest and every target archive from the exact tag.
2. A verified direct installer writes a schema-valid receipt for the final installed bytes and path.
3. Local-server integration, tampered material, package-manager refusal, permission failure, atomic replacement, and rollback tests pass.
4. The distribution contract sets `minimumSupportedCliVersion` to the earliest compatible released updater and changes the `self-update` channel to `available`; tagged release context then copies that floor into the manifest and records the channel in `verifiedChannels`.

Changing source code or documenting the command alone does not activate the channel. Published tags and assets remain immutable; a broken release is withdrawn and replaced by a new patch version.
