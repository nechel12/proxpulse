export type AutoStatusEvent = {
  schedule: "disp" | "auto";
  state: "running" | "done" | "stopped" | "skipped" | "empty" | "error";
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
    const id = ev.schedule === "disp" ? "disp-auto-status" : "auto-status";
    const time = () =>
      new Date(ev.at * 1000).toLocaleTimeString(this.deps.locale() === "ru" ? "ru-RU" : "en-US");
    if (ev.state === "running") {
      this.deps.setStatusText(id, () => this.deps.t("auto_running", { n: ev.total }));
      this.running = true;
      this.buttons(true);
      return;
    }
    if (ev.state === "skipped") {
      this.deps.setStatusText(id, () => this.deps.t("auto_skip"));
      return;
    }
    this.running = false;
    switch (ev.state) {
      case "done":
        this.deps.setStatusText(id, () => `${this.deps.t("auto_pruned", {
          alive: ev.alive,
          total: ev.total,
        })} ${time()}`);
        break;
      case "stopped":
        this.deps.setStatusText(id, () => this.deps.t("st_stopped", {
          alive: ev.alive,
          total: ev.total,
        }));
        break;
      case "empty":
        this.deps.setStatusText(id, () => this.deps.t("pool_empty"));
        break;
      case "error":
        this.deps.setStatusText(id, () => this.deps.t("auto_err", { e: ev.error ?? "" }));
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
