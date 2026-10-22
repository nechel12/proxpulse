import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const read = (path) => readFileSync(join(root, path), "utf8");

const packageVersion = JSON.parse(read("package.json")).version;
const tauriVersion = JSON.parse(read("src-tauri/tauri.conf.json")).version;

const cargoToml = read("src-tauri/Cargo.toml");
const cargoPackage = cargoToml.match(/^\[package\][\s\S]*?^version\s*=\s*"([^"]+)"/m);
const cargoVersion = cargoPackage?.[1];

const cargoLock = read("src-tauri/Cargo.lock");
const lockBlock = cargoLock.match(
  /^name = "proxpulse"\r?\nversion = "([^"]+)"/m,
);
const lockVersion = lockBlock?.[1];

const versions = {
  "package.json": packageVersion,
  "src-tauri/tauri.conf.json": tauriVersion,
  "src-tauri/Cargo.toml": cargoVersion,
  "src-tauri/Cargo.lock": lockVersion,
};

const missing = Object.entries(versions)
  .filter(([, version]) => !version)
  .map(([file]) => file);
if (missing.length > 0) {
  console.error(`Could not read version from: ${missing.join(", ")}`);
  process.exit(1);
}

const unique = new Set(Object.values(versions));
if (unique.size !== 1) {
  console.error("Version mismatch:");
  for (const [file, version] of Object.entries(versions)) {
    console.error(`  ${file}: ${version}`);
  }
  process.exit(1);
}

const version = [...unique][0];
const tag = process.env.GITHUB_REF_NAME;
if (tag?.startsWith("v") && tag.slice(1) !== version) {
  console.error(`Tag ${tag} does not match project version ${version}`);
  process.exit(1);
}

console.log(`Versions are in sync: ${version}`);
