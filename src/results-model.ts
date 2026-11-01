// Pure helpers for the results table: filtering, column sorting and merging
// re-checked entries.

import type { ProxyResult } from "./types.ts";

export type ResultFilter = "all" | "alive" | "dead";
export type SortKey = "proxy" | "proto" | "status" | "ping" | "stab" | "anon" | "geo" | "iptype" | "tls" | "info";
export type SortDir = "asc" | "desc";
export type Sort = { key: SortKey; dir: SortDir } | null;

export type ResultFilters = {
  status: ResultFilter;
  query: string;
  proto: string;
  /** "elite", "anonymous", "transparent", "unknown" or "" for any. */
  anon: string;
  country: string;
};

export const NO_FILTERS: ResultFilters = { status: "all", query: "", proto: "", anon: "", country: "" };

export function countryOf(r: ProxyResult): string {
  return r.country ?? r.country_code ?? "";
}

export function filterResults(results: ProxyResult[], f: ResultFilters): ProxyResult[] {
  const q = f.query.toLowerCase().trim();
  return results.filter((r) => {
    if (f.status === "alive" && !r.alive) return false;
    if (f.status === "dead" && r.alive) return false;
    if (f.proto && r.proto !== f.proto) return false;
    if (f.anon && (r.anonymity ?? "unknown") !== f.anon) return false;
    if (f.country && countryOf(r) !== f.country) return false;
    if (q) {
      const hay = [r.proxy, r.proto, r.error ?? "", r.ip ?? "", r.country ?? "", r.country_code ?? "", r.city ?? "", r.isp ?? "", r.org ?? "", r.asn ?? "", r.ip_type ?? "", r.anonymity ?? "", r.tls ?? ""].join(" ").toLowerCase();
      if (!hay.includes(q)) return false;
    }
    return true;
  });
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
    case "geo": return countryOf(r).toLowerCase() || null;
    case "iptype": return r.ip_type;
    case "tls": return r.tls;
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
export function filterOptions(results: ProxyResult[]): { proto: string[]; country: [string, number][] } {
  const protos = new Map<string, number>();
  const countries = new Map<string, number>();
  for (const r of results) {
    protos.set(r.proto, (protos.get(r.proto) ?? 0) + 1);
    const c = countryOf(r);
    if (c) countries.set(c, (countries.get(c) ?? 0) + 1);
  }
  return {
    proto: [...protos.keys()].sort(),
    country: [...countries.entries()].sort((a, b) => b[1] - a[1] || a[0].localeCompare(b[0])),
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
