import { parseUrlList, renderUrlList } from "./url-list.ts";
import { MAIN_POOL, scopedKey } from "./pools.ts";

export type SubscriptionResult = {
  index: number;
  ok: boolean;
  found: number;
  added: number;
  error: string | null;
};

type Translate = (key: string, params?: Record<string, string | number>) => string;

const STORAGE_KEY = "pp-subscriptions";
const MAX = 10;

export function formatSubscriptionResults(results: SubscriptionResult[], t: Translate): string {
  return results
    .map((r) => `#${r.index + 1} ${r.ok
      ? t("subs_ok", { n: r.found, m: r.added })
      : t("subs_fail", { e: r.error ?? "?" })}`)
    .join(" | ");
}

/** Proxy list URLs fetched by the scheduler before every check. */
export class SubscriptionPanel {
  urls: string[] = [""];
  /** The pool whose subscriptions are shown and stored. */
  pool = MAIN_POOL;

  constructor(private onChange: () => void) {}

  load(): void {
    try {
      this.urls = parseUrlList(localStorage.getItem(scopedKey(STORAGE_KEY, this.pool)), MAX);
    } catch {
      this.urls = [""];
    }
  }

  save(): void {
    try {
      localStorage.setItem(scopedKey(STORAGE_KEY, this.pool), JSON.stringify(this.urls.slice(0, MAX)));
    } catch { /* ignore */ }
    this.onChange();
  }

  getUrls(): string[] {
    return this.urls.map((s) => s.trim()).filter((s) => /^https?:\/\//i.test(s)).slice(0, MAX);
  }

  add(): void {
    if (this.urls.length >= MAX) return;
    this.urls.push("");
  }

  render(t: Translate): void {
    const box = document.getElementById("subs-list");
    if (!box) return;
    renderUrlList(box, this.urls, {
      placeholder: "https://…/proxies.txt",
      removeLabel: t("subs_remove"),
      onChange: () => this.save(),
      onRemove: () => this.render(t),
    });
  }
}
