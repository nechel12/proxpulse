import { describe, expect, it } from "vitest";
import { PoolView } from "./pool.ts";

describe("PoolView", () => {
  it("merges, refreshes and drops over-limit entries", () => {
    const view = new PoolView();
    view.merge([{ raw: "a:80", latency: 20, country: "DE" }], 0, 100);
    view.merge([{ raw: "b:80", latency: 50, country: "NL" }], 0, 100);
    view.merge([{ raw: "a:80", latency: 200, country: "FR" }], 100, 100);
    expect(view.count).toBe(1);
    expect(view.items[0].raw).toBe("b:80");
  });

  it("keeps an explicit normalized URL and removes by raw", () => {
    const view = new PoolView();
    view.merge([{ raw: "u:p@a:80", url: "socks5://u:p@a:80", latency: 10 }], 0, 100);
    expect(view.items[0].url).toBe("socks5://u:p@a:80");
    view.remove("u:p@a:80");
    expect(view.count).toBe(0);
  });
});
