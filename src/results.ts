import type { ProxyResult } from "./types.ts";
import {
  anonShort,
  escapeHtml,
  geoShort,
  pingClass,
  rowTitle,
  stabilityShort,
  tlsShort,
} from "./format.ts";

export const RENDER_STEP = 250;
export const RENDER_MAX = 1000;

export type ResultFilter = "all" | "alive" | "dead";
export type Translate = (key: string, params?: Record<string, string | number>) => string;

export class ResultsView {
  limit = RENDER_STEP;

  reset(): void {
    this.limit = RENDER_STEP;
  }

  render(
    results: ProxyResult[],
    filter: ResultFilter,
    t: Translate,
    visible: () => boolean,
  ): void {
    if (!visible()) return;
    const searchEl = document.getElementById("search") as HTMLInputElement | null;
    const q = (searchEl?.value || "").toLowerCase().trim();
    const tb = document.getElementById("tbody") as HTMLTableSectionElement | null;
    if (!tb) return;
    tb.innerHTML = "";
    const rows = results.filter((r) => {
      if (filter === "alive" && !r.alive) return false;
      if (filter === "dead" && r.alive) return false;
      if (q) {
        const hay = [r.proxy, r.proto, r.error ?? "", r.ip ?? "", r.country ?? "", r.country_code ?? "", r.city ?? "", r.isp ?? "", r.org ?? "", r.asn ?? "", r.ip_type ?? "", r.anonymity ?? "", r.tls ?? ""].join(" ").toLowerCase();
        if (!hay.includes(q)) return false;
      }
      return true;
    });
    if (rows.length === 0) {
      const tr = document.createElement("tr");
      tr.className = "empty";
      const td = document.createElement("td");
      td.colSpan = 11;
      td.textContent = t(results.length === 0 ? "empty_idle" : "empty_filter");
      tr.appendChild(td);
      tb.appendChild(tr);
      return;
    }
    rows.slice(0, this.limit).forEach((r, i) => {
      const tr = document.createElement("tr");
      const pill = r.alive ? `<span class="pill alive">${t("pill_alive")}</span>` : `<span class="pill dead">${t("pill_dead")}</span>`;
      const ping = r.latency_ms == null ? "-" : `${r.latency_ms} ms`;
      const an = anonShort(r.anonymity);
      const tls = tlsShort(r.tls);
      const marks = [
        r.tamper === "modified" ? `<span class="anon-trans" title="tamper">?mod</span>` : "",
        r.tls === "modified" ? `<span class="anon-trans" title="tls">?tls</span>` : "",
      ].join(" ");
      const info = r.alive ? `${escapeHtml(r.ip ?? "ok")} ${marks}` : escapeHtml(r.error ?? "fail");
      const infoTitle = escapeHtml(rowTitle(r));
      tr.innerHTML =
        `<td>${i + 1}</td><td>${escapeHtml(r.proxy)}</td><td>${escapeHtml(r.proto)}</td>` +
        `<td>${pill}</td><td class="${pingClass(r.latency_ms)}">${ping}</td>` +
        `<td>${escapeHtml(stabilityShort(r))}</td><td class="${an.cls}">${escapeHtml(an.txt)}</td>` +
        `<td class="cell-geo" title="${infoTitle}">${escapeHtml(geoShort(r))}</td>` +
        `<td>${escapeHtml(r.ip_type ?? "-")}</td>` +
        `<td class="${tls.cls}" title="${infoTitle}">${escapeHtml(tls.txt)}</td>` +
        `<td class="cell-err" title="${infoTitle}">${info}</td>`;
      tb.appendChild(tr);
    });
    if (rows.length > this.limit) {
      const tr = document.createElement("tr");
      const td = document.createElement("td");
      td.colSpan = 11;
      td.style.textAlign = "center";
      if (this.limit < RENDER_MAX) {
        const b = document.createElement("button");
        b.className = "btn ghost";
        b.textContent = t("show_more", { n: rows.length - this.limit });
        b.addEventListener("click", () => {
          this.limit = Math.min(RENDER_MAX, this.limit + RENDER_STEP);
          this.render(results, filter, t, visible);
        });
        td.appendChild(b);
      } else {
        td.className = "empty";
        td.textContent = t("render_limit", { n: this.limit, total: rows.length });
      }
      tr.appendChild(td);
      tb.appendChild(tr);
    }
  }
}
