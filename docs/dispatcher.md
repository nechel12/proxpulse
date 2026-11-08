# Dispatcher

**English** | [Русский](dispatcher.ru.md)

The **Dispatcher** tab serves a local proxy on `127.0.0.1:1080` (or the next
free port) and routes traffic through the alive pool. One port accepts HTTP
and SOCKS5 (detected from the first byte), or only the selected protocol.

| Mode | Behavior |
|---|---|
| Rotation | starts at the next upstream for every connection; up to five attempts |
| Fastest | always the lowest-latency upstream; a single attempt |
| Failover | the lowest-latency upstream first, then the next ones; up to five attempts |

- An HTTPS tunnel (CONNECT) stays on one upstream until it closes; each plain
  HTTP request gets its own connection (`Connection: close`).
- SOCKS5 upstreams resolve site names themselves, so local DNS does not see
  visited domains (`socks4` cannot and resolves locally).
- The pool fills from every check (existing entries are kept), can be
  filtered, sorted, synced and cleared, and is saved to disk between launches.
- Request and error counters, the current upstream and the last error are
  shown live.

**Filter, sessions and login** apply at once and are kept for the next
start:

- **Upstream countries** — only proxies from these countries (codes or names
  as shown in the pool); empty — all.
- **Sticky session** — one upstream per site for N minutes, so logins and
  carts do not jump between exit IPs.
- An upstream that fails to connect is paused for a minute (the **Paused**
  counter) unless no other is left.
- **Login** — local clients must authenticate: HTTP `Proxy-Authorization:
  Basic` (the browser asks) or SOCKS5 username/password.

## Named pools

The menu at the top of the **Dispatcher** and **Auto** tabs switches between
up to ten pools, for example one for scraping and one for work; **+**, **✎**
and **✕** create, rename and delete them. Each pool has its own proxies,
dispatcher (port, mode, filter, sessions and login), schedule, max ping,
subscriptions and webhooks.

- Alive proxies from a manual check go to the pool that is shown.
- Every pool's schedule runs whichever pool is shown. One check runs at a
  time: pools due together take turns.
- Each pool's dispatcher needs its own port. A dispatcher never uses another
  pool's dispatcher as an upstream.
- **Start the dispatcher** at launch starts every pool's dispatcher that was
  started before. The tray shows and controls the main pool.
- Deleting a pool stops its dispatcher and removes its schedule and proxies.

## Pool API

**Pool API** on the Dispatcher tab hands live proxies to scripts over HTTP on
`127.0.0.1:1090` (the port can be changed). Once on, it starts with the app.

| Request | Answer |
|---|---|
| `GET /random` | a random proxy (with `format=json`, one object, not an array) |
| `GET /fastest` | the proxy with the lowest ping |
| `GET /list` | every matching proxy, one per line |
| `GET /pools` | the pools: id, name and size, tab-separated |
| `GET /` | a short help |

| Parameter | Meaning |
|---|---|
| `pool` | pool id or name (any case); the main pool by default |
| `country` | codes or names as shown in the pool, comma-separated |
| `proto` | `http`, `https`, `socks4`, `socks5` or `socks` (both SOCKS), comma-separated |
| `limit` | 1–20000 proxies from `/list` (all by default); `/random` and `/fastest` ignore it |
| `format` | `txt` (default) or `json`: objects with `url`, `scheme`, `host`, `port`, `country`, `latency` |

```sh
curl "http://127.0.0.1:1090/random?country=DE&proto=socks5"
curl -u user:pass "http://127.0.0.1:1090/list?pool=work&format=json&limit=20"
```

- A pool with a dispatcher login answers only with the same login (HTTP
  Basic, `curl -u`), and `/pools` lists it only then. Its proxies come as URLs
  **with their usernames and passwords**.
- A pool without a login can be read by any local process, so its proxies come
  without usernames and passwords. Set a dispatcher login to get them.
- Upstreams paused after a failure are left out unless nothing else matches.
- Errors: `400` for a bad parameter, `404` for no match or an unknown pool or
  path, `405` for a method other than `GET`.
- Requests whose `Host` is not `127.0.0.1` or `localhost` are refused, so a
  web page cannot read the pool through DNS rebinding. A request without
  `Host` (HTTP/1.0 scripts; browsers always send one) is answered.
