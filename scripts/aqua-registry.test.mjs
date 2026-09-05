import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const configuration = fs.readFileSync(path.join(root, "tests/aqua/aqua.yaml"), "utf8");
const policy = fs.readFileSync(path.join(root, "tests/aqua/aqua-policy.yaml"), "utf8");
const registry = fs.readFileSync(path.join(root, "aqua/registry.yaml"), "utf8");
const distribution = fs.readFileSync(path.join(root, "docs/distribution.md"), "utf8");
const checksums = JSON.parse(
  fs.readFileSync(path.join(root, "tests/aqua/aqua-checksums.json"), "utf8"),
);

const targets = [
  "aarch64-apple-darwin",
  "aarch64-unknown-linux-gnu",
  "x86_64-apple-darwin",
  "x86_64-unknown-linux-gnu",
];

test("the owner registry is pinned to one immutable revision", () => {
  const revision = configuration.match(/^\s+ref: ([0-9a-f]{40})$/m)?.[1];

  assert.match(revision ?? "", /^[0-9a-f]{40}$/);
  assert.ok(policy.includes(`ref: 'Version == "${revision}"'`));
  assert.ok(distribution.includes(`immutable commit \`${revision}\``));
  assert.ok(distribution.includes(`ref: ${revision}`));
  assert.ok(distribution.includes(`ref: 'Version == "${revision}"'`));
  assert.ok(!configuration.includes("ref: main"));
});

test("the registry maps exactly the four supported release targets", () => {
  assert.ok(registry.includes("asset: stack-{{.Version}}-{{.Arch}}-{{.OS}}.{{.Format}}"));
  assert.ok(registry.includes("asset: stack-{{.Version}}-checksums.txt"));
  assert.ok(registry.includes("asset: stack-{{.Version}}-checksums.txt.sigstore.json"));
  assert.ok(registry.includes("src: \"{{.AssetWithoutExt}}/stack\""));
  for (const environment of ["darwin/amd64", "darwin/arm64", "linux/amd64", "linux/arm64"]) {
    assert.ok(registry.includes(`- ${environment}`));
  }
  assert.ok(!registry.includes("windows/"));
});

test("the checksum lock covers every archive and the registry revision", () => {
  const revision = configuration.match(/^\s+ref: ([0-9a-f]{40})$/m)?.[1];
  const expectedIds = targets.map(
    (target) =>
      `github_release/github.com/stack-sh/cli/v0.3.0/stack-v0.3.0-${target}.tar.gz`,
  );
  expectedIds.push(
    `registries/github_content/github.com/stack-sh/cli/${revision}/aqua/registry.yaml`,
  );

  assert.deepEqual(
    checksums.checksums.map(({ id }) => id).sort(),
    expectedIds.sort(),
  );
  for (const entry of checksums.checksums) {
    assert.match(entry.checksum, /^[A-F0-9]+$/);
    assert.equal(entry.algorithm, entry.id.startsWith("registries/") ? "sha512" : "sha256");
  }
});
