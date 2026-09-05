import assert from 'node:assert/strict';
import fs from 'node:fs';
import test from 'node:test';
import { validatePublish, validateRegistry } from './cargo-publish-context.mjs';

const sha = 'a'.repeat(40);
const context = { packageName: 'stack-diagram-cli', version: '0.1.0', expectedSha: sha, publish: 'false' };
const crate = { name: context.packageName, version: context.version, publish: ['crates-io'], license: 'Apache-2.0', rust_version: '1.85', dependencies: [] };
const metadata = { packages: [crate] };
const runs = [{ headSha: sha, status: 'completed', conclusion: 'success' }];

test('accepts exact successful source for verification or publication', () => {
  for (const publish of ['true', 'false']) validatePublish(metadata, runs, { ...context, publish });
  for (const name of ["stack-diagram-cli"]) validatePublish({ packages: [{ ...crate, name }] }, runs, { ...context, packageName: name });
});
test('rejects malformed identity, versions, or implicit publishing', () => {
  for (const change of [{ expectedSha: 'main' }, { expectedSha: sha + '\n' }, { version: '0.1.0-rc.1' }, { version: '0x1x0' }, { version: '01.0.0' }, { packageName: 'other' }, { publish: '' }]) assert.throws(() => validatePublish(metadata, runs, { ...context, ...change }));
  for (const change of [{ version: '0.2.0' }, { license: 'MIT' }, { rust_version: '1.86' }, { publish: null }, { dependencies: [{ source: 'git+https://example.com/source' }] }, { dependencies: [{ source: null, path: '../library', req: '*' }] }]) assert.throws(() => validatePublish({ packages: [{ ...crate, ...change }] }, runs, context));
});
test('rejects missing, stale, incomplete, and unsuccessful CI', () => {
  for (const invalid of [[], [...runs, ...runs], [{ ...runs[0], headSha: 'b'.repeat(40) }], [{ ...runs[0], status: 'in_progress' }], [{ ...runs[0], conclusion: 'failure' }]]) assert.throws(() => validatePublish(metadata, invalid, context));
});
test('never republishes an existing version or ignores registry failures', () => {
  validateRegistry(200, 200, 'false');
  validateRegistry(200, 404, 'false');
  validateRegistry(200, 404, 'true');
  assert.throws(() => validateRegistry(200, 200, 'true'));
  for (const code of [401, 403, 429, 500]) assert.throws(() => validateRegistry(200, code, 'false'));
  assert.throws(() => validateRegistry(404, 404, 'true'));
});
test('workflow keeps manual main-only publishing and ephemeral credentials', () => {
  const workflow = fs.readFileSync(new URL('../.github/workflows/cargo-publish.yaml', import.meta.url), 'utf8');
  assert.match(workflow, /workflow_dispatch:/);
  assert.doesNotMatch(workflow, /\n  (push|pull_request|schedule):|secrets\.|cargo login|self-hosted/);
  assert.ok(workflow.includes("if: github.ref == 'refs/heads/main'"));
  assert.ok(workflow.includes('if: inputs.publish == true'));
  assert.ok(workflow.includes('default: false'));
  assert.match(workflow, /id-token: write/);
  assert.match(workflow, /crates-io-auth-action@[a-f0-9]{40}/);
});
