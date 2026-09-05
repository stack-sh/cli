import fs from "node:fs";
import path from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");

function packageVersion() {
  const cargoToml = fs.readFileSync(path.join(root, "Cargo.toml"), "utf8");
  const match = cargoToml.match(/^version\s*=\s*"([^"]+)"/m);
  if (!match) throw new Error("Cargo.toml package version is missing");
  return match[1];
}

export function createSmokeFixture(directory) {
  if (!directory) throw new Error("an output directory is required");
  fs.mkdirSync(directory, { recursive: true });
  const name = `stack-v${packageVersion()}-supply-chain-smoke.bin`;
  const file = path.join(directory, name);
  if (fs.existsSync(file)) throw new Error(`refusing to replace smoke fixture: ${name}`);
  const source = process.env.GITHUB_SHA ?? "local";
  fs.writeFileSync(file, `Stack supply-chain smoke fixture\nsource=${source}\n`, {
    encoding: "utf8",
    flag: "wx",
    mode: 0o644,
  });
  return file;
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  try {
    console.log(createSmokeFixture(path.resolve(process.argv[2])));
  } catch (error) {
    console.error(error instanceof Error ? error.message : error);
    process.exitCode = 1;
  }
}
