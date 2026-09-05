import fs from "node:fs";
import path from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const versionPattern = /^[0-9]+\.[0-9]+\.[0-9]+(?:-rc\.[1-9][0-9]*)?$/;
const commitPattern = /^[0-9a-f]{40}$/;

function invariant(condition, message) {
  if (!condition) throw new Error(message);
}

function cargoVersion(cargoToml) {
  const match = cargoToml.match(/^version\s*=\s*"([^"]+)"/m);
  invariant(match, "Cargo.toml package version is missing");
  return match[1];
}

export function resolveReleaseContext({ eventName, ref, refName, sha, requestedVersion, cargoToml, contract }) {
  const version = cargoVersion(cargoToml);
  invariant(versionPattern.test(version), "Cargo.toml version is not a supported release version");
  invariant(contract.product?.currentSourceVersion === version, "distribution contract version does not match Cargo.toml");
  invariant(commitPattern.test(sha), "release source must be a full lowercase Git SHA");

  if (eventName === "workflow_dispatch") {
    invariant(ref === "refs/heads/main", "manual release verification must run from main");
    invariant(requestedVersion === version, "requested version must match Cargo.toml");
    return {
      version,
      tag: `v${version}`,
      sourceRef: "refs/heads/main",
      publish: false,
      verifiedChannels: "",
    };
  }

  invariant(eventName === "push", `unsupported release event: ${eventName}`);
  invariant(ref === `refs/tags/v${version}`, "release tag must exactly match Cargo.toml version");
  invariant(refName === `v${version}`, "release ref name must exactly match Cargo.toml version");
  return {
    version,
    tag: refName,
    sourceRef: ref,
    publish: true,
    verifiedChannels: "github-release",
  };
}

function run() {
  const context = resolveReleaseContext({
    eventName: process.env.GITHUB_EVENT_NAME,
    ref: process.env.GITHUB_REF,
    refName: process.env.GITHUB_REF_NAME,
    sha: process.env.GITHUB_SHA,
    requestedVersion: process.env.RELEASE_VERSION ?? "",
    cargoToml: fs.readFileSync(path.join(root, "Cargo.toml"), "utf8"),
    contract: JSON.parse(fs.readFileSync(path.join(root, "distribution", "distribution-contract.json"), "utf8")),
  });
  for (const [name, value] of Object.entries({
    version: context.version,
    tag: context.tag,
    "source-ref": context.sourceRef,
    publish: String(context.publish),
    "verified-channels": context.verifiedChannels,
  })) {
    console.log(`${name}=${value}`);
  }
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  try {
    run();
  } catch (error) {
    console.error(error instanceof Error ? error.message : error);
    process.exitCode = 1;
  }
}
