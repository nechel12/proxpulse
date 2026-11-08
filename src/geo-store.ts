import type { ProxyResult } from "./types.ts";
import {
  type GeoEntry,
  type GeoFields,
  isFreshGeo,
  mergeGeoEntry,
} from "./geocache.ts";

const GEO_CACHE_MAX = 2000;
const STORAGE_KEY = "pp-geocache";

/** Remove `user:pass@` from a proxy URL, keeping the scheme. */
export function stripProxyCredentials(raw: string): string {
  const at = raw.lastIndexOf("@");
  if (at < 0) return raw;
  const host = raw.slice(at + 1);
  if (!host) return raw;
  const schemeSep = raw.indexOf("://");
  if (schemeSep >= 0 && schemeSep < at) {
    return raw.slice(0, schemeSep + 3) + host;
  }
  return host;
}

/** `scheme://host:port` without userinfo, and the login's hash if any. */
const URL_KEY_RE = /^[a-z][a-z0-9+.-]*:\/\/[^\s@/#]+(#u[0-9a-f]{8})?$/i;

/** 32-bit FNV-1a of the UTF-8 bytes, as hex; `fnv1a32` in proxy.rs. */
export function fnv1a32(s: string): string {
  let h = 0x811c9dc5;
  for (const b of new TextEncoder().encode(s)) {
    h = Math.imul(h ^ b, 0x01000193) >>> 0;
  }
  return h.toString(16).padStart(8, "0");
}

/**
 * Cache key for a checked proxy: the normalized URL from the backend with
 * credentials removed, plus a hash of the username, since gateways choose
 * the exit country by username. Raw input lines cannot be used: column
 * formats such as `host:port:user:pass` carry the password without an `@`.
 * Must match `geo_key` in proxy.rs.
 */
export function geoKey(r: Pick<ProxyResult, "url" | "proxy">): string | null {
  const url = r.url || r.proxy;
  const base = stripProxyCredentials(url);
  if (!URL_KEY_RE.test(base)) return null;
  const rest = url.includes("://") ? url.slice(url.indexOf("://") + 3) : url;
  const at = rest.lastIndexOf("@");
  const user = at < 0 ? "" : rest.slice(0, at).split(":")[0];
  return user ? `${base}#u${fnv1a32(user)}` : base;
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
      const merged = mergeGeoEntry(this.cache.get(key), next);
      // Re-insert so the Map order is "least recently confirmed first" and
      // eviction drops stale entries, not the ones just seen again.
      this.cache.delete(key);
      this.cache.set(key, merged);
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
