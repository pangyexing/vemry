# Release handbook

## Where the version lives

There is exactly one source of truth for the version number: **`[workspace.package]
version` in the workspace `Cargo.toml`**. Neither `src-tauri/tauri.conf.json` nor
`package.json` carries a version field — tauri-cli falls back to Cargo.toml on its
own (verified). The release workflow's first step is a guard: if the tag and the
workspace version disagree, it fails immediately.

## One-time setup (before the first release)

1. **Generate the updater signing key** (minisign; unrelated to OS code signing,
   and required):

   ```bash
   pnpm tauri signer generate -w ~/.tauri/vemry.key
   ```

   - Put the public key in `plugins.updater.pubkey` in
     `src-tauri/tauri.conf.json` (skip if it is already there).
   - Put the private key's **contents** and its password in the repository's
     GitHub secrets: `TAURI_SIGNING_PRIVATE_KEY`,
     `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`.
   - **Back the private key up offline** (password manager or an encrypted
     volume). Losing it means every installed user stops receiving automatic
     updates permanently, and the only way out is asking all of them to reinstall
     by hand.

2. (Optional) macOS signing and notarisation secrets: `APPLE_CERTIFICATE`,
   `APPLE_CERTIFICATE_PASSWORD`, `APPLE_SIGNING_IDENTITY`, `APPLE_ID`,
   `APPLE_PASSWORD` (an app-specific password), `APPLE_TEAM_ID`. **Releases work
   without them** — the workflow skips signing, and the first launch needs a
   right-click → Open or `xattr -cr /Applications/vemry.app` (say so in the
   release notes).

## Every release

```bash
# 1. Bump the version (the one place it lives)
#    Edit Cargo.toml: [workspace.package] version = "X.Y.Z"
# 2. Commit and tag (the tag must be exactly vX.Y.Z)
git commit -am "release: vX.Y.Z"
git tag vX.Y.Z
git push && git push --tags
```

Then:

3. Wait for the Release workflow's four legs: macOS aarch64, macOS x86_64,
   Windows, Linux.
4. Check the draft release:
   - All artifacts present: two `.dmg`, one `.exe` (NSIS), `.AppImage` / `.deb`
   - A `.sig` next to every installer
   - `latest.json` exists and its platform keys include `darwin-aarch64`,
     `darwin-x86_64`, `windows-x86_64`, `linux-x86_64`
5. Write the release notes (include the Gatekeeper instructions while there is no
   Apple certificate), then publish.
6. **Test the upgrade for real**: install an older version → *Check for updates*
   in the app → download and install → restart and confirm the version.

> Note that the Release workflow only builds; it does not run tests or clippy. "All
> four legs green" therefore refers to **CI**, not to Release — check CI on `main`
> before you tag.

## Building locally

With `createUpdaterArtifacts` on, **a local `pnpm tauri build` also demands the
signing private key** (without it the last step reports "A public key has been
found, but no private key", though the `.app` itself has already been produced).
To build with the key:

```bash
TAURI_SIGNING_PRIVATE_KEY="$(cat ~/.tauri/vemry.key)" \
TAURI_SIGNING_PRIVATE_KEY_PASSWORD="" \
pnpm tauri build
```

The environment variable must hold the key's **contents**: the `_PATH` variant was
measured not to work for signing at package time. `pnpm tauri dev` is unaffected.

## Pre-releases and dry runs

- A tag containing `-` (such as `v0.6.0-rc.1`) is marked as a prerelease
  automatically. The updater endpoint points at `releases/latest`, and GitHub's
  `latest` excludes prereleases, so **an rc is never pushed to existing users** —
  which makes it a good way to exercise the whole pipeline.
- A manual `workflow_dispatch` from the Actions page is a dry run: it builds
  without creating a release.

## Network-exit note

The update check and download are the fifth controlled network exit (see
"The network-exit list" in [ARCHITECTURE.md](ARCHITECTURE.md)): by default it happens only when the
user asks for it; turning on automatic checking in Settings adds one silent check
at startup; downloading and installing always need the user's confirmation; and
the package's minisign signature is verified.
