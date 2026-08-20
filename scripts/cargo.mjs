import { spawnSync } from "node:child_process";
import { existsSync } from "node:fs";
import { join } from "node:path";

const executable = process.platform === "win32" && process.env.USERPROFILE
  ? join(process.env.USERPROFILE, ".cargo", "bin", "cargo.exe")
  : "cargo";
if (process.platform === "win32" && !existsSync(executable)) throw new Error("Rust is not installed for this user.");
const result = spawnSync(executable, process.argv.slice(2), { stdio: "inherit", env: process.env });
if (result.error) throw result.error;
process.exit(result.status ?? 1);
