# ProxPulse

[![GitHub release](https://img.shields.io/github/v/release/nechel12/proxpulse)](https://github.com/nechel12/proxpulse/releases) [![Downloads](https://img.shields.io/github/downloads/nechel12/proxpulse/total)](https://github.com/nechel12/proxpulse/releases) [![Release build](https://github.com/nechel12/proxpulse/actions/workflows/release.yml/badge.svg)](https://github.com/nechel12/proxpulse/actions/workflows/release.yml) [![CI](https://github.com/nechel12/proxpulse/actions/workflows/ci.yml/badge.svg)](https://github.com/nechel12/proxpulse/actions/workflows/ci.yml) ![Windows](https://img.shields.io/badge/platform-Windows-0078D6?logo=windows&logoColor=white) ![Linux](https://img.shields.io/badge/platform-Linux-FCC624?logo=linux&logoColor=black) ![macOS](https://img.shields.io/badge/platform-macOS-000000?logo=apple&logoColor=white) [![License](https://img.shields.io/github/license/nechel12/proxpulse)](LICENSE)

*[Russian version](README.md)*

ProxPulse is a desktop proxy checker with real-time results: liveness, latency,
geolocation, anonymity, traffic integrity and TLS checks, plus a local proxy
dispatcher, scheduled re-checks and webhook delivery of working proxies.

Stack: **Tauri 2 + Rust (tokio, reqwest, rustls) + Vite + TypeScript**.
Dark theme with red accents; Russian and English UI.

## Features

- Liveness, latency, jitter and success-rate checks.
- Geolocation, IP type, anonymity, content integrity and TLS checks.
- HTTP, HTTPS (TLS to the proxy), SOCKS5/SOCKS5h and SOCKS4/SOCKS4a.
- Local HTTP/SOCKS5 dispatcher with rotation, fastest and failover modes.
- Scheduled re-checks, settings profiles, TXT/JSON/CSV export and webhooks.
- File and URL import, deduplication, dark theme, RU/EN UI.
- Update checks and a system tray.

## Installation

Ready builds are available in [Releases](https://github.com/nechel12/proxpulse/releases):

- **Windows (x64):** `-setup.exe` or `.msi` installer, and a portable `.exe`;
- **Linux (x64):** `.AppImage`, `.deb`, `.rpm`;
- **macOS:** `.dmg` for Apple Silicon and Intel.

Built-in auto-update works for Windows NSIS (`-setup.exe`), macOS and the
Linux AppImage. MSI, `.deb` and `.rpm` are installed and updated manually
(installer or package manager). Portable builds are updated by replacing the
file. The file to download for each platform is described in the release notes.

> **macOS:** builds are not signed with an Apple certificate. On first launch,
> open the app from the context menu → Open, or run `xattr -cr ProxPulse.app`.
> The author does not have access to a Mac: macOS builds are verified by
> successful compilation only; they have not been run and are not guaranteed
> to work. If you encounter problems, attach the diagnostics from the Info tab.

## Build from source

Requirements: [Node.js 22.12+](https://nodejs.org/), [Rust](https://rustup.rs/)
(stable, 1.90 or newer) and WebView2 (built into Windows 10/11). Linux builds
additionally need WebKitGTK and other system packages.

<details>
<summary>Linux build dependencies (Debian/Ubuntu)</summary>

```sh
sudo apt update
sudo apt install -y libwebkit2gtk-4.1-dev build-essential curl wget file \
  libxdo-dev libssl-dev libayatana-appindicator3-dev librsvg2-dev
```

</details>

```sh
npm install           # or npm ci for a lockfile-based install
npm run tauri dev     # development mode
npm run tauri build   # release build; requires TAURI_SIGNING_PRIVATE_KEY
npm run build:unsigned # local build without updater artifacts or a key
```

The release build signs updater artifacts with the CI key
(`TAURI_SIGNING_PRIVATE_KEY`). For a local build without access to the key,
use `npm run build:unsigned`: it disables `createUpdaterArtifacts` and does
not try to sign installers. Do not publish such a build as an update — use it
for local runs and checks only.

Checks:

```sh
npm run build         # tsc + vite build
npm test              # vitest: parsing, profiles, CSV, export, i18n, formatting
npm run lint          # ESLint
cargo fmt --manifest-path src-tauri/Cargo.toml -- --check
cargo test --manifest-path src-tauri/Cargo.toml --lib
cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings
npm run check:versions
```

CI (`.github/workflows/ci.yml`) runs these checks on pushes to `main` and on
pull requests, and also compiles the release binary without bundling
(`npm run build:unsigned -- --no-bundle`) to catch Tauri package version
mismatches and build errors.

## Checking (the “Check” tab)

Paste a list into the field on the left (one proxy per line) and click
**Start**. Results appear as checks complete. **Stop** aborts the current check.

### Supported formats

The format is detected automatically; scheme case does not matter:

- `127.0.0.1:8080`, `proxy.example.com:8080`, `[2001:db8::1]:8080`
- `user:pass@127.0.0.1:8080`
- `http://…`, `https://…`
- `socks5://…`, `socks5h://…`, `socks://…`
- `socks4://…`, `socks4a://…`
- `127.0.0.1:8080:user:pass` and the reverse order `user:pass:127.0.0.1:8080`
- `127.0.0.1:8080:socks5` (protocol as a suffix)
- CSV, tabs and spaces: `127.0.0.1,8080,user,pass`
- JSON: an object, array or log with `host`, `port`, `username`, `password`
- Numbered lists (`1) …`, `- …`) and `# …` / `// …` comments

Duplicates are removed automatically. A list can be imported from any text
file (TXT, CSV, JSON, LOG) or from a URL: the built-in client downloads up to
2 MB and parses it with the same parser.

> Put IPv6 addresses in square brackets: `[2001:db8::1]:8080`. The unbracketed
> form is ambiguous and is not supported.

> `socks4` does not support IPv6 literals. Use `socks5` or `socks4a` for IPv6;
> with `socks4a`, the IPv6 address is passed as a domain name (best effort;
> support depends on the proxy).

### Test URLs

- `google generate_204`, `cloudflare generate_204` — fast, empty body;
- `api.ipify.org`, `httpbin.org/ip` — show the exit IP;
- `http://ip-api.com/json` — IP and geolocation in one request (free tier is HTTP-only);
- `example.com` — a regular page;
- [ProxPulse Judge](https://github.com/nechel12/proxpulse-judge) — a dedicated
  check backend. In Judge mode, the public instance is used by default; the
  field next to it accepts another instance. Modes: **Fast** (`/generate_204`)
  and **Full** (`/judge` — geolocation, anonymity and IP type in one response,
  plus a content-hash integrity check).

A custom URL can also be entered manually.

> **ip-api: free tier vs Pro.** The free ip-api endpoint is HTTP-only: the
> proxy and the network see requests and your IP, and a response can be
> modified in transit. This is a deliberate trade-off; using the free HTTP
> endpoint is at your own risk. The Pro plan enables HTTPS and uses an API
> key (`https://pro.ip-api.com/...?key=...`, Pro page:
> <https://members.ip-api.com/>). Enable "ip-api Pro" and enter the key in
> the check settings; ip-api requests (both the main test URL and the deep
> geo lookup) will then use HTTPS.
>
> ip-api requests are made **through the checked proxy**, so the proxy sees
> both the key and the traffic. Use a dedicated key for this app; ip-api IP
> restrictions will not work when requests go through proxies. The key is
> stored locally in `localStorage` (like webhook URLs). Do not use the free
> tier for commercial use — that is an ip-api restriction.

### Check settings

| Parameter | Default | Description |
|---|---|---|
| Timeout | 5000 ms | single HTTP request limit (1000–30000) |
| Threads | 50 | parallel checks (1–500) |
| Samples | 1 | repeats per proxy for ping, jitter and success rate (1–5) |
| No-scheme type | HTTP | value used when no scheme is given; HTTP + SOCKS5 mode checks both |
| Port precheck | on, 1500 ms | fast TCP connect: unavailable proxies are dropped before the full check |

Settings are stored in named profiles: “Fast”, “Deep”, “Judge” and custom
ones. A profile can be selected, saved or deleted.

### Deep checks (not used in Judge mode)

- **Geolocation and IP type** — via ip-api: country, city, ISP, ORG, ASN,
  mobile/datacenter/residential.
- **Anonymity** — `elite` / `anonymous` / `transparent`: header heuristics via
  httpbin and comparison with the direct IP.
- **Content integrity** — example.com body compared with a direct connection
  (`ok` / `modified`).
- **TLS integrity** — example.com certificate SHA256 compared with a direct
  connection (`ok` / `modified`).

Each deep check makes additional requests through the proxy, so all of them can
be disabled. They are not needed in Judge mode: Judge returns the data itself.

> **Public API limits.** Free ip-api has a per-minute request limit and
> httpbin.org is regularly throttled. To avoid flooding ip-api at 500-way
> concurrency, at most 8 geo lookups run at once; anonymity, integrity and
> TLS checks are also concurrency-limited. Under load the corresponding deep
> check is skipped for some proxies (run the check again). The geo cache (up to 2000 entries,
> 6-hour TTL) reduces repeated lookups but does not replace a self-hosted
> service. For
> stable results, use [ProxPulse Judge](https://github.com/nechel12/proxpulse-judge).

Deep checks are heuristics. If a baseline request fails, anonymity may stay
empty. The TLS check reports `mod` when a certificate is rejected and `n/v`
(`unverified`) when the handshake fails for another reason (for example, the
proxy blocks example.com:443).

### Export

**Export** menu: `.txt` (normalized proxies, including the scheme), `.json`
(full results), `.csv` (table with the original string and `url`), **Copy**.
The **“dead”** toggle controls whether unavailable proxies are included in the
export. Saving uses the system dialog.

Below the table, a sparkline shows the average ping of working proxies across
checks; the history is stored locally.

## Dispatcher (the “Dispatcher” tab)

Starts a local proxy on **127.0.0.1:1080** (default port) and routes traffic
through working proxies from checks. One port accepts **both HTTP and SOCKS5**
(the protocol is detected from the first byte) or only the selected protocol.
If the port is busy, the next free one is selected (+1, +2, …); the actual port
is shown in the field.

- Modes: **rotation**, **fastest**, **failover**. Fastest makes a single
  attempt through the best upstream; failover tries up to five; rotation
  picks an upstream when a connection is established. Browser keep-alive
  keeps one upstream for the lifetime of a connection.
- The pool merges new entries with old ones; “Sync” and “Clear” buttons; pool
  filtering and sorting by ping or country.
- **Max ping**: proxies slower than the limit do not enter the pool (`0` — no
  limit).
- Request and error counters, current upstream proxy, last error.
- Pool auto-check every N minutes with removal of unavailable proxies.

## Auto (the “Auto” tab)

The shared pool is re-checked on a separate timer every N minutes; unavailable
proxies are removed. After each auto-check, the pool is sent as POST requests
to the configured webhooks (up to 10, TXT/JSON/CSV format). “Check now” and
“Send now” run these actions manually.

The scheduler and webhook delivery run in the Rust backend, so checks keep
running while the window is hidden in the tray. The on/off state and intervals
are persisted; after a restart the schedule is restored from the current
settings.

By default proxy credentials are stripped from URLs before posting. The
**“Send credentials”** option includes them, but requires HTTPS; an
`http://` webhook with credentials is rejected.

## Info, tray and diagnostics (the “Info” tab)

- GitHub links: ProxPulse and ProxPulse Judge.
- **Diagnostics**: app version, update check via GitHub Releases, install
  (on Windows the installer closes the app; on macOS/Linux restart manually),
  report copy (version, counters, recent errors).
- The ⇲ button in the header minimizes the window to the system tray. The tray
  menu provides show/hide, dispatcher on/off with the current port, time until
  the next auto-check, webhook state and quit. Left-clicking the icon shows or
  hides the window.

## Privacy and local data

During a check, the following leaves the machine: requests to the selected
test URL; requests to ip-api, httpbin and example.com when deep checks are
enabled; direct no-proxy requests for baselines (your IP and the example.com
reference); requests to the selected Judge instance; a GitHub release check;
POST requests to the configured webhooks; URL list downloads.

Proxy credentials are sent to the selected proxy using its authentication
mechanism (HTTP `Proxy-Authorization` or SOCKS authentication). They also
appear in exports (TXT/JSON/CSV/clipboard) and — **only when the option is
explicitly enabled** — in webhooks. By default webhooks receive URLs without
credentials.

Stored locally in the browser: language, ping limit, webhook URLs (may contain
tokens), the ip-api Pro key (if set), geo cache (up to 2000 entries, 6-hour TTL,
credential-free keys), recent errors, ping history, current check settings,
auto-check state and settings profiles. Clearing `localStorage` removes this
data.

## Security

Do not publish working proxies, passwords or other secrets in issues. To
report a vulnerability, use GitHub's private vulnerability reporting:
[report a vulnerability](https://github.com/nechel12/proxpulse/security/advisories/new).
See [SECURITY.md](SECURITY.md) for details.

## Project structure

```text
index.html                     # screen markup
src/main.ts                    # screen logic and event wiring
src/i18n.ts                    # RU/EN dictionaries
src/parse.ts                   # proxy parsing (pure functions)
src/profiles.ts                # settings profiles (pure functions)
src/csv.ts                     # CSV escaping
src/export.ts                  # TXT/JSON/CSV export
src/format.ts                  # table formatting helpers
src/types.ts                   # shared frontend types
src/*.test.ts                  # pure-module tests (vitest)
src/styles.css                 # dark theme
src-tauri/src/lib.rs           # backend: checker, dispatcher, tray
src-tauri/src/proxy.rs         # proxy parsing/normalization
src-tauri/src/main.rs          # entry point
src-tauri/tauri.conf.json      # Tauri configuration
src-tauri/Cargo.toml           # Rust dependencies
eslint.config.js               # ESLint
.github/workflows/ci.yml       # tests, clippy and version check
.github/workflows/release.yml  # release builds and auto-update files
scripts/check-versions.mjs     # version synchronization
```

## Related projects

- [ProxPulse Judge](https://github.com/nechel12/proxpulse-judge) — a
  self-hosted check backend (Rust/axum, Docker). The public instance is used
  in Judge mode by default.

## Contributing

Bug reports and pull requests are welcome. In a bug report, include the app
version, operating system and diagnostics from the Info tab. Before opening a
PR, run the checks from “Build from source”: `npm run build`, `npm test`,
`cargo test --manifest-path src-tauri/Cargo.toml --lib`,
`cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings`.

Issue tracker: [Issues](https://github.com/nechel12/proxpulse/issues).

## License

Apache-2.0. See [LICENSE](LICENSE).
