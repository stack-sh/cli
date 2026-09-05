import fs from "node:fs"
import path from "node:path"
import { fileURLToPath } from "node:url"

import Ajv2020 from "ajv/dist/2020.js"

const repositoryRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..")
const schemaPath = path.join(repositoryRoot, "schemas/cli-output-v1.schema.json")
const fixtureDirectory = path.join(repositoryRoot, "tests/fixtures/cli-output")
const schema = JSON.parse(fs.readFileSync(schemaPath, "utf8"))
const ajv = new Ajv2020({ allErrors: true, strict: true })
const validate = ajv.compile(schema)
const fixtureNames = fs
  .readdirSync(fixtureDirectory)
  .filter((name) => name.endsWith(".json"))
  .sort()

if (fixtureNames.length === 0) {
  throw new Error("No CLI output fixtures were found")
}

for (const fixtureName of fixtureNames) {
  const fixture = JSON.parse(fs.readFileSync(path.join(fixtureDirectory, fixtureName), "utf8"))
  if (!validate(fixture)) {
    throw new Error(`${fixtureName}: ${ajv.errorsText(validate.errors)}`)
  }
}

process.stdout.write(`Validated ${fixtureNames.length} CLI output fixtures against schema version 1.\n`)
