import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";

import { validateReleaseWorkflow } from "./validate-release-workflow.mjs";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const workflow = fs.readFileSync(path.join(root, ".github", "workflows", "release.yaml"), "utf8");

test("the checked-in release workflow has the reviewed target and trust boundaries", () => {
  assert.deepEqual(validateReleaseWorkflow(workflow), { actions: 18, jobs: 6, permissions: 11, targets: 4 });
});

test("an additional automatic trigger is rejected", () => {
  const candidate = workflow.replace("  workflow_dispatch:\n", "  workflow_dispatch:\n  pull_request:\n");
  assert.throws(() => validateReleaseWorkflow(candidate), /unsupported automatic trigger/);
});

test("a manual run outside main is rejected", () => {
  const candidate = workflow.replace(" && github.ref == 'refs/heads/main'", "");
  assert.throws(() => validateReleaseWorkflow(candidate), /restricted to main/);
});

test("a floating action reference is rejected", () => {
  const candidate = workflow.replace(
    "actions/checkout@3d3c42e5aac5ba805825da76410c181273ba90b1",
    "actions/checkout@v7",
  );
  assert.throws(() => validateReleaseWorkflow(candidate), /full commit/);
});

test("an unpinned build container is rejected", () => {
  const candidate = workflow.replace(
    "rust:1.85.0-slim-bullseye@sha256:a78439ac2ee14dc1c2c188fef0ff0b197e1cc1918d4b3daf486776ac0f60029a",
    "rust:1.85.0-slim-bullseye",
  );
  assert.throws(() => validateReleaseWorkflow(candidate), /digest pinned/);
});

test("an expanded permission is rejected", () => {
  const candidate = workflow.replace("      contents: read\n", "      contents: write\n");
  assert.throws(() => validateReleaseWorkflow(candidate), /permissions are not least privilege/);
});

test("a long-lived secret reference is rejected", () => {
  for (const reference of ["{{ secrets.RELEASE_TOKEN }}", "{{ secrets['RELEASE_TOKEN'] }}"]) {
    assert.throws(() => validateReleaseWorkflow(`${workflow}\n# $${reference}\n`), /long-lived secret/);
  }
});

test("a self-hosted runner substitution is rejected", () => {
  const candidate = workflow.replace("runs-on: ubuntu-24.04", "runs-on: self-hosted");
  assert.throws(() => validateReleaseWorkflow(candidate), /reviewed GitHub-hosted runners/);
});

test("a missing independent archive comparison is rejected", () => {
  const candidate = workflow.replace('          cmp "$first_archive" "$second_archive"\n', "");
  assert.throws(() => validateReleaseWorkflow(candidate), /compare rebuilt archives/);
});

test("publication without the resolved tag gate is rejected", () => {
  const candidate = workflow.replace("    if: needs.context.outputs.publish == 'true'\n", "");
  assert.throws(() => validateReleaseWorkflow(candidate), /resolved tag context/);
});

test("publication without post-upload SBOM verification is rejected", () => {
  const marker = "--predicate-type https://spdx.dev/Document/v2.3";
  const lastIndex = workflow.lastIndexOf(marker);
  assert.notEqual(lastIndex, -1);
  const candidate = workflow.slice(0, lastIndex) + workflow.slice(lastIndex + marker.length);
  assert.throws(() => validateReleaseWorkflow(candidate), /SBOM attestations must verify/);
});
