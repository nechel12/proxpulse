// latest.json for the built-in updater, made by the release workflow's `sign`
// job after it signs the updater artifacts (the platform jobs build without the
// signing key, so tauri-action does not write it).
// Usage: node scripts/updater-manifest.mjs <X.Y.Z> <assets.json> <sig dir>
//        node scripts/updater-manifest.mjs --artifacts <assets.json>
// assets.json is the release's asset list from the GitHub API; --artifacts
// prints the names of the files to sign, one per line.

import { readFileSync } from "node:fs";
import { join } from "node:path";
import { pathToFileURL } from "node:url";

/** Updater platforms of each artifact, by the end of its file name. */
const PLATFORMS = [
  ["_x64-setup.exe", ["windows-x86_64", "windows-x86_64-nsis"]],
  ["_x64_en-US.msi", ["windows-x86_64-msi"]],
  ["_amd64.AppImage", ["linux-x86_64", "linux-x86_64-appimage"]],
  ["_amd64.deb", ["linux-x86_64-deb"]],
  [".x86_64.rpm", ["linux-x86_64-rpm"]],
  ["_aarch64.app.tar.gz", ["darwin-aarch64", "darwin-aarch64-app"]],
  ["_x64.app.tar.gz", ["darwin-x86_64", "darwin-x86_64-app"]],
];

/** The artifacts the updater downloads, as release asset file names. */
export function updaterArtifacts(names) {
  return names.filter((n) => PLATFORMS.some(([end]) => n.endsWith(end)));
}

/**
 * The manifest. `assets` are `{name, url}` from the API: the API URL of an
 * asset stays the same when the draft is published, its download URL does not.
 * `signature(name)` returns the `.sig` file's content.
 */
export function updaterManifest(version, assets, signature, pubDate = new Date().toISOString()) {
  const platforms = {};
  for (const asset of assets) {
    const match = PLATFORMS.find(([end]) => asset.name.endsWith(end));
    if (!match) continue;
    const entry = { signature: signature(asset.name).trim(), url: asset.url };
    for (const platform of match[1]) platforms[platform] = entry;
  }
  return {
    version,
    notes: "See the assets below to download and install this version.",
    pub_date: pubDate,
    platforms,
  };
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  const [version, assetsFile, sigDir] = process.argv.slice(2);
  if (version === "--artifacts" && assetsFile) {
    const names = JSON.parse(readFileSync(assetsFile, "utf8")).map((a) => a.name);
    process.stdout.write(updaterArtifacts(names).map((n) => `${n}
`).join(""));
    process.exit(0);
  }
  if (!version || !assetsFile || !sigDir) {
    console.error("usage: node scripts/updater-manifest.mjs <X.Y.Z> <assets.json> <sig dir>");
    process.exit(1);
  }
  const assets = JSON.parse(readFileSync(assetsFile, "utf8")).map((a) => ({ name: a.name, url: a.url }));
  const manifest = updaterManifest(version, assets, (name) => readFileSync(join(sigDir, `${name}.sig`), "utf8"));
  process.stdout.write(`${JSON.stringify(manifest, null, 2)}\n`);
}
