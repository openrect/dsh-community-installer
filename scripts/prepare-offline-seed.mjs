import { createHash } from "node:crypto";
import { execFileSync } from "node:child_process";
import { cpSync, existsSync, mkdirSync, readFileSync, readdirSync, rmSync, statSync, writeFileSync } from "node:fs";
import { join, relative, resolve, sep } from "node:path";
import { fileURLToPath } from "node:url";
import { unzipSync, zipSync } from "fflate";

const root = resolve(fileURLToPath(new URL("..", import.meta.url)));
const config = JSON.parse(readFileSync(join(root, "build-config.json"), "utf8"));
const seedRoot = join(root, "runtime-seed");
const workRoot = join(seedRoot, "work");
const cacheRoot = join(seedRoot, "cache");
const archivePath = join(cacheRoot, `node-v${config.nodeVersion}-${config.architecture}.zip`);
const outputPath = join(seedRoot, "runtime-seed.zip");
const nodeUrl = `https://nodejs.org/dist/v${config.nodeVersion}/node-v${config.nodeVersion}-${config.architecture}.zip`;
const seedInputs = [
  join(root, "build-config.json"),
  join(root, "payload", "package.json"),
  fileURLToPath(import.meta.url),
];

if (existsSync(outputPath) && statSync(outputPath).mtimeMs >= Math.max(...seedInputs.map((path) => statSync(path).mtimeMs))) {
  process.stdout.write(`Reusing verified offline runtime seed: ${outputPath}\n`);
  process.exit(0);
}

function assertInside(base, candidate) {
  const normalizedBase = resolve(base) + sep;
  const normalizedCandidate = resolve(candidate);
  if (!normalizedCandidate.startsWith(normalizedBase)) throw new Error(`Unsafe generated path: ${candidate}`);
}

function removeGenerated(path) {
  assertInside(seedRoot, path);
  rmSync(path, { recursive: true, force: true });
}

async function downloadNode() {
  mkdirSync(cacheRoot, { recursive: true });
  if (!existsSync(archivePath)) {
    const response = await fetch(nodeUrl);
    if (!response.ok) throw new Error(`Node.js download failed: ${response.status}`);
    writeFileSync(archivePath, Buffer.from(await response.arrayBuffer()));
  }
  const actual = createHash("sha256").update(readFileSync(archivePath)).digest("hex");
  if (actual !== config.nodeArchiveSha256) {
    rmSync(archivePath, { force: true });
    throw new Error(`Node.js SHA-256 mismatch: ${actual}`);
  }
}

function extractNode() {
  const entries = unzipSync(new Uint8Array(readFileSync(archivePath)));
  const expectedRoot = `node-v${config.nodeVersion}-${config.architecture}/`;
  for (const [name, bytes] of Object.entries(entries)) {
    if (!name.startsWith(expectedRoot)) throw new Error(`Unexpected Node.js archive entry: ${name}`);
    const relativeName = name.slice(expectedRoot.length);
    if (!relativeName || relativeName.endsWith("/")) continue;
    const destination = join(workRoot, "node", ...relativeName.split("/"));
    assertInside(workRoot, destination);
    mkdirSync(resolve(destination, ".."), { recursive: true });
    writeFileSync(destination, bytes);
  }
}

function installDsh() {
  const runtime = join(workRoot, "dsh", config.dshVersion, "runtime");
  mkdirSync(runtime, { recursive: true });
  cpSync(join(root, "payload", "package.json"), join(runtime, "package.json"));
  const node = join(workRoot, "node", "node.exe");
  const corepack = join(workRoot, "node", "node_modules", "corepack", "dist", "corepack.js");
  const env = {
    ...process.env,
    PATH: `${join(workRoot, "node")};${process.env.PATH ?? ""}`,
    COREPACK_HOME: join(cacheRoot, "corepack"),
    COREPACK_ENABLE_DOWNLOAD_PROMPT: "0",
  };
  execFileSync(node, [
    corepack,
    "pnpm",
    "install",
    "--ignore-workspace",
    "--prod",
    "--reporter=ndjson",
    "--config.node-linker=hoisted",
    "--config.package-import-method=copy",
    "--config.auto-install-peers=true",
    "--config.confirm-modules-purge=false",
    "--dangerously-allow-all-builds",
    "--registry=https://registry.npmjs.org/",
    `--store-dir=${join(cacheRoot, "pnpm-store")}`,
  ], { cwd: runtime, env, stdio: "inherit" });
  const manifest = JSON.parse(readFileSync(join(runtime, "node_modules", "@deepseek-ai", "dsh", "package.json"), "utf8"));
  if (manifest.version !== config.dshVersion) throw new Error(`Unexpected offline DSH version: ${manifest.version}`);
  const bin = typeof manifest.bin === "string" ? manifest.bin : manifest.bin?.dsh;
  if (!bin) throw new Error("The offline DSH package has no dsh binary.");
  const reported = execFileSync(node, [join(runtime, "node_modules", "@deepseek-ai", "dsh", ...bin.split("/")), "--version"], { cwd: runtime, env, encoding: "utf8" }).trim();
  if (reported !== config.dshVersion) throw new Error(`Offline DSH reported ${reported} instead of ${config.dshVersion}.`);
  generateThirdPartyInventory(runtime);
}

function generateThirdPartyInventory(runtime) {
  const packages = [];
  function visit(directory) {
    for (const name of readdirSync(directory)) {
      if (name === ".pnpm") continue;
      const path = join(directory, name);
      if (!statSync(path).isDirectory()) continue;
      const manifestPath = join(path, "package.json");
      if (existsSync(manifestPath)) {
        const manifest = JSON.parse(readFileSync(manifestPath, "utf8"));
        if (manifest.name && manifest.version) {
          const license = typeof manifest.license === "string" ? manifest.license : "SEE PACKAGE";
          packages.push({ path: relative(runtime, path).split(sep).join("/"), version: manifest.version, license });
        }
      }
      const nested = join(path, "node_modules");
      if (existsSync(nested)) visit(nested);
      if (name.startsWith("@") && !existsSync(manifestPath)) visit(path);
    }
  }
  visit(join(runtime, "node_modules"));
  packages.sort((left, right) => left.path.localeCompare(right.path, "en"));
  const output = [
    "DSH Community Installer third-party package inventory",
    "=====================================================",
    "",
    `Generated from the pnpm ${config.pnpmVersion} offline payload for DSH ${config.dshVersion}.`,
    `Package count: ${packages.length}`,
    "",
    "Format: package path | version | SPDX license",
    "",
    ...packages.map((item) => `${item.path} | ${item.version} | ${item.license}`),
    "",
  ].join("\n");
  writeFileSync(join(root, "THIRD_PARTY_PACKAGES.txt"), output, "utf8");
}

function collect(directory, files = {}) {
  for (const name of readdirSync(directory)) {
    const path = join(directory, name);
    const relativeName = relative(workRoot, path).split(sep).join("/");
    if (statSync(path).isDirectory()) collect(path, files);
    else files[relativeName] = new Uint8Array(readFileSync(path));
  }
  return files;
}

await downloadNode();
removeGenerated(workRoot);
mkdirSync(workRoot, { recursive: true });
extractNode();
installDsh();
writeFileSync(outputPath, Buffer.from(zipSync(collect(workRoot), { level: 6 })));
removeGenerated(workRoot);
const sizeMiB = (statSync(outputPath).size / 1024 / 1024).toFixed(2);
process.stdout.write(`Offline runtime seed: ${outputPath} (${sizeMiB} MiB)\n`);
