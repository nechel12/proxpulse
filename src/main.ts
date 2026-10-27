import { Channel, invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { clip } from "./clip.ts";
import { buildCsv, buildJson, buildTxt } from "./export.ts";
import { loadPingHist, pushPingPointStats, renderPingGraph, type PingPoint } from "./pinggraph.ts";
import { refreshStatusTexts, setStatusText } from "./status.ts";
import {
  anonShort,
  escapeHtml,
  geoShort,
  pingClass,
  poolCountry,
  rowTitle,
  stabilityShort,
  tlsShort,
} from "./format.ts";
import {
  type GeoEntry,
  type GeoFields,
  isFreshGeo,
  mergeGeoEntry,
} from "./geocache.ts";
import { type Lang, translate } from "./i18n.ts";
import { resolveTestUrl } from "./ipapi.ts";
import { resolveJudgeUrl } from "./judge.ts";
import { extractProxiesFromText, stripProxyCredentials } from "./parse.ts";
import {
  BUILTIN_PROFILES,
  type ProfileData,
  sanitizeProfile,
} from "./profiles.ts";
import type { ProxyResult } from "./types.ts";

// ================= i18n =================

let lang: Lang = "ru";
try {
  const savedLang = localStorage.getItem("pp-lang");
  if (savedLang === "ru" || savedLang === "en") lang = savedLang;
} catch { /* localStorage may be unavailable */ }

function t(key: string, params?: Record<string, string | number>): string {
  return translate(lang, key, params);
}

let lastStatus: { key: string; params?: Record<string, string | number> } = { key: "st_ready" };
const $ = <T extends HTMLElement = HTMLElement>(id: string) =>
  document.getElementById(id) as T;

let results: ProxyResult[] = [];
let filter: "all" | "alive" | "dead" = "all";
let renderLimit = 250;
const RENDER_STEP = 250;
const RENDER_MAX = 1000;
const POOL_RENDER_MAX = 1000;
const poolLimits = new Map<string, number>();
let stopFlag = false;
let running = false;
let starting = false;
let includeDead = false;
type PoolItem = { raw: string; url: string; latency: number | null; country: string | null };
let dispatchPool: PoolItem[] = [];
let pollTimer: number | undefined;
let autoRunning = false;
let autoCfgTimer: number | undefined;
const JUDGE_DEFAULT = "https://proxycheck.lmtunnel.com";
let judgeFull = true;
let dispAutoOn = false;
let autoOn = false;
let srvRunning = false;
let srvLastError: string | null = null;

// ---------- ui helpers ----------

function isJudgeMode(): boolean {
  return ($("test-url") as HTMLSelectElement).value === "judge";
}

function judgeBase(): string {
  const custom = ($("test-url-custom") as HTMLInputElement).value;
  return resolveJudgeUrl(custom, judgeFull, JUDGE_DEFAULT);
}

function effectiveTestUrl(): string {
  if (isJudgeMode()) return judgeBase();
  const sel = ($("test-url") as HTMLSelectElement).value.trim();
  const custom = ($("test-url-custom") as HTMLInputElement).value.trim();
  const pro = ($("chk-ipapi-pro") as HTMLInputElement).checked;
  const key = ($("ipapi-key") as HTMLInputElement).value;
  return resolveTestUrl(sel, custom, pro, key);
}

function activeIpapiKey(): string | null {
  const pro = ($("chk-ipapi-pro") as HTMLInputElement).checked;
  const key = ($("ipapi-key") as HTMLInputElement).value.trim();
  return pro && key ? key : null;
}

function updateIpapiUI(): void {
  const el = document.getElementById("ipapi-hint");
  if (!el) return;
  const pro = ($("chk-ipapi-pro") as HTMLInputElement).checked;
  const key = ($("ipapi-key") as HTMLInputElement).value.trim();
  el.textContent = t(pro && !key ? "ipapi_key_missing" : "ipapi_hint");
}

function loadIpapiSettings(): void {
  try {
    const pro = localStorage.getItem("pp-ipapi-pro") === "1";
    const key = localStorage.getItem("pp-ipapi-key") ?? "";
    ($("chk-ipapi-pro") as HTMLInputElement).checked = pro;
    ($("ipapi-key") as HTMLInputElement).value = key;
  } catch { /* localStorage may be unavailable */ }
  updateIpapiUI();
}

function saveIpapiSettings(): void {
  try {
    const pro = ($("chk-ipapi-pro") as HTMLInputElement).checked;
    const key = ($("ipapi-key") as HTMLInputElement).value;
    localStorage.setItem("pp-ipapi-pro", pro ? "1" : "0");
    localStorage.setItem("pp-ipapi-key", key);
  } catch { /* ignore quota/unavailable */ }
  updateIpapiUI();
  scheduleAutoConfig();
}

function updateJudgeUI(): void {
  const j = isJudgeMode();
  for (const id of ["deep-checks", "deep-hint", "precheck-row", "precheck-hint"]) {
    document.getElementById(id)?.classList.toggle("hidden", j);
  }
  document.getElementById("judge-mode-row")?.classList.toggle("hidden", !j);
  let insecure = false;
  if (j) {
    try {
      const u = new URL(judgeBase());
      const local = ["localhost", "127.0.0.1", "::1", "[::1]"].includes(u.hostname);
      insecure = u.protocol === "http:" && !local;
    } catch { /* invalid URL is reported by the backend */ }
  }
  document.getElementById("judge-http-hint")?.classList.toggle("hidden", !insecure);
  ($("test-url-custom") as HTMLInputElement).placeholder = t(j ? "custom_judge_ph" : "custom_ph");
}

function setJudgeFull(full: boolean): void {
  judgeFull = full;
  ($("judge-fast") as HTMLButtonElement).classList.toggle("active", !full);
  ($("judge-full") as HTMLButtonElement).classList.toggle("active", full);
  scheduleAutoConfig();
}

// ---------- settings profiles ----------

let customProfiles: { name: string; data: ProfileData }[] = [];
let currentProfileId = "";

function snapshotProfile(): ProfileData {
  const val = (id: string) => ($(id) as HTMLInputElement).value;
  const chk = (id: string) => ($(id) as HTMLInputElement).checked;
  return {
    testUrlSel: ($("test-url") as HTMLSelectElement).value,
    customUrl: val("test-url-custom"),
    timeout: val("timeout"), concurrency: val("concurrency"), repeats: val("repeats"),
    defaultProto: ($("default-proto") as HTMLSelectElement).value,
    geo: chk("chk-geo"), anon: chk("chk-anon"), tamper: chk("chk-tamper"), tls: chk("chk-tls"),
    precheck: chk("chk-precheck"), precheckTimeout: val("precheck-timeout"),
    judgeFull,
  };
}

function applyProfile(d: ProfileData): void {
  const setVal = (id: string, v: string) => { ($(id) as HTMLInputElement).value = v; };
  const setChk = (id: string, v: boolean) => { ($(id) as HTMLInputElement).checked = v; };
  ($("test-url") as HTMLSelectElement).value = d.testUrlSel;
  setVal("test-url-custom", d.customUrl);
  setVal("timeout", d.timeout);
  setVal("concurrency", d.concurrency);
  setVal("repeats", d.repeats);
  ($("default-proto") as HTMLSelectElement).value = d.defaultProto;
  setChk("chk-geo", d.geo);
  setChk("chk-anon", d.anon);
  setChk("chk-tamper", d.tamper);
  setChk("chk-tls", d.tls);
  setChk("chk-precheck", d.precheck);
  setVal("precheck-timeout", d.precheckTimeout);
  setJudgeFull(d.judgeFull);
  updateJudgeUI();
  scheduleAutoConfig();
}

function saveCurrentSettings(): void {
  try {
    localStorage.setItem("pp-settings", JSON.stringify(snapshotProfile()));
  } catch { /* ignore */ }
}

function loadCurrentSettings(): void {
  try {
    const raw = localStorage.getItem("pp-settings");
    if (!raw) return;
    const data = sanitizeProfile(JSON.parse(raw));
    if (data) applyProfile(data);
  } catch { /* ignore */ }
}

function loadProfiles(): void {
  customProfiles = [];
  try {
    const raw = localStorage.getItem("pp-profiles");
    const arr = raw ? JSON.parse(raw) : null;
    if (!Array.isArray(arr)) return;
    for (const p of arr) {
      if (!p || typeof p.name !== "string" || !p.name.trim()) continue;
      const data = sanitizeProfile(p.data);
      if (!data) continue;
      customProfiles.push({ name: p.name.trim().slice(0, 40), data });
      if (customProfiles.length >= 20) break;
    }
  } catch {
    customProfiles = [];
  }
}

function saveProfiles(): void {
  try {
    localStorage.setItem("pp-profiles", JSON.stringify(customProfiles.slice(0, 20)));
  } catch { /* ignore */ }
}

function renderProfileList(): void {
  const sel = $("profile-sel") as HTMLSelectElement;
  sel.innerHTML = "";
  const ph = document.createElement("option");
  ph.value = "";
  ph.textContent = t("profile_none");
  sel.appendChild(ph);
  for (const b of BUILTIN_PROFILES) {
    const o = document.createElement("option");
    o.value = b.id;
    o.textContent = t(b.nameKey);
    sel.appendChild(o);
  }
  for (const c of customProfiles) {
    const o = document.createElement("option");
    o.value = "custom:" + c.name;
    o.textContent = c.name;
    sel.appendChild(o);
  }
  sel.value = currentProfileId;
  ($("btn-profile-del") as HTMLButtonElement).disabled = !currentProfileId.startsWith("custom:");
}

function findProfile(id: string): ProfileData | null {
  const b = BUILTIN_PROFILES.find((x) => x.id === id);
  if (b) return b.data;
  if (id.startsWith("custom:")) {
    const c = customProfiles.find((x) => "custom:" + x.name === id);
    if (c) return c.data;
  }
  return null;
}

function setStatusT(key: string, params?: Record<string, string | number>) {
  lastStatus = { key, params };
  $("status-line").textContent = t(key, params);
}

let netKey = "net_idle";
let netParams: Record<string, string | number> | undefined;
let netDot = "idle";

function renderNet(): void {
  ($("net-dot") as HTMLElement).className = "dot " + netDot;
  ($("net-text") as HTMLElement).textContent = t(netKey, netParams);
}

function setNet(
  state: "idle" | "work" | "done",
  key: string,
  params?: Record<string, string | number>,
) {
  netDot = state;
  netKey = key;
  netParams = params;
  renderNet();
}

function askText(title: string, value = ""): Promise<string | null> {
  const overlay = $("text-modal");
  const input = $("text-modal-input") as HTMLInputElement;
  const ok = $("text-modal-ok") as HTMLButtonElement;
  const cancel = $("text-modal-cancel") as HTMLButtonElement;
  $("text-modal-title").textContent = title;
  input.value = value;
  overlay.classList.remove("hidden");
  input.focus();
  input.select();
  return new Promise((resolve) => {
    const done = (v: string | null) => {
      overlay.classList.add("hidden");
      ok.onclick = null;
      cancel.onclick = null;
      overlay.onclick = null;
      input.onkeydown = null;
      resolve(v);
    };
    ok.onclick = () => done(input.value);
    cancel.onclick = () => done(null);
    overlay.onclick = (e) => {
      if (e.target === overlay) done(null);
    };
    input.onkeydown = (e) => {
      if (e.key === "Enter") {
        e.preventDefault();
        done(input.value);
      } else if (e.key === "Escape") {
        e.preventDefault();
        done(null);
      }
    };
  });
}

function applyI18n() {
  document.documentElement.lang = lang;
  document.querySelectorAll("[data-i18n]").forEach((el) => {
    const k = (el as HTMLElement).dataset.i18n as string;
    (el as HTMLElement).textContent = t(k);
  });
  document.querySelectorAll("[data-i18n-ph]").forEach((el) => {
    const k = (el as HTMLElement).dataset.i18nPh as string;
    (el as HTMLInputElement).placeholder = t(k);
  });
  document.querySelectorAll("[data-i18n-title]").forEach((el) => {
    const k = (el as HTMLElement).dataset.i18nTitle as string;
    (el as HTMLElement).title = t(k);
  });
  document.querySelectorAll(".lang button").forEach((b) => {
    b.classList.toggle("active", (b as HTMLElement).dataset.lang === lang);
  });
  $("status-line").textContent = t(lastStatus.key, lastStatus.params);
  render();
  renderPool();
  renderProfileList();
  updateCounts();
  updateJudgeUI();
  updateIpapiUI();
  renderNet();
  refreshStatusTexts();
  renderSrvPill();
  void updateTray();
}

function updateCounts() {
  let alive = 0;
  let latSum = 0;
  let latN = 0;
  for (const r of results) {
    if (!r.alive) continue;
    alive += 1;
    if (r.latency_ms != null) {
      latSum += r.latency_ms;
      latN += 1;
    }
  }
  const dead = results.length - alive;
  const avg = latN > 0 ? Math.round(latSum / latN) : null;

  ($("st-total") as HTMLElement).textContent = String(results.length);
  ($("st-alive") as HTMLElement).textContent = String(alive);
  ($("st-dead") as HTMLElement).textContent = String(dead);
  ($("st-ping") as HTMLElement).textContent = avg == null ? "—" : `${avg} ms`;
  ($("f-all") as HTMLElement).textContent = String(results.length);
  ($("f-alive") as HTMLElement).textContent = String(alive);
  ($("f-dead") as HTMLElement).textContent = String(dead);
  ($("foot-info") as HTMLElement).textContent =
    results.length === 0
      ? ""
      : t("foot_alive", { a: alive, b: results.length });
  ($("pool-count") as HTMLElement).textContent = String(dispatchPool.length);
  const apc = document.getElementById("auto-pool-count");
  if (apc) apc.textContent = String(dispatchPool.length);
}

function screenVisible(id: string): boolean {
  const el = document.getElementById(id);
  return !!el && !el.classList.contains("hidden");
}

function render() {
  // The results table lives on the check tab only; skip expensive rebuilds
  // while another tab is visible (they are re-rendered on tab switch).
  if (!screenVisible("screen-check")) return;
  const searchEl = document.getElementById("search") as HTMLInputElement | null;
  const q = (searchEl?.value || "").toLowerCase().trim();
  const tb = $("tbody") as HTMLTableSectionElement;
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
  rows.slice(0, renderLimit).forEach((r, i) => {
    const tr = document.createElement("tr");
    const pill = r.alive ? `<span class="pill alive">${t("pill_alive")}</span>` : `<span class="pill dead">${t("pill_dead")}</span>`;
    const ping = r.latency_ms == null ? "—" : `${r.latency_ms} ms`;
    const an = anonShort(r.anonymity);
    const tls = tlsShort(r.tls);
    const marks = [
      r.tamper === "modified" ? `<span class="anon-trans" title="tamper">⚠mod</span>` : "",
      r.tls === "modified" ? `<span class="anon-trans" title="tls">⚠tls</span>` : "",
    ].join(" ");
    const info = r.alive ? `${escapeHtml(r.ip ?? "ok")} ${marks}` : escapeHtml(r.error ?? "fail");
    const infoTitle = escapeHtml(rowTitle(r));
    tr.innerHTML =
      `<td>${i + 1}</td><td>${escapeHtml(r.proxy)}</td><td>${escapeHtml(r.proto)}</td>` +
      `<td>${pill}</td><td class="${pingClass(r.latency_ms)}">${ping}</td>` +
      `<td>${escapeHtml(stabilityShort(r))}</td><td class="${an.cls}">${escapeHtml(an.txt)}</td>` +
      `<td class="cell-geo" title="${infoTitle}">${escapeHtml(geoShort(r))}</td>` +
      `<td>${escapeHtml(r.ip_type ?? "—")}</td>` +
      `<td class="${tls.cls}" title="${infoTitle}">${escapeHtml(tls.txt)}</td>` +
      `<td class="cell-err" title="${infoTitle}">${info}</td>`;
    tb.appendChild(tr);
  });
  if (rows.length > renderLimit) {
    const tr = document.createElement("tr");
    const td = document.createElement("td");
    td.colSpan = 11;
    td.style.textAlign = "center";
    if (renderLimit < RENDER_MAX) {
      const b = document.createElement("button");
      b.className = "btn ghost";
      b.textContent = t("show_more", { n: rows.length - renderLimit });
      b.addEventListener("click", () => {
        renderLimit = Math.min(RENDER_MAX, renderLimit + RENDER_STEP);
        render();
      });
      td.appendChild(b);
    } else {
      td.className = "empty";
      td.textContent = t("render_limit", { n: renderLimit, total: rows.length });
    }
    tr.appendChild(td);
    tb.appendChild(tr);
  }
}

function getPoolView(tbodyId: string) {
  const f = (document.querySelector(`[data-pool-filter="${tbodyId}"]`) as HTMLInputElement | null)?.value.toLowerCase().trim() ?? "";
  const s = (document.querySelector(`[data-pool-sort="${tbodyId}"]`) as HTMLSelectElement | null)?.value ?? "added";
  let rows = dispatchPool.map((p, i) => ({ ...p, i }));
  if (f) {
    rows = rows.filter((r) =>
      r.raw.toLowerCase().includes(f) || (r.country ?? "").toLowerCase().includes(f)
    );
  }
  if (s === "ping") rows.sort((a, b) => (a.latency ?? 1e12) - (b.latency ?? 1e12));
  else if (s === "country") rows.sort((a, b) => (a.country ?? "~~~").localeCompare(b.country ?? "~~~"));
  return rows;
}

function poolLimit(tbodyId: string): number {
  return Math.min(poolLimits.get(tbodyId) ?? RENDER_STEP, POOL_RENDER_MAX);
}

function renderPoolInto(tbodyId: string) {
  const pb = document.getElementById(tbodyId) as HTMLTableSectionElement | null;
  if (!pb) return;
  pb.innerHTML = "";
  const cols = tbodyId === "pool-body" ? 4 : 3;
  const all = getPoolView(tbodyId);
  if (all.length === 0) {
    const tr = document.createElement("tr");
    tr.className = "empty";
    const td = document.createElement("td");
    td.colSpan = cols;
    td.textContent = t(dispatchPool.length === 0 ? "pool_empty" : "empty_filter");
    tr.appendChild(td);
    pb.appendChild(tr);
    return;
  }
  const limit = poolLimit(tbodyId);
  all.slice(0, limit).forEach((p, n) => {
    const tr = document.createElement("tr");
    const ping = p.latency == null ? "—" : `${p.latency} ms`;
    const geo = p.country ? ` <span class="geo-tag">${escapeHtml(p.country)}</span>` : "";
    tr.innerHTML = `<td>${n + 1}</td><td>${escapeHtml(p.raw)}${geo}</td><td class="${pingClass(p.latency)}">${ping}</td>`;
    if (tbodyId === "pool-body") {
      const td = document.createElement("td");
      const b = document.createElement("button");
      b.className = "btn ghost";
      b.textContent = "✕";
      b.addEventListener("click", () => {
        const at = dispatchPool.findIndex((x) => x.raw === p.raw);
        if (at >= 0) dispatchPool.splice(at, 1);
        void syncPool(false);
        renderPool();
        updateCounts();
      });
      td.appendChild(b);
      tr.appendChild(td);
    }
    pb.appendChild(tr);
  });
  if (all.length > limit) {
    const tr = document.createElement("tr");
    const td = document.createElement("td");
    td.colSpan = cols;
    td.style.textAlign = "center";
    if (limit < POOL_RENDER_MAX) {
      const b = document.createElement("button");
      b.className = "btn ghost";
      b.textContent = t("show_more", { n: all.length - limit });
      b.addEventListener("click", () => {
        poolLimits.set(tbodyId, Math.min(POOL_RENDER_MAX, limit + RENDER_STEP));
        renderPoolInto(tbodyId);
      });
      td.appendChild(b);
    } else {
      td.className = "empty";
      td.textContent = t("render_limit", { n: limit, total: all.length });
    }
    tr.appendChild(td);
    pb.appendChild(tr);
  }
}

function renderPool() {
  // The pool exists on two tabs; only the visible table is rebuilt.
  if (screenVisible("screen-dispatch")) renderPoolInto("pool-body");
  if (screenVisible("screen-auto")) renderPoolInto("auto-pool-body");
}



/** merge: keep existing entries, add new ones, refresh latency.
 *  Entries over maxPingMs never enter the pool; re-checked entries
 *  that went over the limit are dropped. */
function mergePool(items: { raw: string; url?: string | null; latency: number | null; country?: string | null }[]) {
  const keyOf = (it: { raw: string; url?: string | null }) => it.url || it.raw;
  const idx = new Map(dispatchPool.map((p, i) => [keyOf(p), i]));
  const drop = new Set<number>();
  for (const it of items) {
    const key = keyOf(it);
    const at = idx.get(key);
    if (!passPing(it.latency)) {
      if (at != null) drop.add(at);
      continue;
    }
    if (at == null) {
      if (dispatchPool.length >= MAX_PROXIES) continue;
      idx.set(key, dispatchPool.length);
      dispatchPool.push({
        raw: it.raw,
        url: it.url || it.raw,
        latency: it.latency,
        country: it.country ?? null,
      });
    } else {
      if (it.url) dispatchPool[at].url = it.url;
      if (it.latency != null) dispatchPool[at].latency = it.latency;
      if (it.country != null) dispatchPool[at].country = it.country;
    }
  }
  if (drop.size > 0) {
    dispatchPool = dispatchPool.filter((_, i) => !drop.has(i));
  }
}

let maxPingMs = 0;

function passPing(latency: number | null): boolean {
  return !(maxPingMs > 0 && latency != null && latency > maxPingMs);
}

function loadMaxPing(): void {
  try {
    const v = Math.floor(Number(localStorage.getItem("pp-maxping")) || 0);
    maxPingMs = Number.isFinite(v) && v > 0 ? v : 0;
  } catch {
    maxPingMs = 0;
  }
}

function syncMaxPingInputs(): void {
  for (const id of ["disp-max-ping", "auto-max-ping"]) {
    const el = document.getElementById(id) as HTMLInputElement | null;
    if (el) el.value = String(maxPingMs);
  }
}

// ---------- geo cache (skip repeated geo lookups for unchanged exit IPs) ----------

const geoCache = new Map<string, GeoEntry>();
const GEO_CACHE_MAX = 2000;

function loadGeoCache(): void {
  try {
    const raw = localStorage.getItem("pp-geocache");
    if (!raw) return;
    const arr = JSON.parse(raw) as [string, Partial<GeoEntry>][];
    const str = (v: unknown) => (typeof v === "string" ? v : null);
    for (const [k, v] of arr.slice(-GEO_CACHE_MAX)) {
      if (typeof k !== "string" || !v || typeof v !== "object") continue;
      const g = v;
      geoCache.set(stripProxyCredentials(k), {
        ip: str(g.ip),
        country: str(g.country),
        country_code: str(g.country_code),
        city: str(g.city),
        isp: str(g.isp),
        org: str(g.org),
        asn: str(g.asn),
        ip_type: str(g.ip_type),
        // Entries written before timestamps existed are treated as stale.
        ts: typeof g.ts === "number" ? g.ts : 0,
      });
    }
  } catch { /* ignore */ }
}

function persistGeoCache(): void {
  try {
    localStorage.setItem("pp-geocache", JSON.stringify([...geoCache.entries()].slice(-GEO_CACHE_MAX)));
  } catch { /* ignore (quota) */ }
}

function learnGeo(list: ProxyResult[]): void {
  let changed = false;
  for (const r of list) {
    if (!r.alive || r.ip == null) continue;
    const key = stripProxyCredentials(r.proxy);
    const next: GeoFields = {
      ip: r.ip, country: r.country, country_code: r.country_code, city: r.city,
      isp: r.isp, org: r.org, asn: r.asn, ip_type: r.ip_type,
    };
    geoCache.set(key, mergeGeoEntry(geoCache.get(key), next));
    changed = true;
  }
  if (changed) {
    while (geoCache.size > GEO_CACHE_MAX) {
      const first = geoCache.keys().next();
      if (first.done) break;
      geoCache.delete(first.value);
    }
    persistGeoCache();
  }
}

function relevantGeoCache(list: string[]): (GeoEntry & { proxy: string })[] {
  const out: (GeoEntry & { proxy: string })[] = [];
  for (const p of list) {
    const key = stripProxyCredentials(p);
    const f = geoCache.get(key);
    if (f && isFreshGeo(f)) out.push({ proxy: key, ...f });
  }
  return out;
}

function readCheckSettings() {
  const judge = isJudgeMode();
  return {
    testUrl: effectiveTestUrl(),
    timeoutMs: Math.max(1000, Math.min(30000, Number(($("timeout") as HTMLInputElement).value) || 5000)),
    concurrency: Math.max(1, Math.min(500, Number(($("concurrency") as HTMLInputElement).value) || 50)),
    repeats: Math.max(1, Math.min(5, Number(($("repeats") as HTMLInputElement).value) || 1)),
    defaultProto: ($("default-proto") as HTMLSelectElement).value,
    withGeo: judge ? false : ($("chk-geo") as HTMLInputElement).checked,
    withAnonymity: judge ? false : ($("chk-anon") as HTMLInputElement).checked,
    withTamper: judge ? false : ($("chk-tamper") as HTMLInputElement).checked,
    withTls: judge ? false : ($("chk-tls") as HTMLInputElement).checked,
    precheck: ($("chk-precheck") as HTMLInputElement).checked,
    precheckTimeoutMs: Math.max(300, Math.min(10000, Number(($("precheck-timeout") as HTMLInputElement).value) || 1500)),
    judgeMode: judge,
    ipapiKey: activeIpapiKey(),
  };
}

const MAX_PROXIES = 20_000;
const MAX_INPUT_CHARS = 10 * 1024 * 1024;
const MAX_FILE_BYTES = 10 * 1024 * 1024;
const MAX_FILE_MB = 10;

type ParseResult = { list?: string[]; count: number; truncated: boolean };
type ParsePending = {
  resolve: (r: ParseResult) => void;
  reject: (e: unknown) => void;
  timer: number;
};

let parseWorker: Worker | null = null;
let parseSeq = 0;
const parsePending = new Map<number, ParsePending>();

function getParseWorker(): Worker | null {
  if (typeof Worker === "undefined") return null;
  if (parseWorker) return parseWorker;
  try {
    const w = new Worker(new URL("./parseWorker.ts", import.meta.url), {
      type: "module",
    });
    w.onmessage = (
      e: MessageEvent<{
        id: number;
        count: number;
        truncated: boolean;
        list?: string[];
      }>,
    ) => {
      const pending = parsePending.get(e.data.id);
      if (!pending) return;
      window.clearTimeout(pending.timer);
      parsePending.delete(e.data.id);
      pending.resolve({
        list: e.data.list,
        count: e.data.count,
        truncated: e.data.truncated,
      });
    };
    w.onerror = (e) => {
      for (const pending of parsePending.values()) {
        window.clearTimeout(pending.timer);
        pending.reject(e);
      }
      parsePending.clear();
      w.terminate();
      if (parseWorker === w) parseWorker = null;
    };
    parseWorker = w;
  } catch {
    parseWorker = null;
  }
  return parseWorker;
}

function parseFallback(
  text: string,
  limit: number,
  wantList: boolean,
): ParseResult {
  const list = extractProxiesFromText(text, limit);
  return {
    list: wantList ? list : undefined,
    count: list.length,
    truncated: list.length >= limit,
  };
}

function parseTextAsync(
  text: string,
  limit: number,
  wantList: boolean,
): Promise<ParseResult> {
  const w = getParseWorker();
  if (!w) return Promise.resolve(parseFallback(text, limit, wantList));
  return new Promise((resolve, reject) => {
    const id = ++parseSeq;
    const timer = window.setTimeout(() => {
      const pending = parsePending.get(id);
      if (!pending) return;
      parsePending.delete(id);
      pending.reject(new Error("parse timeout"));
    }, 20_000);
    parsePending.set(id, { resolve, reject, timer });
    try {
      w.postMessage({
        id,
        text,
        limit,
        mode: wantList ? "list" : "count",
      });
    } catch (e) {
      window.clearTimeout(timer);
      parsePending.delete(id);
      reject(e);
    }
  });
}

function isTauriRuntime(): boolean {
  return typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
}

async function runCheckList(list: string[], onProgress?: (done: number, total: number) => void, onLive?: (live: ProxyResult[]) => void): Promise<ProxyResult[]> {
  const s = readCheckSettings();
  if (!/^https?:\/\//i.test(s.testUrl)) throw new Error("badurl");
  const out: ProxyResult[] = [];
  const seen = new Set<string>();
  const pushLive = (r: ProxyResult) => {
    if (r.error === "cancelled" || seen.has(r.proxy)) return;
    seen.add(r.proxy);
    out.push(r);
  };
  let lastPaint = 0;
  const paint = (done: number, total: number) => {
    const now = performance.now();
    if (now - lastPaint < 300 && done < total) return;
    lastPaint = now;
    if (done >= total) {
      out.sort((a, b) => Number(b.alive) - Number(a.alive) || (a.latency_ms ?? 1e12) - (b.latency_ms ?? 1e12));
    }
    onLive?.([...out]);
    onProgress?.(done, total);
  };
  const channel = new Channel<ProxyResult>();
  let finishDone: () => void = () => {};
  const done = new Promise<void>((resolve) => {
    finishDone = resolve;
  });
  // The channel is the only result payload; a sentinel marks the end, and the
  // count check covers a dropped sentinel. Late messages are ignored so a
  // previous run cannot repaint a newer one.
  let finished = false;
  channel.onmessage = (r) => {
    if (finished) return;
    if (r.proto === "__done__") {
      finishDone?.();
      return;
    }
    pushLive(r);
    paint(out.length, list.length);
    if (out.length >= list.length) finishDone?.();
  };
  try {
    await invoke<void>("check_proxies", {
      proxies: list,
      testUrl: s.testUrl,
      timeoutMs: s.timeoutMs,
      concurrency: s.concurrency,
      defaultProto: s.defaultProto,
      repeats: s.repeats,
      withGeo: s.withGeo,
      withAnonymity: s.withAnonymity,
      withTamper: s.withTamper,
      withTls: s.withTls,
      precheck: s.precheck,
      precheckTimeoutMs: s.precheckTimeoutMs,
      judgeMode: s.judgeMode,
      ipapiKey: s.ipapiKey,
      geoCache: relevantGeoCache(list),
      onResult: channel,
    });
    await done;
  } catch (e) {
    const msg = String(e);
    if (msg.includes("another check is running")) throw e;
    for (const p of list) pushLive(deadStub(p, msg));
  } finally {
    finished = true;
    finishDone?.();
  }
  paint(out.length, list.length);
  out.sort((a, b) => Number(b.alive) - Number(a.alive) || (a.latency_ms ?? 1e12) - (b.latency_ms ?? 1e12));
  return out;
}

function setProgress(done: number, total: number) {
  const p = total === 0 ? 0 : Math.round((done / total) * 100);
  ($("progress") as HTMLElement).style.width = `${p}%`;
}

// ---------- alive ping history (sparkline) ----------

let pingHist: PingPoint[] = [];

function pushPingPoint(list: ProxyResult[] = results): void {
  const alive = list.filter((r) => r.alive);
  const lat = alive
    .map((r) => r.latency_ms)
    .filter((x): x is number => x != null);
  pingHist = pushPingPointStats(
    pingHist,
    lat.length ? Math.round(lat.reduce((a, b) => a + b, 0) / lat.length) : null,
    alive.length,
  );
  renderPingGraph(pingHist, lang);
}

function deadStub(proxy: string, msg: string): ProxyResult {
  return { proxy, url: proxy, proto: "ERR", alive: false, latency_ms: null, jitter_ms: null, success_rate: 0, attempts: 0, ip: null, country: null, country_code: null, city: null, isp: null, org: null, asn: null, ip_type: null, anonymity: null, tamper: null, tls: null, tls_info: null, error: clip(msg, 120) };
}

async function syncPool(notify: boolean): Promise<number> {
  try {
    const n = await invoke<number>("set_dispatch_pool", {
      items: dispatchPool.map((p) => ({
        raw: p.raw,
        url: p.url,
        latency: p.latency,
        country: p.country,
      })),
    });
    if (notify) setStatusT("pool_synced", { n });
    return n;
  } catch {
    if (notify) setStatusT("pool_sync_err");
    return -1;
  }
}

function armTimers() {
  // Timing lives in Rust: persist the UI state and hand it to the scheduler.
  saveAutoSettings();
  scheduleAutoConfig();
  void updateTray();
}

async function updateTray(): Promise<void> {
  try {
    const urls = getWebhookUrls();
    await invoke("update_tray_status", {
      webhookOn: autoOn && urls.length > 0,
      webhookUrl: urls[0] ?? "",
      lang,
    });
  } catch { /* tray may be unavailable */ }
}

// ---------- scheduled checks (Rust scheduler events) ----------

type WebhookResult = { index: number; ok: boolean; message: string };
type AutoStatusEvent = {
  schedule: "disp" | "auto";
  state: "running" | "done" | "stopped" | "skipped" | "empty" | "error";
  alive: number;
  total: number;
  avgMs: number | null;
  error: string | null;
  at: number;
};
type AutoPoolEvent = { items: PoolItem[] };
type AutoWebhookEvent = { results: WebhookResult[] };

function formatWebhookResults(results: WebhookResult[]): string {
  return results
    .map((r) => {
      const n = r.index < 100 ? r.index + 1 : "?";
      return `#${n} ${r.ok
        ? t("webhook_sent", { r: r.message })
        : t("webhook_fail", { e: r.message })}`;
    })
    .join(" | ");
}

function onAutoStatus(ev: AutoStatusEvent): void {
  const id = ev.schedule === "disp" ? "disp-auto-status" : "auto-status";
  const time = () =>
    new Date(ev.at * 1000).toLocaleTimeString(lang === "ru" ? "ru-RU" : "en-US");
  if (ev.state === "running") {
    setStatusText(id, () => t("auto_running", { n: ev.total }));
    autoRunning = true;
    ($("btn-start") as HTMLButtonElement).disabled = true;
    ($("btn-stop") as HTMLButtonElement).disabled = false;
    ($("btn-auto-now") as HTMLButtonElement).disabled = true;
    return;
  }
  if (ev.state === "skipped") {
    setStatusText(id, () => t("auto_skip"));
    return;
  }
  autoRunning = false;
  switch (ev.state) {
    case "done":
      setStatusText(id, () => `${t("auto_pruned", {
        alive: ev.alive,
        total: ev.total,
      })} ${time()}`);
      break;
    case "stopped":
      setStatusText(id, () => t("st_stopped", {
        alive: ev.alive,
        total: ev.total,
      }));
      break;
    case "empty":
      setStatusText(id, () => t("pool_empty"));
      break;
    case "error":
      setStatusText(id, () => t("auto_err", { e: ev.error ?? "" }));
      break;
  }
  if (ev.state === "done") {
    pingHist = pushPingPointStats(pingHist, ev.avgMs, ev.alive);
    renderPingGraph(pingHist, lang);
  }
  if (!running) {
    ($("btn-start") as HTMLButtonElement).disabled = false;
    ($("btn-stop") as HTMLButtonElement).disabled = true;
  }
  ($("btn-auto-now") as HTMLButtonElement).disabled = false;
}

function onAutoPool(ev: AutoPoolEvent): void {
  dispatchPool = ev.items.map((p) => ({
    raw: p.raw,
    url: p.url,
    latency: p.latency,
    country: p.country ?? null,
  }));
  renderPool();
  updateCounts();
}

function onAutoWebhook(ev: AutoWebhookEvent): void {
  setStatusText("webhook-status", () => formatWebhookResults(ev.results));
}

function buildAutoSettings() {
  const s = readCheckSettings();
  const minutes = (id: string, fallback: number) =>
    Math.max(
      1,
      Math.min(
        1440,
        Math.floor(Number(($(id) as HTMLInputElement).value) || fallback),
      ),
    );
  return {
    dispEnabled: dispAutoOn,
    dispIntervalMin: minutes("disp-auto-min", 30),
    autoEnabled: autoOn,
    autoIntervalMin: minutes("auto-min", 30),
    maxPingMs,
    webhookUrls: getWebhookUrls(),
    webhookFormat: ($("webhook-format") as HTMLSelectElement).value,
    webhookIncludeCredentials: webhookIncludeCreds,
    testUrl: s.testUrl,
    timeoutMs: s.timeoutMs,
    concurrency: s.concurrency,
    repeats: s.repeats,
    defaultProto: s.defaultProto,
    withGeo: s.withGeo,
    withAnonymity: s.withAnonymity,
    withTamper: s.withTamper,
    withTls: s.withTls,
    precheck: s.precheck,
    precheckTimeoutMs: s.precheckTimeoutMs,
    judgeMode: s.judgeMode,
    ipapiKey: s.ipapiKey,
  };
}

function scheduleAutoConfig(): void {
  if (!document.getElementById("timeout")) return;
  window.clearTimeout(autoCfgTimer);
  autoCfgTimer = window.setTimeout(() => {
    try {
      saveCurrentSettings();
      void invoke("set_auto_config", { settings: buildAutoSettings() }).catch(
        (e) => logErr("auto-config: " + String(e).slice(0, 200)),
      );
    } catch (e) {
      logErr("auto-config: " + String(e).slice(0, 200));
    }
  }, 200);
}

function saveAutoSettings(): void {
  try {
    localStorage.setItem("pp-auto-on", autoOn ? "1" : "0");
    localStorage.setItem("pp-disp-auto-on", dispAutoOn ? "1" : "0");
    localStorage.setItem(
      "pp-auto-min",
      String(Math.max(1, Math.min(1440, Number(($("auto-min") as HTMLInputElement).value) || 30))),
    );
    localStorage.setItem(
      "pp-disp-auto-min",
      String(Math.max(1, Math.min(1440, Number(($("disp-auto-min") as HTMLInputElement).value) || 30))),
    );
    localStorage.setItem(
      "pp-webhook-format",
      ($("webhook-format") as HTMLSelectElement).value,
    );
  } catch { /* ignore */ }
}

function loadAutoSettings(): void {
  try {
    autoOn = localStorage.getItem("pp-auto-on") === "1";
    dispAutoOn = localStorage.getItem("pp-disp-auto-on") === "1";
    const autoMin = Number(localStorage.getItem("pp-auto-min"));
    if (autoMin) {
      ($("auto-min") as HTMLInputElement).value = String(
        Math.max(1, Math.min(1440, Math.floor(autoMin))),
      );
    }
    const dispMin = Number(localStorage.getItem("pp-disp-auto-min"));
    if (dispMin) {
      ($("disp-auto-min") as HTMLInputElement).value = String(
        Math.max(1, Math.min(1440, Math.floor(dispMin))),
      );
    }
    const fmt = localStorage.getItem("pp-webhook-format");
    if (fmt === "txt" || fmt === "json" || fmt === "csv") {
      ($("webhook-format") as HTMLSelectElement).value = fmt;
    }
  } catch {
    autoOn = false;
    dispAutoOn = false;
  }
  const setPressed = (id: string, on: boolean) => {
    const el = document.getElementById(id) as HTMLButtonElement | null;
    if (el) el.setAttribute("aria-pressed", String(on));
  };
  setPressed("auto-on", autoOn);
  setPressed("disp-auto-on", dispAutoOn);
}

let webhookUrls: string[] = [""];
let webhookIncludeCreds = false;

function loadWebhookCreds(): void {
  try {
    webhookIncludeCreds = localStorage.getItem("pp-webhook-creds") === "1";
  } catch {
    webhookIncludeCreds = false;
  }
}

function saveWebhookCreds(): void {
  try {
    localStorage.setItem("pp-webhook-creds", webhookIncludeCreds ? "1" : "0");
  } catch { /* ignore */ }
}

function loadWebhooks(): void {
  try {
    const raw = localStorage.getItem("pp-webhooks");
    const arr = raw ? JSON.parse(raw) : null;
    webhookUrls = Array.isArray(arr)
      ? arr.filter((s): s is string => typeof s === "string").slice(0, 10)
      : [""];
  } catch {
    webhookUrls = [""];
  }
  if (webhookUrls.length === 0) webhookUrls = [""];
}

function saveWebhooks(): void {
  try {
    localStorage.setItem("pp-webhooks", JSON.stringify(webhookUrls.slice(0, 10)));
  } catch { /* ignore */ }
  scheduleAutoConfig();
}

function renderWebhookList(): void {
  const box = $("webhook-list");
  box.innerHTML = "";
  webhookUrls.forEach((u, i) => {
    const row = document.createElement("div");
    row.className = "hrow";
    const inp = document.createElement("input");
    inp.value = u;
    inp.placeholder = "https://...";
    inp.spellcheck = false;
    inp.addEventListener("input", () => {
      webhookUrls[i] = inp.value;
      saveWebhooks();
    });
    const del = document.createElement("button");
    del.className = "btn ghost";
    del.textContent = "✕";
    del.addEventListener("click", () => {
      webhookUrls.splice(i, 1);
      if (webhookUrls.length === 0) webhookUrls.push("");
      saveWebhooks();
      scheduleAutoConfig();
      renderWebhookList();
    });
    row.appendChild(inp);
    row.appendChild(del);
    box.appendChild(row);
  });
}

function getWebhookUrls(): string[] {
  return webhookUrls.map((s) => s.trim()).filter(Boolean).slice(0, 10);
}

let webhookSending = false;

async function sendWebhook(manual: boolean): Promise<void> {
  const urls = getWebhookUrls();
  if (urls.length === 0) {
    if (manual) setStatusText("webhook-status", () => t("webhook_hint"));
    return;
  }
  if (dispatchPool.length === 0) {
    setStatusText("webhook-status", () => t("webhook_empty"));
    return;
  }
  if (webhookSending) return;
  webhookSending = true;
  const format = ($("webhook-format") as HTMLSelectElement).value;
  const items = dispatchPool.map((p) => ({
    raw: p.raw,
    url: p.url,
    latency: p.latency,
    country: p.country,
  }));
  setStatusText("webhook-status", () => "...");
  try {
    const results = await invoke<WebhookResult[]>("send_webhooks", {
      urls,
      format,
      items,
      includeCredentials: webhookIncludeCreds,
    });
    setStatusText("webhook-status", () => formatWebhookResults(results));
  } catch (e) {
    const msg = String(e).slice(0, 120);
    setStatusText("webhook-status", () => t("webhook_fail", { e: msg }));
  } finally {
    webhookSending = false;
  }
}

async function start() {
  if (running || starting) return;
  starting = true;
  ($("btn-start") as HTMLButtonElement).disabled = true;
  try {
    await startInner();
  } finally {
    starting = false;
    if (!running) ($("btn-start") as HTMLButtonElement).disabled = false;
  }
}

async function startInner() {
  if (running) return;
  if (autoRunning) {
    setStatusT("auto_skip");
    return;
  }
  try {
    if (await invoke<boolean>("check_busy")) {
      setStatusT("auto_skip");
      return;
    }
  } catch { /* older backend */ }
  const source = ($("proxy-input") as HTMLTextAreaElement).value;
  if (source.length > MAX_INPUT_CHARS) {
    setStatusT("st_input_too_big", { mb: MAX_FILE_MB });
    return;
  }
  if (!/^https?:\/\//i.test(effectiveTestUrl())) {
    setStatusT("st_badurl");
    return;
  }
  setStatusT("st_parsing");
  let parsed: ParseResult;
  try {
    parsed = await parseTextAsync(source, MAX_PROXIES + 1, true);
  } catch {
    parsed = parseFallback(source, MAX_PROXIES + 1, true);
  }
  let list = parsed.list ?? [];
  if (list.length === 0) {
    setStatusT("st_empty");
    return;
  }
  const truncated = list.length > MAX_PROXIES;
  if (truncated) list = list.slice(0, MAX_PROXIES);

  running = true;
  stopFlag = false;
  results = [];
  renderLimit = RENDER_STEP;
  render();
  updateCounts();
  ($("btn-start") as HTMLButtonElement).disabled = true;
  ($("btn-stop") as HTMLButtonElement).disabled = false;
  setNet("work", "net_work", { s: 0 });
  setProgress(0, list.length);
  const t0 = performance.now();

  let busy = false;
  try {
    results = await runCheckList(list, (done, total) => {
      lastStatus = { key: "st_checking", params: { done, total } };
      $("status-line").textContent = t("st_checking", lastStatus.params);
      const secs = Math.floor((performance.now() - t0) / 1000);
      setNet("work", "net_work", { s: secs });
      render();
      updateCounts();
      setProgress(done, total);
    }, (live) => {
      results = live;
    });
  } catch (e) {
    if (String(e).includes("another check is running")) busy = true;
    /* otherwise keep partial results delivered through the channel */
  } finally {
    running = false;
    ($("btn-start") as HTMLButtonElement).disabled = false;
    ($("btn-stop") as HTMLButtonElement).disabled = !autoRunning;
  }
  if (busy) {
    setStatusT("auto_skip");
    setNet("idle", "net_idle");
    return;
  }
  learnGeo(results);
  render();
  updateCounts();
  setProgress(stopFlag ? results.length : list.length, list.length);

  const secs = ((performance.now() - t0) / 1000).toFixed(1);
  const alive = results.filter((r) => r.alive).length;
  if (stopFlag) setStatusT("st_stopped", { alive, total: results.length });
  else if (truncated) setStatusT("st_done_trunc", { secs, alive, total: results.length, n: MAX_PROXIES });
  else setStatusT("st_done", { secs, alive, total: results.length });
  setNet("done", stopFlag ? "net_stopped" : "net_done");

  // merge alive into dispatcher pool (existing entries kept)
  mergePool(results.filter((r) => r.alive).map((r) => ({ raw: r.proxy, url: r.url, latency: r.latency_ms, country: poolCountry(r) })));
  renderPool();
  updateCounts();
  await syncPool(false);
  pushPingPoint(results);
}

// ---------- export ----------

function exportScope(): ProxyResult[] {
  return includeDead ? results : results.filter((r) => r.alive);
}

function browserDownload(name: string, text: string, mime = "text/plain;charset=utf-8") {
  const blob = new Blob([text], { type: mime });
  const a = document.createElement("a");
  a.href = URL.createObjectURL(blob);
  a.download = name;
  document.body.appendChild(a);
  a.click();
  a.remove();
  setTimeout(() => URL.revokeObjectURL(a.href), 2000);
}

async function saveViaDialog(suggested: string, text: string, filters: { name: string; extensions: string[] }[]) {
  if (!isTauriRuntime()) {
    browserDownload(suggested, text);
    lastStatus = { key: "st_downloaded", params: { name: suggested } };
    $("status-line").textContent = t("st_downloaded", { name: suggested });
    return;
  }
  try {
    const path = await invoke<string | null>("export_text_file", {
      contents: text,
      defaultName: suggested,
      filterName: filters[0]?.name ?? "Text",
      extensions: filters[0]?.extensions ?? [],
    });
    if (!path) return;
    lastStatus = { key: "st_saved", params: { path } };
    $("status-line").textContent = t("st_saved", { path });
  } catch (e) {
    const msg = String(e).slice(0, 120);
    lastStatus = { key: "st_savefail", params: { e: msg } };
    $("status-line").textContent = t("st_savefail", { e: msg });
  }
}

async function doExport(kind: "txt" | "json" | "csv" | "copy") {
  const list = exportScope();
  if (kind === "copy") {
    const txt = buildTxt(list);
    if (!txt) { lastStatus = { key: "st_nocopy" }; $("status-line").textContent = t("st_nocopy"); return; }
    try {
      await navigator.clipboard.writeText(txt);
      lastStatus = { key: "st_copied", params: { n: txt.split("\n").length } };
      $("status-line").textContent = t("st_copied", { n: txt.split("\n").length });
    } catch {
      lastStatus = { key: "st_nocopy" };
      $("status-line").textContent = t("st_nocopy");
    }
    return;
  }
  if (list.length === 0) {
    lastStatus = { key: "st_nothing" };
    $("status-line").textContent = t("st_nothing");
    return;
  }
  if (kind === "txt") {
    await saveViaDialog("proxpulse-alive.txt", buildTxt(list), [{ name: "Text", extensions: ["txt"] }]);
  } else if (kind === "json") {
    await saveViaDialog("proxpulse.json", buildJson(list), [{ name: "JSON", extensions: ["json"] }]);
  } else {
    await saveViaDialog("proxpulse.csv", buildCsv(list), [{ name: "CSV", extensions: ["csv"] }]);
  }
}

// ---------- dispatcher ----------

type DispatchStatus = {
  running: boolean;
  port: number;
  mode: string;
  listener: string;
  upstreams: number;
  current: string | null;
  requests: number;
  errors: number;
  last_error: string | null;
};

function refreshSrvAddr() {
  const port = ($("srv-port") as HTMLInputElement).value || "1080";
  ($("srv-addr") as HTMLElement).textContent = `127.0.0.1:${port}`;
}

async function pollDispatch() {
  try {
    const s = await invoke<DispatchStatus>("local_proxy_status");
    ($("srv-req") as HTMLElement).textContent = String(s.requests);
    ($("srv-err") as HTMLElement).textContent = String(s.errors);
    ($("srv-cur") as HTMLElement).textContent = s.current ?? "—";
    ($("btn-srv-start") as HTMLButtonElement).disabled = s.running;
    ($("btn-srv-stop") as HTMLButtonElement).disabled = !s.running;

    const wasRunning = srvRunning;
    const previousError = srvLastError;
    srvRunning = s.running;
    srvLastError = s.last_error;
    renderSrvPill();

    if (s.running) {
      ($("srv-port") as HTMLInputElement).value = String(s.port);
      ($("srv-mode") as HTMLSelectElement).value = s.mode;
      ($("srv-listener") as HTMLSelectElement).value = s.listener;
      refreshSrvAddr();
    }
    const st = $("srv-status");
    if (st && (s.running !== wasRunning || (s.running && s.last_error !== previousError))) {
      if (s.running) {
        const error = s.last_error;
        const addr = `127.0.0.1:${s.port}`;
        setStatusText("srv-status", () => error ?? t("srv_on", { addr }));
      } else {
        setStatusText("srv-status", () => t("srv_off"));
      }
    } else if (st && st.textContent === "—" && !s.running) {
      setStatusText("srv-status", () => t("srv_off"));
    }
  } catch { /* ignore */ }
}

function renderSrvPill(): void {
  const pill = $("srv-pill");
  pill.textContent = srvRunning ? t("pill_on") : t("pill_off");
  pill.className = srvRunning ? "pill alive on" : "pill dead";
}

function setPolling(on: boolean) {
  window.clearInterval(pollTimer);
  if (on) {
    void pollDispatch();
    pollTimer = window.setInterval(() => void pollDispatch(), 2000);
  }
}

// ---------- diagnostics + updates ----------

type ErrItem = { t: string; msg: string };
let errRing: ErrItem[] = [];
let releaseUrl = "";

function renderErrList(): void {
  const el = document.getElementById("err-list");
  if (!el) return;
  el.textContent = errRing.length === 0
    ? "—"
    : errRing.slice(-10).reverse().map((e) => `${e.t} ${e.msg}`).join("\n");
}

function logErr(msg: string): void {
  errRing.push({ t: new Date().toISOString(), msg: clip(msg, 300) });
  if (errRing.length > 50) errRing.shift();
  try {
    localStorage.setItem("pp-errors", JSON.stringify(errRing.slice(-50)));
  } catch { /* ignore */ }
  renderErrList();
}

function loadErrRing(): void {
  try {
    const raw = localStorage.getItem("pp-errors");
    if (!raw) return;
    const arr = JSON.parse(raw) as ErrItem[];
    if (Array.isArray(arr)) {
      errRing = arr.filter((e) => e && typeof e.msg === "string").slice(-50);
    }
  } catch { /* ignore */ }
}

window.addEventListener("error", (e) => logErr("window: " + String(e.message || "error")));
window.addEventListener("unhandledrejection", (e) =>
  logErr("promise: " + clip(String((e as PromiseRejectionEvent).reason ?? "rejection"), 300))
);

window.addEventListener("DOMContentLoaded", () => {
  // lang
  document.querySelectorAll(".lang button").forEach((b) => {
    b.addEventListener("click", () => {
      lang = (b as HTMLElement).dataset.lang as Lang;
      try {
        localStorage.setItem("pp-lang", lang);
      } catch { /* ignore */ }
      applyI18n();
    });
  });

  // tabs
  document.querySelectorAll(".tab").forEach((tb) => {
    tb.addEventListener("click", () => {
      document.querySelectorAll(".tab").forEach((x) => x.classList.remove("active"));
      tb.classList.add("active");
      const name = (tb as HTMLElement).dataset.tab;
      document.querySelectorAll(".screen").forEach((s) => s.classList.add("hidden"));
      const scr = document.getElementById(`screen-${name}`);
      scr?.classList.remove("hidden");
      setPolling(name === "dispatch");
      if (name === "check") render();
      if (name === "dispatch") {
        refreshSrvAddr();
        renderPool();
        void pollDispatch();
      } else if (name === "auto") {
        renderPool();
      }
    });
  });

  const input = $("proxy-input") as HTMLTextAreaElement;
  let updGen = 0;
  const upd = () => {
    const text = input.value;
    if (text.length > MAX_INPUT_CHARS) {
      ($("src-count") as HTMLElement).textContent = "!";
      return;
    }
    const gen = ++updGen;
    void parseTextAsync(text, MAX_PROXIES + 1, false)
      .then((r) => {
        if (gen !== updGen) return;
        ($("src-count") as HTMLElement).textContent =
          String(Math.min(r.count, MAX_PROXIES)) +
          (r.count > MAX_PROXIES ? "+" : "");
      })
      .catch(() => {
        if (gen !== updGen) return;
        try {
          const list = extractProxiesFromText(text, MAX_PROXIES + 1);
          ($("src-count") as HTMLElement).textContent =
            String(Math.min(list.length, MAX_PROXIES)) +
            (list.length > MAX_PROXIES ? "+" : "");
        } catch {
          ($("src-count") as HTMLElement).textContent = "!";
        }
      });
  };
  let updT: number | undefined;
  input.addEventListener("input", () => { window.clearTimeout(updT); updT = window.setTimeout(upd, 250); });
  upd();

  $("btn-start").addEventListener("click", () => void start());
  $("btn-stop").addEventListener("click", () => {
    stopFlag = true;
    if (running) {
      lastStatus = { key: "st_stopping" };
      $("status-line").textContent = t("st_stopping");
    }
    void invoke("cancel_check").catch(() => { /* backend may already be idle */ });
  });
  $("search").addEventListener("input", () => { renderLimit = RENDER_STEP; render(); });
  document.querySelectorAll(".chip").forEach((c) => {
    c.addEventListener("click", () => {
      document.querySelectorAll(".chip").forEach((x) => x.classList.remove("active"));
      c.classList.add("active");
      filter = (c as HTMLElement).dataset.f as typeof filter;
      renderLimit = RENDER_STEP;
      render();
    });
  });

  $("btn-clear-in").addEventListener("click", () => { input.value = ""; upd(); });
  ($("btn-import-url") as HTMLButtonElement).addEventListener("click", async () => {
    const u = (($("import-url") as HTMLInputElement).value || "").trim();
    if (!u) return;
    $("status-line").textContent = "…";
    try {
      const txt = await invoke<string>("fetch_url_text", { url: u });
      let found: string[] = [];
      try {
        const parsed = await parseTextAsync(txt, MAX_PROXIES, true);
        found = parsed.list ?? [];
      } catch {
        found = extractProxiesFromText(txt, MAX_PROXIES);
      }
      if (found.length === 0) {
        lastStatus = { key: "file_none", params: { name: u.slice(0, 60) } };
        $("status-line").textContent = t("file_none", { name: u.slice(0, 60) });
        return;
      }
      input.value = (input.value ? input.value.replace(/\s+$/, "") + "\n" : "") + found.join("\n");
      upd();
      lastStatus = { key: "url_loaded", params: { n: found.length } };
      $("status-line").textContent = t("url_loaded", { n: found.length });
    } catch {
      lastStatus = { key: "url_fail" };
      $("status-line").textContent = t("url_fail");
    }
  });
  async function doPoolSync(statusElId: string): Promise<void> {
    mergePool(results.filter((r) => r.alive).map((r) => ({ raw: r.proxy, url: r.url, latency: r.latency_ms, country: poolCountry(r) })));
    renderPool();
    updateCounts();
    const n = await syncPool(false);
    if (n < 0) {
      setStatusText(statusElId, () => t("pool_sync_err"));
      lastStatus = { key: "pool_sync_err" };
      return;
    }
    setStatusText(statusElId, () => t("pool_synced", { n }));
    lastStatus = { key: "pool_synced", params: { n } };
  }

  function doPoolClear(): void {
    dispatchPool = [];
    // The backend stops a running dispatcher when the pool becomes empty;
    // refresh the status once the sync completes.
    void syncPool(false).then(() => {
      setStatusText("srv-status", () => t("srv_off"));
      void pollDispatch();
    });
    renderPool();
    updateCounts();
  }

  ($("btn-pool-clear") as HTMLButtonElement).addEventListener("click", doPoolClear);
  ($("btn-auto-pool-clear") as HTMLButtonElement).addEventListener("click", doPoolClear);
  $("file-input").addEventListener("change", async (e) => {
    const files = (e.target as HTMLInputElement).files;
    if (!files || files.length === 0) return;
    const parts: string[] = [];
    for (const f of Array.from(files)) {
      if (f.size > MAX_FILE_BYTES) {
        lastStatus = { key: "file_too_big", params: { name: f.name, mb: MAX_FILE_MB } };
        $("status-line").textContent = t("file_too_big", { name: f.name, mb: MAX_FILE_MB });
        continue;
      }
      try {
        const txt = await f.text();
        let found: string[] = [];
        try {
          const parsed = await parseTextAsync(txt, MAX_PROXIES, true);
          found = parsed.list ?? [];
        } catch {
          found = extractProxiesFromText(txt, MAX_PROXIES);
        }
        if (found.length > 0) parts.push(...found);
        else {
          lastStatus = { key: "file_none", params: { name: f.name } };
          $("status-line").textContent = t("file_none", { name: f.name });
        }
      } catch {
        lastStatus = { key: "file_fail", params: { name: f.name } };
        $("status-line").textContent = t("file_fail", { name: f.name });
      }
    }
    if (parts.length > 0) {
      input.value = (input.value ? input.value.replace(/\s+$/, "") + "\n" : "") + parts.join("\n");
      upd();
      lastStatus = { key: "file_loaded", params: { n: parts.length, f: files.length } };
      $("status-line").textContent = t("file_loaded", { n: parts.length, f: files.length });
    }
    (e.target as HTMLInputElement).value = "";
  });

  // export dropdown + dead toggle
  const menu = $("export-menu");
  $("btn-export").addEventListener("click", (e) => {
    e.stopPropagation();
    menu.classList.toggle("hidden");
  });
  document.addEventListener("click", (e) => {
    if (!menu.classList.contains("hidden") && !(e.target as HTMLElement).closest(".dropdown")) {
      menu.classList.add("hidden");
    }
  });
  menu.querySelectorAll("button").forEach((b) => {
    b.addEventListener("click", () => {
      menu.classList.add("hidden");
      void doExport((b as HTMLElement).dataset.exp as "txt" | "json" | "csv" | "copy");
    });
  });
  $("chk-dead").addEventListener("click", () => {
    includeDead = !includeDead;
    ($("chk-dead") as HTMLButtonElement).setAttribute("aria-pressed", String(includeDead));
  });

  $("btn-clear-out").addEventListener("click", () => {
    results = [];
    render(); updateCounts(); setProgress(0, 1);
    lastStatus = { key: "st_cleared" };
    $("status-line").textContent = t("st_cleared");
  });

  $("btn-direct").addEventListener("click", async () => {
    const url = effectiveTestUrl();
    setStatusText("direct-res", () => "...");
    try {
      const ms = await invoke<number>("check_direct", { testUrl: url, timeoutMs: 10000 });
      setStatusText("direct-res", () => t("direct_ok", { ms }));
    } catch (e) {
      const msg = String(e).slice(0, 80);
      setStatusText("direct-res", () => t("direct_fail", { e: msg }));
    }
  });

  // github cards
  const openGh = (url: string) => async () => {
    try {
      const { openUrl } = await import("@tauri-apps/plugin-opener");
      await openUrl(url);
    } catch {
      window.open(url, "_blank");
    }
  };
  $("gh-card").addEventListener("click", openGh("https://github.com/nechel12/proxpulse"));
  $("gh-card-judge").addEventListener("click", openGh("https://github.com/nechel12/proxpulse-judge"));

  // settings profiles
  loadProfiles();
  renderProfileList();
  ($("profile-sel") as HTMLSelectElement).addEventListener("change", (e) => {
    const id = (e.target as HTMLSelectElement).value;
    currentProfileId = id;
    const d = findProfile(id);
    if (d) applyProfile(d);
    ($("btn-profile-del") as HTMLButtonElement).disabled = !id.startsWith("custom:");
  });
  ($("btn-profile-save") as HTMLButtonElement).addEventListener("click", async () => {
    const res = await askText(t("profile_name"));
    if (res == null) return;
    const name = res.trim().slice(0, 40);
    if (!name) return;
    const data = snapshotProfile();
    const at = customProfiles.findIndex((c) => c.name === name);
    if (at >= 0) customProfiles[at] = { name, data };
    else if (customProfiles.length < 20) customProfiles.push({ name, data });
    else {
      setStatusT("profile_limit");
      return;
    }
    saveProfiles();
    currentProfileId = "custom:" + name;
    renderProfileList();
  });
  ($("btn-profile-del") as HTMLButtonElement).addEventListener("click", () => {
    if (!currentProfileId.startsWith("custom:")) return;
    customProfiles = customProfiles.filter((c) => "custom:" + c.name !== currentProfileId);
    saveProfiles();
    currentProfileId = "";
    renderProfileList();
  });
  ($("test-url") as HTMLSelectElement).addEventListener("change", updateJudgeUI);
  ($("test-url-custom") as HTMLInputElement).addEventListener("input", updateJudgeUI);
  ($("judge-fast") as HTMLButtonElement).addEventListener("click", () => setJudgeFull(false));
  ($("judge-full") as HTMLButtonElement).addEventListener("click", () => setJudgeFull(true));

  // ip-api Pro key. The key is stored locally and is visible to checked
  // proxies (the request must originate from the proxy); see README.
  ($("chk-ipapi-pro") as HTMLInputElement).addEventListener("change", saveIpapiSettings);
  ($("ipapi-key") as HTMLInputElement).addEventListener("input", saveIpapiSettings);
  $("btn-ipapi-pro").addEventListener("click", async () => {
    const url = "https://members.ip-api.com/";
    try {
      const { openUrl } = await import("@tauri-apps/plugin-opener");
      await openUrl(url);
    } catch {
      window.open(url, "_blank");
    }
  });

  // auto toggles (sqchk style)
  const bindSqchk = (id: string, get: () => boolean, set: (v: boolean) => void) => {
    $(id).addEventListener("click", () => {
      set(!get());
      ($(id) as HTMLButtonElement).setAttribute("aria-pressed", String(get()));
      armTimers();
    });
  };
  bindSqchk("disp-auto-on", () => dispAutoOn, (v) => { dispAutoOn = v; });
  bindSqchk("auto-on", () => autoOn, (v) => { autoOn = v; });

  // dispatcher
  ($("srv-port") as HTMLInputElement).addEventListener("input", refreshSrvAddr);
  refreshSrvAddr();
  ($("btn-pool-sync") as HTMLButtonElement).addEventListener("click", () => void doPoolSync("srv-status"));
  ($("btn-auto-pool-sync") as HTMLButtonElement).addEventListener("click", () => void doPoolSync("auto-status"));
  ($("btn-srv-copy") as HTMLButtonElement).addEventListener("click", async () => {
    try {
      await navigator.clipboard.writeText(($("srv-addr") as HTMLElement).textContent ?? "");
    } catch { /* ignore */ }
  });
  ($("btn-srv-start") as HTMLButtonElement).addEventListener("click", async () => {
    const port = Math.max(1, Math.min(65535, Number(($("srv-port") as HTMLInputElement).value) || 1080));
    const mode = ($("srv-mode") as HTMLSelectElement).value;
    const listener = ($("srv-listener") as HTMLSelectElement).value;
    setStatusText("srv-status", () => "...");
    try {
      if (dispatchPool.length === 0) {
        mergePool(results.filter((r) => r.alive).map((r) => ({ raw: r.proxy, url: r.url, latency: r.latency_ms, country: poolCountry(r) })));
        renderPool();
        updateCounts();
      }
      const synced = await syncPool(false);
      if (synced < 0) throw new Error(t("pool_sync_err"));
      const addr = await invoke<string>("start_local_proxy", { port, mode, listener });
      setStatusText("srv-status", () => t("srv_on", { addr }));
      try {
        const st = await invoke<DispatchStatus>("local_proxy_status");
        ($("srv-port") as HTMLInputElement).value = String(st.port);
      } catch { /* keep typed value */ }
      refreshSrvAddr();
      void pollDispatch();
      void updateTray();
    } catch (e) {
      const msg = String(e);
      if (msg.includes("pool is empty")) {
        setStatusText("srv-status", () => t("srv_err_empty"));
      } else {
        const short = msg.slice(0, 120);
        setStatusText("srv-status", () => t("srv_err", { e: short }));
      }
    }
  });
  ($("btn-srv-stop") as HTMLButtonElement).addEventListener("click", async () => {
    try {
      await invoke("stop_local_proxy");
    } catch { /* ignore */ }
    setStatusText("srv-status", () => t("srv_off"));
    void pollDispatch();
    void updateTray();
  });

  // tray
  $("btn-tray").addEventListener("click", async () => {
    try {
      await invoke("hide_to_tray");
    } catch { /* ignore */ }
  });

  // diagnostics + updates
  loadGeoCache();
  loadErrRing();
  renderErrList();
  void invoke<string>("app_version")
    .then((v) => {
      ($("app-ver") as HTMLElement).textContent = v;
      ($("brand-ver") as HTMLElement).textContent = `proxy checker // v${v}`;
      ($("brand-ver-info") as HTMLElement).textContent = `proxy checker // v${v}`;
    })
    .catch(() => { /* ignore */ });
  ($("btn-copy-diag") as HTMLButtonElement).addEventListener("click", async () => {
    const data = {
      app: ($("app-ver") as HTMLElement).textContent,
      lang,
      results: results.length,
      alive: results.filter((r) => r.alive).length,
      pool: dispatchPool.length,
      judge: isJudgeMode() ? effectiveTestUrl() : null,
      errors: errRing.slice(-20),
    };
    try {
      await navigator.clipboard.writeText(JSON.stringify(data, null, 2));
    } catch { /* ignore */ }
  });
  type UpdateInfo = {
    available: boolean;
    version: string | null;
    current: string;
    notes: string | null;
  };
  let pendingVersion = "";
  let installKindPromise: Promise<string> | null = null;
  const getInstallKind = (): Promise<string> => {
    if (!installKindPromise) {
      installKindPromise = isTauriRuntime()
        ? invoke<string>("install_kind").catch(() => "auto")
        : Promise.resolve("auto");
    }
    return installKindPromise;
  };
  ($("btn-check-upd") as HTMLButtonElement).addEventListener("click", async () => {
    const openBtn = $("btn-open-rel") as HTMLButtonElement;
    const instBtn = $("btn-install-upd") as HTMLButtonElement;
    openBtn.classList.add("hidden");
    instBtn.classList.add("hidden");
    pendingVersion = "";
    setStatusText("upd-status", () => "...");
    try {
      const u = await invoke<UpdateInfo>("check_app_update");
      ($("app-ver") as HTMLElement).textContent = u.current;
      if (!u.available || !u.version) {
        setStatusText("upd-status", () => t("upd_latest", { v: u.current }));
        return;
      }
      const version = u.version;
      pendingVersion = version;
      releaseUrl = "https://github.com/nechel12/proxpulse/releases";
      const kind = await getInstallKind();
      if (kind === "nsis" || kind === "auto") {
        setStatusText("upd-status", () => t("upd_found", { v: version, cur: u.current }));
        instBtn.classList.remove("hidden");
      } else {
        setStatusText("upd-status", () => t("upd_manual", { v: version }));
      }
      openBtn.classList.remove("hidden");
    } catch (e) {
      logErr("update-check: " + String(e).slice(0, 200));
      setStatusText("upd-status", () => t("upd_fail"));
    }
  });
  ($("btn-install-upd") as HTMLButtonElement).addEventListener("click", async () => {
    if (!pendingVersion) return;
    const btn = $("btn-install-upd") as HTMLButtonElement;
    if (btn.disabled) return;
    try {
      const { confirm } = await import("@tauri-apps/plugin-dialog");
      const ok = await confirm(t("upd_confirm", { v: pendingVersion }), { title: "ProxPulse", kind: "info" });
      if (!ok) return;
      btn.disabled = true;
      setStatusText("upd-status", () => t("upd_installing"));
      await invoke("install_app_update");
      setStatusText("upd-status", () => t("upd_restart"));
    } catch (e) {
      logErr("update-install: " + String(e).slice(0, 200));
      setStatusText("upd-status", () => t("upd_fail"));
    } finally {
      btn.disabled = false;
    }
  });
  ($("btn-open-rel") as HTMLButtonElement).addEventListener("click", async () => {
    if (!releaseUrl) return;
    try {
      const { openUrl } = await import("@tauri-apps/plugin-opener");
      await openUrl(releaseUrl);
    } catch {
      window.open(releaseUrl, "_blank");
    }
  });

  // auto timers + webhook
  for (const id of ["disp-auto-min", "auto-min"]) {
    document.getElementById(id)?.addEventListener("change", armTimers);
  }
  loadMaxPing();
  syncMaxPingInputs();
  document.querySelectorAll<HTMLInputElement>("[data-pool-filter]").forEach((el) => {
    el.addEventListener("input", () => {
      const id = el.dataset.poolFilter;
      if (id) poolLimits.set(id, RENDER_STEP);
      renderPool();
    });
  });
  document.querySelectorAll<HTMLSelectElement>("[data-pool-sort]").forEach((el) => {
    el.addEventListener("change", () => {
      const id = el.dataset.poolSort;
      if (id) poolLimits.set(id, RENDER_STEP);
      renderPool();
    });
  });
  pingHist = loadPingHist();
  renderPingGraph(pingHist, lang);
  for (const id of ["disp-max-ping", "auto-max-ping"]) {
    document.getElementById(id)?.addEventListener("change", (e) => {
      const v = Math.max(0, Math.floor(Number((e.target as HTMLInputElement).value) || 0));
      maxPingMs = Number.isFinite(v) ? v : 0;
      try {
        localStorage.setItem("pp-maxping", String(maxPingMs));
      } catch { /* ignore */ }
      syncMaxPingInputs();
      scheduleAutoConfig();
    });
  }
  loadWebhooks();
  loadWebhookCreds();
  loadIpapiSettings();
  loadAutoSettings();
  loadCurrentSettings();
  ($("chk-webhook-creds") as HTMLInputElement).checked = webhookIncludeCreds;
  ($("chk-webhook-creds") as HTMLInputElement).addEventListener("change", (e) => {
    webhookIncludeCreds = (e.target as HTMLInputElement).checked;
    saveWebhookCreds();
    scheduleAutoConfig();
  });
  renderWebhookList();
  ($("btn-webhook-add") as HTMLButtonElement).addEventListener("click", () => {
    if (webhookUrls.length >= 10) return;
    webhookUrls.push("");
    saveWebhooks();
    renderWebhookList();
    scheduleAutoConfig();
    const inputs = document.querySelectorAll<HTMLInputElement>("#webhook-list input");
    inputs[inputs.length - 1]?.focus();
  });
  ($("btn-auto-now") as HTMLButtonElement).addEventListener("click", () => {
    void invoke("run_auto_now", { schedule: "auto" }).catch((e) => {
      const msg = String(e).slice(0, 120);
      setStatusText("auto-status", () => t("auto_err", { e: msg }));
    });
  });
  ($("btn-webhook-send") as HTMLButtonElement).addEventListener("click", () => void sendWebhook(true));
  window.setInterval(() => void updateTray(), 5000);

  // Settings can be changed anywhere in the three panels; push a debounced
  // snapshot to the Rust scheduler.
  document.querySelectorAll(".settings").forEach((el) => {
    el.addEventListener("input", scheduleAutoConfig);
    el.addEventListener("change", scheduleAutoConfig);
  });
  ($("webhook-format") as HTMLSelectElement).addEventListener("change", () => {
    saveAutoSettings();
    scheduleAutoConfig();
  });
  void listen<AutoStatusEvent>("auto://status", (e) => onAutoStatus(e.payload)).catch(() => {});
  void listen<AutoPoolEvent>("auto://pool", (e) => onAutoPool(e.payload)).catch(() => {});
  void listen<AutoWebhookEvent>("auto://webhook", (e) => onAutoWebhook(e.payload)).catch(() => {});
  void invoke<boolean>("check_busy")
    .then((busy) => {
      if (!busy) return;
      autoRunning = true;
      ($("btn-start") as HTMLButtonElement).disabled = true;
      ($("btn-stop") as HTMLButtonElement).disabled = false;
      ($("btn-auto-now") as HTMLButtonElement).disabled = true;
    })
    .catch(() => {});

  // The renderer can be reloaded independently of the Rust process; restore
  // the authoritative pool and dispatcher state instead of showing empty.
  void invoke<PoolItem[]>("get_dispatch_pool")
    .then((items) => {
      dispatchPool = items.map((p) => ({
        raw: p.raw,
        url: p.url,
        latency: p.latency,
        country: p.country ?? null,
      }));
      renderPool();
      updateCounts();
    })
    .catch(() => { /* older backend */ });
  void pollDispatch();

  applyI18n();
  armTimers();
});
