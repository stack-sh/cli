import { readFile, readdir } from "node:fs/promises"
import path from "node:path"
import { fileURLToPath } from "node:url"

const repositoryRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..")
const expectedCounts = new Map([
  ["aws", 305],
  ["gcp", 45],
  ["azure", 639],
  ["simple-icons", 62],
])
const requiredLegacyIds = [
  "aws:s3",
  "aws:sqs",
  "aws:lambda",
  "aws:ec2",
  "aws:rds",
  "aws:dynamodb",
  "aws:eks",
  "gcp:cloud-run",
  "gcp:cloud-storage",
  "gcp:compute-engine",
  "gcp:gke",
  "gcp:bigquery",
  "gcp:cloud-sql",
  "gcp:serverless",
  "azure:virtual-machines",
  "azure:storage-accounts",
  "azure:azure-sql-database",
  "azure:aks",
  "azure:app-service",
]
const requiredToolIds = [
  "simple-icons:github",
  "simple-icons:githubactions",
  "simple-icons:notion",
  "simple-icons:linear",
  "simple-icons:atlassian",
  "simple-icons:jira",
  "simple-icons:confluence",
  "simple-icons:trello",
  "simple-icons:figma",
  "simple-icons:docker",
  "simple-icons:kubernetes",
  "simple-icons:terraform",
]

function assert(condition, message) {
  if (!condition) throw new Error(message)
}

function validateSource(source, label) {
  assert(typeof source?.pageUrl === "string" && source.pageUrl.startsWith("https://"), `${label}: invalid pageUrl`)
  assert(typeof source?.archiveUrl === "string" && source.archiveUrl.startsWith("https://"), `${label}: invalid archiveUrl`)
  assert(/^sha256:[0-9a-f]{64}$/.test(source?.archiveSha256), `${label}: invalid archiveSha256`)
  assert(typeof source?.release === "string" && source.release.length > 0, `${label}: missing release`)
  assert(typeof source?.termsUrl === "string" && source.termsUrl.startsWith("https://"), `${label}: invalid termsUrl`)
}

const files = (await readdir(path.join(repositoryRoot, "catalogs")))
  .filter((name) => name.endsWith(".json"))
  .sort()
assert(files.length === expectedCounts.size, `expected ${expectedCounts.size} catalogs, found ${files.length}`)

const allIds = new Set()
for (const file of files) {
  const catalog = JSON.parse(await readFile(path.join(repositoryRoot, "catalogs", file), "utf8"))
  const providerId = catalog.provider?.id
  assert(expectedCounts.has(providerId), `${file}: unexpected provider ${providerId}`)
  assert(file === `${providerId}.json`, `${file}: file name does not match provider ID`)
  assert(catalog.catalogVersion === "1.0", `${file}: unsupported catalogVersion`)
  assert(catalog.icons?.length === expectedCounts.get(providerId), `${file}: unexpected icon count`)
  validateSource(catalog.source, `${file}:primary`)

  const sourceIds = new Set(["primary"])
  for (const additional of catalog.additionalSources ?? []) {
    assert(/^[a-z][a-z0-9-]*$/.test(additional.id), `${file}: invalid additional source ID`)
    assert(!sourceIds.has(additional.id), `${file}: duplicate source ID ${additional.id}`)
    sourceIds.add(additional.id)
    validateSource(additional, `${file}:${additional.id}`)
  }

  const rights = catalog.rights
  assert(rights?.termsAcceptanceRequired === true, `${file}: terms acceptance must be required`)
  assert(rights?.processing?.localOnly === true, `${file}: imports must stay local`)
  assert(rights?.processing?.automaticDownload === false, `${file}: automatic download must be disabled`)
  assert(rights?.processing?.serverUpload === false, `${file}: server upload must be disabled`)
  for (const target of ["cargo", "npm", "wasm", "webAsset", "nativeBinary"]) {
    assert(rights?.redistribution?.[target] === false, `${file}: ${target} redistribution must be disabled`)
  }

  const archivePaths = new Set()
  for (const icon of catalog.icons) {
    assert(new RegExp(`^${providerId}:[a-z0-9]+(?:-[a-z0-9]+)*$`).test(icon.id), `${file}: invalid icon ID ${icon.id}`)
    assert(!allIds.has(icon.id), `${file}: duplicate icon ID ${icon.id}`)
    allIds.add(icon.id)
    assert(typeof icon.productName === "string" && icon.productName.length > 0, `${icon.id}: missing productName`)
    assert(typeof icon.category === "string" && icon.category.length > 0, `${icon.id}: missing category`)
    assert(typeof icon.archivePath === "string" && icon.archivePath.endsWith(".svg"), `${icon.id}: invalid archivePath`)
    const pathKey = `${icon.sourceId ?? "primary"}:${icon.archivePath}`
    assert(!archivePaths.has(pathKey), `${icon.id}: duplicate archive path ${pathKey}`)
    archivePaths.add(pathKey)
    assert(sourceIds.has(icon.sourceId ?? "primary"), `${icon.id}: unknown source ID`)
    assert(!JSON.stringify(icon).includes("<svg"), `${icon.id}: asset bytes must not be embedded`)
    if (providerId === "simple-icons") {
      assert(icon.brandSourceUrl?.startsWith("https://"), `${icon.id}: missing brand source`)
      assert(icon.brandGuidelinesUrl?.startsWith("https://"), `${icon.id}: missing brand guidelines`)
    }
  }
}

assert(allIds.size === 1_051, `expected 1051 unique IDs, found ${allIds.size}`)
for (const id of [...requiredLegacyIds, ...requiredToolIds]) {
  assert(allIds.has(id), `required ID is missing: ${id}`)
}

process.stdout.write(`Validated ${files.length} asset-free catalogs with ${allIds.size} unique icon IDs.\n`)
