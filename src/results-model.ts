// Pure helpers for the results table: filtering, column sorting and merging
// re-checked entries.

import { geoShort } from "./format.ts";
import { siteHost } from "./sites.ts";
import type { ProxyResult } from "./types.ts";

export type ResultFilter = "all" | "alive" | "dead";
export type SortKey = "proxy" | "proto" | "status" | "ping" | "stab" | "anon" | "geo" | "iptype" | "tls" | "sites" | "speed" | "uptime" | "info";
export type SortDir = "asc" | "desc";
export type Sort = { key: SortKey; dir: SortDir } | null;

export type ResultFilters = {
  status: ResultFilter;
  query: string;
  proto: string;
  /** "elite", "anonymous", "transparent", "unknown" or "" for any. */
  anon: string;
  country: string;
  /** Keep only the fastest alive proxy per exit IP or per ASN. */
  dedupe: Dedupe;
  /** Host of a site the proxy must open, or "" for any. */
  site: string;
  /** "clean" or "listed" by DNS blocklists, or "" for any. */
  dnsbl: string;
  /** "clean" (no ?mod or ?tls), "modified" (either), "unverified" (?unv), or "" for any. */
  tamper: string;
};

export type Dedupe = "" | "ip" | "asn";

export const NO_FILTERS: ResultFilters = { status: "all", query: "", proto: "", anon: "", country: "", dedupe: "", site: "", dnsbl: "", tamper: "" };

/** Whether the answer through this proxy was found rewritten (?mod or ?tls). */
export function isModified(r: ProxyResult): boolean {
  return r.tamper === "modified" || r.tls === "modified";
}

export function countryOf(r: ProxyResult): string {
  return r.country ?? r.country_code ?? "";
}

export function filterResults(results: ProxyResult[], f: ResultFilters): ProxyResult[] {
  const q = f.query.toLowerCase().trim();
  const rows = results.filter((r) => {
    if (f.status === "alive" && !r.alive) return false;
    if (f.status === "dead" && r.alive) return false;
    if (f.proto && r.proto !== f.proto) return false;
    if (f.anon && (r.anonymity ?? "unknown") !== f.anon) return false;
    if (f.country && countryOf(r) !== f.country) return false;
    if (f.site && !(r.sites ?? []).some((s) => s.ok && siteHost(s.url) === f.site)) return false;
    if (f.dnsbl && (!r.dnsbl || (r.dnsbl.listed.length > 0) !== (f.dnsbl === "listed"))) return false;
    if (f.tamper === "clean" && isModified(r)) return false;
    if (f.tamper === "modified" && !isModified(r)) return false;
    if (f.tamper === "unverified" && r.tamper !== "unverified") return false;
    if (q) {
      const hay = [r.proxy, r.proto, r.error ?? "", r.ip ?? "", r.country ?? "", r.country_code ?? "", r.city ?? "", r.isp ?? "", r.org ?? "", r.asn ?? "", r.ip_type ?? "", r.anonymity ?? "", r.tls ?? ""].join(" ").toLowerCase();
      if (!hay.includes(q)) return false;
    }
    return true;
  });
  return f.dedupe ? onePerGroup(rows, f.dedupe) : rows;
}

function groupKey(r: ProxyResult, by: Exclude<Dedupe, "">): string | null {
  if (!r.alive) return null;
  return (by === "ip" ? r.ip : r.asn?.split(/\s/)[0]) || null;
}

/**
 * The fastest alive proxy of each exit IP or ASN, in the original order. An
 * alive proxy whose IP or ASN is unknown (geo skipped at a limit) is kept:
 * it is in no group, so nothing stands for it.
 */
export function onePerGroup(rows: ProxyResult[], by: Exclude<Dedupe, "">): ProxyResult[] {
  const best = new Map<string, ProxyResult>();
  for (const r of rows) {
    const key = groupKey(r, by);
    if (!key) continue;
    const cur = best.get(key);
    if (!cur || (r.latency_ms ?? 1e12) < (cur.latency_ms ?? 1e12)) best.set(key, r);
  }
  const keep = new Set(best.values());
  return rows.filter((r) => keep.has(r) || (r.alive && !groupKey(r, by)));
}

/** How many alive proxies share each exit IP. */
export function exitIpCounts(results: ProxyResult[]): Map<string, number> {
  const counts = new Map<string, number>();
  for (const r of results) {
    if (r.alive && r.ip) counts.set(r.ip, (counts.get(r.ip) ?? 0) + 1);
  }
  return counts;
}

const ANON_RANK: Record<string, number> = { elite: 0, anonymous: 1, transparent: 2 };

/** The value a column sorts by; null sorts last in both directions. */
function sortValue(r: ProxyResult, key: SortKey): string | number | null {
  switch (key) {
    case "proxy": return r.proxy.toLowerCase();
    case "proto": return r.proto;
    case "status": return r.alive ? 0 : 1;
    case "ping": return r.latency_ms;
    case "stab": return r.jitter_ms;
    case "anon": return r.anonymity == null ? null : (ANON_RANK[r.anonymity] ?? 3);
    // As the column shows it: the country code and city, not the full name.
    case "geo": { const g = geoShort(r); return g === "—" ? null : g.toLowerCase(); }
    case "iptype": return r.ip_type;
    case "tls": return r.tls;
    case "sites": return r.sites?.length ? r.sites.filter((s) => s.ok).length : null;
    case "speed": return r.speed?.ok ? r.speed.mbps : null;
    case "uptime": return r.history ? r.history.uptime : null;
    case "info": return r.alive ? r.ip : r.error;
  }
}

export function sortResults(rows: ProxyResult[], sort: Sort): ProxyResult[] {
  if (!sort) return rows;
  const sign = sort.dir === "asc" ? 1 : -1;
  // Decorate once: the comparator would otherwise recompute values n·log n times.
  return rows
    .map((r, i) => ({ r, i, v: sortValue(r, sort.key) }))
    .sort((a, b) => {
      if (a.v == null || b.v == null) {
        if (a.v == null && b.v == null) return a.i - b.i;
        return a.v == null ? 1 : -1;
      }
      const c = typeof a.v === "number" && typeof b.v === "number"
        ? a.v - b.v
        : String(a.v).localeCompare(String(b.v), undefined, { numeric: true });
      return c * sign || a.i - b.i;
    })
    .map((x) => x.r);
}

/** Next sort after a header click: ascending, descending, then none. */
export function nextSort(current: Sort, key: SortKey): Sort {
  if (!current || current.key !== key) return { key, dir: "asc" };
  return current.dir === "asc" ? { key, dir: "desc" } : null;
}

/** The order a finished check shows: alive first, then by ping. */
export function defaultOrder(rows: ProxyResult[]): ProxyResult[] {
  return [...rows].sort((a, b) => Number(b.alive) - Number(a.alive) || (a.latency_ms ?? 1e12) - (b.latency_ms ?? 1e12));
}

/** Replace re-checked entries in place; entries not re-checked stay as they are. */
export function mergeResults(base: ProxyResult[], updates: ProxyResult[]): ProxyResult[] {
  if (updates.length === 0) return base;
  const fresh = new Map(updates.map((r) => [r.proxy, r]));
  return base.map((r) => fresh.get(r.proxy) ?? r);
}

/** Distinct values for the filter menus, most frequent first. */
export function filterOptions(results: ProxyResult[]): { proto: string[]; country: [string, number][]; site: string[] } {
  const protos = new Map<string, number>();
  const countries = new Map<string, number>();
  const sites = new Set<string>();
  for (const r of results) {
    protos.set(r.proto, (protos.get(r.proto) ?? 0) + 1);
    const c = countryOf(r);
    if (c) countries.set(c, (countries.get(c) ?? 0) + 1);
    for (const s of r.sites ?? []) sites.add(siteHost(s.url));
  }
  return {
    proto: [...protos.keys()].sort(),
    country: [...countries.entries()].sort((a, b) => b[1] - a[1] || a[0].localeCompare(b[0])),
    site: [...sites].sort(),
  };
}

/** First and last (exclusive) row to render for a scroll position. */
export function visibleRange(
  scrollTop: number,
  viewHeight: number,
  rowHeight: number,
  count: number,
  overscan: number,
): [number, number] {
  const h = Math.max(1, rowHeight);
  const first = Math.max(0, Math.floor(scrollTop / h) - overscan);
  const last = Math.min(count, Math.ceil((scrollTop + Math.max(viewHeight, h)) / h) + overscan);
  return [Math.min(first, last), last];
}
