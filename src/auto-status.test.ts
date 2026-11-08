import { afterEach, describe, expect, it, vi } from "vitest";
import { AutoStatusController, type AutoStatusEvent } from "./auto-status.ts";

type El = { disabled: boolean; hidden: boolean; textContent: string };

function page(): Record<string, El> {
  const els: Record<string, El> = {};
  for (const id of ["btn-start", "btn-stop", "btn-auto-now", "auto-check-note"]) {
    els[id] = { disabled: false, hidden: true, textContent: "" };
  }
  vi.stubGlobal("document", { getElementById: (id: string) => els[id] ?? null });
  return els;
}

function controller(): AutoStatusController {
  return new AutoStatusController({
    t: (key, params) => `${key}${params?.pool !== undefined ? `:${params.pool}` : ""}`,
    setStatusText: () => {},
    isManualRunning: () => false,
    onDone: () => {},
    locale: () => "en",
    poolName: (id) => (id === "main" ? "Main" : id),
  });
}

const ev = (pool: string, state: AutoStatusEvent["state"]): AutoStatusEvent => ({
  pool, state, alive: 0, total: 3, avgMs: null, error: null, at: 0,
});

afterEach(() => vi.unstubAllGlobals());

describe("AutoStatusController", () => {
  it("frees Start when a cycle that fetched subscriptions is skipped", () => {
    const els = page();
    const c = controller();
    c.handle(ev("main", "fetching"));
    expect(c.running).toBe(true);
    expect(els["auto-check-note"].hidden).toBe(false);
    c.handle(ev("main", "skipped"));
    expect(c.running).toBe(false);
    expect(els["btn-start"].disabled).toBe(false);
    expect(els["auto-check-note"].hidden).toBe(true);
  });

  it("keeps Start locked while another pool's check still runs", () => {
    const els = page();
    const c = controller();
    c.handle(ev("main", "running"));
    c.handle(ev("p1", "running"));
    c.handle(ev("main", "done"));
    expect(c.running).toBe(true);
    expect(els["btn-start"].disabled).toBe(true);
    expect(els["auto-check-note"].textContent).toBe("auto_check_note:p1");
    c.handle(ev("p1", "done"));
    expect(els["btn-start"].disabled).toBe(false);
  });
});
