import assert from 'node:assert/strict';
import { pathToFileURL } from 'node:url';

function requireResult(needs, job, expected = 'success') {
  assert.equal(needs[job]?.result, expected, `${job} must finish with ${expected}`);
}

export function requireReleaseCompletion(needs) {
  for (const job of ['context', 'build-macos', 'build-linux', 'attest', 'assemble']) requireResult(needs, job);
  const { publish, version } = needs.context.outputs;
  assert.ok(['true', 'false'].includes(publish), 'Publication intent is missing');
  assert.match(version, /^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)(?:-rc\.[1-9]\d*)?$/);
  requireResult(needs, 'publish', publish === 'true' ? 'success' : 'skipped');
  const stablePublished = publish === 'true' && !version.includes('-rc.');
  requireResult(needs, 'native-install', stablePublished ? 'success' : 'skipped');
  return stablePublished ? 'Native publication verified; all-channel activation still requires a full distribution smoke run.'
    : publish === 'true' ? 'Release candidate verified; stable package-manager activation is not claimed.'
      : 'New artifacts passed dry-run verification, including real provider import; nothing was published.';
}

export function requireCargoCompletion(needs, publish) {
  assert.ok(['true', 'false'].includes(publish), 'Publication intent is missing');
  requireResult(needs, 'publish');
  requireResult(needs, 'cargo-install', publish === 'true' ? 'success' : 'skipped');
  return publish === 'true' ? 'Registry publication and all eight Cargo installations verified; other channels are independent.'
    : 'Packaging and OIDC verified without publication; new-version installation is not claimed.';
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  const needs = JSON.parse(process.env.PUBLICATION_NEEDS);
  const mode = process.argv[2];
  assert.ok(['release', 'cargo'].includes(mode));
  console.log(mode === 'release' ? requireReleaseCompletion(needs) : requireCargoCompletion(needs, process.env.PUBLISH));
}
