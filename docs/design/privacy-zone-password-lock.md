# A password lock for privacy zones

*An unscheduled design note.* Nothing here is built or planned for a release; it records
what a password lock would have to cover, so that a future change starts from the whole
list rather than from the editor. The architecture it builds on is in
[ARCHITECTURE.md](../ARCHITECTURE.md) ("Privacy isolation", "Windows and webviews").

## The ask, and how it relates to privacy isolation

The ask: private files should need a password before their **content** can be viewed.
This is orthogonal to privacy isolation, which makes marked content nonexistent to the
agent and leaves it fully visible to the user; the two would stack. Privacy zones are an
agent boundary, not an interface one: today the WebView receives every private note in
full.

## The fork to settle first

- **An interface lock.** Verified in Rust; while locked, private content never enters the
  WebView; the disk stays plaintext Markdown. It defends against a glance or a shared
  screen, not against someone with the disk, and external editors, sync and `git diff`
  keep working.
- **Encryption at rest.** Real confidentiality, but external tools stop working on those
  files, a forgotten password loses them, and indexing waits for unlocking. If ever built,
  an explicit per-vault or per-folder switch, never the default.

## An interface lock, sketched

Enforcement converges in Rust; the locked state lives in memory (`AppState`); the password
is stored only as an Argon2id hash. Content enters the WebView only through IPC — the main
webview's capability grants `core:default`, `dialog:default`, `updater:default` and
`process:allow-restart`, with no asset protocol, and `convertFileSrc` only ever builds a
`vemry-asset://` address; the chat, capture and browser-bar webviews hold core IPC too, and
the one custom protocol, `vemry-asset://`, only proxies remote images. So the list of
channels is closed, but it is **far longer than the editor**:

- **Full text and fragments**: `note_read`, `file_read`, `file_read_binary`, `resolve_ref`,
  `note_history_read`, `search`'s snippets, `query_blocks`' content, and — least obviously —
  `links_get`'s backlink snippets, which come from the *source* note, so a public note's
  panel shows the private notes linking to it.
- **Metadata that is prose**: `notes_list`'s titles and aliases, which also feed the `[[`
  and `#` completions (filtered in the frontend only — a courtesy, not a boundary).
- **Existence and bypasses**: `resolve_link`, `resolve_ref`, `block_ensure_id`,
  `privacy_list`, `session_read` (text the user pasted is logged verbatim) and
  `file_open_system`. The lock must also lock `privacy_unmark` and the two rule commands,
  since a `!` negation un-privates a path.

Refusing IPC does not clear what the WebView already holds — the CodeMirror document, a
preview's blob URL, search results — and the reload driven by `vault://event` must check
the lock too. The suggested shape is a read-only view in `vemry-core` with a
`hide_private` switch: always on for the agent (the `*_for_agent` reads already are this),
following the lock for the interface, with errors that say "does not exist", not "locked".

## Undecided

- Unlock granularity: the whole vault, a zone, a session.
- Recovery after a forgotten password.
- Per-folder passwords.
- Where the hash lives. In `.vemry/config.json` it travels with the vault, so the lock
  follows the folder to another machine; in the app-config directory it does not. The rule
  that keeps consents out of the vault (a vault must never arrive pre-authorised) does not
  decide this — a lock restricts rather than grants — but a hash that travels can be
  replaced by anyone who can write the folder.
