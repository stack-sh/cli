# Local provider icon import

## Boundary

`stack icons import` converts official local ZIP archives into a Stack provider pack. Stack does not download, proxy, mirror, upload, or bundle provider asset bytes. The user obtains every archive from the recorded source, reviews the linked provider and brand terms, and confirms that review with `--accept-terms`.

The command rejects a changed archive rather than guessing new paths or terms. A new upstream release requires a reviewed code change that updates the source, complete archive SHA-256, entry allowlist, terms review, visual comparison, and fixtures.

This guide records technical safeguards and provenance; it is not legal advice. Users remain responsible for applying the provider terms to their generated diagrams.

## Download the audited archives

These commands download the exact official archives audited by the current CLI catalog. `curl` only retrieves the files; `stack icons import` independently verifies the complete SHA-256 before processing them. If a provider publishes a newer archive, use the official source page to review it, but do not substitute it here until the Stack catalog has been updated.

### AWS

Source: [AWS Architecture Icons](https://aws.amazon.com/architecture/icons/)

```sh
$ curl -fL "https://d1.awsstatic.com/onedam/marketing-channels/website/public/shared/architecture-icon-release/Icon-package_07312026.5846e92413caa21490223536cc97f1269e44fa92.zip" -o aws-icons.zip
$ stack icons import aws ./aws-icons.zip --accept-terms -o .stack-icons/aws
```

### Google Cloud

Source: [Google Cloud Icon Library](https://cloud.google.com/icons). Google publishes the required core-product and category icons separately.

```sh
$ curl -fL "https://services.google.com/fh/files/misc/core-products-icons.zip" -o gcp-core-products-icons.zip
$ curl -fL "https://services.google.com/fh/files/misc/category-icons.zip" -o gcp-category-icons.zip
$ stack icons import gcp ./gcp-core-products-icons.zip \
  --source categories=./gcp-category-icons.zip \
  --accept-terms -o .stack-icons/gcp
```

### Azure

Source: [Azure Architecture Icons](https://learn.microsoft.com/azure/architecture/icons/)

```sh
$ curl -fL "https://arch-center.azureedge.net/icons/Azure_Public_Service_Icons_V24.zip" -o azure-icons.zip
$ stack icons import azure ./azure-icons.zip --accept-terms -o .stack-icons/azure
```

### Simple Icons

Source: [Simple Icons 16.29.0](https://github.com/simple-icons/simple-icons/releases/tag/16.29.0). This curated pack includes GitHub and other common developer and collaboration tools.

```sh
$ curl -fL "https://github.com/simple-icons/simple-icons/archive/refs/tags/16.29.0.zip" -o simple-icons-16.29.0.zip
$ stack icons import simple-icons ./simple-icons-16.29.0.zip --accept-terms -o .stack-icons/simple-icons
```

## Usage

```sh
stack icons import aws ~/Downloads/aws-icons.zip \
  --accept-terms \
  -o .stack-icons/aws
```

`PROVIDER` is `aws`, `gcp`, `azure`, or `simple-icons`. The output directory must not exist. A successful import creates:

```text
<output>/
  manifest.json
  NOTICE.md
  assets/
    <provider icon>.svg
```

The manifest follows the public [`stack-sh/theme` provider-pack schema](https://github.com/stack-sh/theme/blob/main/PROVIDER_PACKS.md). It records the official source, archive and asset hashes, upstream paths, allowed output categories, transformations, official product names, terms URL, review date, and non-endorsement notice.

Google publishes the audited core-product and category icons as separate archives. The positional `ARCHIVE` is the primary core-products ZIP. `--source categories=<ARCHIVE>` maps the second local ZIP to the required `categories` source ID; it is not a URL and does not download anything.

```sh
stack icons import gcp /path/to/core-products-icons.zip \
  --source categories=/path/to/category-icons.zip \
  --accept-terms \
  -o .stack-icons/gcp
```

The additional source ID and both archive hashes are preserved in `manifest.json` and `NOTICE.md`.

## Finding IDs

Do not maintain a copied list of more than one thousand IDs in documentation. Search the versioned, asset-free catalog that ships with the CLI:

```sh
# Provider counts and audited releases
stack icons list

# Every AWS catalog entry
stack icons list aws

# Match ID, product name, or category
stack icons list aws s3
stack icons list azure database
stack icons list simple-icons collaboration
```

The tab-separated output has stable `ID`, `PRODUCT`, `CATEGORY`, and recommended `KIND` columns, so it can also be filtered or imported into another tool. Existing documented IDs remain stable when catalogs grow.

## Rendering with local packs

Use every imported directory explicitly when a diagram contains namespaced provider icons. For example, a diagram that uses `gcp:cloud-run` and `simple-icons:github` needs both packs:

```sh
$ stack render architecture.stack \
  --provider-pack .stack-icons/gcp \
  --provider-pack .stack-icons/simple-icons \
  -o architecture.svg \
  --notice architecture.NOTICE.md
```

`--provider-pack` is repeatable for diagrams that use more than one provider. The renderer reads only `manifest.json` and its declared `assets/*.svg` regular files, rejects symbolic links and unsafe relative paths, caps each file at 1 MiB and each validated pack at 32 MiB, and performs no discovery, download, upload, or cache mutation. A provider icon changes only the visual asset; the authored node `kind` remains the source of semantic styling and layout behavior.

`--notice` writes the exact pack revision, official archive hash, source release, terms URL, attribution, non-endorsement text, and used icon IDs for that rendered artifact. A missing pack or icon keeps the existing `STK5001` warning and provider-neutral fallback. Review the imported pack's `NOTICE.md` and linked terms before selecting it, and distribute the generated diagram and sidecar only as those terms permit.

## Audited sources

| Provider | Catalog | Official archive | Audited release | Complete archive SHA-256 | Terms and guidance |
| --- | ---: | --- | --- | --- | --- |
| AWS | 305 | [AWS Architecture Icons](https://aws.amazon.com/architecture/icons/) | `Icon-package_07312026` | `d2d166c453526471749d520e0db022c459abef759d2946cf2dd1d1c992dc6526` | [AWS Trademark Guidelines](https://aws.amazon.com/trademark-guidelines/) |
| Google Cloud core products | 19 | [Google Cloud Icon Library](https://cloud.google.com/icons) | May 2026 guide | `6531a10f58bc599c24d9a455d81dd757c1a03c3c43da9cddf639b859c1c1eece` | [Google Brand Resource Center](https://about.google/brand-resource-center/) |
| Google Cloud product categories | 26 | [Google Cloud Icon Library](https://cloud.google.com/icons) | May 2026 guide | `e5bc3abd3527dc2500e9bff7f15870783e2c764129c49b7cd4c1b4e105345002` | [Google Brand Resource Center](https://about.google/brand-resource-center/) |
| Azure | 639 | [Azure Architecture Icons](https://learn.microsoft.com/azure/architecture/icons/) | `Azure_Public_Service_Icons_V24` | `921594ccd1bf3d9c0a1bd7b6d924e050551a59342f2b353bb74bdcf761c35141` | `Microsoft_Terms_of_Use.pdf` inside the official archive and the source page |
| Curated tools | 62 | [Simple Icons 16.29.0](https://github.com/simple-icons/simple-icons/releases/tag/16.29.0) | `16.29.0` | `99f30fabd5be19dab51e09b2adebb6fe54fce1f3709ddfdc936a1338dfebc68d` | [Simple Icons disclaimer](https://github.com/simple-icons/simple-icons/blob/16.29.0/DISCLAIMER.md), plus the per-icon brand links in the catalog and generated notice |

The AWS and Azure catalogs cover every canonical service SVG entry selected from the audited official archives. Byte-identical Azure aliases are deduplicated, while visually distinct same-name entries keep distinct stable IDs. Google Cloud includes all 19 core-product and all 26 category SVGs in the two audited archives. The curated tools set covers common source control, collaboration, design, infrastructure, data, identity, and observability products, including GitHub, GitHub Actions, GitLab, Bitbucket, Notion, Linear, Atlassian, Jira, Confluence, Trello, Discord, Figma, Miro, Docker, Kubernetes, Terraform, OpenTofu, Pulumi, Datadog, Grafana, and Sentry.

Simple Icons makes its repository available under CC0, but its disclaimer explicitly separates that distribution license from rights in individual brand marks. The Stack catalog therefore records a source and guideline URL on every curated icon, and generated notices preserve those links. Inclusion is discoverability metadata, not an assertion that every use is permitted or endorsed.

## Security and artwork preservation

The importer:

- caps the complete archive at 32 MiB and each selected SVG at 1 MiB;
- verifies every complete ZIP before parsing and reads only exact allowlisted entry names;
- rejects directories, symbolic links, traversal paths, malformed XML, document types, entities, processing instructions, nested SVG, scripts, event handlers, foreign namespaces, visible text, external URLs, data URLs, and executable URLs;
- accepts only a small SVG element and attribute allowlist;
- converts the audited Google Cloud fill classes into equivalent presentation attributes;
- removes comments, titles, unused identifiers, and non-rendering generator metadata;
- namespaces referenced gradient IDs before embedding multiple icons in one document;
- preserves view boxes, geometry, colors, gradient stops, and aspect ratios;
- records original and processed hashes plus every visual-preservation transformation;
- creates the complete pack in a temporary sibling directory and renames it into place without overwriting an existing path.

Catalog metadata is validated for unique stable IDs and archive paths, exact source references, hashes, local-only processing, disabled redistribution targets, legacy ID compatibility, and required per-brand guidance. Real provider archives, source SVGs, and generated packs remain temporary local test inputs and are never committed.
