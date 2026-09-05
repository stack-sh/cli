# Provider icons

`stack icons import` downloads the official archives recorded in the CLI catalog, verifies their complete SHA-256 hashes, sanitizes the selected SVGs, and creates provider packs in the user icon store. `--accept-terms` records the user's confirmation that they reviewed the linked provider and brand terms.

## Quick start

Import every provider used by a diagram once:

```sh
$ stack icons import gcp --accept-terms
$ stack icons import simple-icons --accept-terms
$ stack render architecture.stack -o architecture.svg
```

The same render command works for one or several imported providers. Namespaced icon IDs identify the pack, such as `gcp:cloud-run` and `simple-icons:github`.

## Icon store

The default icon store is:

```text
$XDG_CONFIG_HOME/stack/icons
```

When `XDG_CONFIG_HOME` is unset, Stack uses:

```text
$HOME/.config/stack/icons
```

Each import creates one known provider directory below that root:

```text
icons/
  aws/
    manifest.json
    NOTICE.md
    assets/
  gcp/
  azure/
  simple-icons/
```

`stack render` discovers the `aws`, `gcp`, `azure`, and `simple-icons` directories in this store. Each loaded manifest and its declared `assets/*.svg` files are validated before rendering.

Set a different shared icon store in `$XDG_CONFIG_HOME/stack/config.yaml`:

```yaml
default_icons_path: /absolute/path/to/stack-icons
```

The configured path is used by both `stack icons import` and `stack render`.
Use `stack config get default_icons_path` to inspect the effective value and `stack doctor` to validate the store without changing it. These commands were added after the published 0.4.0 release; see the [configuration discovery and doctor contract](./configuration.md) for availability and failure behavior.

## Keep icons with a project

Use `-o` when the provider packs should live in a repository or another project-specific location. The option names the icon-store root, and the importer creates its provider child directory.

```sh
$ stack icons import gcp --accept-terms -o .stack-icons
$ stack icons import simple-icons --accept-terms -o .stack-icons
```

Use the same root with `--provider-pack` while rendering:

```sh
$ stack render architecture.stack \
  --provider-pack .stack-icons \
  -o architecture.svg \
  --notice architecture.NOTICE.md
```

The project-local layout is:

```text
.stack-icons/
  gcp/
  simple-icons/
```

`--provider-pack` takes precedence over `default_icons_path` for that render. `-o` takes precedence over it for that import.

## Finding IDs

Search the versioned, asset-free catalog included in the CLI:

```sh
# Provider counts and audited releases
$ stack icons list

# Every AWS catalog entry
$ stack icons list aws

# Match an ID, product name, or category
$ stack icons list aws s3
$ stack icons list azure database
$ stack icons list simple-icons collaboration
```

The tab-separated output contains stable `ID`, `PRODUCT`, `CATEGORY`, and recommended `KIND` columns. A provider icon supplies the visual asset, while the authored node `kind` controls semantic styling and layout behavior.

## Audited sources

| Provider | Catalog | Official source | Audited release | Complete archive SHA-256 | Terms and guidance |
| --- | ---: | --- | --- | --- | --- |
| AWS | 305 | [AWS Architecture Icons](https://aws.amazon.com/architecture/icons/) | `Icon-package_07312026` | `d2d166c453526471749d520e0db022c459abef759d2946cf2dd1d1c992dc6526` | [AWS Trademark Guidelines](https://aws.amazon.com/trademark-guidelines/) |
| Google Cloud core products | 19 | [Google Cloud Icon Library](https://cloud.google.com/icons) | May 2026 guide | `6531a10f58bc599c24d9a455d81dd757c1a03c3c43da9cddf639b859c1c1eece` | [Google Brand Resource Center](https://about.google/brand-resource-center/) |
| Google Cloud product categories | 26 | [Google Cloud Icon Library](https://cloud.google.com/icons) | May 2026 guide | `e5bc3abd3527dc2500e9bff7f15870783e2c764129c49b7cd4c1b4e105345002` | [Google Brand Resource Center](https://about.google/brand-resource-center/) |
| Azure | 639 | [Azure Architecture Icons](https://learn.microsoft.com/azure/architecture/icons/) | `Azure_Public_Service_Icons_V24` | `921594ccd1bf3d9c0a1bd7b6d924e050551a59342f2b353bb74bdcf761c35141` | `Microsoft_Terms_of_Use.pdf` in the official archive and the source page |
| Curated tools | 62 | [Simple Icons 16.29.0](https://github.com/simple-icons/simple-icons/releases/tag/16.29.0) | `16.29.0` | `99f30fabd5be19dab51e09b2adebb6fe54fce1f3709ddfdc936a1338dfebc68d` | [Simple Icons disclaimer](https://github.com/simple-icons/simple-icons/blob/16.29.0/DISCLAIMER.md), plus per-icon brand links in the catalog and generated notice |

The importer downloads every archive required by the selected provider. Google Cloud therefore imports its core-product and category archives together with one command.

The curated tools set covers common source control, collaboration, design, infrastructure, data, identity, and observability products, including GitHub, GitHub Actions, GitLab, Bitbucket, Notion, Linear, Atlassian, Jira, Confluence, Trello, Discord, Figma, Miro, Docker, Kubernetes, Terraform, OpenTofu, Pulumi, Datadog, Grafana, and Sentry.

## Verification and notices

The CLI catalog pins each official HTTPS archive URL, release, complete archive SHA-256, allowlisted entry path, terms URL, and review date. Import applies a 32 MiB archive limit and a 1 MiB per-SVG limit before writing a new pack atomically.

SVG processing uses a small element and attribute allowlist, preserves artwork geometry and colors, namespaces local resource identifiers, and records original and processed hashes plus visual-preservation transformations in `manifest.json`.

Each pack includes `NOTICE.md`. `stack render --notice <PATH>` writes the exact provider pack revisions, source releases, terms URLs, attribution, non-endorsement text, and used icon IDs for a rendered artifact.

Simple Icons distributes its repository under CC0 and documents separate rights for individual brand marks. The catalog records a source and guideline URL for every curated brand, and generated notices carry those links for use review.
