import { invoke } from "@tauri-apps/api/core";
import type { CheckSettings } from "./check-client.ts";
import { logErr } from "./diagnostics.ts";
import { MAIN_POOL, scopedKey } from "./pools.ts";
import { parseUrlList } from "./url-list.ts";
import { DEFAULT_TEMPLATE } from "./webhooks.ts";

type Deps = {
  readCheckSettings: () => CheckSettings;
  /** Pools other than the shown one; their schedules come from storage. */
  otherPools: () => string[];
  maxPingMs: () => number;
  webhooks: { getUrls(): string[]; includeCreds: boolean; template: string; changesOnly: boolean };
  subscriptions: { getUrls(): string[] };
  saveCurrentSettings: () => void;
  updateTray: () => Promise<void>;
};

type KeyStore = Pick<Storage, "getItem" | "setItem" | "removeItem">;

/** Webhook body formats (`WEBHOOK_FORMATS` in webhook.rs). */
export const WEBHOOK_FORMATS = ["txt", "json", "csv", "telegram", "discord", "template"];

export function clampMinutes(v: unknown, fallback = 30): number {
  return Math.max(1, Math.min(1440, Math.floor(Number(v) || fallback)));
}

/**
 * Older versions had two schedules re-checking the same pool: one on the
 * dispatcher tab and one (with webhooks) on the auto tab. Fold them into the
 * single schedule: on if either was on, the shorter interval of the enabled
 * ones, and webhooks unless only the dispatcher schedule was used.
 */
export function migrateLegacySchedule(store: KeyStore): void {
  const dispRaw = store.getItem("pp-disp-auto-on");
  if (dispRaw === null) return;
  const disp = dispRaw === "1";
  const auto = store.getItem("pp-auto-on") === "1";
  const autoMin = clampMinutes(store.getItem("pp-auto-min"));
  const dispMin = clampMinutes(store.getItem("pp-disp-auto-min"));
  const interval = auto && disp ? Math.min(autoMin, dispMin) : disp ? dispMin : autoMin;
  store.setItem("pp-auto-on", auto || disp ? "1" : "0");
  store.setItem("pp-auto-min", String(interval));
  store.setItem("pp-auto-webhooks", auto || !disp ? "1" : "0");
  store.removeItem("pp-disp-auto-on");
  store.removeItem("pp-disp-auto-min");
}

/**
 * A pool's schedule as stored, for pools not shown in the UI. The shown
 * pool's schedule is read from the form instead.
 */
export function storedAutoConfig(store: Pick<Storage, "getItem">, pool: string, check: CheckSettings) {
  const get = (base: string) => store.getItem(scopedKey(base, pool));
  const fmt = get("pp-webhook-format");
  return {
    enabled: get("pp-auto-on") === "1",
    intervalMin: clampMinutes(get("pp-auto-min")),
    sendWebhooks: (get("pp-auto-webhooks") ?? "1") === "1",
    maxPingMs: Math.max(0, Math.floor(Number(get("pp-maxping")) || 0)),
    webhookUrls: parseUrlList(get("pp-webhooks")).map((u) => u.trim()).filter(Boolean),
    webhookFormat: fmt && WEBHOOK_FORMATS.includes(fmt) ? fmt : "txt",
    webhookIncludeCredentials: get("pp-webhook-creds") === "1",
    webhookTemplate: get("pp-webhook-template") ?? DEFAULT_TEMPLATE,
    webhookChangesOnly: get("pp-webhook-changes") === "1",
    subscriptions: parseUrlList(get("pp-subscriptions")).map((u) => u.trim()).filter((u) => /^https?:\/\//i.test(u)),
    check,
  };
}

/** The shown pool's schedule, mirrored to the Rust scheduler. */
export class AutoSettingsStore {
  /** The pool shown in the Auto tab. */
  pool = MAIN_POOL;
  enabled = false;
  /** POST the pool to the webhooks after every scheduled check. */
  sendWebhooks = true;
  private timer: number | undefined;

  constructor(private deps: Deps) {}

  load(): void {
    try {
      migrateLegacySchedule(localStorage);
      const key = (base: string) => scopedKey(base, this.pool);
      this.enabled = localStorage.getItem(key("pp-auto-on")) === "1";
      this.sendWebhooks = (localStorage.getItem(key("pp-auto-webhooks")) ?? "1") === "1";
      const min = localStorage.getItem(key("pp-auto-min"));
      const minEl = document.getElementById("auto-min") as HTMLInputElement | null;
      if (minEl) minEl.value = String(clampMinutes(min));
      const fmt = localStorage.getItem(key("pp-webhook-format"));
      const fmtEl = document.getElementById("webhook-format") as HTMLSelectElement | null;
      if (fmtEl) fmtEl.value = fmt && WEBHOOK_FORMATS.includes(fmt) ? fmt : "txt";
    } catch {
      this.enabled = false;
      this.sendWebhooks = true;
    }
    this.setPressed("auto-on", this.enabled);
    this.setPressed("auto-webhook-on", this.sendWebhooks);
  }

  save(): void {
    try {
      const min = document.getElementById("auto-min") as HTMLInputElement | null;
      const fmt = document.getElementById("webhook-format") as HTMLSelectElement | null;
      const key = (base: string) => scopedKey(base, this.pool);
      localStorage.setItem(key("pp-auto-on"), this.enabled ? "1" : "0");
      localStorage.setItem(key("pp-auto-webhooks"), this.sendWebhooks ? "1" : "0");
      localStorage.setItem(key("pp-auto-min"), String(clampMinutes(min?.value)));
      if (fmt) localStorage.setItem(key("pp-webhook-format"), fmt.value);
    } catch { /* ignore */ }
  }

  arm(): void {
    this.save();
    this.schedule();
    void this.deps.updateTray();
  }

  schedule(): void {
    if (!document.getElementById("timeout")) return;
    window.clearTimeout(this.timer);
    this.timer = window.setTimeout(() => {
      try {
        this.deps.saveCurrentSettings();
        const push = (pool: string, settings: unknown) => {
          void invoke("set_auto_config", { pool, settings }).catch(
            (e) => logErr("auto-config: " + String(e).slice(0, 200)),
          );
        };
        push(this.pool, this.build());
        // The shared check settings changed for every pool's schedule.
        const check = this.deps.readCheckSettings();
        for (const other of this.deps.otherPools()) push(other, storedAutoConfig(localStorage, other, check));
        // The tray shows the webhook state: it follows the settings instead
        // of being asked every few seconds.
        void this.deps.updateTray();
      } catch (e) {
        logErr("auto-config: " + String(e).slice(0, 200));
      }
    }, 200);
  }

  private build() {
    const min = document.getElementById("auto-min") as HTMLInputElement | null;
    const fmt = document.getElementById("webhook-format") as HTMLSelectElement | null;
    return {
      enabled: this.enabled,
      intervalMin: clampMinutes(min?.value),
      sendWebhooks: this.sendWebhooks,
      maxPingMs: this.deps.maxPingMs(),
      webhookUrls: this.deps.webhooks.getUrls(),
      webhookFormat: fmt?.value ?? "txt",
      webhookIncludeCredentials: this.deps.webhooks.includeCreds,
      webhookTemplate: this.deps.webhooks.template,
      webhookChangesOnly: this.deps.webhooks.changesOnly,
      subscriptions: this.deps.subscriptions.getUrls(),
      check: this.deps.readCheckSettings(),
    };
  }

  private setPressed(id: string, on: boolean): void {
    const el = document.getElementById(id) as HTMLButtonElement | null;
    if (el) el.setAttribute("aria-pressed", String(on));
  }
}
