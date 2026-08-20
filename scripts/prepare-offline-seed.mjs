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
  join(root, "payload", "package-lock.json"),
  join(root, "payload", "script-policy.json"),
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
  cpSync(join(root, "payload", "package-lock.json"), join(runtime, "package-lock.json"));
  const node = join(workRoot, "node", "node.exe");
  const npm = join(workRoot, "node", "node_modules", "npm", "bin", "npm-cli.js");
  const env = { ...process.env, PATH: `${join(workRoot, "node")};${process.env.PATH ?? ""}` };
  execFileSync(node, [npm, "ci", "--omit=dev", "--no-audit", "--no-fund", "--ignore-scripts", "--registry=https://registry.npmjs.org/"], { cwd: runtime, env, stdio: "inherit" });
  execFileSync(node, [npm, "rebuild", "--no-audit", "--no-fund"], { cwd: runtime, env, stdio: "inherit" });
  const pending = JSON.parse(execFileSync(node, [npm, "approve-scripts", "--allow-scripts-pending", "--json"], { cwd: runtime, env, encoding: "utf8" }));
  if (pending.allowScripts?.length) throw new Error(`Unreviewed install scripts remain: ${pending.allowScripts.map((entry) => entry.name).join(", ")}`);
  const manifest = JSON.parse(readFileSync(join(runtime, "node_modules", "@deepseek-ai", "dsh", "package.json"), "utf8"));
  if (manifest.version !== config.dshVersion) throw new Error(`Unexpected offline DSH version: ${manifest.version}`);
  const bin = typeof manifest.bin === "string" ? manifest.bin : manifest.bin?.dsh;
  if (!bin) throw new Error("The offline DSH package has no dsh binary.");
  const reported = execFileSync(node, [join(runtime, "node_modules", "@deepseek-ai", "dsh", ...bin.split("/")), "--version"], { cwd: runtime, env, encoding: "utf8" }).trim();
  if (reported !== config.dshVersion) throw new Error(`Offline DSH reported ${reported} instead of ${config.dshVersion}.`);
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
