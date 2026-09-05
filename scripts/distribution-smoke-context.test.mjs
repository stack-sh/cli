import assert from "node:assert/strict";
import test from "node:test";
import { readFileSync } from "node:fs";
import { smokeMatrix, validateRelease, validateVersion, requireSuccessfulSmoke } from "./distribution-smoke-context.mjs";

const version = "0.5.1";
const release = {
  tagName: `v${version}`, isDraft: false, isPrerelease: false,
  assets: [
    `stack-v${version}-checksums.txt`, `stack-v${version}-checksums.txt.sigstore.json`,
    `stack-v${version}-release-manifest.json`,
    ...smokeMatrix("native").include.filter(row => row.channel === "direct").flatMap(row => ["tar.gz", "spdx.json", "provenance.sigstore.json", "sbom.sigstore.json"].map(suffix => `stack-v${version}-${row.target}.${suffix}`)),
  ].map(name => ({ name })),
};

test("the all-channel matrix covers the declared supported targets without foreign execution", () => {
  const contract = JSON.parse(readFileSync("distribution/distribution-contract.json", "utf8"));
  const rows = smokeMatrix("all").include;
  assert.equal(rows.length, 19);
  assert.equal(new Set(rows.map(row => `${row.channel}/${row.target}/${row.rust}`)).size, 19);
  for (const channel of contract.channels) {
    const id = channel.id === "github-release" ? "direct" : channel.id;
    assert.deepEqual([...new Set(rows.filter(row => row.channel === id).map(row => row.target))].sort(), [...channel.targets].sort());
  }
  for (const row of rows) {
    assert.ok(row.target.includes("apple") ? row.runner.startsWith("macos-") : row.runner.startsWith("ubuntu-"));
    assert.ok(row.target.startsWith("aarch64") ? !row.runner.endsWith("intel") : row.target.includes("apple") ? row.runner.endsWith("intel") : !row.runner.endsWith("arm"));
    if (row.target.includes("linux")) assert.equal(row.runner.endsWith("-arm"), row.target.startsWith("aarch64"));
  }
  assert.equal(smokeMatrix("native").include.length, 8);
  assert.equal(smokeMatrix("cargo").include.length, 8);
  assert.throws(() => smokeMatrix("skip"), /Unsupported/);
});

test("a complete stable release passes, but missing assets and version drift fail", () => {
  validateRelease(release, version);
  for (let i = 0; i < release.assets.length; i++) {
    assert.throws(() => validateRelease({ ...release, assets: release.assets.filter((_, index) => index !== i) }, version), /Missing release asset/);
  }
  assert.throws(() => validateRelease(release, "0.5.2"), /version mismatch/);
  assert.throws(() => validateRelease({ ...release, isDraft: true }, version), /Draft/);
  assert.throws(() => validateRelease({ ...release, isPrerelease: true }, version), /Prerelease/);
  assert.throws(() => validateRelease({ ...release, assets: [...release.assets, release.assets[0]] }, version), /Duplicate/);
});

test("versions cannot select a range, floating release, prerelease, or shell expression", () => {
  for (const invalid of ["latest", "^0.5", "0.5.1-rc.1", "0.5.1\n", "$(whoami)", "--help"]) {
    assert.throws(() => validateVersion(invalid));
  }
});

test("the completion gate never accepts skipped, cancelled, empty, or failing jobs", () => {
  requireSuccessfulSmoke("success", "success");
  for (const result of ["failure", "cancelled", "skipped", "", undefined]) {
    assert.throws(() => requireSuccessfulSmoke(result, "success"));
    assert.throws(() => requireSuccessfulSmoke("success", result));
  }
});
