import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { readFile, writeFile } from 'node:fs/promises';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const root = fileURLToPath(new URL('../', import.meta.url));
const skillPath = 'skills/stack-diagrams/SKILL.md';
const digest = bytes => createHash('sha256').update(bytes).digest('hex');

export async function fetchSkill(lock, fetchResource = fetch) {
  assert.equal(lock.repository, 'stack-sh/docs');
  assert.match(lock.revision, /^[a-f0-9]{40}$/);
  assert.match(lock.manifestSha256, /^[a-f0-9]{64}$/);
  const base = `https://raw.githubusercontent.com/${lock.repository}/${lock.revision}/generated/`;
  const get = async file => {
    const response = await fetchResource(base + file, { signal: AbortSignal.timeout(15_000) });
    assert.ok(response.ok, `Docs resource unavailable: ${file} (HTTP ${response.status})`);
    const bytes = Buffer.from(await response.arrayBuffer());
    assert.ok(bytes.length <= 1_048_576, 'Docs resource exceeds size limit');
    return bytes;
  };
  const manifestBytes = await get('manifest.json');
  assert.equal(digest(manifestBytes), lock.manifestSha256, 'Docs manifest integrity mismatch');
  const manifest = JSON.parse(manifestBytes.toString('utf8'));
  assert.equal(manifest.schemaVersion, '1.0', 'Unsupported Docs manifest version');
  assert.ok(Array.isArray(manifest.files));
  const entries = manifest.files.filter(entry => entry.path === skillPath);
  assert.equal(entries.length, 1, 'Expected exactly one skill artifact');
  assert.match(entries[0].sha256, /^[a-f0-9]{64}$/);
  const bytes = await get(skillPath);
  assert.equal(digest(bytes), entries[0].sha256, 'Docs skill integrity mismatch');
  return bytes;
}

export async function syncSkill(directory = root, write = false, fetchResource = fetch) {
  const lock = JSON.parse(await readFile(path.join(directory, 'skills/docs-source.json'), 'utf8'));
  const bytes = await fetchSkill(lock, fetchResource);
  const target = path.join(directory, skillPath);
  if (write) await writeFile(target, bytes);
  else assert.deepEqual(await readFile(target), bytes, 'Generated skill drift: update Docs source, then run npm run skills:sync');
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  assert.ok(process.argv.slice(2).every(arg => ['--check', '--sync'].includes(arg)));
  assert.ok(!(process.argv.includes('--check') && process.argv.includes('--sync')));
  await syncSkill(root, process.argv.includes('--sync'));
}
