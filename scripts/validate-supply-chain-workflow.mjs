import fs from "node:fs";
import path from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const workflowFile = path.join(root, ".github", "workflows", "supply-chain.yaml");

const expectedActions = new Map([
  ["actions/checkout", "3d3c42e5aac5ba805825da76410c181273ba90b1"],
  ["anchore/sbom-action/download-syft", "3ad7283483fc7af8ff2b4ea19663c2d5ca935e26"],
  ["actions/attest", "1e69f48acb82d1966a394da916b4c1698aa569d6"],
  ["sigstore/cosign-installer", "6f9f17788090df1f26f669e9d70d6ae9567deba6"],
  ["actions/upload-artifact", "043fb46d1a93c77aae656e7c1c64a875d1fc6a0a"],
]);

function invariant(condition, message) {
  if (!condition) throw new Error(message);
}

export function validateSupplyChainWorkflow(source) {
  const workflow = source.replaceAll("\r\n", "\n");
  invariant(/^on:\n  workflow_dispatch:\n/m.test(workflow), "workflow must be manually dispatched");
  invariant(!/^  (?:pull_request|pull_request_target|push|schedule):/m.test(workflow), "workflow must not run on an automatic trigger");
  invariant(/^permissions: \{\}$/m.test(workflow), "top-level permissions must be empty");
  invariant(
    /^    if: github\.event_name == 'workflow_dispatch' && github\.ref == 'refs\/heads\/main'$/m.test(workflow),
    "the signing job must be restricted to a main-branch manual dispatch",
  );
  invariant(!/^\s+(?:contents|actions|packages|pull-requests): write$/m.test(workflow), "workflow grants an unnecessary write permission");
  const requiredPermissions = [
    "contents: read",
    "id-token: write",
    "attestations: write",
  ];
  const permissionBlock = workflow.match(/^    permissions:\n((?:      [a-z-]+: (?:read|write)\n)+)/m);
  invariant(permissionBlock, "the signing job must declare an explicit permission block");
  const actualPermissions = permissionBlock[1].trim().split("\n").map((line) => line.trim()).sort();
  invariant(
    JSON.stringify(actualPermissions) === JSON.stringify([...requiredPermissions].sort()),
    `job permissions must be exactly: ${requiredPermissions.join(", ")}`,
  );
  invariant(!/\$\{\{\s*secrets(?:\.|\[)/.test(workflow), "workflow must not use a long-lived secret");

  const uses = [...workflow.matchAll(/^\s+uses: ([^\s#]+)(?:\s+#.*)?$/gm)].map((match) => match[1]);
  invariant(uses.length > 0, "workflow must use pinned actions");
  for (const action of uses) {
    const match = action.match(/^([^@]+)@([0-9a-f]{40})$/);
    invariant(match, `action must be pinned to a full commit: ${action}`);
    const expected = expectedActions.get(match[1]);
    invariant(expected, `unexpected action: ${match[1]}`);
    invariant(match[2] === expected, `unexpected commit for ${match[1]}`);
  }
  for (const action of expectedActions.keys()) {
    invariant(uses.some((value) => value.startsWith(`${action}@`)), `required action is missing: ${action}`);
  }

  for (const requirement of [
    "syft-version: v1.51.1",
    "cosign-release: v3.1.3",
    "GH_TOKEN: ${{ github.token }}",
    "--certificate-oidc-issuer https://token.actions.githubusercontent.com",
    "--signer-workflow stack-sh/cli/.github/workflows/supply-chain.yaml",
    "--source-ref refs/heads/main",
    "--deny-self-hosted-runners",
    "Expected tampered artifact verification to fail",
    "Expected tampered checksum verification to fail",
  ]) {
    invariant(workflow.includes(requirement), `workflow security requirement is missing: ${requirement}`);
  }

  return { actions: uses.length, permissions: 3 };
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  try {
    const result = validateSupplyChainWorkflow(fs.readFileSync(workflowFile, "utf8"));
    console.log(`Validated ${result.actions} pinned actions and ${result.permissions} job permissions.`);
  } catch (error) {
    console.error(error instanceof Error ? error.message : error);
    process.exitCode = 1;
  }
}
