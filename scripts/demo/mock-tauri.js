// A stand-in for the Tauri backend: `npm run demo` serves the real frontend
// with it, for interface work without the Rust side and for the README
// screenshots. It answers the commands the page calls with sample data;
// every address is from the documentation ranges (RFC 5737).
//
// ?demo=check | progress | dispatch | auto picks a scene, ?lang=ru | en the
// language.
import pkg from "../../package.json";

const params = new URLSearchParams(location.search);
const demo = params.get("demo") ?? "";
const lang = params.get("lang");
if (lang) localStorage.setItem("pp-lang", lang);
if (params.get("theme")) localStorage.setItem("pp-theme", params.get("theme"));

// Earlier checks, so the ping graph has a line to draw.
localStorage.setItem("pp-pinghist", JSON.stringify(
  [262, 248, 255, 231, 240, 236, 219, 228, 244, 251, 233, 226, 218, 229, 241, 237, 224, 215, 222, 230, 246, 239, 227]
    .map((avg, i) => ({ t: Date.now() - (24 - i) * 1800000, avg, alive: 9 + (i % 4) })),
));

const callbacks = new Map();
let nextId = 1;
const listeners = new Map();

const now = Math.floor(Date.now() / 1000);
const GEO = [
  ["DE", "Germany", "Frankfurt am Main", "Example Hosting GmbH", "AS64500", "hosting"],
  ["NL", "Netherlands", "Amsterdam", "Example Cloud B.V.", "AS64501", "hosting"],
  ["US", "United States", "Ashburn", "Example Networks LLC", "AS64502", "hosting"],
  ["FR", "France", "Paris", "Exemple Telecom", "AS64503", "residential/isp"],
  ["FI", "Finland", "Helsinki", "Example Oy", "AS64504", "hosting"],
  ["PL", "Poland", "Warsaw", "Example Mobile", "AS64505", "mobile"],
  ["SG", "Singapore", "Singapore", "Example Pte Ltd", "AS64506", "hosting"],
  ["JP", "Japan", "Tokyo", "Example KK", "AS64507", "residential/isp"],
];
const LINES = [
  "socks5://203.0.113.14:1080", "http://198.51.100.23:3128", "203.0.113.77:8080",
  "socks5://user:pass@192.0.2.45:1080", "https://198.51.100.61:443", "http://203.0.113.130:8118",
  "192.0.2.88:3128", "socks4://198.51.100.142:4145", "http://192.0.2.199:8080",
  "socks5://203.0.113.201:1080", "198.51.100.7:80", "http://192.0.2.12:8888",
  "socks5://198.51.100.99:1080", "203.0.113.5:3128", "http://198.51.100.180:8080",
  "192.0.2.150:1080",
];
const DEAD = { 8: "timeout", 11: "connection refused", 13: "proxy auth required", 15: "http 502" };
const ANON = ["elite", "elite", "anonymous", "elite", "elite", "transparent", "anonymous", "elite"];
const LAT = [48, 71, 96, 124, 139, 187, 232, 268, 0, 311, 364, 0, 455, 0, 612, 0];

function result(line, i) {
  const scheme = line.includes("://") ? line.split("://")[0] : "http";
  const url = line.includes("://") ? line : `http://${line}`;
  const proto = scheme.toUpperCase();
  const base = {
    proxy: line, url, proto, alive: false, latency_ms: null, jitter_ms: null, success_rate: 0, attempts: 1,
    ip: null, country: null, country_code: null, city: null, isp: null, org: null, asn: null, ip_type: null,
    anonymity: null, tamper: null, tls: null, tls_info: null, error: null, skipped: [], sites: [],
  };
  if (DEAD[i]) return { ...base, error: DEAD[i] };
  const g = GEO[i % GEO.length];
  const host = url.split("://")[1].split("@").pop().split(":")[0];
  const exit = i === 6 ? "203.0.113.77" : host.replace(/\.(\d+)$/, (_, d) => `.${(Number(d) + 3) % 250 + 1}`);
  return {
    ...base, alive: true, latency_ms: LAT[i], jitter_ms: 4 + (i * 7) % 23, success_rate: 1, attempts: 1,
    ip: exit, country: g[1], country_code: g[0], city: g[2], isp: g[3], org: g[3], asn: g[4], ip_type: g[5],
    anonymity: ANON[i % ANON.length],
    tamper: i === 5 ? "modified" : i === 12 ? "unverified" : "ok",
    tls: i === 9 ? "modified" : "ok",
    tls_info: "CN=example.com, issuer Example CA",
    sites: [
      { url: "https://www.google.com", ok: true, ms: LAT[i] + 60, status: 200, error: null },
      { url: "https://www.youtube.com", ok: i % 5 !== 3, ms: i % 5 !== 3 ? LAT[i] + 90 : null, status: i % 5 !== 3 ? 200 : 403, error: i % 5 !== 3 ? null : "http 403" },
    ],
    dnsbl: i % 4 === 2 ? { listed: ["Spamhaus ZEN"], unknown: [] } : { listed: [], unknown: [] },
  };
}

const poolItems = () => LINES.map(result).filter((r) => r.alive).map((r) => ({
  raw: r.proxy, url: r.url, latency: r.latency_ms, country: r.country_code,
}));
let pool = demo === "dispatch" || demo === "auto" ? poolItems() : [];
const prefs = {
  closeToTray: true, startMinimized: true, startDispatcher: true,
  dispatcher: { port: 1080, mode: "round-robin", listener: "both", options: { countries: ["DE", "NL"], stickyMin: 10, authUser: "", authPass: "" } },
  poolDispatchers: {}, poolApi: { enabled: true, port: 1090 },
};
const dispatching = demo === "dispatch";

function emit(event, payload) {
  for (const id of listeners.get(event) ?? []) callbacks.get(id)?.({ event, id, payload });
}

const handlers = {
  "plugin:event|listen": ({ event, handler }) => {
    listeners.set(event, [...(listeners.get(event) ?? []), handler]);
    return handler;
  },
  "plugin:event|unlisten": () => null,
  app_version: () => pkg.version,
  install_kind: () => "nsis",
  get_app_prefs: () => prefs,
  set_app_prefs: ({ prefs: p }) => Object.assign(prefs, p),
  autostart_enabled: () => true,
  set_autostart: () => null,
  check_busy: () => false,
  cancel_check: () => null,
  list_pools: () => [{ id: "main", name: "" }, { id: "p1", name: "Scraping" }, { id: "p2", name: "Work" }],
  get_dispatch_pool: () => pool,
  set_dispatch_pool: ({ items }) => { pool = items ?? []; return pool.length; },
  set_dispatch_options: () => null,
  set_auto_config: () => null,
  update_tray_status: () => null,
  history_size: () => 148,
  pool_api_status: () => ({ running: true, port: 1090, error: null }),
  start_pool_api: () => 1090,
  local_proxy_status: () => ({
    running: dispatching, port: 1080, mode: "round-robin", listener: "both", upstreams: pool.length,
    cooling: dispatching ? 1 : 0, current: dispatching ? "socks5://203.0.113.14:1080" : null,
    requests: dispatching ? 18432 : 0, errors: dispatching ? 37 : 0, last_error: null,
  }),
  proxy_history: ({ urls }) => Object.fromEntries(urls.map((u, i) => [u, {
    checks: 12 + (i % 9), alive: 12 + (i % 9) - (i % 4), uptime: 1 - (i % 4) / (12 + (i % 9)),
    score: 0.9 - (i % 4) * 0.07, lastAlive: now - 600 * (i + 1), lastCheck: now - 300, avgMs: 60 + i * 31,
  }])),
  parse_proxy_text: ({ text, wantList }) => {
    const list = text.split(/\r?\n/).map((l) => l.trim()).filter(Boolean);
    return { count: list.length, truncated: false, list: wantList ? list : null };
  },
  check_proxies: async ({ proxies, onResult }) => {
    const send = callbacks.get(onResult.id);
    let index = 0;
    for (const [i, line] of proxies.entries()) {
      await new Promise((r) => setTimeout(r, demo === "progress" ? 400 : 15));
      send({ message: { kind: "result", ...result(line, LINES.indexOf(line) >= 0 ? LINES.indexOf(line) : i) }, index: index++ });
    }
    send({ message: { kind: "done" }, index: index++ });
    send({ end: true, index });
  },
};

window.__TAURI_INTERNALS__ = {
  transformCallback(cb) {
    const id = nextId++;
    callbacks.set(id, cb);
    return id;
  },
  unregisterCallback: (id) => callbacks.delete(id),
  convertFileSrc: (p) => p,
  metadata: { currentWindow: { label: "main" }, currentWebview: { label: "main" } },
  invoke: async (cmd, args) => {
    if (!(cmd in handlers)) throw new Error(`demo backend: no ${cmd}`);
    return handlers[cmd](args ?? {});
  },
};
window.__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener: () => {} };
window.__DEMO__ = { emit, LINES };

// The scenes the screenshots show.
window.addEventListener("load", () => setTimeout(async () => {
  const $ = (id) => document.getElementById(id);
  const tab = (name) => document.querySelector(`.tab[data-tab="${name}"]`)?.click();
  if (demo === "check" || demo === "progress") {
    const input = $("proxy-input");
    input.value = LINES.join("\n");
    input.dispatchEvent(new Event("input", { bubbles: true }));
    await new Promise((r) => setTimeout(r, 400));
    $("btn-start").click();
  } else if (demo === "dispatch") {
    tab("dispatch");
  } else if (demo === "auto") {
    tab("auto");
    emit("auto://status", { pool: "main", state: "running", done: 9, alive: 0, total: 12, avgMs: null, error: null, at: now });
  }
  document.documentElement.dataset.demoReady = "1";
}, 300));
