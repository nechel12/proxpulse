import { describe, expect, it } from "vitest";
import { applyAutoPool, showAutoWebhook } from "./auto-events.ts";
import type { PoolItem } from "./pool.ts";

describe("auto events", () => {
  it("normalizes and applies the pool payload", () => {
    let got: PoolItem[] = [];
    let after = 0;
    applyAutoPool(
      { items: [{ raw: "a:80", url: "http://a:80", latency: 10, country: null }] },
      (items) => { got = items; },
      () => { after++; },
    );
    expect(got[0].country).toBeNull();
    expect(after).toBe(1);
  });

  it("renders webhook results in the status line", () => {
    let text = "";
    showAutoWebhook(
      { results: [{ index: 0, ok: true, message: "200" }] },
      (key, params) => `${key}:${params?.r}`,
      (id, render) => { text = `${id}:${render()}`; },
    );
    expect(text).toBe("webhook-status:#1 webhook_sent:200");
  });
});
