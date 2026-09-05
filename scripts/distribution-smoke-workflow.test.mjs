import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

const workflow = readFileSync(".github/workflows/distribution-smoke.yaml", "utf8");
const download = readFileSync("scripts/download_smoke_archive.py", "utf8");
const smoke = readFileSync("scripts/smoke_installed_cli.py", "utf8");

function validate(source) {
  assert.match(source, /^on:\n  pull_request:\n  push:\n    branches: \[main\]/m);
  assert.match(source, /^  workflow_dispatch:/m);
  assert.match(source, /^  workflow_call:/m);
  assert.doesNotMatch(source, /pull_request_target:|workflow_run:|continue-on-error:|\bwrite\b|secrets[.[]/);
  assert.match(source, /^permissions:\n  contents: read\n  attestations: read\n/m);
  assert.deepEqual([...source.slice(source.indexOf("\njobs:\n")).matchAll(/^  ([\w-]+):$/gm)].map(match => match[1]), ["context", "install", "completion"]);
  assert.equal([...source.matchAll(/persist-credentials: false/g)].length, 4);
  const actions = [...source.matchAll(/^\s+(?:- )?uses: (\S+)/gm)].map(match => match[1]);
  assert.equal(actions.length, 7);
  for (const action of actions) assert.match(action, /@[0-9a-f]{40}$/);
  for (const guard of [
    "timeout-minutes: 30", "fail-fast: false", "matrix: ${{ fromJSON(needs.context.outputs.matrix) }}",
    "ref: ${{ needs.context.outputs.source }}", "python3 -m scripts.download_smoke_archive",
    'git -C "$tap_path" checkout --detach "$TAP_COMMIT"', "formula.installed.length, 0",
    'cmp "$prefix/etc/bash_completion.d/stack" .release-source/distribution/generated/share/bash-completion/completions/stack',
    'git init --quiet "$project"', "aqua policy allow", "aqua update-checksum", "aqua install",
    'cargo "+$RUST_VERSION" install stack-diagram-cli --version "=$VERSION" --locked --registry crates-io',
    "CARGO_HOME: ${{ runner.temp }}/cargo-registry-home", "CARGO_TARGET_DIR: ${{ runner.temp }}/cargo-registry-target",
    "python3 -m scripts.verify_smoke_cargo_source", "python3 -m scripts.smoke_installed_cli",
    '--canonical-binary "$CANONICAL_BINARY"', "if-no-files-found: error", "path: ${{ runner.temp }}/smoke.json",
    "if: always()\n    needs: [context, install]", "CONTEXT_RESULT: ${{ needs.context.result }}",
    "INSTALL_RESULT: ${{ needs.install.result }}", "requireSuccessfulSmoke(process.env.CONTEXT_RESULT, process.env.INSTALL_RESULT)",
  ]) assert.ok(source.includes(guard), `Missing guard: ${guard}`);
}

test("distribution smoke uses read-only native jobs and a fail-closed aggregate", () => validate(workflow));

test("privileged, skipped, unverified, or non-registry smoke mutations are rejected", () => {
  for (const [before, after] of [
    ["contents: read", "contents: write"], ["pull_request:", "pull_request_target:"],
    ["fail-fast: false", "continue-on-error: true"], ["if: always()", "if: success()"],
    ["python3 -m scripts.smoke_installed_cli", "echo skipped"],
    ["python3 -m scripts.download_smoke_archive", "echo unverified"],
    ["--registry crates-io", "--path ."], ["--canonical-binary", "--unverified-binary"],
    ["requireSuccessfulSmoke(process.env.CONTEXT_RESULT, process.env.INSTALL_RESULT)", "console.log('passed')"],
  ]) assert.throws(() => validate(workflow.replace(before, after)), `Accepted mutation: ${before}`);
});

test("archive authentication precedes extraction and a real provider import precedes rendering", () => {
  assert.ok(download.indexOf("verify_checksum(archive, inventory)") < download.indexOf('run("gh", "attestation"'));
  assert.ok(download.indexOf('"--source-digest", arguments.source_commit') < download.indexOf('run("tar", "-xzf"'));
  assert.ok(download.indexOf('"scripts/package_release.py", "verify"') < download.indexOf('run("tar", "-xzf"'));
  assert.match(smoke, /"icons", "import", "simple-icons", "--accept-terms"/);
  assert.match(smoke, /data-icon-id="simple-icons:rust"/);
  assert.match(smoke, /"simple-icons:rust" in notice.read_text\(\)/);
  assert.doesNotMatch(workflow, /path:.*(?:icons|provider\.svg)/);
});
