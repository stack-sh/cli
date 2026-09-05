import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import test from "node:test";
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
    channels: 5,
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

test("an unverified crates.io package name is rejected", () => {
  const candidate = changed((value) => {
    value.product.publishedCargoPackage = "stack-cli";
  });
  assert.throws(() => validateDistributionContract(candidate, cargoToml), /must remain unset/);
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

test("an unactivated package-manager channel cannot become available", () => {
  const candidate = changed((value) => {
    value.channels.find(({ id }) => id === "cargo").state = "available";
  });
  assert.throws(() => validateDistributionContract(candidate, cargoToml), /cargo state must be planned/);
});
