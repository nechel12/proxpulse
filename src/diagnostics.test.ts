import { describe, expect, it } from "vitest";
import { ErrorLog } from "./diagnostics.ts";

describe("ErrorLog", () => {
  it("keeps the newest errors and clips long messages", () => {
    const log = new ErrorLog();
    for (let i = 0; i < 55; i++) log.log(`error-${i}-${"x".repeat(400)}`);
    const recent = log.recent(2);
    expect(recent).toHaveLength(2);
    expect(recent[1].msg.length).toBe(300);
    expect(recent[1].msg.startsWith("error-54-")).toBe(true);
  });
});
