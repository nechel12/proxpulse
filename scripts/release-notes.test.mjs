import { describe, expect, it } from "vitest";
import { changelogSection, releaseNotes } from "./release-notes.mjs";

const changelog = `# Changelog

## [Unreleased]

## [1.2.0] — 2026-01-02

### Added

- A long entry that wraps
  onto a second line.
- A short entry.

### Fixed

- A fix.

## [1.1.0] — 2026-01-01

## [1.0.0] — 2025-12-01

- First version.

[Unreleased]: https://example.com/compare/v1.2.0...HEAD
[1.2.0]: https://example.com/compare/v1.1.0...v1.2.0
[1.1.0]: https://example.com/compare/v1.0.0...v1.1.0
`;

describe("release notes", () => {
  it("takes one version's section and joins wrapped list items", () => {
    expect(changelogSection(changelog, "1.2.0")).toBe(
      "### Added\n\n- A long entry that wraps onto a second line.\n- A short entry.\n\n### Fixed\n\n- A fix.",
    );
  });

  it("stops at the links after the last section", () => {
    expect(changelogSection(changelog, "1.0.0")).toBe("- First version.");
  });

  it("reads a checkout with CRLF line endings the same way", () => {
    const crlf = changelog.replace(/\n/g, "\r\n");
    expect(changelogSection(crlf, "1.2.0")).toBe(changelogSection(changelog, "1.2.0"));
    expect(releaseNotes(crlf, "1.2.0")).toBe(releaseNotes(changelog, "1.2.0"));
  });

  it("treats a missing or empty section as no section", () => {
    expect(changelogSection(changelog, "9.9.9")).toBeNull();
    expect(changelogSection(changelog, "1.1.0")).toBeNull();
    // A version must match as a whole, not as a prefix.
    expect(changelogSection(changelog, "1.2")).toBeNull();
  });

  it("builds the description from the section, the link and the file map", () => {
    const notes = releaseNotes(changelog, "1.2.0");
    expect(notes.startsWith("## What's new\n\n### Added\n")).toBe(true);
    expect(notes).toContain("\n\nFull changelog: https://example.com/compare/v1.1.0...v1.2.0\n\n## What to download\n");
    expect(notes).toContain("`ProxPulse_1.2.0_x64-setup.exe`");
    expect(notes).toContain("`ProxPulse-1.2.0-1.x86_64.rpm`");
    expect(notes.endsWith("`sha256sum --ignore-missing -c SHA256SUMS.txt`.\n")).toBe(true);
  });

  it("refuses a version without a section or a compare link", () => {
    expect(() => releaseNotes(changelog, "1.1.0")).toThrow("no section");
    expect(() => releaseNotes(changelog, "1.0.0")).toThrow("no compare link");
  });
});
