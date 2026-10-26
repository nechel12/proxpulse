// Pure display formatting helpers.
import type { ProxyResult } from "./types.ts";

export function pingClass(ms: number | null): string {
  if (ms == null) return "";
  if (ms < 800) return "ping-fast";
  if (ms < 2500) return "ping-mid";
  return "ping-slow";
}

export function anonShort(a: string | null): { txt: string; cls: string } {
  if (!a) return { txt: "—", cls: "" };
  if (a === "elite") return { txt: "elite", cls: "anon-elite" };
  if (a === "anonymous") return { txt: "anon", cls: "anon-anon" };
  if (a === "transparent") return { txt: "transp", cls: "anon-trans" };
  return { txt: a, cls: "" };
}

export function tlsShort(v: string | null): { txt: string; cls: string } {
  if (v === "ok") return { txt: "ok", cls: "anon-elite" };
  if (v === "modified") return { txt: "mod", cls: "anon-trans" };
  if (v === "unverified") return { txt: "n/v", cls: "" };
  return { txt: "—", cls: "" };
}

export function geoShort(r: ProxyResult): string {
  if (r.country_code && r.city) return `${r.country_code} ${r.city}`;
  if (r.country_code) return r.country_code;
  if (r.country) return r.country;
  return "—";
}

export function stabilityShort(r: ProxyResult): string {
  if (!r.alive) return "—";
  if ((r.attempts ?? 1) <= 1) return "—";
  const j = r.jitter_ms == null ? "?" : `±${r.jitter_ms}`;
  const sr = `${Math.round((r.success_rate ?? 1) * 100)}%`;
  return `${j} ${sr}`;
}

export function rowTitle(r: ProxyResult): string {
  const parts: string[] = [];
  if (r.ip) parts.push(`IP ${r.ip}`);
  if (r.country || r.city) parts.push(`geo ${[r.country, r.city].filter(Boolean).join(", ")}`);
  if (r.isp) parts.push(`isp ${r.isp}`);
  if (r.org) parts.push(`org ${r.org}`);
  if (r.asn) parts.push(r.asn);
  if (r.ip_type) parts.push(`type ${r.ip_type}`);
  if (r.anonymity) parts.push(`anon ${r.anonymity}`);
  if (r.tamper) parts.push(`tamper ${r.tamper}`);
  if (r.tls) parts.push(`tls ${r.tls}${r.tls_info ? ` (${r.tls_info})` : ""}`);
  else if (r.tls_info) parts.push(`tls ? (${r.tls_info})`);
  if (r.error) parts.push(r.error);
  return parts.join(" | ");
}

export function poolCountry(r: ProxyResult): string | null {
  return r.country_code ?? r.country ?? null;
}
