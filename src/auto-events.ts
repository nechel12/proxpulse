import type { PoolItem } from "./pool.ts";
import { formatWebhookResults, type WebhookResult } from "./webhooks.ts";
import type { Translate, StatusSetter } from "./ui-types.ts";

/** Scheduler events name the pool they belong to. */
export type AutoPoolEvent = { pool?: string; items: PoolItem[] };
export type AutoWebhookEvent = { pool?: string; results: WebhookResult[] };


export function applyAutoPool(
  ev: AutoPoolEvent,
  set: (items: PoolItem[]) => void,
  after: () => void,
): void {
  set(ev.items.map((p) => ({
    raw: p.raw,
    url: p.url,
    latency: p.latency,
    country: p.country ?? null,
  })));
  after();
}

export function showAutoWebhook(
  ev: AutoWebhookEvent,
  t: Translate,
  setStatus: StatusSetter,
): void {
  setStatus("webhook-status", () => formatWebhookResults(ev.results, t));
}
