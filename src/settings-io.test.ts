import { describe, expect, it } from "vitest";
import {
  SETTINGS_KEYS,
  applySettings,
  clearSettings,
  collectSettings,
  mergeImportedPrefs,
  parseSettingsFile,
} from "./settings-io.ts";

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

  it("keeps the dispatcher options", () => {
    const options = { countries: ["DE"], stickyMin: 10, authUser: "me", authPass: "secret" };
    const file = collectSettings(memoryStorage(), { dispatcher: { port: 1080, mode: "failover", listener: "both", options } });
    expect(parseSettingsFile(JSON.stringify(file))?.prefs.dispatcher?.options).toEqual(options);

    const messy = { ...file, prefs: { dispatcher: { port: 1080, mode: "x", listener: "y", options: { countries: [1, "NL"], stickyMin: 1e9 } } } };
    expect(parseSettingsFile(JSON.stringify(messy))?.prefs.dispatcher?.options).toEqual({
      countries: ["NL"], stickyMin: 1440, authUser: "", authPass: "",
    });
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

  it("carries the named pools with their own settings", () => {
    const storage = memoryStorage({
      "pp-auto-on": "1",
      "pp-auto-on@p1": "0",
      "pp-webhooks@p1": '["https://hooks.example.com/w"]',
      "pp-auto-on@gone": "1",
    });
    const items = [{ raw: "1.2.3.4:80", url: "http://1.2.3.4:80", latency: 50, country: null }];
    const file = collectSettings(storage, {
      poolDispatchers: { p1: { port: 1081, mode: "fastest", listener: "both", started: true } },
    }, new Date(), null, [{ id: "main", name: "" }, { id: "p1", name: "Work", items }]);
    expect(file.storage).toEqual({
      "pp-auto-on": "1",
      "pp-auto-on@p1": "0",
      "pp-webhooks@p1": '["https://hooks.example.com/w"]',
    });
    expect(file.pools).toEqual([{ id: "p1", name: "Work", items }]);

    const parsed = parseSettingsFile(JSON.stringify({
      ...file,
      storage: { ...file.storage, "pp-auto-on@other": "1" },
      pools: [...(file.pools ?? []), { id: "main", name: "x" }, { id: "../x", name: "bad" }, { id: "p1", name: "dup" }],
      prefs: { poolDispatchers: { ...file.prefs.poolDispatchers, main: { port: 1, mode: "x", listener: "y" } } },
    }));
    expect(parsed?.pools).toEqual([{ meta: { id: "p1", name: "Work" }, items }]);
    expect(parsed?.storage).toEqual(file.storage);
    expect(parsed?.prefs.poolDispatchers).toEqual({ p1: { port: 1081, mode: "fastest", listener: "both", started: true } });

    // Import replaces the settings of every pool named, reset clears them.
    const target = memoryStorage({ "pp-auto-on@p1": "1", "pp-maxping@p1": "300", "pp-auto-on@p2": "1" });
    applySettings(target, parsed?.storage ?? {}, ["main", "p1"]);
    expect(Object.fromEntries(target.data)).toEqual({ ...file.storage, "pp-auto-on@p2": "1" });
    clearSettings(target, ["main", "p1", "p2"]);
    expect(target.data.size).toBe(0);
  });

  it("keeps the pool API switch and port", () => {
    const file = collectSettings(memoryStorage(), { poolApi: { enabled: true, port: 1090 } });
    expect(parseSettingsFile(JSON.stringify(file))?.prefs.poolApi).toEqual({ enabled: true, port: 1090 });
    const bad = { ...file, prefs: { poolApi: { enabled: "yes", port: 70000 } } };
    expect(parseSettingsFile(JSON.stringify(bad))?.prefs.poolApi).toBeUndefined();
  });

  it("keeps on import what the file leaves out", () => {
    const login = { countries: [], stickyMin: 0, authUser: "u", authPass: "p" };
    const current = {
      closeToTray: true,
      dispatcher: { port: 1080, mode: "fastest", listener: "both", options: login },
      poolDispatchers: { p1: { port: 1081, mode: "round-robin", listener: "both", options: login } },
      poolApi: { enabled: true, port: 1090 },
    };
    // A file with nothing about the dispatchers changes nothing about them.
    expect(mergeImportedPrefs({ closeToTray: false }, current)).toEqual({ closeToTray: false });
    // An older export: the dispatcher without its filter and login.
    const old = mergeImportedPrefs({ dispatcher: { port: 2000, mode: "failover", listener: "http" } }, current);
    expect(old.dispatcher).toEqual({ port: 2000, mode: "failover", listener: "http", options: login });
    // The file's own login wins, and pools it does not mention keep theirs.
    const other = { countries: ["DE"], stickyMin: 5, authUser: "", authPass: "" };
    const merged = mergeImportedPrefs(
      { poolDispatchers: { p2: { port: 1082, mode: "fastest", listener: "both", options: other } } },
      current,
    );
    expect(merged.poolDispatchers?.p1.options).toEqual(login);
    expect(merged.poolDispatchers?.p2.options).toEqual(other);
  });
});
