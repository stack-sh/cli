import { readFileSync, mkdirSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { validateVersion } from "./distribution-smoke-context.mjs";

const [version, destination] = process.argv.slice(2);
validateVersion(version);
mkdirSync(destination, { recursive: false });
const config = readFileSync("tests/aqua/aqua.yaml", "utf8");
writeFileSync(join(destination, "aqua.yaml"), config.replace(/stack-sh\/cli@v\d+\.\d+\.\d+/, `stack-sh/cli@v${version}`));
writeFileSync(join(destination, "aqua-policy.yaml"), readFileSync("tests/aqua/aqua-policy.yaml"));
