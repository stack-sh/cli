import { createHash } from "node:crypto"
import { execFileSync } from "node:child_process"
import { mkdir, readFile, writeFile } from "node:fs/promises"
import path from "node:path"
import { fileURLToPath } from "node:url"

const repositoryRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..")
const argumentsByName = new Map()
for (let index = 2; index < process.argv.length; index += 2) {
  const option = process.argv[index]
  const value = process.argv[index + 1]
  if (!option?.startsWith("--") || !value) {
    throw new Error("Expected --aws, --gcp-core, --gcp-category, --azure, and --simple-icons paths")
  }
  argumentsByName.set(option.slice(2), path.resolve(value))
}

for (const name of ["aws", "gcp-core", "gcp-category", "azure", "simple-icons"]) {
  if (!argumentsByName.has(name)) throw new Error(`Missing --${name}`)
}

const archiveHashes = {
  aws: "d2d166c453526471749d520e0db022c459abef759d2946cf2dd1d1c992dc6526",
  "gcp-core": "6531a10f58bc599c24d9a455d81dd757c1a03c3c43da9cddf639b859c1c1eece",
  "gcp-category": "e5bc3abd3527dc2500e9bff7f15870783e2c764129c49b7cd4c1b4e105345002",
  azure: "921594ccd1bf3d9c0a1bd7b6d924e050551a59342f2b353bb74bdcf761c35141",
  "simple-icons": "99f30fabd5be19dab51e09b2adebb6fe54fce1f3709ddfdc936a1338dfebc68d",
}

for (const [name, expected] of Object.entries(archiveHashes)) {
  const bytes = await readFile(argumentsByName.get(name))
  const actual = createHash("sha256").update(bytes).digest("hex")
  if (actual !== expected) throw new Error(`${name} archive hash changed: ${actual}`)
}

function entries(name) {
  return execFileSync("zipinfo", ["-1", argumentsByName.get(name)], {
    encoding: "utf8",
    maxBuffer: 16 * 1024 * 1024,
  })
    .trim()
    .split("\n")
}

function entryBytes(name, entry) {
  return execFileSync("unzip", ["-p", argumentsByName.get(name), entry], {
    maxBuffer: 16 * 1024 * 1024,
  })
}

function slugify(value) {
  return value
    .normalize("NFKD")
    .replace(/&/g, " and ")
    .replace(/\+/g, " plus ")
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, "-")
    .replace(/^-|-$/g, "")
    .replace(/-+/g, "-")
}

function simpleIconSlug(value) {
  return value
    .normalize("NFKD")
    .toLowerCase()
    .replace(/\+/g, "plus")
    .replace(/\./g, "dot")
    .replace(/&/g, "and")
    .replace(/[^a-z0-9]/g, "")
}

function source({
  pageUrl,
  archiveUrl,
  archiveSha256,
  release,
  termsUrl,
  copyright,
  licenseId,
  archiveLicenseIncluded,
}) {
  return {
    pageUrl,
    archiveUrl,
    archiveSha256: `sha256:${archiveSha256}`,
    release,
    retrievedAt: "2026-09-04",
    termsUrl,
    termsReviewedAt: "2026-09-04",
    reviewAfter: "2026-12-03",
    copyright,
    licenseId,
    archiveLicenseIncluded,
  }
}

function profile({ provider, source, additionalSources = [], rights, notice, icons }) {
  return {
    catalogVersion: "1.0",
    packVersion: "0.2.0",
    provider,
    source,
    additionalSources,
    rights,
    notice,
    icons: icons.sort((left, right) => left.id.localeCompare(right.id)),
  }
}

function rights(permittedOutputs, productNameNearby = true) {
  return {
    termsAcceptanceRequired: true,
    permittedOutputs,
    redistribution: {
      cargo: false,
      npm: false,
      wasm: false,
      webAsset: false,
      nativeBinary: false,
      generatedOutput: true,
    },
    processing: {
      localOnly: true,
      automaticDownload: false,
      serverUpload: false,
      preserveColors: true,
      preserveGeometry: true,
      productNameNearby,
    },
    modificationPolicy: "visual-preservation-only",
  }
}

const awsIdOverrides = new Map([
  ["Amazon-Simple-Storage-Service", "s3"],
  ["Amazon-Simple-Queue-Service", "sqs"],
  ["AWS-Lambda", "lambda"],
  ["Amazon-EC2", "ec2"],
  ["Amazon-RDS", "rds"],
  ["Amazon-DynamoDB", "dynamodb"],
  ["Amazon-Elastic-Kubernetes-Service", "eks"],
])
const awsProductOverrides = new Map([
  ["Amazon-Simple-Storage-Service", "Amazon Simple Storage Service (Amazon S3)"],
  ["Amazon-Simple-Queue-Service", "Amazon Simple Queue Service (Amazon SQS)"],
  ["Amazon-EC2", "Amazon Elastic Compute Cloud (Amazon EC2)"],
  ["Amazon-RDS", "Amazon Relational Database Service (Amazon RDS)"],
  ["Amazon-Elastic-Kubernetes-Service", "Amazon Elastic Kubernetes Service (Amazon EKS)"],
])
function awsNodeKind(category, name) {
  if (name === "AWS-Lambda") return "function"
  if (name.includes("Queue") || name.includes("EventBridge")) return "queue"
  if (category === "Databases") {
    if (name.includes("ElastiCache") || name.includes("MemoryDB")) return "cache"
    return "database"
  }
  if (category === "Storage" || category === "Cloud-Financial-Management") return "storage"
  return "service"
}

const awsRows = entries("aws")
  .filter((entry) => /^Architecture-Service-Icons_[^/]+\/Arch_[^/]+\/48\/.*\.svg$/.test(entry))
  .map((archivePath) => {
    const category = archivePath.match(/\/Arch_([^/]+)\/48\//)?.[1] ?? "Other"
    const name = path.basename(archivePath).replace(/^Arch_/, "").replace(/_48\.svg$/, "")
    return { archivePath, category, name, baseId: awsIdOverrides.get(name) ?? slugify(name) }
  })
const awsGroups = Map.groupBy(awsRows, (row) => row.baseId)
const awsIcons = []
for (const [baseId, rows] of awsGroups) {
  const ordered = rows.toSorted((left, right) => left.archivePath.localeCompare(right.archivePath))
  for (const [index, row] of ordered.entries()) {
    const id = index === 0 ? baseId : `${baseId}-${slugify(row.category)}`
    awsIcons.push({
      id: `aws:${id}`,
      subject: `${row.category.replaceAll("-", " ")} product or service`,
      productName: awsProductOverrides.get(row.name) ?? row.name.replaceAll("-", " "),
      recommendedNodeKind: awsNodeKind(row.category, row.name),
      category: row.category.replaceAll("-", " "),
      archivePath: row.archivePath,
    })
  }
}

const gcpCoreIdOverrides = new Map([
  ["GKE", "gke"],
  ["Cloud Storage", "cloud-storage"],
  ["Compute Engine", "compute-engine"],
  ["Cloud Run", "cloud-run"],
  ["Cloud SQL", "cloud-sql"],
])
function gcpNodeKind(name) {
  if (/Storage|Hyperdisk/.test(name)) return "storage"
  if (/BigQuery|Spanner|SQL|AlloyDB|Databases/.test(name)) return "database"
  if (/Serverless/.test(name)) return "function"
  if (/Agents/.test(name)) return "worker"
  return "service"
}
const gcpCoreIcons = entries("gcp-core")
  .filter((entry) => entry.endsWith(".svg"))
  .map((archivePath) => {
    const name = archivePath.split("/")[1]
    return {
      id: `gcp:${gcpCoreIdOverrides.get(name) ?? slugify(name)}`,
      subject: "Google Cloud core product",
      productName: name === "GKE" ? "Google Kubernetes Engine" : name,
      recommendedNodeKind: gcpNodeKind(name),
      category: "Core products",
      archivePath,
    }
  })
const gcpCategoryIcons = entries("gcp-category")
  .filter((entry) => entry.endsWith(".svg"))
  .map((archivePath) => {
    const rawName = archivePath.split("/")[1]
    const name = rawName.replace(" _ ", " & ")
    const id = name === "Serverless Computing" ? "serverless" : slugify(name)
    return {
      id: `gcp:${id}`,
      subject: "Google Cloud product category",
      productName: `${name} category`,
      recommendedNodeKind: gcpNodeKind(name),
      category: "Product categories",
      sourceId: "categories",
      archivePath,
    }
  })

const azureIdOverrides = new Map([
  ["Virtual-Machine", "virtual-machines"],
  ["Storage-Accounts", "storage-accounts"],
  ["SQL-Database", "azure-sql-database"],
  ["Kubernetes-Services", "aks"],
  ["App-Services", "app-service"],
])
const azureProductOverrides = new Map([
  ["Virtual-Machine", "Azure Virtual Machines"],
  ["Storage-Accounts", "Azure Storage Accounts"],
  ["SQL-Database", "Azure SQL Database"],
  ["Kubernetes-Services", "Azure Kubernetes Service (AKS)"],
  ["App-Services", "Azure App Service"],
])
const azurePreferredPaths = new Map([
  ["aks", "Azure_Public_Service_Icons/Icons/containers/10023-icon-service-Kubernetes-Services.svg"],
  ["app-service", "Azure_Public_Service_Icons/Icons/app services/10035-icon-service-App-Services.svg"],
])
function azureNodeKind(category, name) {
  if (category === "databases") return /Redis|Cache/.test(name) ? "cache" : "database"
  if (category === "storage") return "storage"
  if (/Function-Apps/.test(name)) return "function"
  if (/Queue|Service-Bus|Event-Hub/.test(name)) return "queue"
  return "service"
}
const azureRows = entries("azure")
  .filter((entry) => entry.endsWith(".svg"))
  .map((archivePath) => {
    const fileName = path.basename(archivePath)
    const numericId = fileName.match(/^(\d+)-icon-service-/)?.[1] ?? "unknown"
    const name = fileName.replace(/^\d+-icon-service-/, "").replace(/\.svg$/, "")
    const category = archivePath.split("/").at(-2) ?? "other"
    return {
      archivePath,
      numericId,
      name,
      category,
      baseId: azureIdOverrides.get(name) ?? slugify(name),
      digest: createHash("sha256").update(entryBytes("azure", archivePath)).digest("hex"),
    }
  })
const azureGroups = Map.groupBy(azureRows, (row) => row.baseId)
const azureIcons = []
for (const [baseId, rows] of azureGroups) {
  const preferredPath = azurePreferredPaths.get(baseId)
  const ordered = rows.toSorted((left, right) => {
    if (left.archivePath === preferredPath) return -1
    if (right.archivePath === preferredPath) return 1
    return left.archivePath.localeCompare(right.archivePath)
  })
  const uniqueRows = []
  const seenDigests = new Set()
  for (const row of ordered) {
    if (!seenDigests.has(row.digest)) uniqueRows.push(row)
    seenDigests.add(row.digest)
  }
  for (const [index, row] of uniqueRows.entries()) {
    const id = index === 0 ? baseId : `${baseId}-${row.numericId}`
    azureIcons.push({
      id: `azure:${id}`,
      subject: `${row.category} product or service`,
      productName: azureProductOverrides.get(row.name) ?? row.name.replaceAll("-", " "),
      recommendedNodeKind: azureNodeKind(row.category, row.name),
      category: row.category,
      archivePath: row.archivePath,
    })
  }
}

const selectedSimpleIcons = [
  "1password",
  "ansible",
  "apacheairflow",
  "apachekafka",
  "argo",
  "atlassian",
  "auth0",
  "bitbucket",
  "bun",
  "circleci",
  "cloudflare",
  "confluence",
  "datadog",
  "deno",
  "discord",
  "docker",
  "elastic",
  "figma",
  "firebase",
  "git",
  "github",
  "githubactions",
  "gitlab",
  "go",
  "grafana",
  "helm",
  "jenkins",
  "jira",
  "kubernetes",
  "linear",
  "miro",
  "mongodb",
  "mysql",
  "netlify",
  "newrelic",
  "nextdotjs",
  "nginx",
  "nodedotjs",
  "notion",
  "npm",
  "okta",
  "opentofu",
  "pagerduty",
  "pnpm",
  "postgresql",
  "prometheus",
  "pulumi",
  "python",
  "rabbitmq",
  "react",
  "redis",
  "rust",
  "sentry",
  "snowflake",
  "splunk",
  "supabase",
  "svelte",
  "terraform",
  "trello",
  "vercel",
  "vite",
  "vuedotjs",
]
const simpleEntries = entries("simple-icons")
const simpleRoot = simpleEntries[0]
const simpleMetadata = JSON.parse(
  entryBytes("simple-icons", `${simpleRoot}data/simple-icons.json`).toString("utf8"),
)
const simpleMetadataBySlug = new Map(simpleMetadata.map((icon) => [simpleIconSlug(icon.title), icon]))
const simpleCategories = new Map([
  ...["1password", "atlassian", "bitbucket", "confluence", "discord", "figma", "github", "githubactions", "gitlab", "jira", "linear", "miro", "notion", "trello"].map((id) => [id, "Collaboration"]),
  ...["ansible", "argo", "circleci", "cloudflare", "docker", "helm", "jenkins", "kubernetes", "netlify", "nginx", "opentofu", "pulumi", "terraform", "vercel"].map((id) => [id, "Infrastructure"]),
  ...["datadog", "grafana", "newrelic", "pagerduty", "prometheus", "sentry", "splunk"].map((id) => [id, "Observability"]),
  ...["apacheairflow", "apachekafka", "elastic", "firebase", "mongodb", "mysql", "postgresql", "rabbitmq", "redis", "snowflake", "supabase"].map((id) => [id, "Data"]),
  ...["auth0", "okta"].map((id) => [id, "Identity"]),
  ...["bun", "deno", "git", "go", "nextdotjs", "nodedotjs", "npm", "pnpm", "python", "react", "rust", "svelte", "vite", "vuedotjs"].map((id) => [id, "Development"]),
])
function simpleNodeKind(slug, category) {
  if (["mongodb", "mysql", "postgresql", "snowflake", "supabase", "firebase"].includes(slug)) {
    return "database"
  }
  if (slug === "redis") return "cache"
  if (["apachekafka", "rabbitmq"].includes(slug)) return "queue"
  if (category === "Collaboration" || category === "Identity") return "external"
  return "service"
}
const simpleIcons = selectedSimpleIcons.map((slug) => {
  const metadata = simpleMetadataBySlug.get(slug)
  const archivePath = `${simpleRoot}icons/${slug}.svg`
  if (!metadata || !simpleEntries.includes(archivePath)) {
    throw new Error(`Simple Icons metadata or asset missing for ${slug}`)
  }
  const category = simpleCategories.get(slug)
  if (!category) throw new Error(`Simple Icons category missing for ${slug}`)
  return {
    id: `simple-icons:${slug}`,
    subject: `${category} tool or service`,
    productName: metadata.title,
    brandSourceUrl: metadata.source,
    brandGuidelinesUrl: metadata.guidelines ?? metadata.source,
    recommendedNodeKind: simpleNodeKind(slug, category),
    category,
    archivePath,
  }
})

const profiles = {
  aws: profile({
    provider: { id: "aws", name: "Amazon Web Services" },
    source: source({
      pageUrl: "https://aws.amazon.com/architecture/icons/",
      archiveUrl: "https://d1.awsstatic.com/onedam/marketing-channels/website/public/shared/architecture-icon-release/Icon-package_07312026.5846e92413caa21490223536cc97f1269e44fa92.zip",
      archiveSha256: archiveHashes.aws,
      release: "Icon-package_07312026",
      termsUrl: "https://aws.amazon.com/trademark-guidelines/",
      copyright: "Copyright Amazon Web Services, Inc. or its affiliates",
      licenseId: "LicenseRef-AWS-Architecture-Icons-Terms",
      archiveLicenseIncluded: false,
    }),
    rights: rights(["architecture-diagram", "whitepaper", "presentation", "data-sheet", "poster"]),
    notice: {
      attribution: "AWS architecture icons are owned by Amazon Web Services, Inc. or its affiliates.",
      termsSummary: "Use is limited to the architecture-diagram materials described by the official AWS Architecture Icons page and applicable AWS trademark guidelines.",
      nonEndorsement: "Amazon Web Services does not sponsor or endorse this diagram or Stack.",
    },
    icons: awsIcons,
  }),
  gcp: profile({
    provider: { id: "gcp", name: "Google Cloud" },
    source: source({
      pageUrl: "https://cloud.google.com/icons",
      archiveUrl: "https://services.google.com/fh/files/misc/core-products-icons.zip",
      archiveSha256: archiveHashes["gcp-core"],
      release: "Core product icons (May 2026 guide)",
      termsUrl: "https://about.google/brand-resource-center/",
      copyright: "Copyright Google LLC",
      licenseId: "LicenseRef-Google-Cloud-Product-Icons-Terms",
      archiveLicenseIncluded: false,
    }),
    additionalSources: [
      {
        id: "categories",
        ...source({
          pageUrl: "https://cloud.google.com/icons",
          archiveUrl: "https://services.google.com/fh/files/misc/category-icons.zip",
          archiveSha256: archiveHashes["gcp-category"],
          release: "Product category icons (May 2026 guide)",
          termsUrl: "https://about.google/brand-resource-center/",
          copyright: "Copyright Google LLC",
          licenseId: "LicenseRef-Google-Cloud-Product-Icons-Terms",
          archiveLicenseIncluded: false,
        }),
      },
    ],
    rights: rights(["architecture-diagram", "documentation"]),
    notice: {
      attribution: "Google Cloud product icons are owned by Google LLC.",
      termsSummary: "Use is limited to diagrams and technical documentation described by the official Google Cloud Icon Library and applicable Google brand terms.",
      nonEndorsement: "Google does not sponsor or endorse this diagram or Stack.",
    },
    icons: [...gcpCoreIcons, ...gcpCategoryIcons],
  }),
  azure: profile({
    provider: { id: "azure", name: "Microsoft Azure" },
    source: source({
      pageUrl: "https://learn.microsoft.com/azure/architecture/icons/",
      archiveUrl: "https://arch-center.azureedge.net/icons/Azure_Public_Service_Icons_V24.zip",
      archiveSha256: archiveHashes.azure,
      release: "Azure_Public_Service_Icons_V24",
      termsUrl: "https://learn.microsoft.com/azure/architecture/icons/",
      copyright: "Copyright Microsoft Corporation",
      licenseId: "LicenseRef-Microsoft-Azure-Architecture-Icons-Terms",
      archiveLicenseIncluded: true,
    }),
    rights: rights(["architecture-diagram", "training-material", "documentation"]),
    notice: {
      attribution: "Azure architecture icons are owned by Microsoft Corporation.",
      termsSummary: "Use is limited to architecture diagrams, training materials, and documentation under the terms included in the official Azure icon archive.",
      nonEndorsement: "Microsoft does not sponsor or endorse this diagram or Stack.",
    },
    icons: azureIcons,
  }),
  "simple-icons": profile({
    provider: { id: "simple-icons", name: "Simple Icons (curated tools)" },
    source: source({
      pageUrl: "https://simpleicons.org/",
      archiveUrl: "https://github.com/simple-icons/simple-icons/archive/refs/tags/16.29.0.zip",
      archiveSha256: archiveHashes["simple-icons"],
      release: "16.29.0",
      termsUrl: "https://github.com/simple-icons/simple-icons/blob/16.29.0/DISCLAIMER.md",
      copyright: "Simple Icons contributors and the respective trademark owners",
      licenseId: "LicenseRef-Simple-Icons-CC0-and-Brand-Rights",
      archiveLicenseIncluded: true,
    }),
    rights: rights(["architecture-diagram", "documentation", "presentation"]),
    notice: {
      attribution: "Glyphs are sourced from Simple Icons 16.29.0. Product names and marks belong to their respective owners.",
      termsSummary: "Simple Icons is CC0, but that does not imply that every included brand icon is CC0. Review each icon's recorded brand source and guidelines before use.",
      nonEndorsement: "Simple Icons and the named brands do not sponsor or endorse this diagram or Stack.",
    },
    icons: simpleIcons,
  }),
}

await mkdir(path.join(repositoryRoot, "catalogs"), { recursive: true })
for (const [name, value] of Object.entries(profiles)) {
  await writeFile(path.join(repositoryRoot, "catalogs", `${name}.json`), `${JSON.stringify(value, null, 2)}\n`)
  console.log(`generated ${name}: ${value.icons.length} icons`)
}
