import { extractProxiesFromText } from "./parse.ts";

export type ParseResult = { list?: string[]; count: number; truncated: boolean };

type ParsePending = {
  resolve: (r: ParseResult) => void;
  reject: (e: unknown) => void;
  timer: number;
};

let parseWorker: Worker | null = null;
let parseSeq = 0;
const parsePending = new Map<number, ParsePending>();

function getParseWorker(): Worker | null {
  if (typeof Worker === "undefined") return null;
  if (parseWorker) return parseWorker;
  try {
    const w = new Worker(new URL("./parseWorker.ts", import.meta.url), {
      type: "module",
    });
    w.onmessage = (
      e: MessageEvent<{
        id: number;
        count: number;
        list?: string[];
        truncated: boolean;
        error?: string;
      }>,
    ) => {
      const pending = parsePending.get(e.data.id);
      if (!pending) return;
      window.clearTimeout(pending.timer);
      parsePending.delete(e.data.id);
      if (e.data.error) pending.reject(new Error(e.data.error));
      else pending.resolve({ list: e.data.list, count: e.data.count, truncated: e.data.truncated });
    };
    w.onerror = (e) => {
      for (const pending of parsePending.values()) {
        window.clearTimeout(pending.timer);
        pending.reject(new Error(e.message || "worker error"));
      }
      parsePending.clear();
      w.terminate();
      if (parseWorker === w) parseWorker = null;
    };
    parseWorker = w;
  } catch {
    parseWorker = null;
  }
  return parseWorker;
}

export function parseFallback(
  text: string,
  limit: number,
  wantList: boolean,
): ParseResult {
  const list = extractProxiesFromText(text, limit);
  return {
    list: wantList ? list : undefined,
    count: list.length,
    truncated: list.length >= limit,
  };
}

export function parseTextAsync(
  text: string,
  limit: number,
  wantList: boolean,
): Promise<ParseResult> {
  const w = getParseWorker();
  if (!w) return Promise.resolve(parseFallback(text, limit, wantList));
  return new Promise((resolve, reject) => {
    const id = ++parseSeq;
    const timer = window.setTimeout(() => {
      const pending = parsePending.get(id);
      if (!pending) return;
      parsePending.delete(id);
      pending.reject(new Error("parse timeout"));
    }, 20_000);
    parsePending.set(id, { resolve, reject, timer });
    try {
      w.postMessage({
        id,
        text,
        limit,
        mode: wantList ? "list" : "count",
      });
    } catch (e) {
      window.clearTimeout(timer);
      parsePending.delete(id);
      reject(e);
    }
  });
}
