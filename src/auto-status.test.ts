import { afterEach, describe, expect, it, vi } from "vitest";
import { AutoStatusController, type AutoStatusEvent } from "./auto-status.ts";
import { runButtons, type RunState } from "./run-buttons.ts";

type El = { disabled: boolean; hidden: boolean; textContent: string };

function page(): Record<string, El> {
  const els: Record<string, El> = {};
  for (const id of ["btn-start", "btn-stop", "btn-auto-now", "auto-check-note"]) {
    els[id] = { disabled: false, hidden: true, textContent: "" };
  }
  vi.stubGlobal("document", { getElementById: (id: string) => els[id] ?? null });
  return els;
}

/** The controller wired as main.ts wires it. */
function controller(
  els: Record<string, El>,
  manual = () => false,
  shown: string[] = [],
): AutoStatusController {
  const c: AutoStatusController = new AutoStatusController({
    t: (key, params) =>
      params?.pool !== undefined ? `${key}:${params.pool}`
        : params?.done !== undefined ? `${key}:${params.done}/${params.total}`
        : key,
    setStatusText: (_id, render) => shown.push(render()),
    onChange: () => {
      const on = runButtons({ manual: manual(), starting: false, autoChecking: c.checking, autoRunning: c.running });
      els["btn-start"].disabled = !on.start;
      els["btn-stop"].disabled = !on.stop;
      els["btn-auto-now"].disabled = !on.autoNow;
    },
    onDone: () => {},
    locale: () => "en",
    poolName: (id) => (id === "main" ? "Main" : id),
  });
  return c;
}

const ev = (pool: string, state: AutoStatusEvent["state"]): AutoStatusEvent => ({
  pool, state, alive: 0, total: 3, avgMs: null, error: null, at: 0,
});

afterEach(() => vi.unstubAllGlobals());

describe("AutoStatusController", () => {
  it("frees Start when a cycle that fetched subscriptions is skipped", () => {
    const els = page();
    const c = controller(els);
    c.handle(ev("main", "fetching"));
    expect(c.running).toBe(true);
    // Fetching does not hold the checker yet.
    expect(c.checking).toBe(false);
    expect(els["btn-start"].disabled).toBe(false);
    expect(els["auto-check-note"].hidden).toBe(false);
    c.handle(ev("main", "skipped"));
    expect(c.running).toBe(false);
    expect(els["btn-start"].disabled).toBe(false);
    expect(els["auto-check-note"].hidden).toBe(true);
  });

  it("keeps Start locked while another pool's check still runs", () => {
    const els = page();
    const c = controller(els);
    c.handle(ev("main", "running"));
    c.handle(ev("p1", "running"));
    c.handle(ev("main", "done"));
    expect(c.running).toBe(true);
    expect(els["btn-start"].disabled).toBe(true);
    expect(els["auto-check-note"].textContent).toBe("auto_check_note:p1");
    c.handle(ev("p1", "done"));
    expect(els["btn-start"].disabled).toBe(false);
    expect(els["btn-stop"].disabled).toBe(true);
    expect(els["btn-auto-now"].disabled).toBe(false);
  });

  it("leaves Start locked when a scheduled check ends during a manual one", () => {
    const els = page();
    let manual = false;
    const c = controller(els, () => manual);
    c.handle(ev("main", "running"));
    manual = true;
    c.handle(ev("main", "skipped"));
    expect(els["btn-start"].disabled).toBe(true);
    expect(els["btn-stop"].disabled).toBe(false);
  });

  it("unlocks after a reload once the backend is free", () => {
    const els = page();
    const c = controller(els);
    c.restore();
    expect(els["btn-start"].disabled).toBe(true);
    expect(els["btn-stop"].disabled).toBe(false);
    c.release();
    expect(c.running).toBe(false);
    expect(els["btn-start"].disabled).toBe(false);
    expect(els["btn-stop"].disabled).toBe(true);
  });
});

describe("scheduled check progress", () => {
  it("shows how far a running check is", () => {
    const shown: string[] = [];
    const c = controller(page(), () => false, shown);
    c.handle(ev("main", "running"));
    expect(shown[shown.length - 1]).toBe("auto_running");
    c.handle({ ...ev("main", "running"), done: 7, total: 20 });
    expect(shown[shown.length - 1]).toBe("auto_running_progress:7/20");
    // Still one running check, not a second one.
    expect(c.checking).toBe(true);
    c.handle(ev("main", "done"));
    expect(c.checking).toBe(false);
  });
});

describe("runButtons", () => {
  const idle: RunState = { manual: false, starting: false, autoChecking: false, autoRunning: false };

  it("lets a free checker start and nothing stop", () => {
    expect(runButtons(idle)).toEqual({ start: true, stop: false, autoNow: true });
  });

  it("locks Start for a manual run and for a scheduled check alike", () => {
    expect(runButtons({ ...idle, manual: true })).toEqual({ start: false, stop: true, autoNow: true });
    expect(runButtons({ ...idle, starting: true }).start).toBe(false);
    expect(runButtons({ ...idle, autoChecking: true, autoRunning: true })).toEqual({
      start: false,
      stop: true,
      autoNow: false,
    });
  });
});
