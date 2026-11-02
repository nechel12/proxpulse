# Changelog

All notable changes to ProxPulse. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/); versions follow
[Semantic Versioning](https://semver.org/).

## [Unreleased]

### Added

- Exports for proxy clients: Clash (YAML), sing-box (JSON), a Proxifier
  profile and a PAC file, and export by a custom line template.
- Site access: chosen sites (Google, YouTube, Telegram, Discord, ChatGPT or
  your own) are requested through every alive proxy, with a Sites column and
  filter.
- Speed test from the results context menu: download speed through the proxy
  with the connect, TLS and first-byte times.
- Per-proxy history (`history.json`): an Uptime column with the last time a
  proxy was alive, and a pool sort by reliability. The history can be cleared
  in Settings.
- Grouping by exit IP or ASN keeps one proxy per group; proxies sharing an exit
  IP are marked with ×N.
- Subscriptions: proxy list URLs downloaded before every scheduled check; new
  alive proxies join the pool.
- Webhooks: Telegram and Discord formats, a custom body template, change-only
  delivery after scheduled checks, and two retries for scheduled deliveries.
- Dispatcher: an upstream country filter, sticky sessions per site, a
  one-minute pause for upstreams that fail to connect, and an optional login
  for local clients (HTTP Basic and SOCKS5 username/password).
- Notifications when a scheduled check leaves fewer proxies than a threshold,
  and once when scheduled checks find no network.

## [0.5.0] — 2026-11-01

### Security

- Anonymity and content-integrity checks use plain HTTP. Over `https://` a
  proxy only tunnels the traffic, so every proxy looked `elite` and
  untampered. A judge reached over HTTPS no longer reports a guessed
  anonymity verdict and no longer receives your real IP (`direct_ip`).
- The dispatcher sends one plain HTTP request per connection, so a keep-alive
  request for another site can no longer reach the first site's tunnel
  without upstream credentials.
- SOCKS5 upstreams resolve hostnames through the proxy: the local DNS
  resolver no longer sees the domains visited through the dispatcher.
- Proxy passwords no longer end up in geo cache keys (column formats such as
  `host:port:user:pass`) or in the dispatcher's "current upstream" field.
  Legacy cache entries are dropped on start.
- Webhooks and URL import reject local and private targets (with DNS pinning)
  and do not follow redirects that could leak proxy credentials. The guard
  also covers `0.0.0.0/8` and private IPv4 addresses behind IPv4-compatible,
  NAT64 and 6to4 IPv6 forms.
- The release workflow gives build steps a read-only token and no longer
  leaves the checkout token in `.git/config`.

### Added

- A Settings tab with app-wide options: theme (dark, light, system); hiding
  logins and passwords in the tables; closing the window to the tray;
  starting with the system, optionally in the tray; starting the dispatcher
  at launch with its last configuration; a quiet update check at launch;
  system notifications while the window is hidden (failed scheduled webhook,
  emptied pool, new version); export, import and reset of the settings,
  optionally with the proxy pool.
- Results table: sorting by any column, Protocol, Anonymity and Country
  filters, row selection (click, Ctrl, Shift, Ctrl+A) and a context menu to
  copy, re-check, add to the pool or remove rows; **Re-check dead** updates
  only the dead entries.
- Keyboard shortcuts: Ctrl+Enter starts a check, Esc stops it, Ctrl+F focuses
  the filter. Files can be dropped on the source panel.
- A dispatcher that fails to start shows the error instead of "off".
- Releases include `SHA256SUMS.txt`.
- Update download progress; on macOS and Linux the app restarts after an
  update is installed.
- A deep check skipped at its concurrency limit shows "limit" in the table,
  with a hint, instead of an unexplained empty cell.
- Diagnostics include the OS and the install type.
- Scheduled checks and webhook delivery run in the Rust backend, so they keep
  working while the window is hidden in the tray.
- The dispatcher pool is saved to disk (`pool.json`, owner-only on Unix) and
  restored after a restart.
- Optional ip-api Pro key (HTTPS); the free ip-api endpoint is used over HTTP.
- Redesigned check settings: a public server or ProxPulse Judge as the check
  target, including custom judge paths.
- Confirmation before clearing the pool or the check results.
- The initial UI language follows the system language.
- Only one application instance runs at a time.
- `npm run build:unsigned` for local builds without the updater key.

### Changed

- The Info tab is replaced by Settings: the language switch moved there from
  the header, and the about card became rows linking to the projects. The
  check settings panel is titled "Check settings".
- One parser: proxy lists are extracted by the backend, and every entry must
  pass the same normalization a check uses. The live count in the source
  panel now equals the number of proxies a check will test; entries that
  used to show up as "bad format" are no longer counted or checked, and one
  proxy written two ways is checked once.
- One schedule re-checks the shared pool, replacing the separate dispatcher
  and auto timers; sending the pool to webhooks is an option of it. Saved
  settings of both timers are merged on first start.
- Scheduled checks test liveness, ping and the country only; geo is looked up
  only for pool entries without a country.
- A proxy leaves the pool after two failed scheduled checks in a row. When no
  proxy answers and the check server is unreachable directly (offline, waking
  from sleep), the pool is left unchanged and the check is retried in a
  minute. Scheduled checks never stop the dispatcher.
- Every check sample opens a new connection, so ping and jitter include
  connecting to the proxy.
- A webhook response outside 2xx is reported as an error.
- Manual and scheduled checks share one configuration and the same limits
  (timeout 1–30 s, up to 500 threads, 1–5 samples).
- Start and Stop stay visible at the bottom of the settings column.
- A scheduled check crowded out by a manual check is retried in a minute.
- The Windows portable build is named `ProxPulse_<version>_x64-portable.exe`.
- The Russian UI uses one polite tone; the dispatcher hint names the
  protocols of the selected listener; the tray shows Russian time units.
- Documentation follows ProxPulse Judge: `README.md` is English and
  `README.ru.md` Russian, with install and verdict tables, troubleshooting and
  limitations; `SECURITY.md` defines scope and the security model, and
  `RELEASING.md` describes the release process.

### Fixed

- Fastest and failover dispatcher modes behave differently; Stop closes open
  tunnels; CONNECT tunnel bytes are preserved; idempotent requests fail over
  before any response byte.
- Stop aborts a running check immediately, including the initial baseline
  phase (it used to wait up to 20 s).
- Geo cache eviction keeps recently confirmed entries.
- Copy buttons confirm the copy; a TXT export with dead proxies is no longer
  named "alive"; the empty ping graph frame is hidden; keyboard focus is
  visible and icon-only controls have screen-reader labels.
- Parser: duplicates are detected after normalization, numeric passwords and
  IPv6 forms are handled, malformed input is rejected.
- No panics when truncating non-ASCII error text.
- The updater offers installation only for NSIS, AppImage and macOS builds.
- Exports get the selected extension when the save dialog returns none.

### Performance

- The results and pool tables draw only the rows in view instead of capping
  the list at 1000 rows.
- Large lists are parsed off the UI thread, HTTP clients and the TLS
  configuration are reused, and deep checks are rate-capped.

## [0.4.4] — 2026-10-23

### Added

- CI on pushes and pull requests; security policy.

### Fixed

- Hardened the checker, the dispatcher's upstream handling and the UI.
- The Intel macOS image is named `x64`; the release-notes job runs again.

## [0.4.3] — 2026-09-08

### Added

- Intel macOS build; release notes list which file to download per platform.

## [0.4.2] — 2026-09-08

### Added

- Builds for Windows, Linux and macOS.

## [0.4.1] — 2026-09-04

### Fixed

- Updater artifacts and permissions.

## [0.4.0] — 2026-09-04

- First tagged version: Windows installer and portable build.

[Unreleased]: https://github.com/nechel12/proxpulse/compare/v0.5.0...HEAD
[0.5.0]: https://github.com/nechel12/proxpulse/compare/v0.4.4...v0.5.0
[0.4.4]: https://github.com/nechel12/proxpulse/compare/v0.4.3...v0.4.4
[0.4.3]: https://github.com/nechel12/proxpulse/compare/v0.4.2...v0.4.3
[0.4.2]: https://github.com/nechel12/proxpulse/compare/v0.4.1...v0.4.2
[0.4.1]: https://github.com/nechel12/proxpulse/compare/v0.4.0...v0.4.1
[0.4.0]: https://github.com/nechel12/proxpulse/releases/tag/v0.4.0
