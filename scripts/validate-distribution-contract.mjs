import fs from "node:fs";
import path from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const contractPath = path.join(root, "distribution", "distribution-contract.json");

const expectedTargets = [
  "aarch64-apple-darwin",
  "aarch64-unknown-linux-gnu",
  "x86_64-apple-darwin",
  "x86_64-unknown-linux-gnu",
];
const expectedChannels = ["aqua", "cargo", "github-release", "homebrew", "self-update"];
const availableChannels = new Set(["aqua", "github-release", "homebrew"]);
const requiredArchiveEntries = ["LICENSE", "NOTICE", "THIRD_PARTY_LICENSES.md", "stack"];
const requiredUnsupportedTerms = ["32-bit", "BSD", "Windows", "musl"];
const requiredActivationTerms = ["Cargo package version", "SBOMs", "provenance", "stack --version"];
const targetDefinitions = new Map([
  ["aarch64-apple-darwin", ["macos", "arm64", "system"]],
  ["x86_64-apple-darwin", ["macos", "x86_64", "system"]],
  ["aarch64-unknown-linux-gnu", ["linux", "arm64", "glibc"]],
  ["x86_64-unknown-linux-gnu", ["linux", "x86_64", "glibc"]],
]);

function invariant(condition, message) {
  if (!condition) throw new Error(message);
}

function uniqueSorted(values, label) {
  invariant(new Set(values).size === values.length, `${label} contains duplicates`);
  return [...values].sort();
}

function sameValues(actual, expected, label) {
  invariant(
    JSON.stringify(uniqueSorted(actual, label)) === JSON.stringify([...expected].sort()),
    `${label} must be exactly: ${expected.join(", ")}`,
  );
}

function cargoValue(cargoToml, field) {
  const packageSection = cargoToml.match(/\[package\]([\s\S]*?)(?:\n\[|$)/)?.[1] ?? "";
  return packageSection.match(new RegExp(`^${field}\\s*=\\s*"([^"]+)"`, "m"))?.[1];
}

export function validateDistributionContract(contract, cargoToml) {
  invariant(contract.schemaVersion === 1, "schemaVersion must be 1");
  invariant(contract.product?.binary === "stack", "binary must be stack");
  invariant(contract.product?.sourceCargoPackage === "stack-cli", "source Cargo package must be stack-cli");
  invariant(
    contract.product.sourceCargoPackage === cargoValue(cargoToml, "name"),
    "sourceCargoPackage must match Cargo.toml",
  );
  invariant(contract.product?.publishedCargoPackage === null, "published Cargo package must remain unset before registry ownership is verified");
  invariant(contract.availability?.state === "available", "distribution must be available after the verified stable release");
  invariant(
    contract.availability?.message?.includes("Stack CLI 0.3.0") &&
      contract.availability.message.includes("GitHub Releases") &&
      contract.availability.message.includes("Homebrew") &&
      contract.availability.message.includes("Aqua"),
    "availability message must identify the verified stable GitHub release, Homebrew, and Aqua channels",
  );
  invariant(
    contract.product.currentSourceVersion === cargoValue(cargoToml, "version"),
    "currentSourceVersion must match Cargo.toml",
  );
  invariant(
    contract.product.minimumRustVersion === cargoValue(cargoToml, "rust-version"),
    "minimumRustVersion must match Cargo.toml",
  );

  invariant(Array.isArray(contract.targets), "targets must be an array");
  sameValues(
    contract.targets.map(({ target }) => target),
    expectedTargets,
    "target matrix",
  );
  for (const target of contract.targets) {
    const expected = targetDefinitions.get(target.target);
    invariant(
      JSON.stringify([target.os, target.architecture, target.libc]) === JSON.stringify(expected),
      `${target.target} has inconsistent OS, architecture, or libc metadata`,
    );
    invariant(target.supportTier === "tier-1", `${target.target} must be tier-1`);
    invariant(target.state === "available", `${target.target} must be available after release verification`);
    invariant(target.minimumRuntime, `${target.target} must declare a runtime floor`);
    if (target.os === "linux") invariant(target.libc === "glibc", `${target.target} must use glibc`);
    if (target.os === "macos") invariant(target.libc === "system", `${target.target} must use the system libc`);
  }

  invariant(Array.isArray(contract.channels), "channels must be an array");
  sameValues(
    contract.channels.map(({ id }) => id),
    expectedChannels,
    "channel set",
  );
  const targetIds = new Set(expectedTargets);
  for (const channel of contract.channels) {
    const expectedState = availableChannels.has(channel.id) ? "available" : "planned";
    invariant(channel.state === expectedState, `${channel.id} state must be ${expectedState}`);
    uniqueSorted(channel.targets, `${channel.id} targets`);
    for (const target of channel.targets) {
      invariant(targetIds.has(target), `${channel.id} references unknown target ${target}`);
    }
    invariant(channel.owns && channel.source && channel.updatePolicy, `${channel.id} must define ownership, source, and updates`);
  }
  const githubTargets = contract.channels.find(({ id }) => id === "github-release")?.targets ?? [];
  const cargoTargets = contract.channels.find(({ id }) => id === "cargo")?.targets ?? [];
  const aquaTargets = contract.channels.find(({ id }) => id === "aqua")?.targets ?? [];
  const updateTargets = contract.channels.find(({ id }) => id === "self-update")?.targets ?? [];
  const homebrewTargets = contract.channels.find(({ id }) => id === "homebrew")?.targets ?? [];
  const channels = new Map(contract.channels.map((channel) => [channel.id, channel]));
  sameValues(githubTargets, expectedTargets, "github-release targets");
  sameValues(cargoTargets, expectedTargets, "cargo targets");
  sameValues(aquaTargets, expectedTargets, "aqua targets");
  sameValues(updateTargets, expectedTargets, "self-update targets");
  sameValues(
    homebrewTargets,
    ["aarch64-apple-darwin", "aarch64-unknown-linux-gnu", "x86_64-unknown-linux-gnu"],
    "homebrew targets",
  );
  invariant(channels.get("github-release")?.source === "tagged stack-sh/cli source", "GitHub releases must build tagged source");
  invariant(channels.get("cargo")?.source === "crates.io", "Cargo must install from crates.io");
  for (const id of ["homebrew", "aqua", "self-update"]) {
    invariant(channels.get(id)?.source === "github-release", `${id} must consume GitHub releases`);
  }
  for (const id of ["homebrew", "cargo", "aqua"]) {
    invariant(
      channels.get(id)?.updatePolicy.includes("self-update must refuse replacement"),
      `${id} must own upgrades instead of self-update`,
    );
  }
  invariant(
    channels.get("self-update")?.updatePolicy.includes("refuse without a direct-install receipt"),
    "self-update must require a direct-install receipt",
  );
  invariant(
    channels.get("self-update")?.minimumSupportedCliVersion === null,
    "planned self-update must not claim a minimum supported CLI version",
  );
  for (const id of ["github-release", "homebrew", "cargo", "aqua"]) {
    invariant(
      !("minimumSupportedCliVersion" in channels.get(id)),
      `${id} must not own the self-update compatibility floor`,
    );
  }

  for (const [name, template] of Object.entries(contract.artifacts ?? {})) {
    if (!name.endsWith("Template")) continue;
    invariant(template.includes("{version}"), `${name} must contain {version}`);
  }
  for (const name of [
    "archiveNameTemplate",
    "archiveRootTemplate",
    "sbomNameTemplate",
    "provenanceNameTemplate",
    "sbomAttestationNameTemplate",
  ]) {
    invariant(contract.artifacts?.[name]?.includes("{target}"), `${name} must contain {target}`);
  }
  sameValues(contract.artifacts?.requiredEntries ?? [], requiredArchiveEntries, "archive entries");
  invariant(contract.artifacts?.checksumAlgorithm === "sha256", "checksum algorithm must be sha256");
  invariant(
    contract.artifacts?.installReceiptSchema === "distribution/install-receipt.schema.json",
    "install receipt schema path is invalid",
  );
  invariant(contract.artifacts?.signatureBundleNameTemplate?.endsWith(".sigstore.json"), "signature bundle must use .sigstore.json");
  invariant(contract.artifacts?.sbomNameTemplate?.endsWith(".spdx.json"), "SBOM must use .spdx.json");
  invariant(
    contract.artifacts?.provenanceNameTemplate?.endsWith(".provenance.sigstore.json"),
    "provenance must use a Sigstore bundle",
  );
  invariant(
    contract.artifacts?.sbomAttestationNameTemplate?.endsWith(".sbom.sigstore.json"),
    "SBOM attestation must use a Sigstore bundle",
  );
  invariant(contract.artifacts?.reproducibility?.uid === 0, "archive uid must be zero");
  invariant(contract.artifacts?.reproducibility?.gid === 0, "archive gid must be zero");
  invariant(contract.artifacts?.reproducibility?.mtime === "SOURCE_DATE_EPOCH", "archive mtime must use SOURCE_DATE_EPOCH");

  invariant(contract.versioning?.tagTemplate === "v{version}", "tag template must be v{version}");
  invariant(contract.versioning?.minimumSupportedVersionSource?.includes("minimumSupportedCliVersion"), "minimum version source must be explicit");

  const unsupported = (contract.unsupported ?? []).map(({ platform }) => platform).join(" ");
  for (const term of requiredUnsupportedTerms) {
    invariant(unsupported.includes(term), `unsupported platforms must mention ${term}`);
  }
  const activation = (contract.verification?.releaseActivation ?? []).join(" ");
  for (const term of requiredActivationTerms) {
    invariant(activation.includes(term), `release activation must mention ${term}`);
  }
  const selfUpdateActivation = (contract.verification?.selfUpdateActivation ?? []).join(" ");
  for (const term of ["authenticated release manifest", "direct installer", "tampered material", "atomic replacement", "rollback"]) {
    invariant(selfUpdateActivation.includes(term), `self-update activation must mention ${term}`);
  }
  invariant(contract.verification?.rollback?.includes("Never replace"), "rollback must preserve immutable releases");

  return {
    targets: contract.targets.length,
    channels: contract.channels.length,
  };
}

export function loadAndValidateDistributionContract(file = contractPath) {
  const contract = JSON.parse(fs.readFileSync(file, "utf8"));
  const cargoToml = fs.readFileSync(path.join(root, "Cargo.toml"), "utf8");
  return validateDistributionContract(contract, cargoToml);
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  try {
    const result = loadAndValidateDistributionContract(process.argv[2]);
    console.log(`Validated ${result.targets} distribution targets and ${result.channels} channels.`);
  } catch (error) {
    console.error(error instanceof Error ? error.message : error);
    process.exitCode = 1;
  }
}
