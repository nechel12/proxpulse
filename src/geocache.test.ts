import { describe, expect, it } from "vitest";
import { GEO_CACHE_TTL_MS, isFreshGeo, mergeGeoEntry } from "./geocache.ts";
import type { GeoFields, GeoEntry } from "./geocache.ts";

const base: GeoFields = {
  ip: "1.2.3.4",
  country: "Testland",
  country_code: "TL",
  city: "Testville",
  isp: "ISP",
  org: "ORG",
  asn: "AS1",
  ip_type: "residential/isp",
};

describe("geocache", () => {
  it("returns fresh entries only", () => {
    const now = 1_000_000;
    expect(isFreshGeo({ ts: now }, now)).toBe(true);
    expect(isFreshGeo({ ts: now - GEO_CACHE_TTL_MS - 1 }, now)).toBe(false);
    expect(isFreshGeo({}, now)).toBe(false);
  });

  it("stores the first entry with a timestamp", () => {
    expect(mergeGeoEntry(undefined, base, 42)).toEqual({ ...base, ts: 42 });
  });

  it("does not wipe known fields with nulls for the same IP", () => {
    const prev: GeoEntry = { ...base, ts: 1 };
    const merged = mergeGeoEntry(
      prev,
      { ...base, country: null, city: null, isp: null },
      2,
    );
    expect(merged.country).toBe("Testland");
    expect(merged.city).toBe("Testville");
    expect(merged.isp).toBe("ISP");
    expect(merged.ts).toBe(2);
  });

  it("replaces stale geo when the exit IP changes", () => {
    const prev: GeoEntry = { ...base, ts: 1 };
    const merged = mergeGeoEntry(
      prev,
      { ...base, ip: "5.6.7.8", country: null, city: null },
      2,
    );
    expect(merged.ip).toBe("5.6.7.8");
    expect(merged.country).toBeNull();
    expect(merged.city).toBeNull();
    expect(merged.ts).toBe(2);
  });
});
