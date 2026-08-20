import { execFileSync } from "node:child_process";
import { cpSync, existsSync, mkdirSync, readFileSync, readdirSync, rmSync, statSync } from "node:fs";
import { basename, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const edition = process.argv[2];
if (edition !== "online" && edition !== "offline") throw new Error("Edition must be online or offline.");
const root = resolve(fileURLToPath(new URL("..", import.meta.url)));
if (process.env.USERPROFILE) process.env.PATH = `${join(process.env.USERPROFILE, ".cargo", "bin")};${process.env.PATH ?? ""}`;
const config = JSON.parse(readFileSync(join(root, "build-config.json"), "utf8"));
execFileSync(process.execPath, [join(root, "scripts", "verify-release-version.mjs")], { cwd: root, stdio: "inherit" });
const keyPath = join(process.env.USERPROFILE ?? "", ".tauri", "dsh-community-installer.key");
if (!process.env.TAURI_SIGNING_PRIVATE_KEY && existsSync(keyPath)) {
  process.env.TAURI_SIGNING_PRIVATE_KEY = readFileSync(keyPath, "utf8");
  process.env.TAURI_SIGNING_PRIVATE_KEY_PASSWORD ??= "";
}
if (!process.env.TAURI_SIGNING_PRIVATE_KEY) throw new Error("TAURI_SIGNING_PRIVATE_KEY is required to build signed updater artifacts.");
const tauriCli = join(root, "node_modules", "@tauri-apps", "cli", "tauri.js");
const bundle = join(root, "src-tauri", "target", "release", "bundle", "nsis");
rmSync(bundle, { recursive: true, force: true });
if (edition === "offline") {
  execFileSync(process.execPath, [join(root, "scripts", "prepare-offline-seed.mjs")], { cwd: root, stdio: "inherit" });
  execFileSync(process.execPath, [tauriCli, "build", "--no-bundle"], { cwd: root, stdio: "inherit", env: process.env });
  execFileSync(process.execPath, [tauriCli, "bundle", "--bundles", "nsis", "--config", "src-tauri/tauri.offline.conf.json"], { cwd: root, stdio: "inherit", env: process.env });
} else {
  execFileSync(process.execPath, [tauriCli, "build", "--bundles", "nsis"], { cwd: root, stdio: "inherit", env: process.env });
}

const output = join(root, "dist", "tauri");
if (edition === "online") rmSync(output, { recursive: true, force: true });
mkdirSync(output, { recursive: true });
const prefix = edition === "offline" ? "DSH-Community-Offline-Setup" : "DSH-Community-Setup";
for (const entry of readdirSync(bundle)) {
  const source = join(bundle, entry);
  if (!statSync(source).isFile()) continue;
  if (!entry.endsWith(".exe") && !entry.endsWith(".exe.sig")) continue;
  const extension = entry.endsWith(".exe.sig") ? ".exe.sig" : ".exe";
  const destination = join(output, `${prefix}-${config.installerVersion}-Windows-x64${extension}`);
  rmSync(destination, { force: true });
  cpSync(source, destination);
  process.stdout.write(`${basename(destination)}: ${(statSync(destination).size / 1024 / 1024).toFixed(2)} MiB\n`);
}
