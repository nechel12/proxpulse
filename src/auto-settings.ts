import { invoke } from "@tauri-apps/api/core";
import type { CheckSettings } from "./check-client.ts";
import { logErr } from "./diagnostics.ts";

type Deps = {
  readCheckSettings: () => CheckSettings;
  maxPingMs: () => number;
  webhooks: { getUrls(): string[]; includeCreds: boolean };
  saveCurrentSettings: () => void;
  updateTray: () => Promise<void>;
};

type KeyStore = Pick<Storage, "getItem" | "setItem" | "removeItem">;

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

/** The single pool schedule, mirrored to the Rust scheduler. */
export class AutoSettingsStore {
  enabled = false;
  /** POST the pool to the webhooks after every scheduled check. */
  sendWebhooks = true;
  private timer: number | undefined;

  constructor(private deps: Deps) {}

  load(): void {
    try {
      migrateLegacySchedule(localStorage);
      this.enabled = localStorage.getItem("pp-auto-on") === "1";
      this.sendWebhooks = (localStorage.getItem("pp-auto-webhooks") ?? "1") === "1";
      const min = localStorage.getItem("pp-auto-min");
      const minEl = document.getElementById("auto-min") as HTMLInputElement | null;
      if (min && minEl) minEl.value = String(clampMinutes(min));
      const fmt = localStorage.getItem("pp-webhook-format");
      const fmtEl = document.getElementById("webhook-format") as HTMLSelectElement | null;
      if (fmtEl && (fmt === "txt" || fmt === "json" || fmt === "csv")) fmtEl.value = fmt;
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
      localStorage.setItem("pp-auto-on", this.enabled ? "1" : "0");
      localStorage.setItem("pp-auto-webhooks", this.sendWebhooks ? "1" : "0");
      localStorage.setItem("pp-auto-min", String(clampMinutes(min?.value)));
      if (fmt) localStorage.setItem("pp-webhook-format", fmt.value);
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
        void invoke("set_auto_config", { settings: this.build() }).catch(
          (e) => logErr("auto-config: " + String(e).slice(0, 200)),
        );
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
      check: this.deps.readCheckSettings(),
    };
  }

  private setPressed(id: string, on: boolean): void {
    const el = document.getElementById(id) as HTMLButtonElement | null;
    if (el) el.setAttribute("aria-pressed", String(on));
  }
}
