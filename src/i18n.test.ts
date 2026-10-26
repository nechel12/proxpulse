import { describe, expect, it } from "vitest";
import { I18N, translate } from "./i18n.ts";

describe("i18n", () => {
  it("has the same keys in RU and EN", () => {
    const ru = Object.keys(I18N.ru).sort();
    const en = Object.keys(I18N.en).sort();
    expect(ru).toEqual(en);
  });

  it("interpolates placeholders", () => {
    expect(translate("ru", "st_copied", { n: 3 })).toBe("Скопировано: 3.");
    expect(translate("en", "st_copied", { n: 3 })).toBe("Copied: 3.");
  });

  it("falls back to the key for unknown entries", () => {
    expect(translate("en", "missing.key")).toBe("missing.key");
  });
});
