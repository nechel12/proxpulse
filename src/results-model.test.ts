import { describe, expect, it } from "vitest";
import { deadStub } from "./check-client.ts";
import {
  NO_FILTERS,
  defaultOrder,
  filterOptions,
  filterResults,
  mergeResults,
  nextSort,
  sortResults,
  visibleRange,
} from "./results-model.ts";
import type { ProxyResult } from "./types.ts";

function alive(proxy: string, extra: Partial<ProxyResult> = {}): ProxyResult {
  return { ...deadStub(proxy, ""), proto: "HTTP", alive: true, error: null, success_rate: 1, ...extra };
}

const de = alive("1.1.1.1:80", { latency_ms: 300, country: "Germany", anonymity: "elite", proto: "HTTP" });
const us = alive("2.2.2.2:1080", { latency_ms: 100, country: "United States", anonymity: "transparent", proto: "SOCKS5" });
const unknown = alive("3.3.3.3:80", { latency_ms: null, anonymity: null });
const dead = deadStub("4.4.4.4:80", "timeout");
const all = [de, us, unknown, dead];

describe("filterResults", () => {
  it("combines status, protocol, anonymity, country and text", () => {
    expect(filterResults(all, NO_FILTERS)).toHaveLength(4);
    expect(filterResults(all, { ...NO_FILTERS, status: "dead" })).toEqual([dead]);
    expect(filterResults(all, { ...NO_FILTERS, proto: "SOCKS5" })).toEqual([us]);
    expect(filterResults(all, { ...NO_FILTERS, anon: "unknown", status: "alive" })).toEqual([unknown]);
    expect(filterResults(all, { ...NO_FILTERS, country: "Germany" })).toEqual([de]);
    expect(filterResults(all, { ...NO_FILTERS, query: " TIMEOUT " })).toEqual([dead]);
  });
});

describe("sortResults", () => {
  it("sorts both ways and keeps empty values last", () => {
    const ping = sortResults(all, { key: "ping", dir: "asc" }).map((r) => r.proxy);
    expect(ping).toEqual([us.proxy, de.proxy, unknown.proxy, dead.proxy]);
    const desc = sortResults(all, { key: "ping", dir: "desc" }).map((r) => r.proxy);
    expect(desc).toEqual([de.proxy, us.proxy, unknown.proxy, dead.proxy]);
  });

  it("orders anonymity by strength and keeps ties stable", () => {
    const anon = sortResults(all, { key: "anon", dir: "asc" }).map((r) => r.proxy);
    expect(anon).toEqual([de.proxy, us.proxy, unknown.proxy, dead.proxy]);
    const status = sortResults(all, { key: "status", dir: "asc" }).map((r) => r.proxy);
    expect(status).toEqual([de.proxy, us.proxy, unknown.proxy, dead.proxy]);
  });

  it("compares numbers inside text naturally", () => {
    const rows = [alive("10.0.0.1:8080"), alive("9.0.0.1:8080")];
    expect(sortResults(rows, { key: "proxy", dir: "asc" })[0].proxy).toBe("9.0.0.1:8080");
  });

  it("cycles a header through ascending, descending and off", () => {
    expect(nextSort(null, "ping")).toEqual({ key: "ping", dir: "asc" });
    expect(nextSort({ key: "ping", dir: "asc" }, "ping")).toEqual({ key: "ping", dir: "desc" });
    expect(nextSort({ key: "ping", dir: "desc" }, "ping")).toBeNull();
    expect(nextSort({ key: "ping", dir: "desc" }, "geo")).toEqual({ key: "geo", dir: "asc" });
  });
});

describe("re-check merging", () => {
  it("replaces re-checked entries in place", () => {
    const revived = alive(dead.proxy, { latency_ms: 50 });
    const merged = mergeResults(all, [revived]);
    expect(merged).toHaveLength(4);
    expect(merged[3]).toBe(revived);
    expect(defaultOrder(merged)[0]).toBe(revived);
  });
});

describe("filterOptions", () => {
  it("lists protocols and countries present in the results", () => {
    const o = filterOptions(all);
    expect(o.proto).toEqual(["ERR", "HTTP", "SOCKS5"]);
    expect(o.country.map(([c]) => c)).toEqual(["Germany", "United States"]);
  });
});

describe("visibleRange", () => {
  it("covers the viewport plus overscan within bounds", () => {
    expect(visibleRange(0, 300, 30, 1000, 5)).toEqual([0, 15]);
    expect(visibleRange(3000, 300, 30, 1000, 5)).toEqual([95, 115]);
    expect(visibleRange(29_990, 300, 30, 1000, 5)).toEqual([994, 1000]);
    expect(visibleRange(0, 0, 30, 3, 5)).toEqual([0, 3]);
    expect(visibleRange(0, 300, 30, 0, 5)).toEqual([0, 0]);
  });
});
