// Sites requested through every alive proxy during a check.

import type { ProxyResult } from "./types.ts";

export const SITE_PRESETS: { url: string; label: string }[] = [
  { url: "https://www.google.com", label: "Google" },
  { url: "https://www.youtube.com", label: "YouTube" },
  { url: "https://web.telegram.org", label: "Telegram" },
  { url: "https://discord.com", label: "Discord" },
  { url: "https://chatgpt.com", label: "ChatGPT" },
];

/** Same limit as `MAX_SITES` in check.rs. */
export const MAX_SITES = 8;
const STORAGE_KEY = "pp-sites";

/** Host of a site URL, for labels and the filter. */
export function siteHost(url: string): string {
  const m = /^[a-z]+:\/\/([^/:?#]+)/i.exec(url);
  return (m?.[1] ?? url).replace(/^www\./, "");
}

/** Custom sites typed as hosts or URLs, separated by spaces or commas. */
export function parseCustomSites(text: string): string[] {
  return text
    .split(/[\s,;]+/)
    .map((s) => s.trim())
    .filter(Boolean)
    .map((s) => (/^https?:\/\//i.test(s) ? s : `https://${s}`))
    .filter((s) => /^https?:\/\/[a-z0-9.-]+\.[a-z]{2,}(:\d+)?(\/\S*)?$/i.test(s));
}

/** Reached and probed sites of a result; null when none were probed. */
export function siteSummary(r: ProxyResult): { ok: number; total: number } | null {
  const sites = r.sites ?? [];
  if (sites.length === 0) return null;
  return { ok: sites.filter((s) => s.ok).length, total: sites.length };
}

export class SitesPanel {
  selected = new Set<string>();
  custom = "";

  load(): void {
    try {
      const data = JSON.parse(localStorage.getItem(STORAGE_KEY) ?? "{}") as { selected?: unknown; custom?: unknown };
      const presets = new Set(SITE_PRESETS.map((p) => p.url));
      if (Array.isArray(data.selected)) {
        this.selected = new Set(data.selected.filter((u): u is string => typeof u === "string" && presets.has(u)));
      }
      if (typeof data.custom === "string") this.custom = data.custom.slice(0, 1000);
    } catch { /* defaults */ }
  }

  save(): void {
    try {
      localStorage.setItem(STORAGE_KEY, JSON.stringify({ selected: [...this.selected], custom: this.custom }));
    } catch { /* ignore */ }
  }

  urls(): string[] {
    const all = [...SITE_PRESETS.map((p) => p.url).filter((u) => this.selected.has(u)), ...parseCustomSites(this.custom)];
    return [...new Set(all)].slice(0, MAX_SITES);
  }

  init(onChange: () => void): void {
    this.load();
    const chips = document.getElementById("site-chips");
    if (chips) {
      chips.replaceChildren(...SITE_PRESETS.map((p) => {
        const b = document.createElement("button");
        b.type = "button";
        b.className = "preset-chip";
        b.textContent = p.label;
        b.dataset.url = p.url;
        b.addEventListener("click", () => {
          if (this.selected.has(p.url)) this.selected.delete(p.url);
          else this.selected.add(p.url);
          this.save();
          this.render();
          onChange();
        });
        return b;
      }));
    }
    const input = document.getElementById("sites-custom") as HTMLInputElement | null;
    if (input) {
      input.value = this.custom;
      input.addEventListener("input", () => {
        this.custom = input.value;
        this.save();
        onChange();
      });
    }
    this.render();
  }

  render(): void {
    document.querySelectorAll<HTMLButtonElement>("#site-chips .preset-chip").forEach((b) => {
      const on = this.selected.has(b.dataset.url ?? "");
      b.classList.toggle("active", on);
      b.setAttribute("aria-pressed", String(on));
    });
  }
}
