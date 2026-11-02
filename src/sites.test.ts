import { describe, expect, it } from "vitest";
import { deadStub } from "./check-client.ts";
import { NO_FILTERS, filterOptions, filterResults, sortResults } from "./results-model.ts";
import { parseCustomSites, siteHost, siteSummary } from "./sites.ts";
import type { ProxyResult } from "./types.ts";

const probe = (url: string, ok: boolean) => ({ url, ok, ms: ok ? 100 : null, status: ok ? 200 : null, error: ok ? null : "timeout" });

function res(proxy: string, sites: ReturnType<typeof probe>[], mbps?: number): ProxyResult {
  return {
    ...deadStub(proxy, ""),
    alive: true,
    error: null,
    sites,
    ...(mbps == null ? {} : { speed: { proxy, ok: true, connectMs: 1, tlsMs: 1, ttfbMs: 1, mbps, bytes: 1, error: null } }),
  };
}

describe("sites", () => {
  it("accepts hosts and URLs as custom sites", () => {
    expect(parseCustomSites("example.com, https://web.telegram.org/k/  nope ftp://x.org")).toEqual([
      "https://example.com",
      "https://web.telegram.org/k/",
    ]);
    expect(siteHost("https://www.youtube.com/watch")).toBe("youtube.com");
  });

  it("summarizes, filters and sorts by reachable sites", () => {
    const a = res("a:1", [probe("https://www.youtube.com", true), probe("https://discord.com", false)], 5);
    const b = res("b:1", [probe("https://www.youtube.com", true), probe("https://discord.com", true)], 50);
    const c = res("c:1", []);
    expect(siteSummary(a)).toEqual({ ok: 1, total: 2 });
    expect(siteSummary(c)).toBeNull();
    expect(filterResults([a, b, c], { ...NO_FILTERS, site: "discord.com" })).toEqual([b]);
    expect(filterOptions([a, b, c]).site).toEqual(["discord.com", "youtube.com"]);
    expect(sortResults([a, b, c], { key: "sites", dir: "desc" }).map((r) => r.proxy)).toEqual(["b:1", "a:1", "c:1"]);
    expect(sortResults([a, b, c], { key: "speed", dir: "desc" }).map((r) => r.proxy)).toEqual(["b:1", "a:1", "c:1"]);
  });
});
