import { describe, expect, it } from "vitest";
import { formatSubscriptionResults } from "./subscriptions.ts";
import { parseUrlList } from "./url-list.ts";

describe("subscriptions", () => {
  it("formats per-list results", () => {
    const t = (k: string, p?: Record<string, string | number>) => `${k}:${JSON.stringify(p)}`;
    expect(formatSubscriptionResults([
      { index: 0, ok: true, found: 10, added: 3, error: null },
      { index: 1, ok: false, found: 0, added: 0, error: "http 404" },
    ], t)).toBe(`#1 subs_ok:{"n":10,"m":3} | #2 subs_fail:{"e":"http 404"}`);
  });

  it("restores saved URL lists defensively", () => {
    expect(parseUrlList('["https://a", 5, "https://b"]')).toEqual(["https://a", "https://b"]);
    expect(parseUrlList("[]")).toEqual([""]);
    expect(parseUrlList("{broken")).toEqual([""]);
    expect(parseUrlList(JSON.stringify(Array(20).fill("u")), 10)).toHaveLength(10);
  });
});
