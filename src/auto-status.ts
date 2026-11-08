export type AutoStatusEvent = {
  /** The pool whose schedule ran. */
  pool?: string;
  state: "fetching" | "running" | "done" | "stopped" | "skipped" | "empty" | "error" | "offline" | "suspect";
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

  /**
   * One checker serves every pool's schedule, so the buttons follow every
   * event; the status lines only show the pool in the tabs (`shown`).
   */
  handle(ev: AutoStatusEvent, shown = true): void {
    const time = () =>
      new Date(ev.at * 1000).toLocaleTimeString(this.deps.locale() === "ru" ? "ru-RU" : "en-US");
    // Both tabs show the schedule of the pool they show.
    const show = (render: () => string) => {
      if (!shown) return;
      for (const id of ["auto-status", "disp-auto-status"]) this.deps.setStatusText(id, render);
    };
    if (ev.state === "fetching") {
      show(() => this.deps.t("auto_fetching"));
      this.running = true;
      return;
    }
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
      case "suspect":
        show(() => `${this.deps.t("auto_suspect", { alive: ev.alive, total: ev.total })} ${time()}`);
        break;
    }
    if (ev.state === "done" && shown) this.deps.onDone(ev.avgMs, ev.alive);
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
