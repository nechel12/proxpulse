import { invoke } from "@tauri-apps/api/core";

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
    set_title: "▸ Настройки", f_testurl: "Тестовый URL",
    opt_google: "google generate_204 (быстро)", opt_cf: "cloudflare generate_204 (быстро)",
    opt_ipify: "api.ipify.org (покажет IP)", opt_httpbin: "httpbin.org/ip (покажет IP)",
    opt_ipapi: "ip-api.com/json (IP + гео)", opt_example: "example.com",
    custom_ph: "или свой URL https://...",
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
    pool_hint: "Пул пополняется автоматически после каждой проверки.",
    pool_empty: "Пусто — запусти проверку.", pool_synced: "Пул: {n}.",
    srv_on: "Сервер на {addr}.", srv_off: "Сервер выключен.",
    srv_err_empty: "Пул пуст — сначала проверь прокси.", srv_err: "Ошибка сервера: {e}",
  },
  en: {
    stat_total: "Total", stat_alive: "Alive", stat_dead: "Dead", stat_ping: "Avg ping",
    tab_check: "Check", tab_dispatch: "Dispatcher",
    src_title: "▸ Source",
    src_ph: "Paste proxies in any format",
    btn_clear: "Clear", btn_import: "Import", btn_copy: "Copy",
    import_hint: "Import: txt, csv, json, log and anything else — formats are auto-detected.",
    set_title: "▸ Settings", f_testurl: "Test URL",
    opt_google: "google generate_204 (fast)", opt_cf: "cloudflare generate_204 (fast)",
    opt_ipify: "api.ipify.org (shows IP)", opt_httpbin: "httpbin.org/ip (shows IP)",
    opt_ipapi: "ip-api.com/json (IP + geo)", opt_example: "example.com",
    custom_ph: "or custom URL https://...",
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
    pool_hint: "Pool auto-fills after every check.",
    pool_empty: "Empty — run a check.", pool_synced: "Pool: {n}.",
    srv_on: "Server on {addr}.", srv_off: "Server off.",
    srv_err_empty: "Pool is empty — check proxies first.", srv_err: "Server error: {e}",
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
let stopFlag = false;
let running = false;
let includeDead = false;
let dispatchPool: { raw: string; latency: number | null }[] = [];
let pollTimer: number | undefined;

// ---------- proxy extraction (many formats) ----------

const PROTO_KEYS = ["protocol", "proto", "type", "scheme"];
const HOST_KEYS = ["host", "hostname", "ip", "server", "address", "addr"];
const PORT_KEYS = ["port", "port_number", "portnumber"];
const USER_KEYS = ["user", "username", "login", "usr", "name"];
const PASS_KEYS = ["pass", "password", "pwd", "passw", "passwd", "secret"];

function getKey(obj: Record<string, unknown>, keys: string[]): string | null {
  const lower: Record<string, unknown> = {};
  for (const k of Object.keys(obj)) lower[k.toLowerCase()] = obj[k];
  for (const k of keys) {
    const v = lower[k];
    if (v != null && String(v).trim() !== "") return String(v).trim();
  }
  return null;
}

function buildFromParts(host: string, port: string, user?: string | null, pass?: string | null, proto?: string | null): string | null {
  host = host.trim().replace(/\/+$/, "");
  port = port.trim();
  if (!host || !/^\d{1,5}$/.test(port)) return null;
  const p = Number(port);
  if (p < 1 || p > 65535) return null;
  let prefix = "";
  if (proto && proto.trim()) {
    const s = proto.trim().toLowerCase();
    prefix = s.includes("://") ? s : `${s}://`;
  }
  if (user && pass) return `${prefix}${user}:${pass}@${host}:${port}`;
  if (user) return `${prefix}${user}@${host}:${port}`;
  return `${prefix}${host}:${port}`;
}

function proxiesFromJson(v: unknown, out: string[]): void {
  if (v == null) return;
  if (typeof v === "string") {
    const s = v.trim();
    if (s.length > 2) out.push(s);
    return;
  }
  if (Array.isArray(v)) {
    for (const el of v) proxiesFromJson(el, out);
    return;
  }
  if (typeof v === "object") {
    const o = v as Record<string, unknown>;
    const host = getKey(o, HOST_KEYS);
    const port = getKey(o, PORT_KEYS);
    if (host && port) {
      const built = buildFromParts(host, port, getKey(o, USER_KEYS), getKey(o, PASS_KEYS), getKey(o, PROTO_KEYS));
      if (built) out.push(built);
    }
    for (const k of ["proxies", "proxy", "data", "list", "items", "results", "rows"]) {
      if (Array.isArray(o[k])) {
        for (const el of o[k] as unknown[]) proxiesFromJson(el, out);
        return;
      }
    }
    if (!host) {
      for (const val of Object.values(o)) {
        if (typeof val === "string" || Array.isArray(val) || (val && typeof val === "object")) {
          proxiesFromJson(val, out);
        }
      }
    }
  }
}

function tryCsvLine(line: string): string | null {
  let parts: string[] | null = null;
  if (/[,;|\t]/.test(line)) {
    parts = line.split(/[,;|\t]/).map((s) => s.trim().replace(/^["'`]+|["'`]+$/g, "")).filter(Boolean);
  } else if (!line.includes("://") && !line.includes("@")) {
    const ws = line.split(/\s+/).filter(Boolean);
    if (ws.length >= 2 && ws.length <= 5) parts = ws;
  }
  if (!parts || parts.length < 2 || parts.length > 5) return null;
  const isPort = (s: string) => /^\d{2,5}$/.test(s) && Number(s) >= 1 && Number(s) <= 65535;
  const isProto = (s: string) => /^(https?|socks5h?|socks4a?|socks)$/i.test(s.trim());
  if (parts.length === 2 && isPort(parts[1])) return `${parts[0]}:${parts[1]}`;
  if (parts.length === 3) {
    if (isPort(parts[1]) && isProto(parts[2])) return `${parts[2]}://${parts[0]}:${parts[1]}`;
    if (isProto(parts[0]) && isPort(parts[2])) return `${parts[0]}://${parts[1]}:${parts[2]}`;
    return null;
  }
  if (parts.length === 4) {
    if (isPort(parts[1])) return `${parts[2]}:${parts[3]}@${parts[0]}:${parts[1]}`;
    if (isPort(parts[3])) return `${parts[0]}:${parts[1]}@${parts[2]}:${parts[3]}`;
    return null;
  }
  if (parts.length === 5 && isProto(parts[0]) && isPort(parts[2])) {
    return `${parts[0]}://${parts[3]}:${parts[4]}@${parts[1]}:${parts[2]}`;
  }
  return null;
}

const SCHEME_URL_RE = /\b(?:socks5h?|socks4a?|socks|https?)\:\/\/[^\s"'<>,;()\[\]]+/gi;

function cleanTail(s: string): string {
  return s.replace(/[.,;!?)\]}>]+$/, "").replace(/^[(<{]+/, "").trim();
}

export function extractProxiesFromText(text: string): string[] {
  const out: string[] = [];
  const seen = new Set<string>();
  const push = (s: string) => {
    s = cleanTail(s.trim().replace(/^["'`]+|["'`]+$/g, ""));
    if (s.length < 3 || s.length > 512 || s.startsWith("#") || s.startsWith("//")) return;
    if (seen.has(s)) return;
    seen.add(s);
    out.push(s);
  };

  try {
    const j = JSON.parse(text);
    const tmp: string[] = [];
    proxiesFromJson(j, tmp);
    if (tmp.length > 0) {
      tmp.forEach(push);
      return out;
    }
  } catch { /* not json */ }

  const lines = text.split(/\r?\n/);
  for (let raw of lines) {
    let line = raw.trim();
    if (!line || line.startsWith("#") || line.startsWith("//")) continue;
    line = line.replace(/^(\d{1,6}[).\-:]\s+|[-*•>]\s+|\[\s*[xX✓]?\s*\]\s*)/, "").trim();
    if (!line) continue;

    const csv = tryCsvLine(line);
    if (csv) { push(csv); continue; }

    const urls = line.match(SCHEME_URL_RE);
    if (urls && urls.length > 0) {
      urls.forEach(push);
      continue;
    }
    const hashIdx = line.indexOf(" #");
    if (hashIdx > 0) line = line.slice(0, hashIdx).trim();
    push(line);
  }

  if (out.length < 5) {
    const found = text.match(/\b(?:\d{1,3}\.){3}\d{1,3}:\d{2,5}\b/g);
    if (found) {
      for (const f of found) {
        if (seen.has(f)) continue;
        if (out.some((s) => s.includes(f))) continue;
        push(f);
      }
    }
  }
  return out;
}

function parseInput(text: string): string[] {
  return extractProxiesFromText(text);
}

// ---------- ui helpers ----------

function effectiveTestUrl(): string {
  const sel = ($("test-url") as HTMLSelectElement).value.trim();
  const custom = ($("test-url-custom") as HTMLInputElement).value.trim();
  return custom || sel;
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
  updateCounts();
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
  rows.forEach((r, i) => {
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
}

function renderPool() {
  const pb = document.getElementById("pool-body") as HTMLTableSectionElement | null;
  if (!pb) return;
  pb.innerHTML = "";
  if (dispatchPool.length === 0) {
    const tr = document.createElement("tr");
    tr.className = "empty";
    const td = document.createElement("td");
    td.colSpan = 4;
    td.textContent = t("pool_empty");
    tr.appendChild(td);
    pb.appendChild(tr);
    return;
  }
  dispatchPool.forEach((p, i) => {
    const tr = document.createElement("tr");
    const ping = p.latency == null ? "—" : `${p.latency} ms`;
    tr.innerHTML = `<td>${i + 1}</td><td>${escapeHtml(p.raw)}</td><td class="${pingClass(p.latency)}">${ping}</td>`;
    const td = document.createElement("td");
    const b = document.createElement("button");
    b.className = "btn ghost";
    b.textContent = "✕";
    b.addEventListener("click", () => {
      dispatchPool.splice(i, 1);
      void syncPool(false);
      renderPool();
      updateCounts();
    });
    td.appendChild(b);
    tr.appendChild(td);
    pb.appendChild(tr);
  });
}

function escapeHtml(s: string): string {
  return s.replace(/[&<>"']/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" }[c] as string));
}

function setProgress(done: number, total: number) {
  const p = total === 0 ? 0 : Math.round((done / total) * 100);
  ($("progress") as HTMLElement).style.width = `${p}%`;
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

async function start() {
  if (running) return;
  const list = parseInput(($("proxy-input") as HTMLTextAreaElement).value);
  if (list.length === 0) {
    setStatusT("st_empty");
    return;
  }
  const testUrl = effectiveTestUrl();
  if (!/^https?:\/\//i.test(testUrl)) {
    setStatusT("st_badurl");
    return;
  }
  const timeoutMs = Math.max(1000, Math.min(30000, Number(($("timeout") as HTMLInputElement).value) || 8000));
  const concurrency = Math.max(1, Math.min(500, Number(($("concurrency") as HTMLInputElement).value) || 50));
  const repeats = Math.max(1, Math.min(5, Number(($("repeats") as HTMLInputElement).value) || 3));
  const defaultProto = ($("default-proto") as HTMLSelectElement).value;
  const withGeo = ($("chk-geo") as HTMLInputElement).checked;
  const withAnonymity = ($("chk-anon") as HTMLInputElement).checked;
  const withTamper = ($("chk-tamper") as HTMLInputElement).checked;
  const withTls = ($("chk-tls") as HTMLInputElement).checked;
  const precheck = ($("chk-precheck") as HTMLInputElement).checked;
  const precheckTimeoutMs = Math.max(300, Math.min(10000, Number(($("precheck-timeout") as HTMLInputElement).value) || 1500));

  running = true;
  stopFlag = false;
  results = [];
  render();
  updateCounts();
  ($("btn-start") as HTMLButtonElement).disabled = true;
  ($("btn-stop") as HTMLButtonElement).disabled = false;
  setNet("work", "checking");
  setProgress(0, list.length);

  const CHUNK = 120;
  let done = 0;
  const t0 = performance.now();

  for (let i = 0; i < list.length; i += CHUNK) {
    if (stopFlag) break;
    const chunk = list.slice(i, i + CHUNK);
    lastStatus = { key: "st_checking", params: { done: Math.min(i + CHUNK, list.length), total: list.length } };
    $("status-line").textContent = t("st_checking", lastStatus.params);
    try {
      const part = await invoke<ProxyResult[]>("check_proxies", {
        proxies: chunk,
        testUrl,
        timeoutMs,
        concurrency,
        defaultProto,
        repeats,
        withGeo,
        withAnonymity,
        withTamper,
        withTls,
        precheck,
        precheckTimeoutMs,
      });
      results.push(...part);
    } catch (e) {
      const msg = String(e);
      for (const p of chunk) results.push(deadStub(p, msg));
    }
    done = Math.min(list.length, i + CHUNK);
    results.sort((a, b) => Number(b.alive) - Number(a.alive) || (a.latency_ms ?? 1e12) - (b.latency_ms ?? 1e12));
    render();
    updateCounts();
    setProgress(done, list.length);
  }

  const secs = ((performance.now() - t0) / 1000).toFixed(1);
  const alive = results.filter((r) => r.alive).length;
  if (stopFlag) setStatusT("st_stopped", { alive, total: results.length });
  else setStatusT("st_done", { secs, alive, total: results.length });
  setNet("done", stopFlag ? "stopped" : "done");
  ($("btn-start") as HTMLButtonElement).disabled = false;
  ($("btn-stop") as HTMLButtonElement).disabled = true;
  running = false;

  // auto-export alive into dispatcher pool
  dispatchPool = results.filter((r) => r.alive).map((r) => ({ raw: r.proxy, latency: r.latency_ms }));
  renderPool();
  updateCounts();
  await syncPool(false);
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
  $("btn-stop").addEventListener("click", () => { stopFlag = true; lastStatus = { key: "st_stopping" }; $("status-line").textContent = t("st_stopping"); });
  $("search").addEventListener("input", render);
  document.querySelectorAll(".chip").forEach((c) => {
    c.addEventListener("click", () => {
      document.querySelectorAll(".chip").forEach((x) => x.classList.remove("active"));
      c.classList.add("active");
      filter = (c as HTMLElement).dataset.f as typeof filter;
      render();
    });
  });

  $("btn-clear-in").addEventListener("click", () => { input.value = ""; upd(); });
  ($("btn-pool-clear") as HTMLButtonElement).addEventListener("click", () => {
    dispatchPool = [];
    void syncPool(false);
    renderPool();
    updateCounts();
  });
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

  // github card
  $("gh-card").addEventListener("click", async () => {
    const url = "https://github.com/nechel12/proxpulse";
    try {
      const { openUrl } = await import("@tauri-apps/plugin-opener");
      await openUrl(url);
    } catch {
      window.open(url, "_blank");
    }
  });

  // dispatcher
  ($("srv-port") as HTMLInputElement).addEventListener("input", refreshSrvAddr);
  refreshSrvAddr();
  ($("btn-pool-sync") as HTMLButtonElement).addEventListener("click", async () => {
    dispatchPool = results.filter((r) => r.alive).map((r) => ({ raw: r.proxy, latency: r.latency_ms }));
    renderPool();
    updateCounts();
    const n = await syncPool(false);
    lastStatus = { key: "pool_synced", params: { n } };
    ($("srv-status") as HTMLElement).textContent = t("pool_synced", { n });
  });
  ($("btn-srv-copy") as HTMLButtonElement).addEventListener("click", async () => {
    try {
      await navigator.clipboard.writeText(($("srv-addr") as HTMLElement).textContent ?? "");
    } catch { /* ignore */ }
  });
  ($("btn-srv-start") as HTMLButtonElement).addEventListener("click", async () => {
    const port = Math.max(1, Math.min(65535, Number(($("srv-port") as HTMLInputElement).value) || 1080));
    const mode = ($("srv-mode") as HTMLSelectElement).value;
    ($("srv-status") as HTMLElement).textContent = "...";
    try {
      if (dispatchPool.length === 0) {
        dispatchPool = results.filter((r) => r.alive).map((r) => ({ raw: r.proxy, latency: r.latency_ms }));
        renderPool();
        updateCounts();
      }
      await syncPool(false);
      const addr = await invoke<string>("start_local_proxy", { port, mode });
      ($("srv-status") as HTMLElement).textContent = t("srv_on", { addr });
      refreshSrvAddr();
      void pollDispatch();
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
  });

  applyI18n();
});
