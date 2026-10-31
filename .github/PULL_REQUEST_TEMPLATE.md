## Summary

<!-- What changes, and why? Keep it focused. -->

## Testing

- [ ] `npm run build`, `npm test`, `npm run lint`
- [ ] `cargo fmt --manifest-path src-tauri/Cargo.toml -- --check`
- [ ] `cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings`
- [ ] `cargo test --manifest-path src-tauri/Cargo.toml --lib`
- [ ] Checked in the running app (`npm run tauri dev`) for UI changes

## Checklist

- [ ] Tests added or updated for behavior changes.
- [ ] `README.md`, `README_EN.md` and `CHANGELOG.md` updated for user-visible changes.
- [ ] New UI strings added to both dictionaries in `src/i18n.ts`.
- [ ] No secrets, real proxy lists, credentials, or build artifacts committed.
- [ ] Security issues are reported via `SECURITY.md`, not this PR.
