// The GitHub release description of one version: its CHANGELOG section, a link
// to the full diff and the file-to-platform map (file names contain the version).
// Usage: node scripts/release-notes.mjs <X.Y.Z>

import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

/**
 * The version's section without its heading; null when missing or empty.
 * Release descriptions keep line breaks, so wrapped list items become one line.
 */
export function changelogSection(changelog, version) {
  const lines = changelog.replace(/\r\n/g, "\n").split("\n");
  const start = lines.findIndex((l) => l.startsWith(`## [${version}]`));
  if (start < 0) return null;
  const out = [];
  for (const line of lines.slice(start + 1)) {
    if (line.startsWith("## [") || /^\[[^\]]+\]: /.test(line)) break;
    if (/^\s+\S/.test(line) && out.at(-1)?.startsWith("- ")) {
      out[out.length - 1] += ` ${line.trim()}`;
    } else {
      out.push(line);
    }
  }
  return out.join("\n").trim() || null;
}

function downloads(version) {
  return `## What to download

**Windows (x64)**
- \`ProxPulse_${version}_x64-setup.exe\` — NSIS installer (recommended, auto-updates)
- \`ProxPulse_${version}_x64_en-US.msi\` — MSI installer (manual updates)
- \`ProxPulse_${version}_x64-portable.exe\` — portable, no install (manual updates)

**Linux (x64)**
- \`ProxPulse_${version}_amd64.AppImage\` — portable (auto-updates)
- \`ProxPulse_${version}_amd64.deb\` — Debian/Ubuntu package (update via apt/dpkg)
- \`ProxPulse-${version}-1.x86_64.rpm\` — Fedora/openSUSE package (update via rpm/dnf)

**macOS (Apple Silicon)** — unsigned, see README for Gatekeeper steps
- \`ProxPulse_${version}_aarch64.dmg\`

**macOS (Intel)** — unsigned and untested
- \`ProxPulse_${version}_x64.dmg\`

Linux auto-update is AppImage-only. \`latest.json\` and \`.sig\` files
are for the built-in updater; you do not need to download them.
\`SHA256SUMS.txt\` lists the SHA-256 checksum of every file:
\`sha256sum --ignore-missing -c SHA256SUMS.txt\`.`;
}

/** Throws when the changelog lacks the version's section or compare link. */
export function releaseNotes(changelog, version) {
  const section = changelogSection(changelog, version);
  if (!section) throw new Error(`CHANGELOG.md has no section for ${version}`);
  const ref = `[${version}]: `;
  const link = changelog
    .split(/\r?\n/)
    .find((l) => l.startsWith(ref))
    ?.slice(ref.length)
    .trim();
  if (!link) throw new Error(`CHANGELOG.md has no compare link for ${version}`);
  return `## What's new\n\n${section}\n\nFull changelog: ${link}\n\n${downloads(version)}\n`;
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  const version = process.argv[2];
  if (!version) {
    console.error("usage: node scripts/release-notes.mjs <X.Y.Z>");
    process.exit(1);
  }
  const root = join(dirname(fileURLToPath(import.meta.url)), "..");
  try {
    process.stdout.write(releaseNotes(readFileSync(join(root, "CHANGELOG.md"), "utf8"), version));
  } catch (e) {
    console.error(e.message);
    process.exit(1);
  }
}
