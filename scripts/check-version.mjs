// Ensures the version is identical in package.json, src-tauri/tauri.conf.json, src-tauri/Cargo.toml
// and the workspace Cargo.toml (and, with --tag vX.Y.Z, the git tag).
import { readFileSync } from "node:fs";

const pkg = JSON.parse(readFileSync("package.json", "utf8")).version;
const conf = JSON.parse(readFileSync("src-tauri/tauri.conf.json", "utf8")).version;
const cargoApp = readFileSync("src-tauri/Cargo.toml", "utf8").match(/^version\s*=\s*"([^"]+)"/m)?.[1];
const cargoWs = readFileSync("Cargo.toml", "utf8").match(/\[workspace\.package\][^[]*?version\s*=\s*"([^"]+)"/s)?.[1];
const i = process.argv.indexOf("--tag");
const tag = i > 0 ? process.argv[i + 1].replace(/^v/, "") : null;
const all = { "package.json": pkg, "tauri.conf.json": conf, "src-tauri/Cargo.toml": cargoApp, "Cargo.toml (workspace)": cargoWs };
if (tag) all["git tag"] = tag;
const values = new Set(Object.values(all));
console.log(all);
if (values.size !== 1 || !/^\d+\.\d+\.\d+$/.test(pkg)) {
  console.error("Versões divergentes ou não SemVer");
  process.exit(1);
}
console.log(`OK: ${pkg}`);
