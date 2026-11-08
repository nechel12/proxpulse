import { invoke } from "@tauri-apps/api/core";
import { MAIN_POOL } from "./pools.ts";
import type { Translate, StatusSetter } from "./ui-types.ts";

export type DispatchStatus = {
  running: boolean;
  port: number;
  mode: string;
  listener: string;
  upstreams: number;
  /** Upstreams skipped for a minute after a failure. */
  cooling: number;
  current: string | null;
  requests: number;
  errors: number;
  last_error: string | null;
};


export class DispatchMonitor {
  /** The pool whose dispatcher is shown. */
  pool = MAIN_POOL;
  running = false;
  private lastError: string | null = null;
  private timer: number | undefined;
  private filled = false;

  refreshAddr(): void {
    const port = (document.getElementById("srv-port") as HTMLInputElement | null)?.value || "1080";
    const el = document.getElementById("srv-addr");
    if (el) el.textContent = `127.0.0.1:${port}`;
  }

  /** Show another pool: its saved configuration fills the form again. */
  reset(): void {
    this.running = false;
    this.lastError = null;
    this.filled = false;
  }

  renderPill(t: Translate): void {
    const pill = document.getElementById("srv-pill");
    if (!pill) return;
    pill.textContent = this.running ? t("pill_on") : t("pill_off");
    pill.className = this.running ? "pill alive on" : "pill dead";
  }

  async poll(t: Translate, setStatus: StatusSetter): Promise<void> {
    try {
      const pool = this.pool;
      const s = await invoke<DispatchStatus>("local_proxy_status", { pool });
      // The user switched pools while this poll was in flight.
      if (pool !== this.pool) return;
      const req = document.getElementById("srv-req");
      if (req) req.textContent = String(s.requests);
      const cooling = document.getElementById("srv-cooling");
      if (cooling) cooling.textContent = String(s.cooling ?? 0);
      const err = document.getElementById("srv-err");
      if (err) err.textContent = String(s.errors);
      const cur = document.getElementById("srv-cur");
      if (cur) cur.textContent = s.current ?? "-";
      const start = document.getElementById("btn-srv-start") as HTMLButtonElement | null;
      if (start) start.disabled = s.running;
      const stop = document.getElementById("btn-srv-stop") as HTMLButtonElement | null;
      if (stop) stop.disabled = !s.running;

      const wasRunning = this.running;
      const previousError = this.lastError;
      this.running = s.running;
      this.lastError = s.last_error;
      this.renderPill(t);

      // The first poll also shows the saved configuration of a stopped
      // dispatcher; later ones must not overwrite what the user types.
      if (s.running || !this.filled) {
        this.filled = true;
        const port = document.getElementById("srv-port") as HTMLInputElement | null;
        if (port) port.value = String(s.port);
        const mode = document.getElementById("srv-mode") as HTMLSelectElement | null;
        if (mode) mode.value = s.mode;
        const listener = document.getElementById("srv-listener") as HTMLSelectElement | null;
        if (listener) listener.value = s.listener;
        this.refreshAddr();
      }
      const st = document.getElementById("srv-status");
      if (st && (s.running !== wasRunning || (s.running && s.last_error !== previousError))) {
        if (s.running) {
          const error = s.last_error;
          const addr = `127.0.0.1:${s.port}`;
          setStatus("srv-status", () => error ?? t("srv_on", { addr }));
        } else if (s.last_error) {
          const short = s.last_error.slice(0, 120);
          setStatus("srv-status", () => t("srv_err", { e: short }));
        } else {
          setStatus("srv-status", () => t("srv_off"));
        }
      } else if (st && st.textContent === "-" && !s.running) {
        const error = s.last_error?.slice(0, 120);
        setStatus("srv-status", () => (error ? t("srv_err", { e: error }) : t("srv_off")));
      }
    } catch { /* ignore */ }
  }

  setPolling(on: boolean, t: Translate, setStatus: StatusSetter): void {
    window.clearInterval(this.timer);
    if (on) {
      void this.poll(t, setStatus);
      this.timer = window.setInterval(() => void this.poll(t, setStatus), 2000);
    }
  }
}
