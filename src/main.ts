import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { clip } from "./clip.ts";
import { ResultsView, type RowAction } from "./results.ts";
import { defaultOrder, mergeResults } from "./results-model.ts";
import { SitesPanel } from "./sites.ts";
import { fetchHistory } from "./history.ts";
import { formatMbps, loadSpeedMb, runSpeedTest, saveSpeedMb } from "./speed-client.ts";
import { PoolView, type PoolItem } from "./pool.ts";
import { exportList, type ExportKind } from "./export-panel.ts";
import { loadPingHist, pushPingPointStats, renderPingGraph, type PingPoint } from "./pinggraph.ts";
import { refreshStatusTexts, setStatusText } from "./status.ts";
import { formatDuration, poolCountry } from "./format.ts";
import { GeoCacheStore } from "./geo-store.ts";
import { WebhookPanel } from "./webhooks.ts";
import { type Lang, detectLang, translate } from "./i18n.ts";
import { parseTextAsync, type ParseResult } from "./parse-runner.ts";
import { ProfilePanel } from "./profiles-panel.ts";
import { type TargetMode, withOwnAddresses } from "./profiles.ts";
import { SettingsPanel } from "./settings-panel.ts";
import { initUpdater } from "./updater.ts";
import { errorLog, logErr } from "./diagnostics.ts";
import { DispatchMonitor, type DispatchStatus } from "./dispatch-monitor.ts";
import { PoolApiPanel } from "./pool-api.ts";
import { AutoSettingsStore, storedAutoConfig } from "./auto-settings.ts";
import { MAIN_POOL, MAX_POOLS, forgetPoolSettings, poolLabel, scopedKey, validPoolId, type PoolMeta } from "./pools.ts";
import { AutoStatusController, type AutoStatusEvent } from "./auto-status.ts";
import { runButtons } from "./run-buttons.ts";
import { initSourcePanel } from "./source-panel.ts";
import { initInfoPanel } from "./info-panel.ts";
import { AppSettings, readFlag, writeFlag } from "./app-settings.ts";
import type { DispatcherOptions, PrefsUpdate } from "./settings-io.ts";
import { applyTheme, loadThemePref } from "./theme.ts";
import { Notifier } from "./notify.ts";
import { formatWebhookResults } from "./webhooks.ts";
import { SubscriptionPanel, formatSubscriptionResults, type SubscriptionResult } from "./subscriptions.ts";
import { askConfirm, askText, flashLabel, screenVisible, setProgress } from "./ui.ts";
import { NetStatus } from "./net-status.ts";
import { CHECK_LIMITS, clampInt, runCheckList, type CheckSettings } from "./check-client.ts";
import { applyAutoPool, showAutoWebhook, type AutoPoolEvent, type AutoWebhookEvent } from "./auto-events.ts";
import type { ProxyResult } from "./types.ts";

// Apply the theme before the first paint of the app.
applyTheme(loadThemePref());

// ================= i18n =================

let savedLang: string | null = null;
try {
  savedLang = localStorage.getItem("pp-lang");
} catch { /* localStorage may be unavailable */ }
let lang: Lang = detectLang(
  savedLang,
  typeof navigator === "undefined" ? [] : (navigator.languages ?? [navigator.language]),
);

function t(key: string, params?: Record<string, string | number>): string {
  return translate(lang, key, params);
}

let lastStatus: { key: string; params?: Record<string, string | number> } = { key: "st_ready" };
const $ = <T extends HTMLElement = HTMLElement>(id: string) =>
  document.getElementById(id) as T;

let results: ProxyResult[] = [];
const resultsView = new ResultsView();
let filter: "all" | "alive" | "dead" = "all";
let stopFlag = false;
let running = false;
let starting = false;
let includeDead = false;
const pool = new PoolView();
const dispatchMonitor = new DispatchMonitor();
/** Named pools (`pools.rs`); the shown one fills the Dispatcher and Auto tabs and takes new alive proxies. */
let pools: PoolMeta[] = [{ id: MAIN_POOL, name: "" }];
let currentPool = loadCurrentPool();
/** Pool sizes after the last scheduled check, for the low-pool warning. */
const poolCounts = new Map<string, number>();
const poolApi = new PoolApiPanel({ t, setStatusText, pool: () => currentPool });

function loadCurrentPool(): string {
  try {
    const id = localStorage.getItem("pp-pool-current");
    if (id && validPoolId(id)) return id;
  } catch { /* ignore */ }
  return MAIN_POOL;
}
const profiles = new ProfilePanel();
const sitesPanel = new SitesPanel();
const settings = new SettingsPanel({ t, schedule: () => scheduleAutoConfig() });
const appSettings = new AppSettings({
  t,
  setStatusText,
  logErr,
  isTauriRuntime,
  pools: () => pools,
  isBusy: () => running || autoStatus.running,
  onHideCreds: () => {
    resultsView.hideCreds = appSettings.hideCreds;
    pool.hideCreds = appSettings.hideCreds;
    render();
    renderPool();
  },
});
const notifier = new Notifier({
  enabled: () => appSettings.notifications,
  isTauriRuntime,
  logErr,
});

function setStatusT(key: string, params?: Record<string, string | number>) {
  lastStatus = { key, params };
  $("status-line").textContent = t(key, params);
}

/** The last run verified answers with a judge key that came over plain HTTP. */
let keyOverHttp = false;

function renderKeyNote(): void {
  const note = $("status-note");
  note.hidden = !keyOverHttp;
  note.textContent = keyOverHttp ? t("st_key_http") : "";
}

const netStatus = new NetStatus();

function renderNet(): void {
  netStatus.render(t);
}

function setNet(
  state: "idle" | "work" | "done",
  key: string,
  params?: Record<string, string | number>,
) {
  netStatus.set(state, key, params);
  renderNet();
}

function applyI18n() {
  document.documentElement.lang = lang;
  resultsView.locale = lang;
  pool.locale = lang;
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
  document.querySelectorAll("[data-i18n-aria]").forEach((el) => {
    const k = (el as HTMLElement).dataset.i18nAria as string;
    el.setAttribute("aria-label", t(k));
  });
  document.querySelectorAll(".lang button").forEach((b) => {
    const active = (b as HTMLElement).dataset.lang === lang;
    b.classList.toggle("active", active);
    b.setAttribute("aria-pressed", String(active));
  });
  updateBrowserHint();
  $("status-line").textContent = t(lastStatus.key, lastStatus.params);
  renderKeyNote();
  render();
  renderPool();
  profiles.render(t);
  updateCounts();
  settings.updateTargetUI();
  renderNet();
  refreshStatusTexts();
  dispatchMonitor.renderPill(t);
  webhooks.renderFormat(t);
  renderPoolSwitchers();
  poolApi.render();
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
  ($("pool-count") as HTMLElement).textContent = String(pool.count);
  const apc = document.getElementById("auto-pool-count");
  if (apc) apc.textContent = String(pool.count);
}

function render() {
  resultsView.render(results, filter, t, () => screenVisible("screen-check"));
}

/** Attach the per-proxy history to the results and the pool. */
async function attachHistory(): Promise<void> {
  const map = await fetchHistory([...results.map((r) => r.url), ...pool.items.map((p) => p.url)]);
  if (Object.keys(map).length === 0) return;
  results = results.map((r) => (map[r.url] ? { ...r, history: map[r.url] } : r));
  for (const p of pool.items) p.history = map[p.url];
  render();
  renderPool();
  void refreshHistorySize();
}

async function refreshHistorySize(): Promise<void> {
  if (!isTauriRuntime()) return;
  try {
    const n = await invoke<number>("history_size");
    setStatusText("hist-size", () => t("hist_size", { n }));
  } catch { /* older backend */ }
}

function renderPool() {
  pool.render(t, (raw) => {
    pool.remove(raw);
    void syncPool(false);
    renderPool();
    updateCounts();
  }, screenVisible);
}

/** A check's alive rows into the shown pool. */
function mergeAlive(rows: ProxyResult[]): void {
  const items = rows
    .filter((r) => r.alive)
    .map((r) => ({ raw: r.proxy, url: r.url, latency: r.latency_ms, country: poolCountry(r) }));
  pool.merge(items, maxPingMs, MAX_PROXIES);
}

let maxPingMs = 0;

function loadMaxPing(): void {
  try {
    const v = Math.floor(Number(localStorage.getItem(scopedKey("pp-maxping", currentPool))) || 0);
    maxPingMs = Number.isFinite(v) && v > 0 ? v : 0;
  } catch {
    maxPingMs = 0;
  }
}

function syncMaxPingInputs(): void {
  const el = document.getElementById("auto-max-ping") as HTMLInputElement | null;
  if (el) el.value = String(maxPingMs);
}

// ---------- geo cache (skip repeated geo lookups for unchanged exit IPs) ----------

const geoCache = new GeoCacheStore();

/** The field's value as the check will use it, written back so it is seen. */
function numberIn(id: string, limits: readonly [number, number], fallback: number): number {
  const el = $(id) as HTMLInputElement;
  const value = clampInt(el.value, limits, fallback);
  if (el.value !== String(value)) el.value = String(value);
  return value;
}

function readCheckSettings(): CheckSettings {
  const judge = settings.isJudgeMode();
  return {
    testUrl: settings.effectiveTestUrl(),
    timeoutMs: numberIn("timeout", CHECK_LIMITS.timeoutMs, 5000),
    concurrency: numberIn("concurrency", CHECK_LIMITS.concurrency, 50),
    repeats: numberIn("repeats", CHECK_LIMITS.repeats, 1),
    defaultProto: ($("default-proto") as HTMLSelectElement).value,
    withGeo: judge ? false : ($("chk-geo") as HTMLInputElement).checked,
    withAnonymity: judge ? false : ($("chk-anon") as HTMLInputElement).checked,
    withTamper: judge ? false : ($("chk-tamper") as HTMLInputElement).checked,
    withTls: judge ? false : ($("chk-tls") as HTMLInputElement).checked,
    precheck: ($("chk-precheck") as HTMLInputElement).checked,
    precheckTimeoutMs: numberIn("precheck-timeout", CHECK_LIMITS.precheckTimeoutMs, 1500),
    judgeMode: judge,
    ipapiKey: settings.activeIpapiKey(),
    sites: sitesPanel.urls(),
    judgeHttpUrl: settings.judgeHttpUrl(),
    withDnsbl: ($("chk-dnsbl") as HTMLInputElement).checked,
  };
}

const MAX_PROXIES = 20_000;
const MAX_INPUT_CHARS = 10 * 1024 * 1024;
const MAX_FILE_BYTES = 10 * 1024 * 1024;
const MAX_FILE_MB = 10;

/** Parse with the current "type without scheme": it decides duplicates. */
function parseList(text: string, limit: number, wantList: boolean): Promise<ParseResult> {
  return parseTextAsync(text, limit, wantList, ($("default-proto") as HTMLSelectElement).value);
}

function isTauriRuntime(): boolean {
  return typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
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

/** False while a switched-to pool is loading: its list is empty until then. */
let poolLoaded = true;

async function syncPool(notify: boolean): Promise<number> {
  // Syncing the empty list of a pool still loading would wipe it.
  if (!poolLoaded) return -1;
  try {
    const n = await invoke<number>("set_dispatch_pool", {
      pool: currentPool,
      items: pool.items.map((p) => ({
        raw: p.raw,
        url: p.url,
        latency: p.latency,
        country: p.country,
      })),
    });
    if (notify) setStatusT("pool_synced", { n });
    return n;
  } catch (e) {
    logErr("pool-sync: " + String(e).slice(0, 200));
    if (notify) setStatusT("pool_sync_err_why", { e: String(e).slice(0, 120) });
    return -1;
  }
}

/** The tray shows the main pool's schedule. */
async function updateTray(): Promise<void> {
  try {
    let on: boolean;
    let urls: string[];
    if (currentPool === MAIN_POOL) {
      on = autoSettings.enabled && autoSettings.sendWebhooks;
      urls = webhooks.getUrls();
    } else {
      const main = storedAutoConfig(localStorage, MAIN_POOL, readCheckSettings());
      on = main.enabled && main.sendWebhooks;
      urls = main.webhookUrls;
    }
    await invoke("update_tray_status", {
      webhookOn: on && urls.length > 0,
      webhookUrl: urls[0] ?? "",
      lang,
    });
  } catch { /* tray may be unavailable */ }
}

// ---------- named pools ----------

function shownPool(id: string | undefined): boolean {
  return (id ?? MAIN_POOL) === currentPool;
}

function currentPoolMeta(): PoolMeta {
  return pools.find((p) => p.id === currentPool) ?? { id: currentPool, name: "" };
}

/** A notification title names the pool when there is more than one. */
function notifTitle(key: string, id: string): string {
  const meta = pools.find((p) => p.id === id);
  return pools.length > 1 && meta ? `${t(key)} · ${poolLabel(meta, t)}` : t(key);
}

/** Load the shown pool's proxies from the backend, which keeps them. */
async function loadPoolItems(): Promise<void> {
  const id = currentPool;
  try {
    const items = await invoke<PoolItem[]>("get_dispatch_pool", { pool: id });
    if (id !== currentPool) return;
    pool.set(items.map((p) => ({
      raw: p.raw,
      url: p.url,
      latency: p.latency,
      country: p.country ?? null,
    })));
    poolLoaded = true;
    poolCounts.set(id, pool.count);
    renderPool();
    updateCounts();
    void attachHistory();
  } catch { /* older backend */ }
}

function renderPoolSwitchers(): void {
  const label = poolLabel(currentPoolMeta(), t);
  document.querySelectorAll<HTMLSelectElement>(".pool-select").forEach((sel) => {
    sel.replaceChildren(...pools.map((p) => new Option(poolLabel(p, t), p.id)));
    sel.value = currentPool;
  });
  document.querySelectorAll<HTMLButtonElement>(".pool-new").forEach((b) => { b.disabled = pools.length >= MAX_POOLS; });
  document.querySelectorAll<HTMLButtonElement>(".pool-delete").forEach((b) => { b.disabled = currentPool === MAIN_POOL; });
  document.querySelectorAll<HTMLElement>(".pool-tag").forEach((el) => {
    el.textContent = pools.length > 1 ? ` · ${label}` : "";
  });
  const into = document.getElementById("check-pool-note");
  if (into) {
    into.hidden = pools.length < 2;
    into.textContent = pools.length > 1 ? t("check_into_pool", { pool: label }) : "";
  }
}

function showWebhookOptions(): void {
  ($("chk-webhook-creds") as HTMLInputElement).checked = webhooks.includeCreds;
  ($("chk-webhook-changes") as HTMLInputElement).checked = webhooks.changesOnly;
  $<HTMLTextAreaElement>("webhook-template").value = webhooks.template;
}

/** Show another pool in the Dispatcher and Auto tabs. */
async function switchPool(id: string): Promise<void> {
  if (id === currentPool || !pools.some((p) => p.id === id)) {
    renderPoolSwitchers();
    return;
  }
  saveAutoSettings();
  currentPool = id;
  try {
    localStorage.setItem("pp-pool-current", id);
  } catch { /* ignore */ }
  autoSettings.pool = id;
  webhooks.pool = id;
  subscriptions.pool = id;
  dispatchMonitor.pool = id;
  dispatchMonitor.reset();
  loadAutoSettings();
  loadMaxPing();
  syncMaxPingInputs();
  webhooks.load();
  showWebhookOptions();
  webhooks.render(t);
  webhooks.renderFormat(t);
  subscriptions.load();
  subscriptions.render(t);
  for (const el of ["auto-status", "disp-auto-status", "subs-status", "webhook-status", "srv-opts-status"]) {
    setStatusText(el, () => "—");
  }
  setStatusText("srv-status", () => "-");
  showDispatchOptions();
  renderPoolSwitchers();
  poolApi.render();
  poolLoaded = false;
  pool.set([]);
  renderPool();
  updateCounts();
  await loadPoolItems();
  void dispatchMonitor.poll(t, setStatusText);
  void updateTray();
}

/** Read the pool registry; a remembered pool that is gone falls back to the main one. */
async function loadPools(): Promise<void> {
  try {
    pools = await invoke<PoolMeta[]>("list_pools");
  } catch {
    return; // browser preview or older backend: only the main pool
  }
  if (!pools.some((p) => p.id === currentPool)) await switchPool(MAIN_POOL);
  renderPoolSwitchers();
  // Push every pool's schedule now that all are known.
  scheduleAutoConfig();
}

async function newPool(): Promise<void> {
  if (pools.length >= MAX_POOLS) return;
  const name = (await askText(t("pool_new_q"), "", 40))?.trim();
  if (!name) return;
  try {
    const meta = await invoke<PoolMeta>("create_pool", { name });
    pools = [...pools, meta];
    await switchPool(meta.id);
  } catch (e) {
    const msg = String(e).slice(0, 120);
    setStatusText("srv-status", () => t("srv_err", { e: msg }));
  }
}

async function renamePool(): Promise<void> {
  const meta = currentPoolMeta();
  const name = (await askText(t("pool_rename_q"), poolLabel(meta, t), 40))?.trim();
  if (name == null) return;
  try {
    const out = await invoke<PoolMeta>("rename_pool", { id: meta.id, name });
    pools = pools.map((p) => (p.id === out.id ? out : p));
    renderPoolSwitchers();
  } catch (e) {
    setStatusT("pool_rename_err", { e: String(e).slice(0, 120) });
  }
}

async function deletePool(): Promise<void> {
  const meta = currentPoolMeta();
  if (meta.id === MAIN_POOL) return;
  if (!(await askConfirm(t("pool_delete_confirm", { name: poolLabel(meta, t) })))) return;
  try {
    await invoke("delete_pool", { id: meta.id });
  } catch (e) {
    const msg = String(e).slice(0, 120);
    setStatusText("srv-status", () => t("srv_err", { e: msg }));
    return;
  }
  pools = pools.filter((p) => p.id !== meta.id);
  poolCounts.delete(meta.id);
  // Switching saves the shown pool's settings: forget them only afterwards.
  await switchPool(MAIN_POOL);
  try {
    forgetPoolSettings(localStorage, meta.id);
  } catch { /* ignore */ }
}

function initPoolSwitchers(): void {
  document.querySelectorAll<HTMLSelectElement>(".pool-select").forEach((sel) => {
    sel.addEventListener("change", () => void switchPool(sel.value));
  });
  document.querySelectorAll(".pool-new").forEach((b) => b.addEventListener("click", () => void newPool()));
  document.querySelectorAll(".pool-rename").forEach((b) => b.addEventListener("click", () => void renamePool()));
  document.querySelectorAll(".pool-delete").forEach((b) => b.addEventListener("click", () => void deletePool()));
  renderPoolSwitchers();
}

// ---------- scheduled checks (Rust scheduler events) ----------

const webhooks = new WebhookPanel(() => scheduleAutoConfig());
const subscriptions = new SubscriptionPanel(() => scheduleAutoConfig());
const autoSettings = new AutoSettingsStore({
  readCheckSettings,
  otherPools: () => pools.map((p) => p.id).filter((id) => id !== currentPool),
  maxPingMs: () => maxPingMs,
  webhooks,
  subscriptions,
  saveCurrentSettings: () => settings.saveCurrentSettings(),
  updateTray,
});
// The remembered pool is shown from the first load of its settings.
autoSettings.pool = currentPool;
webhooks.pool = currentPool;
subscriptions.pool = currentPool;
dispatchMonitor.pool = currentPool;

const autoStatus = new AutoStatusController({
  t,
  setStatusText,
  onChange: () => syncRunButtons(),
  poolName: (id) => poolLabel(pools.find((p) => p.id === id) ?? { id, name: "" }, t),
  onDone: (avg, alive) => {
    pingHist = pushPingPointStats(pingHist, avg, alive);
    renderPingGraph(pingHist, lang);
  },
  locale: () => lang,
});

function scheduleAutoConfig(): void { autoSettings.schedule(); }
function saveAutoSettings(): void { autoSettings.save(); }
function loadAutoSettings(): void { autoSettings.load(); }
function armTimers(): void { autoSettings.arm(); }

async function sendWebhook(manual: boolean): Promise<void> {
  await webhooks.send(manual, pool.items, t, setStatusText);
}

/** Run `body` as the one manual check; Start stays disabled meanwhile. */
async function withStart(body: () => Promise<void>): Promise<void> {
  if (running || starting) return;
  starting = true;
  syncRunButtons();
  try {
    await body();
  } finally {
    starting = false;
    syncRunButtons();
  }
}

/** Start, Stop and "Check now" follow the run state; nothing else sets them. */
function syncRunButtons(): void {
  const on = runButtons({
    manual: running,
    starting,
    autoChecking: autoStatus.checking,
    autoRunning: autoStatus.running,
  });
  ($("btn-start") as HTMLButtonElement).disabled = !on.start;
  ($("btn-stop") as HTMLButtonElement).disabled = !on.stop;
  ($("btn-auto-now") as HTMLButtonElement).disabled = !on.autoNow;
}

async function start() {
  await withStart(startInner);
}

/** Re-check some entries of the current results and update them in place. */
async function recheck(list: string[]) {
  if (list.length === 0) return;
  await withStart(async () => {
    if (!(await checkerFree())) return;
    if (!/^https?:\/\//i.test(settings.effectiveTestUrl())) {
      setStatusT(settings.effectiveTestUrl() ? "st_badurl" : "st_nourl");
      return;
    }
    await runList(list, { recheck: true, truncated: false });
  });
}

/** False (with a status) while a scheduled check holds the checker. */
async function checkerFree(): Promise<boolean> {
  if (running) return false;
  if (autoStatus.running) {
    setStatusT("auto_skip");
    return false;
  }
  try {
    if (await invoke<boolean>("check_busy")) {
      setStatusT("auto_skip");
      return false;
    }
  } catch { /* older backend */ }
  return true;
}

async function startInner() {
  if (!(await checkerFree())) return;
  const source = ($("proxy-input") as HTMLTextAreaElement).value;
  if (source.length > MAX_INPUT_CHARS) {
    setStatusT("st_input_too_big", { mb: MAX_FILE_MB });
    return;
  }
  if (!/^https?:\/\//i.test(settings.effectiveTestUrl())) {
    setStatusT(settings.effectiveTestUrl() ? "st_badurl" : "st_nourl");
    return;
  }
  setStatusT("st_parsing");
  let parsed: ParseResult;
  try {
    parsed = await parseList(source, MAX_PROXIES + 1, true);
  } catch (e) {
    setStatusT("st_parse_fail", { e: String(e).slice(0, 120) });
    return;
  }
  let list = parsed.list ?? [];
  if (list.length === 0) {
    setStatusT("st_empty");
    return;
  }
  const truncated = list.length > MAX_PROXIES;
  if (truncated) list = list.slice(0, MAX_PROXIES);
  await runList(list, { recheck: false, truncated });
}

/**
 * Merge alive proxies into a pool that is not shown any more: the run
 * started for it, and the pool picked meanwhile must not get its results.
 */
async function mergeIntoPool(id: string, alive: ProxyResult[]): Promise<void> {
  try {
    const items = await invoke<PoolItem[]>("get_dispatch_pool", { pool: id });
    const target = new PoolView();
    target.set(items.map((p) => ({ raw: p.raw, url: p.url, latency: p.latency, country: p.country ?? null })));
    let maxPing = 0;
    try {
      maxPing = Math.max(0, Math.floor(Number(localStorage.getItem(scopedKey("pp-maxping", id))) || 0));
    } catch { /* ignore */ }
    target.merge(alive.map((r) => ({ raw: r.proxy, url: r.url, latency: r.latency_ms, country: poolCountry(r) })), maxPing, MAX_PROXIES);
    const n = await invoke<number>("set_dispatch_pool", { pool: id, items: target.items });
    poolCounts.set(id, n);
  } catch { /* the pool was deleted meanwhile */ }
}

async function runList(list: string[], opts: { recheck: boolean; truncated: boolean }) {
  running = true;
  // The pool shown when the run starts takes its alive proxies, even if
  // another one is picked before it ends.
  const targetPool = currentPool;
  stopFlag = false;
  keyOverHttp = false;
  renderKeyNote();
  // A re-check keeps the other entries and replaces the re-checked ones.
  const base = opts.recheck ? results : null;
  if (!base) {
    results = [];
    resultsView.reset();
  }
  render();
  updateCounts();
  syncRunButtons();
  setNet("work", "net_work", { s: 0 });
  setProgress(0, list.length);
  // Not "parsing" until the first result: the list is parsed already.
  setStatusT("st_checking", { done: 0, total: list.length });
  const t0 = performance.now();
  // The elapsed time ticks on its own, also while no result arrives.
  const ticker = window.setInterval(() => {
    setNet("work", "net_work", { s: Math.floor((performance.now() - t0) / 1000) });
  }, 1000);

  let busy = false;
  let unreachable = false;
  try {
    const fresh = await runCheckList(list, readCheckSettings(), geoCache.fresh(), (done, total) => {
      const secs = (performance.now() - t0) / 1000;
      // Left at the pace so far, once there is a pace to go by.
      const left = done >= 5 && done < total && secs > 2 ? Math.ceil(((total - done) * secs) / done) : null;
      lastStatus = left == null
        ? { key: "st_checking", params: { done, total } }
        : { key: "st_checking_eta", params: { done, total, left: formatDuration(left) } };
      $("status-line").textContent = t(lastStatus.key, lastStatus.params);
      setNet("work", "net_work", { s: Math.floor(secs) });
      render();
      updateCounts();
      setProgress(done, total);
    }, (live) => {
      results = base ? mergeResults(base, live) : live;
    }, () => {
      keyOverHttp = true;
      renderKeyNote();
    }, () => {
      unreachable = true;
    });
    results = base ? defaultOrder(mergeResults(base, fresh)) : fresh;
    // A re-check that reached nobody says nothing about the proxies: rows
    // that were alive stay as they were.
    if (unreachable && base) results = base;
  } catch (e) {
    if (String(e).includes("another check is running")) busy = true;
    /* otherwise keep partial results delivered through the channel */
  } finally {
    running = false;
    syncRunButtons();
    window.clearInterval(ticker);
  }

  if (busy) {
    if (base) results = base;
    setStatusT("auto_skip");
    setNet("idle", "net_idle");
    render();
    return;
  }
  const listed = new Set(list);
  const checked = base ? results.filter((r) => listed.has(r.proxy)) : results;
  geoCache.learn(checked);
  render();
  updateCounts();
  setProgress(stopFlag ? checked.length : list.length, list.length);

  const secs = ((performance.now() - t0) / 1000).toFixed(1);
  const alive = checked.filter((r) => r.alive).length;
  if (stopFlag) setStatusT("st_stopped", { alive, total: checked.length });
  else if (unreachable) setStatusT("st_unreachable", { total: checked.length });
  else if (base) setStatusT("st_rechecked", { secs, alive, total: checked.length });
  else if (opts.truncated) setStatusT("st_done_trunc", { secs, alive, total: checked.length, n: MAX_PROXIES });
  else setStatusT("st_done", { secs, alive, total: checked.length });
  setNet("done", stopFlag ? "net_stopped" : "net_done");

  // merge alive into dispatcher pool (existing entries kept)
  if (currentPool === targetPool) {
    mergeAlive(checked);
    renderPool();
    updateCounts();
    await syncPool(false);
  } else {
    await mergeIntoPool(targetPool, checked.filter((r) => r.alive));
  }
  // A re-check of a few entries would distort the per-run ping history.
  if (!base) pushPingPoint(results);
  void attachHistory();
}

/** Download speed through the given alive proxies, filled in their rows. */
async function speedTest(rows: ProxyResult[]): Promise<void> {
  if (rows.length === 0) return;
  await withStart(async () => {
    if (!(await checkerFree())) return;
    running = true;
    stopFlag = false;
    syncRunButtons();
    setNet("work", "net_work", { s: 0 });
    const total = rows.length;
    let done = 0;
    let best = 0;
    let ok = 0;
    let failed: string | null = null;
    setStatusT("speed_running", { done, total });
    setProgress(0, total);
    try {
      await runSpeedTest(rows.map((r) => ({ proxy: r.proxy, url: r.url })), loadSpeedMb(), (s) => {
        done += 1;
        if (s.ok && s.mbps != null) {
          ok += 1;
          best = Math.max(best, s.mbps);
        }
        results = results.map((r) => (r.proxy === s.proxy ? { ...r, speed: s } : r));
        setStatusT("speed_running", { done, total });
        setProgress(done, total);
        render();
      });
    } catch (e) {
      failed = String(e);
      logErr("speed: " + failed.slice(0, 200));
    } finally {
      running = false;
      syncRunButtons();
    }
    if (failed?.includes("another check is running")) setStatusT("auto_skip");
    else if (failed) setStatusT("speed_err", { e: failed.slice(0, 120) });
    else if (stopFlag) setStatusT("speed_stopped", { ok, done, total });
    else if (ok === 0) setStatusT("speed_none");
    else setStatusT("speed_done", { ok, total, best: formatMbps(best) });
    setNet("done", stopFlag ? "net_stopped" : "net_done");
  });
}

/** Actions from the results context menu and keyboard. */
async function rowAction(action: RowAction, rows: ProxyResult[]): Promise<void> {
  if (rows.length === 0) return;
  const keys = new Set(rows.map((r) => r.proxy));
  if (action === "copy") {
    try {
      // Copies keep credentials, like exports.
      // The normalized URL, as Export → Copy gives it.
      await navigator.clipboard.writeText(rows.map((r) => r.url || r.proxy).join("\n"));
      setStatusT("st_copied", { n: rows.length });
    } catch { /* clipboard unavailable */ }
  } else if (action === "recheck") {
    await recheck(rows.map((r) => r.proxy));
  } else if (action === "speed") {
    await speedTest(rows.filter((r) => r.alive));
  } else if (action === "pool") {
    const alive = rows.filter((r) => r.alive);
    const before = pool.count;
    mergeAlive(alive);
    renderPool();
    updateCounts();
    const n = await syncPool(false);
    // What entered the pool: Max ping keeps slow ones out, known ones are
    // only updated.
    setStatusT(n < 0 ? "pool_sync_err" : "pool_added", { n: Math.max(0, pool.count - before) });
  } else if (action === "remove") {
    if (running) return;
    results = results.filter((r) => !keys.has(r.proxy));
    resultsView.clearSelection();
    render();
    updateCounts();
    setStatusT("st_removed", { n: keys.size });
  }
}

// ---------- export ----------

/** What the table shows, as the user sees it: filters, grouping and order. */
function exportScope(): { list: ProxyResult[]; total: number } {
  const keep = (r: ProxyResult) => includeDead || r.alive;
  return {
    list: resultsView.shown(results, filter).filter(keep),
    total: results.filter(keep).length,
  };
}

async function doExport(kind: ExportKind): Promise<void> {
  const { list, total } = exportScope();
  const filtered = list.length < total ? { shown: list.length, total } : undefined;
  await exportList(kind, list, { isTauriRuntime, notify: setStatusT, includeDead, askTemplate, filtered });
}

/** The line template for "Export by template", remembered between uses. */
async function askTemplate(): Promise<string | null> {
  let saved = "{scheme}://{auth}{host}:{port}";
  try {
    saved = localStorage.getItem("pp-export-template") ?? saved;
  } catch { /* ignore */ }
  const value = (await askText(t("exp_template_q"), saved, 300))?.trim();
  if (!value) return null;
  try {
    localStorage.setItem("pp-export-template", value);
  } catch { /* ignore */ }
  return value;
}

/** Show the dispatcher filter, sessions and login of the shown pool. */
function showDispatchOptions(): void {
  const id = currentPool;
  const show = (o: DispatcherOptions) => {
    $<HTMLInputElement>("srv-countries").value = o.countries.join(", ");
    $<HTMLInputElement>("srv-sticky").value = String(o.stickyMin);
    $<HTMLInputElement>("srv-user").value = o.authUser;
    $<HTMLInputElement>("srv-pass").value = o.authPass;
  };
  show({ countries: [], stickyMin: 0, authUser: "", authPass: "" });
  void invoke<PrefsUpdate>("get_app_prefs")
    .then((p) => {
      if (id !== currentPool) return;
      const options = id === MAIN_POOL ? p.dispatcher?.options : p.poolDispatchers?.[id]?.options;
      if (options) show(options);
    })
    .catch(() => { /* browser preview or older backend */ });
}

/** Dispatcher filter, sticky sessions and login: applied at once, kept by the backend. */
function initDispatchOptions(): void {
  const countries = $<HTMLInputElement>("srv-countries");
  const sticky = $<HTMLInputElement>("srv-sticky");
  const user = $<HTMLInputElement>("srv-user");
  const pass = $<HTMLInputElement>("srv-pass");
  showDispatchOptions();
  let timer: number | undefined;
  const apply = () => {
    window.clearTimeout(timer);
    // Read now: the pool may be switched before the debounce ends.
    const target = currentPool;
    const options: DispatcherOptions = {
      countries: countries.value.split(/[\s,;]+/).map((c) => c.trim()).filter(Boolean),
      stickyMin: Math.max(0, Math.min(1440, Math.floor(Number(sticky.value) || 0))),
      authUser: user.value.trim(),
      authPass: pass.value,
    };
    timer = window.setTimeout(async () => {
      try {
        await invoke("set_dispatch_options", { pool: target, options });
        setStatusText("srv-opts-status", () => t("srv_opts_saved"));
      } catch (e) {
        const msg = String(e).slice(0, 120);
        setStatusText("srv-opts-status", () => t("srv_err", { e: msg }));
      }
    }, 400);
  };
  for (const el of [countries, sticky, user, pass]) el.addEventListener("input", apply);
}

/** The dispatcher hint names the protocols the selected listener accepts. */
function updateBrowserHint(): void {
  const hint = document.getElementById("browser-hint");
  const listener = (document.getElementById("srv-listener") as HTMLSelectElement | null)?.value;
  if (!hint) return;
  const key = listener === "http" ? "browser_hint_http" : listener === "socks5" ? "browser_hint_socks" : "browser_hint";
  hint.dataset.i18n = key;
  hint.textContent = t(key);
}

// ---------- diagnostics + updates ----------

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
      document.querySelectorAll(".tab").forEach((x) => {
        x.classList.toggle("active", x === tb);
        x.setAttribute("aria-selected", String(x === tb));
      });
      const name = (tb as HTMLElement).dataset.tab;
      if (name === "settings") tb.classList.remove("has-update");
      document.querySelectorAll(".screen").forEach((s) => s.classList.add("hidden"));
      const scr = document.getElementById(`screen-${name}`);
      scr?.classList.remove("hidden");
      dispatchMonitor.setPolling(name === "dispatch", t, setStatusText);
      if (name === "check") render();
      if (name === "dispatch") {
        dispatchMonitor.refreshAddr();
        renderPool();
        void dispatchMonitor.poll(t, setStatusText);
      } else if (name === "auto") {
        renderPool();
      }
    });
  });

  initSourcePanel({
    parseTextAsync: parseList,
    fetchUrl: (url) => invoke<string>("fetch_url_text", { url }),
    setStatus: setStatusT,
    maxProxies: MAX_PROXIES,
    maxInputChars: MAX_INPUT_CHARS,
    maxFileBytes: MAX_FILE_BYTES,
    maxFileMb: MAX_FILE_MB,
  });

  $("btn-start").addEventListener("click", () => void start());
  $("btn-stop").addEventListener("click", () => {
    stopFlag = true;
    if (running) {
      lastStatus = { key: "st_stopping" };
      $("status-line").textContent = t("st_stopping");
    }
    void invoke("cancel_check").catch(() => { /* backend may already be idle */ });
  });
  resultsView.init({
    onChange: () => render(),
    onAction: (action, rows) => void rowAction(action, rows),
    canRecheck: () => !running && !starting && !autoStatus.running,
    checking: () => running || starting,
  });
  $("search").addEventListener("input", () => { resultsView.reset(); render(); });
  $("btn-recheck-dead").addEventListener("click", () => {
    void recheck(results.filter((r) => !r.alive).map((r) => r.proxy));
  });
  document.querySelectorAll(".chip").forEach((c) => {
    c.addEventListener("click", () => {
      document.querySelectorAll(".chip").forEach((x) => {
        x.classList.remove("active");
        x.setAttribute("aria-pressed", "false");
      });
      c.classList.add("active");
      c.setAttribute("aria-pressed", "true");
      filter = (c as HTMLElement).dataset.f as typeof filter;
      resultsView.reset();
      render();
    });
  });

  async function doPoolSync(statusElId: string): Promise<void> {
    mergeAlive(results);
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

  async function doPoolClear(): Promise<void> {
    // The pool is shared by both tabs, persisted, and clearing it stops a
    // running dispatcher: ask first.
    if (pool.count === 0) return;
    if (!(await askConfirm(t("pool_clear_confirm", { n: pool.count })))) return;
    pool.clear();
    void syncPool(false).then(() => {
      setStatusText("srv-status", () => t("srv_off"));
      void dispatchMonitor.poll(t, setStatusText);
    });
    renderPool();
    updateCounts();
  }

  ($("btn-pool-clear") as HTMLButtonElement).addEventListener("click", () => void doPoolClear());
  ($("btn-auto-pool-clear") as HTMLButtonElement).addEventListener("click", () => void doPoolClear());

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
      void doExport((b as HTMLElement).dataset.exp as ExportKind);
    });
  });
  $("chk-dead").addEventListener("click", () => {
    includeDead = !includeDead;
    ($("chk-dead") as HTMLButtonElement).setAttribute("aria-pressed", String(includeDead));
  });

  $("btn-clear-out").addEventListener("click", async () => {
    if (results.length === 0) return;
    // The next result of a running check would bring them back.
    if (running || starting) {
      setStatusT("st_clear_busy");
      return;
    }
    if (!(await askConfirm(t("results_clear_confirm", { n: results.length })))) return;
    results = [];
    render(); updateCounts(); setProgress(0, 1);
    lastStatus = { key: "st_cleared" };
    $("status-line").textContent = t("st_cleared");
    keyOverHttp = false;
    renderKeyNote();
  });

  $("btn-direct").addEventListener("click", async () => {
    const url = settings.effectiveTestUrl();
    setStatusText("direct-res", () => "...");
    try {
      const ms = await invoke<number>("check_direct", { testUrl: url, timeoutMs: 10000 });
      setStatusText("direct-res", () => t("direct_ok", { ms }));
    } catch (e) {
      const msg = String(e).slice(0, 80);
      setStatusText("direct-res", () => t("direct_fail", { e: msg }));
    }
  });

  // Sites and DNSBL are not part of presets, and scheduled checks skip them.
  sitesPanel.init(() => {});
  const dnsbl = $<HTMLInputElement>("chk-dnsbl");
  dnsbl.checked = readFlag("pp-dnsbl", false);
  dnsbl.addEventListener("change", () => writeFlag("pp-dnsbl", dnsbl.checked));
  const speedSel = $<HTMLSelectElement>("speed-mb");
  speedSel.value = String(loadSpeedMb());
  speedSel.addEventListener("change", () => saveSpeedMb(Number(speedSel.value)));

  // presets and check target
  profiles.load();
  profiles.render(t);

  // Any manual tweak turns the configuration into a custom one instead of
  // pretending the selected preset still matches.
  const markCustomPreset = () => {
    if (!profiles.currentId) return;
    profiles.currentId = "";
    profiles.render(t);
  };

  ($("preset-chips") as HTMLElement).addEventListener("click", (e) => {
    const node = e.target as HTMLElement;
    const del = node.closest<HTMLElement>("[data-preset-delete]");
    if (del) {
      profiles.deleteId(del.dataset.presetDelete ?? "");
      profiles.save();
      profiles.render(t);
      return;
    }
    const chip = node.closest<HTMLElement>("[data-preset-id]");
    const id = chip?.dataset.presetId ?? "";
    const found = profiles.find(id);
    if (!found) return;
    const data = id.startsWith("builtin:") ? withOwnAddresses(found, settings.snapshotProfile()) : found;
    profiles.currentId = id;
    settings.applyProfile(data);
    profiles.render(t);
  });
  ($("btn-preset-save") as HTMLButtonElement).addEventListener("click", async () => {
    const res = await askText(t("preset_name"));
    if (res == null) return;
    const name = res.trim().slice(0, 40);
    if (!name) return;
    if (!profiles.upsert(name, settings.snapshotProfile())) {
      setStatusT("preset_limit");
      return;
    }
    profiles.save();
    profiles.currentId = "custom:" + name;
    profiles.render(t);
  });

  document.querySelectorAll<HTMLButtonElement>("#target-mode .seg-btn").forEach((b) => {
    b.addEventListener("click", () => {
      const mode = (b.dataset.target as TargetMode) ?? "public";
      if (settings.targetMode() === mode) return;
      markCustomPreset();
      settings.setTargetMode(mode);
    });
  });
  $("test-url").addEventListener("change", () => settings.updateTargetUI());
  $("test-url-custom").addEventListener("input", () => settings.updateTargetUI());
  $("judge-server").addEventListener("change", () => settings.updateTargetUI());
  $("judge-url").addEventListener("input", () => settings.updateTargetUI());
  $("judge-fast").addEventListener("click", () => {
    markCustomPreset();
    settings.setJudgeFull(false);
  });
  $("judge-full").addEventListener("click", () => {
    markCustomPreset();
    settings.setJudgeFull(true);
  });
  for (const id of ["judge-https", "judge-http"]) {
    $<HTMLInputElement>(id).addEventListener("change", (e) => {
      settings.keepOneScheme(e.target as HTMLInputElement);
      markCustomPreset();
      settings.updateTargetUI();
      scheduleAutoConfig();
    });
  }
  $("check-settings").addEventListener("input", markCustomPreset);
  $("check-settings").addEventListener("change", markCustomPreset);

  // ip-api Pro key. The key is stored locally and is visible to checked
  // proxies (the request must originate from the proxy); see README.
  ($("chk-ipapi-pro") as HTMLInputElement).addEventListener("change", () => settings.saveIpapiSettings());
  ($("ipapi-key") as HTMLInputElement).addEventListener("input", () => settings.saveIpapiSettings());
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
  bindSqchk("auto-on", () => autoSettings.enabled, (v) => { autoSettings.enabled = v; });
  bindSqchk("auto-webhook-on", () => autoSettings.sendWebhooks, (v) => { autoSettings.sendWebhooks = v; });

  // dispatcher
  ($("srv-port") as HTMLInputElement).addEventListener("input", () => dispatchMonitor.refreshAddr());
  dispatchMonitor.refreshAddr();
  ($("btn-pool-sync") as HTMLButtonElement).addEventListener("click", () => void doPoolSync("srv-status"));
  ($("btn-auto-pool-sync") as HTMLButtonElement).addEventListener("click", () => void doPoolSync("auto-status"));
  ($("btn-srv-copy") as HTMLButtonElement).addEventListener("click", async (e) => {
    try {
      await navigator.clipboard.writeText(($("srv-addr") as HTMLElement).textContent ?? "");
      flashLabel(e.currentTarget as HTMLElement, t("copied"));
    } catch { /* ignore */ }
  });
  ($("srv-listener") as HTMLSelectElement).addEventListener("change", updateBrowserHint);
  initDispatchOptions();
  poolApi.init();
  ($("btn-api-copy") as HTMLButtonElement).addEventListener("click", async (e) => {
    try {
      await navigator.clipboard.writeText(poolApi.exampleUrl());
      flashLabel(e.currentTarget as HTMLElement, t("copied"));
    } catch { /* ignore */ }
  });
  ($("btn-srv-start") as HTMLButtonElement).addEventListener("click", async () => {
    const port = Math.max(1, Math.min(65535, Number(($("srv-port") as HTMLInputElement).value) || 1080));
    const mode = ($("srv-mode") as HTMLSelectElement).value;
    const listener = ($("srv-listener") as HTMLSelectElement).value;
    setStatusText("srv-status", () => "...");
    try {
      if (pool.count === 0) {
        mergeAlive(results);
        renderPool();
        updateCounts();
      }
      const synced = await syncPool(false);
      if (synced < 0) throw new Error(t("pool_sync_err"));
      const addr = await invoke<string>("start_local_proxy", { pool: currentPool, port, mode, listener });
      setStatusText("srv-status", () => t("srv_on", { addr }));
      try {
        const st = await invoke<DispatchStatus>("local_proxy_status", { pool: currentPool });
        ($("srv-port") as HTMLInputElement).value = String(st.port);
      } catch { /* keep typed value */ }
      dispatchMonitor.refreshAddr();
      void dispatchMonitor.poll(t, setStatusText);
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
      await invoke("stop_local_proxy", { pool: currentPool });
    } catch { /* ignore */ }
    setStatusText("srv-status", () => t("srv_off"));
    void dispatchMonitor.poll(t, setStatusText);
    void updateTray();
  });

  // tray
  $("btn-tray").addEventListener("click", async () => {
    try {
      await invoke("hide_to_tray");
    } catch { /* ignore */ }
  });

  // diagnostics + updates
  geoCache.load();
  errorLog.load();
  errorLog.render();
  initInfoPanel({
    t,
    getData: () => ({
      app: document.getElementById("app-ver")?.textContent,
      platform: navigator.userAgent,
      lang,
      results: results.length,
      alive: results.filter((r) => r.alive).length,
      pool: pool.count,
      judge: settings.isJudgeMode() ? settings.effectiveTestUrl() : null,
      errors: errorLog.recent(20),
    }),
  });
  const updater = initUpdater({
    t,
    setStatusText,
    logErr,
    isTauriRuntime,
    onFound: (version) => {
      if (!screenVisible("screen-settings")) {
        document.querySelector('.tab[data-tab="settings"]')?.classList.add("has-update");
      }
      // One notification per version, not one per launch.
      let seen: string | null = null;
      try {
        seen = localStorage.getItem("pp-upd-seen");
      } catch { /* ignore */ }
      if (seen === version) return;
      void notifier.send(t("notif_upd_t", { v: version }), t("notif_upd_b")).then((sent) => {
        if (!sent) return;
        try {
          localStorage.setItem("pp-upd-seen", version);
        } catch { /* ignore */ }
      });
    },
  });
  appSettings.init();
  resultsView.hideCreds = appSettings.hideCreds;
  pool.hideCreds = appSettings.hideCreds;

  // auto timers + webhook
  document.getElementById("auto-min")?.addEventListener("change", armTimers);
  loadMaxPing();
  syncMaxPingInputs();
  document.querySelectorAll<HTMLInputElement>("[data-pool-filter]").forEach((el) => {
    el.addEventListener("input", () => {
      const id = el.dataset.poolFilter;
      if (id) pool.resetScroll(id);
      renderPool();
    });
  });
  document.querySelectorAll<HTMLSelectElement>("[data-pool-sort]").forEach((el) => {
    el.addEventListener("change", () => {
      const id = el.dataset.poolSort;
      if (id) pool.resetScroll(id);
      renderPool();
    });
  });
  pingHist = loadPingHist();
  renderPingGraph(pingHist, lang);
  document.getElementById("auto-max-ping")?.addEventListener("change", (e) => {
    const v = Math.max(0, Math.floor(Number((e.target as HTMLInputElement).value) || 0));
    maxPingMs = Number.isFinite(v) ? v : 0;
    try {
      localStorage.setItem(scopedKey("pp-maxping", currentPool), String(maxPingMs));
    } catch { /* ignore */ }
    syncMaxPingInputs();
    scheduleAutoConfig();
  });
  webhooks.load();
  subscriptions.load();
  subscriptions.render(t);
  ($("btn-subs-add") as HTMLButtonElement).addEventListener("click", () => {
    subscriptions.add();
    subscriptions.render(t);
    const inputs = document.querySelectorAll<HTMLInputElement>("#subs-list input");
    inputs[inputs.length - 1]?.focus();
  });
  void listen<{ pool?: string; results: SubscriptionResult[] }>("auto://subscriptions", (e) => {
    if (!shownPool(e.payload.pool)) return;
    setStatusText("subs-status", () => formatSubscriptionResults(e.payload.results, t));
  }).catch(() => {});
  settings.loadIpapiSettings();
  loadAutoSettings();
  if (!settings.loadCurrentSettings()) profiles.currentId = "builtin:standard";
  showWebhookOptions();
  ($("chk-webhook-changes") as HTMLInputElement).addEventListener("change", (e) => {
    webhooks.changesOnly = (e.target as HTMLInputElement).checked;
    webhooks.saveOptions();
  });
  const tpl = $<HTMLTextAreaElement>("webhook-template");
  tpl.addEventListener("input", () => {
    webhooks.template = tpl.value;
    webhooks.saveOptions();
  });
  ($("chk-webhook-creds") as HTMLInputElement).addEventListener("change", (e) => {
    webhooks.includeCreds = (e.target as HTMLInputElement).checked;
    webhooks.saveCreds();
    scheduleAutoConfig();
  });
  webhooks.render(t);
  ($("btn-webhook-add") as HTMLButtonElement).addEventListener("click", () => {
    if (webhooks.urls.length >= 10) return;
    webhooks.urls.push("");
    webhooks.saveUrls();
    webhooks.render(t);
    const inputs = document.querySelectorAll<HTMLInputElement>("#webhook-list input");
    inputs[inputs.length - 1]?.focus();
  });
  ($("btn-auto-now") as HTMLButtonElement).addEventListener("click", () => {
    void invoke("run_auto_now", { pool: currentPool }).catch((e) => {
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
    webhooks.renderFormat(t);
  });
  void listen<AutoStatusEvent>("auto://status", (e) => {
    const id = e.payload.pool ?? MAIN_POOL;
    autoStatus.handle(e.payload, shownPool(id));
    // An offline cycle repeats every minute: report it once until a check succeeds.
    if (e.payload.state === "offline") {
      void notifier.alert(`offline@${id}`, notifTitle("notif_offline_t", id), t("notif_offline_b"));
    } else if (e.payload.state === "suspect") {
      void notifier.alert(`suspect@${id}`, notifTitle("notif_suspect_t", id), t("notif_suspect_b"));
    } else if (e.payload.state === "done") {
      notifier.latch.clear(`offline@${id}`);
      notifier.latch.clear(`suspect@${id}`);
    }
  }).catch(() => {});
  void listen<AutoPoolEvent>("auto://pool", (e) => {
    const id = e.payload.pool ?? MAIN_POOL;
    const before = shownPool(id) ? pool.count : poolCounts.get(id);
    const after = e.payload.items.length;
    poolCounts.set(id, after);
    if (shownPool(id)) {
      applyAutoPool(e.payload, (items) => pool.set(items), () => { renderPool(); updateCounts(); });
      void attachHistory();
    }
    const min = appSettings.poolMin;
    if (before != null && before >= min && after < min) {
      void notifier.send(
        notifTitle(after === 0 ? "notif_pool_t" : "notif_pool_low_t", id),
        after === 0 ? t("notif_pool_b") : t("notif_pool_low_b", { n: after, min }),
      );
    }
  }).catch(() => {});
  void listen<AutoWebhookEvent>("auto://webhook", (e) => {
    const id = e.payload.pool ?? MAIN_POOL;
    if (shownPool(id)) showAutoWebhook(e.payload, t, setStatusText);
    const failed = e.payload.results.filter((r) => !r.ok);
    if (failed.length === 0) notifier.latch.clear(`webhook@${id}`);
    else void notifier.alert(`webhook@${id}`, notifTitle("notif_webhook_t", id), formatWebhookResults(failed, t));
  }).catch(() => {});
  // A file the backend could not save: what it holds is lost at the next
  // launch unless the cause (a full disk, a read-only folder) is removed.
  void listen<{ file: string; error: string }>("app://save-error", (e) => {
    logErr(`save ${e.payload.file}: ${e.payload.error}`);
    setStatusT("st_savefail", { e: `${e.payload.file}: ${e.payload.error}` });
  }).catch(() => {});
  void listen("app://hidden-to-tray", () => {
    if (readFlag("pp-tray-hint", false)) return;
    void notifier.send(t("notif_tray_t"), t("notif_tray_b")).then((sent) => {
      if (sent) writeFlag("pp-tray-hint", true);
    });
  }).catch(() => {});
  // After a reload during a check the buttons stay locked, and a manual
  // check sends this page no events: ask the backend until it is free.
  const waitUntilFree = (): void => {
    window.setTimeout(() => {
      void invoke<boolean>("check_busy")
        .then((busy) => {
          if (busy) waitUntilFree();
          else if (!running) autoStatus.release();
        })
        .catch(() => {});
    }, 2000);
  };
  void invoke<boolean>("check_busy")
    .then((busy) => {
      if (!busy) return;
      autoStatus.restore();
      waitUntilFree();
    })
    .catch(() => {});

  // The renderer can be reloaded independently of the Rust process; restore
  // the authoritative pool and dispatcher state instead of showing empty.
  initPoolSwitchers();
  void loadPools();
  void loadPoolItems();
  void dispatchMonitor.poll(t, setStatusText);

  $("btn-hist-clear").addEventListener("click", async () => {
    if (!(await askConfirm(t("hist_clear_confirm")))) return;
    try {
      await invoke("clear_history");
    } catch { /* older backend */ }
    results = results.map((r) => ({ ...r, history: undefined }));
    for (const p of pool.items) p.history = undefined;
    render();
    renderPool();
    void refreshHistorySize();
  });

  // Keyboard shortcuts: Ctrl+Enter starts, Esc stops, Ctrl+F filters.
  document.addEventListener("keydown", (e) => {
    if (e.defaultPrevented || !$("text-modal").classList.contains("hidden")) return;
    const mod = e.ctrlKey || e.metaKey;
    const onCheck = screenVisible("screen-check");
    if (mod && e.key === "Enter" && onCheck) {
      e.preventDefault();
      const b = $("btn-start") as HTMLButtonElement;
      if (!b.disabled) b.click();
    } else if (e.key === "Escape") {
      // Esc closes an open menu first; typing in a field never stops a run,
      // which cannot be resumed and whose results Start would replace.
      const open = document.querySelectorAll<HTMLElement>(".menu:not(.hidden)");
      if (open.length > 0) {
        e.preventDefault();
        open.forEach((m) => m.classList.add("hidden"));
        return;
      }
      const target = e.target as HTMLElement | null;
      if (target?.closest("input, textarea, select, [contenteditable='true']")) return;
      if (!onCheck || !(running || autoStatus.running)) return;
      e.preventDefault();
      const b = $("btn-stop") as HTMLButtonElement;
      if (!b.disabled) b.click();
    } else if (mod && e.code === "KeyF") {
      const field = onCheck
        ? $<HTMLInputElement>("search")
        : document.querySelector<HTMLInputElement>(".screen:not(.hidden) .pool-filter");
      if (!field) return;
      e.preventDefault();
      field.focus();
      field.select();
    }
  });

  applyI18n();
  armTimers();
  if (appSettings.updateOnStart && isTauriRuntime()) {
    // Let the window settle; the result shows on the Settings tab.
    window.setTimeout(() => void updater.checkQuietly(), 3000);
  }
});
