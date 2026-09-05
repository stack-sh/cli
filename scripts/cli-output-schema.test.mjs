import assert from "node:assert/strict"
import fs from "node:fs"
import path from "node:path"
import test from "node:test"
import { fileURLToPath } from "node:url"

import Ajv2020 from "ajv/dist/2020.js"

const repositoryRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..")
const schema = JSON.parse(
  fs.readFileSync(path.join(repositoryRoot, "schemas/cli-output-v1.schema.json"), "utf8"),
)
const valid = JSON.parse(
  fs.readFileSync(
    path.join(repositoryRoot, "tests/fixtures/cli-output/check-success.json"),
    "utf8",
  ),
)
const validate = new Ajv2020({ allErrors: true, strict: true }).compile(schema)

function changed(change) {
  const candidate = structuredClone(valid)
  change(candidate)
  return candidate
}

test("schema version 1 accepts its stable success envelope", () => {
  assert.equal(validate(valid), true)
})

test("schema version and unknown fields cannot drift within version 1", () => {
  assert.equal(validate(changed((value) => (value.schemaVersion = 2))), false)
  assert.equal(validate(changed((value) => (value.newField = true))), false)
})

test("outcome, exit status, diagnostics, and error remain coherent", () => {
  assert.equal(validate(changed((value) => (value.exitStatus = 1))), false)
  assert.equal(validate(changed((value) => (value.outcome = "stack-error"))), false)
  assert.equal(validate(changed((value) => (value.outcome = "operational-error"))), false)
})
