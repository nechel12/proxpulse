import { escapeHtml, maskProxy, pingClass } from "./format.ts";
import { VirtualRows } from "./virtual-rows.ts";
import { historyTitle, uptimeClass, uptimeLabel, type HistSummary } from "./history.ts";
import type { Translate } from "./ui-types.ts";
export type { Translate };

export type PoolItem = {
  raw: string;
  url: string;
  latency: number | null;
  country: string | null;
  /** Results of earlier checks; filled in by the UI. */
  history?: HistSummary;
};


export class PoolView {
  items: PoolItem[] = [];
  hideCreds = false;
  private views = new Map<string, VirtualRows<PoolItem>>();
  private onDelete: (raw: string) => void = () => {};
  private t: Translate = (k) => k;
  locale = "ru";

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

  /** A new filter or sort starts from the top of the table. */
  resetScroll(tbodyId: string): void {
    const wrap = document.getElementById(tbodyId)?.closest<HTMLElement>(".table-wrap");
    if (wrap) wrap.scrollTop = 0;
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

  private view(tbodyId: string): VirtualRows<PoolItem> | null {
    const existing = this.views.get(tbodyId);
    if (existing) return existing;
    const body = document.getElementById(tbodyId) as HTMLTableSectionElement | null;
    const wrap = body?.closest<HTMLElement>(".table-wrap");
    if (!body || !wrap) return null;
    const withDelete = tbodyId === "pool-body";
    const view = new VirtualRows<PoolItem>(wrap, body, withDelete ? 4 : 3, (p, n) => this.row(p, n, withDelete));
    if (withDelete) {
      body.addEventListener("click", (e) => {
        const b = (e.target as HTMLElement).closest<HTMLButtonElement>("button[data-raw]");
        if (b?.dataset.raw != null) this.onDelete(b.dataset.raw);
      });
    }
    this.views.set(tbodyId, view);
    return view;
  }

  private row(p: PoolItem, n: number, withDelete: boolean): HTMLTableRowElement {
    const tr = document.createElement("tr");
    const ping = p.latency == null ? "-" : `${p.latency} ms`;
    const geo = p.country ? ` <span class="geo-tag">${escapeHtml(p.country)}</span>` : "";
    const up = p.history
      ? ` <span class="geo-tag ${uptimeClass(p.history)}" title="${escapeHtml(historyTitle(p.history, this.t, this.locale))}">${uptimeLabel(p.history)}</span>`
      : "";
    tr.innerHTML = `<td>${n + 1}</td><td>${escapeHtml(this.hideCreds ? maskProxy(p.raw, p.url) : p.raw)}${geo}</td><td class="${pingClass(p.latency)}">${ping}${up}</td>`;
    if (withDelete) {
      const td = document.createElement("td");
      const b = document.createElement("button");
      b.className = "btn ghost";
      b.textContent = "×";
      b.title = this.t("pool_remove");
      b.setAttribute("aria-label", this.t("pool_remove"));
      b.dataset.raw = p.raw;
      td.appendChild(b);
      tr.appendChild(td);
    }
    return tr;
  }

  private renderInto(tbodyId: string, t: Translate, onDelete: (raw: string) => void): void {
    this.t = t;
    this.onDelete = onDelete;
    const pb = document.getElementById(tbodyId) as HTMLTableSectionElement | null;
    const view = this.view(tbodyId);
    if (!pb || !view) return;
    const filter = (document.querySelector(`[data-pool-filter="${tbodyId}"]`) as HTMLInputElement | null)?.value.toLowerCase().trim() ?? "";
    const sort = (document.querySelector(`[data-pool-sort="${tbodyId}"]`) as HTMLSelectElement | null)?.value ?? "added";
    let rows = this.items;
    if (filter) {
      rows = rows.filter((r) =>
        r.raw.toLowerCase().includes(filter) || (r.country ?? "").toLowerCase().includes(filter)
      );
    }
    if (sort === "ping") rows = [...rows].sort((a, b) => (a.latency ?? 1e12) - (b.latency ?? 1e12));
    else if (sort === "country") rows = [...rows].sort((a, b) => (a.country ?? "~~~").localeCompare(b.country ?? "~~~"));
    else if (sort === "score") {
      rows = [...rows].sort((a, b) => (b.history?.score ?? -1) - (a.history?.score ?? -1) || (a.latency ?? 1e12) - (b.latency ?? 1e12));
    }

    if (rows.length === 0) {
      view.set([]);
      const tr = document.createElement("tr");
      tr.className = "empty";
      const td = document.createElement("td");
      td.colSpan = tbodyId === "pool-body" ? 4 : 3;
      td.textContent = t(this.items.length === 0 ? "pool_empty" : "empty_filter");
      tr.appendChild(td);
      pb.replaceChildren(tr);
      return;
    }
    view.set(rows);
  }
}
