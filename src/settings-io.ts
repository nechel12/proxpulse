// Export, import and reset of the app settings kept in localStorage.

import { MAIN_POOL, POOL_KEYS, scopedKey, validPoolId, type PoolMeta } from "./pools.ts";

/** Settings keys; caches, logs and the ping history are not settings. */
export const SETTINGS_KEYS = [
  "pp-settings",
  "pp-profiles",
  "pp-lang",
  "pp-theme",
  "pp-hide-creds",
  "pp-upd-auto",
  "pp-notify",
  "pp-notify-pool-min",
  "pp-auto-on",
  "pp-auto-min",
  "pp-auto-webhooks",
  "pp-maxping",
  "pp-webhooks",
  "pp-subscriptions",
  "pp-webhook-format",
  "pp-webhook-creds",
  "pp-webhook-changes",
  "pp-webhook-template",
  "pp-ipapi-key",
  "pp-ipapi-pro",
  "pp-export-template",
  "pp-sites",
  "pp-dnsbl",
  "pp-speed-mb",
] as const;

const MAX_VALUE_CHARS = 1024 * 1024;
/** Same cap as the pool in the app. */
const MAX_POOL = 20_000;
const MAX_NAMED_POOLS = 9;
const FILE_VERSION = 1;

/** `DispatcherPrefs` in prefs.rs. */
type DispatcherPrefs = { port: number; mode: string; listener: string; options?: DispatcherOptions; started?: boolean };

/** Backend preferences carried in the file (see `prefs.rs`). */
export type PrefsUpdate = {
  closeToTray?: boolean;
  startMinimized?: boolean;
  startDispatcher?: boolean;
  dispatcher?: DispatcherPrefs;
  /** The dispatchers of the named pools, by pool id. */
  poolDispatchers?: Record<string, DispatcherPrefs>;
  /** The local pool API (`PoolApiPrefs` in pool_api.rs). */
  poolApi?: { enabled: boolean; port: number };
};

/** `DispatchOptions` in dispatch.rs. */
export type DispatcherOptions = { countries: string[]; stickyMin: number; authUser: string; authPass: string };

function pickOptions(raw: unknown): DispatcherOptions | undefined {
  if (!raw || typeof raw !== "object") return undefined;
  const o = raw as Record<string, unknown>;
  const str = (v: unknown) => (typeof v === "string" ? v.slice(0, 255) : "");
  return {
    countries: Array.isArray(o.countries)
      ? o.countries.filter((c): c is string => typeof c === "string").slice(0, 50)
      : [],
    stickyMin: typeof o.stickyMin === "number" && Number.isFinite(o.stickyMin)
      ? Math.max(0, Math.min(1440, Math.floor(o.stickyMin)))
      : 0,
    authUser: str(o.authUser),
    authPass: str(o.authPass),
  };
}

/** A dispatcher pool entry (`DispatchPoolItem` in dispatch.rs). */
export type PoolEntry = { raw: string; url: string; latency: number | null; country: string | null };

/** A named pool in the file; its proxies only when the user chose to include them. */
export type PoolFileEntry = PoolMeta & { items?: PoolEntry[] };

export type SettingsFile = {
  app: "ProxPulse";
  kind: "settings";
  version: number;
  exported: string;
  storage: Record<string, string>;
  prefs: PrefsUpdate;
  /** Only when the user chose to include it; holds credentials. */
  pool?: PoolEntry[];
  /** The named pools; their settings are in `storage` under `key@id`. */
  pools?: PoolFileEntry[];
};

export type ParsedSettings = {
  storage: Record<string, string>;
  prefs: PrefsUpdate;
  /** null when the file has no pool; the current pool is then kept. */
  pool: PoolEntry[] | null;
  /** The named pools; `items` is null when the file has no proxies for one. */
  pools: { meta: PoolMeta; items: PoolEntry[] | null }[];
};

/** Every settings key, with the per-pool keys of the named pools. */
function allKeys(poolIds: readonly string[]): string[] {
  const keys: string[] = [...SETTINGS_KEYS];
  for (const id of poolIds) {
    if (id === MAIN_POOL || !validPoolId(id)) continue;
    for (const base of POOL_KEYS) keys.push(scopedKey(base, id));
  }
  return keys;
}

type ReadStorage = Pick<Storage, "getItem">;
type WriteStorage = Pick<Storage, "setItem" | "removeItem">;

export function collectSettings(
  storage: ReadStorage,
  prefs: PrefsUpdate,
  now = new Date(),
  pool: PoolEntry[] | null = null,
  pools: PoolFileEntry[] = [],
): SettingsFile {
  const named = pools.filter((p) => p.id !== MAIN_POOL);
  const values: Record<string, string> = {};
  for (const key of allKeys(named.map((p) => p.id))) {
    const v = storage.getItem(key);
    if (v != null) values[key] = v;
  }
  return {
    app: "ProxPulse",
    kind: "settings",
    version: FILE_VERSION,
    exported: now.toISOString(),
    storage: values,
    prefs,
    ...(pool ? { pool } : {}),
    ...(named.length ? { pools: named } : {}),
  };
}

function pickDispatcher(raw: unknown): DispatcherPrefs | undefined {
  const d = raw as Record<string, unknown> | undefined;
  if (
    !d || typeof d !== "object" ||
    !Number.isInteger(d.port) || (d.port as number) < 1 || (d.port as number) > 65535 ||
    typeof d.mode !== "string" || typeof d.listener !== "string"
  ) return undefined;
  const options = pickOptions(d.options);
  return {
    port: d.port as number,
    mode: d.mode,
    listener: d.listener,
    ...(options ? { options } : {}),
    ...(d.started === true ? { started: true } : {}),
  };
}

/**
 * The preferences an import sets: the file's, over the current ones. A
 * dispatcher the file describes without its filter and login (an older
 * export) keeps the current ones, and named pools the file does not mention
 * keep their dispatchers.
 */
export function mergeImportedPrefs(file: PrefsUpdate, current: PrefsUpdate): PrefsUpdate {
  const keep = (d: DispatcherPrefs, old: DispatcherPrefs | undefined): DispatcherPrefs =>
    d.options || !old?.options ? d : { ...d, options: old.options };
  const out: PrefsUpdate = { ...file };
  if (file.dispatcher) out.dispatcher = keep(file.dispatcher, current.dispatcher);
  if (file.poolDispatchers) {
    const merged: Record<string, DispatcherPrefs> = { ...(current.poolDispatchers ?? {}) };
    for (const [id, d] of Object.entries(file.poolDispatchers)) merged[id] = keep(d, merged[id]);
    out.poolDispatchers = merged;
  }
  return out;
}

function pickPrefs(raw: unknown): PrefsUpdate {
  if (!raw || typeof raw !== "object") return {};
  const src = raw as Record<string, unknown>;
  const out: PrefsUpdate = {};
  for (const k of ["closeToTray", "startMinimized", "startDispatcher"] as const) {
    if (typeof src[k] === "boolean") out[k] = src[k];
  }
  const dispatcher = pickDispatcher(src.dispatcher);
  if (dispatcher) out.dispatcher = dispatcher;
  const named = src.poolDispatchers;
  if (named && typeof named === "object") {
    const map: Record<string, DispatcherPrefs> = {};
    for (const [id, d] of Object.entries(named as Record<string, unknown>)) {
      const picked = id !== MAIN_POOL && validPoolId(id) ? pickDispatcher(d) : undefined;
      if (picked) map[id] = picked;
    }
    if (Object.keys(map).length) out.poolDispatchers = map;
  }
  const api = src.poolApi as Record<string, unknown> | undefined;
  if (
    api && typeof api === "object" && typeof api.enabled === "boolean" &&
    Number.isInteger(api.port) && (api.port as number) >= 1 && (api.port as number) <= 65535
  ) {
    out.poolApi = { enabled: api.enabled, port: api.port as number };
  }
  return out;
}

function pickPools(raw: unknown): ParsedSettings["pools"] {
  if (!Array.isArray(raw)) return [];
  const out: ParsedSettings["pools"] = [];
  for (const it of raw.slice(0, MAX_NAMED_POOLS)) {
    if (!it || typeof it !== "object") continue;
    const p = it as Record<string, unknown>;
    if (typeof p.id !== "string" || p.id === MAIN_POOL || !validPoolId(p.id)) continue;
    if (out.some((o) => o.meta.id === p.id)) continue;
    out.push({
      meta: { id: p.id, name: typeof p.name === "string" ? p.name.slice(0, 40) : "" },
      items: pickPool(p.items),
    });
  }
  return out;
}

function pickPool(raw: unknown): PoolEntry[] | null {
  if (!Array.isArray(raw)) return null;
  const out: PoolEntry[] = [];
  for (const it of raw.slice(0, MAX_POOL)) {
    if (!it || typeof it !== "object") continue;
    const p = it as Record<string, unknown>;
    if (typeof p.raw !== "string" || !p.raw || p.raw.length > 2048) continue;
    out.push({
      raw: p.raw,
      url: typeof p.url === "string" && p.url ? p.url : p.raw,
      latency: typeof p.latency === "number" && Number.isFinite(p.latency) && p.latency >= 0 ? Math.round(p.latency) : null,
      country: typeof p.country === "string" && p.country ? p.country.slice(0, 64) : null,
    });
  }
  return out;
}

/** Parse an exported file; null when it is not a ProxPulse settings file. */
export function parseSettingsFile(text: string): ParsedSettings | null {
  let data: unknown;
  try {
    data = JSON.parse(text);
  } catch {
    return null;
  }
  if (!data || typeof data !== "object") return null;
  const file = data as Record<string, unknown>;
  if (file.app !== "ProxPulse" || file.kind !== "settings" || file.version !== FILE_VERSION) return null;
  if (!file.storage || typeof file.storage !== "object") return null;
  const pools = pickPools(file.pools);
  const src = file.storage as Record<string, unknown>;
  const storage: Record<string, string> = {};
  for (const key of allKeys(pools.map((p) => p.meta.id))) {
    const v = src[key];
    if (typeof v === "string" && v.length <= MAX_VALUE_CHARS) storage[key] = v;
  }
  return { storage, prefs: pickPrefs(file.prefs), pool: pickPool(file.pool), pools };
}

/**
 * Replace all settings: keys missing from the file return to defaults.
 * `poolIds` names the pools whose own settings are replaced too.
 */
export function applySettings(
  storage: WriteStorage,
  values: Record<string, string>,
  poolIds: readonly string[] = [],
): void {
  for (const key of allKeys(poolIds)) {
    const v = values[key];
    if (v == null) storage.removeItem(key);
    else storage.setItem(key, v);
  }
}

export function clearSettings(storage: WriteStorage, poolIds: readonly string[] = []): void {
  applySettings(storage, {}, poolIds);
}
