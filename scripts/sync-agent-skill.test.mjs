import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { mkdir, mkdtemp, readFile, rm, writeFile } from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import test from 'node:test';
import { fetchSkill, syncSkill } from './sync-agent-skill.mjs';

const digest = text => createHash('sha256').update(text).digest('hex');
const content = 'canonical skill\n';
function fixture(schemaVersion = '1.0') {
  const manifest = JSON.stringify({ schemaVersion, files: [{ path: 'skills/stack-diagrams/SKILL.md', sha256: digest(content) }] });
  const lock = { repository: 'stack-sh/docs', revision: 'a'.repeat(40), manifestSha256: digest(manifest) };
  const fetchResource = async url => {
    assert.ok(url.startsWith(`https://raw.githubusercontent.com/stack-sh/docs/${lock.revision}/generated/`));
    return new Response(url.endsWith('manifest.json') ? manifest : content);
  };
  return { lock, manifest, fetchResource };
}

test('fetches only immutable provider artifacts with verified hashes', async () => {
  const { lock, fetchResource } = fixture();
  assert.equal((await fetchSkill(lock, fetchResource)).toString(), content);
});

test('rejects mutable refs, unsupported schemas, missing resources, and tampering', async () => {
  const { lock, manifest, fetchResource } = fixture();
  await assert.rejects(fetchSkill({ ...lock, revision: 'main' }, fetchResource));
  await assert.rejects(fetchSkill(lock, async () => new Response('', { status: 404 })), /HTTP 404/);
  await assert.rejects(fetchSkill(lock, async () => new Response('altered')), /manifest integrity/);
  await assert.rejects(fetchSkill(lock, async url => new Response(url.endsWith('manifest.json') ? manifest : 'altered')), /skill integrity/);
  const newer = fixture('2.0');
  await assert.rejects(fetchSkill(newer.lock, newer.fetchResource), /Unsupported Docs manifest/);
});

test('check is read-only, rejects drift, and explicit sync restores canonical bytes', async t => {
  const directory = await mkdtemp(path.join(os.tmpdir(), 'stack-cli-docs-source-'));
  t.after(() => rm(directory, { recursive: true, force: true }));
  const { lock, fetchResource } = fixture();
  await mkdir(path.join(directory, 'skills/stack-diagrams'), { recursive: true });
  await writeFile(path.join(directory, 'skills/docs-source.json'), JSON.stringify(lock));
  const target = path.join(directory, 'skills/stack-diagrams/SKILL.md');
  await writeFile(target, 'manual change');
  await assert.rejects(syncSkill(directory, false, fetchResource), /Generated skill drift/);
  assert.equal(await readFile(target, 'utf8'), 'manual change');
  await syncSkill(directory, true, fetchResource);
  await syncSkill(directory, false, fetchResource);
  assert.equal(await readFile(target, 'utf8'), content);
});
