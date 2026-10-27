import { invoke } from "@tauri-apps/api/core";
import type { PoolItem } from "./pool.ts";

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

export class WebhookPanel {
  urls: string[] = [""];
  includeCreds = false;
  private sending = false;

  constructor(private onChange: () => void) {}

  load(): void {
    try {
      this.includeCreds = localStorage.getItem("pp-webhook-creds") === "1";
    } catch {
      this.includeCreds = false;
    }
    try {
      const raw = localStorage.getItem("pp-webhooks");
      const arr = raw ? JSON.parse(raw) : null;
      this.urls = Array.isArray(arr)
        ? arr.filter((s): s is string => typeof s === "string").slice(0, 10)
        : [""];
    } catch {
      this.urls = [""];
    }
    if (this.urls.length === 0) this.urls = [""];
  }

  saveCreds(): void {
    try {
      localStorage.setItem("pp-webhook-creds", this.includeCreds ? "1" : "0");
    } catch { /* ignore */ }
  }

  saveUrls(): void {
    try {
      localStorage.setItem("pp-webhooks", JSON.stringify(this.urls.slice(0, 10)));
    } catch { /* ignore */ }
    this.onChange();
  }

  render(): void {
    const box = document.getElementById("webhook-list");
    if (!box) return;
    box.innerHTML = "";
    this.urls.forEach((u, i) => {
      const row = document.createElement("div");
      row.className = "hrow";
      const inp = document.createElement("input");
      inp.value = u;
      inp.placeholder = "https://...";
      inp.spellcheck = false;
      inp.addEventListener("input", () => {
        this.urls[i] = inp.value;
        this.saveUrls();
      });
      const del = document.createElement("button");
      del.className = "btn ghost";
      del.textContent = "?";
      del.addEventListener("click", () => {
        this.urls.splice(i, 1);
        if (this.urls.length === 0) this.urls.push("");
        this.saveUrls();
        this.render();
      });
      row.appendChild(inp);
      row.appendChild(del);
      box.appendChild(row);
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
