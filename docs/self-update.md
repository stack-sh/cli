# Upgrading Stack

Stack 0.5.0 removes `stack update` and its self-replacement implementation. Update with the tool that installed Stack; mixing package-manager installs with direct binary replacement can invalidate ownership, checksums, and version pins.

## Homebrew

Run `brew update`, then `brew upgrade stack-sh/tap/stack`. Homebrew also updates its managed completion and manual files.

## Aqua

In the project containing your Aqua configuration, run `aqua update`, review the Stack version change, then run `aqua update-checksum` and `aqua install`. Commit the reviewed configuration and checksum lock. Follow the [owner registry guide](../aqua/README.md) to refresh an immutable registry revision when needed.

## Cargo

Cargo owns updates for the published `stack-diagram-cli` package. Run `cargo install stack-diagram-cli --locked` for the latest release, or add `--version 0.5.4` to select that exact version. The binary remains `stack`; the unrelated `stack-cli` crate is not this project. See the [Cargo installation guide](./distribution.md#cargo-installation).

## Direct GitHub download

Download the desired version for your supported target. Follow the [supply-chain verification guide](./supply-chain.md), then the manual installation procedure in the [distribution guide](./distribution.md). Replace only a binary you installed manually, and refresh completion/manual files from that same release. Run `stack --version`, `stack check`, and `stack render` to verify your installation. Keep the previous verified archive for manual rollback; published assets are never rewritten.

## Migration from 0.4.0

Remove any `stack update` invocation from automation and use the installation owner above. In 0.5.0 it is an unknown command with exit status 2 and performs no update. The 0.4.0 command existed but its release never activated the self-update channel or provided a verified receipt-writing installer. Stack 0.5.0 neither reads nor deletes old installation receipts or backup files. The [historical 0.4.0 contract](https://github.com/stack-sh/cli/blob/v0.4.0/docs/self-update.md) and its immutable schemas remain available; no future self-update channel is planned.
