import { afterEach, describe, expect, it, vi } from "vitest";
import { GeoCacheStore, geoKey } from "./geo-store.ts";
import type { ProxyResult } from "./types.ts";

const result = {
  proxy: "u:p@1.2.3.4:8080",
  url: "http://u:p@1.2.3.4:8080",
  alive: true,
  ip: "5.6.7.8",
  country: "Testland",
  country_code: "TL",
  city: "Testville",
  isp: "ISP",
  org: "ORG",
  asn: "AS1",
  ip_type: "residential",
} as ProxyResult;

function memoryStorage(initial: Record<string, string> = {}) {
  const data = new Map(Object.entries(initial));
  return {
    data,
    getItem: (k: string) => data.get(k) ?? null,
    setItem: (k: string, v: string) => void data.set(k, v),
    removeItem: (k: string) => void data.delete(k),
  };
}

afterEach(() => {
  vi.unstubAllGlobals();
});

describe("GeoCacheStore", () => {
  it("learns credential-free keys and returns fresh entries", () => {
    const store = new GeoCacheStore();
    store.learn([result]);
    const entries = store.fresh();
    expect(entries).toHaveLength(1);
    expect(entries[0].proxy).toBe("http://1.2.3.4:8080");
    expect(entries[0].country).toBe("Testland");
  });

  it("ignores dead results", () => {
    const store = new GeoCacheStore();
    store.learn([{ ...result, alive: false }]);
    expect(store.fresh()).toHaveLength(0);
  });

  it("never keys by a raw column line with a password", () => {
    // `host:port:user:pass` has no `@`; the normalized URL does.
    expect(geoKey({ proxy: "1.2.3.4:8080:user:secret", url: "http://user:secret@1.2.3.4:8080" }))
      .toBe("http://1.2.3.4:8080");
    expect(geoKey({ proxy: "1.2.3.4:8080:user:secret", url: "" })).toBeNull();
  });

  it("drops legacy raw keys from storage on load", () => {
    const legacy = JSON.stringify([
      ["1.2.3.4:8080:user:secret", { ip: "5.6.7.8", ts: Date.now() }],
      ["socks5://9.9.9.9:1080", { ip: "9.9.9.9", ts: Date.now() }],
    ]);
    const storage = memoryStorage({ "pp-geocache": legacy });
    vi.stubGlobal("localStorage", storage);
    const store = new GeoCacheStore();
    store.load();
    expect(store.fresh().map((e) => e.proxy)).toEqual(["socks5://9.9.9.9:1080"]);
    expect(storage.data.get("pp-geocache")).not.toContain("secret");
  });
});
