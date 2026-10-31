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

/** `scheme://host:port` without userinfo. */
const URL_KEY_RE = /^[a-z][a-z0-9+.-]*:\/\/[^\s@/]+$/i;

/**
 * Cache key for a checked proxy: the normalized URL from the backend with
 * credentials removed. Raw input lines cannot be used: column formats such
 * as `host:port:user:pass` carry the password without an `@`.
 */
export function geoKey(r: Pick<ProxyResult, "url" | "proxy">): string | null {
  const key = stripProxyCredentials(r.url || r.proxy);
  return URL_KEY_RE.test(key) ? key : null;
}

export class GeoCacheStore {
  private cache = new Map<string, GeoEntry>();

  load(): void {
    let dropped = false;
    try {
      const raw = localStorage.getItem(STORAGE_KEY);
      if (!raw) return;
      const arr = JSON.parse(raw) as [string, Partial<GeoEntry>][];
      const str = (v: unknown) => (typeof v === "string" ? v : null);
      for (const [k, v] of arr.slice(-GEO_CACHE_MAX)) {
        if (typeof k !== "string" || !v || typeof v !== "object") continue;
        // Older versions keyed entries by the raw input line, which could
        // contain a password; such entries are dropped and rewritten below.
        if (!URL_KEY_RE.test(k)) {
          dropped = true;
          continue;
        }
        this.cache.set(k, {
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
    if (dropped) this.save();
  }

  private save(): void {
    try {
      localStorage.setItem(
        STORAGE_KEY,
        JSON.stringify([...this.cache.entries()].slice(-GEO_CACHE_MAX)),
      );
    } catch { /* quota */ }
  }

  learn(list: ProxyResult[]): void {
    let changed = false;
    for (const r of list) {
      if (!r.alive || r.ip == null) continue;
      const key = geoKey(r);
      if (!key) continue;
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
      this.save();
    }
  }

  /**
   * All fresh entries. Keys are normalized URLs, which only the backend can
   * derive from raw input lines, so it does the lookup itself.
   */
  fresh(): (GeoEntry & { proxy: string })[] {
    const out: (GeoEntry & { proxy: string })[] = [];
    for (const [key, f] of this.cache) {
      if (isFreshGeo(f)) out.push({ proxy: key, ...f });
    }
    return out;
  }
}
