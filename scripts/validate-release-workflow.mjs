import fs from "node:fs";
import path from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const workflowFile = path.join(root, ".github", "workflows", "release.yaml");

const expectedActions = new Map([
  ["actions/checkout", "3d3c42e5aac5ba805825da76410c181273ba90b1"],
  ["actions/download-artifact", "3e5f45b2cfb9172054b4087a40e8e0b5a5461e7c"],
  ["actions/upload-artifact", "043fb46d1a93c77aae656e7c1c64a875d1fc6a0a"],
  ["actions/attest", "1e69f48acb82d1966a394da916b4c1698aa569d6"],
  ["anchore/sbom-action/download-syft", "3ad7283483fc7af8ff2b4ea19663c2d5ca935e26"],
  ["sigstore/cosign-installer", "6f9f17788090df1f26f669e9d70d6ae9567deba6"],
]);

const expectedPermissions = new Map([
  ["context", ["contents: read"]],
  ["build-macos", ["contents: read"]],
  ["build-linux", ["contents: read"]],
  ["attest", ["attestations: write", "contents: read", "id-token: write"]],
  ["assemble", ["attestations: read", "contents: read", "id-token: write"]],
  ["publish", ["attestations: read", "contents: write"]],
]);

function invariant(condition, message) {
  if (!condition) throw new Error(message);
}

function occurrences(source, value) {
  return source.split(value).length - 1;
}

function extractJobs(workflow) {
  const marker = "\njobs:\n";
  const jobsIndex = workflow.indexOf(marker);
  invariant(jobsIndex >= 0, "workflow jobs are missing");
  const source = workflow.slice(jobsIndex + marker.length);
  const matches = [...source.matchAll(/^  ([a-z][a-z0-9-]+):\n/gm)];
  invariant(matches.length > 0, "workflow jobs are missing");
  return new Map(
    matches.map((match, index) => [
      match[1],
      source.slice(match.index, matches[index + 1]?.index ?? source.length),
    ]),
  );
}

function jobPermissions(job, name) {
  const block = job.match(/^    permissions:\n((?:      [a-z-]+: (?:read|write)\n)+)/m);
  invariant(block, `${name} must declare explicit job permissions`);
  return block[1]
    .trim()
    .split("\n")
    .map((line) => line.trim())
    .sort();
}

export function validateReleaseWorkflow(source) {
  const workflow = source.replaceAll("\r\n", "\n");

  invariant(/^on:\n  workflow_dispatch:\n/m.test(workflow), "release workflow must support manual verification");
  invariant(/^  push:\n    tags:\n      - "v\*"$/m.test(workflow), "release workflow must publish only from version tags");
  invariant(
    !/^  (?:pull_request|pull_request_target|schedule|repository_dispatch|workflow_run):/m.test(workflow),
    "release workflow has an unsupported automatic trigger",
  );
  invariant(!/^    branches(?:-ignore)?:/m.test(workflow), "release workflow must not publish from a branch push");
  invariant(/^permissions: \{\}$/m.test(workflow), "top-level permissions must be empty");
  invariant(!/continue-on-error:/.test(workflow), "release checks must never continue on error");
  invariant(!/\$\{\{\s*secrets(?:\.|\[)/.test(workflow), "release workflow must not use a long-lived secret");

  const jobs = extractJobs(workflow);
  invariant(
    JSON.stringify([...jobs.keys()].sort()) === JSON.stringify([...expectedPermissions.keys()].sort()),
    "release workflow must contain only the six reviewed jobs",
  );
  let permissionCount = 0;
  for (const [name, expected] of expectedPermissions) {
    const actual = jobPermissions(jobs.get(name), name);
    invariant(JSON.stringify(actual) === JSON.stringify([...expected].sort()), `${name} permissions are not least privilege`);
    permissionCount += actual.length;
  }

  const context = jobs.get("context");
  invariant(
    context.includes("if: github.event_name == 'push' || (github.event_name == 'workflow_dispatch' && github.ref == 'refs/heads/main')"),
    "manual verification must be restricted to main",
  );
  for (const requirement of [
    "node scripts/resolve-release-context.mjs",
    "git cat-file -t \"refs/tags/$GITHUB_REF_NAME\"",
    "git merge-base --is-ancestor \"$GITHUB_SHA\" refs/remotes/origin/main",
    "test -s \"docs/releases/v${RELEASE_VERSION:-${GITHUB_REF_NAME#v}}.md\"",
  ]) {
    invariant(context.includes(requirement), `immutable release context check is missing: ${requirement}`);
  }

  const expectedTimeouts = new Map([
    ["context", "10"],
    ["build-macos", "45"],
    ["build-linux", "45"],
    ["attest", "20"],
    ["assemble", "15"],
    ["publish", "15"],
  ]);
  for (const [name, minutes] of expectedTimeouts) {
    invariant(jobs.get(name).includes(`timeout-minutes: ${minutes}`), `${name} timeout is missing or changed`);
  }

  const runsOn = [...workflow.matchAll(/^    runs-on: (.+)$/gm)].map((match) => match[1]);
  invariant(
    JSON.stringify(runsOn.sort()) ===
      JSON.stringify(["${{ matrix.runner }}", "${{ matrix.runner }}", "ubuntu-24.04", "ubuntu-24.04", "ubuntu-24.04", "ubuntu-24.04"].sort()),
    "release jobs must use only reviewed GitHub-hosted runners",
  );
  const targetRunners = [
    ["aarch64-apple-darwin", "macos-15"],
    ["x86_64-apple-darwin", "macos-15-intel"],
    ["aarch64-unknown-linux-gnu", "ubuntu-22.04-arm"],
    ["x86_64-unknown-linux-gnu", "ubuntu-22.04"],
  ];
  for (const [target, runner] of targetRunners) {
    invariant(workflow.includes(`- target: ${target}\n            runner: ${runner}`), `runner mapping is missing for ${target}`);
  }

  const macos = jobs.get("build-macos");
  const linux = jobs.get("build-linux");
  invariant(
    linux.includes("rust:1.85.0-slim-bullseye@sha256:a78439ac2ee14dc1c2c188fef0ff0b197e1cc1918d4b3daf486776ac0f60029a"),
    "GNU/Linux build container must be digest pinned",
  );
  for (const requirement of [
    "snapshot.debian.org/archive/debian/20250317T000000Z",
    "snapshot.debian.org/archive/debian-security/20250317T000000Z",
    "file=1:5.39-3+deb11u1",
    "git=1:2.30.2-1+deb11u4",
    "python3=3.9.2-3",
  ]) {
    invariant(linux.includes(requirement), `GNU/Linux snapshot requirement is missing: ${requirement}`);
  }
  for (const [name, job] of [["macOS", macos], ["GNU/Linux", linux]]) {
    invariant(occurrences(job, "CARGO_TARGET_DIR=") === 2, `${name} must perform two isolated builds`);
    invariant(job.includes('cmp "$first_binary" "$second_binary"'), `${name} must compare rebuilt binaries`);
    invariant(occurrences(job, "python3 scripts/package_release.py create") === 2, `${name} must package twice`);
    invariant(job.includes('cmp "$first_archive" "$second_archive"'), `${name} must compare rebuilt archives`);
    invariant(job.includes("python3 scripts/verify_release_binary.py"), `${name} must run native binary smoke tests`);
    invariant(job.includes("--remap-path-prefix=$GITHUB_WORKSPACE=/workspace"), `${name} must remap source paths`);
  }
  for (const requirement of [
    "rustup toolchain install 1.85.0 --profile minimal",
    "MACOSX_DEPLOYMENT_TARGET=13.0",
  ]) {
    invariant(macos.includes(requirement), `macOS reproducibility requirement is missing: ${requirement}`);
  }
  invariant(
    occurrences(macos, "python3 scripts/normalize_macos_binary.py") === 2,
    "macOS builds must normalize UUIDs and ad-hoc signatures before comparison",
  );
  invariant(!macos.includes("-no_uuid"), "macOS binaries must retain a valid UUID");
  invariant(linux.includes('test "$(rustc --version)" = "rustc 1.85.0 (4d91de4e4 2025-02-17)"'), "GNU/Linux Rust version must be exact");

  const uses = [...workflow.matchAll(/^\s+uses: ([^\s#]+)(?:\s+#.*)?$/gm)].map((match) => match[1]);
  invariant(uses.length === 18, "release workflow action count changed and requires review");
  for (const action of uses) {
    const match = action.match(/^([^@]+)@([0-9a-f]{40})$/);
    invariant(match, `action must be pinned to a full commit: ${action}`);
    const expected = expectedActions.get(match[1]);
    invariant(expected, `unexpected action: ${match[1]}`);
    invariant(match[2] === expected, `unexpected commit for ${match[1]}`);
  }
  for (const action of expectedActions.keys()) {
    invariant(uses.some((value) => value.startsWith(`${action}@`)), `required action is missing: ${action}`);
  }
  invariant(
    occurrences(workflow, "persist-credentials: false") === occurrences(workflow, "actions/checkout@"),
    "every checkout must disable persisted credentials",
  );

  const attest = jobs.get("attest");
  const assemble = jobs.get("assemble");
  for (const requirement of [
    "syft-version: v1.51.1",
    "--source-name \"$archive\"",
    "--predicate-type https://spdx.dev/Document/v2.3",
    "--deny-self-hosted-runners",
  ]) {
    invariant(attest.includes(requirement), `target attestation requirement is missing: ${requirement}`);
  }
  for (const requirement of [
    "cosign-release: v3.1.3",
    "--certificate-oidc-issuer https://token.actions.githubusercontent.com",
    "--signer-workflow stack-sh/cli/.github/workflows/release.yaml",
    "node scripts/release-security.mjs verify --directory dist/release",
  ]) {
    invariant(assemble.includes(requirement), `release assembly requirement is missing: ${requirement}`);
  }

  const publish = jobs.get("publish");
  invariant(publish.includes("if: needs.context.outputs.publish == 'true'"), "publication must require the resolved tag context");
  invariant(publish.includes("gh release create \"$TAG\" dist/release/*"), "publication must begin as a draft with exact assets");
  invariant(publish.includes("--draft"), "release assets must be uploaded to a draft first");
  invariant(occurrences(publish, "gh release download \"$TAG\"") === 2, "draft assets must be downloaded before publication");
  invariant(occurrences(publish, ')" = "19"') === 2, "both draft verification paths must require all 19 assets");
  invariant(occurrences(publish, 'cmp "$source"') === 2, "downloaded draft assets must match the assembled bytes");
  invariant(occurrences(publish, "node scripts/release-security.mjs verify") === 2, "release metadata must verify before and after upload");
  invariant(occurrences(publish, "cosign verify-blob") === 2, "checksum signatures must verify before and after upload");
  invariant(occurrences(publish, "gh attestation verify") === 4, "provenance and SBOM attestations must verify before and after upload");
  invariant(
    occurrences(publish, "--predicate-type https://spdx.dev/Document/v2.3") === 2,
    "SBOM attestations must verify before and after upload",
  );
  invariant(publish.includes("python3 scripts/verify_release_binary.py"), "a downloaded native binary must pass install smoke tests");
  invariant(publish.includes('gh release edit "$TAG" --draft=false'), "only a fully verified draft may be published");

  return { actions: uses.length, jobs: jobs.size, permissions: permissionCount, targets: targetRunners.length };
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  try {
    const result = validateReleaseWorkflow(fs.readFileSync(workflowFile, "utf8"));
    console.log(
      `Validated ${result.jobs} release jobs, ${result.targets} native targets, ${result.actions} pinned action uses, and ${result.permissions} permission grants.`,
    );
  } catch (error) {
    console.error(error instanceof Error ? error.message : error);
    process.exitCode = 1;
  }
}
