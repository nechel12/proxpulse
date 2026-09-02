import { invoke } from "@tauri-apps/api/core";

type ProxyResult = {
  proxy: string;
  proto: string;
  alive: boolean;
  latency_ms: number | null;
  ip: string | null;
  error: string | null;
};

const $ = <T extends HTMLElement = HTMLElement>(id: string) =>
  document.getElementById(id) as T;

let results: ProxyResult[] = [];
let filter: "all" | "alive" | "dead" = "all";
let stopFlag = false;
let running = false;

function parseInput(text: string): string[] {
  return text
    .split(/\r?\n/)
    .map((s) => s.trim())
    .filter((s) => s.length > 0 && !s.startsWith("#"));
}

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

function render() {
  const q = (($("search") as HTMLInputElement).value || "").toLowerCase().trim();
  const tb = $("tbody") as HTMLTableSectionElement;
  tb.innerHTML = "";
  const rows = results.filter((r) => {
    if (filter === "alive" && !r.alive) return false;
    if (filter === "dead" && r.alive) return false;
    if (q && !(r.proxy.toLowerCase().includes(q) || r.proto.toLowerCase().includes(q) || (r.error ?? "").toLowerCase().includes(q))) return false;
    return true;
  });
  if (rows.length === 0) {
    const tr = document.createElement("tr");
    tr.className = "empty";
    const td = document.createElement("td");
    td.colSpan = 6;
    td.textContent = results.length === 0 ? "Пока пусто — вставь список слева и жми Старт." : "Ничего не найдено под фильтр.";
    tr.appendChild(td);
    tb.appendChild(tr);
    return;
  }
  rows.forEach((r, i) => {
    const tr = document.createElement("tr");
    const pill = r.alive ? `<span class="pill alive">ALIVE</span>` : `<span class="pill dead">DEAD</span>`;
    const ping = r.latency_ms == null ? "—" : `${r.latency_ms} ms`;
    const info = r.alive ? (r.ip ?? "ok") : (r.error ?? "fail");
    tr.innerHTML =
      `<td>${i + 1}</td><td>${escapeHtml(r.proxy)}</td><td>${escapeHtml(r.proto)}</td>` +
      `<td>${pill}</td><td class="${pingClass(r.latency_ms)}">${ping}</td>` +
      `<td class="cell-err" title="${escapeHtml(info)}">${escapeHtml(info)}</td>`;
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
  const defaultProto = ($("default-proto") as HTMLSelectElement).value;

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
      });
      results.push(...part);
    } catch (e) {
      const msg = String(e);
      for (const p of chunk) {
        results.push({ proxy: p, proto: "ERR", alive: false, latency_ms: null, ip: null, error: msg.slice(0, 120) });
      }
    }
    done = Math.min(list.length, i + CHUNK);
    // alive first for display
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

function aliveText(): string {
  return results.filter((r) => r.alive).map((r) => r.proxy).join("\n");
}

window.addEventListener("DOMContentLoaded", () => {
  const input = $("proxy-input") as HTMLTextAreaElement;
  const upd = () => { ($("src-count") as HTMLElement).textContent = String(parseInput(input.value).length); };
  input.addEventListener("input", upd);
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

  $("btn-sample").addEventListener("click", () => {
    input.value = `# формат: host:port / user:pass@host:port / scheme://... / host:port:user:pass\n8.8.8.8:8080\n1.1.1.1:3128\nsocks5://127.0.0.1:1080\n127.0.0.1:8080:user:pass`;
    upd();
  });
  $("btn-clear-in").addEventListener("click", () => { input.value = ""; upd(); });
  $("file-input").addEventListener("change", async (e) => {
    const f = (e.target as HTMLInputElement).files?.[0];
    if (!f) return;
    const t = await f.text();
    input.value = (input.value ? input.value + "\n" : "") + t;
    upd();
    setStatus(`Загружен файл ${f.name}.`);
  });

  $("btn-copy").addEventListener("click", async () => {
    const t = aliveText();
    if (!t) { setStatus("Нет живых для копирования."); return; }
    await navigator.clipboard.writeText(t);
    setStatus(`Скопировано живых: ${t.split("\n").length}.`);
  });
  $("btn-export").addEventListener("click", () => {
    const t = aliveText();
    if (!t) { setStatus("Нет живых для экспорта."); return; }
    const blob = new Blob([t], { type: "text/plain" });
    const a = document.createElement("a");
    a.href = URL.createObjectURL(blob);
    a.download = "proxpulse-alive.txt";
    a.click();
    URL.revokeObjectURL(a.href);
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
