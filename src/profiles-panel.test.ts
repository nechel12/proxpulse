import { describe, expect, it } from "vitest";
import { PROFILE_DEFAULTS } from "./profiles.ts";
import { ProfilePanel } from "./profiles-panel.ts";

describe("ProfilePanel", () => {
  it("adds, updates and deletes custom presets", () => {
    const panel = new ProfilePanel();
    expect(panel.upsert("one", PROFILE_DEFAULTS)).toBe(true);
    expect(panel.find("custom:one")?.timeout).toBe(PROFILE_DEFAULTS.timeout);
    panel.upsert("one", { ...PROFILE_DEFAULTS, timeout: "9000" });
    expect(panel.find("custom:one")?.timeout).toBe("9000");
    panel.currentId = "custom:one";
    panel.deleteId("custom:one");
    expect(panel.find("custom:one")).toBeNull();
    expect(panel.currentId).toBe("");
  });

  it("keeps the active preset when another one is deleted", () => {
    const panel = new ProfilePanel();
    panel.upsert("one", PROFILE_DEFAULTS);
    panel.upsert("two", PROFILE_DEFAULTS);
    panel.currentId = "custom:one";
    panel.deleteId("custom:two");
    expect(panel.currentId).toBe("custom:one");
    expect(panel.find("custom:one")).not.toBeNull();
    expect(panel.find("custom:two")).toBeNull();
  });

  it("never deletes built-in presets", () => {
    const panel = new ProfilePanel();
    panel.currentId = "builtin:fast";
    panel.deleteId("builtin:fast");
    expect(panel.currentId).toBe("builtin:fast");
    expect(panel.find("builtin:fast")).not.toBeNull();
  });

  it("caps custom profiles at 20", () => {
    const panel = new ProfilePanel();
    for (let i = 0; i < 20; i++) expect(panel.upsert(`p${i}`, PROFILE_DEFAULTS)).toBe(true);
    expect(panel.upsert("overflow", PROFILE_DEFAULTS)).toBe(false);
  });
});
