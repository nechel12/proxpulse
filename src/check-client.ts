import { Channel, invoke } from "@tauri-apps/api/core";
import { clip } from "./clip.ts";
import type { GeoEntry } from "./geocache.ts";
import type { ProxyResult } from "./types.ts";

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
  };
}

export async function runCheckList(
  list: string[],
  settings: CheckSettings,
  geoCache: (GeoEntry & { proxy: string })[],
  onProgress?: (done: number, total: number) => void,
  onLive?: (live: ProxyResult[]) => void,
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
  const channel = new Channel<ProxyResult>();
  let finishDone: () => void = () => {};
  const done = new Promise<void>((resolve) => {
    finishDone = resolve;
  });
  let finished = false;
  channel.onmessage = (r) => {
    if (finished) return;
    if (r.proto === "__done__") {
      finishDone();
      return;
    }
    pushLive(r);
    paint(out.length, list.length);
    if (out.length >= list.length) finishDone();
  };
  try {
    await invoke<void>("check_proxies", {
      proxies: list,
      testUrl: settings.testUrl,
      timeoutMs: settings.timeoutMs,
      concurrency: settings.concurrency,
      defaultProto: settings.defaultProto,
      repeats: settings.repeats,
      withGeo: settings.withGeo,
      withAnonymity: settings.withAnonymity,
      withTamper: settings.withTamper,
      withTls: settings.withTls,
      precheck: settings.precheck,
      precheckTimeoutMs: settings.precheckTimeoutMs,
      judgeMode: settings.judgeMode,
      ipapiKey: settings.ipapiKey,
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
