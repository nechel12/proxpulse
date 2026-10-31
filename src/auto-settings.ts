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

export class AutoSettingsStore {
  autoOn = false;
  dispAutoOn = false;
  private timer: number | undefined;

  constructor(private deps: Deps) {}

  load(): void {
    try {
      this.autoOn = localStorage.getItem("pp-auto-on") === "1";
      this.dispAutoOn = localStorage.getItem("pp-disp-auto-on") === "1";
      const autoMin = Number(localStorage.getItem("pp-auto-min"));
      const autoEl = document.getElementById("auto-min") as HTMLInputElement | null;
      if (autoMin && autoEl) autoEl.value = String(Math.max(1, Math.min(1440, Math.floor(autoMin))));
      const dispMin = Number(localStorage.getItem("pp-disp-auto-min"));
      const dispEl = document.getElementById("disp-auto-min") as HTMLInputElement | null;
      if (dispMin && dispEl) dispEl.value = String(Math.max(1, Math.min(1440, Math.floor(dispMin))));
      const fmt = localStorage.getItem("pp-webhook-format");
      const fmtEl = document.getElementById("webhook-format") as HTMLSelectElement | null;
      if (fmtEl && (fmt === "txt" || fmt === "json" || fmt === "csv")) fmtEl.value = fmt;
    } catch {
      this.autoOn = false;
      this.dispAutoOn = false;
    }
    this.setPressed("auto-on", this.autoOn);
    this.setPressed("disp-auto-on", this.dispAutoOn);
  }

  save(): void {
    try {
      const autoMin = document.getElementById("auto-min") as HTMLInputElement | null;
      const dispMin = document.getElementById("disp-auto-min") as HTMLInputElement | null;
      const fmt = document.getElementById("webhook-format") as HTMLSelectElement | null;
      localStorage.setItem("pp-auto-on", this.autoOn ? "1" : "0");
      localStorage.setItem("pp-disp-auto-on", this.dispAutoOn ? "1" : "0");
      localStorage.setItem("pp-auto-min", String(Math.max(1, Math.min(1440, Number(autoMin?.value) || 30))));
      localStorage.setItem("pp-disp-auto-min", String(Math.max(1, Math.min(1440, Number(dispMin?.value) || 30))));
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
    const s = this.deps.readCheckSettings();
    const minutes = (id: string, fallback: number) => {
      const el = document.getElementById(id) as HTMLInputElement | null;
      return Math.max(1, Math.min(1440, Math.floor(Number(el?.value) || fallback)));
    };
    const fmt = document.getElementById("webhook-format") as HTMLSelectElement | null;
    return {
      dispEnabled: this.dispAutoOn,
      dispIntervalMin: minutes("disp-auto-min", 30),
      autoEnabled: this.autoOn,
      autoIntervalMin: minutes("auto-min", 30),
      maxPingMs: this.deps.maxPingMs(),
      webhookUrls: this.deps.webhooks.getUrls(),
      webhookFormat: fmt?.value ?? "txt",
      webhookIncludeCredentials: this.deps.webhooks.includeCreds,
      testUrl: s.testUrl,
      timeoutMs: s.timeoutMs,
      concurrency: s.concurrency,
      repeats: s.repeats,
      defaultProto: s.defaultProto,
      withGeo: s.withGeo,
      withAnonymity: s.withAnonymity,
      withTamper: s.withTamper,
      withTls: s.withTls,
      precheck: s.precheck,
      precheckTimeoutMs: s.precheckTimeoutMs,
      judgeMode: s.judgeMode,
      ipapiKey: s.ipapiKey,
    };
  }

  private setPressed(id: string, on: boolean): void {
    const el = document.getElementById(id) as HTMLButtonElement | null;
    if (el) el.setAttribute("aria-pressed", String(on));
  }
}
