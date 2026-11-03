// Exports for proxy clients: Clash, sing-box, Proxifier, PAC and a custom
// line template. Each builder takes checked results and skips proxies the
// target format cannot express.

import type { ProxyResult } from "./types.ts";

export type ProxyParts = {
  scheme: string;
  host: string;
  port: number;
  user: string;
  pass: string;
};

/** Split a normalized proxy URL (`scheme://user:pass@host:port`). */
export function parseProxyUrl(url: string): ProxyParts | null {
  const m = /^([a-z][a-z0-9+.-]*):\/\/(?:([^:@/]*)(?::([^@/]*))?@)?(\[[^\]]+\]|[^:/@]+):(\d{1,5})\/?$/i.exec(url.trim());
  if (!m) return null;
  const port = Number(m[5]);
  if (port < 1 || port > 65535) return null;
  const dec = (s: string | undefined) => {
    try {
      return decodeURIComponent(s ?? "");
    } catch {
      return s ?? "";
    }
  };
  return {
    scheme: m[1].toLowerCase().replace(/^socks5h$/, "socks5").replace(/^socks4a$/, "socks4"),
    host: m[4].replace(/^\[|\]$/g, ""),
    port,
    user: dec(m[2]),
    pass: dec(m[3]),
  };
}

type Entry = { r: ProxyResult; p: ProxyParts; name: string };

/** Parsed entries with unique display names, fastest first. */
function entries(list: ProxyResult[], schemes: string[]): Entry[] {
  const used = new Map<string, number>();
  return list
    .map((r) => ({ r, p: parseProxyUrl(r.url || r.proxy) }))
    .filter((e): e is { r: ProxyResult; p: ProxyParts } => !!e.p && schemes.includes(e.p.scheme))
    .sort((a, b) => (a.r.latency_ms ?? 1e12) - (b.r.latency_ms ?? 1e12))
    .map(({ r, p }) => {
      const base = `${r.country_code ?? r.country ?? "--"} ${p.host}:${p.port}`;
      const n = (used.get(base) ?? 0) + 1;
      used.set(base, n);
      return { r, p, name: n === 1 ? base : `${base} #${n}` };
    });
}

const q = (s: string) => JSON.stringify(s);

export function buildClash(list: ProxyResult[]): string {
  // Clash has no SOCKS4 outbound.
  const items = entries(list, ["http", "https", "socks5"]);
  const lines = ["proxies:"];
  for (const { p, name } of items) {
    lines.push(`  - name: ${q(name)}`);
    lines.push(`    type: ${p.scheme === "socks5" ? "socks5" : "http"}`);
    lines.push(`    server: ${q(p.host)}`);
    lines.push(`    port: ${p.port}`);
    if (p.user) lines.push(`    username: ${q(p.user)}`);
    if (p.pass) lines.push(`    password: ${q(p.pass)}`);
    if (p.scheme === "https") lines.push("    tls: true");
  }
  if (items.length > 0) {
    lines.push("proxy-groups:");
    lines.push(`  - name: "ProxPulse"`);
    lines.push("    type: url-test");
    lines.push(`    url: "http://www.gstatic.com/generate_204"`);
    lines.push("    interval: 300");
    lines.push("    proxies:");
    for (const { name } of items) lines.push(`      - ${q(name)}`);
  }
  return lines.join("\n") + "\n";
}

export function buildSingBox(list: ProxyResult[]): string {
  const items = entries(list, ["http", "https", "socks5", "socks4"]);
  const outbounds: Record<string, unknown>[] = items.map(({ p, name }) => {
    const auth = { ...(p.user ? { username: p.user } : {}), ...(p.pass ? { password: p.pass } : {}) };
    if (p.scheme.startsWith("socks")) {
      return { type: "socks", tag: name, server: p.host, server_port: p.port, version: p.scheme === "socks4" ? "4" : "5", ...auth };
    }
    return {
      type: "http",
      tag: name,
      server: p.host,
      server_port: p.port,
      ...auth,
      ...(p.scheme === "https" ? { tls: { enabled: true } } : {}),
    };
  });
  if (items.length > 0) {
    outbounds.push({ type: "urltest", tag: "ProxPulse", outbounds: items.map((e) => e.name) });
  }
  return JSON.stringify({ outbounds }, null, 2) + "\n";
}

const xml = (s: string) =>
  s.replace(/[&<>"']/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&apos;" }[c] as string));

/** Proxifier profile with the proxies; the default rule uses the fastest one. */
export function buildProxifier(list: ProxyResult[]): string {
  // Proxifier cannot speak TLS to the proxy itself.
  const items = entries(list, ["http", "socks5", "socks4"]);
  const type: Record<string, string> = { http: "HTTPS", socks5: "SOCKS5", socks4: "SOCKS4" };
  const lines = [
    `<?xml version="1.0" encoding="UTF-8" standalone="yes"?>`,
    `<ProxifierProfile version="102" platform="Windows" product_id="0" product_minver="400">`,
    "  <ProxyList>",
  ];
  items.forEach(({ p }, i) => {
    lines.push(`    <Proxy id="${100 + i}" type="${type[p.scheme]}">`);
    lines.push(`      <Address>${xml(p.host)}</Address>`);
    lines.push(`      <Port>${p.port}</Port>`);
    lines.push("      <Options>48</Options>");
    if (p.user || p.pass) {
      lines.push(`      <Authentication enabled="true">`);
      lines.push(`        <Username>${xml(p.user)}</Username>`);
      lines.push(`        <Password>${xml(p.pass)}</Password>`);
      lines.push("      </Authentication>");
    }
    lines.push("    </Proxy>");
  });
  lines.push("  </ProxyList>", "  <ChainList />", "  <RuleList>");
  lines.push(`    <Rule enabled="true">`, "      <Name>Default</Name>");
  lines.push(items.length > 0 ? `      <Action type="Proxy">100</Action>` : `      <Action type="Direct" />`);
  lines.push("    </Rule>", "  </RuleList>", "</ProxifierProfile>");
  return lines.join("\n") + "\n";
}

/** PAC file trying the fastest proxies in order, then a direct connection. */
export function buildPac(list: ProxyResult[], max = 50): string {
  const kind: Record<string, string> = { http: "PROXY", https: "HTTPS", socks5: "SOCKS5", socks4: "SOCKS" };
  const items = entries(list, Object.keys(kind)).slice(0, max);
  const chain = [...items.map(({ p }) => `${kind[p.scheme]} ${p.host.includes(":") ? `[${p.host}]` : p.host}:${p.port}`), "DIRECT"];
  return [
    "// ProxPulse PAC. PAC files cannot carry logins and passwords:",
    "// proxies that need them ask for them in the browser.",
    "function FindProxyForURL(url, host) {",
    `  return ${q(chain.join("; "))};`,
    "}",
    "",
  ].join("\n");
}

/** One line per proxy; `{field}` placeholders are filled, others kept. */
export function buildTemplate(list: ProxyResult[], template: string): string {
  return list
    .map((r) => {
      const p = parseProxyUrl(r.url || r.proxy);
      const values: Record<string, string> = {
        url: r.url || r.proxy,
        scheme: p?.scheme ?? "",
        host: p?.host ?? "",
        port: p ? String(p.port) : "",
        user: p?.user ?? "",
        pass: p?.pass ?? "",
        country: r.country_code ?? r.country ?? "",
        ping: r.latency_ms == null ? "" : String(r.latency_ms),
        ip: r.ip ?? "",
      };
      return template.replace(/\{(\w+)\}/g, (all, key: string) => values[key] ?? all);
    })
    .join("\n");
}
