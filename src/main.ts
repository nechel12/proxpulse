import { Channel, invoke } from "@tauri-apps/api/core";
import { extractProxiesFromText, parseInput } from "./parse.ts";

type ProxyResult = {
  proxy: string;
  proto: string;
  alive: boolean;
  latency_ms: number | null;
  jitter_ms: number | null;
  success_rate: number;
  attempts: number;
  ip: string | null;
  country: string | null;
  country_code: string | null;
  city: string | null;
  isp: string | null;
  org: string | null;
  asn: string | null;
  ip_type: string | null;
  anonymity: string | null;
  tamper: string | null;
  tls: string | null;
  tls_info: string | null;
  error: string | null;
};

type Lang = "ru" | "en";

// ================= i18n =================

const I18N: Record<Lang, Record<string, string>> = {
  ru: {
    stat_total: "Всего", stat_alive: "Живые", stat_dead: "Мёртвые", stat_ping: "Ср. пинг",
    tab_check: "Проверка", tab_dispatch: "Диспетчер",
    src_title: "▸ Источник",
    src_ph: "Вставь прокси в любом формате",
    btn_clear: "Очистить", btn_import: "Импорт", btn_copy: "Копировать",
    import_hint: "Импорт: txt, csv, json, log и любые другие — форматы распознаются автоматически.",
    import_url_ph: "https://… — список прокси", import_url_btn: "Загрузить",
    url_loaded: "Импортировано по ссылке: {n}.", url_fail: "Не удалось загрузить ссылку",
    set_title: "▸ Настройки", f_testurl: "Тестовый URL",
    opt_google: "google generate_204 (быстро)", opt_cf: "cloudflare generate_204 (быстро)",
    opt_ipify: "api.ipify.org (покажет IP)", opt_httpbin: "httpbin.org/ip (покажет IP)",
    opt_ipapi: "ip-api.com/json (IP + гео)", opt_example: "example.com",
    opt_judge: "ProxPulse Judge",
    custom_ph: "или свой URL https://...",
    custom_judge_ph: "свой judge, например https://...",
    judge_fast: "Быстрая", judge_full: "Полная",
    profile: "Профиль", profile_save: "Сохранить", profile_del: "Удалить",
    profile_none: "— пресет —", profile_name: "Название профиля",
    prof_fast: "Быстрый", prof_deep: "Глубокий", prof_judge: "Judge",
    f_timeout: "Таймаут, мс", f_threads: "Потоки", f_repeats: "Замеры",
    f_protonoscheme: "Тип без схемы", opt_http: "HTTP",
    chk_geo: "Гео и тип IP", chk_anon: "Анонимность", chk_tamper: "Целостность", chk_tls: "TLS-интегрити",
    deep_hint: "Глубокие проверки медленнее: гео — через ip-api, анонимность — через httpbin, целостность — сверка с example.com, TLS — сверка сертификата.",
    precheck_label: "Пречек порта", precheck_hint: "Быстрый TCP-пречек отсеивает мёртвые хосты до полной проверки.",
    btn_direct: "Проверить связь", btn_start: "▶ Старт", btn_stop: "■ Стоп",
    res_title: "▸ Результат", chip_all: "Все", chip_alive: "Живые", chip_dead: "Мёртвые",
    search_ph: "Фильтр: 1.2.3.4, :8080, socks, страна, isp...",
    export_btn: "Экспорт", exp_copy: "Скопировать",
    include_dead: "мёртвые", include_dead_t: "Включать мёртвые в экспорт",
    th_proxy: "Прокси", th_type: "Тип", th_status: "Статус", th_ping: "Пинг",
    th_stab: "Джиттер/SR", th_anon: "Анон", th_geo: "Гео", th_iptype: "Тип IP", th_info: "Инфо",
    empty_idle: "Пока пусто — вставь список слева и жми Старт.",
    empty_filter: "Ничего не найдено под фильтр.",
    pill_alive: "ALIVE", pill_dead: "DEAD",
    net_idle: "ожидание", net_work: "проверка {s}с", net_done: "готово", net_stopped: "остановлено",
    st_ready: "Готов к проверке.", st_empty: "Список пуст — вставь прокси.",
    st_badurl: "Тестовый URL должен начинаться с http(s)://",
    st_checking: "Проверка {done}/{total} ...", st_done: "Готово за {secs}с. Живых: {alive}/{total}.",
    st_stopped: "Остановлено. Живых: {alive}/{total}.", st_stopping: "Останавливаю...",
    st_cleared: "Результат очищен.", st_nocopy: "Нечего копировать.",
    st_copied: "Скопировано: {n}.", st_nothing: "Нечего экспортировать.",
    st_saved: "Сохранено: {path}", st_downloaded: "Скачано: {name} (браузерный режим).",
    direct_ok: "прямой доступ OK {ms}ms", direct_fail: "связи нет: {e}",
    file_loaded: "Импортировано: {n} из {f} ф.", file_none: "В {name} прокси не найдены.",
    file_fail: "Не удалось прочитать {name}.",
    srv_title: "▸ Локальный сервер", srv_hint: "Поднимает прокси 127.0.0.1:порт и гоняет трафик через живые из проверки.",
    srv_port: "Порт", srv_mode: "Режим",
    mode_rr: "Ротация", mode_fastest: "Самый быстрый", mode_failover: "Замена при падении",
    srv_start: "Включить", srv_stop: "Выключить",
    stat_req: "Запросы", stat_err: "Ошибки", stat_cur: "Текущий апстрим",
    browser_hint: "Вставь в браузер как HTTP-прокси: 127.0.0.1 и порт выше.",
    pool_title: "▸ Пул живых", pool_sync: "Синхронизировать",
    pool_filter_ph: "Страна, IP…", pool_sort_added: "Порядок",
    pool_sort_ping: "Пинг", pool_sort_country: "Страна",
    pool_hint: "Пул пополняется автоматически после каждой проверки.",
    pool_empty: "Пусто — запусти проверку.", pool_synced: "Пул: {n}.",
    srv_on: "Сервер на {addr}.", srv_off: "Сервер выключен.",
    srv_err_empty: "Пул пуст — сначала проверь прокси.", srv_err: "Ошибка сервера: {e}",
    tray_hide: "Скрыть в трей", tab_auto: "Авто", tab_info: "Инфо",
    srv_listener: "Протокол сервера", lis_both: "HTTP + SOCKS5",
    lis_http: "Только HTTP", lis_socks: "Только SOCKS5",
    disp_auto_title: "▸ Автопроверка", auto_on: "Вкл", auto_every: "Каждые, мин",
    max_ping: "Макс. пинг, мс", max_ping_ph: "0 — без лимита",
    auto_title: "▸ Авторежим",
    auto_hint: "Общий рабочий пул перепроверяется по таймеру, мёртвые вылетают, живые уходят на вебхук.",
    auto_now: "Проверить сейчас",
    webhook_title: "▸ Вебхук", webhook_url: "URL вебхука", webhook_format: "Формат",
    webhook_add: "+ Добавить",
    webhook_send: "Отправить сейчас",
    webhook_hint: "После каждой автопроверки список уходит POST-запросом. Пусто — отправки нет.",
    info_title: "▸ О приложении",
    info_about: "Быстрый чекер прокси: проверка живости, гео, анонимности и TLS, локальный диспетчер и авторассылка пула на вебхук.",
    show_more: "Показать ещё ({n})",
    graph_title: "Средний пинг живых по проверкам",
    diag_title: "▸ Диагностика", diag_ver: "Версия", diag_copy: "Копировать",
    diag_upd: "Проверить обновления", upd_open: "Открыть релиз",
    upd_latest: "Установлена последняя ({v})", upd_found: "Доступна {v} (у тебя {cur})",
    upd_none: "Релизов пока нет", upd_fail: "Не удалось проверить",
    upd_install: "Установить", upd_confirm: "Установить {v}? Приложение перезапустится.",
    upd_installing: "Установка…", upd_restart: "Установлено, перезапусти приложение",
    auto_running: "Автопроверка {n} шт...",
    auto_pruned: "Автопроверка: живых {alive}/{total}.",
    auto_skip: "Пропуск: идёт другая проверка.",
    webhook_sent: "Вебхук: {r}", webhook_fail: "Вебхук не ушёл: {e}",
    webhook_empty: "Пул пуст, отправлять нечего.",
  },
  en: {
    stat_total: "Total", stat_alive: "Alive", stat_dead: "Dead", stat_ping: "Avg ping",
    tab_check: "Check", tab_dispatch: "Dispatcher",
    src_title: "▸ Source",
    src_ph: "Paste proxies in any format",
    btn_clear: "Clear", btn_import: "Import", btn_copy: "Copy",
    import_hint: "Import: txt, csv, json, log and anything else — formats are auto-detected.",
    import_url_ph: "https://… — proxy list", import_url_btn: "Load",
    url_loaded: "Imported from URL: {n}.", url_fail: "URL fetch failed",
    set_title: "▸ Settings", f_testurl: "Test URL",
    opt_google: "google generate_204 (fast)", opt_cf: "cloudflare generate_204 (fast)",
    opt_ipify: "api.ipify.org (shows IP)", opt_httpbin: "httpbin.org/ip (shows IP)",
    opt_ipapi: "ip-api.com/json (IP + geo)", opt_example: "example.com",
    opt_judge: "ProxPulse Judge",
    custom_ph: "or custom URL https://...",
    custom_judge_ph: "custom judge, e.g. https://...",
    judge_fast: "Fast", judge_full: "Full",
    profile: "Profile", profile_save: "Save", profile_del: "Delete",
    profile_none: "— preset —", profile_name: "Profile name",
    prof_fast: "Fast", prof_deep: "Deep", prof_judge: "Judge",
    f_timeout: "Timeout, ms", f_threads: "Threads", f_repeats: "Samples",
    f_protonoscheme: "No-scheme type", opt_http: "HTTP",
    chk_geo: "Geo & IP type", chk_anon: "Anonymity", chk_tamper: "Integrity", chk_tls: "TLS integrity",
    deep_hint: "Deep checks are slower: geo via ip-api, anonymity via httpbin, integrity vs example.com, TLS vs certificate.",
    precheck_label: "Port precheck", precheck_hint: "Fast TCP precheck drops dead hosts before the full check.",
    btn_direct: "Test connection", btn_start: "▶ Start", btn_stop: "■ Stop",
    res_title: "▸ Results", chip_all: "All", chip_alive: "Alive", chip_dead: "Dead",
    search_ph: "Filter: 1.2.3.4, :8080, socks, country, isp...",
    export_btn: "Export", exp_copy: "Copy",
    include_dead: "dead", include_dead_t: "Include dead in export",
    th_proxy: "Proxy", th_type: "Type", th_status: "Status", th_ping: "Ping",
    th_stab: "Jitter/SR", th_anon: "Anon", th_geo: "Geo", th_iptype: "IP type", th_info: "Info",
    empty_idle: "Empty — paste a list on the left and hit Start.",
    empty_filter: "Nothing matches the filter.",
    pill_alive: "ALIVE", pill_dead: "DEAD",
    net_idle: "idle", net_work: "checking {s}s", net_done: "done", net_stopped: "stopped",
    st_ready: "Ready.", st_empty: "List is empty — paste proxies.",
    st_badurl: "Test URL must start with http(s)://",
    st_checking: "Checking {done}/{total} ...", st_done: "Done in {secs}s. Alive: {alive}/{total}.",
    st_stopped: "Stopped. Alive: {alive}/{total}.", st_stopping: "Stopping...",
    st_cleared: "Results cleared.", st_nocopy: "Nothing to copy.",
    st_copied: "Copied: {n}.", st_nothing: "Nothing to export.",
    st_saved: "Saved: {path}", st_downloaded: "Downloaded: {name} (browser mode).",
    direct_ok: "direct OK {ms}ms", direct_fail: "no connection: {e}",
    file_loaded: "Imported: {n} from {f} file(s).", file_none: "No proxies found in {name}.",
    file_fail: "Failed to read {name}.",
    srv_title: "▸ Local server", srv_hint: "Serves a proxy on 127.0.0.1:port routed through alive proxies from checks.",
    srv_port: "Port", srv_mode: "Mode",
    mode_rr: "Rotation", mode_fastest: "Fastest", mode_failover: "Failover",
    srv_start: "Enable", srv_stop: "Disable",
    stat_req: "Requests", stat_err: "Errors", stat_cur: "Current upstream",
    browser_hint: "Put it into your browser as HTTP proxy: 127.0.0.1 and the port above.",
    pool_title: "▸ Alive pool", pool_sync: "Sync",
    pool_filter_ph: "Country, IP…", pool_sort_added: "Added",
    pool_sort_ping: "Ping", pool_sort_country: "Country",
    pool_hint: "Pool auto-fills after every check.",
    pool_empty: "Empty — run a check.", pool_synced: "Pool: {n}.",
    srv_on: "Server on {addr}.", srv_off: "Server off.",
    srv_err_empty: "Pool is empty — check proxies first.", srv_err: "Server error: {e}",
    tray_hide: "Hide to tray", tab_auto: "Auto", tab_info: "Info",
    srv_listener: "Server protocol", lis_both: "HTTP + SOCKS5",
    lis_http: "HTTP only", lis_socks: "SOCKS5 only",
    disp_auto_title: "▸ Auto-check", auto_on: "On", auto_every: "Every, min",
    max_ping: "Max ping, ms", max_ping_ph: "0 — no limit",
    auto_title: "▸ Auto mode",
    auto_hint: "Shared alive pool is re-checked on a timer; dead drop out, alive go to the webhook.",
    auto_now: "Check now",
    webhook_title: "▸ Webhook", webhook_url: "Webhook URL", webhook_format: "Format",
    webhook_add: "+ Add",
    webhook_send: "Send now",
    webhook_hint: "After each auto-check the list is POSTed. Empty — no send.",
    info_title: "▸ About",
    info_about: "Fast proxy checker: liveness, geo, anonymity and TLS checks, local dispatcher and pool auto-posting to a webhook.",
    show_more: "Show more ({n})",
    graph_title: "Alive avg ping per check",
    diag_title: "▸ Diagnostics", diag_ver: "Version", diag_copy: "Copy",
    diag_upd: "Check for updates", upd_open: "Open release",
    upd_latest: "Up to date ({v})", upd_found: "Available {v} (yours {cur})",
    upd_none: "No releases yet", upd_fail: "Check failed",
    upd_install: "Install", upd_confirm: "Install {v}? The app will restart.",
    upd_installing: "Installing…", upd_restart: "Installed — restart the app",
    auto_running: "Auto-checking {n}...",
    auto_pruned: "Auto-check: {alive}/{total} alive.",
    auto_skip: "Skipped: another check is running.",
    webhook_sent: "Webhook: {r}", webhook_fail: "Webhook failed: {e}",
    webhook_empty: "Pool is empty, nothing to send.",
  },
};

let lang: Lang = (localStorage.getItem("pp-lang") as Lang) || "ru";
if (lang !== "ru" && lang !== "en") lang = "ru";

function t(key: string, params?: Record<string, string | number>): string {
  let s = I18N[lang][key] ?? I18N.ru[key] ?? key;
  if (params) {
    for (const [k, v] of Object.entries(params)) s = s.replace(`{${k}}`, String(v));
  }
  return s;
}

let lastStatus: { key: string; params?: Record<string, string | number> } = { key: "st_ready" };

const $ = <T extends HTMLElement = HTMLElement>(id: string) =>
  document.getElementById(id) as T;

let results: ProxyResult[] = [];
let filter: "all" | "alive" | "dead" = "all";
let renderLimit = 250;
const RENDER_STEP = 250;
let stopFlag = false;
let running = false;
let includeDead = false;
type PoolItem = { raw: string; latency: number | null; country: string | null };
let dispatchPool: PoolItem[] = [];
let pollTimer: number | undefined;
let autoBusy = false;
let dispAutoTimer: number | undefined;
let autoTimer: number | undefined;
const JUDGE_DEFAULT = "https://proxycheck.lmtunnel.com";
let judgeFull = true;
let dispAutoOn = false;
let autoOn = false;

// ---------- ui helpers ----------

function isJudgeMode(): boolean {
  return ($("test-url") as HTMLSelectElement).value === "judge";
}

function judgeBase(): string {
  const custom = ($("test-url-custom") as HTMLInputElement).value.trim();
  const base = (custom || JUDGE_DEFAULT).replace(/\/+$/, "");
  if (/\/(judge|generate_204)$/.test(base)) return base;
  try {
    const u = new URL(base);
    // custom path on a self-hosted judge (e.g. https://host/svc) → use as-is
    if (u.pathname && u.pathname !== "/") return base;
  } catch { /* invalid URL — backend will report it */ }
  return base + (judgeFull ? "/judge" : "/generate_204");
}

function effectiveTestUrl(): string {
  if (isJudgeMode()) return judgeBase();
  const sel = ($("test-url") as HTMLSelectElement).value.trim();
  const custom = ($("test-url-custom") as HTMLInputElement).value.trim();
  return custom || sel;
}

function updateJudgeUI(): void {
  const j = isJudgeMode();
  for (const id of ["deep-checks", "deep-hint", "precheck-row", "precheck-hint"]) {
    document.getElementById(id)?.classList.toggle("hidden", j);
  }
  document.getElementById("judge-mode-row")?.classList.toggle("hidden", !j);
  ($("test-url-custom") as HTMLInputElement).placeholder = t(j ? "custom_judge_ph" : "custom_ph");
}

function setJudgeFull(full: boolean): void {
  judgeFull = full;
  ($("judge-fast") as HTMLButtonElement).classList.toggle("active", !full);
  ($("judge-full") as HTMLButtonElement).classList.toggle("active", full);
}

// ---------- settings profiles ----------

type ProfileData = {
  testUrlSel: string;
  customUrl: string;
  timeout: string;
  concurrency: string;
  repeats: string;
  defaultProto: string;
  geo: boolean;
  anon: boolean;
  tamper: boolean;
  tls: boolean;
  precheck: boolean;
  precheckTimeout: string;
  judgeFull: boolean;
};

const BUILTIN_PROFILES: { id: string; nameKey: string; data: ProfileData }[] = [
  {
    id: "builtin:fast", nameKey: "prof_fast",
    data: {
      testUrlSel: "https://www.google.com/generate_204", customUrl: "",
      timeout: "5000", concurrency: "100", repeats: "1", defaultProto: "http",
      geo: false, anon: false, tamper: false, tls: false,
      precheck: true, precheckTimeout: "1500", judgeFull: false,
    },
  },
  {
    id: "builtin:deep", nameKey: "prof_deep",
    data: {
      testUrlSel: "https://ip-api.com/json", customUrl: "",
      timeout: "8000", concurrency: "50", repeats: "3", defaultProto: "http",
      geo: true, anon: true, tamper: true, tls: true,
      precheck: true, precheckTimeout: "1500", judgeFull: false,
    },
  },
  {
    id: "builtin:judge", nameKey: "prof_judge",
    data: {
      testUrlSel: "judge", customUrl: "",
      timeout: "5000", concurrency: "50", repeats: "1", defaultProto: "http",
      geo: false, anon: false, tamper: false, tls: false,
      precheck: true, precheckTimeout: "1500", judgeFull: true,
    },
  },
];

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
}

function loadProfiles(): void {
  try {
    const raw = localStorage.getItem("pp-profiles");
    const arr = raw ? JSON.parse(raw) : null;
    customProfiles = Array.isArray(arr)
      ? arr.filter((p) => p && typeof p.name === "string" && p.data && typeof p.data === "object").slice(0, 20)
      : [];
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

function setNet(state: "idle" | "work" | "done", text: string) {
  const dot = $("net-dot");
  dot.className = "dot " + state;
  $("net-text").textContent = text;
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
}

function updateCounts() {
  const alive = results.filter((r) => r.alive).length;
  const dead = results.length - alive;
  const lat = results
    .filter((r) => r.alive && r.latency_ms != null)
    .map((r) => r.latency_ms as number);
  const avg = lat.length ? Math.round(lat.reduce((a, b) => a + b, 0) / lat.length) : null;

  ($("st-total") as HTMLElement).textContent = String(results.length);
  ($("st-alive") as HTMLElement).textContent = String(alive);
  ($("st-dead") as HTMLElement).textContent = String(dead);
  ($("st-ping") as HTMLElement).textContent = avg == null ? "—" : `${avg} ms`;
  ($("f-all") as HTMLElement).textContent = String(results.length);
  ($("f-alive") as HTMLElement).textContent = String(alive);
  ($("f-dead") as HTMLElement).textContent = String(dead);
  ($("foot-info") as HTMLElement).textContent =
    results.length === 0 ? "" : `alive ${alive}/${results.length}`;
  ($("pool-count") as HTMLElement).textContent = String(dispatchPool.length);
  const apc = document.getElementById("auto-pool-count");
  if (apc) apc.textContent = String(dispatchPool.length);
}

function pingClass(ms: number | null): string {
  if (ms == null) return "";
  if (ms < 800) return "ping-fast";
  if (ms < 2500) return "ping-mid";
  return "ping-slow";
}

function anonShort(a: string | null): { txt: string; cls: string } {
  if (!a) return { txt: "—", cls: "" };
  if (a === "elite") return { txt: "elite", cls: "anon-elite" };
  if (a === "anonymous") return { txt: "anon", cls: "anon-anon" };
  if (a === "transparent") return { txt: "transp", cls: "anon-trans" };
  return { txt: a, cls: "" };
}

function tlsShort(v: string | null): { txt: string; cls: string } {
  if (v === "ok") return { txt: "ok", cls: "anon-elite" };
  if (v === "modified") return { txt: "mod", cls: "anon-trans" };
  return { txt: "—", cls: "" };
}

function geoShort(r: ProxyResult): string {
  if (r.country_code && r.city) return `${r.country_code} ${r.city}`;
  if (r.country_code) return r.country_code;
  if (r.country) return r.country;
  return "—";
}

function stabilityShort(r: ProxyResult): string {
  if (!r.alive) return "—";
  if ((r.attempts ?? 1) <= 1) return "—";
  const j = r.jitter_ms == null ? "?" : `±${r.jitter_ms}`;
  const sr = `${Math.round((r.success_rate ?? 1) * 100)}%`;
  return `${j} ${sr}`;
}

function rowTitle(r: ProxyResult): string {
  const parts: string[] = [];
  if (r.ip) parts.push(`IP ${r.ip}`);
  if (r.country || r.city) parts.push(`geo ${[r.country, r.city].filter(Boolean).join(", ")}`);
  if (r.isp) parts.push(`isp ${r.isp}`);
  if (r.org) parts.push(`org ${r.org}`);
  if (r.asn) parts.push(r.asn);
  if (r.ip_type) parts.push(`type ${r.ip_type}`);
  if (r.anonymity) parts.push(`anon ${r.anonymity}`);
  if (r.tamper) parts.push(`tamper ${r.tamper}`);
  if (r.tls) parts.push(`tls ${r.tls}${r.tls_info ? ` (${r.tls_info})` : ""}`);
  else if (r.tls_info) parts.push(`tls ? (${r.tls_info})`);
  if (r.error) parts.push(r.error);
  return parts.join(" | ");
}

function render() {
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
    const b = document.createElement("button");
    b.className = "btn ghost";
    b.textContent = t("show_more", { n: rows.length - renderLimit });
    b.addEventListener("click", () => {
      renderLimit += RENDER_STEP;
      render();
    });
    td.appendChild(b);
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

function renderPoolInto(tbodyId: string) {
  const pb = document.getElementById(tbodyId) as HTMLTableSectionElement | null;
  if (!pb) return;
  pb.innerHTML = "";
  const rows = getPoolView(tbodyId);
  if (rows.length === 0) {
    const tr = document.createElement("tr");
    tr.className = "empty";
    const td = document.createElement("td");
    td.colSpan = tbodyId === "pool-body" ? 4 : 3;
    td.textContent = t(dispatchPool.length === 0 ? "pool_empty" : "empty_filter");
    tr.appendChild(td);
    pb.appendChild(tr);
    return;
  }
  rows.forEach((p, n) => {
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
}

function renderPool() {
  renderPoolInto("pool-body");
  renderPoolInto("auto-pool-body");
}

function poolCountry(r: ProxyResult): string | null {
  return r.country_code ?? r.country ?? null;
}

/** merge: keep existing entries, add new ones, refresh latency.
 *  Entries over maxPingMs never enter the pool; re-checked entries
 *  that went over the limit are dropped. */
function mergePool(items: { raw: string; latency: number | null; country?: string | null }[]) {
  const idx = new Map(dispatchPool.map((p, i) => [p.raw, i]));
  const drop = new Set<number>();
  for (const it of items) {
    const at = idx.get(it.raw);
    if (!passPing(it.latency)) {
      if (at != null) drop.add(at);
      continue;
    }
    if (at == null) {
      idx.set(it.raw, dispatchPool.length);
      dispatchPool.push({ raw: it.raw, latency: it.latency, country: it.country ?? null });
    } else {
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

type GeoFields = {
  ip: string | null;
  country: string | null;
  country_code: string | null;
  city: string | null;
  isp: string | null;
  org: string | null;
  asn: string | null;
  ip_type: string | null;
};

let geoCache = new Map<string, GeoFields>();
const GEO_CACHE_MAX = 2000;

function loadGeoCache(): void {
  try {
    const raw = localStorage.getItem("pp-geocache");
    if (!raw) return;
    const arr = JSON.parse(raw) as [string, GeoFields][];
    for (const [k, v] of arr.slice(-GEO_CACHE_MAX)) {
      if (typeof k === "string" && v && typeof v === "object") geoCache.set(k, v);
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
    geoCache.set(r.proxy, {
      ip: r.ip, country: r.country, country_code: r.country_code, city: r.city,
      isp: r.isp, org: r.org, asn: r.asn, ip_type: r.ip_type,
    });
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

function relevantGeoCache(list: string[]): (GeoFields & { proxy: string })[] {
  const out: (GeoFields & { proxy: string })[] = [];
  for (const p of list) {
    const f = geoCache.get(p);
    if (f) out.push({ proxy: p, ...f });
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
  };
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
    out.sort((a, b) => Number(b.alive) - Number(a.alive) || (a.latency_ms ?? 1e12) - (b.latency_ms ?? 1e12));
    onLive?.([...out]);
    render();
    updateCounts();
    setProgress(done, total);
  };
  const CHUNK = 120;
  for (let i = 0; i < list.length; i += CHUNK) {
    if (stopFlag) break;
    const chunk = list.slice(i, i + CHUNK);
    const baseCount = out.length;
    const channel = new Channel<ProxyResult>();
    channel.onmessage = (r) => {
      pushLive(r);
      paint(i + out.length - baseCount, list.length);
    };
    try {
      const part = await invoke<ProxyResult[]>("check_proxies", {
        proxies: chunk,
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
        geoCache: relevantGeoCache(list),
        onResult: channel,
      });
      for (const r of part) pushLive(r);
    } catch (e) {
      const msg = String(e);
      for (const p of chunk) pushLive(deadStub(p, msg));
    }
    onProgress?.(Math.min(list.length, i + CHUNK), list.length);
  }
  out.sort((a, b) => Number(b.alive) - Number(a.alive) || (a.latency_ms ?? 1e12) - (b.latency_ms ?? 1e12));
  return out;
}

function escapeHtml(s: string): string {
  return s.replace(/[&<>"']/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" }[c] as string));
}

function setProgress(done: number, total: number) {
  const p = total === 0 ? 0 : Math.round((done / total) * 100);
  ($("progress") as HTMLElement).style.width = `${p}%`;
}

// ---------- alive ping history (sparkline) ----------

type PingPoint = { t: number; avg: number | null; alive: number };
let pingHist: PingPoint[] = [];
const PING_HIST_MAX = 120;

function loadPingHist(): void {
  try {
    const raw = localStorage.getItem("pp-pinghist");
    const arr = raw ? JSON.parse(raw) : null;
    if (Array.isArray(arr)) {
      pingHist = arr
        .filter((p) => p && typeof p.t === "number")
        .map((p) => ({ t: p.t, avg: typeof p.avg === "number" ? p.avg : null, alive: Number(p.alive) || 0 }))
        .slice(-PING_HIST_MAX);
    }
  } catch { /* ignore */ }
}

function pushPingPoint(): void {
  const alive = results.filter((r) => r.alive);
  const lat = alive
    .map((r) => r.latency_ms)
    .filter((x): x is number => x != null);
  pingHist.push({
    t: Date.now(),
    avg: lat.length ? Math.round(lat.reduce((a, b) => a + b, 0) / lat.length) : null,
    alive: alive.length,
  });
  if (pingHist.length > PING_HIST_MAX) pingHist.splice(0, pingHist.length - PING_HIST_MAX);
  try {
    localStorage.setItem("pp-pinghist", JSON.stringify(pingHist));
  } catch { /* ignore */ }
  renderPingGraph();
}

function renderPingGraph(): void {
  const svg = document.getElementById("ping-graph") as unknown as SVGSVGElement | null;
  if (!svg) return;
  const pts = pingHist.filter((p) => p.avg != null);
  if (pts.length < 2) {
    svg.innerHTML = "";
    return;
  }
  const vals = pts.map((p) => p.avg as number);
  const min = Math.min(...vals);
  const span = Math.max(1, Math.max(...vals) - min);
  const n = pts.length;
  const d = pts
    .map((p, i) => `${((i / (n - 1)) * 100).toFixed(1)},${(30 - (((p.avg as number) - min) / span) * 26).toFixed(1)}`)
    .join(" ");
  const last = pts[pts.length - 1];
  svg.innerHTML =
    `<title>${last.avg} ms · alive ${last.alive} · ${new Date(last.t).toLocaleTimeString(lang === "ru" ? "ru-RU" : "en-US")}</title>` +
    `<polyline points="${d}" />`;
}

function deadStub(proxy: string, msg: string): ProxyResult {
  return { proxy, proto: "ERR", alive: false, latency_ms: null, jitter_ms: null, success_rate: 0, attempts: 0, ip: null, country: null, country_code: null, city: null, isp: null, org: null, asn: null, ip_type: null, anonymity: null, tamper: null, tls: null, tls_info: null, error: msg.slice(0, 120) };
}

async function syncPool(notify: boolean): Promise<number> {
  try {
    const n = await invoke<number>("set_dispatch_pool", {
      items: dispatchPool.map((p) => ({ raw: p.raw, latency: p.latency })),
    });
    if (notify) setStatusT("pool_synced", { n });
    return n;
  } catch {
    return 0;
  }
}

/** full re-check of the shared pool; dead are pruned. if sendHook, POST pool to webhook after. */
async function autoCycle(statusEl: string, sendHook: boolean): Promise<void> {
  const el = document.getElementById(statusEl) as HTMLElement | null;
  if (running || autoBusy) {
    if (el) el.textContent = t("auto_skip");
    return;
  }
  if (dispatchPool.length === 0) {
    if (el) el.textContent = t("pool_empty");
    return;
  }
  autoBusy = true;
  stopFlag = false;
  try {
    if (el) el.textContent = t("auto_running", { n: dispatchPool.length });
    const res = await runCheckList(dispatchPool.map((p) => p.raw));
    const alive = res.filter((r) => r.alive && passPing(r.latency_ms));
    learnGeo(res);
    dispatchPool = alive.map((r) => ({ raw: r.proxy, latency: r.latency_ms, country: poolCountry(r) }));
    renderPool();
    updateCounts();
    await syncPool(false);
    pushPingPoint();
    if (el) el.textContent = `${t("auto_pruned", { alive: alive.length, total: res.length })} ${new Date().toLocaleTimeString(lang === "ru" ? "ru-RU" : "en-US")}`;
    if (sendHook) await sendWebhook(false);
  } finally {
    autoBusy = false;
  }
}

let dispSched = { ms: 0, next: 0 };
let autoSched = { ms: 0, next: 0 };

function armTimers() {
  window.clearInterval(dispAutoTimer);
  window.clearInterval(autoTimer);
  dispSched = { ms: 0, next: 0 };
  autoSched = { ms: 0, next: 0 };
  if (dispAutoOn) {
    const mins = Math.max(1, Math.min(1440, Number(($("disp-auto-min") as HTMLInputElement).value) || 30));
    const ms = mins * 60_000;
    dispSched = { ms, next: Date.now() + ms };
    dispAutoTimer = window.setInterval(() => {
      dispSched.next = Date.now() + dispSched.ms;
      void autoCycle("disp-auto-status", false);
    }, ms);
  }
  if (autoOn) {
    const mins = Math.max(1, Math.min(1440, Number(($("auto-min") as HTMLInputElement).value) || 30));
    const ms = mins * 60_000;
    autoSched = { ms, next: Date.now() + ms };
    autoTimer = window.setInterval(() => {
      autoSched.next = Date.now() + autoSched.ms;
      void autoCycle("auto-status", true);
    }, ms);
  }
  void updateTray();
}

async function updateTray(): Promise<void> {
  try {
    const now = Date.now();
    const d = dispAutoOn && dispSched.next > now ? Math.round((dispSched.next - now) / 1000) : null;
    const a = autoOn && autoSched.next > now ? Math.round((autoSched.next - now) / 1000) : null;
    const urls = getWebhookUrls();
    await invoke("update_tray_status", {
      dispNext: d,
      autoNext: a,
      webhookOn: autoOn && urls.length > 0,
      webhookUrl: urls[0] ?? "",
    });
  } catch { /* tray may be unavailable */ }
}

let webhookUrls: string[] = [""];

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

async function sendWebhook(manual: boolean): Promise<void> {
  const urls = getWebhookUrls();
  const statusEl = $("webhook-status") as HTMLElement;
  if (urls.length === 0) {
    if (manual) statusEl.textContent = t("webhook_hint");
    return;
  }
  if (dispatchPool.length === 0) {
    statusEl.textContent = t("webhook_empty");
    return;
  }
  const format = ($("webhook-format") as HTMLSelectElement).value;
  const items = dispatchPool.map((p) => ({ raw: p.raw, latency: p.latency }));
  statusEl.textContent = "...";
  const lines: string[] = [];
  for (let i = 0; i < urls.length; i++) {
    try {
      const r = await invoke<string>("send_webhook", { url: urls[i], format, items });
      lines.push(`#${i + 1} ${t("webhook_sent", { r })}`);
    } catch (e) {
      lines.push(`#${i + 1} ${t("webhook_fail", { e: String(e).slice(0, 120) })}`);
    }
  }
  statusEl.textContent = lines.join(" | ");
}

async function start() {
  if (running || autoBusy) return;
  const list = parseInput(($("proxy-input") as HTMLTextAreaElement).value);
  if (list.length === 0) {
    setStatusT("st_empty");
    return;
  }
  if (!/^https?:\/\//i.test(effectiveTestUrl())) {
    setStatusT("st_badurl");
    return;
  }

  running = true;
  stopFlag = false;
  results = [];
  renderLimit = RENDER_STEP;
  render();
  updateCounts();
  ($("btn-start") as HTMLButtonElement).disabled = true;
  ($("btn-stop") as HTMLButtonElement).disabled = false;
  setNet("work", t("net_work", { s: 0 }));
  setProgress(0, list.length);
  const t0 = performance.now();

  try {
    results = await runCheckList(list, (done, total) => {
      lastStatus = { key: "st_checking", params: { done, total } };
      $("status-line").textContent = t("st_checking", lastStatus.params);
      const secs = Math.floor((performance.now() - t0) / 1000);
      setNet("work", t("net_work", { s: secs }));
      render();
      updateCounts();
      setProgress(done, total);
    }, (live) => {
      results = live;
    });
  } catch {
    results = [];
  }
  learnGeo(results);
  render();
  updateCounts();

  const secs = ((performance.now() - t0) / 1000).toFixed(1);
  const alive = results.filter((r) => r.alive).length;
  if (stopFlag) setStatusT("st_stopped", { alive, total: results.length });
  else setStatusT("st_done", { secs, alive, total: results.length });
  setNet("done", stopFlag ? t("net_stopped") : t("net_done"));
  ($("btn-start") as HTMLButtonElement).disabled = false;
  ($("btn-stop") as HTMLButtonElement).disabled = true;
  running = false;

  // merge alive into dispatcher pool (existing entries kept)
  mergePool(results.filter((r) => r.alive).map((r) => ({ raw: r.proxy, latency: r.latency_ms, country: poolCountry(r) })));
  renderPool();
  updateCounts();
  await syncPool(false);
  pushPingPoint();
}

// ---------- export ----------

function exportScope(): ProxyResult[] {
  return includeDead ? results : results.filter((r) => r.alive);
}

function csvCell(v: unknown): string {
  const s = v == null ? "" : String(v);
  return /[",\n]/.test(s) ? `"${s.replace(/"/g, '""')}"` : s;
}

function buildTxt(list: ProxyResult[]): string {
  return list.map((r) => r.proxy).join("\n");
}

function buildJson(list: ProxyResult[]): string {
  return JSON.stringify(list, null, 2);
}

function buildCsv(list: ProxyResult[]): string {
  const head = ["proxy", "proto", "alive", "latency_ms", "jitter_ms", "success_rate", "attempts", "ip", "country_code", "country", "city", "isp", "org", "asn", "ip_type", "anonymity", "tamper", "tls", "tls_info", "error"];
  const lines = [head.join(",")];
  for (const r of list) {
    lines.push([
      r.proxy, r.proto, r.alive ? 1 : 0, r.latency_ms ?? "", r.jitter_ms ?? "",
      Math.round((r.success_rate ?? 0) * 100), r.attempts ?? "", r.ip ?? "",
      r.country_code ?? "", r.country ?? "", r.city ?? "", r.isp ?? "", r.org ?? "",
      r.asn ?? "", r.ip_type ?? "", r.anonymity ?? "", r.tamper ?? "",
      r.tls ?? "", r.tls_info ?? "", r.error ?? "",
    ].map(csvCell).join(","));
  }
  return lines.join("\n");
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
  try {
    const { save } = await import("@tauri-apps/plugin-dialog");
    const path = await save({ defaultPath: suggested, filters });
    if (!path) return;
    await invoke("write_text_file", { path, contents: text });
    lastStatus = { key: "st_saved", params: { path: String(path) } };
    $("status-line").textContent = t("st_saved", { path: String(path) });
  } catch {
    browserDownload(suggested, text);
    lastStatus = { key: "st_downloaded", params: { name: suggested } };
    $("status-line").textContent = t("st_downloaded", { name: suggested });
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
    const pill = $("srv-pill");
    pill.textContent = s.running ? "ON" : "OFF";
    pill.className = s.running ? "pill alive on" : "pill dead";
    ($("btn-srv-start") as HTMLButtonElement).disabled = s.running;
    ($("btn-srv-stop") as HTMLButtonElement).disabled = !s.running;
    if (s.last_error && !s.running) {
      ($("srv-status") as HTMLElement).textContent = s.last_error;
    }
  } catch { /* ignore */ }
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
  errRing.push({ t: new Date().toISOString(), msg: msg.slice(0, 300) });
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
  logErr("promise: " + String((e as PromiseRejectionEvent).reason ?? "rejection").slice(0, 300))
);

window.addEventListener("DOMContentLoaded", () => {
  // lang
  document.querySelectorAll(".lang button").forEach((b) => {
    b.addEventListener("click", () => {
      lang = (b as HTMLElement).dataset.lang as Lang;
      localStorage.setItem("pp-lang", lang);
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
      if (name === "dispatch") {
        refreshSrvAddr();
        renderPool();
        void pollDispatch();
      }
    });
  });

  const input = $("proxy-input") as HTMLTextAreaElement;
  const upd = () => { ($("src-count") as HTMLElement).textContent = String(parseInput(input.value).length); };
  let updT: number | undefined;
  input.addEventListener("input", () => { window.clearTimeout(updT); updT = window.setTimeout(upd, 250); });
  upd();

  $("btn-start").addEventListener("click", () => void start());
  $("btn-stop").addEventListener("click", () => {
    stopFlag = true;
    lastStatus = { key: "st_stopping" };
    $("status-line").textContent = t("st_stopping");
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
      const found = extractProxiesFromText(txt);
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
    mergePool(results.filter((r) => r.alive).map((r) => ({ raw: r.proxy, latency: r.latency_ms, country: poolCountry(r) })));
    renderPool();
    updateCounts();
    const n = await syncPool(false);
    const el = document.getElementById(statusElId);
    if (el) el.textContent = t("pool_synced", { n });
    lastStatus = { key: "pool_synced", params: { n } };
  }

  function doPoolClear(): void {
    dispatchPool = [];
    void syncPool(false);
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
      try {
        const txt = await f.text();
        const found = extractProxiesFromText(txt);
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
    ($("direct-res") as HTMLElement).textContent = "...";
    try {
      const ms = await invoke<number>("check_direct", { testUrl: url, timeoutMs: 10000 });
      ($("direct-res") as HTMLElement).textContent = t("direct_ok", { ms });
    } catch (e) {
      ($("direct-res") as HTMLElement).textContent = t("direct_fail", { e: String(e).slice(0, 80) });
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
  ($("btn-profile-save") as HTMLButtonElement).addEventListener("click", () => {
    const res = window.prompt(t("profile_name"));
    if (res == null) return;
    const name = res.trim();
    if (!name) return;
    const data = snapshotProfile();
    const at = customProfiles.findIndex((c) => c.name === name);
    if (at >= 0) customProfiles[at] = { name, data };
    else customProfiles.push({ name, data });
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
  ($("judge-fast") as HTMLButtonElement).addEventListener("click", () => setJudgeFull(false));
  ($("judge-full") as HTMLButtonElement).addEventListener("click", () => setJudgeFull(true));

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
    ($("srv-status") as HTMLElement).textContent = "...";
    try {
      if (dispatchPool.length === 0) {
        mergePool(results.filter((r) => r.alive).map((r) => ({ raw: r.proxy, latency: r.latency_ms, country: poolCountry(r) })));
        renderPool();
        updateCounts();
      }
      await syncPool(false);
      const addr = await invoke<string>("start_local_proxy", { port, mode, listener });
      ($("srv-status") as HTMLElement).textContent = t("srv_on", { addr });
      try {
        const st = await invoke<DispatchStatus>("local_proxy_status");
        ($("srv-port") as HTMLInputElement).value = String(st.port);
      } catch { /* keep typed value */ }
      refreshSrvAddr();
      void pollDispatch();
      void updateTray();
    } catch (e) {
      const msg = String(e);
      ($("srv-status") as HTMLElement).textContent = msg.includes("pool is empty")
        ? t("srv_err_empty")
        : t("srv_err", { e: msg.slice(0, 120) });
    }
  });
  ($("btn-srv-stop") as HTMLButtonElement).addEventListener("click", async () => {
    try {
      await invoke("stop_local_proxy");
    } catch { /* ignore */ }
    ($("srv-status") as HTMLElement).textContent = t("srv_off");
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
  ($("btn-check-upd") as HTMLButtonElement).addEventListener("click", async () => {
    const el = $("upd-status") as HTMLElement;
    const openBtn = $("btn-open-rel") as HTMLButtonElement;
    const instBtn = $("btn-install-upd") as HTMLButtonElement;
    openBtn.classList.add("hidden");
    instBtn.classList.add("hidden");
    pendingVersion = "";
    el.textContent = "...";
    try {
      const u = await invoke<UpdateInfo>("check_app_update");
      ($("app-ver") as HTMLElement).textContent = u.current;
      if (!u.available || !u.version) {
        el.textContent = t("upd_latest", { v: u.current });
        return;
      }
      pendingVersion = u.version;
      releaseUrl = "https://github.com/nechel12/proxpulse/releases";
      el.textContent = t("upd_found", { v: u.version, cur: u.current });
      openBtn.classList.remove("hidden");
      instBtn.classList.remove("hidden");
    } catch (e) {
      logErr("update-check: " + String(e).slice(0, 200));
      el.textContent = t("upd_fail");
    }
  });
  ($("btn-install-upd") as HTMLButtonElement).addEventListener("click", async () => {
    if (!pendingVersion) return;
    const el = $("upd-status") as HTMLElement;
    try {
      const { confirm } = await import("@tauri-apps/plugin-dialog");
      const ok = await confirm(t("upd_confirm", { v: pendingVersion }), { title: "ProxPulse", kind: "info" });
      if (!ok) return;
      el.textContent = t("upd_installing");
      await invoke("install_app_update");
      el.textContent = t("upd_restart");
    } catch (e) {
      logErr("update-install: " + String(e).slice(0, 200));
      el.textContent = t("upd_fail");
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
    el.addEventListener("input", renderPool);
  });
  document.querySelectorAll<HTMLSelectElement>("[data-pool-sort]").forEach((el) => {
    el.addEventListener("change", renderPool);
  });
  loadPingHist();
  renderPingGraph();
  for (const id of ["disp-max-ping", "auto-max-ping"]) {
    document.getElementById(id)?.addEventListener("change", (e) => {
      const v = Math.max(0, Math.floor(Number((e.target as HTMLInputElement).value) || 0));
      maxPingMs = Number.isFinite(v) ? v : 0;
      try {
        localStorage.setItem("pp-maxping", String(maxPingMs));
      } catch { /* ignore */ }
      syncMaxPingInputs();
    });
  }
  loadWebhooks();
  renderWebhookList();
  ($("btn-webhook-add") as HTMLButtonElement).addEventListener("click", () => {
    if (webhookUrls.length >= 10) return;
    webhookUrls.push("");
    saveWebhooks();
    renderWebhookList();
    const inputs = document.querySelectorAll<HTMLInputElement>("#webhook-list input");
    inputs[inputs.length - 1]?.focus();
  });
  ($("btn-auto-now") as HTMLButtonElement).addEventListener("click", () => void autoCycle("auto-status", true));
  ($("btn-webhook-send") as HTMLButtonElement).addEventListener("click", () => void sendWebhook(true));
  window.setInterval(() => void updateTray(), 5000);

  applyI18n();
  armTimers();
});
