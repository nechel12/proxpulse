# Security Policy

## Supported versions

Security fixes are provided for the latest release (0.6.x) and `main` on a
best-effort basis. Older versions are not supported; the built-in updater
moves NSIS, AppImage and macOS installs to the latest release.

## Reporting a vulnerability

Please use GitHub's private vulnerability reporting:

1. Open the repository's **Security** tab.
2. Choose **Report a vulnerability**
   (`https://github.com/nechel12/proxpulse/security/advisories/new`).
3. Include the ProxPulse version, the OS and install type (Settings tab →
   Diagnostics), a reproduction, and the impact.

Do not open a public issue or include working proxies, proxy credentials,
webhook URLs, API keys, or personal data; redact them from logs and
screenshots. If private reporting is unavailable, open a minimal public issue
asking the maintainer for a private channel without disclosing details.

## What to expect

- Acknowledgement within 7 days, best effort.
- Triage and a fix/mitigation plan; timelines depend on severity.
- No bug bounty.
- Credit in the advisory if desired.

## Scope

In scope:

- Parsing of proxy lists, imported files and URL imports; handling of
  responses from checked proxies.
- The guard for user-supplied URLs (webhooks, URL import): local and private
  targets, DNS rebinding, redirects.
- The local dispatchers: request parsing, upstream handling, header and
  credential forwarding, the optional login for local clients.
- The local pool API: request parsing, the pool login and the `Host` check
  against DNS rebinding.
- Judge answer signatures: fetching the judge's key and verifying signed
  answers.
- Handling of proxy credentials, webhook URLs and the ip-api key: exports,
  webhook payloads, pool API answers, logs, the diagnostics report, the
  settings file.
- The WebView: CSP, Tauri capabilities and commands, injection through proxy
  or server data.
- The updater, release signing, and the CI/release workflows.

Out of scope:

- Accuracy of verdicts: anonymity, integrity and IP type are heuristics, and a
  proxy can withhold or forge headers.
- Behavior of the checked proxies and of third-party services (ip-api,
  httpbin.org, example.com, the DNS blocklists). Issues with a judge server
  belong to [proxpulse-judge](https://github.com/nechel12/proxpulse-judge/security).
- What third parties learn by design: the blocklist check sends exit IPs to
  your DNS resolver and the list operators, and a judge sees the checked
  proxies' exit IPs.
- Attackers who already control the user's account: data in the WebView
  storage, the pool files, `prefs.json` and `history.json` is stored
  unencrypted, and the dispatchers and the pool API on `127.0.0.1` accept any
  local process unless a login is set, both by design and documented.
- Missing code signing (SmartScreen, Gatekeeper warnings).
- Social engineering, physical access, and denial of service against the
  user's own machine.

## Security model

ProxPulse is a desktop tool that trusts its local user. Checked proxies, their
responses, imported lists and judge responses are untrusted input: response
bodies are size-capped and rendered escaped. A signing judge's key is fetched
directly, never through a checked proxy, over HTTPS when the judge serves it
and otherwise over plain HTTP; an answer through a proxy that fails the
signature is treated as modified. The key protects against the checked
proxies, not against someone on your own network path to an HTTP-only
judge. User-supplied URLs may reach only
public hosts, with DNS pinned per request: webhooks do not follow redirects,
and every redirect of a URL import is checked like the original URL, so
neither can be pointed at local services. Proxy credentials are sent to
webhooks only when explicitly enabled and only over HTTPS. The dispatchers
and the pool API listen on loopback only, and the API refuses requests for
other host names. Updates are verified with the minisign public key
embedded in the app. Release tags are protected against being moved or
deleted and, from 0.6.0 on, signed with the SSH key in
[`docs/proxpulse-release.pub`](docs/proxpulse-release.pub).

## Safe harbor

Good-faith research on your own machine, proxies and judge instances is
welcome. Do not test against the shared public judge instances or third-party
services in a way that affects other users, and do not access data or systems
you do not own. Follow applicable law.
