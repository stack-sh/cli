import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import test from "node:test";
import Ajv2020 from "ajv/dist/2020.js";
import { fileURLToPath } from "node:url";

import { validateDistributionContract } from "./validate-distribution-contract.mjs";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const contract = JSON.parse(
  fs.readFileSync(path.join(root, "distribution", "distribution-contract.json"), "utf8"),
);
const cargoToml = fs.readFileSync(path.join(root, "Cargo.toml"), "utf8");

function changed(change) {
  const copy = structuredClone(contract);
  change(copy);
  return copy;
}

test("the checked-in distribution contract is valid", () => {
  assert.deepEqual(validateDistributionContract(contract, cargoToml), {
    targets: 4,
    channels: 4,
  });
});

test("an unknown channel target is rejected", () => {
  const candidate = changed((value) => value.channels[0].targets.push("x86_64-pc-windows-msvc"));
  assert.throws(
    () => validateDistributionContract(candidate, cargoToml),
    /references unknown target x86_64-pc-windows-msvc/,
  );
});

test("source version drift is rejected", () => {
  const candidate = changed((value) => {
    value.product.currentSourceVersion = "0.2.0";
  });
  assert.throws(() => validateDistributionContract(candidate, cargoToml), /must match Cargo.toml/);
});

test("a stale stable release in the availability message is rejected", () => {
  const candidate = changed((value) => {
    value.availability.message = value.availability.message.replace(
      `Stack CLI ${value.product.currentReleaseVersion}`,
      "Stack CLI 0.0.0",
    );
  });
  assert.throws(
    () => validateDistributionContract(candidate, cargoToml),
    /must identify the verified stable GitHub release/,
  );
});

test("release preparation preserves the last verified distribution version", () => {
  const candidate = changed(value => { value.product.currentSourceVersion = "0.99.0"; });
  const futureSource = cargoToml.replace(/^version = "[^"]+"/m, 'version = "0.99.0"');
  assert.notEqual(candidate.product.currentSourceVersion, candidate.product.currentReleaseVersion);
  assert.deepEqual(validateDistributionContract(candidate, futureSource), { targets: 4, channels: 4 });
});

test("an unverified crates.io package name is rejected", () => {
  const candidate = changed((value) => {
    value.product.publishedCargoPackage = "stack-cli";
  });
  assert.throws(() => validateDistributionContract(candidate, cargoToml), /must be stack-diagram-cli/);
});

test("an incomplete archive contract is rejected", () => {
  const candidate = changed((value) => value.artifacts.requiredEntries.pop());
  assert.throws(() => validateDistributionContract(candidate, cargoToml), /archive entries must be exactly/);
});

test("package-manager ownership cannot be delegated to self-update", () => {
  const candidate = changed((value) => {
    value.channels.find(({ id }) => id === "homebrew").updatePolicy = "stack replaces the binary";
  });
  assert.throws(() => validateDistributionContract(candidate, cargoToml), /must own upgrades/);
});

test("the GitHub release cannot be activated with a planned target", () => {
  const candidate = changed((value) => {
    value.targets[0].state = "planned";
  });
  assert.throws(() => validateDistributionContract(candidate, cargoToml), /must be available after release verification/);
});

test("the activated Homebrew channel cannot regress to planned", () => {
  const candidate = changed((value) => {
    value.channels.find(({ id }) => id === "homebrew").state = "planned";
  });
  assert.throws(() => validateDistributionContract(candidate, cargoToml), /homebrew state must be available/);
});

test("the activated Aqua channel cannot regress to planned", () => {
  const candidate = changed((value) => {
    value.channels.find(({ id }) => id === "aqua").state = "planned";
  });
  assert.throws(() => validateDistributionContract(candidate, cargoToml), /aqua state must be available/);
});

test("the activated Cargo channel cannot regress to planned", () => {
  const candidate = changed((value) => {
    value.channels.find(({ id }) => id === "cargo").state = "planned";
  });
  assert.throws(() => validateDistributionContract(candidate, cargoToml), /cargo state must be available/);
});

test("removed self-update cannot be reintroduced", () => {
  const candidate = changed(value => value.channels.push({ id: "self-update", state: "available" }));
  assert.throws(() => validateDistributionContract(candidate, cargoToml), /channel set must be exactly/);
});

test("distribution v3 schema rejects removed updater fields", () => {
  const schema = JSON.parse(fs.readFileSync(path.join(root, "distribution/distribution-contract-v3.schema.json"), "utf8"));
  const validate = new Ajv2020({ strict: false }).compile(schema);
  assert.equal(validate(contract), true, JSON.stringify(validate.errors));
  for (const mutate of [value => value.artifacts.installReceiptSchema = "distribution/install-receipt.schema.json", value => value.verification.selfUpdateActivation = ["obsolete"], value => value.channels[0].minimumSupportedCliVersion = "0.4.0"]) {
    assert.equal(validate(changed(mutate)), false);
  }
});
