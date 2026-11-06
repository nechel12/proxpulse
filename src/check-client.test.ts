import { describe, expect, it, vi } from "vitest";
import { deadStub, runCheckList, type CheckSettings } from "./check-client.ts";
import type { CheckEvent } from "./types.ts";

const backend = vi.hoisted(() => ({ events: [] as unknown[] }));

vi.mock("@tauri-apps/api/core", () => ({
  Channel: class {
    onmessage: (ev: unknown) => void = () => {};
  },
  invoke: async (_cmd: string, args: { onResult: { onmessage: (ev: unknown) => void } }) => {
    for (const ev of backend.events) args.onResult.onmessage(ev);
  },
}));

describe("check client", () => {
  it("builds a dead result with a clipped error", () => {
    const r = deadStub("1.2.3.4:80", "x".repeat(200));
    expect(r.alive).toBe(false);
    expect(r.proxy).toBe("1.2.3.4:80");
    expect(r.error?.length).toBe(120);
  });

  it("reports a key over plain HTTP without taking it for a result", async () => {
    const result: CheckEvent = { ...deadStub("1.2.3.4:80", "timeout"), kind: "result" };
    backend.events = [{ kind: "keyOverHttp" }, result, { kind: "done" }];
    const onKeyOverHttp = vi.fn();
    const settings = { testUrl: "http://judge.example.com/judge" } as CheckSettings;

    const out = await runCheckList(["1.2.3.4:80"], settings, [], undefined, undefined, onKeyOverHttp);

    expect(onKeyOverHttp).toHaveBeenCalledOnce();
    expect(out.map((r) => r.proxy)).toEqual(["1.2.3.4:80"]);
  });
});
