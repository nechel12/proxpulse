# ProxPulse

*[Русская версия](README.md)*

Desktop proxy checker with a live UI: liveness, ping, geo, anonymity,
traffic integrity and TLS checks — plus a local proxy dispatcher,
timer-based auto-checks and posting alive proxies to webhooks.

Stack: **Tauri 2 + Rust (tokio, reqwest, rustls) + Vite + TypeScript**.
Dark theme with red accents, Russian and English UI.

> Ready Windows build — under [Releases](https://github.com/nechel12/proxpulse/releases):
> installer and portable `.exe`. The app updates itself.

## Quick start

Requires [Node.js 18+](https://nodejs.org/), [Rust](https://rustup.rs/)
and WebView2 (built into Windows 10/11).

```sh
npm install
npm run tauri dev     # dev mode
npm run tauri build   # build installer
```

Code checks:

```sh
npm run build         # tsc + vite build
npm test              # vitest: proxy parsing (frontend)
cargo test --manifest-path src-tauri/Cargo.toml --lib   # unit tests (backend)
cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets
```

## Checking (the “Check” tab)

Paste a list into the left field (one per line) and hit **Start**.
Results stream in live, as they arrive. The **Stop** button really stops
the check.

### Supported formats

Auto-detected, scheme case doesn't matter:

- `127.0.0.1:8080`, `proxy.example.com:8080`, `[2001:db8::1]:8080`
- `user:pass@127.0.0.1:8080`
- `http://…`, `https://…`, `socks5://…`, `socks4://…`
- `127.0.0.1:8080:user:pass` and reversed `user:pass:127.0.0.1:8080`
- `127.0.0.1:8080:socks5` (protocol as suffix)
- CSV/tabs/spaces: `127.0.0.1,8080,user,pass`
- JSON: `{"host":"…","port":8080,"username":"u","password":"p"}`, arrays and logs
- Numbered lists (`1) …`, `- …`), `# …` comments

Duplicates are dropped automatically. File import (txt, csv, json,
log — actually any text file) and URL import — the list is fetched
with the built-in client (2 MB cap) and parsed the same way.

### Test URLs

- `google generate_204`, `cloudflare generate_204` — fast, empty body
- `api.ipify.org`, `httpbin.org/ip` — reveal the exit IP
- `ip-api.com/json` — IP + geo in one shot
- `example.com` — a regular page
- **ProxPulse Judge** — own check backend (see below);
  the field next to it accepts a custom instance. Two modes:
  **Fast** (`/generate_204`) and **Full** (`/judge` — geo, anonymity
  and IP type in a single response, plus content-hash integrity check)

A custom URL can also be typed manually.

### Measurement settings

| Parameter | Default | Meaning |
|---|---|---|
| Timeout, ms | 5000 | single HTTP request limit (1000–30000) |
| Threads | 50 | parallel checks (1–500) |
| Samples | 1 | repeats per proxy for ping/jitter/SR (1–5) |
| No-scheme type | HTTP | assumed when no scheme given; HTTP + SOCKS5 mode checks both |
| Port precheck | on, 1500 ms | fast TCP connect: drops dead hosts before the heavy check |

Settings are stored as named **profiles** (“Fast”, “Deep”,
“Judge” built in + your own): pick, save and delete in one click.

### Deep checks (not for judge mode)

- **Geo & IP type** — via ip-api (country/city/ISP/ORG/ASN, mobile/datacenter/residential)
- **Anonymity** — `elite` / `anonymous` / `transparent` (header heuristics via httpbin + comparison against your direct IP)
- **Integrity** — example.com body vs direct connection (`ok` / `modified`)
- **TLS integrity** — example.com certificate SHA256 vs direct (`ok` / `modified`)

Each deep check means extra requests through the proxy, so all of them
are toggleable. Not needed in judge mode: the judge returns everything itself.

### Export

**Export** menu: `.txt` (proxies only), `.json` (full results),
`.csv` (table), **Copy**. The square **“dead”** toggle next to it switches
whether dead proxies go into the export. Saving uses the system dialog.

Below the table — a sparkline of alive avg ping per check (history
stored locally).

## Dispatcher (the “Dispatcher” tab)

Spins up a **local proxy on 127.0.0.1:port** (default 1080) routing traffic
through alive proxies from checks. One port answers **both HTTP and SOCKS5**
(auto-detected by the first byte) or just the selected protocol.
If the port is busy, the next free one is taken automatically
(+1, +2, …); the actual port is written back into the field and shown
to the client.

- Modes: **rotation**, **fastest**, **failover**
- The pool merges (old entries are kept); “Sync” / “Clear” buttons;
  pool filter and sorting (by ping / country)
- **Max ping**: proxies slower than the limit never enter the pool (`0` — no limit)
- Request/error counters, current upstream, last error
- Pool auto-check every N minutes with dead pruning

## Auto (the “Auto” tab)

The same shared pool on its own timer: re-checked every N minutes,
dead ones drop out. After each auto-check the pool is POSTed
to **webhooks** (up to 10, shared txt/json/csv format).
“Check now” and “Send now” buttons for manual runs.

## Info, tray, diagnostics (the “Info” tab)

- GitHub links: the checker itself and ProxPulse Judge
- **Diagnostics**: app version, update check via GitHub Releases
  (install with restart), report copy (version, counters, recent errors)
- The ⇲ button in the header minimizes the window to tray. Tray (right click):
  show/hide, dispatcher on/off with the current port, time until next
  auto-checks, webhook state, quit. Left click toggles the window.

## Privacy and local data

What leaves the machine during a check: requests to the selected test URL
(plus ip-api/httpbin/example.com when deep checks are on), direct
no-proxy requests for baselines (your IP, example.com reference), requests
to your/public Judge instance, a GitHub releases check, POSTs to your
webhooks, URL list download. Proxy passwords only go to the proxies
themselves in auth headers — nowhere else.

Stored locally in the browser: language, ping limit, webhooks, geo cache
(up to 2000 entries), recent errors, ping history, settings profiles.
Clearing localStorage wipes it.

## Project structure

```
index.html            # all screens markup
src/main.ts           # all frontend logic + RU/EN dictionaries (I18N)
src/parse.ts          # proxy parsing (pure functions + tests)
src/parse.test.ts     # 18 parsing tests (vitest)
src/styles.css        # dark theme
src-tauri/src/lib.rs  # backend: checker, dispatcher, tray (~2600 lines)
src-tauri/src/main.rs # entry point
.github/workflows/release.yml  # release builds and auto-update files
```

## Related projects

- [proxpulse-judge](https://github.com/nechel12/proxpulse-judge) —
  self-hosted check backend (Rust/axum, Docker). The public instance
  is embedded in the checker by default.

## License

Apache-2.0, see [LICENSE](LICENSE).
