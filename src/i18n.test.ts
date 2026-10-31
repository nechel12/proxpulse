import { describe, expect, it } from "vitest";
import { I18N, detectLang, translate } from "./i18n.ts";

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

  it("picks the initial language from the saved choice or the system", () => {
    expect(detectLang("en", ["ru-RU"])).toBe("en");
    expect(detectLang("ru", ["en-US"])).toBe("ru");
    expect(detectLang(null, ["ru-RU", "en-US"])).toBe("ru");
    expect(detectLang(null, ["RU"])).toBe("ru");
    expect(detectLang(null, ["en-GB", "ru-RU"])).toBe("en");
    expect(detectLang(null, ["de-DE"])).toBe("en");
    expect(detectLang("garbage", [])).toBe("en");
  });
});
