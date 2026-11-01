import { describe, expect, it } from "vitest";
import { AlertLatch } from "./notify.ts";

describe("AlertLatch", () => {
  it("reports a problem once until it clears", () => {
    const latch = new AlertLatch();
    expect(latch.raise("webhook")).toBe(true);
    expect(latch.raise("webhook")).toBe(false);
    expect(latch.raise("pool")).toBe(true);
    latch.clear("webhook");
    expect(latch.raise("webhook")).toBe(true);
  });
});
