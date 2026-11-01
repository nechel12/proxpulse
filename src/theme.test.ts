import { describe, expect, it } from "vitest";
import { parseThemePref, resolveTheme } from "./theme.ts";

describe("theme", () => {
  it("defaults unknown values to the dark theme", () => {
    expect(parseThemePref("light")).toBe("light");
    expect(parseThemePref("system")).toBe("system");
    expect(parseThemePref(null)).toBe("dark");
    expect(parseThemePref("neon")).toBe("dark");
  });

  it("follows the system only for the system preference", () => {
    expect(resolveTheme("system", true)).toBe("light");
    expect(resolveTheme("system", false)).toBe("dark");
    expect(resolveTheme("dark", true)).toBe("dark");
    expect(resolveTheme("light", false)).toBe("light");
  });
});
