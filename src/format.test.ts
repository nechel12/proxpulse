import { describe, expect, it } from "vitest";
import {
  anonShort,
  geoShort,
  pingClass,
  poolCountry,
  rowTitle,
  stabilityShort,
  tlsShort,
} from "./format.ts";
import type { ProxyResult } from "./types.ts";

const base: ProxyResult = {
  proxy: "user:pass@1.2.3.4:8080",
  url: "http://user:pass@1.2.3.4:8080",
  proto: "HTTP",
  alive: true,
  latency_ms: 120,
  jitter_ms: 5,
  success_rate: 0.75,
  attempts: 4,
  ip: "1.2.3.4",
  country: "Testland",
  country_code: "TL",
  city: "Testville",
  isp: "ISP",
  org: "ORG",
  asn: "AS1",
  ip_type: "residential/isp",
  anonymity: "elite",
  tamper: "ok",
  tls: "ok",
  tls_info: "TLS 1.3",
  error: null,
};

describe("format helpers", () => {
  it("maps ping thresholds", () => {
    expect(pingClass(null)).toBe("");
    expect(pingClass(100)).toBe("ping-fast");
    expect(pingClass(1000)).toBe("ping-mid");
    expect(pingClass(3000)).toBe("ping-slow");
  });

  it("maps anonymity and TLS states", () => {
    expect(anonShort(null).txt).toBe("—");
    expect(anonShort("elite")).toEqual({ txt: "elite", cls: "anon-elite" });
    expect(anonShort("transparent")).toEqual({ txt: "transp", cls: "anon-trans" });
    expect(tlsShort("ok")).toEqual({ txt: "ok", cls: "anon-elite" });
    expect(tlsShort("modified")).toEqual({ txt: "mod", cls: "anon-trans" });
    expect(tlsShort("unverified")).toEqual({ txt: "n/v", cls: "" });
  });

  it("formats geo, country and stability", () => {
    expect(geoShort(base)).toBe("TL Testville");
    expect(geoShort({ ...base, city: null })).toBe("TL");
    expect(geoShort({ ...base, country_code: null, city: null })).toBe("Testland");
    expect(poolCountry(base)).toBe("TL");
    expect(stabilityShort(base)).toContain("75%");
    expect(stabilityShort({ ...base, attempts: 1 })).toBe("—");
    expect(stabilityShort({ ...base, alive: false })).toBe("—");
  });

  it("includes TLS status in the tooltip", () => {
    const title = rowTitle({ ...base, tls: "unverified", tls_info: "handshake failed" });
    expect(title).toContain("tls unverified (handshake failed)");
    expect(title).toContain("IP 1.2.3.4");
  });
});
