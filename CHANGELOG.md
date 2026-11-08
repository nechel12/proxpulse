# Changelog

All notable changes to ProxPulse. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/); versions follow
[Semantic Versioning](https://semver.org/).

## [Unreleased]

### Security

- A judge's key that came over HTTPS is only replaced over HTTPS during a run,
  so nobody on the direct path can force plain HTTP on the second key request.
- A line with a scheme ProxPulse cannot speak (`vless://`, `trojan://`, `ss://`,
  or JSON whose `type` is `elite`) is no longer taken for a proxy. Such a line
  was "checked" with no proxy at all, straight from your own connection, and
  showed up alive and `elite` with your IP and country.
- SOCKS5 proxies resolve host names through the proxy, as the dispatcher does,
  instead of on your machine, which leaked every checked host name to your DNS
  resolver. A SOCKS5 server that refuses host names now shows as dead, as it
  would fail in the dispatcher, and the ping includes the proxy's own lookup.
- Error texts no longer contain the requested URL, which could hold the ip-api
  Pro key, a Telegram bot token, a subscription URL or your own IP, in the
  table, CSV exports, the status line and notifications.
- Releases are published only after every platform is built: until then the
  release is a draft, so the updater never sees a partial `latest.json`.
- With a judge checked over both HTTPS and HTTP, the HTTP answer, which
  carries the anonymity verdict, is checked against the judge's key even when
  the main address ends in `/generate_204`. Before, no key was fetched and any
  proxy could rewrite the verdict.
- The judge's key is no longer fetched through redirects, which could hand the
  lookup to another host and make its key look delivered over HTTPS. A "this
  judge does not sign" answer that came over plain HTTP now shows the same
  note as a key that did: anyone on the direct path could send it to switch
  signatures off.
- An answer whose signature fails loses its exit IP, country, city, ASN and IP
  type as well as its anonymity verdict, so forged geo no longer reaches the
  pool, the dispatcher's country filter, DNSBL or the geo cache.
- Subscriptions can no longer add addresses on this machine or its local
  network (`127.0.0.1`, `192.168.x.x`, `localhost`, …) to a pool; such lines
  are skipped. Your own lists are not limited.
- The Pool API gives out proxy usernames and passwords only for a pool with a
  dispatcher login. Without one, any user of the computer could read them,
  since loopback is shared.

### Added

- Answers whose signature could not be checked, because the judge's key could
  not be fetched, are marked `?unv`, with `unverified` in exports.
- A note under the status line when the judge's key came over plain HTTP.
- An integrity filter in the results: without rewrites, rewritten (`?mod`,
  `?tls`), or signature not checked (`?unv`).
- Tooltips on the result column headers, and translated tooltips on the `?mod`
  and `?tls` marks.
- A pool backup, `pool.json.bak` (or `pool-<id>.json.bak`) in the app data
  folder, written before a scheduled check removes entries.

### Changed

- The Info column, with the exit IP, the `?mod`, `?unv`, `?tls` and blocklist
  marks, and the reason a proxy is dead, follows Status instead of being the
  last column, off-screen in the default window. The row number and proxy
  columns stay in place while the table scrolls sideways.
- Export and Copy in the export menu take the rows the table shows, with its
  filters and grouping; the status line says "N of M" when filters hid some.

### Fixed

- Errors say what happened again (`timeout`, `connection refused`,
  `proxy auth required`): with reqwest 0.13 the cause was lost, so with the
  default judge every failure read "proxy error". A refused connection is
  recognised on non-English Windows too.
- One proxy answering 403 or 429 to the free ip-api lookup no longer pauses
  geo lookups for the whole run; only the shared Pro key does.
- A scheduled check can no longer empty the pool in one or two cycles. A proxy
  over the max ping is removed only after two slow checks in a row, like a dead
  one. A cycle in which more than half of the pool (at least three proxies)
  fails changes neither the pool nor the history and says so; only when the
  next cycle shows the same is it applied, so a congested uplink or a VPN that
  blocks proxy ports for one cycle costs nothing, and a pool that really died
  is still pruned.
- A manual check where no proxy answered and the check server does not answer
  directly either says so, and is not written to the proxies' history, where
  it marked every proxy down.
- A hidden DNSBL filter no longer keeps filtering after DNSBL is turned off.
- A proxy that failed both as HTTP and as SOCKS5 is recorded in the history
  under its address, like a live one, instead of the raw line: its uptime no
  longer splits in two, and a `host:port:user:pass` line no longer puts the
  password into `history.json`.
- Proxies on one gateway with different logins (`user-country-de`,
  `user-country-us`) no longer share a cached country for six hours; the geo
  cache tells them apart by a hash of the username, never the password.
- Scheduled checks use the pool's known countries instead of looking geo up
  again through every live proxy each cycle, which used up the ip-api limit.
- When your own IP could not be learned, a judge's answer no longer gives an
  anonymity verdict: without it the judge cannot see a leak, so a transparent
  proxy showed as `anon` or even `elite`. The judge's own `/ip` over HTTPS is
  now asked when `api.ipify.org` does not answer.
- The TLS check no longer marks a proxy `?tls` when the certificate it got is
  valid but differs from the direct one, as a CDN's regional certificate does.
  A proxy that intercepts TLS cannot present a valid chain, so only a rejected
  certificate counts; a different valid one is noted in the tooltip.
- Passwords with `/`, `?`, `#`, `@`, `;` or `!` are no longer cut short or
  dropped (`1.2.3.4:8080:user:pa/ss` became `user:pa`, and the proxy answered
  407); they are percent-encoded in the proxy URL.
- Common list formats are read as meant: `1.2.3.4 8080 US elite` is no longer
  a login `US` with the password `elite`, `1.2.3.4:8080 user pass` and
  `1.2.3.4:8080@user:pass` keep their login, tables copied from free-proxy
  sites (more than five columns) are read instead of giving 0 proxies,
  commented-out lines stay out of short lists, a JSON `name` is no longer a
  login, and a JSON `"protocol": "https"` no longer opens TLS to a plain HTTP
  proxy. A bare IPv6 address with a port (`fe80::1:8080`) is refused instead
  of turning into a wrong URL.
- Stop takes effect at once in large, fast runs. It was only noticed when no
  proxy finished for 50 ms, so with hundreds of threads queued proxies kept
  being checked and written to the history as dead.
- The speed test stops at once on Stop, instead of finishing measurements of
  up to a minute and leaving "another check is running" for the next check.
  Its speed no longer counts the bytes that arrived with the response
  headers, which inflated it; a failed TLS handshake keeps the connect time;
  and a slow proxy returns what it measured within a minute instead of a bare
  "timeout".
- SOCKS4 proxies work on machines whose resolver lists an IPv6 address first:
  the TLS check, the speed test and the dispatcher took the first address,
  which SOCKS4 cannot carry, and failed.
- A proxy counts as alive only when a known check address gives its own
  answer: `204` for `generate_204`, the JSON of ip-api, httpbin or the judge,
  the address from ipify, the example.com page. A captive portal or a server
  that answers 200 to everything was alive before, and so was an answer whose
  body failed to arrive. A judge's own "busy" answer (429, 503, 504) no longer
  counts against the proxy, which delivered it; a judge answer without the
  content hash the direct one had is `?mod`; and a site answering 403 or 451
  no longer counts as open.

## [0.6.1] — 2026-11-07

### Security

- The release workflow rejects a tag that is not on `main` or not signed with
  the release key.
- When a judge's key cannot be fetched, its answers no longer count as
  verified: integrity stays empty instead of `ok`, because a proxy can keep the
  content hash and rewrite the verdict.

### Changed

- Release descriptions include the version's changelog section automatically.

### Fixed

- The HTTP judge mode no longer reports anonymity from a judge that redirects
  HTTP to HTTPS, such as Render. It took the verdict from the HTTPS answer,
  usually a false `elite`, with integrity `ok`.
- A judge restarting with a new key during a check no longer marks the rest of
  the run `?mod`: ProxPulse fetches the key once more before treating an answer
  as rewritten.

## [0.6.0] — 2026-11-04

### Security

- Release tags are signed with an SSH key from this version on. The public
  half is `docs/proxpulse-release.pub`; RELEASING.md shows how to check a tag.

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
  for local clients (HTTP Basic and SOCKS5 username/password). The settings
  file carries them; an import or reset applies them to running dispatchers
  at once.
- Notifications when a scheduled check leaves fewer proxies than a threshold,
  and once when scheduled checks find no network.
- Signed judge answers: a judge with `SIGNING_KEY` signs each `/judge` answer
  with a per-request nonce and your direct IP. ProxPulse fetches the judge's
  key directly and marks an answer that fails the check as modified,
  dropping its anonymity verdict.
- Blocklist check (DNSBL), off by default: the exit IP is looked up in
  Spamhaus ZEN, Barracuda, SpamCop and DroneBL; `bl N` in the Info column, a
  filter and a `dnsbl` column in CSV exports. Scheduled checks skip it.
- Named pools: up to ten pools, each with its own proxies, dispatcher,
  schedule, subscriptions and webhooks, switched on the Dispatcher and Auto
  tabs. The settings file carries the named pools, and with **Start the
  dispatcher** every pool's dispatcher that was started before starts at
  launch.
- Pool API, off by default: `GET 127.0.0.1:1090/random`, `/fastest`, `/list`
  and `/pools` hand live proxies to scripts, filtered by pool, country and
  protocol, as text or JSON. A pool with a dispatcher login needs the same
  login, and requests for other host names are refused against DNS
  rebinding. The settings file carries its state.

### Changed

- The judge address is entered without a scheme, with **HTTPS** and **HTTP**
  boxes. With both (the default), liveness, ping and geo use HTTPS and a
  second plain-HTTP request gives anonymity and integrity, so the built-in
  judge reports anonymity again. Saved settings with an `http://` judge keep
  HTTP only.
- Fewer explanations in the UI; the details are in the README.

### Fixed

- Logins and passwords with `%` escapes (`p%40ss`) now reach the proxy
  decoded through the dispatcher, the speed test and the TLS check as well,
  as they already did in the check itself.
- URL imports, subscriptions and webhooks also refuse deprecated site-local
  IPv6 addresses (`fec0::/10`).

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

[Unreleased]: https://github.com/nechel12/proxpulse/compare/v0.6.1...HEAD
[0.6.1]: https://github.com/nechel12/proxpulse/compare/v0.6.0...v0.6.1
[0.6.0]: https://github.com/nechel12/proxpulse/compare/v0.5.0...v0.6.0
[0.5.0]: https://github.com/nechel12/proxpulse/compare/v0.4.4...v0.5.0
[0.4.4]: https://github.com/nechel12/proxpulse/compare/v0.4.3...v0.4.4
[0.4.3]: https://github.com/nechel12/proxpulse/compare/v0.4.2...v0.4.3
[0.4.2]: https://github.com/nechel12/proxpulse/compare/v0.4.1...v0.4.2
[0.4.1]: https://github.com/nechel12/proxpulse/compare/v0.4.0...v0.4.1
[0.4.0]: https://github.com/nechel12/proxpulse/releases/tag/v0.4.0
