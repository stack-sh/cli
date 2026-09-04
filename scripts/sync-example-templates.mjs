import assert from "node:assert/strict"
import { execFile } from "node:child_process"
import { mkdir, readFile, readdir, rm, writeFile } from "node:fs/promises"
import path from "node:path"
import { promisify } from "node:util"

const execute = promisify(execFile)
const specificationRootValue = process.env.STACK_SPECIFICATION_DIR
if (!specificationRootValue) {
  throw new Error("STACK_SPECIFICATION_DIR must point to the pinned specification checkout")
}

const checkOnly = process.argv.includes("--check")
const specificationRoot = path.resolve(specificationRootValue)
const expectedRevision = (await readFile("tests/specification-revision", "utf8")).trim()
assert.match(expectedRevision, /^[0-9a-f]{40}$/)
const { stdout } = await execute("git", ["rev-parse", "HEAD"], {
  cwd: specificationRoot,
  encoding: "utf8",
})
assert.equal(stdout.trim(), expectedRevision, "Specification checkout does not match the pin")

const catalogSource = await readFile(path.join(specificationRoot, "examples/catalog.json"), "utf8")
const catalog = JSON.parse(catalogSource)
const sourceNames = catalog.examples.map((example) => example.source).sort()
const templateRoot = path.resolve("templates")
const sourceRoot = path.join(templateRoot, "sources")
const snapshots = new Map([[path.join(templateRoot, "catalog.json"), catalogSource]])
for (const sourceName of sourceNames) {
  snapshots.set(
    path.join(sourceRoot, sourceName),
    await readFile(path.join(specificationRoot, "examples", sourceName), "utf8"),
  )
}

if (checkOnly) {
  const actualSources = (await readdir(sourceRoot))
    .filter((entry) => entry.endsWith(".stack"))
    .sort()
  assert.deepEqual(actualSources, sourceNames, "Template source inventory has drifted")
  for (const [destination, expected] of snapshots) {
    assert.equal(
      await readFile(destination, "utf8"),
      expected,
      `${path.relative(process.cwd(), destination)} has drifted from the pinned specification`,
    )
  }
  console.log(`Verified ${sourceNames.length} templates against stack-sh/specification@${expectedRevision}.`)
} else {
  await mkdir(sourceRoot, { recursive: true })
  for (const entry of await readdir(sourceRoot)) {
    if (entry.endsWith(".stack") && !sourceNames.includes(entry)) {
      await rm(path.join(sourceRoot, entry))
    }
  }
  for (const [destination, contents] of snapshots) {
    await mkdir(path.dirname(destination), { recursive: true })
    await writeFile(destination, contents)
  }
  console.log(`Synchronized ${sourceNames.length} templates from stack-sh/specification@${expectedRevision}.`)
}
