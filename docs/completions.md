# Shell completion and offline manual

Stack CLI generates bash, zsh, and fish completion source and a roff manual page from the command metadata compiled into the `stack` binary. The generated source is deterministic: CI compares all four tracked files with fresh command output and a release is rejected when the archive, runtime generator, and command definition differ.

```sh
stack completions bash
stack completions zsh
stack completions fish
stack manpage
```

The published release archive stores the exact same bytes at these paths:

| Asset | Archive path |
| --- | --- |
| bash | `share/bash-completion/completions/stack` |
| zsh | `share/zsh/site-functions/_stack` |
| fish | `share/fish/vendor_completions.d/stack.fish` |
| manual | `share/man/man1/stack.1` |

The source tree after Stack CLI 0.3.0 contains these commands and assets. The immutable 0.3.0 archives predate them; do not infer that an older installed binary can generate them.

## Homebrew

The owner-maintained formula installs the archived files through Homebrew's `bash_completion`, `zsh_completion`, `fish_completion`, and `man1` helpers. These helpers link files into the package-manager prefix described by the [Homebrew Formula Cookbook](https://docs.brew.sh/Formula-Cookbook). The shell may still require its normal completion initialization; follow [Homebrew's shell-completion setup](https://docs.brew.sh/Shell-Completion).

No shell startup file is modified by the formula.

## Direct, Aqua, and future Cargo installs

Aqua's registry `files` mapping owns executable placement, not a user's shell startup or global manual database. A direct binary copy and a future Cargo install have the same boundary. Generate files into a user-owned location after installing the binary:

```sh
data_root="${XDG_DATA_HOME:-$HOME/.local/share}"
config_root="${XDG_CONFIG_HOME:-$HOME/.config}"

mkdir -p "$data_root/bash-completion/completions"
stack completions bash > "$data_root/bash-completion/completions/stack"

mkdir -p "$data_root/zsh/site-functions"
stack completions zsh > "$data_root/zsh/site-functions/_stack"

mkdir -p "$config_root/fish/completions"
stack completions fish > "$config_root/fish/completions/stack.fish"

mkdir -p "$data_root/man/man1"
stack manpage > "$data_root/man/man1/stack.1"
```

For zsh, add `$data_root/zsh/site-functions` to `fpath` before running `compinit`. Bash must load the chosen completion directory through its normal completion framework. Fish discovers `$XDG_CONFIG_HOME/fish/completions` automatically. Add `$data_root/man` to `MANPATH` when the host does not already include the user data prefix. These are user shell choices, so `stack` prints bytes only and never edits dotfiles.

You can also read the manual without installation:

```sh
stack manpage > stack.1
man ./stack.1
```

## Maintainer workflow

Build the binary, regenerate the tracked assets, and review the diff:

```sh
cargo build --locked
python3 scripts/generate_cli_assets.py --binary target/debug/stack
python3 scripts/generate_cli_assets.py --binary target/debug/stack --check
```

Release packaging accepts only the four exact tracked paths, regular files, bounded sizes, fixed permissions, and deterministic archive metadata. `scripts/verify_release_binary.py` independently proves that the target binary reproduces every archived file.
