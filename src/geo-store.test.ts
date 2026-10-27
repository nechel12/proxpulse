import { describe, expect, it } from "vitest";
import { GeoCacheStore } from "./geo-store.ts";
import type { ProxyResult } from "./types.ts";

const result = {
  proxy: "u:p@1.2.3.4:8080",
  alive: true,
  ip: "5.6.7.8",
  country: "Testland",
  country_code: "TL",
  city: "Testville",
  isp: "ISP",
  org: "ORG",
  asn: "AS1",
  ip_type: "residential",
} as ProxyResult;

describe("GeoCacheStore", () => {
  it("learns credential-free keys and returns fresh entries", () => {
    const store = new GeoCacheStore();
    store.learn([result]);
    const entries = store.relevant(["1.2.3.4:8080"]);
    expect(entries).toHaveLength(1);
    expect(entries[0].proxy).toBe("1.2.3.4:8080");
    expect(entries[0].country).toBe("Testland");
  });

  it("ignores dead results", () => {
    const store = new GeoCacheStore();
    store.learn([{ ...result, alive: false }]);
    expect(store.relevant(["1.2.3.4:8080"])).toHaveLength(0);
  });
});
