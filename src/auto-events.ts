import type { PoolItem } from "./pool.ts";
import { formatWebhookResults, type WebhookResult } from "./webhooks.ts";

export type AutoPoolEvent = { items: PoolItem[] };
export type AutoWebhookEvent = { results: WebhookResult[] };

type Translate = (key: string, params?: Record<string, string | number>) => string;
type StatusSetter = (id: string, render: () => string) => void;

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
