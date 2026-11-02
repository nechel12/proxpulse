import type { ProxyResult } from "./types.ts";
import {
  anonShort,
  escapeHtml,
  geoShort,
  maskProxy,
  pingClass,
  rowTitle,
  stabilityShort,
  tlsShort,
} from "./format.ts";
import {
  exitIpCounts,
  filterOptions,
  filterResults,
  nextSort,
  sortResults,
  type Dedupe,
  type ResultFilter,
  type Sort,
  type SortKey,
} from "./results-model.ts";
import { VirtualRows } from "./virtual-rows.ts";

export type { ResultFilter } from "./results-model.ts";
export type Translate = (key: string, params?: Record<string, string | number>) => string;
export type RowAction = "copy" | "recheck" | "pool" | "remove";

type Deps = {
  /** Re-render with the current results after a sort or filter change. */
  onChange: () => void;
  onAction: (action: RowAction, rows: ProxyResult[]) => void;
  canRecheck: () => boolean;
};

const COLS = 11;
const ANON_VALUES = ["elite", "anonymous", "transparent", "unknown"];

const $ = <T extends HTMLElement>(id: string) => document.getElementById(id) as T | null;

export class ResultsView {
  hideCreds = false;
  sort: Sort = null;
  private rows: ProxyResult[] = [];
  private selected = new Set<string>();
  private anchor = -1;
  private virtual: VirtualRows<ProxyResult> | null = null;
  private t: Translate = (k) => k;
  private optionsKey = "";
  private shared = new Map<string, number>();
  private deps: Deps | null = null;

  init(deps: Deps): void {
    this.deps = deps;
    const wrap = $("results-wrap");
    const body = $<HTMLTableSectionElement>("tbody");
    if (!wrap || !body) return;
    this.virtual = new VirtualRows(wrap, body, COLS, (r, i) => this.row(r, i));

    document.querySelectorAll<HTMLButtonElement>("#results-head [data-sort]").forEach((b) => {
      b.addEventListener("click", () => {
        this.sort = nextSort(this.sort, b.dataset.sort as SortKey);
        wrap.scrollTop = 0;
        deps.onChange();
      });
    });
    for (const id of ["f-proto", "f-anon", "f-country", "f-dedupe"]) {
      $(id)?.addEventListener("change", () => {
        this.clearSelection();
        wrap.scrollTop = 0;
        deps.onChange();
      });
    }

    body.addEventListener("mousedown", (e) => {
      // Shift and Ctrl clicks select rows, not text.
      if ((e.shiftKey || e.ctrlKey || e.metaKey) && (e.target as HTMLElement).closest("tr[data-index]")) e.preventDefault();
    });
    body.addEventListener("click", (e) => {
      const tr = (e.target as HTMLElement).closest<HTMLElement>("tr[data-index]");
      if (!tr) return;
      this.select(Number(tr.dataset.index), e.shiftKey, e.ctrlKey || e.metaKey);
    });
    body.addEventListener("contextmenu", (e) => {
      const tr = (e.target as HTMLElement).closest<HTMLElement>("tr[data-index]");
      if (!tr) return;
      e.preventDefault();
      const i = Number(tr.dataset.index);
      const r = this.rows[i];
      if (r && !this.selected.has(r.proxy)) this.select(i, false, false);
      this.openMenu(e.clientX, e.clientY);
    });
    wrap.addEventListener("keydown", (e) => {
      if ((e.ctrlKey || e.metaKey) && e.code === "KeyA") {
        e.preventDefault();
        this.selected = new Set(this.rows.map((r) => r.proxy));
        this.virtual?.repaint();
      } else if (e.key === "Delete" && this.selected.size > 0) {
        e.preventDefault();
        deps.onAction("remove", this.selectedRows());
      } else if (e.key === "Escape" && this.selected.size > 0) {
        e.preventDefault();
        this.clearSelection();
      }
    });

    const menu = $("row-menu");
    menu?.addEventListener("click", (e) => {
      const b = (e.target as HTMLElement).closest<HTMLButtonElement>("button[data-act]");
      if (!b || b.disabled) return;
      this.closeMenu();
      deps.onAction(b.dataset.act as RowAction, this.selectedRows());
    });
    document.addEventListener("mousedown", (e) => {
      if (menu && !menu.classList.contains("hidden") && !menu.contains(e.target as Node)) this.closeMenu();
    });
    document.addEventListener("keydown", (e) => {
      if (e.key === "Escape" && menu && !menu.classList.contains("hidden")) {
        e.preventDefault();
        this.closeMenu();
      }
    });
    wrap.addEventListener("scroll", () => this.closeMenu(), { passive: true });
    window.addEventListener("blur", () => this.closeMenu());
  }

  /** A new check: no selection, back to the top. */
  reset(): void {
    this.selected.clear();
    this.anchor = -1;
    this.virtual?.set([], false);
  }

  clearSelection(): void {
    if (this.selected.size === 0) return;
    this.selected.clear();
    this.anchor = -1;
    this.virtual?.repaint();
  }

  selectedRows(): ProxyResult[] {
    return this.rows.filter((r) => this.selected.has(r.proxy));
  }

  render(results: ProxyResult[], status: ResultFilter, t: Translate, visible: () => boolean): void {
    this.t = t;
    if (!visible()) return;
    this.updateOptions(results);
    this.renderSortState();
    this.shared = exitIpCounts(results);
    const body = $<HTMLTableSectionElement>("tbody");
    if (!body || !this.virtual) return;
    this.rows = sortResults(filterResults(results, {
      status,
      query: $<HTMLInputElement>("search")?.value ?? "",
      proto: $<HTMLSelectElement>("f-proto")?.value ?? "",
      anon: $<HTMLSelectElement>("f-anon")?.value ?? "",
      country: $<HTMLSelectElement>("f-country")?.value ?? "",
      dedupe: ($<HTMLSelectElement>("f-dedupe")?.value ?? "") as Dedupe,
    }), this.sort);
    if (this.rows.length === 0) {
      this.virtual.set([]);
      const tr = document.createElement("tr");
      tr.className = "empty";
      const td = document.createElement("td");
      td.colSpan = COLS;
      td.textContent = t(results.length === 0 ? "empty_idle" : "empty_filter");
      tr.appendChild(td);
      body.replaceChildren(tr);
      return;
    }
    this.virtual.set(this.rows);
  }

  private select(i: number, range: boolean, toggle: boolean): void {
    const r = this.rows[i];
    if (!r) return;
    if (range && this.anchor >= 0) {
      const [a, b] = this.anchor < i ? [this.anchor, i] : [i, this.anchor];
      if (!toggle) this.selected.clear();
      for (let k = a; k <= b; k++) {
        const row = this.rows[k];
        if (row) this.selected.add(row.proxy);
      }
    } else if (toggle) {
      if (this.selected.has(r.proxy)) this.selected.delete(r.proxy);
      else this.selected.add(r.proxy);
      this.anchor = i;
    } else {
      this.selected = new Set([r.proxy]);
      this.anchor = i;
    }
    this.virtual?.repaint();
  }

  private openMenu(x: number, y: number): void {
    const menu = $("row-menu");
    if (!menu || !this.deps) return;
    const rows = this.selectedRows();
    const alive = rows.filter((r) => r.alive).length;
    const label = (act: RowAction, text: string, disabled = false) => {
      const b = menu.querySelector<HTMLButtonElement>(`button[data-act="${act}"]`);
      if (!b) return;
      b.textContent = text;
      b.disabled = disabled;
    };
    const t = this.t;
    label("copy", t("ctx_copy", { n: rows.length }));
    label("recheck", t("ctx_recheck", { n: rows.length }), !this.deps.canRecheck());
    label("pool", t("ctx_pool", { n: alive }), alive === 0);
    label("remove", t("ctx_remove", { n: rows.length }));
    menu.classList.remove("hidden");
    const w = menu.offsetWidth;
    const h = menu.offsetHeight;
    menu.style.left = `${Math.max(4, Math.min(x, window.innerWidth - w - 4))}px`;
    menu.style.top = `${Math.max(4, Math.min(y, window.innerHeight - h - 4))}px`;
    menu.querySelector<HTMLButtonElement>("button:not(:disabled)")?.focus();
  }

  private closeMenu(): void {
    $("row-menu")?.classList.add("hidden");
  }

  private renderSortState(): void {
    document.querySelectorAll<HTMLButtonElement>("#results-head [data-sort]").forEach((b) => {
      const th = b.closest("th");
      if (!th) return;
      const on = this.sort?.key === b.dataset.sort;
      th.setAttribute("aria-sort", on ? (this.sort?.dir === "asc" ? "ascending" : "descending") : "none");
    });
  }

  /** Rebuild the filter menus when the language or the set of values changes. */
  private updateOptions(results: ProxyResult[]): void {
    const t = this.t;
    const opts = filterOptions(results);
    const countries = opts.country.map(([c]) => c).sort((a, b) => a.localeCompare(b));
    const key = [t("f_proto_all"), opts.proto.join(","), countries.join(",")].join("|");
    if (key === this.optionsKey) return;
    this.optionsKey = key;
    const fill = (id: string, all: string, values: [string, string][]) => {
      const sel = $<HTMLSelectElement>(id);
      if (!sel) return;
      const current = sel.value;
      sel.replaceChildren(new Option(all, ""), ...values.map(([v, text]) => new Option(text, v)));
      sel.value = values.some(([v]) => v === current) ? current : "";
    };
    fill("f-proto", t("f_proto_all"), opts.proto.map((p) => [p, p]));
    fill("f-anon", t("f_anon_all"), ANON_VALUES.map((a) => [a, a === "unknown" ? t("anon_unknown") : a]));
    fill("f-country", t("f_country_all"), countries.map((c) => [c, c]));
  }

  private row(r: ProxyResult, i: number): HTMLTableRowElement {
    const t = this.t;
    const tr = document.createElement("tr");
    tr.dataset.index = String(i);
    if (this.selected.has(r.proxy)) {
      tr.className = "selected";
      tr.setAttribute("aria-selected", "true");
    }
    const pill = r.alive ? `<span class="pill alive">${t("pill_alive")}</span>` : `<span class="pill dead">${t("pill_dead")}</span>`;
    const ping = r.latency_ms == null ? "-" : `${r.latency_ms} ms`;
    const an = anonShort(r.anonymity);
    const tls = tlsShort(r.tls);
    const marks = [
      r.tamper === "modified" ? `<span class="anon-trans" title="tamper">?mod</span>` : "",
      r.tls === "modified" ? `<span class="anon-trans" title="tls">?tls</span>` : "",
    ].join(" ");
    // Several "different" proxies often leave through one exit IP.
    const same = r.ip ? this.shared.get(r.ip) ?? 0 : 0;
    const shared = same > 1 ? ` <span class="geo-tag" title="${escapeHtml(t("same_ip_t", { n: same }))}">×${same}</span>` : "";
    const info = r.alive ? `${escapeHtml(r.ip ?? "ok")}${shared} ${marks}` : escapeHtml(r.error ?? "fail");
    const infoTitle = escapeHtml(rowTitle(r));
    // An empty cell for a stage skipped at its limit gets an explanation
    // instead of looking like "no data".
    const skipped = new Set(r.skipped ?? []);
    const cell = (stage: string, txt: string) =>
      r.alive && txt === "—" && skipped.has(stage)
        ? `<span class="dim" title="${escapeHtml(t("skipped_hint"))}">${escapeHtml(t("skipped_short"))}</span>`
        : escapeHtml(txt);
    tr.innerHTML =
      `<td>${i + 1}</td><td>${escapeHtml(this.hideCreds ? maskProxy(r.proxy, r.url) : r.proxy)}</td><td>${escapeHtml(r.proto)}</td>` +
      `<td>${pill}</td><td class="${pingClass(r.latency_ms)}">${ping}</td>` +
      `<td>${escapeHtml(stabilityShort(r))}</td><td class="${an.cls}">${cell("anon", an.txt)}</td>` +
      `<td class="cell-geo" title="${infoTitle}">${cell("geo", geoShort(r))}</td>` +
      `<td>${escapeHtml(r.ip_type ?? "-")}</td>` +
      `<td class="${tls.cls}" title="${infoTitle}">${cell("tls", tls.txt)}</td>` +
      `<td class="cell-err" title="${infoTitle}">${info}</td>`;
    return tr;
  }
}
