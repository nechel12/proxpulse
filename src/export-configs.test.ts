import { describe, expect, it } from "vitest";
import { deadStub } from "./check-client.ts";
import { buildClash, buildPac, buildProxifier, buildSingBox, buildTemplate, parseProxyUrl } from "./export-configs.ts";
import type { ProxyResult } from "./types.ts";

function res(url: string, latency: number, cc = "DE"): ProxyResult {
  return { ...deadStub(url, ""), url, alive: true, error: null, latency_ms: latency, country_code: cc, ip: "9.9.9.9" };
}

const list = [
  res("socks5://u%40x:p%3Ass@1.2.3.4:1080", 200),
  res("http://5.6.7.8:8080", 100, "NL"),
  res("socks4://9.9.9.9:4145", 300),
  res("https://[2001:db8::1]:443", 400),
];

describe("parseProxyUrl", () => {
  it("splits and decodes normalized URLs", () => {
    expect(parseProxyUrl("socks5://u%40x:p%3Ass@1.2.3.4:1080")).toEqual({ scheme: "socks5", host: "1.2.3.4", port: 1080, user: "u@x", pass: "p:ss" });
    expect(parseProxyUrl("socks5h://h:1")?.scheme).toBe("socks5");
    expect(parseProxyUrl("https://[2001:db8::1]:443")?.host).toBe("2001:db8::1");
    expect(parseProxyUrl("1.2.3.4:80")).toBeNull();
    expect(parseProxyUrl("http://h:70000")).toBeNull();
  });
});

describe("config exports", () => {
  it("writes Clash without SOCKS4, fastest first, with a url-test group", () => {
    const y = buildClash(list);
    expect(y).toContain(`  - name: "NL 5.6.7.8:8080"\n    type: http\n    server: "5.6.7.8"\n    port: 8080`);
    expect(y).toContain(`    username: "u@x"\n    password: "p:ss"`);
    expect(y).toContain("    tls: true");
    expect(y).not.toContain("4145");
    expect(y.indexOf("5.6.7.8")).toBeLessThan(y.indexOf("1.2.3.4"));
    expect(y).toContain("type: url-test");
  });

  it("writes sing-box outbounds for every scheme", () => {
    const j = JSON.parse(buildSingBox(list)) as { outbounds: Record<string, unknown>[] };
    expect(j.outbounds).toHaveLength(5);
    expect(j.outbounds.find((o) => o.server_port === 4145)).toMatchObject({ type: "socks", version: "4" });
    expect(j.outbounds.find((o) => o.server_port === 1080)).toMatchObject({ username: "u@x", password: "p:ss", version: "5" });
    expect(j.outbounds[j.outbounds.length - 1]).toMatchObject({ type: "urltest", tag: "ProxPulse" });
  });

  it("writes a Proxifier profile with escaped credentials", () => {
    const x = buildProxifier([res("socks5://a%26b:c%3Cd@1.2.3.4:1080", 10), res("https://h:443", 5)]);
    expect(x).toContain(`<Proxy id="100" type="SOCKS5">`);
    expect(x).toContain("<Username>a&amp;b</Username>");
    expect(x).toContain("<Password>c&lt;d</Password>");
    expect(x).not.toContain("<Address>h</Address>");
    expect(x).toContain(`<Action type="Proxy">100</Action>`);
  });

  it("writes a PAC chain ending in DIRECT", () => {
    expect(buildPac(list)).toContain(
      `return "PROXY 5.6.7.8:8080; SOCKS5 1.2.3.4:1080; SOCKS 9.9.9.9:4145; HTTPS [2001:db8::1]:443; DIRECT";`,
    );
    expect(buildPac([])).toContain(`return "DIRECT";`);
  });

  it("fills line templates and keeps unknown placeholders", () => {
    expect(buildTemplate([list[0]], "{host}:{port}:{user}:{pass} {country} {ping}ms {nope}")).toBe("1.2.3.4:1080:u@x:p:ss DE 200ms {nope}");
  });
});
