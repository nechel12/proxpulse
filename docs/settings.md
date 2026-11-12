# Settings and local data

**English** | [Русский](settings.ru.md)

## Settings, tray and updates

The **Settings** tab holds the app-wide options:

- **General**: language, theme (dark, light or the system one), and hiding
  logins and passwords in the tables; exports and copies keep them.
- **Startup and tray**: what closing the window does (quit, or hide to the
  tray); starting with the system, optionally straight to the tray; starting
  the dispatcher at launch with the last port, mode and listener when the
  pool is not empty.
- **Updates and notifications**: the version and **Check for updates**, a
  quiet check at launch, and system notifications while the window is hidden:
  a scheduled webhook delivery failed, the pool fell below a threshold,
  scheduled checks found no network, an update is out. With several pools the
  title names the pool.
- **Data**: export, import and reset of the settings as a JSON file (it holds
  webhook URLs, the ip-api key, the dispatcher logins and the named pools'
  settings, and the pools with their credentials only when you choose so on
  export; import adds the file's pools up to ten in total and leaves what
  the file does not mention as it is; running dispatchers take an imported
  or reset login and country filter at once, and one whose login that
  removes is stopped rather than left open);
  **Copy** puts a diagnostics report (version, OS, install type, counters,
  recent errors) on the clipboard for bug reports; the error log; clearing
  the proxy history.

**⇲** hides the window to the tray. The tray menu shows or hides the window,
toggles the dispatcher and shows the next scheduled check and the webhook
state. Unless closing is set to hide to the tray, closing the window quits the
app, including the dispatcher and scheduled checks.

Updates come from GitHub Releases; installation shows download progress and
restarts the app. MSI, portable, `.deb` and `.rpm` builds point to the
release page instead.

## Privacy and local data

Network requests leave the machine for: the selected check target; ip-api,
httpbin.org and example.com when deep checks are on; direct (no proxy)
baselines — your IP from `api.ipify.org` (or, when it does not answer, from
the judge's own `/ip` over HTTPS) and the example.com reference — and a
connectivity check when a scheduled check finds no alive proxy; the selected
judge; GitHub
Releases (update check, also at launch unless turned off); configured
webhooks; URL imports and subscriptions; the chosen sites through each alive
proxy; `speed.cloudflare.com` through the proxies of a speed test; your DNS
resolver and the four blocklists with the exit IPs when DNSBL is on.

Proxy credentials go to the proxy itself (HTTP `Proxy-Authorization` or SOCKS
authentication), into exports, and to webhooks only with **Send credentials**.

Stored locally:

- In the WebView `localStorage`: language, theme, settings and presets,
  webhook URLs (may contain tokens), the ip-api Pro key, the geo cache
  (credential-free keys), ping history, recent errors.
- In `pool.json` in the app data directory (`%APPDATA%\com.nechel12.proxpulse`,
  `~/.local/share/com.nechel12.proxpulse`,
  `~/Library/Application Support/com.nechel12.proxpulse`): the pool **with
  usernames and passwords**, owner-only on Linux and macOS (named pools:
  `pool-<id>.json`, their names in `pools.json`). **Clear pool** clears it.
  `pool.json.bak` (`pool-<id>.json.bak`) holds the pool as it was before the
  last scheduled check removed entries; to restore it, quit ProxPulse and
  rename it to `pool.json`.
  `prefs.json` next to it holds the startup options, the pool API port and
  the dispatcher settings, including login passwords, `history.json` the
  per-proxy history (URLs without credentials), and `schedule.json`
  (`schedule-<id>.json`) the scheduled check's state: when it last ran, the
  miss counts, and the lists last sent to each webhook (with credentials when
  **Send credentials** is on), owner-only like the pool.
- Starting with the system adds a login item: the `Run` registry key on
  Windows, a LaunchAgent on macOS, a `.desktop` file in `~/.config/autostart`
  on Linux. Turning the option off removes it.
