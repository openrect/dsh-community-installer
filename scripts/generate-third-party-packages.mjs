import { createHash } from "node:crypto";
import { existsSync, readFileSync, writeFileSync } from "node:fs";
import { join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const root = resolve(fileURLToPath(new URL("..", import.meta.url)));
const lockPath = join(root, "payload", "package-lock.json");
const outputPath = join(root, "THIRD_PARTY_PACKAGES.txt");
const lockBytes = readFileSync(lockPath);
const lock = JSON.parse(lockBytes.toString("utf8"));
const packages = Object.entries(lock.packages)
  .filter(([packagePath]) => packagePath.startsWith("node_modules/"))
  .map(([packagePath, metadata]) => {
    if (!metadata.version || !metadata.license) throw new Error(`Package metadata is incomplete: ${packagePath}`);
    return { path: packagePath, version: metadata.version, license: metadata.license, source: metadata.resolved ?? "" };
  })
  .sort((left, right) => left.path.localeCompare(right.path, "en"));
const output = [
  "DSH Community Installer locked third-party package inventory",
  "========================================================",
  "",
  `package-lock.json SHA-256: ${createHash("sha256").update(lockBytes).digest("hex")}`,
  `Package count: ${packages.length}`,
  "",
  "Format: package path | version | SPDX license | source",
  "",
  ...packages.map((item) => `${item.path} | ${item.version} | ${item.license} | ${item.source}`),
  "",
].join("\n");
if (!existsSync(outputPath) || readFileSync(outputPath, "utf8") !== output) writeFileSync(outputPath, output, "utf8");
process.stdout.write(`Third-party package inventory: ${outputPath}\nPackages: ${packages.length}\n`);
