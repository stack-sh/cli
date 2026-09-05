import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";

import { createSmokeFixture } from "./create-supply-chain-smoke-fixture.mjs";
import { validateSupplyChainWorkflow } from "./validate-supply-chain-workflow.mjs";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const workflow = fs.readFileSync(path.join(root, ".github", "workflows", "supply-chain.yaml"), "utf8");
const cargoToml = fs.readFileSync(path.join(root, "Cargo.toml"), "utf8");
const packageVersion = cargoToml.match(/^version\s*=\s*"([^"]+)"/m)?.[1];

test("the checked-in supply-chain workflow is least privilege and fully pinned", () => {
  assert.deepEqual(validateSupplyChainWorkflow(workflow), { actions: 6, permissions: 3 });
});

test("automatic signing triggers are rejected", () => {
  const candidate = workflow.replace("  workflow_dispatch:\n", "  workflow_dispatch:\n  pull_request:\n");
  assert.throws(() => validateSupplyChainWorkflow(candidate), /must not run on an automatic trigger/);
});

test("a signing job that is not main-only is rejected", () => {
  const candidate = workflow.replace(" && github.ref == 'refs/heads/main'", "");
  assert.throws(() => validateSupplyChainWorkflow(candidate), /main-branch manual dispatch/);
});

test("an unnecessary write permission is rejected", () => {
  const candidate = workflow.replace("contents: read", "contents: write");
  assert.throws(() => validateSupplyChainWorkflow(candidate), /unnecessary write permission/);
});

test("an additional job permission is rejected", () => {
  const candidate = workflow.replace("      contents: read\n", "      contents: read\n      issues: write\n");
  assert.throws(() => validateSupplyChainWorkflow(candidate), /job permissions must be exactly/);
});

test("a floating action reference is rejected", () => {
  const candidate = workflow.replace(
    "actions/checkout@3d3c42e5aac5ba805825da76410c181273ba90b1",
    "actions/checkout@v7",
  );
  assert.throws(() => validateSupplyChainWorkflow(candidate), /full commit/);
});

test("a long-lived secret reference is rejected", () => {
  for (const reference of ["{{ secrets.RELEASE_TOKEN }}", "{{ secrets['RELEASE_TOKEN'] }}"]) {
    const candidate = `${workflow}\n# $${reference}\n`;
    assert.throws(() => validateSupplyChainWorkflow(candidate), /long-lived secret/);
  }
});

test("the smoke fixture is bounded and never overwritten", (t) => {
  const directory = fs.mkdtempSync(path.join(os.tmpdir(), "stack-supply-chain-smoke-"));
  t.after(() => fs.rmSync(directory, { force: true, recursive: true }));

  const fixture = createSmokeFixture(directory);
  assert.ok(packageVersion, "Cargo.toml package version is missing");
  assert.equal(path.basename(fixture), `stack-v${packageVersion}-supply-chain-smoke.bin`);
  assert.match(fs.readFileSync(fixture, "utf8"), /^Stack supply-chain smoke fixture\nsource=/);
  assert.throws(() => createSmokeFixture(directory), /refusing to replace smoke fixture/);
});
