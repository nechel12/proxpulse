import { invoke } from "@tauri-apps/api/core";

export type DispatchStatus = {
  running: boolean;
  port: number;
  mode: string;
  listener: string;
  upstreams: number;
  current: string | null;
  requests: number;
  errors: number;
  last_error: string | null;
};

type Translate = (key: string, params?: Record<string, string | number>) => string;
type StatusSetter = (id: string, render: () => string) => void;

export class DispatchMonitor {
  running = false;
  private lastError: string | null = null;
  private timer: number | undefined;

  refreshAddr(): void {
    const port = (document.getElementById("srv-port") as HTMLInputElement | null)?.value || "1080";
    const el = document.getElementById("srv-addr");
    if (el) el.textContent = `127.0.0.1:${port}`;
  }

  renderPill(t: Translate): void {
    const pill = document.getElementById("srv-pill");
    if (!pill) return;
    pill.textContent = this.running ? t("pill_on") : t("pill_off");
    pill.className = this.running ? "pill alive on" : "pill dead";
  }

  async poll(t: Translate, setStatus: StatusSetter): Promise<void> {
    try {
      const s = await invoke<DispatchStatus>("local_proxy_status");
      const req = document.getElementById("srv-req");
      if (req) req.textContent = String(s.requests);
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

      if (s.running) {
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
        } else {
          setStatus("srv-status", () => t("srv_off"));
        }
      } else if (st && st.textContent === "-" && !s.running) {
        setStatus("srv-status", () => t("srv_off"));
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
