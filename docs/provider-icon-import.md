# Local provider icon import

## Boundary

`stack icons import` converts a provider's official local ZIP archive into a Stack provider pack. Stack does not download, proxy, mirror, upload, or bundle provider asset bytes. The user obtains the archive from the official source, reviews the linked terms, and confirms that review with `--accept-terms`.

The command rejects a changed archive rather than guessing new paths or terms. A new upstream release requires a reviewed code change that updates the source, complete archive SHA-256, entry allowlist, terms review, visual comparison, and fixtures.

This guide records technical safeguards and provenance; it is not legal advice. Users remain responsible for applying the provider terms to their generated diagrams.

## Usage

```sh
stack icons import aws ~/Downloads/aws-icons.zip \
  --accept-terms \
  -o .stack-icons/aws
```

`PROVIDER` is `aws`, `gcp`, or `azure`. The output directory must not exist. A successful import creates:

```text
<output>/
  manifest.json
  NOTICE.md
  assets/
    <provider icon>.svg
```

The manifest follows the public [`stack-sh/theme` provider-pack schema](https://github.com/stack-sh/theme/blob/main/PROVIDER_PACKS.md). It records the official source, archive and asset hashes, upstream paths, allowed output categories, transformations, official product names, terms URL, review date, and non-endorsement notice.

## Rendering with a local pack

Use the imported directory explicitly when a diagram contains a namespaced provider icon:

```sh
stack render architecture.stack \
  --provider-pack .stack-icons/aws \
  -o architecture.svg \
  --notice architecture.NOTICE.md
```

`--provider-pack` is repeatable for diagrams that use more than one provider. The renderer reads only `manifest.json` and its declared `assets/*.svg` regular files, rejects symbolic links and unsafe relative paths, caps each file at 1 MiB and each validated pack at 32 MiB, and performs no discovery, download, upload, or cache mutation. A provider icon changes only the visual asset; the authored node `kind` remains the source of semantic styling and layout behavior.

`--notice` writes the exact pack revision, official archive hash, source release, terms URL, attribution, non-endorsement text, and used icon IDs for that rendered artifact. A missing pack or icon keeps the existing `STK5001` warning and provider-neutral fallback. Review the imported pack's `NOTICE.md` and linked terms before selecting it, and distribute the generated diagram and sidecar only as those terms permit.

## Audited sources

| Provider | Official source | Audited release | Complete archive SHA-256 | Terms and guidance | Imported IDs |
| --- | --- | --- | --- | --- | --- |
| AWS | [AWS Architecture Icons](https://aws.amazon.com/architecture/icons/) | `Icon-package_07312026` | `d2d166c453526471749d520e0db022c459abef759d2946cf2dd1d1c992dc6526` | [AWS Trademark Guidelines](https://aws.amazon.com/trademark-guidelines/) | `aws:s3`, `aws:sqs`, `aws:lambda`, `aws:ec2`, `aws:rds`, `aws:dynamodb`, `aws:eks` |
| Google Cloud | [Google Cloud Icon Library](https://cloud.google.com/icons) | Core product icons from the May 2026 guide | `6531a10f58bc599c24d9a455d81dd757c1a03c3c43da9cddf639b859c1c1eece` | [Google Brand Resource Center](https://about.google/brand-resource-center/) | `gcp:cloud-run`, `gcp:cloud-storage`, `gcp:compute-engine`, `gcp:gke`, `gcp:bigquery`, `gcp:cloud-sql` |
| Azure | [Azure Architecture Icons](https://learn.microsoft.com/azure/architecture/icons/) | `Azure_Public_Service_Icons_V24` | `921594ccd1bf3d9c0a1bd7b6d924e050551a59342f2b353bb74bdcf761c35141` | `Microsoft_Terms_of_Use.pdf` inside the official archive and the source page | `azure:virtual-machines`, `azure:storage-accounts`, `azure:azure-sql-database`, `azure:aks`, `azure:app-service` |

The Google Cloud category archive is separately audited but is not accepted by this first importer slice. In particular, `gcp:serverless` remains unavailable until the manifest contract can identify every source archive used by one pack.

## Security and artwork preservation

The importer:

- caps the complete archive at 32 MiB and each selected SVG at 1 MiB;
- verifies the complete ZIP before parsing and reads only exact allowlisted entry names;
- rejects directories, symbolic links, traversal paths, malformed XML, document types, entities, processing instructions, nested SVG, scripts, event handlers, foreign namespaces, visible text, external URLs, data URLs, and executable URLs;
- accepts only a small SVG element and attribute allowlist;
- converts the audited Google Cloud fill classes into equivalent presentation attributes;
- removes comments, titles, unused identifiers, and non-rendering generator metadata;
- namespaces referenced Azure gradient IDs before embedding multiple icons in one document;
- preserves view boxes, geometry, colors, gradient stops, and aspect ratios;
- records original and processed hashes plus every visual-preservation transformation;
- creates the complete pack in a temporary sibling directory and renames it into place without overwriting an existing path.

The audited 18 source/processed pairs were rasterized at 512 by 512 pixels and compared with zero changed pixels. Real provider archives and generated packs remain temporary local test inputs and are never committed.
