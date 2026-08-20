import { readFileSync } from "node:fs";
import { join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const root = resolve(fileURLToPath(new URL("..", import.meta.url)));
const build = JSON.parse(readFileSync(join(root, "build-config.json"), "utf8"));
const packageManifest = JSON.parse(readFileSync(join(root, "package.json"), "utf8"));
const tauri = JSON.parse(readFileSync(join(root, "src-tauri", "tauri.conf.json"), "utf8"));
const cargo = readFileSync(join(root, "src-tauri", "Cargo.toml"), "utf8");
const cargoVersion = cargo.match(/^version = "([^"]+)"$/m)?.[1];
const bridge = readFileSync(join(root, "frontend", "bridge.ts"), "utf8");
const payload = JSON.parse(readFileSync(join(root, "payload", "package.json"), "utf8"));
const lock = JSON.parse(readFileSync(join(root, "payload", "package-lock.json"), "utf8"));
const buildScript = readFileSync(join(root, "src-tauri", "build.rs"), "utf8");
const readmes = ["README.md", "README.zh-CN.md"];

const versions = new Map([
  ["build-config.json", build.installerVersion],
  ["package.json", packageManifest.version],
  ["src-tauri/tauri.conf.json", tauri.version],
  ["src-tauri/Cargo.toml", cargoVersion],
]);
for (const [source, version] of versions) {
  if (version !== build.installerVersion) {
    throw new Error(`${source} has version ${version ?? "<missing>"}; expected ${build.installerVersion}.`);
  }
}
if (!bridge.includes(`appVersion: "${build.installerVersion}"`)) {
  throw new Error("The frontend preview version does not match build-config.json.");
}
if (!bridge.includes(`dshVersion: "${build.dshVersion}"`)
    || !bridge.includes(`nodeVersion: "${build.nodeVersion}"`)) {
  throw new Error("The frontend preview runtime versions do not match build-config.json.");
}
if (payload.dependencies?.["@deepseek-ai/dsh"] !== build.dshVersion
    || lock.packages?.["node_modules/@deepseek-ai/dsh"]?.version !== build.dshVersion) {
  throw new Error(`The payload does not pin @deepseek-ai/dsh ${build.dshVersion}.`);
}
for (const environment of ["DSH_NODE_VERSION", "DSH_NODE_ARCHIVE_SHA256", "DSH_UPSTREAM_VERSION", "DSH_RUNTIME_ARCHITECTURE"]) {
  if (!buildScript.includes(environment)) {
    throw new Error(`src-tauri/build.rs does not export ${environment} from build-config.json.`);
  }
}
for (const readme of readmes) {
  const contents = readFileSync(join(root, readme), "utf8");
  if (!contents.includes(`/v${build.installerVersion}/DSH-Community-Setup-${build.installerVersion}-Windows-x64.exe`)
      || !contents.includes(`/v${build.installerVersion}/DSH-Community-Offline-Setup-${build.installerVersion}-Windows-x64.exe`)) {
    throw new Error(`${readme} does not link to both ${build.installerVersion} installers.`);
  }
}
process.stdout.write(`Release version ${build.installerVersion} is consistent.\n`);
