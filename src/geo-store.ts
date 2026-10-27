import type { ProxyResult } from "./types.ts";
import { stripProxyCredentials } from "./parse.ts";
import {
  type GeoEntry,
  type GeoFields,
  isFreshGeo,
  mergeGeoEntry,
} from "./geocache.ts";

const GEO_CACHE_MAX = 2000;
const STORAGE_KEY = "pp-geocache";

export class GeoCacheStore {
  private cache = new Map<string, GeoEntry>();

  load(): void {
    try {
      const raw = localStorage.getItem(STORAGE_KEY);
      if (!raw) return;
      const arr = JSON.parse(raw) as [string, Partial<GeoEntry>][];
      const str = (v: unknown) => (typeof v === "string" ? v : null);
      for (const [k, v] of arr.slice(-GEO_CACHE_MAX)) {
        if (typeof k !== "string" || !v || typeof v !== "object") continue;
        this.cache.set(stripProxyCredentials(k), {
          ip: str(v.ip),
          country: str(v.country),
          country_code: str(v.country_code),
          city: str(v.city),
          isp: str(v.isp),
          org: str(v.org),
          asn: str(v.asn),
          ip_type: str(v.ip_type),
          ts: typeof v.ts === "number" ? v.ts : 0,
        });
      }
    } catch { /* ignore */ }
  }

  learn(list: ProxyResult[]): void {
    let changed = false;
    for (const r of list) {
      if (!r.alive || r.ip == null) continue;
      const key = stripProxyCredentials(r.proxy);
      const next: GeoFields = {
        ip: r.ip,
        country: r.country,
        country_code: r.country_code,
        city: r.city,
        isp: r.isp,
        org: r.org,
        asn: r.asn,
        ip_type: r.ip_type,
      };
      this.cache.set(key, mergeGeoEntry(this.cache.get(key), next));
      changed = true;
    }
    if (changed) {
      while (this.cache.size > GEO_CACHE_MAX) {
        const first = this.cache.keys().next();
        if (first.done) break;
        this.cache.delete(first.value);
      }
      try {
        localStorage.setItem(
          STORAGE_KEY,
          JSON.stringify([...this.cache.entries()].slice(-GEO_CACHE_MAX)),
        );
      } catch { /* quota */ }
    }
  }

  relevant(list: string[]): (GeoEntry & { proxy: string })[] {
    const out: (GeoEntry & { proxy: string })[] = [];
    for (const p of list) {
      const key = stripProxyCredentials(p);
      const f = this.cache.get(key);
      if (f && isFreshGeo(f)) out.push({ proxy: key, ...f });
    }
    return out;
  }
}
