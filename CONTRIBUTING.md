# Contributing

Bug reports and pull requests are welcome. Please keep changes focused and
explain the behavior and tradeoffs in the PR description.

## Development

- [Node.js](https://nodejs.org/) 22.12+ and [Rust](https://rustup.rs/) 1.90+.
- WebView2 on Windows (preinstalled on Windows 10/11); WebKitGTK and the other
  system packages listed in the README on Linux.

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

## Pull requests

- Add or update tests for behavior changes: Rust tests live in
  `src-tauri/src/tests/` and next to the module, frontend tests in
  `src/*.test.ts`.
- Update `README.md` and `README_EN.md` for user-visible changes, and add an
  entry under `[Unreleased]` in [CHANGELOG.md](CHANGELOG.md).
- New UI strings go into both dictionaries in `src/i18n.ts`.
- Do not commit secrets, real proxy lists or credentials, `dist/`, or build
  artifacts.

## Reporting security issues

Follow [SECURITY.md](SECURITY.md). Do not open a public issue with exploit
details.

## Conduct

Be respectful and constructive. By participating, you agree to the
[Code of Conduct](CODE_OF_CONDUCT.md).

## License

By contributing, you agree that your contributions are licensed under
Apache-2.0, the project's license.
