import { describe, expect, it } from "vitest";
import { clampMinutes, migrateLegacySchedule, storedAutoConfig } from "./auto-settings.ts";
import type { CheckSettings } from "./check-client.ts";

function store(initial: Record<string, string>) {
  const data = new Map(Object.entries(initial));
  return {
    data,
    getItem: (k: string) => data.get(k) ?? null,
    setItem: (k: string, v: string) => void data.set(k, v),
    removeItem: (k: string) => void data.delete(k),
  };
}

describe("migrateLegacySchedule", () => {
  it("folds both schedules into one with the shorter interval", () => {
    const s = store({ "pp-auto-on": "1", "pp-auto-min": "30", "pp-disp-auto-on": "1", "pp-disp-auto-min": "10" });
    migrateLegacySchedule(s);
    expect(s.data.get("pp-auto-on")).toBe("1");
    expect(s.data.get("pp-auto-min")).toBe("10");
    expect(s.data.get("pp-auto-webhooks")).toBe("1");
    expect(s.data.has("pp-disp-auto-on")).toBe(false);
    expect(s.data.has("pp-disp-auto-min")).toBe(false);
  });

  it("keeps webhooks off when only the dispatcher schedule was on", () => {
    const s = store({ "pp-auto-on": "0", "pp-auto-min": "30", "pp-disp-auto-on": "1", "pp-disp-auto-min": "15" });
    migrateLegacySchedule(s);
    expect(s.data.get("pp-auto-on")).toBe("1");
    expect(s.data.get("pp-auto-min")).toBe("15");
    expect(s.data.get("pp-auto-webhooks")).toBe("0");
  });

  it("leaves current settings alone", () => {
    const s = store({ "pp-auto-on": "1", "pp-auto-min": "45", "pp-auto-webhooks": "0" });
    migrateLegacySchedule(s);
    expect(Object.fromEntries(s.data)).toEqual({ "pp-auto-on": "1", "pp-auto-min": "45", "pp-auto-webhooks": "0" });
  });

  it("clamps minutes", () => {
    expect(clampMinutes("0")).toBe(30);
    expect(clampMinutes("9999")).toBe(1440);
    expect(clampMinutes("7.9")).toBe(7);
    expect(clampMinutes(null)).toBe(30);
  });

  it("reads another pool's schedule from its own keys", () => {
    const check = { testUrl: "https://example.com" } as CheckSettings;
    const s = store({
      "pp-auto-on": "1",
      "pp-auto-on@p1": "1",
      "pp-auto-min@p1": "5",
      "pp-maxping@p1": "800",
      "pp-webhooks@p1": '["https://hooks.example.com/a", ""]',
      "pp-webhook-format@p1": "bogus",
      "pp-subscriptions@p1": '["https://lists.example.com/x.txt", "ftp://nope"]',
    });
    const cfg = storedAutoConfig(s, "p1", check);
    expect(cfg).toMatchObject({
      enabled: true,
      intervalMin: 5,
      sendWebhooks: true,
      maxPingMs: 800,
      webhookUrls: ["https://hooks.example.com/a"],
      webhookFormat: "txt",
      subscriptions: ["https://lists.example.com/x.txt"],
      check,
    });
    expect(storedAutoConfig(s, "p2", check).enabled).toBe(false);
  });
});
