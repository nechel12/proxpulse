import { invoke } from "@tauri-apps/api/core";
import type { Translate } from "./ui-types.ts";

/** `HistSummary` in history.rs. */
export type HistSummary = {
  checks: number;
  alive: number;
  /** Share of alive checks, 0..1. */
  uptime: number;
  /** Uptime that trusts few checks less, for sorting. */
  score: number;
  lastAlive: number | null;
  lastCheck: number;
  avgMs: number | null;
};


/** History of the given proxy URLs; empty outside the desktop app. */
export async function fetchHistory(urls: string[]): Promise<Record<string, HistSummary>> {
  if (urls.length === 0) return {};
  try {
    return await invoke<Record<string, HistSummary>>("proxy_history", { urls: [...new Set(urls)] });
  } catch {
    return {};
  }
}

export function uptimeLabel(h: HistSummary | undefined): string {
  return h ? `${Math.round(h.uptime * 100)}%` : "—";
}

export function uptimeClass(h: HistSummary | undefined): string {
  if (!h) return "";
  return h.uptime >= 0.9 ? "ping-fast" : h.uptime >= 0.5 ? "ping-mid" : "ping-slow";
}

export function historyTitle(h: HistSummary | undefined, t: Translate, locale: string): string {
  if (!h) return t("hist_none");
  const when = h.lastAlive ? new Date(h.lastAlive * 1000).toLocaleString(locale) : t("hist_never");
  return t("hist_t", { a: h.alive, n: h.checks, when, ms: h.avgMs ?? "—" });
}
