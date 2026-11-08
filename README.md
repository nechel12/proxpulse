# ProxPulse

**English** | [Русский](README.ru.md)

[![CI](https://github.com/nechel12/proxpulse/actions/workflows/ci.yml/badge.svg?branch=main)](https://github.com/nechel12/proxpulse/actions/workflows/ci.yml)
[![Release](https://img.shields.io/github/v/release/nechel12/proxpulse)](https://github.com/nechel12/proxpulse/releases/latest)
[![Downloads](https://img.shields.io/github/downloads/nechel12/proxpulse/total)](https://github.com/nechel12/proxpulse/releases)
[![License](https://img.shields.io/github/license/nechel12/proxpulse)](LICENSE)

A desktop proxy checker for Windows, Linux and macOS, for anyone who keeps
lists of HTTP, HTTPS and SOCKS proxies and needs to know which ones work, how
fast, from where and how anonymously. It keeps the working ones in pools,
serves each pool as a local rotating proxy and to scripts over a local API,
re-checks the pools on a schedule and posts them to webhooks.

**[Download the latest release](https://github.com/nechel12/proxpulse/releases/latest)** ·
Russian and English interface · free and open source (Apache-2.0)

> Status: pre-1.0. Settings, export formats and webhook payloads may change in
> minor releases; changes are listed in [CHANGELOG.md](CHANGELOG.md).

![ProxPulse main window](docs/screenshots/main-en.png)

## Contents

- [Install](#install) · [Quick start](#quick-start) · [FAQ](#faq)
- Checking: [Input formats](#input-formats) · [Check target](#check-target) ·
  [Check settings](#check-settings) · [How verdicts are decided](#how-verdicts-are-decided)
- [Pools, dispatcher and API](#pools-dispatcher-and-api) ·
  [Scheduled checks and webhooks](#scheduled-checks-and-webhooks) ·
  [Settings and privacy](#settings-and-privacy)
- [Troubleshooting](#troubleshooting) · [Limitations](#limitations) ·
  [Security](#security) · [Build from source](#build-from-source)
- Guides: [checking](docs/checking.md) · [dispatcher and pool API](docs/dispatcher.md) ·
  [schedules and webhooks](docs/automation.md) · [settings and local data](docs/settings.md)

## Install

Builds for every platform are attached to each
[GitHub release](https://github.com/nechel12/proxpulse/releases):

| Platform | File | Updates |
|---|---|---|
| Windows x64 | `…_x64-setup.exe` — NSIS installer (recommended) | built-in |
| | `…_x64_en-US.msi` — MSI installer | manual |
| | `…-portable.exe` — no installer (data still goes to `%APPDATA%\com.nechel12.proxpulse`) | replace the file |
| Linux x64 | `…_amd64.AppImage` | built-in |
| | `…_amd64.deb`, `…x86_64.rpm` | install the new package by hand (no repository) |
| macOS | `…_aarch64.dmg` (Apple Silicon), `…_x64.dmg` (Intel) | built-in |

`SHA256SUMS.txt` lists the checksum of every file; `latest.json` and `.sig`
files serve the built-in updater. Release tags from 0.6.0 on are signed with
the SSH key in [`docs/proxpulse-release.pub`](docs/proxpulse-release.pub);
[RELEASING.md](RELEASING.md) shows how to check one.

> [!NOTE]
> **Windows:** builds are not Authenticode-signed, so SmartScreen may show
> "Windows protected your PC": **More info → Run anyway**. Before that, the
> file can be checked against `SHA256SUMS.txt` in PowerShell:
> `(Get-FileHash .\ProxPulse_X.Y.Z_x64-setup.exe).Hash` (compare
> case-insensitively).
>
> **macOS:** builds are signed ad hoc, not with an Apple certificate. On first
> launch macOS refuses to open the app; on macOS 15 and later the context
> menu's **Open** no longer helps: **System Settings → Privacy & Security →
> Open Anyway** (shown right after the refused launch). Or, after copying the
> app to Applications: `xattr -dr com.apple.quarantine /Applications/ProxPulse.app`.
> The author has no Mac: macOS builds are verified by compilation only and are
> not guaranteed to work. Problem reports should include the diagnostics from
> the Settings tab.
>
> **Linux:** an AppImage needs `chmod +x ProxPulse_*.AppImage` and FUSE 2
> (`libfuse2`, `libfuse2t64` on Ubuntu 24.04); without FUSE it runs with
> `--appimage-extract-and-run`.

## Quick start

1. Paste a proxy list in any format into **Source**, drop files on it, or
   import files or a URL. The counter shows how many proxies a check will test.
2. Pick a preset — **Standard** checks liveness plus every deep check,
   **Fast** liveness only — and press **Start** (Ctrl+Enter). Results stream
   in as proxies finish; **Stop** (Esc) aborts at once.
3. Export the alive proxies (**Export** → `.txt`, `.json`, `.csv` or **Copy**),
   or turn on the dispatcher and point a browser at `127.0.0.1:1080` as an
   HTTP or SOCKS5 proxy.
4. Optionally, the **Auto** tab re-checks the pool every N minutes and posts it
   to webhooks.

## FAQ

**No proxy comes out alive.** Free lists are mostly dead within hours, so a
few percent alive is normal. If a list known to work fails too, the check
target may be unreachable from your network: **Check server** → another
target, or the **Fast** preset with a lower thread count.

**My antivirus or SmartScreen warns about the installer.** The builds are not
code-signed, which is what these warnings are about. Each file can be checked
against `SHA256SUMS.txt` of its release, as shown under [Install](#install).

**Anonymity shows `—`.** It needs a plain-HTTP request through the proxy:
HTTPS-only proxies and a judge that redirects to HTTPS give no verdict; see
[Troubleshooting](#troubleshooting).

## Checking

### Input formats

The format is detected per line; scheme case does not matter:

- `127.0.0.1:8080`, `proxy.example.com:8080`, `[2001:db8::1]:8080`
- `user:pass@127.0.0.1:8080`
- `http://…`, `https://…` (TLS to the proxy), `socks5://…`, `socks5h://…`,
  `socks://…`, `socks4://…`, `socks4a://…`
- `127.0.0.1:8080:user:pass`, `user:pass:127.0.0.1:8080`,
  `127.0.0.1:8080@user:pass` and `127.0.0.1:8080 user pass`; a password keeps
  any characters, `/`, `#`, `;` or `!` included
- `127.0.0.1:8080:socks5` (protocol as a suffix)
- CSV, tabs or spaces: `127.0.0.1,8080,user,pass`, `socks5,127.0.0.1,8080`
- Tables copied from free-proxy sites: the IP and port columns are taken, the
  country, anonymity and other columns are ignored
- JSON: an object, an array or a log with `host`/`ip`, `port`, `username`,
  `password`, `type` fields (`"https"` there means an HTTP proxy that supports
  HTTPS sites)
- Numbered or bulleted lists (`1) …`, `- …`, `[x] …`), `# …` and `// …`
  comments, `ip:port` inside prose

Text files of any kind (TXT, CSV, JSON, LOG) and URLs (up to 2 MB, public
hosts only) are imported with the same parser. Every entry must normalize to a
proxy the checker can use, and one proxy written two ways counts once, so the
counter equals the number of proxies a check tests. A check and a pool take up
to 20,000 proxies, and the source field up to 10 MB of text; a longer list is
checked up to that number, and the status line says so.

> [!NOTE]
> IPv6 addresses need square brackets (`[2001:db8::1]:8080`); the bare form is
> ambiguous and is not accepted. `socks4` cannot carry IPv6 or host names:
> `socks5` or `socks4a` is used for them.

### Check target

Every proxy requests the chosen target. **Public server** uses a ready-made
endpoint (`generate_204`, ipify, httpbin, ip-api, example.com) or your URL; a
proxy is alive only when the target gives its own answer, so captive portals
count as dead. **ProxPulse Judge** uses the built-in
[judge](https://github.com/nechel12/proxpulse-judge) or your own: **Full**
mode gets geo, IP type, anonymity and integrity in one request.

Anonymity needs **plain HTTP**: inside an HTTPS tunnel the proxy can add
nothing. The judge signs each answer, and ProxPulse checks the signature, so a
proxy that rewrites the verdict is marked `?mod`. Deep checks (geo, anonymity,
integrity, TLS, site access, blocklists), the HTTPS and HTTP choices, the
results table and export are described in [docs/checking.md](docs/checking.md).

### Check settings

| Setting | Default | Meaning |
|---|---|---|
| Timeout | 5000 ms | limit for one request through the proxy (1000–30000) |
| Threads | 50 | proxies checked in parallel (1–500) |
| Requests per proxy | 1 | requests per proxy for ping, jitter and success rate (1–5); each opens a new connection, so ping includes connecting to the proxy |
| Protocol for lines without a scheme | HTTP | protocol for entries without a scheme; **HTTP + SOCKS5** tries both |
| Check the port first | on, 1500 ms | fast TCP connect that drops dead hosts before the full check |

Presets: **Fast** (liveness only), **Standard** (all deep checks),
**Deep** (ip-api, 3 requests per proxy), **Judge** (built-in judge, full mode). Editing
any setting turns the selection into a custom configuration; **+ Save**
stores it as a named preset.

### How verdicts are decided

| Column | Value | Meaning |
|---|---|---|
| Anon | `elite` | No proxy headers and no trace of your direct IP. |
| | `anon` | The proxy reveals itself (`Via`, its own `X-Forwarded-For` entries, `Forwarded`, `X-Real-IP`, `Proxy-*`) but not your IP. |
| | `transp` | Your direct IP appears in a forwarded header: the proxy leaks it. |
| TLS | `ok` / `mod` / `n/v` | The certificate chain verifies against the public roots (another valid certificate than the direct one, such as a different CDN edge, is only noted in the tooltip) / it is rejected, so the proxy swaps it / the handshake failed for another reason. |
| Info | `?mod`, `?tls` | The page through the proxy differs from the direct one or fails the judge's signature / the TLS certificate is rejected. The row tooltip lists every result. |
| Info | `?unv` | The judge's key could not be fetched, so the answer's signature was not checked. |
| Info | `bl N` | The exit IP is on N blocklists; the tooltip names them. |
| Anon, Geo, TLS | `—` | Not checked: off, unsupported by the proxy, or an HTTPS judge. Anon is also `—` when your own IP could not be learned (neither `api.ipify.org` nor the judge's `/ip` answered): without it a leak cannot be seen. |
| Anon, Geo, TLS | `limit` | Skipped at a concurrency limit; running the check again fills it. |

## Pools, dispatcher and API

Alive proxies go into a pool; up to ten named pools keep their own proxies,
dispatcher, schedule and webhooks. The **Dispatcher** tab serves a pool as a
local HTTP and SOCKS5 proxy on `127.0.0.1:1080` with rotation, fastest or
failover modes, a country filter, sticky sessions and an optional login.
**Pool API** hands proxies to scripts:

```sh
curl "http://127.0.0.1:1090/random?country=DE&proto=socks5"
```

Modes, sessions, the API's parameters and its rules for logins:
[docs/dispatcher.md](docs/dispatcher.md).

## Scheduled checks and webhooks

The **Auto** tab re-checks a pool every N minutes. A proxy leaves after two
failed checks in a row; a check in which most of the pool fails is held back
as a local problem, and an offline machine is retried instead of emptying the
pool. Subscriptions add new proxies from list URLs before each check. After
each check the pool can be posted to up to ten webhooks (TXT, JSON, CSV,
Telegram, Discord or a template), whole or as changes only. Everything runs in
the backend, so it works with the window in the tray. Details and payload
examples: [docs/automation.md](docs/automation.md).

## Settings and privacy

The **Settings** tab holds the language and theme, the tray and startup
options, update checks and notifications, settings export and import, and the
diagnostics for bug reports.

Requests leave the machine only for what you use: the check target and deep
check services, the judge, GitHub Releases for updates, your webhooks, URL
imports and subscriptions, the chosen sites, the speed test and blocklists.
Proxy logins go to the proxy itself, into exports, and to webhooks only when
you allow it. The pool, **with logins and passwords**, is stored unencrypted in
the app data folder (owner-only on Linux and macOS). The full list of
requests and files: [docs/settings.md](docs/settings.md).

## Troubleshooting

| Symptom | Cause and fix |
|---|---|
| Anon is `—` and the integrity result is missing in Judge mode | **HTTP** is off, the judge does not answer over plain HTTP (it redirects to HTTPS), or the proxy allows only port 443. Anonymity needs a plain-HTTP request through the proxy. |
| Anon is `—` for working proxies | The proxy allows only CONNECT to port 443, so the plain-HTTP probes fail. Expected for HTTPS-only proxies. |
| **limit** in Geo, Anon or TLS | The concurrency limit or the ip-api 429 cooldown was hit. Fewer threads or a second run fill the gaps. |
| Many alive proxies without geo on free ip-api | The free tier allows 45 requests per minute. ip-api Pro or the judge removes the limit; the geo cache helps on repeated runs. |
| The source counter shows `!` | The input exceeds 10 MB, or parsing failed (the status line shows the error). |
| A line is not counted | It does not normalize to a proxy (bad port, bare IPv6, missing host) or is the same proxy as an earlier line. |
| Auto status: "Check server unreachable directly" | No network, or the check target is blocked without a proxy. The pool is kept and the check is retried in a minute. |
| Scheduled checks stop when the window is closed | Closing quits the app by default: **Settings → When the window is closed → Hide to tray**, or **⇲**, keeps it running. |
| No system notifications | They are shown only while the window is hidden or in the background, and the OS may block them for ProxPulse. |
| The dispatcher starts on 1081, 1082, … | The requested port is busy; the next free one is used and shown. |
| The browser gets `502 All upstreams failed` | Every tried upstream failed; the dispatcher status shows the last error. Re-checking the pool or switching to failover helps. |
| `Webhook error: http 4xx/5xx` | The receiver rejected the POST. Local and private hosts are blocked, and redirects are not followed. |
| DNSBL: Spamhaus is unknown for every proxy | Your DNS is a large public resolver, which Spamhaus refuses. Another resolver (for example your provider's) gets its answers. |
| The pool API answers `401` | The pool has a dispatcher login: send it, for example `curl -u user:pass`. |
| The pool API answers `403` | The `Host` header is not `127.0.0.1` or `localhost`. |
| The pool API does not start | The port is taken, for example by a dispatcher: choose another. |
| macOS: "the app is damaged" or "cannot be opened" | Not signed by an Apple developer: **System Settings → Privacy & Security → Open Anyway**, or `xattr -dr com.apple.quarantine /Applications/ProxPulse.app`, then open it again. |

## Limitations

- Anonymity, integrity and IP type are heuristics: a proxy can withhold or
  forge headers.
- Header and content checks need plain HTTP: HTTPS-only proxies and an HTTPS
  judge yield no verdict.
- The TLS check compares the `example.com` leaf certificate with a direct
  connection; a legitimately different valid certificate (another CDN edge, a
  rotation) is reported as `mod`.
- ip-api and httpbin.org are shared public services with rate limits and
  outages; for regular use a self-hosted judge is more reliable.
- The dispatchers and the pool API listen on `127.0.0.1`; without a login
  any local process can use a pool.
- Blocklists list IPs for spam and abuse: shared and residential exits can be
  listed because of other users.
- Webhook URLs, the ip-api key and the pool are stored unencrypted.
- Builds are not code-signed; macOS builds are compile-checked only.

## Security

Vulnerability reporting is described in [SECURITY.md](SECURITY.md); working
proxies, credentials and webhook URLs must not be posted in public issues.

ProxPulse is a diagnostic tool, not a security boundary. Proxies are untrusted
input: response sizes are capped, user-supplied URLs (webhooks, URL import)
are restricted to public hosts with DNS pinning (webhooks follow no redirects,
URL imports re-check every redirect), and credentials are sent to webhooks
only over HTTPS on request. Updates are
verified against the signing key embedded in the app.

## Build from source

[Node.js](https://nodejs.org/) 22.13+ (or 24+), [Rust](https://rustup.rs/)
1.90+, WebView2 on Windows (preinstalled on Windows 10/11), WebKitGTK on
Linux:

```sh
npm ci
npm run tauri dev        # development with hot reload
npm run build:unsigned   # local release build without the updater key
```

The Linux packages, the checks a change must pass, the project structure and
the release process are in [CONTRIBUTING.md](CONTRIBUTING.md) and
[RELEASING.md](RELEASING.md).

## Related projects

- [ProxPulse Judge](https://github.com/nechel12/proxpulse-judge) — self-hosted
  check backend (Rust/axum, Docker): exit IP, anonymity, local geo/ASN and a
  tamper check in one request. Its public instance is the default judge here.

## Contributing

Bug reports and pull requests are welcome. The development workflow is
described in [CONTRIBUTING.md](CONTRIBUTING.md); participation implies
agreement with the [Code of Conduct](CODE_OF_CONDUCT.md). Bug reports should
include the version, OS and the diagnostics from the Settings tab — the issue form
asks for them. Security issues go through [SECURITY.md](SECURITY.md), not a
public issue. Changes are listed in [CHANGELOG.md](CHANGELOG.md).

## License

The code is licensed under Apache-2.0; the license text is in
[LICENSE](LICENSE). Third-party services used by the checks (ip-api,
httpbin.org, example.com, public judge instances) have their own terms.

---

## Support the project

ProxPulse is free and open source. If it saves time, further development can
be supported here:

<p align="center">
  <a href="https://boosty.to/nechel12/donate">
    <img src="https://img.shields.io/badge/Boosty-Donate-F15F2C?style=for-the-badge&logo=boosty&logoColor=white" alt="Donate via Boosty">
  </a>
</p>

### Crypto

| Network | Address |
|:--------|:--------|
| **TRON** | `TMM2Cdm4tJx1nKRRq5RCBcdwfn9cHuJZy9` |
| **TON** | `UQBlVvOfTleWn4pkY384mYWiGyhdHBryoC_N1Si1OSWAxPKl` |
| **Bitcoin** | `bc1qagjtnutjya2rss3u3ls8ur7qk0u7zutc6t6fwc` |
| **BNB Smart Chain** | `0x423aF840f67355371bf32480744A84069dfab5ff` |

> [!WARNING]
> Only the asset matching the specified network may be sent. Transfers to
> another network cannot be recovered.
