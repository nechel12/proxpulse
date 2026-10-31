import { invoke } from "@tauri-apps/api/core";

export type ParseResult = { list?: string[]; count: number; truncated: boolean };

/**
 * Extract proxies with the backend parser (`src-tauri/src/extract.rs`), the
 * same code path a check uses, so the live count matches what gets checked.
 * Parsing runs on a blocking backend thread, off the UI thread.
 */
export async function parseTextAsync(
  text: string,
  limit: number,
  wantList: boolean,
  defaultProto = "http",
): Promise<ParseResult> {
  const r = await invoke<{ count: number; truncated: boolean; list: string[] | null }>(
    "parse_proxy_text",
    { text, limit, wantList, defaultProto },
  );
  return { count: r.count, truncated: r.truncated, list: r.list ?? undefined };
}
