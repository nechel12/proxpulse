# Checking

**English** | [Русский](checking.ru.md)

The check target, the deep checks, the results table and export.
Input formats, settings and verdicts are in the
[README](../README.md#checking).

## Check target

The **public server** mode requests a ready-made endpoint or a custom URL
through every proxy: `google`/`cloudflare` `generate_204` (fastest),
`api.ipify.org` and `httpbin.org/ip` (show the exit IP), `http://ip-api.com/json`
(exit IP and geo in one request), `example.com`, or **Custom URL…**. The URL
that will actually be requested is shown under the list. A proxy counts as
alive only when a known address gives its own answer (`204` for
`generate_204`, the JSON of ip-api, httpbin or the judge, the address from
ipify, the example.com page), so a captive portal or a server that answers 200
to everything is dead; a custom URL is taken on any 2xx. A judge's own "busy"
answer (429, 503) still shows the proxy works, without a verdict.

The **ProxPulse Judge** mode uses a [ProxPulse Judge](https://github.com/nechel12/proxpulse-judge)
instance: the built-in public one (`proxycheck.lmtunnel.com`) or your own
server, entered as a host (`judge.example.com`, optionally with a port or a
path). **Fast** checks liveness only (`/generate_204`); **Full** asks `/judge`
for geo, IP type, anonymity and a content hash and replaces the deep checks.
The **HTTPS** and **HTTP** boxes decide how the judge is reached:

| Choice | Through each proxy | Result |
|---|---|---|
| HTTPS + HTTP (default) | `https://…/judge`, then `http://…/judge` | liveness, ping and geo over HTTPS; anonymity and integrity over HTTP |
| HTTPS | `https://…/judge` | liveness, ping and geo; no anonymity or integrity |
| HTTP | `http://…/judge` | everything over HTTP; proxies limited to port 443 look dead |

> [!IMPORTANT]
> **Anonymity needs plain HTTP.** Inside an HTTPS tunnel the proxy can neither
> add headers nor alter the answer, so every proxy would look `elite`. Over
> HTTP the proxy sees the request, and could also rewrite the answer. The
> judge signs each `/judge` answer together with a random nonce and your direct
> IP; ProxPulse fetches the judge's key directly, not through a proxy (over
> HTTPS when the judge serves it, otherwise over HTTP). An answer that fails the
> check is marked `?mod` and its anonymity verdict is dropped; before that,
> ProxPulse fetches the key once more per run, in case the judge restarted with
> a new one. A judge without `/key` is used unsigned. If the key could not be
> fetched, answers are marked `?unv`; a key that came over plain HTTP brings a
> note under the status line, because it protects against the checked proxy but
> not against anyone on your path to the judge. Your direct IP (`direct_ip`) goes
> only into HTTP requests. A judge that redirects HTTP to HTTPS (such as
> Render) and proxies that allow only port 443 give no anonymity; the proxy
> stays alive.

A self-hosted judge for anonymity must answer over plain HTTP (for example
`docker run -p 80:8000 -e TRUST_PROXY=0 ghcr.io/nechel12/proxpulse-judge`
on a server with a public IP: proxies connect to it from the internet). It
signs with a key generated at startup; `SIGNING_KEY` is needed only to share one
key between replicas or keep it across restarts. Serve `/key` over HTTPS too if
you can: anyone on your own path to the judge can replace a key fetched over
plain HTTP. See the
[judge's guide to signed answers](https://github.com/nechel12/proxpulse-judge/blob/main/docs/signing.md).

> [!NOTE]
> **ip-api: free tier and Pro.** The free endpoint is HTTP-only and limited to
> 45 requests per minute. With **ip-api Pro** and a key, requests use HTTPS:
> checked proxies cannot see the key, but ip-api receives it from the addresses
> of all of them, so a dedicated key is recommended and IP restrictions on the
> key do not work. The free tier is not for commercial use.

## Deep checks

Deep checks run after a proxy is found alive and are off in Judge mode:

- **Geo and IP type** — country, city, ISP, organization, ASN and
  mobile/datacenter/residential, via ip-api.
- **Anonymity** — the headers `http://httpbin.org/headers?show_env=1` receives
  through the proxy, compared with your direct IP.
- **Integrity** — the `http://example.com` body compared with a direct request.
- **TLS** — the certificate fingerprint of `example.com:443` compared with a
  direct connection.

Anonymity and integrity use **plain HTTP**: only then does the proxy see the
request and get a chance to add headers or change the response. Proxies that
allow only CONNECT to port 443 cannot be classified this way.

**Site access** requests the chosen sites (Google, YouTube, Telegram,
Discord, ChatGPT or your own hosts and URLs, up to 8) through every alive
proxy, also in Judge mode. A normal answer (2xx or a redirect) counts as
reachable, a 403 or 451 does not; the **Sites** column shows how many opened
and the tooltip lists each with its time or code.

**Blocklists (DNSBL)**, off by default (**IP blocklists (DNSBL)** under
**Site access**), look the exit IP up in Spamhaus ZEN, Barracuda, SpamCop and
DroneBL, also in Judge mode (IPv4 only, cached for an hour). Scheduled checks
skip them. `bl N` in the Info column means N lists have it; the tooltip names
them, and a filter shows clean or listed proxies. The lookups go through your
DNS resolver, so the list operators see the exit IPs you check. A list that
does not answer within 4 s counts as unknown. Spamhaus refuses queries from
large public resolvers (Google, Cloudflare, Quad9): with such a DNS its answer
counts as unknown, not clean.

At most 8 geo lookups and 64 of each other deep check run at once, and geo
pauses for a minute after ip-api answers 429. A check that hits a limit is
skipped instead of waiting and marked **limit** (a skipped blocklist lookup
shows only in the row tooltip). The geo cache (2000 entries, 6-hour TTL)
avoids repeated lookups for unchanged exit IPs.

## Results table

The table shows every result, however long the list: only the rows in view
are drawn. A click on a column header sorts by it (ascending, descending,
off); the text filter (Ctrl+F) and the **Protocol**, **Anonymity**,
**Country** and **Integrity** menus narrow the list. The Info column, right
after Status, holds the exit IP, the marks below and the reason a proxy is
dead; the row number and proxy columns stay in place while the table scrolls
sideways, and every column header explains itself in a tooltip. A click selects a row, Ctrl+click and
Shift+click select several, Ctrl+A selects all shown. Right-click opens
actions for the selection: copy (with credentials), re-check, speed test, add
the alive ones to the pool, remove from the list (also Delete). **Re-check
dead** checks only the dead proxies again and updates their rows in place.

- **Grouping** keeps the fastest proxy per exit IP or per ASN; proxies sharing
  an exit IP are marked with ×N in the Info column.
- **Speed test** downloads 1–25 MB (set under the check parameters) from
  `speed.cloudflare.com` through up to three proxies at a time and shows
  Mbit/s, with the connect, TLS and first-byte times in the tooltip.
- **Uptime** is the share of checks, manual and scheduled, in which the proxy
  was alive; the tooltip adds the check count, the last time it was alive and
  its average ping. The pool can be sorted by reliability, which trusts few
  checks less.

## Export

**Export** takes the rows the table shows, with its filters, grouping and
order (the status line says "N of M" when filters hid some). It saves `.txt`
(normalized proxy URLs), `.json` (full results) or
`.csv` (table with the original line and URL; the `dnsbl` column holds the
lists that have the exit IP, `clean`, or nothing when not checked) through the
system dialog, or
copies the list. Configs for proxy clients are exported too, fastest first:
Clash (`url-test` group; no SOCKS4), sing-box (`urltest` outbound), a
Proxifier profile whose default rule uses the fastest proxy (no HTTPS
proxies), and a PAC file (up to 50 proxies, then `DIRECT`; PAC cannot carry
logins). **By template** writes one line per proxy from placeholders such as
`{scheme}://{auth}{host}:{port}` (`{auth}` is `user:pass@`, or nothing without a login). The **dead** toggle includes unavailable proxies. A sparkline
under the toolbar shows the average ping of alive proxies across checks.
