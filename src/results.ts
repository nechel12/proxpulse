import type { ProxyResult } from "./types.ts";
import {
  errorText,
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
import { siteHost, siteSummary } from "./sites.ts";
import { speedLabel } from "./speed-client.ts";
import { historyTitle, uptimeClass, uptimeLabel } from "./history.ts";
import type { Translate } from "./ui-types.ts";
export type { Translate };

export type { ResultFilter } from "./results-model.ts";
export type RowAction = "copy" | "recheck" | "speed" | "pool" | "remove";

type Deps = {
  /** Re-render with the current results after a sort or filter change. */
  onChange: () => void;
  onAction: (action: RowAction, rows: ProxyResult[]) => void;
  canRecheck: () => boolean;
  /** A manual check is running: an empty table is waiting, not idle. */
  checking?: () => boolean;
};

const COLS = 14;
const ANON_VALUES = ["elite", "anonymous", "transparent", "unknown"];

const $ = <T extends HTMLElement>(id: string) => document.getElementById(id) as T | null;

export class ResultsView {
  hideCreds = false;
  sort: Sort = null;
  private rows: ProxyResult[] = [];
  private selected = new Set<string>();
  /** The proxy a Shift range starts from: an index goes stale on re-sort. */
  private anchor: string | null = null;
  /** The row the arrow keys move from: the last one clicked or reached. */
  private cursor: string | null = null;
  private virtual: VirtualRows<ProxyResult> | null = null;
  private t: Translate = (k) => k;
  locale = "ru";
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
    for (const id of ["f-proto", "f-anon", "f-country", "f-site", "f-tamper", "f-dnsbl", "f-dedupe"]) {
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
      } else if ((e.key === "ArrowDown" || e.key === "ArrowUp") && this.rows.length > 0) {
        // Arrows move the selection, Shift extends it: the table works
        // without a mouse.
        e.preventDefault();
        const at = this.cursorIndex();
        const step = e.key === "ArrowDown" ? 1 : -1;
        const i = at < 0 ? 0 : Math.max(0, Math.min(this.rows.length - 1, at + step));
        this.select(i, e.shiftKey, false);
        this.virtual?.reveal(i);
      } else if (e.key === "ContextMenu" || (e.shiftKey && e.key === "F10")) {
        // The keyboard's way to the row menu.
        e.preventDefault();
        this.openMenuForKeyboard(wrap);
      }
    });
    // The Menu key can also arrive as a contextmenu event on the table
    // itself (a row's own handler takes clicks on rows).
    wrap.addEventListener("contextmenu", (e) => {
      if (e.defaultPrevented || e.target !== wrap || this.rows.length === 0) return;
      e.preventDefault();
      this.openMenuForKeyboard(wrap);
    });

    const menu = $("row-menu");
    menu?.addEventListener("contextmenu", (e) => e.preventDefault());
    menu?.addEventListener("keydown", (e) => {
      if (!["ArrowDown", "ArrowUp", "Home", "End"].includes(e.key)) return;
      e.preventDefault();
      const items = [...menu.querySelectorAll<HTMLButtonElement>("button[data-act]:not(:disabled)")];
      if (items.length === 0) return;
      const at = items.indexOf(document.activeElement as HTMLButtonElement);
      const next = e.key === "Home" ? 0
        : e.key === "End" ? items.length - 1
        : (at + (e.key === "ArrowDown" ? 1 : items.length - 1)) % items.length;
      items[next].focus();
    });
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
    this.anchor = null;
    this.cursor = null;
    this.virtual?.set([], false);
  }

  clearSelection(): void {
    if (this.selected.size === 0) return;
    this.selected.clear();
    this.anchor = null;
    this.virtual?.repaint();
  }

  selectedRows(): ProxyResult[] {
    return this.rows.filter((r) => this.selected.has(r.proxy));
  }

  /** The rows the table shows for these results: filtered, deduplicated, sorted. */
  shown(results: ProxyResult[], status: ResultFilter): ProxyResult[] {
    return sortResults(filterResults(results, {
      status,
      query: $<HTMLInputElement>("search")?.value ?? "",
      proto: $<HTMLSelectElement>("f-proto")?.value ?? "",
      anon: $<HTMLSelectElement>("f-anon")?.value ?? "",
      country: $<HTMLSelectElement>("f-country")?.value ?? "",
      dedupe: ($<HTMLSelectElement>("f-dedupe")?.value ?? "") as Dedupe,
      site: $<HTMLSelectElement>("f-site")?.value ?? "",
      dnsbl: $<HTMLSelectElement>("f-dnsbl")?.value ?? "",
      tamper: $<HTMLSelectElement>("f-tamper")?.value ?? "",
    }), this.sort);
  }

  render(results: ProxyResult[], status: ResultFilter, t: Translate, visible: () => boolean): void {
    this.t = t;
    if (!visible()) return;
    this.updateOptions(results);
    this.renderSortState();
    this.shared = exitIpCounts(results);
    const body = $<HTMLTableSectionElement>("tbody");
    if (!body || !this.virtual) return;
    this.rows = this.shown(results, status);
    if (this.rows.length === 0) {
      this.virtual.set([]);
      const tr = document.createElement("tr");
      tr.className = "empty";
      const td = document.createElement("td");
      td.colSpan = COLS;
      td.textContent = t(
        results.length > 0 ? "empty_filter" : this.deps?.checking?.() ? "empty_running" : "empty_idle",
      );
      tr.appendChild(td);
      body.replaceChildren(tr);
      return;
    }
    this.virtual.set(this.rows);
  }

  private cursorIndex(): number {
    return this.cursor === null ? -1 : this.rows.findIndex((x) => x.proxy === this.cursor);
  }

  /** The row menu for the selection, under its current row. */
  private openMenuForKeyboard(wrap: HTMLElement): void {
    if (this.rows.length === 0) return;
    let i = this.cursorIndex();
    if (this.selected.size === 0 || i < 0) {
      i = Math.max(0, i);
      this.select(i, false, false);
      this.virtual?.reveal(i);
    }
    const tr = wrap.querySelector<HTMLElement>(`tr[data-index="${i}"]`);
    const box = (tr ?? wrap).getBoundingClientRect();
    this.openMenu(box.left + 24, tr ? box.bottom : box.top + 24);
  }

  private select(i: number, range: boolean, toggle: boolean): void {
    const r = this.rows[i];
    if (!r) return;
    this.cursor = r.proxy;
    const from = this.anchor === null ? -1 : this.rows.findIndex((x) => x.proxy === this.anchor);
    if (range && from >= 0) {
      const [a, b] = from < i ? [from, i] : [i, from];
      if (!toggle) this.selected.clear();
      for (let k = a; k <= b; k++) {
        const row = this.rows[k];
        if (row) this.selected.add(row.proxy);
      }
    } else if (toggle) {
      if (this.selected.has(r.proxy)) this.selected.delete(r.proxy);
      else this.selected.add(r.proxy);
      this.anchor = r.proxy;
    } else {
      this.selected = new Set([r.proxy]);
      this.anchor = r.proxy;
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
    label("speed", t("ctx_speed", { n: alive }), alive === 0 || !this.deps.canRecheck());
    label("pool", t("ctx_pool", { n: alive }), alive === 0);
    // A running check would put removed rows back with its next result.
    label("remove", t("ctx_remove", { n: rows.length }), !this.deps.canRecheck());
    menu.classList.remove("hidden");
    const w = menu.offsetWidth;
    const h = menu.offsetHeight;
    menu.style.left = `${Math.max(4, Math.min(x, window.innerWidth - w - 4))}px`;
    menu.style.top = `${Math.max(4, Math.min(y, window.innerHeight - h - 4))}px`;
    menu.querySelector<HTMLButtonElement>("button:not(:disabled)")?.focus();
  }

  private closeMenu(): void {
    const menu = $("row-menu");
    if (!menu || menu.classList.contains("hidden")) return;
    // Focus left inside a hidden menu would be lost: back to the table.
    const hadFocus = menu.contains(document.activeElement);
    menu.classList.add("hidden");
    if (hadFocus) $("results-wrap")?.focus({ preventScroll: true });
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
    // Filters for data this run has. Hidden otherwise, and reset when hidden:
    // a filter left on with no visible control would empty the table.
    const offer = (id: string, has: boolean) => {
      const sel = $<HTMLSelectElement>(id);
      if (!sel) return;
      sel.classList.toggle("hidden", !has);
      if (!has) sel.value = "";
    };
    offer("f-dnsbl", results.some((r) => r.dnsbl));
    offer("f-tamper", results.some((r) => r.tamper || r.tls === "modified"));
    const countries = opts.country.map(([c]) => c).sort((a, b) => a.localeCompare(b));
    const key = [t("f_proto_all"), opts.proto.join(","), countries.join(","), opts.site.join(",")].join("|");
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
    fill("f-site", t("f_site_all"), opts.site.map((h) => [h, t("f_site_opens", { site: h })]));
    $("f-site")?.classList.toggle("hidden", opts.site.length === 0);
  }

  private sitesCell(r: ProxyResult): string {
    const sum = siteSummary(r);
    if (!sum) {
      const skipped = r.alive && (r.skipped ?? []).includes("sites");
      return skipped
        ? `<td><span class="dim" title="${escapeHtml(this.t("skipped_hint"))}">${escapeHtml(this.t("skipped_short"))}</span></td>`
        : "<td>—</td>";
    }
    const title = (r.sites ?? [])
      .map((s) => `${s.ok ? "✓" : "✗"} ${siteHost(s.url)} ${s.ok ? `${s.ms ?? "?"} ms${s.status ? ` (${s.status})` : ""}` : s.error ?? ""}`)
      .join("\n");
    const cls = sum.ok === sum.total ? "ping-fast" : sum.ok === 0 ? "ping-slow" : "ping-mid";
    return `<td class="${cls}" title="${escapeHtml(title)}">${sum.ok}/${sum.total}</td>`;
  }

  private speedCell(r: ProxyResult): string {
    const s = r.speed;
    if (!s) return "<td>—</td>";
    const ms = (v: number | null) => (v == null ? "?" : `${v}`);
    const title = s.ok
      ? this.t("speed_t", { c: ms(s.connectMs), tls: ms(s.tlsMs), fb: ms(s.ttfbMs), mb: (s.bytes / 1e6).toFixed(1) })
      : s.error ?? "";
    return `<td class="${s.ok ? "" : "ping-slow"}" title="${escapeHtml(title)}">${escapeHtml(speedLabel(s))}</td>`;
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
      r.tamper === "modified" ? `<span class="anon-trans" title="${escapeHtml(t("tamper_modified_t"))}">?mod</span>` : "",
      r.tamper === "unverified" ? `<span class="anon-anon" title="${escapeHtml(t("tamper_unverified_t"))}">?unv</span>` : "",
      r.tls === "modified" ? `<span class="anon-trans" title="${escapeHtml(t("tls_modified_t"))}">?tls</span>` : "",
      r.dnsbl?.listed.length
        ? `<span class="anon-trans" title="${escapeHtml(t("dnsbl_listed_t", { lists: r.dnsbl.listed.join(", ") }))}">bl ${r.dnsbl.listed.length}</span>`
        : "",
    ].join(" ");
    // Several "different" proxies often leave through one exit IP.
    const same = r.ip ? this.shared.get(r.ip) ?? 0 : 0;
    const shared = same > 1 ? ` <span class="geo-tag" title="${escapeHtml(t("same_ip_t", { n: same }))}">×${same}</span>` : "";
    const info = r.alive ? `${escapeHtml(r.ip ?? "ok")}${shared} ${marks}` : escapeHtml(errorText(r.error ?? "fail", t));
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
      `<td>${pill}</td><td class="cell-err" title="${infoTitle}">${info}</td>` +
      `<td class="${pingClass(r.latency_ms)}">${ping}</td>` +
      `<td>${escapeHtml(stabilityShort(r))}</td><td class="${an.cls}">${cell("anon", an.txt)}</td>` +
      `<td class="cell-geo" title="${infoTitle}">${cell("geo", geoShort(r))}</td>` +
      `<td>${escapeHtml(r.ip_type ?? "-")}</td>` +
      `<td class="${tls.cls}" title="${infoTitle}">${cell("tls", tls.txt)}</td>` +
      this.sitesCell(r) +
      this.speedCell(r) +
      `<td class="${uptimeClass(r.history)}" title="${escapeHtml(historyTitle(r.history, t, this.locale))}">${uptimeLabel(r.history)}</td>`;
    return tr;
  }
}
