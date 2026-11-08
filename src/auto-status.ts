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
  /** The pool's name as the tabs show it. */
  poolName?: (id: string) => string;
};

export class AutoStatusController {
  running = false;
  /** Pools whose scheduled check is in progress. */
  private active = new Set<string>();

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
    const pool = ev.pool ?? "main";
    if (ev.state === "fetching" || ev.state === "running") {
      show(() =>
        ev.state === "fetching" ? this.deps.t("auto_fetching") : this.deps.t("auto_running", { n: ev.total }),
      );
      this.active.add(pool);
      this.running = true;
      if (ev.state === "running") this.buttons(true);
      this.renderCheckNote(pool);
      return;
    }
    // Every other state ends that pool's cycle, "skipped" too: a cycle that
    // fetched subscriptions and then met a manual check is over.
    this.active.delete(pool);
    this.running = this.active.size > 0;
    this.renderCheckNote([...this.active][0]);
    switch (ev.state) {
      case "skipped":
        show(() => this.deps.t("auto_skip"));
        break;
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
    if (this.running) return;
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
    this.renderCheckNote("");
  }

  /**
   * The checker is free again although no event said so (a check running
   * before a page reload): unlock unless a scheduled check is known to run.
   */
  release(): void {
    if (this.active.size > 0) return;
    this.running = false;
    this.buttons(false);
    this.renderCheckNote(undefined);
  }

  /**
   * On the Check tab, why Start does not work: a scheduled check of `pool`
   * holds the checker, and Stop would cancel it. Hidden when none runs.
   */
  private renderCheckNote(pool: string | undefined): void {
    const note = document.getElementById("auto-check-note");
    if (!note) return;
    if (pool === undefined) {
      note.hidden = true;
      note.textContent = "";
      return;
    }
    const name = pool && this.deps.poolName ? this.deps.poolName(pool) : "";
    note.textContent = this.deps.t("auto_check_note", { pool: name });
    note.hidden = false;
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
