import { Channel, invoke } from "@tauri-apps/api/core";
import { clip } from "./clip.ts";
import type { GeoEntry } from "./geocache.ts";
import type { CheckEvent, ProxyResult } from "./types.ts";

/** Same shape and limits as `CheckConfig` in `src-tauri/src/commands/check.rs`. */
export const CHECK_LIMITS = {
  timeoutMs: [1000, 30000],
  concurrency: [1, 500],
  repeats: [1, 5],
  precheckTimeoutMs: [300, 10000],
} as const;

/**
 * A whole number within the limits. The backend takes integers only: "2.5"
 * threads made it reject the whole check. An empty or unreadable field takes
 * the default; 0 is a number, so it becomes the minimum, not the default.
 */
export function clampInt(raw: string, [min, max]: readonly [number, number], fallback: number): number {
  const n = Number(raw);
  if (raw.trim() === "" || !Number.isFinite(n)) return fallback;
  return Math.max(min, Math.min(max, Math.round(n)));
}

export type CheckSettings = {
  testUrl: string;
  timeoutMs: number;
  concurrency: number;
  repeats: number;
  defaultProto: string;
  withGeo: boolean;
  withAnonymity: boolean;
  withTamper: boolean;
  withTls: boolean;
  precheck: boolean;
  precheckTimeoutMs: number;
  judgeMode: boolean;
  ipapiKey: string | null;
  /** Sites requested through every alive proxy. */
  sites: string[];
  /** Judge over HTTPS and HTTP: the plain-HTTP request for anonymity. */
  judgeHttpUrl: string | null;
  /** Look exit IPs up in DNS blocklists. */
  withDnsbl: boolean;
};

export function deadStub(proxy: string, msg: string): ProxyResult {
  return {
    proxy,
    url: proxy,
    proto: "ERR",
    alive: false,
    latency_ms: null,
    jitter_ms: null,
    success_rate: 0,
    attempts: 0,
    ip: null,
    country: null,
    country_code: null,
    city: null,
    isp: null,
    org: null,
    asn: null,
    ip_type: null,
    anonymity: null,
    tamper: null,
    tls: null,
    tls_info: null,
    error: clip(msg, 120),
    skipped: [],
  };
}

export async function runCheckList(
  list: string[],
  settings: CheckSettings,
  geoCache: (GeoEntry & { proxy: string })[],
  onProgress?: (done: number, total: number) => void,
  onLive?: (live: ProxyResult[]) => void,
  onKeyOverHttp?: () => void,
  /** No proxy was alive and the check server did not answer directly. */
  onUnreachable?: () => void,
): Promise<ProxyResult[]> {
  if (!/^https?:\/\//i.test(settings.testUrl)) throw new Error("badurl");
  const out: ProxyResult[] = [];
  const seen = new Set<string>();
  const pushLive = (r: ProxyResult) => {
    if (r.error === "cancelled" || seen.has(r.proxy)) return;
    seen.add(r.proxy);
    out.push(r);
  };
  let lastPaint = 0;
  const paint = (done: number, total: number) => {
    const now = performance.now();
    if (now - lastPaint < 300 && done < total) return;
    lastPaint = now;
    if (done >= total) {
      out.sort((a, b) => Number(b.alive) - Number(a.alive) || (a.latency_ms ?? 1e12) - (b.latency_ms ?? 1e12));
    }
    onLive?.([...out]);
    onProgress?.(done, total);
  };
  const channel = new Channel<CheckEvent>();
  let finishDone: () => void = () => {};
  const done = new Promise<void>((resolve) => {
    finishDone = resolve;
  });
  let finished = false;
  channel.onmessage = (ev) => {
    if (finished) return;
    if (ev.kind === "done") {
      finishDone();
      return;
    }
    if (ev.kind === "keyOverHttp") {
      onKeyOverHttp?.();
      return;
    }
    if (ev.kind === "unreachable") {
      onUnreachable?.();
      return;
    }
    const { kind: _kind, ...r } = ev;
    pushLive(r);
    paint(out.length, list.length);
    if (out.length >= list.length) finishDone();
  };
  try {
    await invoke<void>("check_proxies", {
      proxies: list,
      config: settings,
      geoCache,
      onResult: channel,
    });
    await done;
  } catch (e) {
    const msg = String(e);
    if (msg.includes("another check is running")) throw e;
    for (const p of list) pushLive(deadStub(p, msg));
  } finally {
    finished = true;
    finishDone();
  }
  paint(out.length, list.length);
  out.sort((a, b) => Number(b.alive) - Number(a.alive) || (a.latency_ms ?? 1e12) - (b.latency_ms ?? 1e12));
  return out;
}
