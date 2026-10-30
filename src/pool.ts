import { escapeHtml, pingClass } from "./format.ts";
import { RENDER_STEP } from "./results.ts";

export const POOL_RENDER_MAX = 1000;

export type PoolItem = {
  raw: string;
  url: string;
  latency: number | null;
  country: string | null;
};

export type Translate = (key: string, params?: Record<string, string | number>) => string;

export class PoolView {
  items: PoolItem[] = [];
  private limits = new Map<string, number>();

  get count(): number {
    return this.items.length;
  }

  set(items: PoolItem[]): void {
    this.items = items;
  }

  clear(): void {
    this.items = [];
  }

  remove(raw: string): void {
    const at = this.items.findIndex((x) => x.raw === raw);
    if (at >= 0) this.items.splice(at, 1);
  }

  resetLimit(tbodyId: string): void {
    this.limits.set(tbodyId, RENDER_STEP);
  }

  /** Keep existing entries, add new ones and refresh latency. */
  merge(
    incoming: { raw: string; url?: string | null; latency: number | null; country?: string | null }[],
    maxPingMs: number,
    maxItems: number,
  ): void {
    const keyOf = (it: { raw: string; url?: string | null }) => it.url || it.raw;
    const idx = new Map(this.items.map((p, i) => [keyOf(p), i]));
    const drop = new Set<number>();
    for (const it of incoming) {
      const key = keyOf(it);
      const at = idx.get(key);
      if (maxPingMs > 0 && it.latency != null && it.latency > maxPingMs) {
        if (at != null) drop.add(at);
        continue;
      }
      if (at == null) {
        if (this.items.length >= maxItems) continue;
        idx.set(key, this.items.length);
        this.items.push({
          raw: it.raw,
          url: it.url || it.raw,
          latency: it.latency,
          country: it.country ?? null,
        });
      } else {
        if (it.url) this.items[at].url = it.url;
        if (it.latency != null) this.items[at].latency = it.latency;
        if (it.country != null) this.items[at].country = it.country;
      }
    }
    if (drop.size > 0) {
      this.items = this.items.filter((_, i) => !drop.has(i));
    }
  }

  render(
    t: Translate,
    onDelete: (raw: string) => void,
    visible: (id: string) => boolean,
  ): void {
    if (visible("screen-dispatch")) this.renderInto("pool-body", t, onDelete);
    if (visible("screen-auto")) this.renderInto("auto-pool-body", t, onDelete);
  }

  private renderInto(
    tbodyId: string,
    t: Translate,
    onDelete: (raw: string) => void,
  ): void {
    const pb = document.getElementById(tbodyId) as HTMLTableSectionElement | null;
    if (!pb) return;
    pb.innerHTML = "";
    const cols = tbodyId === "pool-body" ? 4 : 3;
    const filter = (document.querySelector(`[data-pool-filter="${tbodyId}"]`) as HTMLInputElement | null)?.value.toLowerCase().trim() ?? "";
    const sort = (document.querySelector(`[data-pool-sort="${tbodyId}"]`) as HTMLSelectElement | null)?.value ?? "added";
    let rows = this.items.map((p, i) => ({ ...p, i }));
    if (filter) {
      rows = rows.filter((r) =>
        r.raw.toLowerCase().includes(filter) || (r.country ?? "").toLowerCase().includes(filter)
      );
    }
    if (sort === "ping") rows.sort((a, b) => (a.latency ?? 1e12) - (b.latency ?? 1e12));
    else if (sort === "country") rows.sort((a, b) => (a.country ?? "~~~").localeCompare(b.country ?? "~~~"));

    if (rows.length === 0) {
      const tr = document.createElement("tr");
      tr.className = "empty";
      const td = document.createElement("td");
      td.colSpan = cols;
      td.textContent = t(this.items.length === 0 ? "pool_empty" : "empty_filter");
      tr.appendChild(td);
      pb.appendChild(tr);
      return;
    }
    const limit = Math.min(this.limits.get(tbodyId) ?? RENDER_STEP, POOL_RENDER_MAX);
    rows.slice(0, limit).forEach((p, n) => {
      const tr = document.createElement("tr");
      const ping = p.latency == null ? "-" : `${p.latency} ms`;
      const geo = p.country ? ` <span class="geo-tag">${escapeHtml(p.country)}</span>` : "";
      tr.innerHTML = `<td>${n + 1}</td><td>${escapeHtml(p.raw)}${geo}</td><td class="${pingClass(p.latency)}">${ping}</td>`;
      if (tbodyId === "pool-body") {
        const td = document.createElement("td");
        const b = document.createElement("button");
        b.className = "btn ghost";
        b.textContent = "×";
        b.title = t("pool_remove");
        b.setAttribute("aria-label", t("pool_remove"));
        b.addEventListener("click", () => onDelete(p.raw));
        td.appendChild(b);
        tr.appendChild(td);
      }
      pb.appendChild(tr);
    });
    if (rows.length > limit) {
      const tr = document.createElement("tr");
      const td = document.createElement("td");
      td.colSpan = cols;
      td.style.textAlign = "center";
      if (limit < POOL_RENDER_MAX) {
        const b = document.createElement("button");
        b.className = "btn ghost";
        b.textContent = t("show_more", { n: rows.length - limit });
        b.addEventListener("click", () => {
          this.limits.set(tbodyId, Math.min(POOL_RENDER_MAX, limit + RENDER_STEP));
          this.renderInto(tbodyId, t, onDelete);
        });
        td.appendChild(b);
      } else {
        td.className = "empty";
        td.textContent = t("render_limit", { n: limit, total: rows.length });
      }
      tr.appendChild(td);
      pb.appendChild(tr);
    }
  }
}
