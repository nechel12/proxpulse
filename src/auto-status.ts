export type AutoStatusEvent = {
  state: "running" | "done" | "stopped" | "skipped" | "empty" | "error" | "offline";
  alive: number;
  total: number;
  avgMs: number | null;
  error: string | null;
  at: number;
};

type Translate = (key: string, params?: Record<string, string | number>) => string;
type StatusSetter = (id: string, render: () => string) => void;

type Deps = {
  t: Translate;
  setStatusText: StatusSetter;
  isManualRunning: () => boolean;
  onDone: (avgMs: number | null, alive: number) => void;
  locale: () => string;
};

export class AutoStatusController {
  running = false;

  constructor(private deps: Deps) {}

  handle(ev: AutoStatusEvent): void {
    const time = () =>
      new Date(ev.at * 1000).toLocaleTimeString(this.deps.locale() === "ru" ? "ru-RU" : "en-US");
    // One schedule for the shared pool: both tabs show its status.
    const show = (render: () => string) => {
      for (const id of ["auto-status", "disp-auto-status"]) this.deps.setStatusText(id, render);
    };
    if (ev.state === "running") {
      show(() => this.deps.t("auto_running", { n: ev.total }));
      this.running = true;
      this.buttons(true);
      return;
    }
    if (ev.state === "skipped") {
      show(() => this.deps.t("auto_skip"));
      return;
    }
    this.running = false;
    switch (ev.state) {
      case "done":
        show(() => `${this.deps.t("auto_pruned", { alive: ev.alive, total: ev.total })} ${time()}`);
        break;
      case "stopped":
        show(() => this.deps.t("st_stopped", { alive: ev.alive, total: ev.total }));
        break;
      case "empty":
        show(() => this.deps.t("pool_empty"));
        break;
      case "error":
        show(() => this.deps.t("auto_err", { e: ev.error ?? "" }));
        break;
      case "offline":
        show(() => `${this.deps.t("auto_offline")} ${time()}`);
        break;
    }
    if (ev.state === "done") this.deps.onDone(ev.avgMs, ev.alive);
    if (!this.deps.isManualRunning()) {
      const start = document.getElementById("btn-start") as HTMLButtonElement | null;
      if (start) start.disabled = false;
      const stop = document.getElementById("btn-stop") as HTMLButtonElement | null;
      if (stop) stop.disabled = true;
    }
    const now = document.getElementById("btn-auto-now") as HTMLButtonElement | null;
    if (now) now.disabled = false;
  }

  restore(): void {
    this.running = true;
    this.buttons(true);
  }

  private buttons(active: boolean): void {
    const start = document.getElementById("btn-start") as HTMLButtonElement | null;
    if (start) start.disabled = active;
    const stop = document.getElementById("btn-stop") as HTMLButtonElement | null;
    if (stop) stop.disabled = !active;
    const now = document.getElementById("btn-auto-now") as HTMLButtonElement | null;
    if (now) now.disabled = active;
  }
}
