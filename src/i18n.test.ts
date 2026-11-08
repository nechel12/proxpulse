/// <reference types="vite/client" />
import { describe, expect, it } from "vitest";
import { I18N, detectLang, translate } from "./i18n.ts";
import html from "../index.html?raw";

const sources = import.meta.glob(["./*.ts", "!./*.test.ts"], {
  query: "?raw",
  import: "default",
  eager: true,
}) as Record<string, string>;

/** Every key the page and the code ask for, by where it is asked. */
function usedKeys(): Map<string, string> {
  const used = new Map<string, string>();
  const add = (re: RegExp, text: string, where: string) => {
    for (const m of text.matchAll(re)) used.set(m[1], where);
  };
  add(/data-i18n(?:-title|-aria|-ph)?="([^"]+)"/g, html, "index.html");
  for (const [file, text] of Object.entries(sources)) {
    // t("key"), setStatusT("key"), deps.notify("key") and the like.
    add(/(?:\bt|setStatusT|setStatus|notify|notifTitle)\(\s*"([a-z][a-z0-9]*_[a-z0-9_]+)"/g, text, file);
    // Keys kept in tables: presets and backend error words.
    add(/(?:nameKey|hintKey):\s*"([a-z0-9_]+)"/g, text, file);
    add(/:\s*"(err_[a-z0-9_]+)"/g, text, file);
  }
  return used;
}

describe("i18n", () => {
  it("has the same keys in RU and EN", () => {
    const ru = Object.keys(I18N.ru).sort();
    const en = Object.keys(I18N.en).sort();
    expect(ru).toEqual(en);
  });

  it("has every key the page and the code use", () => {
    const used = usedKeys();
    expect(used.size).toBeGreaterThan(200);
    const missing = [...used].filter(([key]) => !(key in I18N.en)).map(([key, where]) => `${key} (${where})`);
    expect(missing).toEqual([]);
  });

  it("uses the same placeholders in RU and EN", () => {
    const names = (text: string) => [...text.matchAll(/\{(\w+)\}/g)].map((m) => m[1]).sort();
    const differ = Object.keys(I18N.en).filter(
      (key) => names(I18N.en[key]).join() !== names(I18N.ru[key] ?? "").join(),
    );
    expect(differ).toEqual([]);
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
