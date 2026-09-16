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

2. **macOS signing and notarisation secrets** (in place since 2026-09-16):
   - `APPLE_CERTIFICATE`: the Developer ID Application certificate, exported
     from Keychain Access as a `.p12` **together with its private key** and
     base64-encoded (`base64 -i cert.p12`). A `.cer` downloaded from Apple has
     no private key and makes the keychain import in CI fail.
   - `APPLE_CERTIFICATE_PASSWORD`: the password chosen at export.
   - `APPLE_SIGNING_IDENTITY`: the certificate's full name,
     `Developer ID Application: <name> (<team id>)`, exactly as
     `security find-identity -v -p codesigning` prints it.
   - `APPLE_ID`, `APPLE_PASSWORD`, `APPLE_TEAM_ID`: the account that owns the
     certificate, an **app-specific password** for it (not the account
     password; it is invalidated whenever the account password changes) and
     the ten-character team id (the part in parentheses above).

   The workflow signs, notarises and staples only when `APPLE_CERTIFICATE` is
   non-empty. A release built without the secrets still works, but its first
   launch needs a right-click → Open or `xattr -cr /Applications/vemry.app`,
   and the release notes must say so. After changing any of these secrets, run
   a `workflow_dispatch` dry run (see below) before tagging.

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
5. Confirm that both macOS legs signed and notarised: their logs contain
   `Signing with identity`, `Notarizing Finished with status Accepted` and
   `Stapling app`. A green macOS leg **without** those lines was built with the
   certificate guard skipped (an empty or missing secret); treat it as a failed
   release and do not publish. Until the run as a whole has finished, per-job
   logs are only reachable through the API:

   ```bash
   gh run view <run-id> --json jobs --jq '.jobs[] | select(.name|test("macos")) | "\(.databaseId) \(.name)"'
   gh api --allow-escape-sequences repos/<owner>/<repo>/actions/jobs/<job-id>/logs | grep -E 'Signing|Notarizing|Stapling'
   ```
6. Write the release notes, then publish.
7. Mirror the release to `dl.vemry.app`, the updater's first endpoint (the
   tooling for that lives outside this repository). Until the mirror is done,
   the updater still serves the previous version.
8. **Test the upgrade for real**: install an older version → *Check for updates*
   in the app → *Install update* in the dialog → *Restart now* and confirm the
   version.

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

### Signing and notarising locally

The same build signs and notarises when the certificate is in the login keychain
and these four variables are set (the `APPLE_CERTIFICATE*` pair is only for CI,
where a temporary keychain has to be created):

```bash
export APPLE_SIGNING_IDENTITY="Developer ID Application: <name> (<team id>)"
export APPLE_TEAM_ID=… APPLE_ID=… APPLE_PASSWORD=…
```

The bundles land in the **workspace** target directory, `target/release/bundle/`
(not `src-tauri/target/`). Check the result with:

```bash
APP=target/release/bundle/macos/vemry.app
spctl -a -vv -t install "$APP"   # expected: accepted, source=Notarized Developer ID
xcrun stapler validate "$APP"    # expected: The validate action worked!
```

The `.dmg` is signed but not notarised on its own (the bundler notarises the
`.app`, which is what Gatekeeper checks at launch); `spctl -t open` on the
`.dmg` reporting "Unnotarized" is therefore expected.

> On a machine with only the Command Line Tools (no Xcode),
> `security find-identity -v -p codesigning` can report **0 valid identities**
> although the certificate and its private key are both in the keychain. The
> cause is the missing Developer ID G2 intermediate certificate, which Xcode
> installs on sign-in and nothing else does; without `-v` the identity is listed
> as `CSSMERR_TP_NOT_TRUSTED`, while `security verify-cert` still succeeds
> because it consults the network cache. Import
> `https://www.apple.com/certificateauthority/DeveloperIDG2CA.cer` into the
> login keychain and the identity becomes valid.

## Pre-releases and dry runs

- A tag containing `-` (such as `v0.6.0-rc.1`) is marked as a prerelease
  automatically. A prerelease is mirrored without its `latest.json`, and
  GitHub's `releases/latest` excludes prereleases, so **an rc is never pushed to
  existing users** — which makes it a good way to exercise the whole pipeline.
- A manual `workflow_dispatch` from the Actions page (or `gh workflow run
  release.yml`) is a dry run: it builds without creating a release. It is also
  the way to verify the Apple secrets: the macOS legs' logs must show the
  signing and notarising lines listed under "Every release".

## Network-exit note

The update check and download are the fifth controlled network exit (see
"The network-exit list" in [ARCHITECTURE.md](ARCHITECTURE.md)): by default it happens only when the
user asks for it; turning on automatic checking in Settings adds one silent check
at startup (which opens the update dialog only for a new version the user has not
skipped); downloading and installing always need the user's confirmation; and the
package's minisign signature is verified.
