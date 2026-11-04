# Releasing

A release is a signed `vX.Y.Z` tag on `main`. The `release` workflow repeats
the CI checks, builds every platform, signs the updater artifacts, and
publishes the GitHub release with `latest.json` for the built-in updater.

1. **Prepare**
   - Update `CHANGELOG.md`: rename `[Unreleased]` to `[X.Y.Z] — YYYY-MM-DD`,
     add a fresh empty `[Unreleased]`, and update the compare links at the
     bottom. The release description is built from this section;
     `check:versions` fails without it or its link.
   - Bump the version in `package.json`, `src-tauri/tauri.conf.json` and
     `src-tauri/Cargo.toml`; run `cargo check --manifest-path src-tauri/Cargo.toml`
     so `Cargo.lock` follows, then `npm run check:versions`.
   - Update the supported version in `SECURITY.md` and the placeholder in
     `.github/ISSUE_TEMPLATE/bug_report.yml`.
   - Run the checks from [CONTRIBUTING.md](CONTRIBUTING.md).

2. **Publish**
   - Push the release commit to `main` and wait for a green CI run.
   - Create a signed tag on that commit and push it:

     ```sh
     git tag -s vX.Y.Z -m "vX.Y.Z"
     git push origin vX.Y.Z
     ```

     The tag is signed with the release key whose public half is
     [`docs/proxpulse-release.pub`](docs/proxpulse-release.pub); ProxPulse Judge
     uses the same key. One-time setup: `git config gpg.format ssh` and
     `git config user.signingkey /path/to/proxpulse-release` (the private key,
     or its `.pub` when the private key lies next to it or is in `ssh-agent`).

   - The tag triggers `.github/workflows/release.yml`:
     - `tag` rejects a tag that is not on `main` or not signed with the
       release key;
     - `verify` runs the CI checks and `cargo audit`, and
       `check:versions` rejects a tag that does not match the project
       version;
     - `release` builds Windows (NSIS, MSI, portable), Linux (AppImage, deb,
       rpm) and macOS (Apple Silicon, Intel) with `tauri-action`, signing the
       updater artifacts with the `TAURI_SIGNING_PRIVATE_KEY` secret;
     - `notes` writes the changelog section and the file-to-platform map into
       the release description and uploads `SHA256SUMS.txt`.

   `v*` tags are protected by a repository ruleset: they cannot be moved or
   deleted. A broken release is fixed with a new patch version, not by
   re-tagging.

3. **Verify the release**
   - The tag's signature is valid (anyone can check this):

     ```sh
     printf 'proxpulse-release namespaces="git" %s\n' \
       "$(cut -d' ' -f1,2 docs/proxpulse-release.pub)" > /tmp/allowed_signers
     git -c gpg.ssh.allowedSignersFile=/tmp/allowed_signers tag -v vX.Y.Z
     ```

   - Every expected asset is attached, and `latest.json` lists
     `windows-x86_64-nsis`, `linux-x86_64-appimage` and both `darwin`
     targets.
   - Downloads match the checksums:

     ```sh
     sha256sum --ignore-missing -c SHA256SUMS.txt
     ```

   - An install of the previous version finds the update (Settings → Check for
     updates), installs it and restarts on the new version.

The updater signing key is a minisign key; its public half is embedded in
`src-tauri/tauri.conf.json` (`plugins.updater.pubkey`). Losing the private key
means existing installs cannot verify further updates, so it must be backed up
outside the repository.
