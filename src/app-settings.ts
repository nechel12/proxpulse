import { invoke } from "@tauri-apps/api/core";
import { askChoice, askConfirm } from "./ui.ts";
import { applyTheme, loadThemePref, parseThemePref, saveThemePref, watchSystemTheme, type ThemePref } from "./theme.ts";
import {
  applySettings,
  clearSettings,
  collectSettings,
  parseSettingsFile,
  type PoolEntry,
  type PoolFileEntry,
  type PrefsUpdate,
} from "./settings-io.ts";
import { MAIN_POOL, forgetPoolSettings, type PoolMeta } from "./pools.ts";

type Translate = (key: string, params?: Record<string, string | number>) => string;
type StatusSetter = (id: string, render: () => string) => void;

type Deps = {
  t: Translate;
  setStatusText: StatusSetter;
  logErr: (msg: string) => void;
  isTauriRuntime: () => boolean;
  onHideCreds: () => void;
  /** The pool registry, for the per-pool settings. */
  pools: () => PoolMeta[];
  /** A check runs: importing or resetting settings reloads the page. */
  isBusy?: () => boolean;
};

/** Backend preferences (`prefs.rs`). */
type BackendPrefs = Required<PrefsUpdate>;

/**
 * What a reset sets, and an import for what its file leaves out. The
 * dispatchers' filters and logins are included: the backend applies them to
 * running dispatchers at once.
 */
const DEFAULT_PREFS: PrefsUpdate = {
  closeToTray: false,
  startMinimized: true,
  startDispatcher: false,
  dispatcher: { port: 1080, mode: "round-robin", listener: "both" },
  poolDispatchers: {},
  poolApi: { enabled: false, port: 1090 },
};

/** Start or stop the pool API as the preferences say. */
async function applyPoolApi(api: PrefsUpdate["poolApi"]): Promise<void> {
  if (api?.enabled) await invoke("start_pool_api", { port: api.port });
  else await invoke("stop_pool_api");
}

export function readFlag(key: string, fallback: boolean): boolean {
  try {
    const v = localStorage.getItem(key);
    return v == null ? fallback : v === "1";
  } catch {
    return fallback;
  }
}

export function writeFlag(key: string, value: boolean): void {
  try {
    localStorage.setItem(key, value ? "1" : "0");
  } catch { /* ignore */ }
}

export function clampPoolMin(v: number): number {
  return Number.isFinite(v) ? Math.max(1, Math.min(20_000, Math.floor(v))) : 1;
}

function readPoolMin(): number {
  try {
    return clampPoolMin(Number(localStorage.getItem("pp-notify-pool-min") ?? 1));
  } catch {
    return 1;
  }
}

const el = <T extends HTMLElement>(id: string) => document.getElementById(id) as T | null;

function setSwitch(id: string, on: boolean): void {
  el(id)?.setAttribute("aria-checked", String(on));
}

/** Global app settings on the Settings tab. */
export class AppSettings {
  hideCreds = readFlag("pp-hide-creds", false);
  updateOnStart = readFlag("pp-upd-auto", true);
  notifications = readFlag("pp-notify", true);
  /** Notify when a scheduled check leaves fewer proxies than this. */
  poolMin = readPoolMin();
  theme: ThemePref = loadThemePref();
  private autostart = false;

  constructor(private deps: Deps) {}

  init(): void {
    const theme = el<HTMLSelectElement>("pref-theme");
    if (theme) {
      theme.value = this.theme;
      theme.addEventListener("change", () => {
        this.theme = parseThemePref(theme.value);
        saveThemePref(this.theme);
        applyTheme(this.theme);
      });
    }
    watchSystemTheme(() => this.theme);

    this.bindFlag("pref-hide-creds", "pp-hide-creds", () => this.hideCreds, (v) => {
      this.hideCreds = v;
      this.deps.onHideCreds();
    });
    this.bindFlag("pref-upd-auto", "pp-upd-auto", () => this.updateOnStart, (v) => { this.updateOnStart = v; });
    this.bindFlag("pref-notify", "pp-notify", () => this.notifications, (v) => {
      this.notifications = v;
      this.renderNotify();
    });
    const poolMin = el<HTMLInputElement>("pref-pool-min");
    if (poolMin) {
      poolMin.value = String(this.poolMin);
      poolMin.addEventListener("change", () => {
        this.poolMin = clampPoolMin(Number(poolMin.value));
        poolMin.value = String(this.poolMin);
        try {
          localStorage.setItem("pp-notify-pool-min", String(this.poolMin));
        } catch { /* ignore */ }
      });
    }
    this.renderNotify();

    el<HTMLSelectElement>("pref-close")?.addEventListener("change", (e) => {
      void this.savePrefs({ closeToTray: (e.target as HTMLSelectElement).value === "tray" });
    });
    this.bindPref("pref-minimized", "startMinimized");
    this.bindPref("pref-dispatch", "startDispatcher");
    el("pref-autostart")?.addEventListener("click", () => void this.toggleAutostart());

    el("btn-settings-export")?.addEventListener("click", () => void this.exportSettings());
    el<HTMLInputElement>("settings-file")?.addEventListener("change", (e) => {
      const input = e.target as HTMLInputElement;
      const file = input.files?.[0];
      input.value = "";
      if (file) void this.importSettings(file);
    });
    el("btn-settings-reset")?.addEventListener("click", () => void this.resetSettings());

    void this.loadBackend();
  }

  private bindFlag(id: string, key: string, get: () => boolean, set: (v: boolean) => void): void {
    setSwitch(id, get());
    el(id)?.addEventListener("click", () => {
      set(!get());
      writeFlag(key, get());
      setSwitch(id, get());
    });
  }

  private bindPref(id: string, key: "startMinimized" | "startDispatcher"): void {
    el(id)?.addEventListener("click", () => {
      const on = el(id)?.getAttribute("aria-checked") !== "true";
      setSwitch(id, on);
      void this.savePrefs({ [key]: on });
    });
  }

  private renderNotify(): void {
    el("pref-pool-min-row")?.classList.toggle("off", !this.notifications);
    const input = el<HTMLInputElement>("pref-pool-min");
    if (input) input.disabled = !this.notifications;
  }

  private renderPrefs(p: PrefsUpdate): void {
    const close = el<HTMLSelectElement>("pref-close");
    if (close && p.closeToTray != null) close.value = p.closeToTray ? "tray" : "quit";
    if (p.startMinimized != null) setSwitch("pref-minimized", p.startMinimized);
    if (p.startDispatcher != null) setSwitch("pref-dispatch", p.startDispatcher);
  }

  private renderAutostart(): void {
    setSwitch("pref-autostart", this.autostart);
    el("pref-minimized-row")?.classList.toggle("off", !this.autostart);
    const min = el<HTMLButtonElement>("pref-minimized");
    if (min) min.disabled = !this.autostart;
  }

  private async loadBackend(): Promise<void> {
    this.renderAutostart();
    if (!this.deps.isTauriRuntime()) return;
    try {
      this.renderPrefs(await invoke<BackendPrefs>("get_app_prefs"));
    } catch { /* older backend */ }
    try {
      this.autostart = await invoke<boolean>("autostart_enabled");
    } catch (e) {
      this.deps.logErr("autostart: " + String(e).slice(0, 200));
    }
    this.renderAutostart();
  }

  private async savePrefs(update: PrefsUpdate): Promise<void> {
    try {
      this.renderPrefs(await invoke<BackendPrefs>("set_app_prefs", { prefs: update }));
    } catch (e) {
      const msg = String(e).slice(0, 120);
      this.deps.setStatusText("startup-status", () => this.deps.t("prefs_err", { e: msg }));
      void this.loadBackend();
    }
  }

  private async toggleAutostart(): Promise<void> {
    try {
      this.autostart = await invoke<boolean>("set_autostart", { enabled: !this.autostart });
      this.deps.setStatusText("startup-status", () => "");
    } catch (e) {
      const msg = String(e).slice(0, 120);
      this.deps.logErr("autostart: " + msg);
      this.deps.setStatusText("startup-status", () => this.deps.t("autostart_err", { e: msg }));
    }
    this.renderAutostart();
  }

  private async exportSettings(): Promise<void> {
    const { t, setStatusText } = this.deps;
    let prefs: PrefsUpdate = {};
    let pool: PoolEntry[] = [];
    const named: PoolFileEntry[] = this.deps.pools().filter((p) => p.id !== MAIN_POOL).map((p) => ({ ...p }));
    try {
      prefs = await invoke<BackendPrefs>("get_app_prefs");
      pool = await invoke<PoolEntry[]>("get_dispatch_pool");
      for (const p of named) p.items = await invoke<PoolEntry[]>("get_dispatch_pool", { pool: p.id });
    } catch { /* browser preview or older backend */ }
    // The pools carry logins and passwords: include them only on request.
    const total = pool.length + named.reduce((n, p) => n + (p.items?.length ?? 0), 0);
    const withPool = total > 0 &&
      await askChoice(t("settings_pool_q", { n: total }), t("settings_pool_yes"), t("settings_pool_no"));
    if (!withPool) for (const p of named) delete p.items;
    const data = collectSettings(localStorage, prefs, new Date(), withPool ? pool : null, named);
    const day = data.exported.slice(0, 10);
    try {
      const path = await invoke<string | null>("export_text_file", {
        contents: JSON.stringify(data, null, 2),
        defaultName: `proxpulse-settings-${day}.json`,
        filterName: "JSON",
        extensions: ["json"],
      });
      if (path) setStatusText("data-status", () => t("settings_saved", { path }));
    } catch (e) {
      const msg = String(e).slice(0, 120);
      setStatusText("data-status", () => t("st_savefail", { e: msg }));
    }
  }

  /** Both reload the page, which would lose track of a check in progress. */
  private refuseWhileBusy(): boolean {
    if (!this.deps.isBusy?.()) return false;
    this.deps.setStatusText("data-status", () => this.deps.t("settings_busy"));
    return true;
  }

  private async importSettings(file: File): Promise<void> {
    const { t, setStatusText } = this.deps;
    if (this.refuseWhileBusy()) return;
    const parsed = file.size > 16 * 1024 * 1024 ? null : parseSettingsFile(await file.text());
    if (!parsed) {
      setStatusText("data-status", () => t("settings_bad"));
      return;
    }
    const proxies = (parsed.pool?.length ?? 0) + parsed.pools.reduce((n, p) => n + (p.items?.length ?? 0), 0);
    const question = parsed.pool || parsed.pools.some((p) => p.items)
      ? t("settings_import_pool_confirm", { n: proxies })
      : t("settings_import_confirm");
    if (!(await askConfirm(question))) return;
    const ids = new Set([...this.deps.pools().map((p) => p.id), ...parsed.pools.map((p) => p.meta.id)]);
    applySettings(localStorage, parsed.storage, [...ids]);
    try {
      const prefs = { ...DEFAULT_PREFS, ...parsed.prefs };
      await invoke("set_app_prefs", { prefs });
      await applyPoolApi(prefs.poolApi).catch(() => { /* the port may be taken */ });
      if (parsed.pool) await invoke("set_dispatch_pool", { items: parsed.pool });
      if (parsed.pools.length) {
        const restored = await invoke<PoolMeta[]>("restore_pools", { pools: parsed.pools.map((p) => p.meta) });
        const known = new Set(restored.map((p) => p.id));
        for (const p of parsed.pools) {
          // A full registry takes no more pools: drop what was written for them.
          if (!known.has(p.meta.id)) forgetPoolSettings(localStorage, p.meta.id);
          else if (p.items) await invoke("set_dispatch_pool", { pool: p.meta.id, items: p.items });
        }
      }
    } catch { /* browser preview or older backend */ }
    location.reload();
  }

  private async resetSettings(): Promise<void> {
    if (this.refuseWhileBusy()) return;
    if (!(await askConfirm(this.deps.t("settings_reset_confirm")))) return;
    clearSettings(localStorage, this.deps.pools().map((p) => p.id));
    try {
      await invoke("set_app_prefs", { prefs: DEFAULT_PREFS });
      await applyPoolApi(DEFAULT_PREFS.poolApi);
      await invoke("set_autostart", { enabled: false });
    } catch { /* browser preview or older backend */ }
    location.reload();
  }
}
