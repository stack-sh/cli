import assert from "node:assert/strict";
import crypto from "node:crypto";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import test from "node:test";

import {
  generateReleaseMetadata,
  releaseLayout,
  sha256File,
  verifyReleaseMetadata,
} from "./release-security.mjs";

const version = "0.3.0";
const commit = "0123456789abcdef0123456789abcdef01234567";
const provenancePredicate = "https://slsa.dev/provenance/v1";
const sbomPredicate = "https://spdx.dev/Document/v2.3";
const bundleMediaType = "application/vnd.dev.sigstore.bundle.v0.3+json";

function temporaryDirectory(t) {
  const directory = fs.mkdtempSync(path.join(os.tmpdir(), "stack-release-security-"));
  t.after(() => fs.rmSync(directory, { force: true, recursive: true }));
  return directory;
}

function spdxDocument(archive) {
  return {
    spdxVersion: "SPDX-2.3",
    dataLicense: "CC0-1.0",
    SPDXID: "SPDXRef-DOCUMENT",
    name: `${archive} SBOM`,
    documentNamespace: `https://stack-diagram.com/sbom/${archive}`,
    creationInfo: {
      created: "2026-09-05T00:00:00Z",
      creators: ["Tool: syft-1.51.1"],
    },
    packages: [
      {
        name: "stack",
        SPDXID: "SPDXRef-Package-stack",
        versionInfo: version,
        downloadLocation: "NOASSERTION",
      },
    ],
  };
}

function attestation(subject, digest, predicateType, predicate) {
  const statement = {
    _type: "https://in-toto.io/Statement/v1",
    subject: [{ name: subject, digest: { sha256: digest } }],
    predicateType,
    predicate,
  };
  return {
    mediaType: bundleMediaType,
    verificationMaterial: { certificate: { rawBytes: "Zml4dHVyZQ==" } },
    dsseEnvelope: {
      payloadType: "application/vnd.in-toto+json",
      payload: Buffer.from(JSON.stringify(statement)).toString("base64"),
      signatures: [{ sig: "Zml4dHVyZQ==" }],
    },
  };
}

function writeJson(directory, name, value) {
  fs.writeFileSync(path.join(directory, name), `${JSON.stringify(value, null, 2)}\n`);
}

function createInputs(directory, mutate = () => {}) {
  for (const layout of releaseLayout(version)) {
    fs.writeFileSync(path.join(directory, layout.archive), `archive fixture for ${layout.target}\n`);
    const digest = sha256File(path.join(directory, layout.archive));
    const spdx = spdxDocument(layout.archive);
    const fixture = {
      layout,
      digest,
      spdx,
      provenance: attestation(layout.archive, digest, provenancePredicate, {
        buildDefinition: { buildType: "https://github.com/actions/runner" },
        runDetails: { builder: { id: "https://github.com/stack-sh/cli/actions" } },
      }),
      sbomAttestation: attestation(layout.archive, digest, sbomPredicate, spdx),
    };
    mutate(fixture);
    writeJson(directory, layout.sbom, fixture.spdx);
    writeJson(directory, layout.provenance, fixture.provenance);
    writeJson(directory, layout.sbomAttestation, fixture.sbomAttestation);
  }
}

function generate(directory) {
  return generateReleaseMetadata({
    directory,
    version,
    commit,
    minimumSupportedCliVersion: "0.3.0",
    sourceDateEpoch: 1_788_566_400,
    builderWorkflow: "stack-sh/cli/.github/workflows/release.yaml",
    verifiedChannels: ["github-release"],
  });
}

function writeChecksumSignature(directory, checksumName, digest) {
  const signatureName = `${checksumName}.sigstore.json`;
  writeJson(directory, signatureName, {
    mediaType: bundleMediaType,
    verificationMaterial: { certificate: { rawBytes: "Zml4dHVyZQ==" } },
    messageSignature: {
      messageDigest: {
        algorithm: "SHA2_256",
        digest: digest ?? Buffer.from(sha256File(path.join(directory, checksumName)), "hex").toString("base64"),
      },
      signature: "Zml4dHVyZQ==",
    },
  });
}

function preparedRelease(t) {
  const directory = temporaryDirectory(t);
  createInputs(directory);
  const generated = generate(directory);
  writeChecksumSignature(directory, generated.checksumName);
  return { directory, generated };
}

test("complete metadata for the four-target release verifies", (t) => {
  const { directory, generated } = preparedRelease(t);

  assert.deepEqual(generated, {
    manifestName: `stack-v${version}-release-manifest.json`,
    checksumName: `stack-v${version}-checksums.txt`,
    targets: 4,
    checksums: 17,
  });
  assert.deepEqual(verifyReleaseMetadata(directory), {
    version,
    targets: 4,
    checksums: 17,
  });
  const manifest = JSON.parse(fs.readFileSync(path.join(directory, generated.manifestName), "utf8"));
  assert.equal(
    manifest.$schema,
    `https://raw.githubusercontent.com/stack-sh/cli/${commit}/distribution/release-manifest.schema.json`,
  );
});

test("an archive modified after metadata generation is rejected", (t) => {
  const { directory } = preparedRelease(t);
  const archive = releaseLayout(version)[0].archive;
  fs.appendFileSync(path.join(directory, archive), "tampered\n");

  assert.throws(() => verifyReleaseMetadata(directory), /archive checksum mismatch/);
});

test("provenance for a different artifact is rejected", (t) => {
  const directory = temporaryDirectory(t);
  let changed = false;
  createInputs(directory, (fixture) => {
    if (!changed) {
      const statement = JSON.parse(
        Buffer.from(fixture.provenance.dsseEnvelope.payload, "base64").toString("utf8"),
      );
      statement.subject[0].name = "another-artifact.tar.gz";
      fixture.provenance.dsseEnvelope.payload = Buffer.from(JSON.stringify(statement)).toString("base64");
      changed = true;
    }
  });

  assert.throws(() => generate(directory), /does not attest/);
});

test("an attested SBOM that differs from the published SBOM is rejected", (t) => {
  const directory = temporaryDirectory(t);
  let changed = false;
  createInputs(directory, (fixture) => {
    if (!changed) {
      const statement = JSON.parse(
        Buffer.from(fixture.sbomAttestation.dsseEnvelope.payload, "base64").toString("utf8"),
      );
      statement.predicate.name = "different SBOM";
      fixture.sbomAttestation.dsseEnvelope.payload = Buffer.from(JSON.stringify(statement)).toString("base64");
      changed = true;
    }
  });

  assert.throws(() => generate(directory), /does not attest the published SBOM/);
});

test("a missing target material is rejected", (t) => {
  const directory = temporaryDirectory(t);
  createInputs(directory);
  fs.rmSync(path.join(directory, releaseLayout(version)[0].sbom));

  assert.throws(() => generate(directory), /release metadata inputs must be exactly/);
});

test("an unexpected release file is rejected", (t) => {
  const { directory } = preparedRelease(t);
  fs.writeFileSync(path.join(directory, "unreviewed.txt"), "unexpected\n");

  assert.throws(() => verifyReleaseMetadata(directory), /release files must be exactly/);
});

test("a signature bundle for another checksum file is rejected", (t) => {
  const { directory, generated } = preparedRelease(t);
  writeJson(directory, `${generated.checksumName}.sigstore.json`, {
    mediaType: bundleMediaType,
    verificationMaterial: { certificate: { rawBytes: "Zml4dHVyZQ==" } },
    messageSignature: {
      messageDigest: { algorithm: "SHA2_256", digest: Buffer.alloc(32).toString("base64") },
      signature: "Zml4dHVyZQ==",
    },
  });

  assert.throws(() => verifyReleaseMetadata(directory), /does not sign the checksum file/);
});

test("generation never overwrites existing release metadata", (t) => {
  const directory = temporaryDirectory(t);
  createInputs(directory);
  generate(directory);

  assert.throws(() => generate(directory), /release metadata inputs must be exactly/);
});

test("release metadata cannot drift from the source version", (t) => {
  const directory = temporaryDirectory(t);
  createInputs(directory);

  assert.throws(
    () => generateReleaseMetadata({
      directory,
      version: "0.4.0-rc.1",
      commit,
      minimumSupportedCliVersion: "0.3.0",
      sourceDateEpoch: 1_788_566_400,
      builderWorkflow: "stack-sh/cli/.github/workflows/release.yaml",
    }),
    /version must match the distribution contract and Cargo.toml/,
  );
});

test("the minimum supported version cannot be newer than the release", (t) => {
  const directory = temporaryDirectory(t);
  createInputs(directory);

  assert.throws(
    () => generateReleaseMetadata({
      directory,
      version,
      commit,
      minimumSupportedCliVersion: "0.4.0",
      sourceDateEpoch: 1_788_566_400,
      builderWorkflow: "stack-sh/cli/.github/workflows/release.yaml",
    }),
    /minimumSupportedCliVersion must not be newer than version/,
  );
});
