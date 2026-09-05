import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { readFileSync } from "node:fs";
import { pathToFileURL } from "node:url";

const targets = [
  ["aarch64-apple-darwin", "macos-15"],
  ["x86_64-apple-darwin", "macos-15-intel"],
  ["aarch64-unknown-linux-gnu", "ubuntu-24.04-arm"],
  ["x86_64-unknown-linux-gnu", "ubuntu-24.04"],
];

export function smokeMatrix(scope) {
  assert.ok(["all", "native", "cargo"].includes(scope), "Unsupported smoke scope");
  const include = [];
  for (const [target, runner] of targets) {
    if (scope !== "cargo") {
      for (const channel of ["direct", "aqua"]) include.push({ channel, target, runner, rust: "none" });
    }
    if (scope !== "native") {
      for (const rust of ["1.85.0", "stable"]) include.push({ channel: "cargo", target, runner, rust });
    }
    if (scope === "all" && target !== "x86_64-apple-darwin") {
      include.push({ channel: "homebrew", target, runner: target.endsWith("apple-darwin") ? "macos-26" : runner, rust: "none" });
    }
  }
  return { include };
}

export function validateVersion(version) {
  assert.match(version, /^\d+\.\d+\.\d+$/, "Only exact stable versions are supported");
  return version;
}

export function validateRelease(release, version) {
  validateVersion(version);
  assert.equal(release.tagName, `v${version}`, "Release version mismatch");
  assert.equal(release.isDraft, false, "Draft release is not installable");
  assert.equal(release.isPrerelease, false, "Prerelease is not a stable channel release");
  const names = release.assets.map(asset => asset.name);
  assert.equal(new Set(names).size, names.length, "Duplicate release asset");
  for (const name of [
    `stack-v${version}-checksums.txt`,
    `stack-v${version}-checksums.txt.sigstore.json`,
    `stack-v${version}-release-manifest.json`,
    ...targets.flatMap(([target]) => ["tar.gz", "spdx.json", "provenance.sigstore.json", "sbom.sigstore.json"].map(suffix => `stack-v${version}-${target}.${suffix}`)),
  ]) assert.ok(names.includes(name), `Missing release asset: ${name}`);
}

export function requireSuccessfulSmoke(context, install) {
  assert.equal(context, "success", "Smoke context failed or was skipped");
  assert.equal(install, "success", "At least one install failed, was cancelled, or was skipped");
}

function gh(...args) {
  return JSON.parse(execFileSync("gh", args, { encoding: "utf8", timeout: 60_000 }));
}

export function resolveContext(env = process.env) {
  const scope = env.SMOKE_SCOPE || "all";
  const contract = JSON.parse(readFileSync("distribution/distribution-contract.json", "utf8"));
  const version = validateVersion(env.SMOKE_VERSION || contract.product.currentReleaseVersion);
  const matrix = smokeMatrix(scope);
  let source = env.SMOKE_SOURCE_COMMIT;
  if (scope === "cargo") {
    assert.match(source || "", /^[0-9a-f]{40}$/, "Cargo-only smoke requires its published source commit");
  } else {
    const release = gh("release", "view", `v${version}`, "--repo", "stack-sh/cli", "--json", "tagName,isDraft,isPrerelease,assets");
    validateRelease(release, version);
    const tagged = gh("api", `repos/stack-sh/cli/commits/v${version}`).sha;
    if (source) assert.equal(source, tagged, "Published source does not match the release tag");
    source = tagged;
  }
  assert.match(source, /^[0-9a-f]{40}$/);
  const tap = scope === "all" ? gh("api", "repos/stack-sh/homebrew-tap/git/ref/heads/main").object.sha : "";
  if (tap) assert.match(tap, /^[0-9a-f]{40}$/);
  return { version, scope, source, tap, matrix: JSON.stringify(matrix) };
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  for (const [key, value] of Object.entries(resolveContext())) console.log(`${key}=${value}`);
}
