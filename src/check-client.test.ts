import { describe, expect, it, vi } from "vitest";
import { clampInt, deadStub, runCheckList, type CheckSettings } from "./check-client.ts";
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

  it("reports an unreachable check server without taking it for a result", async () => {
    const result: CheckEvent = { ...deadStub("1.2.3.4:80", "timeout"), kind: "result" };
    backend.events = [result, { kind: "unreachable" }, { kind: "done" }];
    const onUnreachable = vi.fn();
    const settings = { testUrl: "https://example.com/" } as CheckSettings;

    const out = await runCheckList(["1.2.3.4:80"], settings, [], undefined, undefined, undefined, onUnreachable);

    expect(onUnreachable).toHaveBeenCalledOnce();
    expect(out.map((r) => r.proxy)).toEqual(["1.2.3.4:80"]);
  });
});

describe("clampInt", () => {
  it("gives the backend whole numbers within the limits", () => {
    expect(clampInt("2.5", [1, 500], 50)).toBe(3);
    expect(clampInt("0", [1, 500], 50)).toBe(1);
    expect(clampInt("9999", [1, 500], 50)).toBe(500);
    expect(clampInt("", [1, 500], 50)).toBe(50);
    expect(clampInt("abc", [1, 500], 50)).toBe(50);
    expect(clampInt(" 20 ", [1, 500], 50)).toBe(20);
  });
});
