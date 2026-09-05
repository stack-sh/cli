import assert from 'node:assert/strict';
import fs from 'node:fs';
import test from 'node:test';
import { requireCargoCompletion, requireReleaseCompletion } from './publication-completion.mjs';
import { validateReleaseWorkflow } from './validate-release-workflow.mjs';

function releaseNeeds(publish, version = '0.5.1') {
  const needs = Object.fromEntries(['context', 'build-macos', 'build-linux', 'attest', 'assemble'].map(job => [job, { result: 'success' }]));
  needs.context.outputs = { publish, version };
  needs.publish = { result: publish === 'true' ? 'success' : 'skipped' };
  needs['native-install'] = { result: publish === 'true' && !version.includes('-rc.') ? 'success' : 'skipped' };
  return needs;
}

test('stable release, prerelease, and dry-run have distinct completion requirements', () => {
  assert.match(requireReleaseCompletion(releaseNeeds('true')), /all-channel activation still requires/);
  assert.match(requireReleaseCompletion(releaseNeeds('true', '0.6.0-rc.1')), /stable package-manager activation is not claimed/);
  assert.match(requireReleaseCompletion(releaseNeeds('false', '0.6.0')), /nothing was published/);
  assert.throws(() => requireReleaseCompletion(releaseNeeds('')), /intent is missing/);
});

test('publication cannot succeed with a failed, cancelled, skipped, or missing required job', () => {
  for (const publish of ['true', 'false']) {
    const original = releaseNeeds(publish);
    for (const job of Object.keys(original)) {
      for (const result of ['failure', 'cancelled', 'skipped', 'success', undefined]) {
        if (result === original[job].result) continue;
        const needs = structuredClone(original);
        needs[job].result = result;
        assert.throws(() => requireReleaseCompletion(needs), `${publish}/${job}/${result}`);
      }
    }
  }
});

test('Cargo upload and no-upload verification cannot be confused', () => {
  const needs = { publish: { result: 'success' }, 'cargo-install': { result: 'success' } };
  assert.match(requireCargoCompletion(needs, 'true'), /all eight Cargo installations verified/);
  assert.throws(() => requireCargoCompletion(needs, 'false'));
  needs['cargo-install'].result = 'skipped';
  assert.match(requireCargoCompletion(needs, 'false'), /without publication/);
  assert.throws(() => requireCargoCompletion(needs, 'true'));
  for (const result of ['failure', 'cancelled', undefined]) {
    assert.throws(() => requireCargoCompletion({ ...needs, publish: { result } }, 'false'));
    assert.throws(() => requireCargoCompletion({ ...needs, 'cargo-install': { result } }, 'true'));
  }
});

test('release integration rejects missing import, mutable reuse, and detached completion', () => {
  const workflow = fs.readFileSync('.github/workflows/release.yaml', 'utf8');
  for (const [before, after] of [
    ['python3 -m scripts.smoke_installed_cli', 'echo skipped'],
    ['./.github/workflows/distribution-smoke.yaml', 'stack-sh/cli/.github/workflows/distribution-smoke.yaml@main'],
    ['needs: [context, publish]', 'needs: context'],
    ['scope: native', 'scope: all'],
    ['source_commit: ${{ github.sha }}', 'source_commit: main'],
    ['if: always()', 'if: success()'],
    ['node scripts/publication-completion.mjs release', 'echo passed'],
  ]) assert.throws(() => validateReleaseWorkflow(workflow.replace(before, after)), before);
});

test('Cargo integration starts only after upload and always reports completion', () => {
  const workflow = fs.readFileSync('.github/workflows/cargo-publish.yaml', 'utf8');
  for (const requirement of [
    "cargo-install:\n    if: inputs.publish == true\n    needs: publish",
    "uses: ./.github/workflows/distribution-smoke.yaml", "scope: cargo",
    "version: ${{ inputs.version }}", "source_commit: ${{ inputs.expected_sha }}",
    "if: always()\n    needs: [publish, cargo-install]", "node scripts/publication-completion.mjs cargo",
  ]) assert.ok(workflow.includes(requirement), requirement);
  assert.doesNotMatch(workflow, /continue-on-error:|secrets[.[]/);
});
