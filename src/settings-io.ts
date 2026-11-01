// Export, import and reset of the app settings kept in localStorage.

/** Settings keys; caches, logs and the ping history are not settings. */
export const SETTINGS_KEYS = [
  "pp-settings",
  "pp-profiles",
  "pp-lang",
  "pp-theme",
  "pp-hide-creds",
  "pp-upd-auto",
  "pp-notify",
  "pp-auto-on",
  "pp-auto-min",
  "pp-auto-webhooks",
  "pp-maxping",
  "pp-webhooks",
  "pp-webhook-format",
  "pp-webhook-creds",
  "pp-ipapi-key",
  "pp-ipapi-pro",
] as const;

const MAX_VALUE_CHARS = 1024 * 1024;
const FILE_VERSION = 1;

/** Backend preferences carried in the file (see `prefs.rs`). */
export type PrefsUpdate = {
  closeToTray?: boolean;
  startMinimized?: boolean;
  startDispatcher?: boolean;
  dispatcher?: { port: number; mode: string; listener: string };
};

export type SettingsFile = {
  app: "ProxPulse";
  kind: "settings";
  version: number;
  exported: string;
  storage: Record<string, string>;
  prefs: PrefsUpdate;
};

export type ParsedSettings = { storage: Record<string, string>; prefs: PrefsUpdate };

type ReadStorage = Pick<Storage, "getItem">;
type WriteStorage = Pick<Storage, "setItem" | "removeItem">;

export function collectSettings(storage: ReadStorage, prefs: PrefsUpdate, now = new Date()): SettingsFile {
  const values: Record<string, string> = {};
  for (const key of SETTINGS_KEYS) {
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
  };
}

function pickPrefs(raw: unknown): PrefsUpdate {
  if (!raw || typeof raw !== "object") return {};
  const src = raw as Record<string, unknown>;
  const out: PrefsUpdate = {};
  for (const k of ["closeToTray", "startMinimized", "startDispatcher"] as const) {
    if (typeof src[k] === "boolean") out[k] = src[k];
  }
  const d = src.dispatcher as Record<string, unknown> | undefined;
  if (
    d && typeof d === "object" &&
    Number.isInteger(d.port) && (d.port as number) >= 1 && (d.port as number) <= 65535 &&
    typeof d.mode === "string" && typeof d.listener === "string"
  ) {
    out.dispatcher = { port: d.port as number, mode: d.mode, listener: d.listener };
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
  const src = file.storage as Record<string, unknown>;
  const storage: Record<string, string> = {};
  for (const key of SETTINGS_KEYS) {
    const v = src[key];
    if (typeof v === "string" && v.length <= MAX_VALUE_CHARS) storage[key] = v;
  }
  return { storage, prefs: pickPrefs(file.prefs) };
}

/** Replace all settings: keys missing from the file return to defaults. */
export function applySettings(storage: WriteStorage, values: Record<string, string>): void {
  for (const key of SETTINGS_KEYS) {
    const v = values[key];
    if (v == null) storage.removeItem(key);
    else storage.setItem(key, v);
  }
}

export function clearSettings(storage: WriteStorage): void {
  applySettings(storage, {});
}
