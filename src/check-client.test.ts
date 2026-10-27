import { describe, expect, it } from "vitest";
import { deadStub } from "./check-client.ts";

describe("check client", () => {
  it("builds a dead result with a clipped error", () => {
    const r = deadStub("1.2.3.4:80", "x".repeat(200));
    expect(r.alive).toBe(false);
    expect(r.proxy).toBe("1.2.3.4:80");
    expect(r.error?.length).toBe(120);
  });
});
