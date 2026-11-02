import { invoke } from "@tauri-apps/api/core";
import { MAIN_POOL } from "./pools.ts";

type Translate = (key: string, params?: Record<string, string | number>) => string;
type StatusSetter = (id: string, render: () => string) => void;

/** `PoolApiStatus` in pool_api.rs. */
type PoolApiStatus = { running: boolean; port: number; last_error: string | null };

type Deps = {
  t: Translate;
  setStatusText: StatusSetter;
  /** The pool shown in the tab; the example URL names it. */
  pool: () => string;
};

const DEFAULT_PORT = 1090;

/** The local pool API on the Dispatcher tab: on/off, port and an example URL. */
export class PoolApiPanel {
  private running = false;

  constructor(private deps: Deps) {}

  init(): void {
    document.getElementById("btn-api-toggle")?.addEventListener("click", () => void this.toggle());
    document.getElementById("api-port")?.addEventListener("input", () => this.render());
    void this.refresh();
  }

  private portInput(): HTMLInputElement | null {
    return document.getElementById("api-port") as HTMLInputElement | null;
  }

  private port(): number {
    return Math.max(1, Math.min(65535, Math.floor(Number(this.portInput()?.value) || DEFAULT_PORT)));
  }

  /** A request for the shown pool, ready to copy. */
  exampleUrl(): string {
    const pool = this.deps.pool();
    const query = pool === MAIN_POOL ? "" : `?pool=${encodeURIComponent(pool)}`;
    return `http://127.0.0.1:${this.port()}/random${query}`;
  }

  render(): void {
    const { t } = this.deps;
    const pill = document.getElementById("api-pill");
    if (pill) {
      pill.textContent = this.running ? t("pill_on") : t("pill_off");
      pill.className = this.running ? "pill alive on" : "pill dead";
    }
    const btn = document.getElementById("btn-api-toggle");
    if (btn) btn.textContent = this.running ? t("srv_stop") : t("srv_start");
    const port = this.portInput();
    if (port) port.disabled = this.running;
    const example = document.getElementById("api-example");
    if (example) example.textContent = this.exampleUrl();
  }

  async refresh(): Promise<void> {
    const { t, setStatusText } = this.deps;
    try {
      const s = await invoke<PoolApiStatus>("pool_api_status");
      this.running = s.running;
      const port = this.portInput();
      if (port) port.value = String(s.port);
      const error = s.last_error?.slice(0, 120);
      if (s.running) setStatusText("api-status", () => t("api_on", { addr: `127.0.0.1:${s.port}` }));
      else if (error) setStatusText("api-status", () => t("srv_err", { e: error }));
      else setStatusText("api-status", () => t("srv_off"));
    } catch { /* browser preview or older backend */ }
    this.render();
  }

  private async toggle(): Promise<void> {
    const { t, setStatusText } = this.deps;
    try {
      if (this.running) await invoke("stop_pool_api");
      else await invoke<string>("start_pool_api", { port: this.port() });
    } catch (e) {
      const msg = String(e).slice(0, 120);
      setStatusText("api-status", () => t("srv_err", { e: msg }));
      this.render();
      return;
    }
    await this.refresh();
  }
}
