import { createHash } from "node:crypto";
import { existsSync, readFileSync, readdirSync, statSync, writeFileSync } from "node:fs";
import { join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const root = resolve(fileURLToPath(new URL("..", import.meta.url)));
const directory = join(root, "dist", "tauri");
if (!existsSync(directory)) throw new Error("No Tauri release artifacts were built.");
const config = JSON.parse(readFileSync(join(root, "build-config.json"), "utf8"));
const updaterArchive = `DSH-Community-Setup-${config.installerVersion}-Windows-x64.exe`;
const updaterSignature = `${updaterArchive}.sig`;
const offlineArchive = `DSH-Community-Offline-Setup-${config.installerVersion}-Windows-x64.exe`;
const offlineSignature = `${offlineArchive}.sig`;
if (!existsSync(join(directory, updaterArchive)) || !existsSync(join(directory, updaterSignature))) {
  throw new Error("The online updater archive and signature are required.");
}
if (!existsSync(join(directory, offlineArchive)) || !existsSync(join(directory, offlineSignature))) {
  throw new Error("The offline installer and signature are required.");
}
const expectedArtifacts = new Set([updaterArchive, updaterSignature, offlineArchive, offlineSignature]);
const staleInstallers = readdirSync(directory).filter((name) =>
  (name.endsWith(".exe") || name.endsWith(".exe.sig")) && !expectedArtifacts.has(name));
if (staleInstallers.length > 0) {
  throw new Error(`Stale release artifacts must be removed: ${staleInstallers.join(", ")}`);
}
const latest = {
  version: config.installerVersion,
  notes: "DSH Community Installer controller update.",
  pub_date: new Date().toISOString(),
  platforms: {
    "windows-x86_64": {
      signature: readFileSync(join(directory, updaterSignature), "utf8").trim(),
      url: `https://github.com/openrect/dsh-community-installer/releases/download/v${config.installerVersion}/${updaterArchive}`,
    },
  },
};
writeFileSync(join(directory, "latest.json"), `${JSON.stringify(latest, null, 2)}\n`);
const files = readdirSync(directory).filter((name) => statSync(join(directory, name)).isFile() && name !== "SHA256SUMS.txt").sort();
const lines = files.map((name) => `${createHash("sha256").update(readFileSync(join(directory, name))).digest("hex")}  ${name}`);
writeFileSync(join(directory, "SHA256SUMS.txt"), `${lines.join("\n")}\n`);
process.stdout.write(`${lines.join("\n")}\n`);
