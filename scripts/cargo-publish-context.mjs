import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { fileURLToPath } from 'node:url';
import path from 'node:path';

const packages = ["stack-diagram-cli"];

export function validatePublish(metadata, runs, context) {
  assert.match(context.expectedSha, /^[a-f0-9]{40}$/);
  assert.match(context.version, /^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)$/);
  assert.ok(packages.includes(context.packageName), 'Unexpected crate');
  assert.ok(['true', 'false'].includes(context.publish), 'Explicit publish mode required');
  const matches = metadata.packages.filter(crate => crate.name === context.packageName);
  assert.equal(matches.length, 1);
  const crate = matches[0];
  assert.equal(crate.version, context.version);
  assert.equal(crate.license, 'Apache-2.0');
  assert.equal(crate.rust_version, '1.85');
  assert.deepEqual(crate.publish, ['crates-io']);
  for (const dependency of crate.dependencies) {
    assert.ok(dependency.source === 'registry+https://github.com/rust-lang/crates.io-index' || (dependency.source === null && dependency.path && /^=[0-9]+\.[0-9]+\.[0-9]+$/.test(dependency.req)), 'Dependencies must resolve from crates.io when packaged');
  }
  assert.equal(runs.length, 1, 'Exact main source needs successful CI');
  assert.equal(runs[0].headSha, context.expectedSha);
  assert.equal(runs[0].status, 'completed');
  assert.equal(runs[0].conclusion, 'success');
}

export function validateRegistry(crateStatus, versionStatus, publish) {
  assert.equal(crateStatus, 200, 'Only existing crates may use trusted publishing');
  assert.ok([200, 404].includes(versionStatus), 'Registry version lookup failed');
  if (publish === 'true') assert.equal(versionStatus, 404, 'Published versions are immutable');
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const context = { packageName: process.env.PACKAGE_NAME, version: process.env.EXPECTED_VERSION, expectedSha: process.env.EXPECTED_SHA, publish: process.env.PUBLISH };
  const metadata = JSON.parse(await readFile(process.argv[2], 'utf8'));
  const runs = JSON.parse(await readFile(process.argv[3], 'utf8'));
  validatePublish(metadata, runs, context);
  const base = 'https://crates.io/api/v1/crates/' + context.packageName;
  const status = async url => (await fetch(url, { headers: { 'User-Agent': 'stack-sh/cli publication (https://github.com/stack-sh/cli)' }, signal: AbortSignal.timeout(30000) })).status;
  validateRegistry(await status(base), await status(base + '/' + context.version), context.publish);
  console.log('Exact source, package, main CI, and registry state verified.');
}
