import { invoke } from "@tauri-apps/api/core";
import type { PoolItem } from "./pool.ts";
import { parseUrlList, renderUrlList } from "./url-list.ts";

export type WebhookResult = { index: number; ok: boolean; message: string };
export type Translate = (key: string, params?: Record<string, string | number>) => string;
export type StatusSetter = (id: string, render: () => string) => void;

export function formatWebhookResults(results: WebhookResult[], t: Translate): string {
  return results
    .map((r) => {
      const n = r.index < 100 ? r.index + 1 : "?";
      return `#${n} ${r.ok
        ? t("webhook_sent", { r: r.message })
        : t("webhook_fail", { e: r.message })}`;
    })
    .join(" | ");
}

/** Example body for the "template" format. */
export const DEFAULT_TEMPLATE = `{"count": {count}, "time": "{time}", "proxies": {proxies_json}}`;

export class WebhookPanel {
  urls: string[] = [""];
  includeCreds = false;
  /** Scheduled sends carry only additions and removals. */
  changesOnly = false;
  template = DEFAULT_TEMPLATE;
  private sending = false;

  constructor(private onChange: () => void) {}

  load(): void {
    try {
      this.includeCreds = localStorage.getItem("pp-webhook-creds") === "1";
      this.changesOnly = localStorage.getItem("pp-webhook-changes") === "1";
      this.template = localStorage.getItem("pp-webhook-template") ?? DEFAULT_TEMPLATE;
    } catch {
      this.includeCreds = false;
    }
    try {
      this.urls = parseUrlList(localStorage.getItem("pp-webhooks"));
    } catch {
      this.urls = [""];
    }
  }

  saveCreds(): void {
    try {
      localStorage.setItem("pp-webhook-creds", this.includeCreds ? "1" : "0");
    } catch { /* ignore */ }
  }

  saveOptions(): void {
    try {
      localStorage.setItem("pp-webhook-changes", this.changesOnly ? "1" : "0");
      localStorage.setItem("pp-webhook-template", this.template);
    } catch { /* ignore */ }
    this.onChange();
  }

  /** Show the template field and the URL hint for the selected format. */
  renderFormat(t: Translate): void {
    const format = (document.getElementById("webhook-format") as HTMLSelectElement | null)?.value ?? "txt";
    document.getElementById("webhook-template-wrap")?.classList.toggle("hidden", format !== "template");
    const hint = document.getElementById("webhook-format-hint");
    if (!hint) return;
    const key = format === "telegram" ? "webhook_tg_hint" : format === "discord" ? "webhook_dc_hint" : format === "template" ? "webhook_tpl_hint" : "";
    hint.classList.toggle("hidden", !key);
    hint.textContent = key ? t(key) : "";
  }

  saveUrls(): void {
    try {
      localStorage.setItem("pp-webhooks", JSON.stringify(this.urls.slice(0, 10)));
    } catch { /* ignore */ }
    this.onChange();
  }

  render(t: Translate): void {
    const box = document.getElementById("webhook-list");
    if (!box) return;
    renderUrlList(box, this.urls, {
      placeholder: "https://...",
      removeLabel: t("webhook_remove"),
      onChange: () => this.saveUrls(),
      onRemove: () => this.render(t),
    });
  }

  getUrls(): string[] {
    return this.urls.map((s) => s.trim()).filter(Boolean).slice(0, 10);
  }

  async send(
    manual: boolean,
    items: PoolItem[],
    t: Translate,
    setStatus: StatusSetter,
  ): Promise<void> {
    const urls = this.getUrls();
    if (urls.length === 0) {
      if (manual) setStatus("webhook-status", () => t("webhook_hint"));
      return;
    }
    if (items.length === 0) {
      setStatus("webhook-status", () => t("webhook_empty"));
      return;
    }
    if (this.sending) return;
    this.sending = true;
    const format = (document.getElementById("webhook-format") as HTMLSelectElement | null)?.value ?? "txt";
    setStatus("webhook-status", () => "...");
    try {
      const results = await invoke<WebhookResult[]>("send_webhooks", {
        urls,
        format,
        items,
        includeCredentials: this.includeCreds,
        template: this.template,
      });
      setStatus("webhook-status", () => formatWebhookResults(results, t));
    } catch (e) {
      const msg = String(e).slice(0, 120);
      setStatus("webhook-status", () => t("webhook_fail", { e: msg }));
    } finally {
      this.sending = false;
    }
  }
}
