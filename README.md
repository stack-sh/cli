# Stack CLI

`stack-sh/cli` is the open-source native Rust `stack` command for Stack architecture diagrams.

The repository contains native validation, formatting, and rendering commands. The interface remains pre-release and no supported binary distribution is published yet. The planned target matrix, artifact names, verification material, channel ownership, and rollback rules are defined by the [distribution contract](./docs/distribution.md).

## Commands

```text
stack help
stack help render
stack version
stack init
stack init --template groups-and-layout
stack init --template aws-serverless-checkout -o checkout.stack
stack check arch.stack
stack fmt arch.stack
stack fmt --check arch.stack
stack fmt -
stack render arch.stack
stack render arch.stack -o arch.svg
stack icons list
stack icons list aws s3
stack icons import gcp --accept-terms
stack icons import simple-icons --accept-terms
stack render arch.stack -o arch.svg --notice arch.NOTICE.md
```

`stack help`, `stack -h`, and `stack --help` print top-level help. Use `stack help <COMMAND>` or `<COMMAND> -h` / `<COMMAND> --help` for command-specific usage and examples; nested icon help is available through `stack help icons <COMMAND>`. `stack version`, `stack -v`, `stack -V`, and `stack --version` print the same Cargo package version. Help and version output use standard output and exit with status `0`. Invalid arguments and unknown commands use standard error and status `2`; close command typos include a suggested command and the relevant help invocation.

`stack init` creates `diagram.stack` from the versioned `hello-stack` template without prompting. Use `--template <ID>` to select any of the nine curated examples shared with the public Stack specification and Web gallery, and `-o` / `--output` to choose another file. Existing paths are never replaced unless `--force` is explicit; forced writes use the same atomic output behavior as rendering. Provider templates print the exact `stack icons import` commands needed for branded rendering and remain valid with deterministic fallback icons when packs are absent. The embedded catalog and source bytes are pinned by `tests/specification-revision`, and CI rejects drift from that public specification commit.

`stack check` reads the file as bytes and runs the full compiler, theme, layout, and routing validation pipeline without changing the source. Diagnostics are written to standard error in source order. Standard output remains empty.

`stack fmt` uses the engine formatter and preserves comments. File mode replaces changed source atomically through a temporary file in the same directory; unchanged files are not replaced. Syntax, encoding, and host I/O failures leave the original file untouched. `stack fmt -` reads bytes from standard input and writes only canonical source to standard output. `--check` never writes source and exits with status `1` when formatting is required.

`stack render` uses the same engine pipeline to produce deterministic standalone SVG. Without `-o`, standard output contains only SVG. With `-o`, the output is written atomically in the destination directory. It discovers imported `aws`, `gcp`, `azure`, and `simple-icons` packs below the shared icon store. Use `--provider-pack <DIRECTORY>` for a project-local icon-store root, and use `--notice <NOTICE>` to save the exact provider pack revisions, terms, source archives, and icon IDs embedded in that artifact. Pack files are bounded and validated before rendering. Diagnostics remain on standard error, warnings preserve SVG, and Stack errors never create or replace output.

`stack icons list [PROVIDER] [QUERY]` searches the asset-free catalog by ID, product name, or category. The catalog currently contains 1,051 IDs: 305 AWS, 45 Google Cloud, 639 Azure, and 62 curated developer and collaboration tool icons. This command reads only metadata embedded in the CLI.

`stack icons import <PROVIDER> --accept-terms` downloads the audited official archive set, verifies every complete SHA-256 before ZIP processing, reads allowlisted SVG entries with fixed size limits, sanitizes active and external content, preserves official colors and geometry, and writes the manifest, notice, and processed SVGs atomically. The default store is `$XDG_CONFIG_HOME/stack/icons`, falling back to `$HOME/.config/stack/icons`. `$XDG_CONFIG_HOME/stack/config.yaml` can set an absolute `default_icons_path`. Use `-o <DIRECTORY>` to put provider child directories below a project-local root. See [the provider icon guide](./docs/provider-icon-import.md) for configuration, project-local usage, sources, hashes, and rights.

| Result | Exit status |
| --- | ---: |
| No error diagnostics, including warning-only input | `0` |
| One or more Stack error diagnostics, or `fmt --check` finds a difference | `1` |
| Invalid arguments, host I/O failure, or engine operational failure | `2` |

The CLI links `stack-engine` as a native Rust dependency. It owns filesystem and standard-stream behavior, process exit codes, configuration discovery, provider-pack import, notice output, and command presentation. It must not duplicate compiler, formatter, layout, or SVG-rendering logic.

The bundled engine resolves 30 provider-neutral core icons: `api`, `web`, `mobile`, `desktop`, `server`, `container`, `cluster`, `cloud`, `scheduler`, `webhook`, `identity`, `observability`, `gateway`, `load-balancer`, `dns`, `cdn`, `firewall`, `network`, `event`, `stream`, `search`, `analytics`, `repository`, `pipeline`, `secret`, `document`, `task`, `chat`, `email`, and `ai`. User-managed provider packs preserve upstream artwork and attach source, archive hash, transformation, terms, and notice metadata. Rendering resolves namespaced IDs such as `aws:s3`, preserves the authored semantic `kind`, embeds the selected local asset, and writes its provenance into SVG metadata and the optional notice sidecar.

## Development

The CLI requires Rust 1.85 or newer.

```sh
cargo run -- check arch.stack
cargo test --locked
cargo clippy --all-targets --all-features --locked -- -D warnings
```

CI validates formatting, unit and process-level integration tests, at least 90% line/region coverage and 95% function coverage, Clippy, documentation, a release build, `--help`, and `--version` on stable Rust. Tests and Clippy also run on Rust 1.85.

Canonical formatter behavior is checked against the pinned `stack-sh/specification` fixture revision recorded in `tests/specification-revision`.

The same checkout validates and updates the embedded `stack init` templates:

```sh
STACK_SPECIFICATION_DIR=../specification \
  node scripts/sync-example-templates.mjs --check
STACK_SPECIFICATION_DIR=../specification \
  node scripts/sync-example-templates.mjs
STACK_SPECIFICATION_DIR=../specification \
  cargo test --features conformance --test template-conformance --locked
```

See [CONTRIBUTING.md](./CONTRIBUTING.md) before opening a change. Please report security vulnerabilities through the process in [SECURITY.md](./SECURITY.md), not a public issue.

## Licensing

Repository-authored work is licensed under the [Apache License 2.0](./LICENSE) for personal and commercial use. Runtime and build dependency licenses are recorded in [THIRD_PARTY_LICENSES.md](./THIRD_PARTY_LICENSES.md). A future binary release must ship the applicable license and notice files described there.
