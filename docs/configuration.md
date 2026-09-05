# Configuration discovery and doctor

`stack config` exposes the paths already used by rendering and provider import, while `stack doctor` diagnoses that configuration and its known-provider packs. Every command on this page is read-only: it does not create a configuration directory, config file, icon store, provider pack, receipt, or temporary file.

These commands were added to the source tree after Stack CLI 0.4.0. The published 0.4.0 GitHub, Homebrew, and Aqua binaries do not contain them. They will become available through those channels in a later release.

## Discovery order

Stack selects one configuration file path:

1. An absolute, non-empty `XDG_CONFIG_HOME` selects `$XDG_CONFIG_HOME/stack/config.yaml`.
2. Otherwise, an absolute, non-empty `HOME` selects `$HOME/.config/stack/config.yaml`.
3. If neither value supplies an absolute path, discovery fails with exit status `2`.

A relative `XDG_CONFIG_HOME` is ignored rather than resolved against the current directory. `stack config path` prints the selected path even when the file does not exist, and never reads or creates it:

```sh
stack config path
```

The config file is optional. A missing or whitespace-only file uses the default icon store beside it at `stack/icons`. A non-empty file accepts exactly one optional key:

```yaml
default_icons_path: /absolute/path/to/stack-icons
```

Unknown keys, malformed YAML, relative `default_icons_path` values, symlinks, non-regular files, unreadable files, and files larger than 64 KiB fail closed. `stack config get` applies those checks and prints the one effective value:

```sh
stack config get default_icons_path
```

Command output is a single path followed by a newline. It is intended for inspection and shell composition, not as a stable structured-data protocol.

## Doctor report

Run the default diagnosis with:

```sh
stack doctor
```

The report includes:

- the executable's package version;
- the selected config path and whether `XDG_CONFIG_HOME` or the `HOME` fallback selected it;
- whether the config file is missing, loaded, or invalid;
- the effective icon-store path and whether it came from the default or `default_icons_path`;
- the number of valid installed `aws`, `gcp`, `azure`, and `simple-icons` packs.

Use an explicit project-local icon-store root without changing config resolution:

```sh
stack doctor --provider-pack .stack-icons
```

An absent implicit default store is healthy and means that zero packs are installed. An absent configured store is a warning because the user selected it explicitly. An absent `--provider-pack` directory is an error. Existing stores must be real readable directories, and each known-provider directory must contain a bounded, valid manifest and all of its declared regular SVG assets.

| Result | Exit status |
| --- | ---: |
| Healthy, including a missing implicit store | `0` |
| Warning only, including a missing configured store | `0` |
| Invalid or unreadable config, unresolved config path, or invalid/unreadable/absent explicit provider store | `2` |
| Invalid command arguments | `2` |

## Information boundary

Doctor prints paths because path discovery is the behavior being diagnosed. It does not print config file contents, provider manifest contents, unrelated environment variables, tokens, credentials, or raw parser errors. Invalid and permission-denied inputs are reduced to stable categories with a corrective action. If a path itself contains sensitive text, do not publish or paste the report; paths are user-controlled values and are intentionally visible.

For provider source, rights, import, and notice behavior, see the [provider icon guide](./provider-icon-import.md).
