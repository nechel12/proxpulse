import type { Translate } from "./ui-types.ts";
// Named pools: ids, labels and per-pool storage keys.

export const MAIN_POOL = "main";
/** The main pool plus nine named ones (`MAX_POOLS` in pools.rs). */
export const MAX_POOLS = 10;

/** `PoolMeta` in pools.rs. */
export type PoolMeta = { id: string; name: string };


/** Settings that belong to one pool: the schedule, webhooks, subscriptions. */
export const POOL_KEYS = [
  "pp-auto-on",
  "pp-auto-min",
  "pp-auto-webhooks",
  "pp-maxping",
  "pp-webhooks",
  "pp-webhook-format",
  "pp-webhook-creds",
  "pp-webhook-changes",
  "pp-webhook-template",
  "pp-subscriptions",
] as const;

/** A pool's storage key; the main pool keeps the keys older versions used. */
export function scopedKey(base: string, pool: string): string {
  return pool === MAIN_POOL ? base : `${base}@${pool}`;
}

export function poolLabel(p: PoolMeta, t: Translate): string {
  return p.name || (p.id === MAIN_POOL ? t("pool_main") : p.id);
}

/** Pool ids are short lowercase letters and digits (`valid_pool_id` in pools.rs). */
export function validPoolId(id: string): boolean {
  return /^[a-z0-9]{1,16}$/.test(id);
}

type KeyStore = Pick<Storage, "removeItem">;

/** Remove every setting of a deleted pool. */
export function forgetPoolSettings(storage: KeyStore, pool: string): void {
  if (pool === MAIN_POOL) return;
  for (const base of POOL_KEYS) storage.removeItem(scopedKey(base, pool));
}
