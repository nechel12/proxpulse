import { describe, expect, it } from "vitest";
import { rowCells } from "./results.ts";
import type { ProxyResult } from "./types.ts";

// Markup a proxy, a subscription or a site could put into any text field.
const EVIL = `"><img src=x onerror=alert(1)><script>alert(2)</script>'`;

function hostile(alive: boolean): ProxyResult {
  return {
    proxy: `1.2.3.4:80${EVIL}`,
    url: `http://1.2.3.4:80`,
    proto: `HTTP${EVIL}`,
    alive,
    latency_ms: 120,
    jitter_ms: 3,
    success_rate: 1,
    attempts: 1,
    ip: `5.6.7.8${EVIL}`,
    country: `Land${EVIL}`,
    country_code: `TL${EVIL}`,
    city: `City${EVIL}`,
    isp: `ISP${EVIL}`,
    org: `Org${EVIL}`,
    asn: `AS1${EVIL}`,
    ip_type: `hosting${EVIL}`,
    anonymity: `weird${EVIL}`,
    tamper: "modified",
    tls: "modified",
    tls_info: `issuer${EVIL}`,
    error: alive ? null : `boom${EVIL}`,
    skipped: [],
    sites: [{ url: `https://site.example/${EVIL}`, ok: false, ms: null, status: null, error: `fail${EVIL}` }],
    speed: { proxy: "x", ok: false, connectMs: null, tlsMs: null, ttfbMs: null, mbps: null, bytes: 0, error: `slow${EVIL}` },
    history: { checks: 2, alive: 1, uptime: 0.5, score: 0.4, lastAlive: null, lastCheck: 1 } as ProxyResult["history"],
    dnsbl: { listed: [`bl.example${EVIL}`], unknown: [] },
  };
}

const ctx = (hideCreds = false) => ({
  t: (key: string, params?: Record<string, string | number>) =>
    params ? `${key} ${Object.values(params).join(" ")}` : key,
  locale: "en",
  hideCreds,
  shared: new Map([[`5.6.7.8${EVIL}`, 3]]),
});

describe("rowCells", () => {
  for (const alive of [true, false]) {
    it(`escapes everything a ${alive ? "live" : "dead"} proxy reported`, () => {
      const html = rowCells(hostile(alive), 0, ctx());
      // Only the row's own markup is real: cells and spans with a class or a
      // title, whose values hold no quote that could end them early.
      const tags = html.match(/<[^>]*>/g) ?? [];
      expect(tags.length).toBeGreaterThan(20);
      for (const tag of tags) expect(tag).toMatch(/^<\/?(td|span)(\s+(class|title)="[^"<>]*")*>$/);
      expect(html).toContain("&lt;script&gt;");
    });
  }

  it("parses into exactly the table's cells", () => {
    const tr = `<tr>${rowCells(hostile(true), 4, ctx(true))}</tr>`;
    expect(tr.match(/<td\b/g)?.length).toBe(14);
    expect(tr.startsWith("<tr><td>5</td>")).toBe(true);
  });
});
