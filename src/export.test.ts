import { describe, expect, it } from "vitest";
import { buildCsv, buildJson, buildTxt } from "./export.ts";
import type { ProxyResult } from "./types.ts";

const base: ProxyResult = {
  proxy: "user:pass@1.2.3.4:8080",
  url: "http://user:pass@1.2.3.4:8080",
  proto: "HTTP",
  alive: true,
  latency_ms: 120,
  jitter_ms: 0,
  success_rate: 1,
  attempts: 1,
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

describe("export", () => {
  it("writes the normalized URL for TXT", () => {
    expect(buildTxt([base])).toBe("http://user:pass@1.2.3.4:8080");
  });

  it("includes both raw and normalized values in CSV", () => {
    const csv = buildCsv([base]);
    expect(csv.split("\n")[0]).toContain("proxy,url,proto");
    expect(csv.split("\n")[1]).toContain("user:pass@1.2.3.4:8080");
    expect(csv.split("\n")[1]).toContain("http://user:pass@1.2.3.4:8080");
  });

  it("guards CSV formula characters", () => {
    const csv = buildCsv([{ ...base, proxy: "=cmd", url: "+1", error: "@x" }]);
    const row = csv.split("\n")[1];
    expect(row).toContain("'=cmd");
    expect(row).toContain("'+1");
    expect(row).toContain("'@x");
  });

  it("serializes full results as JSON", () => {
    expect(JSON.parse(buildJson([base]))[0].url).toBe(
      "http://user:pass@1.2.3.4:8080"
    );
  });
});
