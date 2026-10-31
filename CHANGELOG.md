# Changelog

All notable changes to ProxPulse. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/); versions follow
[Semantic Versioning](https://semver.org/).

## [Unreleased]

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
  and do not follow redirects that could leak proxy credentials.
- The release workflow gives build steps a read-only token and no longer
  leaves the checkout token in `.git/config`.

### Added

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

### Fixed

- Fastest and failover dispatcher modes behave differently; Stop closes open
  tunnels; CONNECT tunnel bytes are preserved; idempotent requests fail over
  before any response byte.
- Stop aborts a running check immediately.
- Parser: duplicates are detected after normalization, numeric passwords and
  IPv6 forms are handled, malformed input is rejected.
- No panics when truncating non-ASCII error text.
- The updater offers installation only for NSIS, AppImage and macOS builds.
- Exports get the selected extension when the save dialog returns none.

### Performance

- Large lists are parsed off the UI thread, rendered rows are capped, HTTP clients
  and the TLS configuration are reused, and deep checks are rate-capped.

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

[Unreleased]: https://github.com/nechel12/proxpulse/compare/v0.4.4...HEAD
[0.4.4]: https://github.com/nechel12/proxpulse/compare/v0.4.3...v0.4.4
[0.4.3]: https://github.com/nechel12/proxpulse/compare/v0.4.2...v0.4.3
[0.4.2]: https://github.com/nechel12/proxpulse/compare/v0.4.1...v0.4.2
[0.4.1]: https://github.com/nechel12/proxpulse/compare/v0.4.0...v0.4.1
[0.4.0]: https://github.com/nechel12/proxpulse/releases/tag/v0.4.0
