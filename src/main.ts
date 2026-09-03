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
  error: string | null;
};

const $ = <T extends HTMLElement = HTMLElement>(id: string) =>
  document.getElementById(id) as T;

let results: ProxyResult[] = [];
let filter: "all" | "alive" | "dead" = "all";
let stopFlag = false;
let running = false;

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
    // common list wrappers
    for (const k of ["proxies", "proxy", "data", "list", "items", "results", "rows"]) {
      if (Array.isArray(o[k])) {
        for (const el of o[k] as unknown[]) proxiesFromJson(el, out);
        return;
      }
    }
    // fallback: scan all values
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
  // split by , ; | tab (keep : and @ intact)
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
    if (isPort(parts[1])) return `${parts[2]}:${parts[3]}@${parts[0]}:${parts[1]}`; // host,port,user,pass
    if (isPort(parts[3])) return `${parts[0]}:${parts[1]}@${parts[2]}:${parts[3]}`; // user,pass,host,port
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

  // 1) whole-text JSON
  try {
    const j = JSON.parse(text);
    const tmp: string[] = [];
    proxiesFromJson(j, tmp);
    if (tmp.length > 0) {
      tmp.forEach(push);
      return out;
    }
  } catch { /* not json */ }

  // 2) line based
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
      // line may also contain trailing user:pass part? push remainder check below
      continue;
    }
    // strip inline comments
    const hashIdx = line.indexOf(" #");
    if (hashIdx > 0) line = line.slice(0, hashIdx).trim();
    push(line);
  }

  // 3) global scan fallback: catch ip:port buried in logs/tables
  // skip candidates already covered by a longer entry (e.g. bare host:port inside user:pass@host:port)
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

function setStatus(t: string) {
  $("status-line").textContent = t;
}

function setNet(state: "idle" | "work" | "done", text: string) {
  const dot = $("net-dot");
  dot.className = "dot " + state;
  $("net-text").textContent = text;
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
}

function pingClass(ms: number | null): string {
  if (ms == null) return "";
  if (ms < 800) return "ping-fast";
  if (ms < 2500) return "ping-mid";
  return "ping-slow";
}

function anonShort(a: string | null): { t: string; c: string } {
  if (!a) return { t: "—", c: "" };
  if (a === "elite") return { t: "elite", c: "anon-elite" };
  if (a === "anonymous") return { t: "anon", c: "anon-anon" };
  if (a === "transparent") return { t: "transp", c: "anon-trans" };
  return { t: a, c: "" };
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
  if (r.error) parts.push(r.error);
  return parts.join(" | ");
}

function render() {
  const q = (($("search") as HTMLInputElement).value || "").toLowerCase().trim();
  const tb = $("tbody") as HTMLTableSectionElement;
  tb.innerHTML = "";
  const rows = results.filter((r) => {
    if (filter === "alive" && !r.alive) return false;
    if (filter === "dead" && r.alive) return false;
    if (q) {
      const hay = [r.proxy, r.proto, r.error ?? "", r.ip ?? "", r.country ?? "", r.country_code ?? "", r.city ?? "", r.isp ?? "", r.org ?? "", r.asn ?? "", r.ip_type ?? "", r.anonymity ?? ""].join(" ").toLowerCase();
      if (!hay.includes(q)) return false;
    }
    return true;
  });
  if (rows.length === 0) {
    const tr = document.createElement("tr");
    tr.className = "empty";
    const td = document.createElement("td");
    td.colSpan = 10;
    td.textContent = results.length === 0 ? "Пока пусто — вставь список слева и жми Старт." : "Ничего не найдено под фильтр.";
    tr.appendChild(td);
    tb.appendChild(tr);
    return;
  }
  rows.forEach((r, i) => {
    const tr = document.createElement("tr");
    const pill = r.alive ? `<span class="pill alive">ALIVE</span>` : `<span class="pill dead">DEAD</span>`;
    const ping = r.latency_ms == null ? "—" : `${r.latency_ms} ms`;
    const an = anonShort(r.anonymity);
    const tamperMark = r.tamper === "modified" ? ` <span class="anon-trans" title="Трафик модифицируется">⚠mod</span>` : "";
    const info = r.alive ? `${escapeHtml(r.ip ?? "ok")}${tamperMark}` : escapeHtml(r.error ?? "fail");
    const infoTitle = escapeHtml(rowTitle(r));
    tr.innerHTML =
      `<td>${i + 1}</td><td>${escapeHtml(r.proxy)}</td><td>${escapeHtml(r.proto)}</td>` +
      `<td>${pill}</td><td class="${pingClass(r.latency_ms)}">${ping}</td>` +
      `<td>${escapeHtml(stabilityShort(r))}</td><td class="${an.c}">${escapeHtml(an.t)}</td>` +
      `<td class="cell-geo" title="${infoTitle}">${escapeHtml(geoShort(r))}</td>` +
      `<td>${escapeHtml(r.ip_type ?? "—")}</td>` +
      `<td class="cell-err" title="${infoTitle}">${info}</td>`;
    tb.appendChild(tr);
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
  return { proxy, proto: "ERR", alive: false, latency_ms: null, jitter_ms: null, success_rate: 0, attempts: 0, ip: null, country: null, country_code: null, city: null, isp: null, org: null, asn: null, ip_type: null, anonymity: null, tamper: null, error: msg.slice(0, 120) };
}

async function start() {
  if (running) return;
  const list = parseInput(($("proxy-input") as HTMLTextAreaElement).value);
  if (list.length === 0) {
    setStatus("Список пуст — вставь прокси.");
    return;
  }
  const testUrl = effectiveTestUrl();
  if (!/^https?:\/\//i.test(testUrl)) {
    setStatus("Тестовый URL должен начинаться с http(s)://");
    return;
  }
  const timeoutMs = Math.max(1000, Math.min(30000, Number(($("timeout") as HTMLInputElement).value) || 8000));
  const concurrency = Math.max(1, Math.min(500, Number(($("concurrency") as HTMLInputElement).value) || 50));
  const repeats = Math.max(1, Math.min(5, Number(($("repeats") as HTMLInputElement).value) || 3));
  const defaultProto = ($("default-proto") as HTMLSelectElement).value;
  const withGeo = ($("chk-geo") as HTMLInputElement).checked;
  const withAnonymity = ($("chk-anon") as HTMLInputElement).checked;
  const withTamper = ($("chk-tamper") as HTMLInputElement).checked;

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
    setStatus(`Проверка ${Math.min(i + CHUNK, list.length)}/${list.length} ...`);
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
  setStatus(stopFlag ? `Остановлено. Живых: ${alive}/${results.length}.` : `Готово за ${secs}с. Живых: ${alive}/${results.length}.`);
  setNet("done", stopFlag ? "stopped" : "done");
  ($("btn-start") as HTMLButtonElement).disabled = false;
  ($("btn-stop") as HTMLButtonElement).disabled = true;
  running = false;
}

// ---------- export ----------

function aliveList(): string[] {
  return results.filter((r) => r.alive).map((r) => r.proxy);
}

function csvCell(v: unknown): string {
  const s = v == null ? "" : String(v);
  return /[",\n]/.test(s) ? `"${s.replace(/"/g, '""')}"` : s;
}

function buildTxt(): string {
  return aliveList().join("\n");
}

function buildJson(): string {
  return JSON.stringify(results, null, 2);
}

function buildCsv(): string {
  const head = ["proxy", "proto", "alive", "latency_ms", "jitter_ms", "success_rate", "attempts", "ip", "country_code", "country", "city", "isp", "org", "asn", "ip_type", "anonymity", "tamper", "error"];
  const lines = [head.join(",")];
  for (const r of results) {
    lines.push([
      r.proxy, r.proto, r.alive ? 1 : 0, r.latency_ms ?? "", r.jitter_ms ?? "",
      Math.round((r.success_rate ?? 0) * 100), r.attempts ?? "", r.ip ?? "",
      r.country_code ?? "", r.country ?? "", r.city ?? "", r.isp ?? "", r.org ?? "",
      r.asn ?? "", r.ip_type ?? "", r.anonymity ?? "", r.tamper ?? "", r.error ?? "",
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
  // Tauri: диалог + запись через бэкенд; браузер: обычное скачивание
  try {
    const { save } = await import("@tauri-apps/plugin-dialog");
    const path = await save({ defaultPath: suggested, filters });
    if (!path) return; // отмена
    await invoke("write_text_file", { path, contents: text });
    setStatus(`Сохранено: ${path}`);
  } catch {
    browserDownload(suggested, text);
    setStatus(`Скачано: ${suggested} (браузерный режим).`);
  }
}

async function doExport(kind: "txt" | "json" | "csv" | "copy") {
  if (kind === "copy") {
    const t = buildTxt();
    if (!t) { setStatus("Нет живых для копирования."); return; }
    try {
      await navigator.clipboard.writeText(t);
      setStatus(`Скопировано живых: ${t.split("\n").length}.`);
    } catch {
      setStatus("Не удалось скопировать.");
    }
    return;
  }
  if (results.length === 0) { setStatus("Нечего экспортировать."); return; }
  if (kind === "txt") {
    const t = buildTxt();
    if (!t) { setStatus("Нет живых для экспорта."); return; }
    await saveViaDialog("proxpulse-alive.txt", t, [{ name: "Text", extensions: ["txt"] }]);
  } else if (kind === "json") {
    await saveViaDialog("proxpulse-results.json", buildJson(), [{ name: "JSON", extensions: ["json"] }]);
  } else {
    await saveViaDialog("proxpulse-results.csv", buildCsv(), [{ name: "CSV", extensions: ["csv"] }]);
  }
}

window.addEventListener("DOMContentLoaded", () => {
  const input = $("proxy-input") as HTMLTextAreaElement;
  const upd = () => { ($("src-count") as HTMLElement).textContent = String(parseInput(input.value).length); };
  let updT: number | undefined;
  input.addEventListener("input", () => { window.clearTimeout(updT); updT = window.setTimeout(upd, 250); });
  upd();

  $("btn-start").addEventListener("click", start);
  $("btn-stop").addEventListener("click", () => { stopFlag = true; setStatus("Останавливаю..."); });
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
  $("file-input").addEventListener("change", async (e) => {
    const files = (e.target as HTMLInputElement).files;
    if (!files || files.length === 0) return;
    const parts: string[] = [];
    for (const f of Array.from(files)) {
      try {
        const t = await f.text();
        const found = extractProxiesFromText(t);
        if (found.length > 0) parts.push(...found);
        else setStatus(`В ${f.name} прокси не найдены.`);
      } catch {
        setStatus(`Не удалось прочитать ${f.name}.`);
      }
    }
    if (parts.length > 0) {
      input.value = (input.value ? input.value.replace(/\s+$/, "") + "\n" : "") + parts.join("\n");
      upd();
      setStatus(`Импортировано: ${parts.length} из ${files.length} ф.`);
    }
    (e.target as HTMLInputElement).value = "";
  });

  // export dropdown
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

  $("btn-clear-out").addEventListener("click", () => {
    results = [];
    render(); updateCounts(); setProgress(0, 1);
    setStatus("Результат очищен.");
  });

  $("btn-direct").addEventListener("click", async () => {
    const url = effectiveTestUrl();
    ($("direct-res") as HTMLElement).textContent = "...";
    try {
      const ms = await invoke<number>("check_direct", { testUrl: url, timeoutMs: 10000 });
      ($("direct-res") as HTMLElement).textContent = `прямой доступ OK ${ms}ms`;
    } catch (e) {
      ($("direct-res") as HTMLElement).textContent = `связи нет: ${String(e).slice(0, 80)}`;
    }
  });
});
