import { describe, expect, it } from "vitest";
import { MAIN_POOL, POOL_KEYS, forgetPoolSettings, poolLabel, scopedKey, validPoolId } from "./pools.ts";

describe("named pools", () => {
  it("keeps the main pool's keys and scopes the others", () => {
    expect(scopedKey("pp-auto-on", MAIN_POOL)).toBe("pp-auto-on");
    expect(scopedKey("pp-auto-on", "p1a2b3")).toBe("pp-auto-on@p1a2b3");
  });

  it("labels the main pool and validates ids", () => {
    const t = (k: string) => (k === "pool_main" ? "Main" : k);
    expect(poolLabel({ id: MAIN_POOL, name: "" }, t)).toBe("Main");
    expect(poolLabel({ id: MAIN_POOL, name: "Own" }, t)).toBe("Own");
    expect(poolLabel({ id: "p1", name: "" }, t)).toBe("p1");
    expect(validPoolId("p1a2b3")).toBe(true);
    expect(validPoolId("P1")).toBe(false);
    expect(validPoolId("a@b")).toBe(false);
  });

  it("forgets a deleted pool's settings only", () => {
    const data = new Map<string, string>([["pp-auto-on", "1"], ["pp-auto-on@p1", "1"], ["pp-lang", "ru"]]);
    for (const base of POOL_KEYS) data.set(`${base}@p1`, "x");
    const storage = { removeItem: (k: string) => void data.delete(k) };
    forgetPoolSettings(storage, "p1");
    forgetPoolSettings(storage, MAIN_POOL);
    expect(Object.fromEntries(data)).toEqual({ "pp-auto-on": "1", "pp-lang": "ru" });
  });
});
