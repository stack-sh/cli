import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";

import { resolveReleaseContext } from "./resolve-release-context.mjs";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const cargoToml = fs.readFileSync(path.join(root, "Cargo.toml"), "utf8");
const contract = JSON.parse(
  fs.readFileSync(path.join(root, "distribution", "distribution-contract.json"), "utf8"),
);
const sha = "0123456789abcdef0123456789abcdef01234567";

test("main dispatch resolves a non-publishing verification run", () => {
  assert.deepEqual(
    resolveReleaseContext({
      eventName: "workflow_dispatch",
      ref: "refs/heads/main",
      refName: "main",
      sha,
      requestedVersion: "0.4.0",
      cargoToml,
      contract,
    }),
    {
      version: "0.4.0",
      tag: "v0.4.0",
      sourceRef: "refs/heads/main",
      publish: false,
      verifiedChannels: "",
      minimumSupportedCliVersion: "0.4.0",
    },
  );
});

test("an exact version tag resolves a publishing run", () => {
  assert.deepEqual(
    resolveReleaseContext({
      eventName: "push",
      ref: "refs/tags/v0.4.0",
      refName: "v0.4.0",
      sha,
      requestedVersion: "",
      cargoToml,
      contract,
    }),
    {
      version: "0.4.0",
      tag: "v0.4.0",
      sourceRef: "refs/tags/v0.4.0",
      publish: true,
      verifiedChannels: "github-release",
      minimumSupportedCliVersion: "0.4.0",
    },
  );
});

test("an activated self-update channel is recorded in a tagged release", () => {
  const activated = structuredClone(contract);
  const selfUpdate = activated.channels.find(({ id }) => id === "self-update");
  selfUpdate.state = "available";
  selfUpdate.minimumSupportedCliVersion = "0.2.1";
  assert.deepEqual(
    resolveReleaseContext({
      eventName: "push",
      ref: "refs/tags/v0.4.0",
      refName: "v0.4.0",
      sha,
      requestedVersion: "",
      cargoToml,
      contract: activated,
    }),
    {
      version: "0.4.0",
      tag: "v0.4.0",
      sourceRef: "refs/tags/v0.4.0",
      publish: true,
      verifiedChannels: "github-release,self-update",
      minimumSupportedCliVersion: "0.2.1",
    },
  );
});

test("self-update activation rejects a missing or future compatibility floor", () => {
  for (const floor of [null, "0.5.0", "invalid"]) {
    const activated = structuredClone(contract);
    const selfUpdate = activated.channels.find(({ id }) => id === "self-update");
    selfUpdate.state = "available";
    selfUpdate.minimumSupportedCliVersion = floor;
    assert.throws(
      () => resolveReleaseContext({
        eventName: "push",
        ref: "refs/tags/v0.4.0",
        refName: "v0.4.0",
        sha,
        requestedVersion: "",
        cargoToml,
        contract: activated,
      }),
      /minimum supported CLI version|invalid updater compatibility version/,
    );
  }
});

test("manual runs from another ref or version are rejected", () => {
  const common = {
    eventName: "workflow_dispatch",
    refName: "main",
    sha,
    cargoToml,
    contract,
  };
  assert.throws(
    () => resolveReleaseContext({ ...common, ref: "refs/heads/topic", requestedVersion: "0.4.0" }),
    /must run from main/,
  );
  assert.throws(
    () => resolveReleaseContext({ ...common, ref: "refs/heads/main", requestedVersion: "0.5.0" }),
    /must match Cargo.toml/,
  );
});

test("floating and mismatched tags are rejected", () => {
  for (const ref of ["refs/tags/latest", "refs/tags/v0.4", "refs/tags/v0.5.0"]) {
    assert.throws(
      () => resolveReleaseContext({
        eventName: "push",
        ref,
        refName: ref.slice("refs/tags/".length),
        sha,
        requestedVersion: "",
        cargoToml,
        contract,
      }),
      /tag must exactly match/,
    );
  }
});

test("source and contract version drift is rejected", () => {
  const drifted = structuredClone(contract);
  drifted.product.currentSourceVersion = "0.5.0";
  assert.throws(
    () => resolveReleaseContext({
      eventName: "workflow_dispatch",
      ref: "refs/heads/main",
      refName: "main",
      sha,
      requestedVersion: "0.4.0",
      cargoToml,
      contract: drifted,
    }),
    /contract version does not match/,
  );
});
