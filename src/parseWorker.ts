// Background proxy-list parser. Keeps large pastes off the UI thread.
import { extractProxiesFromText } from "./parse.ts";

type Request = {
  id: number;
  text: string;
  limit: number;
  mode: "list" | "count";
};

type Reply = {
  id: number;
  count: number;
  truncated: boolean;
  list?: string[];
};

const ctx = self as unknown as {
  onmessage: ((ev: MessageEvent<Request>) => void) | null;
  postMessage: (msg: Reply) => void;
};

ctx.onmessage = (ev) => {
  const { id, text, limit, mode } = ev.data;
  try {
    const list = extractProxiesFromText(text, limit);
    ctx.postMessage({
      id,
      count: list.length,
      truncated: list.length >= limit,
      list: mode === "list" ? list : undefined,
    });
  } catch {
    ctx.postMessage({ id, count: 0, truncated: false, list: mode === "list" ? [] : undefined });
  }
};
