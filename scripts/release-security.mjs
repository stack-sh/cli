import crypto from "node:crypto";
import fs from "node:fs";
import path from "node:path";
import { isDeepStrictEqual } from "node:util";
import { fileURLToPath, pathToFileURL } from "node:url";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const contract = JSON.parse(
  fs.readFileSync(path.join(root, "distribution", "distribution-contract.json"), "utf8"),
);
const maximumJsonBytes = 16 * 1024 * 1024;
const versionPattern = /^[0-9]+\.[0-9]+\.[0-9]+(?:-rc\.[1-9][0-9]*)?$/;
const commitPattern = /^[0-9a-f]{40}$/;
const digestPattern = /^[0-9a-f]{64}$/;
const provenancePredicate = "https://slsa.dev/provenance/v1";
const sbomPredicate = "https://spdx.dev/Document/v2.3";
const bundleMediaType = "application/vnd.dev.sigstore.bundle.v0.3+json";

function invariant(condition, message) {
  if (!condition) throw new Error(message);
}

function exactKeys(value, keys, label) {
  invariant(value && typeof value === "object" && !Array.isArray(value), `${label} must be an object`);
  const actual = Object.keys(value).sort();
  const expected = [...keys].sort();
  invariant(JSON.stringify(actual) === JSON.stringify(expected), `${label} fields must be exactly: ${expected.join(", ")}`);
}

function sameValues(actual, expected, label) {
  invariant(new Set(actual).size === actual.length, `${label} contains duplicates`);
  invariant(
    JSON.stringify([...actual].sort()) === JSON.stringify([...expected].sort()),
    `${label} must be exactly: ${expected.join(", ")}`,
  );
}

function expand(template, version, target) {
  return template.replaceAll("{version}", version).replaceAll("{target}", target ?? "");
}

function assertVersion(version, label = "version") {
  invariant(versionPattern.test(version), `${label} must be a stable or rc Semantic Version`);
}

function readJson(file, label) {
  const stats = fs.statSync(file);
  invariant(stats.isFile(), `${label} must be a regular file`);
  invariant(stats.size > 0 && stats.size <= maximumJsonBytes, `${label} must be between 1 byte and 16 MiB`);
  return JSON.parse(fs.readFileSync(file, "utf8"));
}

export function sha256File(file) {
  const hash = crypto.createHash("sha256");
  const handle = fs.openSync(file, "r");
  const buffer = Buffer.allocUnsafe(1024 * 1024);
  try {
    let bytesRead;
    while ((bytesRead = fs.readSync(handle, buffer, 0, buffer.length, null)) > 0) {
      hash.update(buffer.subarray(0, bytesRead));
    }
  } finally {
    fs.closeSync(handle);
  }
  return hash.digest("hex");
}

function descriptor(directory, name) {
  const file = path.join(directory, name);
  invariant(fs.existsSync(file), `missing release material: ${name}`);
  invariant(fs.statSync(file).isFile(), `release material must be a regular file: ${name}`);
  return { name, sha256: sha256File(file) };
}

export function releaseLayout(version) {
  assertVersion(version);
  return contract.targets.map(({ target }) => ({
    target,
    archive: expand(contract.artifacts.archiveNameTemplate, version, target),
    sbom: expand(contract.artifacts.sbomNameTemplate, version, target),
    provenance: expand(contract.artifacts.provenanceNameTemplate, version, target),
    sbomAttestation: expand(contract.artifacts.sbomAttestationNameTemplate, version, target),
  }));
}

function validateSpdx(file, expectedName) {
  const document = readJson(file, `SBOM ${expectedName}`);
  invariant(document.spdxVersion === "SPDX-2.3", `${expectedName} must use SPDX 2.3`);
  invariant(document.dataLicense === "CC0-1.0", `${expectedName} must use the SPDX CC0-1.0 data license`);
  invariant(document.SPDXID === "SPDXRef-DOCUMENT", `${expectedName} must identify the SPDX document`);
  invariant(typeof document.name === "string" && document.name.length > 0, `${expectedName} must have a name`);
  invariant(
    typeof document.documentNamespace === "string" && document.documentNamespace.startsWith("https://"),
    `${expectedName} must have an HTTPS document namespace`,
  );
  invariant(
    Array.isArray(document.creationInfo?.creators) && document.creationInfo.creators.length > 0,
    `${expectedName} must identify its creator`,
  );
  invariant(Array.isArray(document.packages) && document.packages.length > 0, `${expectedName} must contain at least one package`);
  return document;
}

function decodeAttestation(file, expectedSubject, expectedDigest, expectedPredicate) {
  const bundle = readJson(file, `attestation ${path.basename(file)}`);
  invariant(bundle.mediaType === bundleMediaType, `${path.basename(file)} must use Sigstore bundle v0.3`);
  invariant(
    bundle.verificationMaterial && Object.keys(bundle.verificationMaterial).length > 0,
    `${path.basename(file)} must contain verification material`,
  );
  invariant(bundle.dsseEnvelope?.payloadType === "application/vnd.in-toto+json", `${path.basename(file)} must contain an in-toto DSSE payload`);
  invariant(
    Array.isArray(bundle.dsseEnvelope?.signatures) && bundle.dsseEnvelope.signatures.length > 0,
    `${path.basename(file)} must contain a DSSE signature`,
  );
  let statement;
  try {
    statement = JSON.parse(Buffer.from(bundle.dsseEnvelope.payload, "base64").toString("utf8"));
  } catch {
    throw new Error(`${path.basename(file)} contains an invalid DSSE payload`);
  }
  invariant(statement._type === "https://in-toto.io/Statement/v1", `${path.basename(file)} must use in-toto Statement v1`);
  invariant(statement.predicateType === expectedPredicate, `${path.basename(file)} has the wrong predicate type`);
  invariant(
    Array.isArray(statement.subject) && statement.subject.length === 1,
    `${path.basename(file)} must contain exactly one subject`,
  );
  const [subject] = statement.subject;
  invariant(subject.name === expectedSubject, `${path.basename(file)} does not attest ${expectedSubject}`);
  invariant(subject.digest?.sha256 === expectedDigest, `${path.basename(file)} subject digest does not match ${expectedSubject}`);
  invariant(
    statement.predicate && typeof statement.predicate === "object" && !Array.isArray(statement.predicate),
    `${path.basename(file)} must contain an object predicate`,
  );
  if (expectedPredicate === sbomPredicate) {
    invariant(statement.predicate?.spdxVersion === "SPDX-2.3", `${path.basename(file)} must attest an SPDX 2.3 predicate`);
  }
  return statement;
}

function validateChecksumSignature(file, checksumFile) {
  const bundle = readJson(file, `checksum signature ${path.basename(file)}`);
  invariant(bundle.mediaType === bundleMediaType, `${path.basename(file)} must use Sigstore bundle v0.3`);
  invariant(
    bundle.verificationMaterial && Object.keys(bundle.verificationMaterial).length > 0,
    `${path.basename(file)} must contain verification material`,
  );
  invariant(bundle.messageSignature?.messageDigest?.algorithm === "SHA2_256", `${path.basename(file)} must sign a SHA-256 digest`);
  const expected = Buffer.from(sha256File(checksumFile), "hex").toString("base64");
  invariant(bundle.messageSignature.messageDigest.digest === expected, `${path.basename(file)} does not sign the checksum file`);
  invariant(typeof bundle.messageSignature.signature === "string" && bundle.messageSignature.signature.length > 0, `${path.basename(file)} must contain a signature`);
}

function atomicCreate(file, contents) {
  invariant(!fs.existsSync(file), `refusing to replace existing release metadata: ${path.basename(file)}`);
  const temporary = `${file}.tmp-${process.pid}`;
  try {
    fs.writeFileSync(temporary, contents, { encoding: "utf8", flag: "wx", mode: 0o644 });
    fs.renameSync(temporary, file);
  } finally {
    fs.rmSync(temporary, { force: true });
  }
}

function regularFileNames(directory) {
  return fs.readdirSync(directory, { withFileTypes: true }).map((entry) => {
    invariant(entry.isFile(), `release directory contains a non-file entry: ${entry.name}`);
    return entry.name;
  });
}

function expectedInputNames(version) {
  return releaseLayout(version).flatMap(({ archive, sbom, provenance, sbomAttestation }) => [
    archive,
    sbom,
    provenance,
    sbomAttestation,
  ]);
}

function validateChannels(channels) {
  invariant(Array.isArray(channels), "verifiedChannels must be an array");
  const allowed = contract.channels.map(({ id }) => id);
  invariant(new Set(channels).size === channels.length, "verifiedChannels contains duplicates");
  for (const channel of channels) invariant(allowed.includes(channel), `unknown verified channel: ${channel}`);
}

function compareVersions(left, right) {
  const parse = (version) => {
    const [core, prerelease] = version.split("-rc.");
    return { core: core.split(".").map(Number), prerelease: prerelease ? Number(prerelease) : Infinity };
  };
  const leftVersion = parse(left);
  const rightVersion = parse(right);
  for (let index = 0; index < 3; index += 1) {
    if (leftVersion.core[index] !== rightVersion.core[index]) {
      return leftVersion.core[index] - rightVersion.core[index];
    }
  }
  if (leftVersion.prerelease === rightVersion.prerelease) return 0;
  if (leftVersion.prerelease === Infinity) return 1;
  if (rightVersion.prerelease === Infinity) return -1;
  return leftVersion.prerelease - rightVersion.prerelease;
}

export function generateReleaseMetadata({
  directory,
  version,
  commit,
  minimumSupportedCliVersion,
  sourceDateEpoch,
  builderWorkflow,
  verifiedChannels = [],
}) {
  assertVersion(version);
  assertVersion(minimumSupportedCliVersion, "minimumSupportedCliVersion");
  invariant(version === contract.product.currentSourceVersion, "version must match the distribution contract and Cargo.toml");
  invariant(compareVersions(minimumSupportedCliVersion, version) <= 0, "minimumSupportedCliVersion must not be newer than version");
  invariant(commitPattern.test(commit), "commit must be a full lowercase Git SHA");
  invariant(Number.isSafeInteger(sourceDateEpoch) && sourceDateEpoch >= 0, "sourceDateEpoch must be a non-negative integer");
  invariant(
    /^stack-sh\/cli\/\.github\/workflows\/[a-z0-9-]+\.yaml$/.test(builderWorkflow),
    "builderWorkflow must identify a stack-sh/cli workflow",
  );
  validateChannels(verifiedChannels);
  invariant(fs.statSync(directory).isDirectory(), "release directory must exist");
  sameValues(regularFileNames(directory), expectedInputNames(version), "release metadata inputs");

  const targets = releaseLayout(version).map((layout) => {
    const archive = descriptor(directory, layout.archive);
    const sbom = descriptor(directory, layout.sbom);
    const spdx = validateSpdx(path.join(directory, layout.sbom), layout.sbom);
    decodeAttestation(
      path.join(directory, layout.provenance),
      layout.archive,
      archive.sha256,
      provenancePredicate,
    );
    const sbomStatement = decodeAttestation(
      path.join(directory, layout.sbomAttestation),
      layout.archive,
      archive.sha256,
      sbomPredicate,
    );
    invariant(
      isDeepStrictEqual(sbomStatement.predicate, spdx),
      `${layout.sbomAttestation} does not attest the published SBOM`,
    );
    return {
      target: layout.target,
      archive,
      sbom,
      provenance: descriptor(directory, layout.provenance),
      sbomAttestation: descriptor(directory, layout.sbomAttestation),
    };
  });

  const manifest = {
    $schema: `https://raw.githubusercontent.com/stack-sh/cli/${commit}/distribution/release-manifest.schema.json`,
    schemaVersion: 1,
    version,
    tag: `v${version}`,
    source: { repository: "stack-sh/cli", commit },
    minimumSupportedCliVersion,
    sourceDateEpoch,
    builderWorkflow,
    verifiedChannels: [...verifiedChannels].sort(),
    targets,
  };
  const manifestName = expand(contract.artifacts.releaseManifestNameTemplate, version);
  atomicCreate(path.join(directory, manifestName), `${JSON.stringify(manifest, null, 2)}\n`);

  const checksumNames = [...expectedInputNames(version), manifestName].sort();
  const checksums = checksumNames
    .map((name) => `${sha256File(path.join(directory, name))}  ${name}`)
    .join("\n");
  const checksumName = expand(contract.artifacts.checksumNameTemplate, version);
  atomicCreate(path.join(directory, checksumName), `${checksums}\n`);
  return { manifestName, checksumName, targets: targets.length, checksums: checksumNames.length };
}

function validateDescriptor(value, directory, expectedName, label) {
  exactKeys(value, ["name", "sha256"], label);
  invariant(value.name === expectedName, `${label} has the wrong filename`);
  invariant(digestPattern.test(value.sha256), `${label} has an invalid SHA-256 digest`);
  invariant(sha256File(path.join(directory, value.name)) === value.sha256, `${label} checksum mismatch`);
}

function parseChecksums(contents) {
  const entries = new Map();
  const lines = contents.trimEnd().split("\n");
  for (const line of lines) {
    const match = line.match(/^([0-9a-f]{64})  ([A-Za-z0-9._-]+)$/);
    invariant(match, `invalid checksum line: ${line}`);
    invariant(!entries.has(match[2]), `duplicate checksum entry: ${match[2]}`);
    entries.set(match[2], match[1]);
  }
  return entries;
}

export function verifyReleaseMetadata(directory) {
  invariant(fs.statSync(directory).isDirectory(), "release directory must exist");
  const manifestCandidates = regularFileNames(directory).filter((name) => name.endsWith("-release-manifest.json"));
  invariant(manifestCandidates.length === 1, "release directory must contain exactly one release manifest");
  const manifest = readJson(path.join(directory, manifestCandidates[0]), "release manifest");
  exactKeys(
    manifest,
    [
      "$schema",
      "schemaVersion",
      "version",
      "tag",
      "source",
      "minimumSupportedCliVersion",
      "sourceDateEpoch",
      "builderWorkflow",
      "verifiedChannels",
      "targets",
    ],
    "release manifest",
  );
  assertVersion(manifest.version);
  assertVersion(manifest.minimumSupportedCliVersion, "minimumSupportedCliVersion");
  invariant(manifest.version === contract.product.currentSourceVersion, "release manifest version must match the distribution contract and Cargo.toml");
  invariant(
    compareVersions(manifest.minimumSupportedCliVersion, manifest.version) <= 0,
    "minimumSupportedCliVersion must not be newer than version",
  );
  invariant(
    manifest.$schema === `https://raw.githubusercontent.com/stack-sh/cli/${manifest.source.commit}/distribution/release-manifest.schema.json`,
    "release manifest has the wrong schema reference",
  );
  invariant(manifest.schemaVersion === 1, "release manifest schemaVersion must be 1");
  invariant(manifest.tag === `v${manifest.version}`, "release manifest tag must match version");
  exactKeys(manifest.source, ["repository", "commit"], "release manifest source");
  invariant(manifest.source.repository === "stack-sh/cli", "release manifest source repository is invalid");
  invariant(commitPattern.test(manifest.source.commit), "release manifest source commit is invalid");
  invariant(Number.isSafeInteger(manifest.sourceDateEpoch) && manifest.sourceDateEpoch >= 0, "sourceDateEpoch is invalid");
  invariant(
    /^stack-sh\/cli\/\.github\/workflows\/[a-z0-9-]+\.yaml$/.test(manifest.builderWorkflow),
    "builderWorkflow is invalid",
  );
  validateChannels(manifest.verifiedChannels);
  invariant(
    JSON.stringify(manifest.verifiedChannels) === JSON.stringify([...manifest.verifiedChannels].sort()),
    "verifiedChannels must be sorted",
  );

  const layout = releaseLayout(manifest.version);
  sameValues(
    manifest.targets.map(({ target }) => target),
    layout.map(({ target }) => target),
    "release manifest targets",
  );
  for (const expected of layout) {
    const target = manifest.targets.find(({ target: id }) => id === expected.target);
    exactKeys(target, ["target", "archive", "sbom", "provenance", "sbomAttestation"], `target ${expected.target}`);
    validateDescriptor(target.archive, directory, expected.archive, `${expected.target} archive`);
    validateDescriptor(target.sbom, directory, expected.sbom, `${expected.target} SBOM`);
    validateDescriptor(target.provenance, directory, expected.provenance, `${expected.target} provenance`);
    validateDescriptor(
      target.sbomAttestation,
      directory,
      expected.sbomAttestation,
      `${expected.target} SBOM attestation`,
    );
    const spdx = validateSpdx(path.join(directory, expected.sbom), expected.sbom);
    decodeAttestation(
      path.join(directory, expected.provenance),
      expected.archive,
      target.archive.sha256,
      provenancePredicate,
    );
    const sbomStatement = decodeAttestation(
      path.join(directory, expected.sbomAttestation),
      expected.archive,
      target.archive.sha256,
      sbomPredicate,
    );
    invariant(
      isDeepStrictEqual(sbomStatement.predicate, spdx),
      `${expected.sbomAttestation} does not attest the published SBOM`,
    );
  }

  const manifestName = expand(contract.artifacts.releaseManifestNameTemplate, manifest.version);
  invariant(manifestCandidates[0] === manifestName, "release manifest filename does not match its version");
  const checksumName = expand(contract.artifacts.checksumNameTemplate, manifest.version);
  const signatureName = expand(contract.artifacts.signatureBundleNameTemplate, manifest.version);
  const expectedChecksums = [...expectedInputNames(manifest.version), manifestName].sort();
  const expectedFiles = [...expectedChecksums, checksumName, signatureName];
  sameValues(regularFileNames(directory), expectedFiles, "release files");

  const checksumFile = path.join(directory, checksumName);
  const entries = parseChecksums(fs.readFileSync(checksumFile, "utf8"));
  sameValues([...entries.keys()], expectedChecksums, "checksum entries");
  for (const [name, digest] of entries) {
    invariant(sha256File(path.join(directory, name)) === digest, `checksum mismatch: ${name}`);
  }
  validateChecksumSignature(path.join(directory, signatureName), checksumFile);
  return { version: manifest.version, targets: layout.length, checksums: entries.size };
}

function parseOptions(arguments_, allowed) {
  const options = {};
  for (let index = 0; index < arguments_.length; index += 2) {
    const flag = arguments_[index];
    const value = arguments_[index + 1];
    invariant(flag?.startsWith("--") && value !== undefined, `invalid option: ${flag ?? ""}`);
    const name = flag.slice(2);
    invariant(allowed.includes(name), `unknown option: ${flag}`);
    invariant(options[name] === undefined, `duplicate option: ${flag}`);
    options[name] = value;
  }
  return options;
}

function requireOptions(options, required) {
  for (const name of required) invariant(options[name] !== undefined, `missing required option: --${name}`);
}

function run() {
  const command = process.argv[2];
  if (command === "generate") {
    const required = [
      "directory",
      "version",
      "commit",
      "minimum-supported-version",
      "source-date-epoch",
      "builder-workflow",
    ];
    const options = parseOptions(process.argv.slice(3), [...required, "verified-channels"]);
    requireOptions(options, required);
    const result = generateReleaseMetadata({
      directory: path.resolve(options.directory),
      version: options.version,
      commit: options.commit,
      minimumSupportedCliVersion: options["minimum-supported-version"],
      sourceDateEpoch: Number(options["source-date-epoch"]),
      builderWorkflow: options["builder-workflow"],
      verifiedChannels: options["verified-channels"] ? options["verified-channels"].split(",") : [],
    });
    console.log(`Generated ${result.checksums} checksums for ${result.targets} targets.`);
    return;
  }
  if (command === "verify") {
    const options = parseOptions(process.argv.slice(3), ["directory"]);
    requireOptions(options, ["directory"]);
    const result = verifyReleaseMetadata(path.resolve(options.directory));
    console.log(`Verified ${result.checksums} checksums for ${result.targets} targets at ${result.version}.`);
    return;
  }
  throw new Error("usage: release-security.mjs <generate|verify> --directory <path> [generate options]");
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  try {
    run();
  } catch (error) {
    console.error(error instanceof Error ? error.message : error);
    process.exitCode = 1;
  }
}
