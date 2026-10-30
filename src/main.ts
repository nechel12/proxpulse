import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { clip } from "./clip.ts";
import { ResultsView } from "./results.ts";
import { PoolView, type PoolItem } from "./pool.ts";
import { exportList, type ExportKind } from "./export-panel.ts";
import { loadPingHist, pushPingPointStats, renderPingGraph, type PingPoint } from "./pinggraph.ts";
import { refreshStatusTexts, setStatusText } from "./status.ts";
import { poolCountry } from "./format.ts";
import { GeoCacheStore } from "./geo-store.ts";
import { WebhookPanel } from "./webhooks.ts";
import { type Lang, translate } from "./i18n.ts";
import { extractProxiesFromText } from "./parse.ts";
import { parseFallback, parseTextAsync, type ParseResult } from "./parse-runner.ts";
import { ProfilePanel } from "./profiles-panel.ts";
import { type TargetMode } from "./profiles.ts";
import { SettingsPanel } from "./settings-panel.ts";
import { initUpdater } from "./updater.ts";
import { errorLog, logErr } from "./diagnostics.ts";
import { DispatchMonitor, type DispatchStatus } from "./dispatch-monitor.ts";
import { AutoSettingsStore } from "./auto-settings.ts";
import { AutoStatusController, type AutoStatusEvent } from "./auto-status.ts";
import { initSourcePanel } from "./source-panel.ts";
import { initInfoPanel } from "./info-panel.ts";
import { askText, screenVisible, setProgress } from "./ui.ts";
import { NetStatus } from "./net-status.ts";
import { runCheckList } from "./check-client.ts";
import { applyAutoPool, showAutoWebhook, type AutoPoolEvent, type AutoWebhookEvent } from "./auto-events.ts";
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
const resultsView = new ResultsView();
let filter: "all" | "alive" | "dead" = "all";
let stopFlag = false;
let running = false;
let starting = false;
let includeDead = false;
const pool = new PoolView();
const dispatchMonitor = new DispatchMonitor();
const profiles = new ProfilePanel();
const settings = new SettingsPanel({ t, schedule: () => scheduleAutoConfig() });

function setStatusT(key: string, params?: Record<string, string | number>) {
  lastStatus = { key, params };
  $("status-line").textContent = t(key, params);
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
  profiles.render(t);
  updateCounts();
  settings.updateTargetUI();
  renderNet();
  refreshStatusTexts();
  dispatchMonitor.renderPill(t);
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

function renderPool() {
  pool.render(t, (raw) => {
    pool.remove(raw);
    void syncPool(false);
    renderPool();
    updateCounts();
  }, screenVisible);
}

function mergePool(items: { raw: string; url?: string | null; latency: number | null; country?: string | null }[]) {
  pool.merge(items, maxPingMs, MAX_PROXIES);
}

let maxPingMs = 0;


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

const geoCache = new GeoCacheStore();

function readCheckSettings() {
  const judge = settings.isJudgeMode();
  return {
    testUrl: settings.effectiveTestUrl(),
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
    ipapiKey: settings.activeIpapiKey(),
  };
}

const MAX_PROXIES = 20_000;
const MAX_INPUT_CHARS = 10 * 1024 * 1024;
const MAX_FILE_BYTES = 10 * 1024 * 1024;
const MAX_FILE_MB = 10;

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

async function syncPool(notify: boolean): Promise<number> {
  try {
    const n = await invoke<number>("set_dispatch_pool", {
      items: pool.items.map((p) => ({
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

async function updateTray(): Promise<void> {
  try {
    const urls = webhooks.getUrls();
    await invoke("update_tray_status", {
      webhookOn: autoSettings.autoOn && urls.length > 0,
      webhookUrl: urls[0] ?? "",
      lang,
    });
  } catch { /* tray may be unavailable */ }
}

// ---------- scheduled checks (Rust scheduler events) ----------

const webhooks = new WebhookPanel(() => scheduleAutoConfig());
const autoSettings = new AutoSettingsStore({
  readCheckSettings,
  maxPingMs: () => maxPingMs,
  webhooks,
  saveCurrentSettings: () => settings.saveCurrentSettings(),
  updateTray,
});

const autoStatus = new AutoStatusController({
  t,
  setStatusText,
  isManualRunning: () => running,
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
  if (autoStatus.running) {
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
  if (!/^https?:\/\//i.test(settings.effectiveTestUrl())) {
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
  resultsView.reset();
  render();
  updateCounts();
  ($("btn-start") as HTMLButtonElement).disabled = true;
  ($("btn-stop") as HTMLButtonElement).disabled = false;
  setNet("work", "net_work", { s: 0 });
  setProgress(0, list.length);
  const t0 = performance.now();

  let busy = false;
  try {
    results = await runCheckList(list, readCheckSettings(), geoCache.relevant(list), (done, total) => {
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
    ($("btn-stop") as HTMLButtonElement).disabled = !autoStatus.running;
  }
  if (busy) {
    setStatusT("auto_skip");
    setNet("idle", "net_idle");
    return;
  }
  geoCache.learn(results);
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

async function doExport(kind: ExportKind): Promise<void> {
  await exportList(kind, exportScope(), { isTauriRuntime, notify: setStatusT });
}

// ---------- dispatcher ----------

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
      document.querySelectorAll(".tab").forEach((x) => x.classList.remove("active"));
      tb.classList.add("active");
      const name = (tb as HTMLElement).dataset.tab;
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
    parseTextAsync,
    extractProxies: extractProxiesFromText,
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
  $("search").addEventListener("input", () => { resultsView.reset(); render(); });
  document.querySelectorAll(".chip").forEach((c) => {
    c.addEventListener("click", () => {
      document.querySelectorAll(".chip").forEach((x) => x.classList.remove("active"));
      c.classList.add("active");
      filter = (c as HTMLElement).dataset.f as typeof filter;
      resultsView.reset();
      render();
    });
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
    pool.clear();
    void syncPool(false).then(() => {
      setStatusText("srv-status", () => t("srv_off"));
      void dispatchMonitor.poll(t, setStatusText);
    });
    renderPool();
    updateCounts();
  }

  ($("btn-pool-clear") as HTMLButtonElement).addEventListener("click", doPoolClear);
  ($("btn-auto-pool-clear") as HTMLButtonElement).addEventListener("click", doPoolClear);

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
    const data = profiles.find(id);
    if (!data) return;
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
  bindSqchk("disp-auto-on", () => autoSettings.dispAutoOn, (v) => { autoSettings.dispAutoOn = v; });
  bindSqchk("auto-on", () => autoSettings.autoOn, (v) => { autoSettings.autoOn = v; });

  // dispatcher
  ($("srv-port") as HTMLInputElement).addEventListener("input", () => dispatchMonitor.refreshAddr());
  dispatchMonitor.refreshAddr();
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
      if (pool.count === 0) {
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
      await invoke("stop_local_proxy");
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
    getData: () => ({
      app: document.getElementById("app-ver")?.textContent,
      lang,
      results: results.length,
      alive: results.filter((r) => r.alive).length,
      pool: pool.count,
      judge: settings.isJudgeMode() ? settings.effectiveTestUrl() : null,
      errors: errorLog.recent(20),
    }),
  });
  initUpdater({ t, setStatusText, logErr, isTauriRuntime });

  // auto timers + webhook
  for (const id of ["disp-auto-min", "auto-min"]) {
    document.getElementById(id)?.addEventListener("change", armTimers);
  }
  loadMaxPing();
  syncMaxPingInputs();
  document.querySelectorAll<HTMLInputElement>("[data-pool-filter]").forEach((el) => {
    el.addEventListener("input", () => {
      const id = el.dataset.poolFilter;
      if (id) pool.resetLimit(id);
      renderPool();
    });
  });
  document.querySelectorAll<HTMLSelectElement>("[data-pool-sort]").forEach((el) => {
    el.addEventListener("change", () => {
      const id = el.dataset.poolSort;
      if (id) pool.resetLimit(id);
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
  webhooks.load();
  settings.loadIpapiSettings();
  loadAutoSettings();
  if (!settings.loadCurrentSettings()) profiles.currentId = "builtin:standard";
  ($("chk-webhook-creds") as HTMLInputElement).checked = webhooks.includeCreds;
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
  void listen<AutoStatusEvent>("auto://status", (e) => autoStatus.handle(e.payload)).catch(() => {});
  void listen<AutoPoolEvent>("auto://pool", (e) => applyAutoPool(e.payload, (items) => pool.set(items), () => { renderPool(); updateCounts(); })).catch(() => {});
  void listen<AutoWebhookEvent>("auto://webhook", (e) => showAutoWebhook(e.payload, t, setStatusText)).catch(() => {});
  void invoke<boolean>("check_busy")
    .then((busy) => {
      if (!busy) return;
      autoStatus.restore();
      ($("btn-start") as HTMLButtonElement).disabled = true;
      ($("btn-stop") as HTMLButtonElement).disabled = false;
      ($("btn-auto-now") as HTMLButtonElement).disabled = true;
    })
    .catch(() => {});

  // The renderer can be reloaded independently of the Rust process; restore
  // the authoritative pool and dispatcher state instead of showing empty.
  void invoke<PoolItem[]>("get_dispatch_pool")
    .then((items) => {
      pool.set(items.map((p) => ({
        raw: p.raw,
        url: p.url,
        latency: p.latency,
        country: p.country ?? null,
      })));
      renderPool();
      updateCounts();
    })
    .catch(() => { /* older backend */ });
  void dispatchMonitor.poll(t, setStatusText);

  applyI18n();
  armTimers();
});
