import { describe, expect, it } from "vitest";
import { updaterArtifacts, updaterManifest } from "./updater-manifest.mjs";

const names = [
  "ProxPulse_0.7.0_x64-setup.exe",
  "ProxPulse_0.7.0_x64_en-US.msi",
  "ProxPulse_0.7.0_x64-portable.exe",
  "ProxPulse_0.7.0_amd64.AppImage",
  "ProxPulse_0.7.0_amd64.deb",
  "ProxPulse-0.7.0-1.x86_64.rpm",
  "ProxPulse_0.7.0_aarch64.app.tar.gz",
  "ProxPulse_0.7.0_aarch64.dmg",
  "ProxPulse_0.7.0_x64.app.tar.gz",
  "ProxPulse_0.7.0_x64.dmg",
  "SHA256SUMS.txt",
];

describe("updater manifest", () => {
  it("signs only what the updater downloads", () => {
    expect(updaterArtifacts(names)).toEqual([
      "ProxPulse_0.7.0_x64-setup.exe",
      "ProxPulse_0.7.0_x64_en-US.msi",
      "ProxPulse_0.7.0_amd64.AppImage",
      "ProxPulse_0.7.0_amd64.deb",
      "ProxPulse-0.7.0-1.x86_64.rpm",
      "ProxPulse_0.7.0_aarch64.app.tar.gz",
      "ProxPulse_0.7.0_x64.app.tar.gz",
    ]);
  });

  it("has every platform key the 0.6.1 manifest had", () => {
    const assets = names.map((name, i) => ({ name, url: `https://api.github.com/assets/${i}` }));
    const m = updaterManifest("0.7.0", assets, (name) => `sig-of-${name}\n`, "2026-01-01T00:00:00.000Z");
    expect(Object.keys(m.platforms).sort()).toEqual([
      "darwin-aarch64", "darwin-aarch64-app", "darwin-x86_64", "darwin-x86_64-app",
      "linux-x86_64", "linux-x86_64-appimage", "linux-x86_64-deb", "linux-x86_64-rpm",
      "windows-x86_64", "windows-x86_64-msi", "windows-x86_64-nsis",
    ]);
    // NSIS for the plain Windows key, as updaterJsonPreferNsis gave.
    expect(m.platforms["windows-x86_64"]).toEqual({
      signature: "sig-of-ProxPulse_0.7.0_x64-setup.exe",
      url: "https://api.github.com/assets/0",
    });
    expect(m.platforms["darwin-x86_64"].url).toBe("https://api.github.com/assets/8");
    expect(m.version).toBe("0.7.0");
  });
});
