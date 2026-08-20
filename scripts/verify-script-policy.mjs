import { readFileSync } from "node:fs";
import { join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const root = resolve(fileURLToPath(new URL("..", import.meta.url)));
const payload = JSON.parse(readFileSync(join(root, "payload", "package.json"), "utf8"));
const policy = JSON.parse(readFileSync(join(root, "payload", "script-policy.json"), "utf8"));

if (policy.schemaVersion !== 1 || !Array.isArray(policy.allowed)) {
  throw new Error("payload/script-policy.json must use schema version 1 with an allowed array.");
}
const allowed = [...policy.allowed].sort();
const payloadAllowScripts = Object.keys(payload.allowScripts ?? {}).sort();
if (new Set(allowed).size !== allowed.length || JSON.stringify(policy.allowed) !== JSON.stringify(allowed)) {
  throw new Error("The install script policy must contain unique, sorted package@version entries.");
}
if (JSON.stringify(allowed) !== JSON.stringify(payloadAllowScripts)) {
  throw new Error("The offline payload allowScripts entries do not match script-policy.json.");
}
for (const key of payloadAllowScripts) {
  if (payload.allowScripts[key] !== true) {
    throw new Error(`payload/package.json must explicitly allow ${key}.`);
  }
}
process.stdout.write(`Install script policy contains ${allowed.length} approved packages.\n`);
