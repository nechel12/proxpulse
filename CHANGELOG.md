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
- The updater signing key no longer reaches the platform builds, which run
  npm's and every crate's build scripts: a malicious dependency could have
  read it and signed updates for every install. The builds now run unsigned,
  and a separate job that runs only the pinned Tauri CLI signs the updater
  files and writes `latest.json`. A build can still put wrong code into the
  release it builds, but no longer takes the key with it.
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
- Keyboard use of the results table: the arrow keys select rows (Shift
  extends), the Menu key or Shift+F10 opens the row menu, whose items the
  arrow keys move through. The Import buttons are reachable with Tab, the
  name dialog keeps focus inside and returns it on close, and the status
  filters and pool sort menus are announced to screen readers.

### Changed

- The README is half as long and starts with who the app is for and a link to
  the latest release, with a short FAQ. The check target, deep checks,
  results table, export, dispatcher, pool API, schedules, webhooks, settings
  and local data moved to `docs/`, each page in English and Russian; the
  build dependencies and project structure moved to `CONTRIBUTING.md`.
- The Info column, with the exit IP, the `?mod`, `?unv`, `?tls` and blocklist
  marks, and the reason a proxy is dead, follows Status instead of being the
  last column, off-screen in the default window. The row number and proxy
  columns stay in place while the table scrolls sideways.
- Export and Copy in the export menu take the rows the table shows, with its
  filters and grouping; the status line says "N of M" when filters hid some.
- Start is disabled during a speed test, as during a check: one checker
  serves both, and Start, Stop and "Check now" now follow a single run state.
- macOS builds are signed ad hoc, so Apple Silicon no longer calls an
  unsigned download "damaged". The README describes opening it on macOS 15
  and later (Privacy & Security → Open Anyway), the AppImage's FUSE need, and
  checking a Windows download against `SHA256SUMS.txt` in PowerShell, and
  shows what each webhook format sends.

### Fixed

- A pool's scheduled webhook delivery, with retries up to about a minute, no
  longer holds back the other pools' schedules; it runs in the background.
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
- The dispatcher no longer hangs on an upstream that accepts and stays
  silent: a GET, HEAD or OPTIONS request moves on to the next upstream when
  one does not start answering within 30 seconds, or closes without a byte,
  and that upstream is paused like a failed one. A session sticks to an
  upstream only once it answered, and any connection that moves no data for
  5 minutes is closed, so silent upstreams can no longer use up the 1024
  connection slots. The Pool API stops writing to a client that does not
  read after 10 seconds.
- The schedule survives restarts. Its next run used to be "now + interval"
  at every launch, so a daily check on a machine restarted daily never ran;
  it is now one interval after the last run (at least a minute after launch).
  Miss counts and the lists last sent to change-only webhooks are kept too,
  in `schedule.json`: after a restart the whole pool was posted as added,
  and proxies removed before it stayed with the receiver for good.
- An offline machine is no longer re-checked every minute: retries wait 1, 2,
  4… minutes up to the interval, each first asking the check server
  directly instead of downloading subscriptions and checking the whole pool,
  and a manual check in progress is noticed before subscriptions are fetched.
  **Check now** moves the schedule on, so a scheduled run due seconds later no
  longer checks the pool and posts the webhooks a second time.
- A fractional number in the check settings (2.5 threads) no longer makes
  every proxy dead with a raw error, and 0 no longer silently becomes the
  default: values are rounded and clamped, and the field shows what is used.
- Esc closes an open menu first and does nothing while you type in a field;
  before, closing the export menu with Esc stopped the check, which cannot be
  resumed.
- After a scheduled check with subscriptions was skipped because of a manual
  check, Start stayed locked until the next cycle ended. While a scheduled
  check runs, the Check tab now says which pool it checks and that Stop
  cancels it.
- A click on a built-in preset no longer wipes your own judge address and
  custom check URL; it sets how to check, and keeps where.
- Alive proxies of a check go to the pool shown when it started, even if
  another is picked before it ends, and the Check tab names that pool when
  there are several. A pool still loading after a switch is no longer synced
  as empty, which could wipe it.
- Importing or resetting settings during a check is refused: both reload the
  window, after which Start and Check now stayed locked until the app was
  restarted. After any reload during a check, the buttons unlock once the
  check ends.
- "One per IP" and "One per ASN" no longer hide alive proxies whose IP or ASN
  is unknown (geo skipped at a limit); Geo sorts by the code and city it
  shows and Uptime by the uptime it shows; and a Shift range starts from the
  proxy last clicked, not a row position that changed with a re-sort.
- A problem notification (offline, pool unchanged, webhook failed) is no
  longer lost when the problem first came up while the window was in front.
- DNSBL no longer shows an IP as clean when the resolver fails: each list is
  first checked with its RFC 5782 test entry, a list that does not answer it
  counts as unknown, and results with unknown lists are not cached for an
  hour.
- The window may open only the app's own GitHub pages and the ip-api
  members page, and show message dialogs: the default permissions also let it
  open any URL, reveal folders and show file dialogs, which it never uses.
- The prompts for importing or resetting settings say that the check results
  and the pasted list are lost when the window reloads.
- Statuses match what happens: a check says "Checking 0/N" from the start
  instead of "Parsing the list…" and its empty table no longer asks for a
  list; the speed test reports its own errors and a stop as a speed test;
  Clear and Remove are refused during a check, whose next result brought the
  rows back.
- "Added to the pool: N" counts what entered the pool, not every alive proxy
  (Max ping keeps slow ones out); the button that adds the check's alive
  proxies to the shown pool says so instead of "Sync"; the export template's
  default no longer gives `http://:@host:port` for proxies without a login
  (new `{auth}` placeholder), and its prompt no longer shows placeholders in
  capitals; Copy in the row menu gives the normalized URL, like Export → Copy.
- A proxy measured alive no longer turns into a dead "timeout" with 0
  attempts when the deep checks run out of its time: they are skipped (shown
  as `limit`) and the result stays. A connection through a proxy for the TLS
  check, the speed test and the dispatcher takes at most twice the timeout as
  a whole, not one timeout per step.
- Automation details: a webhook template like `{count} proxies online` is no
  longer sent as JSON (and refused) because it starts with a brace; a pool
  proxy that missed a scheduled check loses its old ping, which kept it first
  in the dispatcher's "fastest" order; sticky sessions find hosts written in
  capitals; export files are written whole and owner-only; and a portable
  copy run next to an installed one no longer offers the installer.
- The built-in updater downloads updates from `github.com` instead of
  `api.github.com`, which allows 60 requests an hour per IP without a token:
  many users behind one address could not update.
- Messages say why: a failed URL import shows the reason (`http 404`, too many
  redirects) and that it is loading; pool sync and rename errors are shown;
  a failed update install no longer says "Check failed"; an empty judge
  address gets its own message instead of the one about `http(s)://`; and the
  backend's error words (`timeout`, `connection refused`, `http 502`) appear
  in the interface's language.
- Check settings say what they do: "Requests per proxy", "Protocol for lines
  without a scheme", "Check the port first" with a visible timeout and unit,
  "Speed test size", "Max ping, ms (0 — no limit)", each with a tooltip.
- A check shows about how long is left once its pace is clear, and its timer
  keeps ticking while no result arrives instead of freezing.
- Building from source needs Node.js 22.13 or 24+, as ESLint 10 does; 22.12
  and the odd releases 23 and 25 were accepted before.

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
