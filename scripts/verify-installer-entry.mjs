import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { fileURLToPath } from "node:url";

const root = resolve(fileURLToPath(new URL("..", import.meta.url)));
const hooks = ["installer-hooks-online.nsh", "installer-hooks-offline.nsh"];

for (const hook of hooks) {
  const source = readFileSync(resolve(root, "src-tauri", hook), "utf8");
  const required = [
    "SilentInstall silent",
    "${GetParameters} $0",
    '${GetOptions} "$0" "/S" $1',
    "IfErrors launch_application postinstall_done",
    "Exec '\"$INSTDIR\\${MAINBINARYNAME}.exe\"'",
  ];
  for (const statement of required) {
    if (!source.includes(statement)) {
      throw new Error(`${hook} is missing the installer entry rule: ${statement}`);
    }
  }
  if (source.includes("IfSilent")) {
    throw new Error(`${hook} must distinguish an explicit /S argument from forced silent presentation.`);
  }
}

process.stdout.write("Online and offline installers share the approved entry behavior.\n");
