import { csvCell } from "./csv.ts";
import type { ProxyResult } from "./types.ts";

export function buildTxt(list: ProxyResult[]): string {
  return list.map((r) => r.url || r.proxy).join("\n");
}

export function buildJson(list: ProxyResult[]): string {
  return JSON.stringify(list, null, 2);
}

export function buildCsv(list: ProxyResult[]): string {
  const head = ["proxy", "url", "proto", "alive", "latency_ms", "jitter_ms", "success_rate", "attempts", "ip", "country_code", "country", "city", "isp", "org", "asn", "ip_type", "anonymity", "tamper", "tls", "tls_info", "error"];
  const lines = [head.join(",")];
  for (const r of list) {
    lines.push([
      r.proxy, r.url, r.proto, r.alive ? 1 : 0, r.latency_ms ?? "", r.jitter_ms ?? "",
      Math.round((r.success_rate ?? 0) * 100), r.attempts ?? "", r.ip ?? "",
      r.country_code ?? "", r.country ?? "", r.city ?? "", r.isp ?? "", r.org ?? "",
      r.asn ?? "", r.ip_type ?? "", r.anonymity ?? "", r.tamper ?? "",
      r.tls ?? "", r.tls_info ?? "", r.error ?? "",
    ].map(csvCell).join(","));
  }
  return lines.join("\n");
}
