import { describe, expect, it } from "vitest";
import {
  anonShort,
  errorText,
  formatDuration,
  escapeHtml,
  geoShort,
  maskProxy,
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

  it("lists deep checks skipped at their limit", () => {
    expect(rowTitle({ ...base, skipped: ["geo", "tls"] })).toContain("skipped geo, tls");
    expect(rowTitle({ ...base, skipped: [] })).not.toContain("skipped");
  });
});

describe("escapeHtml", () => {
  it("escapes markup characters", () => {
    expect(escapeHtml(`<a href="x">&'"</a>`)).toBe("&lt;a href=&quot;x&quot;&gt;&amp;&#39;&quot;&lt;/a&gt;");
  });
});

describe("maskProxy", () => {
  it("hides the userinfo of the normalized URL", () => {
    expect(maskProxy("1.2.3.4:8080:user:pass", "http://user:pass@1.2.3.4:8080")).toBe("http://•••@1.2.3.4:8080");
    expect(maskProxy("socks5://u:p@[2001:db8::1]:1080", "socks5://u:p@[2001:db8::1]:1080")).toBe("socks5://•••@[2001:db8::1]:1080");
  });

  it("keeps proxies without credentials as entered", () => {
    expect(maskProxy("1.2.3.4:8080", "http://1.2.3.4:8080")).toBe("1.2.3.4:8080");
  });

  it("falls back to the raw line without a URL", () => {
    expect(maskProxy("user:pass@1.2.3.4:8080")).toBe("•••@1.2.3.4:8080");
    expect(maskProxy("http://user:pass@1.2.3.4:8080", null)).toBe("http://•••@1.2.3.4:8080");
    expect(maskProxy("1.2.3.4:8080")).toBe("1.2.3.4:8080");
  });
});

describe("errorText", () => {
  const t = (key: string, p?: Record<string, string | number>) => `${key}${p?.code ? `:${p.code}` : ""}`;
  it("translates the backend's short error words", () => {
    expect(errorText("timeout", t)).toBe("err_timeout");
    expect(errorText("http 502", t)).toBe("err_http:502");
    expect(errorText("unexpected answer (http 200)", t)).toBe("err_unexpected_http:200");
    expect(errorText("http: timeout / socks5: connection refused", t)).toBe("http: err_timeout / socks5: err_refused");
    expect(errorText("something new", t)).toBe("something new");
  });
});

describe("formatDuration", () => {
  it("reads at a glance", () => {
    expect(formatDuration(45)).toBe("45 s");
    expect(formatDuration(200)).toBe("3 min 20 s");
    expect(formatDuration(3900)).toBe("1 h 05 min");
    expect(formatDuration(200, { s: "с", min: "мин", h: "ч" })).toBe("3 мин 20 с");
  });
});
