import { describe, expect, it } from "vitest";
import { SETTINGS_KEYS, applySettings, clearSettings, collectSettings, parseSettingsFile } from "./settings-io.ts";

function memoryStorage(init: Record<string, string> = {}) {
  const data = new Map(Object.entries(init));
  return {
    data,
    getItem: (k: string) => data.get(k) ?? null,
    setItem: (k: string, v: string) => void data.set(k, v),
    removeItem: (k: string) => void data.delete(k),
  };
}

describe("settings export and import", () => {
  it("exports settings only, not caches or logs", () => {
    const storage = memoryStorage({
      "pp-lang": "en",
      "pp-webhooks": '["https://hooks.example.com/x"]',
      "pp-geocache": "{}",
      "pp-errors": "[]",
      "pp-pinghist": "[]",
    });
    const file = collectSettings(storage, { closeToTray: true }, new Date("2026-11-01T10:00:00Z"));
    expect(file.storage).toEqual({ "pp-lang": "en", "pp-webhooks": '["https://hooks.example.com/x"]' });
    expect(file.prefs).toEqual({ closeToTray: true });
    expect(file.exported).toBe("2026-11-01T10:00:00.000Z");
  });

  it("round-trips through JSON", () => {
    const file = collectSettings(memoryStorage({ "pp-theme": "light" }), {
      startDispatcher: true,
      dispatcher: { port: 3128, mode: "failover", listener: "http" },
    });
    const parsed = parseSettingsFile(JSON.stringify(file));
    expect(parsed?.storage).toEqual({ "pp-theme": "light" });
    expect(parsed?.prefs).toEqual({
      startDispatcher: true,
      dispatcher: { port: 3128, mode: "failover", listener: "http" },
    });
  });

  it("rejects other files and drops unknown or malformed values", () => {
    expect(parseSettingsFile("not json")).toBeNull();
    expect(parseSettingsFile(JSON.stringify({ app: "Other", kind: "settings", version: 1, storage: {} }))).toBeNull();
    expect(parseSettingsFile(JSON.stringify({ app: "ProxPulse", kind: "settings", version: 2, storage: {} }))).toBeNull();

    const parsed = parseSettingsFile(JSON.stringify({
      app: "ProxPulse",
      kind: "settings",
      version: 1,
      storage: { "pp-lang": "ru", "pp-geocache": "{}", "evil": "x", "pp-maxping": 5 },
      prefs: { closeToTray: "yes", startMinimized: false, dispatcher: { port: 70000, mode: "x", listener: "y" } },
    }));
    expect(parsed?.storage).toEqual({ "pp-lang": "ru" });
    expect(parsed?.prefs).toEqual({ startMinimized: false });
  });

  it("replaces every setting on import and clears them on reset", () => {
    const storage = memoryStorage({ "pp-lang": "en", "pp-theme": "light", "pp-geocache": "{}" });
    applySettings(storage, { "pp-lang": "ru" });
    expect(Object.fromEntries(storage.data)).toEqual({ "pp-lang": "ru", "pp-geocache": "{}" });

    clearSettings(storage);
    expect(Object.fromEntries(storage.data)).toEqual({ "pp-geocache": "{}" });
    expect(SETTINGS_KEYS).not.toContain("pp-geocache");
  });

  it("carries the pool only when asked and validates its entries", () => {
    const storage = memoryStorage();
    expect(collectSettings(storage, {}).pool).toBeUndefined();
    expect(parseSettingsFile(JSON.stringify(collectSettings(storage, {})))?.pool).toBeNull();

    const pool = [{ raw: "u:p@1.2.3.4:80", url: "http://u:p@1.2.3.4:80", latency: 120, country: "DE" }];
    const file = collectSettings(storage, {}, new Date(), pool);
    expect(parseSettingsFile(JSON.stringify(file))?.pool).toEqual(pool);

    const messy = { ...file, pool: [{ raw: "5.6.7.8:3128", latency: -1 }, { raw: 42 }, null, { raw: "" }] };
    expect(parseSettingsFile(JSON.stringify(messy))?.pool).toEqual([
      { raw: "5.6.7.8:3128", url: "5.6.7.8:3128", latency: null, country: null },
    ]);
  });
});
