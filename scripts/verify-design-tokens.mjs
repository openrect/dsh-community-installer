import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { fileURLToPath } from "node:url";

const root = resolve(fileURLToPath(new URL("..", import.meta.url)));
const css = readFileSync(resolve(root, "frontend", "styles.css"), "utf8");
const rust = readFileSync(resolve(root, "src-tauri", "src", "lib.rs"), "utf8");
const allowedSizes = new Set(["14px", "18px", "24px"]);
const allowedWeights = new Set(["400", "500", "600"]);
for (const match of css.matchAll(/font-size:\s*([^;]+)/g)) {
  const value = match[1].trim();
  if (!value.startsWith("var(") && !allowedSizes.has(value)) throw new Error(`Disallowed font size: ${value}`);
}
for (const match of css.matchAll(/font-weight:\s*([^;]+)/g)) {
  const value = match[1].trim();
  if (!value.startsWith("var(") && !allowedWeights.has(value)) throw new Error(`Disallowed font weight: ${value}`);
}
for (const match of css.matchAll(/border-radius:\s*([^;]+)/g)) {
  const value = match[1].trim();
  if (value !== "var(--radius)" && value !== "15px") throw new Error(`Disallowed radius: ${value}`);
}
if (/window\.chrome\.webview|WebView2Loader|System\.Windows\.Forms|PresentationFramework/.test(readFileSync(resolve(root, "frontend", "main.ts"), "utf8"))) {
  throw new Error("Legacy UI APIs remain in the Tauri frontend.");
}
if ((rust.match(/\.transparent\(true\)/g) ?? []).length !== 4) {
  throw new Error("Every product window must enable native transparency.");
}
if (!/html, body, #app[^}]+background:\s*transparent/.test(css)
    || !/#ambient[^}]+clip-path:\s*inset\(0 round var\(--radius\)\)/.test(css)) {
  throw new Error("The WebView and animated background must be clipped to the approved radius.");
}
process.stdout.write("Design tokens are restricted to the approved values.\n");
