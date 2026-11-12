# Contributing

Bug reports and pull requests are welcome. Please keep changes focused and
explain the behavior and tradeoffs in the PR description.

## Development

- [Node.js](https://nodejs.org/) 22.13+ (or 24+) and [Rust](https://rustup.rs/) 1.90+.
- WebView2 on Windows (preinstalled on Windows 10/11); WebKitGTK and the
  other system packages below on Linux.

<details>
<summary>Linux build dependencies (Debian/Ubuntu)</summary>

```sh
sudo apt update
sudo apt install -y libwebkit2gtk-4.1-dev build-essential curl wget file \
  libxdo-dev libssl-dev libayatana-appindicator3-dev librsvg2-dev
```

</details>

```sh
npm ci
npm run tauri dev        # run the app with hot reload
npm run build:unsigned   # local release build without the updater key
```

Before sending a change, run the same checks as CI:

```sh
npm run build
npm test
npm run lint
npm run check:versions
cargo fmt --manifest-path src-tauri/Cargo.toml -- --check
cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings
cargo test --manifest-path src-tauri/Cargo.toml --lib
```

`@tauri-apps/*` npm packages and the `tauri*` crates must share a major.minor
version; Tauri refuses to build otherwise. Update them together.

### Project structure

```text
index.html                          # screen markup
src/main.ts                         # UI wiring
src/check-client.ts                 # check runs over a Tauri channel
src/source-panel.ts, parse-runner.ts # list input, import, backend parsing
src/settings-panel.ts, judge.ts     # check settings, judge URLs
src/profiles*.ts                    # presets
src/results*.ts, virtual-rows.ts    # results table: sorting, filters, selection, virtual rows
src/format.ts                       # cell formatting
src/export*.ts, csv.ts              # TXT/JSON/CSV export
src/pool.ts, dispatch-monitor.ts    # pool view and dispatcher status
src/pools.ts, pool-api.ts           # named pools, pool API panel
src/auto-*.ts, webhooks.ts          # schedule, its status and webhooks
src/app-settings.ts, settings-io.ts # Settings tab, settings export/import
src/theme.ts, notify.ts             # theme and system notifications
src/updater.ts, info-panel.ts       # updates and diagnostics
src/export-configs.ts               # Clash, sing-box, Proxifier, PAC, templates
src/sites.ts, speed-client.ts       # site access and speed test
src/history.ts, subscriptions.ts    # per-proxy history, list subscriptions
src/geocache.ts, geo-store.ts       # geo cache
src/i18n.ts                         # RU/EN strings
src/*.test.ts                       # frontend tests (vitest)
src-tauri/src/lib.rs                # backend entry, tray, shared types
src-tauri/src/commands/*.rs         # Tauri commands: check, dispatch, auto, webhook, …
src-tauri/src/extract.rs            # proxy extraction from text (the only parser)
src-tauri/src/proxy.rs              # proxy normalization
src-tauri/src/http.rs               # HTTP clients, body limits, deep-check probes
src-tauri/src/transport.rs          # SOCKS/HTTP CONNECT and TLS to proxies
src-tauri/src/tls.rs                # TLS config and certificate fingerprints
src-tauri/src/netguard.rs           # guard for user-supplied URLs
src-tauri/src/dnsbl.rs              # blocklist lookups
src-tauri/src/judge.rs              # judge answers: direct URLs, body, integrity
src-tauri/src/judge_sig.rs          # judge answer signatures
src-tauri/src/baseline.rs           # direct measurements a run compares answers with
src-tauri/src/tests/*.rs            # backend tests
scripts/*.mjs                       # version check, release description
.github/workflows/ci.yml            # checks on every push and PR
.github/workflows/release.yml       # tagged release builds
```

### Repository notes

- `.githooks/commit-msg` removes AI attribution lines from commit messages;
  enable it with `git config core.hooksPath .githooks`.
- `package.json` overrides `minimatch` to 11.x: eslint and typescript-eslint
  pin 10.x, which an audit advisory covers, and npm's own fix is a downgrade
  to eslint 9. Remove the override once both depend on 11.x or later
  (`npm ls minimatch` shows no 10.x left).

## Pull requests

- Add or update tests for behavior changes: Rust tests live in
  `src-tauri/src/tests/` and next to the module, frontend tests in
  `src/*.test.ts`, tests of the release scripts in `scripts/*.test.mjs`.
- Update `README.md` and `README.ru.md` for user-visible changes, and add an
  entry under `[Unreleased]` in [CHANGELOG.md](CHANGELOG.md).
- New UI strings go into both dictionaries in `src/i18n.ts`.
- Do not commit secrets, real proxy lists or credentials, `dist/`, or build
  artifacts.

## Reporting security issues

Follow [SECURITY.md](SECURITY.md). Do not open a public issue with exploit
details.

## Releases

See [RELEASING.md](RELEASING.md) and [CHANGELOG.md](CHANGELOG.md).

## Conduct

Be respectful and constructive. By participating, you agree to the
[Code of Conduct](CODE_OF_CONDUCT.md).

## License

By contributing, you agree that your contributions are licensed under
Apache-2.0, the project's license.
