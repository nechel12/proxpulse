// Geo cache merge/freshness helpers (pure, covered by geocache.test.ts).

export type GeoFields = {
  ip: string | null;
  country: string | null;
  country_code: string | null;
  city: string | null;
  isp: string | null;
  org: string | null;
  asn: string | null;
  ip_type: string | null;
};

export type GeoEntry = GeoFields & { ts: number };

export const GEO_CACHE_TTL_MS = 6 * 60 * 60 * 1000;

export function isFreshGeo(entry: { ts?: number }, now = Date.now()): boolean {
  return typeof entry.ts === "number" && now - entry.ts < GEO_CACHE_TTL_MS;
}

/**
 * Store a check result in the cache. For the same exit IP, null fields do
 * not wipe previously known data. A different exit IP invalidates the old
 * geo entirely: mixing countries of two IPs would be wrong.
 */
export function mergeGeoEntry(
  prev: GeoEntry | undefined,
  next: GeoFields,
  now = Date.now(),
): GeoEntry {
  if (!prev) return { ...next, ts: now };
  const sameIp = !!prev.ip && !!next.ip && prev.ip === next.ip;
  if (!sameIp) return { ...next, ts: now };
  const pick = <K extends keyof GeoFields>(k: K): GeoFields[K] =>
    next[k] != null ? next[k] : prev[k];
  return {
    ip: pick("ip"),
    country: pick("country"),
    country_code: pick("country_code"),
    city: pick("city"),
    isp: pick("isp"),
    org: pick("org"),
    asn: pick("asn"),
    ip_type: pick("ip_type"),
    ts: now,
  };
}
