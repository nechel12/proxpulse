# Scheduled checks and webhooks

**English** | [Русский](automation.ru.md)

The **Auto** tab re-checks the pool every N minutes; each
[named pool](dispatcher.md#named-pools) has its own schedule:

- Scheduled checks are light: liveness, ping and country; geo is requested
  only for entries without a country.
- A proxy leaves the pool after two failed checks in a row; with **Max ping**
  set (`0` — no limit), two checks in a row slower than it count the same.
- A check in which more than half of the pool (at least three proxies) fails
  changes neither the pool nor the history and says so: that is more often a
  problem on this machine (a congested uplink, a VPN or firewall blocking
  proxy ports) than the proxies dying. Only when the next check shows the same
  is it applied. Before a check removes entries, the pool is copied to
  `pool.json.bak` (see [Privacy and local data](settings.md#privacy-and-local-data)).
- When no proxy answers and the check server is unreachable directly (offline,
  waking from sleep), the pool is left unchanged and the check is retried in a
  minute, then after 2, 4, 8… minutes up to the interval, each time first
  asking the check server directly. A check crowded out by a manual one is
  retried in a minute.
- The schedule survives restarts: the next check comes one interval after the
  last one (at least a minute after launch), with the same miss counts and,
  for change-only webhooks, the same last-sent lists. **Check now** moves the
  schedule on like a scheduled check.
- Scheduled checks never stop the dispatcher.
- While a check runs, the Auto and Dispatcher tabs show how far it is
  ("N of M"), and the Check tab says which pool holds the checker.
- **Subscriptions** — up to 10 proxy list URLs are downloaded before every
  scheduled check; proxies not in the pool yet are checked with it, and the
  alive ones join it. The status shows how many each list had and how many
  were new.

With **Send after each auto-check**, the pool is POSTed to up to 10 webhooks
as TXT, JSON, CSV, a Telegram or Discord message, or a custom template
(`{count}`, `{time}`, `{proxies}`, `{proxies_json}`, `{added}`, `{removed}`
and more; a template starting with `{` or `[` is sent as JSON). **Changes
only** sends just the added and removed proxies, and nothing when the pool did
not change. A scheduled delivery that hits a network error, 429 or 5xx is
retried after 5 and 20 seconds. Credentials are stripped unless **Send
credentials** is on, which requires an `https://` webhook. Webhooks must be
public hosts; redirects are not followed, and a non-2xx answer is shown as an
error.

What a receiver gets (`latency_ms` is the ping, `null` when unknown):

```text
TXT        socks5://1.2.3.4:1080            one proxy per line
JSON       [{"proxy":"socks5://1.2.3.4:1080","latency_ms":120}]
JSON, changes only
           {"added":[{"proxy":"socks5://1.2.3.4:1080","latency_ms":120}],
            "removed":["http://5.6.7.8:3128"]}
CSV        proxy,latency_ms
           socks5://1.2.3.4:1080,120
CSV, changes only
           change,proxy,latency_ms
           added,socks5://1.2.3.4:1080,120
           removed,http://5.6.7.8:3128,
Telegram   {"text":"ProxPulse: 1 proxies
…","disable_web_page_preview":true}
Discord    {"content":"ProxPulse: 1 proxies
…"}
```

The Telegram URL is
`https://api.telegram.org/bot<token>/sendMessage?chat_id=<chat id>`; a long
list is cut to 4096 characters (Discord: 2000).

The scheduler and webhooks run in the backend, so they keep working while the
window is in the tray.
