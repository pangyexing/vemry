# vemry architecture

This is the map of the codebase for a contributor: the layers and where state lives, the key
flows, the security and privacy boundaries with their enforcement points, the network exits,
and the reason behind each decision the code alone does not explain. It describes the
current release; history lives in git and the changelog. Detail a module's header comment
already states is not repeated — the text names the file instead. The Markdown dialect is in
[SYNTAX.md](SYNTAX.md), the development setup and review principles in
[CONTRIBUTING.md](../CONTRIBUTING.md), the terms in [GLOSSARY.md](GLOSSARY.md).

**Contents**

- [First principle](#first-principle-your-knowledge-stays-in-markdown-and-every-view-grows-from-it) — one source of truth, three constraints
- [What lives where](#what-lives-where) — every file vemry writes, and whether it travels with the vault
- [Layers](#layers) — the crates, the glue, the frontend, the windows and webviews
- [Key flows](#key-flows) — an agent turn, an AI write, an external edit, landing text
  from another window, a clipboard paste
- [Security boundaries](#security-boundaries) — every boundary in a line or two, with a pointer
- [The network-exit list](#the-network-exit-list) — the six ways vemry reaches the network
- [Privacy isolation](#privacy-isolation) — the enforcement points and the known limitations
- [The agent](#the-agent) — prompt, references, tools, gates and grants, provenance, sources, skills
- [Programmability](#programmability) — the script sandbox, and why workflows cannot loop
- [The block model](#the-block-model) — block types and block change times
- [The editor and the app shell](#the-editor-and-the-app-shell) — one live buffer, the
  flush funnel, projections, commands
- [The built-in browser and the reading
  projection](#the-built-in-browser-and-the-reading-projection), [Side-by-side
  translation](#side-by-side-translation), [The hosted account and the phone
  inbox](#the-hosted-account-and-the-phone-inbox), [Opening a folder as a
  vault](#opening-a-folder-as-a-vault), [Internationalisation](#internationalisation)
- [Seams left for later](#seams-left-for-later)

## First principle: your knowledge stays in Markdown, and every view grows from it

```
Storage (the single truth)    Semantics (one index)             Projections (a view is a query)
──────────────────────────    ─────────────────────             ───────────────────────────────
Markdown vault           →    typed block index           →     the editor's live preview
  *.md notes, VEMRY.md          Block{id, kind, span}             Agenda / Board / Timeline
  attachments, clips            heading, paragraph, list item,    Claims / Evidence / Graph
                                task, claim, evidence, private,   Review
                                code, quote …                     the agent's query_blocks
                                + link graph and backlinks        a script's vemry.query
                                + tag and field tables
                                + one query()
.vemry/sessions/*.jsonl  →    (read as it is)             →     Session replay
```

Two views come from outside the index: the reading projection shows a web page that is never
written to disk, and side-by-side translation shows the open document cut into segments.
Neither is data. Three constraints follow:

1. **Disk is the single source of truth.** The index lives in memory, is always derived and
   is fully rebuildable; there is no database. The durable files beside the notes — review
   state, history, session logs — are listed in "What lives where", with which are caches.
2. **One `query()` serves three consumers**: the UI's views, user scripts (`vemry.query`)
   and the agent's `query_blocks` tool. Any projection a person can see, a script and the AI
   can compute — no AI-only capability, no UI-only data.
3. **Semantics are a superset of standard Markdown** (the dialect and its fallbacks are in
   [SYNTAX.md](SYNTAX.md)); no private formats. Take the vault away from vemry and it is
   still complete.

## What lives where

| Path | Written by | Kind | Travels with the vault | The agent sees it |
|---|---|---|---|---|
| `*.md` notes and their files | the user; the agent through the gate; scripts with a grant; clips, captures | state — the truth | yes | yes, outside privacy zones |
| `VEMRY.md` | the user (also through the chat's quick memory, `memory_append`); the agent through the gate | state — the vault memory | yes | in the prompt, private blocks cut |
| `attachments/<note path minus .md>/` | a paste, drop or capture into that note | state | yes | with the note's privacy |
| `clips/`, `clips/assets/` | clipping, image localisation | state | yes | yes, deliberately |
| `.vemry/privacy` | the privacy commands, any editor | configuration | yes | no (a dot path) |
| `.vemry/config.json` | Settings and commands: the vault's name, the capture pin, `skills_disabled` | configuration | yes | no |
| `.vemry/types/*.md`, `.vemry/skills/*.md` | the user; "customise" in Settings | configuration | yes | a type's schema; a skill through `read_skill` |
| `.vemry/scripts/*.js` | the user | configuration (code) | yes | no |
| `.vemry/sessions/<id>.jsonl` | the runner | state — the session log | yes | no |
| `.vemry/sessions/<id>.history.json` | the glue, after every turn | state — the sidecar | yes | it is the model's context |
| `.vemry/provenance.jsonl` | the vault, when text lands | state — the review ledger | yes | no |
| `.vemry/history/<note path>/` | the vault, before every write | state — what each write replaced | yes | no |
| `.vemry/script-log.jsonl`, `.vemry/inbox-seen.json` | the script host; the phone inbox | state — audit, seen-list | yes | no |
| `.vemry/block-times.json`, `.vemry/cache/pdf/`, `.vemry/cache/translate/` | the vault; PDF extraction; translation | cache | yes | change dates; PDF text |
| app config: `settings.json` | Settings: preferences, your providers with their API keys, the current chat and translation models, recent vaults, `write_allow_vaults`, `inbox_vault`, window geometry | configuration, consent, secrets (the keys; mode 0600) | no | no |
| app config: `session-consents.json`, `script-state.json` | the gates' standing consents; script flags and grants | consent | no | no |
| app config: `account.json` (0600) | the device token, the entitlement snapshot | secret | no | no |
| app config: `browser_history.json`, `bookmarks.json`, `adblock/*.txt` | the browser | state, rules | no | no |
| app cache: `reading-images/` | the image proxy (256 MB) | cache | no | no |

The app-config directory is the OS's directory for the bundle identifier
`app.vemry.desktop`. Two rules decide the split. What is the user's knowledge, or describes
the vault, travels with the folder. What lets something skip a dialog (the vault-wide write
policy, a session's standing consents, a script's write grant) or identifies the user (the
device token) stays with the app, keyed by the vault's canonical root: a vault copied,
cloned or synced from elsewhere must never arrive pre-authorised. Deleting a cache costs
time or precision; deleting a state file loses review state, history or an audit trail.
`Vault::open` itself creates nothing (`Vault::meta_dir`); `.vemry/` is made when a folder
first becomes a vault.

## Layers

```
src/  Svelte 5 — one bundle, mounted in four webviews of our own
  │   IPC: src/lib/api.ts ⇄ the Rust #[tauri::command]s (changed together)
src-tauri/  the glue: commands, bridges, windows, browser, account, settings
  ├──→ vemry-agent ──┐
  ├──→ vemry-script ─┼──→ vemry-core   (no UI, AI or Tauri dependency)
  ├──→ vemry-clip ───┘
  └──→ vemry-core
```

Dependencies point one way; nothing below `src-tauri` depends on Tauri, so every crate is
tested without a window and core can be reused on its own.

### crates/vemry-core — the vault engine (no UI, AI or Tauri dependencies)

| Module | Responsibility |
|---|---|
| `vault.rs` | Orchestrates disk ↔ parse ↔ index ↔ search, and is the one place paths are checked (see "Security boundaries"). A note write names its writer (`write_note_as`) and lands through a temporary file renamed over the note: a crash leaves the previous text, never an empty file (and a hard link, or a tag the OS keeps on the file, does not follow) |
| `parser.rs` | Line-level scanner: frontmatter, headings, list items, tasks, callouts, fences, wikilinks and embeds, tags, `^blockId`. Every construct is line-level or inline, so no full Markdown AST is needed |
| `block.rs` / `note.rs` | `Block` / `BlockKind`, the atom of every projection; `NoteId` (vault-relative, `/`-separated, NFC); `Frontmatter` (unknown keys kept verbatim, vemry's own under `vemry:`) |
| `index.rs` | `BlockIndex`: notes, blocks, the link graph with derived backlinks, name resolution (stem or alias, shortest path wins), tags; an update reparses one file and re-resolves every link |
| `query.rs` | `BlockQuery` → `Vec<BlockHit>`, the one query entry point; one field-condition grammar (`key = v`) for query blocks, the agent and scripts |
| `search.rs` | The `SearchProvider` trait and an in-memory multi-term AND substring match (no tokenisation, so CJK works by construction) |
| `locate.rs` / `blockid.rs` | The one definition of "which block, which section"; block-id validity, and minting a random id, unique in the note, only on demand |
| `privacy.rs` | Path globs (`PrivacyScope`) and the functions that remove, blank and match around `> [!private]` blocks. See "Privacy isolation" |
| `provenance.rs` | Who authored each block: the normalized-content hash, the `.vemry/provenance.jsonl` ledger, what a write added. See "Block provenance" |
| `history.rs` | A note's text before each write, under `.vemry/history/<note path>/`, named by moment and next writer; autosaves coalesce to one per five minutes unless over another writer's text. Sixty per note, thirty days; a restore is itself undoable |
| `watcher.rs` / `event.rs` | notify with a 500 ms debounce → `Vault::handle_fs_change`; `VaultEvent`: note added, modified, removed, `FilesChanged`, `IndexReady`, and the batch-level `PrivacyChanged`, `ScriptsChanged`, `TypesChanged` (the last two at most once per batch) |
| `types.rs` | Block types from `.vemry/types/<name>.md`. See "Block types" |
| `segment.rs` | Translation units: what the model translates and what is copied through, each structural marker split into a `prefix`; `segment_pages` rebuilds a PDF's paragraphs |
| `taskattrs.rs` / `taskedit.rs` / `repeat.rs` | Task attributes, the line rewrites behind `update_task` and `set_block_field`, recurrence. Mirrored in `views/tasks.ts` and `lib/fields.ts`, with no test comparing them: a change goes to both |
| `insert.rs` / `restructure.rs` / `relink.rs` / `format.rs` | Pure write planning: the landing rule of an append, a clip or a capture; block-level rearrangement; link rewriting on a move; the formatter |
| `audit.rs` / `blocktimes.rs` / `attach_migrate.rs` / `lang.rs` / `weburl.rs` | Link health (`audit_links_for_agent` for the agent); block change times; attachment-bucket ownership; a note's language; the one web-URL normalisation |

### crates/vemry-agent — the AI layer (depends on core, not on Tauri)

| Module | Responsibility |
|---|---|
| `provider.rs` | `ProviderConfig` and `ProviderKind`: one endpoint and one model per request — `Custom` (any OpenAI-compatible endpoint; Zhipu `glm-5.3` on a first start), or `Hosted` with the account's model catalog. The `Vendor`, read off the host, and per-vendor tables of each vendor's current models decide what a request carries: usage on the stream, whether and how a thinking level is spelled (`chat_effort_fields`, `responses_scale`), whether earlier thinking text goes back, search and image support; a model of an earlier generation gets the common protocol and nothing more |
| `client.rs` / `anthropic.rs` / `responses.rs` / `types.rs` | The `ChatTransport` trait — **the LLM channel boundary**: Chat Completions over SSE; the Messages shape, which is DeepSeek's Anthropic-format line (it carries web search) and Anthropic's own API (every request to it); the Responses API of OpenAI, xAI and Meta (every request to them: OpenAI's current models call tools while reasoning only there, xAI's Chat Completions is a legacy endpoint, and Meta's carries no reasoning across turns). `transport_for` is the one place a request's transport is chosen. One read loop, one connection pool, a stream-idle timeout |
| `runner.rs` | The agent loop, at most 25 iterations; session-level tools execute here. See "An agent turn" |
| `prompt.rs` | The system prompt, frozen per session; the user envelope and turn tail; the one untrusted-page envelope (`page_envelope_part`) |
| `context.rs` | `@` references, resolved under the runner's vault lock; `inflate_images` and `scrub_private_refs`, which re-gate every request; `read_file`'s text and PDF reading |
| `tools.rs` | The domain tools: read, write, session-level. See "Tools" |
| `permission.rs` | `PermissionGate` beside `PlanGate`, `ActionsGate` and `ReadGate`, with `WriteRequest` and `WriteGrants`; enforced in the layer that owns writing |
| `undo.rs` / `sources.rs` / `import.rs` | The in-memory write journal behind "undo this turn's writes"; the session's web source table; `import_page`'s composer (a paper's paragraphs with stable `^s<n>p<m>` ids, the model receiving only the outline) |
| `compact.rs` | Two-level compaction: older tool results become recoverable handles (never an ERROR line); then an LLM summary of everything before the current turn, hard truncation as the fallback |
| `session.rs` | The session log and the sidecar; `fork_session`. See "The session log, forks and evals" |
| `skills.rs` | Built-in skills and the vault's `.vemry/skills/*.md`, one listing by name; `read_skill` |
| `translate.rs` | Side-by-side translation's engine (tool-free batches, a streaming parser, the segment-hash cache) |
| `testing.rs` | `ScriptedTransport` for the offline evals, which replay scripted model output through the real runner |

### crates/vemry-script — the scripting layer (depends on core, not on agent or Tauri)

| Module | Responsibility |
|---|---|
| `engine.rs` | A dedicated OS thread owns the QuickJS runtime (64 MB, an interrupt fuse), one context per script; tasks serialised through a channel; the loop is also the scheduler |
| `bindings.rs` | The `vemry.*` surface through one `__vemry_call(method, json)` dispatch; every read and write goes through existing `Vault` methods, so path checks and indexing come free |
| `grants.rs` / `audit.rs` | `enabled` and `write_grant` per script in the app's `script-state.json`, by canonical vault root; `.vemry/script-log.jsonl` |
| `scheduler.rs` | Five-field cron in local time; missed triggers are skipped and coalesce into one `schedule_missed` |
| `lab.rs` | The typed, capped contract of a lab block's controls and views; nothing here executes |

### crates/vemry-clip — the clipping layer (depends on core, no Tauri or UI)

Pure logic: `extract_page` (dom_smoothie Readability, then htmd, the whole page as the
fallback), `fragment_to_markdown`, `compose_clip_note` (`source:`, `domain:`, `clipped:`,
`vemry: origin: clip`), `clip_rel_path` (`clips/<title>.md`, flat — retrieval is by topic,
not by site), `quote_block`, `link_entry`, image localisation helpers. Every URL in the
output is in `[..](url)` or `<url>` form: a bare URL's `#fragment` would be parsed as a tag
(tested).

### src-tauri — the glue layer

| Module | Responsibility |
|---|---|
| `commands.rs` | Most IPC commands (the window, browser, translation, account and inbox modules define their own, all in `lib.rs`'s one handler list) and `broadcast` (below) |
| `agent_bridge.rs` | `AgentEvent` out through a `tauri::ipc::Channel`, each gate's answer back through a oneshot; `AgentState`, the session-scoped state |
| `consents.rs` | A session's standing consents in `session-consents.json`: saved after each turn and on each change, never rebuilt from the session log |
| `script_bridge.rs` | The script host's `UiBridge`: `script://event` pushes; a write grant (300 s) and `vemry.prompt` (120 s) block on a round trip, denying on timeout |
| `browser/` | Tabs (`window.rs`), the detached bar (`browserbar.rs`), clipping (`clip.rs`), ad blocking (`adblock.rs`), the image proxy (`assets.rs`), the reader (`reader.rs`), in-page translation (`pagetr.rs`), history and bookmarks |
| `translate.rs` | Translation's glue: segments the source (a private one refused whole), holds one job, relays `translate://event`, saves through `write_note` |
| `agent_window.rs` / `capture_window.rs` / `tray.rs` | The chat window, quick capture, the resident state; each header walks through its handshakes |
| `menu.rs` | The macOS menu bar. See "Everything is a command" |
| `account.rs` / `inbox.rs` | The hosted account and the phone inbox — network exit 6 |
| `settings.rs` | The preferences, in the app-config directory, never in a vault. Holds the provider list and the two model choices (`ModelRef`: a provider and one of its model ids), and resolves a choice into the `ProviderConfig` a request is built from (`chat`, `translation`) — the one builder, repaired when a provider has gone; migrates a file from before the list |
| `pasteboard.rs`, helpers | macOS NSPasteboard (file paths, image bytes, text); `i18n.rs`, `window_geom.rs`, `imagesize.rs`, `config_store.rs`, `type_templates.rs` |

`broadcast(app, event)` is the single fan-out for vault events: the frontend always receives
them (`vault://event`), `ScriptsChanged` hot-reloads the scripts, note and file events
dispatch to workflows, and `PrivacyChanged`, `TypesChanged` or a change to VEMRY.md drops
the agent's prompt snapshot. The user's IPC writes, the agent's landed writes, clips and the
watcher's external changes reach it; the script host's own writes never do (see "Why
workflows cannot loop").

### src — the Svelte 5 frontend

| Path | Responsibility |
|---|---|
| `lib/api.ts` | The single IPC entry point, a typed mirror of the Rust commands: the two change together |
| `lib/stores/` | `vault.svelte.ts`, the global store: the one funnel for opening a document, the tabs, the save path, vault events; beside it `agent`, `browser` (a mirror of Rust's `BrowserState`), `reading`, `translate`, `viewContext` |
| `lib/editor/` | CodeMirror 6: the live preview, the block model, embeds, provenance marks, query and run blocks, remote images |
| `lib/views/`, `components/views/` | The projections' logic and components |
| `lib/commands/registry.ts` | The command registry, script commands included (`script:<script>:<command>`); a title is `cmd.<id>.title` |
| `lib/host.ts` | `hostDispatch`: an action that needs the main window's editor or clip target, run there or relayed from another window |
| `lib/markdown.ts` | The own-subset renderer for chat answers, embeds and the reading projection |
| `lib/persist.ts`, `lib/i18n/`, `lib/shortcut.ts` | Per-vault localStorage; `t()` / `tn()`; key hints per platform |

### Windows and webviews

| Label | What it is | Content | Capability | What it may do |
|---|---|---|---|---|
| `main` | the main window's webview | our bundle | `default.json`: core, dialog, updater, process restart | holds the editor buffer, the vault store and the relay endpoints — the only webview that lands text in a note; hidden, not destroyed, in the resident state |
| `agent` | the detached AI chat window | our bundle | `agent.json`: core, dialog — the main window's core IPC, a deliberate widening | a projection of the session file; while it is open the main window's chat is a placeholder, so a session has one visible composer; editor actions go to `main` through `hostDispatch` |
| `capture` | quick capture | our bundle | `capture.json`: core | built hidden, only ever hidden; relays its text to `main`; writes only attachments into the target's bucket and a new, empty note picked as the target (`note_create`) |
| `browserbar` | the detached browser's chrome, a child of `browserwin` | our bundle | `browserbar.json`: core, window dragging | relays every clip action to `main` |
| `browserwin` | the detached browser window | no webview of its own | — | holds `browserbar` and the tabs |
| `browser-N` | a tab, a child of `main` or `browserwin`; the warm-up shell is one built ahead | remote pages | none | navigates under a guard, with the ad-block rules |
| `reader` | the hidden reading window | remote pages | none | fetches pages for the reading projection, sharing the browser's web data (a login carries over); no history hooks |
| `reader-agent` | the read gate's hidden reading window | remote pages | none | fetches the pages the agent reads, in a non-persistent data store of its own: no cookie or site storage of the user's goes out, and nothing outlives the run; its own extraction cache; no history hooks |

Capabilities match **webviews** by exact label, never a wildcard: the main window hosts the
docked tabs as child webviews, so a window-scoped capability would hand every remote page
the main webview's IPC. A remote page matches none (`window.__TAURI_INTERNALS__` does not
exist in it). Tauri's ACL does not constrain an application's own commands one by one, so
the command lists in `browserbar.json` and `capture.json` describe rather than enforce: the
boundary that holds is between our bundle and remote content. One bundle serves our four
webviews, mounting its root by `getCurrentWebview().label` (`src/lib/window.ts`), each with
its own stores. Once the main window has a child webview, `get_webview_window("main")`
returns None, so Rust reaches it through `browser::window::main_window`.

## Key flows

### An agent turn

```
agent_send (commands.rs)
  → the session log: the open session's, or a new one
  → the prompt snapshot (SessionSetup: system prompt + plan switch), reused until dropped
  → per-turn inputs: the current model (read afresh: a provider and model id, resolved to an
    endpoint and key), web search (it picks the transport), thinking level, working note,
    formatter, standing grants (consents.rs)
  → runner.run, holding the vault lock: the open and working notes dropped when private;
    @ references resolved through the privacy gate; the history scrubbed (scrub_private_refs)
  → the session log's user row (kind and path of each reference, never a body), then the
    user envelope: the message, the reference envelopes, the turn tail
  → loop, at most 25 iterations: images re-gated (inflate_images) → stream →
      a read tool runs behind the privacy gate
      a write tool is prepared, passes a gate or a standing grant, then lands —
        history, journal, provenance, broadcast (the next flow)
      a session-level tool keeps the plan, the sources, an action list, a read
      results go back; compaction when the context nears its limit
  → end of turn: the citations settle into the source table
  → the glue: the sidecar written, consents saved, a hosted snapshot refreshed
```

Three terms recur. The **turn tail** is the volatile end of the user envelope — the date,
the open note, the working note — kept out of the system prompt so the prompt stays a stable
prefix. The **sidecar**, `.vemry/sessions/<id>.history.json`, is the exact model context,
written whole after every turn, an error or a cancel included; the session log is the audit
view (a tool result keeps only a preview), so the context is never rebuilt from it. The
**turn id** stamps the log's user row, the history envelope and `TurnStarted`; forks and
regenerates locate by it, since counting messages falls out of step after a compaction.

The loop is built to finish: a transient error backs off and retries while nothing has been
produced; a context overflow compacts, then summarises, then retries; a wrap-up hint arrives
with five iterations left; a tool result entering the history stops at 32k characters;
the thinking text goes back where the vendor reads it (DeepSeek's thinking contract) and
nowhere a strict API would refuse the field; a recorded Messages block (a thinking signature,
a search result) goes back only to the line that wrote it (`ChatMessage::blocks_line`), so a
session can change vendor between any two messages; a response a server tool paused is sent
back to resume, and a refusal ends the turn without entering the history. The
`ChatTransport` mock makes every branch testable.

### The AI writes a note

```
the model streams a tool_call(edit_note)
  → prepare_write() computes the diff in memory (nothing touches disk);
    a private target is refused here, before any dialog
  → the session log records write_prepared
  → PermissionGate.request() suspends the loop (unless a standing grant, or the working
    note's auto for a local write, covers it)
  → a Tauri Channel pushes PermissionRequest{diff} → the dialog
  → the user: allow / allow for this session (everything, this folder, or this note)
    / always allow in this vault / edit, then apply / deny — a oneshot wakes the loop
  → allowed: is the note still the text the request was prepared against
    (WriteRequest.baseline)? The gate waits with the vault lock released, and meanwhile
    a blur may save the buffer, a sync client replace the note, or a note appear where one
    was to be created; the approved diff
    would land over text nobody reviewed, so the write is refused and the model reads again
    → vault.write_note_as(Agent): the previous text kept in .vemry/history, the index updated
    → the journal records what the write replaced; record_provenance records the blocks
      the applied bytes added, under the session id
    → AgentEvent::VaultChanged → broadcast like an IPC write (the watcher reads the write
      as the vault's own echo, so this is the open buffer's only news of it)
    denied: an error tool result goes back to the model, which adjusts
  → the session log records tool_result (its permission line went in right after the decision)
```

What each grant covers and where it is kept is in "Gates and grants"; which landed writes
stay unreviewed, in "Block provenance".

### An external edit (another editor can be open at the same time)

```
a notify event → 500 ms debounce → handle_fs_change
  → a note reached through a symbolic link is not indexed (it is not part of the vault);
    one that becomes a link drops out like a removed note
  → the file holds what the index holds? the echo of our own write: ignore it
    (by content, not a time window — a sync client writing the same note right after
    us is a change, and a window would swallow it)
  → reparse the one file, then keep the indexed text it replaced in .vemry/history (who: external)
  → update the index incrementally; a flip of the note's privacy marks the batch
  → the batch's events, PrivacyChanged among them → broadcast → vault://event
  → the UI: a clean buffer reloads as one undoable edit; a dirty one keeps typing and
    is told, once, that its next save lands over that change and where it is kept
```

A non-Markdown file the vault wrote (`write_file`) is the one echo told by a time window.

### Landing text from another window

**Only the main webview lands text in a note.** The editor buffer lives there, and a disk
write to a note the main window has open would be overwritten by its next autosave;
`vault.capture`, `clipTarget.deliver` and `insertAtSection` / `insertAtCursor` know whether
the target is the open buffer or a file on disk (`note_insert`). Every other window relays:

| From | Relay | Lands through |
|---|---|---|
| quick capture (`capture`) | `capture_to_main` → `capture://action` → … → `capture_reply` → `capture://result`, under the request's id | `vault.capture` |
| the detached browser bar (`browserbar`) | `browserbar_to_main` → `browserbar://action` → … → `browserbar://result` | the registry's clip commands |
| the chat window (`agent`) | `hostDispatch` → `agent_window_to_main` → `agent-window://action` | `runHostAction`, the one implementation |
| the phone inbox (Rust) | each item handed to the main window | `vault.capture` — except into a landing vault that is not open |

The phone inbox's exception: when its landing vault is not the vault open in the main
window, Rust opens that vault as a second `Vault` for the moment and writes the items
itself. Nothing in the app watches that vault and no buffer can hold its notes, so the write
is as safe as any external program's — safe for exactly the reason the rule exists.

The capture window lands no text in a note; it writes an attachment pasted into the composer,
saved into the target note's bucket at landing time, and creates a note the user picks as a
new target (`note_create`) — a file no buffer can hold yet. Its pinned target is the vault's
(`capture_pin` in `.vemry/config.json`), since a path names a note in one vault.

**The resident state follows from the rule** (`tray.rs`, on by default, because quick
capture's one key from any application would otherwise end silently at the first close): closing the main
window hides it and a menu bar or tray icon stands for the app, so a main webview — buffer,
vault store, relay endpoints — exists for as long as the app runs, and no second write path
had to be built in Rust. When no tray icon can be created the close button keeps quitting: a
hidden window nobody can bring back must never happen. Every quit goes through the handshake
that writes the buffer out first (see "The flush funnel").

### A clipboard paste (a Finder file or a screenshot → the vault)

Two entry points share `clipboard_paste(dest, stamp, reuse_vault_files)` with different
meanings. **The file tree** (⌘V, or "Paste") is a **placement**: `dest` is the folder
clicked, and a file already in the vault is copied too. **A note's body** (⌘V) is a
**reference**: `dest` is the note's attachment bucket, `attachments/<the note's path minus
.md>/` — unique per note (`noteBucketOf` mirrors `attach_migrate::bucket_for`), private with
a private note, not following a renamed note (a known trade-off) — and a file already in the
vault is not copied; its path is returned.

```
clipboard_paste(dest, stamp, reuse)
  → does NSPasteboard hold file paths? reuse and the source already in the vault →
    its relative path; otherwise Vault::import_files(): an fs copy (folders recursed,
    no bytes through IPC, any size), names deduplicated with -1/-2, a rescan when any
    .md is involved
  → otherwise, image bytes? save_attachment(dest/<stamp>.png|tiff)
  → neither → empty, and the frontend falls back to the File the paste event carried
    (the path off macOS), the editor then to plain text (a copied URL)
  → the tree opens the first new file; the editor inserts a link at the cursor
```

The editor inserts `![](…)` or `[filename](…)`: a `[[wikilink]]` is a semantic reference
between notes, while a Markdown link is a literal, vault-root-relative path, which also lets
`read_file` find an attachment from a body link. File paths take priority over image bytes,
because copying a file in Finder also puts a preview TIFF on the clipboard. The editor must
decide `preventDefault` synchronously while the file path is asked for asynchronously, so
the interception is deliberately loose, and a paste that turns out not to hold files has its
plain text inserted by hand.

## Security boundaries

Each boundary in a line or two; the section named after it holds the mechanism.

- **Paths.** Every path the agent or the frontend hands in passes `validate_rel` (no
  absolute path, no leading `~`, no `..` under either separator, no Windows drive or UNC prefix,
  which joined to the root would replace it) and `refuse_links`: a path that passes through
  a symbolic link below the root is refused by every read, write, move and delete
  (`crates/vemry-core/src/vault.rs`). The index, the listings and the watcher never follow a
  link — the watcher skips a note reached through one — and `Vault::checked_path` applies
  the same rules to the command that opens a file in the OS (`file_open_system`). So a link
  reaches neither a file outside the vault nor a private note under a public name. A new
  path also refuses names Windows cannot hold (`validate_portable_rel`).
- **The permission gate** (the write gate) lives in Rust, beside the ability to write files
  (`permission.rs`); the frontend renders the decision and nothing else. See "Gates and
  grants".
- **Privacy zones** make marked content nonexistent to the agent at six enforcement points
  (see "Privacy isolation"). They are an agent boundary, not an interface one: the WebView
  receives private notes in full.
- **Consents never travel with a vault**: the vault-wide write policy
  (`write_allow_vaults`), a session's standing consents (`session-consents.json`) and a
  script's write grant (`script-state.json`) live in the app-config directory, so a
  `write_policy` key in a vault's `.vemry/config.json`, or an `allow_session` line in a
  session log, grants nothing. See "What lives where" and "Gates and grants".
- **Secrets.** Every request carrying one of your API keys or the hosted device token is
  made in Rust. A key reaches a webview only in `settings_get`'s reply, to our own bundle
  (`settings://changed`, which tells every window to read the settings again, carries no
  payload); the token never reaches one; neither is stored in a vault. `ZHIPU_API_KEY` from
  the environment fills only a Zhipu provider's blank key, so it is never sent to another
  host. `VEMRY_API_BASE` redirects the relay in a debug build only.
- **No bash, no run_code.** Every capability the agent has is a typed domain tool —
  interceptable, renderable, auditable — and it can run neither a script nor a code block.
- **Webviews and IPC.** Our four webviews are matched by exact label; remote pages (the tabs
  and both reader windows) match no capability and get no IPC (see "Windows and webviews"). The
  navigation guard: `on_navigation` allows only http(s) (and a tab's own `about:blank`) and
  refuses the app's own origin; `on_new_window` always denies — a tab reopens an http(s)
  target in itself, a reader window drops it. Clipping is a fixed extraction
  script the app evaluates on a user command, so a page cannot write notes.
- **Our webviews never talk to a remote host.** The app's CSP limits `connect-src` to IPC
  and every other source to the bundle, and allows images only from
  itself, `blob:`, `data:` and the proxy (`vemry-asset:`, spelled
  `http://vemry-asset.localhost` on Windows) — no remote http or https origin
  (`tauri.conf.json`). A remote image appears only through the proxy, and opening a note
  asks the proxy's cache alone (see "Attachments and remote images in a note").
- **The image proxy** (`src-tauri/src/browser/assets.rs`) treats every image URL as
  untrusted: no cookies, http(s) only, every resolved address public, the resolution pinned
  into the request against DNS rebinding, every redirect hop checked again.
- **Untrusted content is enveloped.** A web page reaching the model — handed in, or read
  through the read gate — is wrapped in `<web-page-context source=…>`, which declares it
  untrusted and its instructions not to be followed (`prompt.rs::page_envelope_part`, the
  one place it is assembled). A reference envelope neutralises a closing tag inside its
  body, so a note cannot end its own envelope.
- **The read-consent trade-off.** Under a session's standing read consent, a page the agent
  read could steer it into fetching a URL it composed. What remains is the card listing
  every URL as it is fetched, the stop button beside it, and the consent ending with the
  session. The fetch carries none of the user's cookies or site storage (`reader-agent`), so
  such a URL cannot act as the user on a site they are signed in to, nor read what only
  their session would see. For the same reason an agent write localises only the images on
  its source page's own site (exit 4).
- **Our own renderer.** `lib/markdown.ts` escapes all text first and allows only http(s)
  links and wikilinks, so the XSS surface of chat answers, embeds and the reading projection
  is our own; KaTeX (`trust:false`, only on fragments identified as formulas) is its one
  third-party HTML injection point.
- **The script sandbox**: no fs, net or process, a first write needs a grant, every write is
  audited (see "Programmability"). **The network exits** are a closed list (next).

## The network-exit list

"Offline first" is not "the only network call is the LLM API" but six **controlled exits**,
with everything else usable offline. Adding any outbound request, a convenience fetch
included, changes this list and is a design decision. There is no telemetry.

1. **The LLM channel** (`vemry-agent`: Chat Completions, the Anthropic-format line with
   the provider's server-side search, Anthropic's Messages API, or the Responses API of
   OpenAI, xAI and Meta; the last with `store: false`, so nothing of a conversation is
   kept on their side). A request goes out only for something the user asked
   of the model — a turn sent, a translation started — and carries what the runner
   assembled, already through the privacy gates. A model from one of your providers is
   reached straight at that provider's endpoint; a vemry hosted model, through exit 6.
2. **The browser.** Tabs the user opened, isolated from IPC. The hidden `reader` window is
   the headless form of the same exit, triggered by a click. The read gate has a hidden
   window of its own, `reader-agent`: a page the agent proposes (`open_for_reading`,
   `import_page`) is fetched there once the user approves it on the card, or without a click
   under the session's standing consent, with no cookie or site storage of the user's. The DNS
   prefetch (`browser/window.rs::prefetch_dns`) is a prelude: when ⌘L input pauses, a
   candidate row is highlighted, or `browser_open` runs, the host name goes to the system
   resolver (no HTTP; only names with a dot, once per host per 30 s), exposing which domain
   is about to be visited.
3. **Ad-block rule downloads.** No filter list is bundled (see "Ad blocking"), so the
   subscriptions (`adblock_rule_urls`: EasyList and EasyList China by default) are fetched
   when the first browser or reader page of a run opens with ad blocking on (the default) and
   nothing cached (once per run, riding on the exit just taken at 2), when ad blocking is turned on,
   and when the user clicks "update now". The first two run on macOS only, where something
   consumes the rules; "update now" fetches on any platform. Importing rule files by hand skips this exit.
4. **The image proxy** (`vemry-asset://`). It fetches a projected page's images
   (`reading_images_enabled`), and a note's remote images only once the user lets them load
   (`k=note`, which `reading_images_enabled` does not govern): a click on the placeholder,
   "load every image in this note", the
   `note_remote_images` setting, or a paste. Opening a note asks the cache alone (`c=1`) and
   makes no request. Image localisation reuses the same fetch and downloads once, after
   which the note references local files: when the user clips (every image — a trust level
   equal to looking at the page), when the user runs "save this note's remote images" on any
   note (every image, the note's `source:` as Referer when it has one, none otherwise), and when an agent
   write lands a note whose `source:` is a web address — then only the images on the source
   page's own site (`maybe_localize_page_figures` and `same_site` in
   `src-tauri/src/browser/clip.rs`): the user approved the write and the page, not a URL the
   model composed, which stays a placeholder until the user loads it. All three follow
   `clip_localize_images`; an image in a private block is never fetched.
5. **App updates** (tauri-plugin-updater). By default the check runs only when the user
   clicks "check for updates"; "check at startup" adds one silent query at launch. The
   updater asks `https://dl.vemry.app/latest.json` first and GitHub Releases second (the two
   endpoints in `tauri.conf.json`). Downloading and installing always need a click, and the
   package's minisign signature is verified against the public key compiled into the app
   (see [RELEASING.md](RELEASING.md)).
6. **The hosted account and relay** (`api.vemry.app`, `src-tauri/src/account.rs`), taken
   only after the user signs in. What leaves: at
   sign-in, the e-mail address, the device's name, platform and app version (for the
   account's device list) and the language of the sign-in mail; while signed in, the
   entitlement snapshot's refresh (`GET /v1/me` with its ETag: before a hosted turn when
   older than ten minutes, after every hosted turn, and when the account panel asks), a
   sign-in link for the account pages (`POST /v1/auth/web-link`) when the user opens one,
   and the phone inbox — a long-poll of `GET /v1/inbox/pull`, each held up to 25 s and
   renewed at once for as long as the app runs, paired phone or not, the acks of items
   written, and pairing, listing and revoking phones when the settings page asks; while the
   chosen chat or translation model is a vemry hosted one, the model requests themselves, **byte for byte what bring-your-own-key mode would
   send to the vendor**, plus four headers. The relay is not a new read path: privacy zones,
   the gate and the session log are unchanged and client-side. Signing out closes the exit
   (its last request, `POST /v1/auth/logout`, revokes the device); picking one of your own
   models closes the relay but not the refresh and the inbox, which follow the sign-in. A
   release build always talks to `api.vemry.app`: no environment variable can point this
   exit, or the token it carries, anywhere else.

## Privacy isolation

Any folder, note or single block can be marked a privacy zone: the AI never reads it, never
writes to it, and it never appears in the system prompt — as far as the model is concerned
that content **does not exist**. The user keeps seeing all of it.

**Privacy is a property of the vault, not of a session.** The rules live in
`.vemry/privacy`, a gitignore-syntax glob list compiled by
`vemry-core/src/privacy.rs::PrivacyScope` with the `ignore` crate, case-insensitively (APFS
is case-insensitive, and a path `fs` can open while the glob misses would be a leak). They
travel with the vault and hold against any model endpoint. Four markers:

- a **glob rule** in `.vemry/privacy` (a folder, a note, any file);
- **frontmatter** `private: true` on a note (`NoteSummary.private`), which travels with the
  file;
- a **dot path**: any segment starting with `.` is private by construction, which covers
  `.vemry/` itself — the rules, the session logs — and `.obsidian`;
- a **`> [!private]` block** (`BlockKind::Private`), whose tags, links and fields the parser
  never indexes, so no aggregate — tag counts, backlinks, the graph — carries a trace of it.

`Vault::is_private()` is the union of the first three. A note's attachment bucket
(`attachments/<note path minus .md>/`) shares the note's privacy
(`Vault::in_private_bucket`, consulted by both `is_private` and `is_private_rel`): the
bucket's name spells the note's path and its files are the note's own, so they stay out of
`list_files`, `read_file` and `@` with it — a `.md` dropped into the bucket included, which
is indexed as a note and judged private like the rest. A path through a symbolic link is
refused outright (see "Security boundaries").

### Enforcement points

All in Rust; the agent reaches the vault only along these paths.

1. **Reads mean "it does not exist", not "you may not"**: in `execute_read_tool`,
   list_notes, list_files, search_notes, query_blocks, aggregate_blocks, recent_changes and
   clips_lookup filter out private notes and their blocks; `read_block` and `read_file`
   answer a private target like a missing one; `get_links` shows a link into a private note
   as UNRESOLVED and drops backlinks from one; `read_note` returns a byte-identical `note
   not found` for a private path and a missing one ("not in the index means it does not
   exist", which also closes a bare `fs::read` of a `.md` in a hidden directory). Saying "no
   permission" would disclose that the file exists.
2. **Writes are refused at `prepare_write`**: no diff, no dialog, no `write_prepared` line;
   an error result goes straight back, indistinguishable from `invalid path` or `note not
   found`. Glob rules apply to paths that do not exist yet, so creating a file inside a
   private folder is refused too.
3. **The prompt does not leak**: `build_system_prompt` takes its note count, folders and top
   tags from visible notes only; the open note and the working note are left out of the
   turn tail when private; a private VEMRY.md is not injected; and the prompt snapshot is dropped on every
   privacy change (below).
4. **`@` references pass the same gate**, inside the runner's vault lock at the moment of
   sending (`context.rs::resolve_context_refs`) — privacy can change between picking and
   sending, and only the state at resolution counts. Notes and folders reuse the
   `execute_read_tool` funnel; text and PDF attachments get a gate of the same shape
   (`gate_private`: privacy before existence, worded like `read_file`'s not-found). An
   **editor selection** is the one reference whose body the frontend supplies (an unsaved
   buffer), so it is gated by path and then by content (`gate_selection`, in the table); one
   made inside a private note is dropped whole as "attachment unavailable". **Images** pass
   the gate (privacy before format, existence and size), but their bytes never enter the
   envelope: history stores
   `Part::ImageRef{path}`, and each time a request is assembled `inflate_images` swaps a data
   URL into the request's copy, gating again on the first read of each turn (the read is
   cached for the turn, `image_cache`), so an image deleted or made private later is a
   placeholder from the next turn on; summarisation requests carry none. A **block**
   passes the gate and then `Vault::block_for_agent`; a **view** re-runs its query through
   `query_for_agent`; a **skill** is read by `skills::read_skill` (the `.vemry/skills/`
   exception). The session log's user row records kind and path (a selection's line range, a
   block's handle), never a body.
5. **History envelopes are re-gated every turn** (`context.rs::scrub_private_refs`, at the
   point `inflate_images` runs, plus once over the stored history at the top of each turn,
   so what is persisted is already clean; level-two compaction's summarisation request
   passes the same gate first). A reference's body, once in a user envelope, is frozen in
   history and the sidecar; when it turns private this gate rewrites the envelope to exactly
   what resolution would produce now — a note, attachment, selection or block becomes the
   single "attachment unavailable" line, a folder's body `(no notes)`, and the turn tail's
   open-note and working-note items disappear. Envelopes are located by
   `ChatMessage.context_refs` (kind and path, `serde(skip_serializing)` so it never enters a
   request body, written into the sidecar by hand), with attribute scanning as the fallback
   for older sidecars. With privacy unchanged the bytes do not change, so the prefix cache
   still hits. A retry's replay, a switch to an older session and a fork all pass this gate.
6. **Private blocks are cut out of every read by one set of functions** (`privacy.rs`:
   `redact_private_blocks`, `blank_private_blocks`, `apply_redacted_edit`,
   `private_blocks_preserved`). The vault names the agent's reads — `read_note_for_agent`,
   `search_for_agent`, `query_for_agent`, `block_for_agent`, `memory_file_for_agent`,
   `resolve_ref_for_agent`, `audit_links_for_agent` — so the choke points can be grepped,
   and the raw `read_note` the UI uses is never handed to the agent by a wrong default:

   | Agent-visible path | Function | What is removed |
   |---|---|---|
   | `read_note`, an `@note` reference | `read_note_for_agent` (disk-fresh, re-parsed; no placeholder; one adjacent blank line goes with the block) | the private blocks, counted as filtered |
   | `search_notes` | `search_for_agent` | a hit in a note with private blocks is re-matched over a blanked copy: the snippet cannot come from a private line, and a query satisfied only together with private text drops out (the AND matcher would otherwise confirm private words one term at a time) |
   | `query_blocks`, `aggregate_blocks`, a view reference | `query_for_agent` | private notes and private blocks (a `kind: private` query finds nothing); positional ids are **not** renumbered around the gap — one id space for the UI, the editor and id minting |
   | `get_links` | the index | nothing to do: a private block's links were never indexed |
   | `read_block`, an `@block` reference | `block_for_agent` | a private block is a missing block; a heading's section or an item's subtree is handed over with its private blocks cut out |
   | `audit_links` | `audit_links_for_agent` | a private note itself: a link to it is unresolved, a backlink from it does not count against an orphan, and an anchor or a `supports::` into it is missing — no section can say the note is there |
   | an `@selection` | `gate_selection`, two layers | lines equal to a line of one of the note's private blocks on disk (by content, not line number, so an unsaved edit above cannot shift the cut and a selection starting mid-callout is caught), then the private blocks the selection text itself contains |
   | history envelopes, every turn | `scrub_private_refs` | a note or block envelope whose note now holds private blocks is rebuilt from the same read (`read_note_for_agent`, `block_for_agent`); a folder envelope drops the lines of notes that have turned private; a view envelope drops the rows whose note or block has turned private (`filter_view_block`, read off the index); a selection envelope with a line equal to a private line becomes "unavailable" |
   | the system prompt | `memory_file_for_agent` | VEMRY.md's private blocks; a VEMRY.md write also invalidates the prompt snapshot, so the change applies from the next turn |
   | side-by-side translation (not an agent turn, but the same model channel) | `translate_open`, the segmenter's `private` flag | a private note or file is refused whole; a private block is never enqueued and is left out of a saved translation |
   | `audit_links`' anchors today, a block reference or embed for the agent later (seam) | `resolve_ref_for_agent` | a private target is a missing anchor; a section or subtree is handed over redacted |
   | scripts under `hide_private` | `read_note_for_agent`, `search_for_agent`, `query_for_agent` | the same |
   | `append_note`, `update_task`, `set_block_field`, `restructure_note`, `move_blocks` | `locate_block`, `private_blocks_preserved`, re-checked in `execute_write` | a private block's handle is a missing block; a write that would move a private block to another note or stop it parsing as one is refused |
   | `write_note` | `prepare_write`, re-checked in `execute_write` | refused outright for a note holding a private block ("use edit_note"): the model never saw the block, so its "full content" would delete it |
   | `edit_note` | `apply_redacted_edit`, `private_blocks_preserved` | `old_str` is matched in the model's view of the note — the text `read_note` returned, private blocks cut out — so a passage the model copied always matches, even one spanning a hidden block; the replacement lands in the raw text, a private block that sat inside the replaced span is kept and re-inserted right after the replacement on lines of its own, a block at the edge stays put, and the edit is refused when what the model wrote around a block would stop it parsing as one (a fence opened above it and never closed, its line glued onto a quote) or when it changes nothing (the line break before a hidden block cannot be taken away, and a "wrote" with no change would send the model round again); the diff the user approves is over the raw text, and the size in the "wrote" result is the view's |

**Auditing.** When filtering happens the runner writes
`{"kind":"privacy","tool":…,"filtered":N}` into the session log — a count, no paths. An `@`
reference the gate filtered out does not even get its kind and path into the user row: the
session log lives inside the vault, and recording the path would write the private path back
to disk.

### When privacy changes

A change takes effect at once, however it is made: the marking commands; an external edit of
`.vemry/privacy`; a note whose privacy flips — `private: true` typed into its frontmatter,
in any program — or that moves across a zone's edge. The vault notices a flip when it
reindexes the note or makes the move (`privacy_flipped`), the watcher's next batch, the echo
of the vault's own write included, raises `PrivacyChanged`, and `broadcast` drops the
agent's prompt snapshot; an edit that leaves privacy alone keeps the snapshot and the
provider's cache. Marking a `.md` note writes its frontmatter (so it travels with the file)
and broadcasts that write, so an open editor reloads it rather than saving over it; marking
the open note is refused while its buffer is dirty. Marking a folder or another file writes a rule line.

**The interface does not filter.** The file tree, every projection, embeds and the status
bar show private content, with the privacy badge on each private path (in the tree on the
zone root only). Completion candidates leave private notes out as a courtesy, not a
boundary (a link to a private note still counts as resolved), and a selection touching a
private block is never offered as a chat chip.

### Scripts see what the user sees

Every script binding takes a `Perspective{hide_private}`. All script execution is
user-triggered, so it is always the user's perspective: privacy zones are visible, as in the
interface, and a script the user installed is trusted as the user is, with the grant gate
and the script log as the backstop for writes. The `hide_private=true` filtering — with
errors byte-identical to a genuine absence — is implemented and tested, but no code path
enables it: it is reserved for the day the agent can trigger a script, so the perspective
will follow the trigger. Today the agent cannot, so the two trust domains do not connect.

### Two deliberate exceptions

`.vemry/skills/` is readable by the agent — the point of a skill is to be read — but only
through `skills::read_skill` (the tool, and a skill attached as `/name`), only for `.md`
files directly in it, plus each skill's name and description in the system prompt; a disabled
skill — or a disabled copy, which hides the built-in behind it — is not readable at all.
`.vemry/types/` is the other: the prompt lists each enabled block type's schema — name,
kind, tag, field keys, a state field's steps, description — never a file's body, and
`disabled: true` takes a type out. `read_note` and every other tool still cannot touch
anything inside `.vemry` (a regression test pins this).

### Test baseline

Unit tests in core's `privacy.rs` and `vault.rs`; direct tool tests in the agent layer; and
a full-chain `MockTransport` sweep in which the model reads, searches, queries, follows
links, writes and edits private paths, asserting that a sentinel string never leaks into
history, events, request bodies or the session log.

### Known limitations (deliberate)

- A summary **already generated** by level-two compaction cannot be scrubbed: it is
  model-written text with no envelope structure (its request passed the gate, so only
  content not yet private then can be in it). Content read through tool results is the same
  — a `tool` message carries no reference metadata; level-one compaction or a vault switch
  clears it.
- `privacy_mark` does not scan every session's sidecar (a plaintext file on the same disk as
  the note; the channel worth defending is the request body); another session is cleaned by
  the top-of-turn scrub when it is continued.
- An older sidecar without `context_refs`, combined with a file name holding `"`, `<`, `>`
  or a newline, defeats attribute scanning, and the scrub misses it.
- A selection envelope has no disk source and is re-gated by line content, so a passage
  edited and then wrapped in `[!private]` after it was attached stays in history.
- A session log's `tool_call` rows hold the path arguments the model itself wrote.
- Unmarking removes only the exact rule line, so a broader rule may keep the path private.
  After a rename the frontmatter marking follows the file; a glob rule does not.
- Writing to a private path errors while writing to a public one opens a dialog — a
  detectable difference, accepted, since the error result is a design requirement. The
  `write_note` refusal for a note holding a private block, a gap in positional block ids and
  a re-gated selection's "no longer available" line are the same class: the model learns
  that something is hidden, never what.
- A private block in a never-saved note, selected from its middle: Rust cannot see the block
  on disk and the selection has no `[!private]` head line, so the editor refuses that
  selection itself. Rust stays the real gate; the editor covers this one gap.
- `search_notes` line numbers are those of the raw text and match `read_note`'s only when no
  private block precedes the hit; the model edits by text, not by line.
- The search index keeps every note's raw text in memory; the agent's hits are re-matched
  over a blanked copy. The `write_prepared` diff is over the raw text: the user sees the
  private block in the dialog and in replay; the model never does.
- A private block's tags and links are absent from the graph, backlinks and tag counts for
  the user too — the price of indexing them nowhere.
- A hard link is the same file under a second name, which no check can tell from an ordinary
  file: a note private by a glob rule and hard-linked to a public path is public there (a
  frontmatter marking is content and holds under both names); `refuse_links` covers symbolic
  links only.
- A private block typed into a note between `edit_note`'s prepare and execute makes the
  execute refuse, whether or not the user rewrote the proposal (a rewrite must also keep a
  block that appeared meanwhile, `late_private_blocks_kept`); a change to the public text in
  that window is refused as well, since every write is checked against its baseline.
- The interface does not filter: an embed of a private block renders in the editor.

## The agent

### The system prompt and its session snapshot

The system prompt is fixed rules + VEMRY.md (capped at 8k characters, truncated past that
with a pointer to index it) + a skills summary + the block types' schemas + a vault overview
(note count, folders, top tags, from visible notes only) + the interface language, in which
a message with no prose of its own is answered; otherwise the model follows the user's
language. `print_system_prompt` (an ignored test) prints it.

The prompt is **frozen as a session-level snapshot** (`SessionSetup`): a prompt rebuilt
between turns guarantees a miss of the provider's prefix cache, and on DeepSeek a hit costs
roughly a thirtieth of a miss (Anthropic's cache is asked for on every request, and reads
cost a tenth of the input price). A Claude thinking block is also bound to the conversation
before it; vemry's own compaction rewrites old tool results, so a block that invalidated is
dropped by the API (`drop_block`) rather than failing the request. It is dropped, and the next turn rebuilds it, by a change to
what is private, a write to VEMRY.md, a block-type change, or a skill switched, copied or
added — privacy outranks the cache. Volatile facts ride the turn tail. The plan-confirmation
switch is frozen with the prompt, since it adds a tool. Web search is read afresh every turn
instead: one sentence of the prompt, true either way, says search exists only when a
web_search tool is listed, so a switch changes only the tool table and costs the next
message a cache miss — the price of "click it and it is on" rather than "start a new
session". The thinking level is a per-turn parameter outside the prefix too.

### Context references

The user hands in a **note**, a **folder**, a **file** (text, or a PDF as extracted text),
an **image**, an editor **selection**, a **block**, a **view** or a **skill**, or a **web
page** (the browser's AI button, the picker's "the page in the browser"). All resolve at
send inside the runner's vault lock, through the gates of "Privacy isolation", point 4; the
session log records kind and path only, and the body exists only in the model's history.

- A **block** resolves through `Vault::block_for_agent` (a heading hands over its section, a
  list item its subtree) into `<block-context path="…" block="^id">`, whose intro says how
  to cite it. A selection inside one block carries the same handle, so "ask the AI about
  this block" yields a citable answer.
- A **view** is what a projection shows now: the view registers its live query
  (`stores/viewContext`), and Rust re-runs it through `query_for_agent` into
  `<view-context>`, one block per line with its citation handle — the user's table made
  citable, minus the private blocks, not a new read path.
- A **web page** is the article extracted from the tab (`extract_page`, cut at 20 000
  characters) in the untrusted envelope; the session log keeps its URL and title.
- An **image**: whether the model can see is the endpoint's business; a model the app knows
  cannot (`ProviderConfig::images_unsupported`) gets a toast at attach time.
- A **PDF** is extracted once and cached under `.vemry/cache/pdf/`, which also makes it
  searchable by `search_notes`; the search re-checks each cached PDF's path, so a private one
  is skipped and counted (`context.rs::cached_pdf_files`).

### Tools

- **Read-level** — list_notes, list_files, read_note, read_file, read_block, search_notes,
  get_links, clips_lookup, recent_changes, aggregate_blocks, query_blocks, audit_links,
  read_skill — execute behind the privacy gate. `aggregate_blocks` counts (per field value,
  per note, per ISO week) because `query_blocks` stops at 200 rows.
- **Write-level** (`is_write_tool`) — write_note, edit_note, append_note, set_frontmatter,
  update_task, set_block_field, restructure_note, move_blocks, move_note, move_file — are
  prepared into a `WriteRequest` (the diff and the exact content that would land) and
  execute only after the gate. They write as the user does: `append_note` lands by the
  vault's rule (`insert.rs`; a missing heading is an error, never a silent fallback);
  `update_task` and `set_block_field` go through `taskedit`, so a task the AI ticks lands
  byte for byte as one dragged on the board; `set_frontmatter` refuses the `private` and
  `vemry` keys; a move is the file tree's own `Vault::rename_path`.
- **Session-level** (`is_session_tool`) — update_plan, list_sources, propose_plan,
  propose_actions, open_for_reading — touch no note and pass no write gate.

`import_page` is both: its page passes the read gate, and the note `import.rs` composes
lands as a write_note — a paper archived without its text passing through the model.

### Gates and grants

Four gates suspend the runner until the user decides. Each is a trait in `permission.rs`
with a channel implementation in `agent_bridge.rs`: the frontend renders the card, Rust
decides.

| Gate | Raised by | The card offers | The model gets back | The session log records |
|---|---|---|---|---|
| write (`PermissionGate`) | a write tool's `WriteRequest` | allow; allow for this session — everything, this folder or this note; always allow in this vault; edit, then apply; deny | the write's result, or an error result | `write_prepared`, `permission` (decision, `scope`, `via`, an edit's `applied_diff`) |
| plan (`PlanGate`) | `propose_plan`, when the plan is the user's to steer | run it; edit; reject | the confirmed list, published as the first `plan_update`; a rejection ends the turn | `plan`, `plan_decision`, `plan_update` |
| action list (`ActionsGate`) | `propose_actions` with the whole list | untick rows; correct dates, owners, targets, fields; "write N items"; discard | the kept list, or "write nothing" | `actions`, `actions_decision` |
| read (`ReadGate`) | `open_for_reading` (at most five URLs), `import_page` | a checkbox per URL; decline; "read without asking" for the session | each page in the untrusted envelope, or "work from what you have" | `read`, `read_decision`, `read_auto` — URLs only |

Rules shared by the four:

- **A refusal is one click and carries no reason** — a deny, a reject, a discard. What
  should change is the user's next message, so the model neither revises blind nor spends a
  call asking (a rejection that carries a reason, from a scripted gate, has the model revise
  and propose again within the turn). A rejected plan ends the turn (the batch's calls get
  their results, so the
  history never dangles); a discarded action list tells the model to write nothing and not
  propose it again unless asked.
- **A gate resolved by a stop** (`permission::CANCELLED`) words its result as a
  cancellation: a "revise and propose again" left in the history would make the model
  re-propose on whatever the user asks next.
- **A gate waits with the vault lock released**, so `execute_write` checks the write again,
  against its baseline and against privacy.

**Grants** let a write pass without a dialog:

- **Allow for this session** is a `WriteGrants`: the `all` flag, or path prefixes from the
  scoped form (the note's folder, or the note itself), so a tidy-up of one project runs
  without a dialog per note while a write elsewhere still asks. A write is covered only when
  every note it touches is (`WriteRequest::paths`: a move's two ends, a block move's
  destination, the referrers it relinks). It holds across a session switch and a restart
  because it is kept in `AgentState.session_grant` and in `session-consents.json`
  (`consents.rs`), saved after each turn and on each change — never rebuilt from the session
  log, which lives in the vault, where a copied vault or a collaborator's commit could have
  written an `allow_session` line. The log's `permission` lines stay the record (a write a
  scoped grant covered is marked `via: scope`; one the `all` flag covered carries no
  `via`). A grant given on a dialog covers the next writes, not the one
  the dialog reviewed. A fork inherits its source's consents.
- **Always allow in this vault** is `write_allow_vaults` in the app settings, by canonical
  root, read when a turn starts; revoking it clears the open session's grant at once (a
  `grants_cleared` line), while a running turn keeps the policy it started with and
  per-write approval resumes from the next turn.
- **The working note.** A session can **follow** a note — "keep your findings in this one"
  (`agent_follow_set`). The runner names it in the turn tail, privacy-filtered like the open
  note, and with `auto` on **its local writes pass without a dialog**: append_note,
  set_frontmatter, edit_note, update_task and set_block_field on that one path
  (`is_local_write`) — never an overwrite, a restructure, a move or another note. The path
  is a `follow` line in the session log; `auto` is a consent in `session-consents.json`.
  Each such write is marked `via: follow`.
- **Read without asking** is the read gate's standing consent (`ReadGate::standing`, asked
  before a card is raised). It is not limited to URLs already in the source table, which
  settles only at the end of a turn — such a limit would be a dead letter. The trade-off is
  in "Security boundaries".

A write a standing grant let through stays **unreviewed** in the editor; one approved on the
dialog is accepted, because the dialog was the review (see "Block provenance").

The read gate fetches through the reader's extraction in a window of its own
(`page_read_for_agent`, label `reader-agent`: the tabs' navigation guard and ad-block rules,
the reader's image, media and font blocking, and a non-persistent data store, so none of
the user's cookies or site storage goes out with an agent's fetch and nothing it sets is
kept; its own extraction cache, apart from the reading projection's), a long page in parts of
about 20k characters; a later
part, or a page already read this turn, is served from the turn's cache with no second fetch
and no second card. A page read joins the source table as verified. An action list's
proposal touches no note: the
rows the user kept are written afterwards by the ordinary write tools under their own gate
(an item with no home is a staying row, `stay`, unticked until given a target, so "write N"
counts only the moves the user asked for);
a target under an `attachments/` folder is refused (`vemry_core::in_attachments`), since
such a folder holds a note's files, not homes.

### The plan

The plan is **session state the model keeps through a tool**, never text it recites:
`update_plan` takes the whole step list each time, the runner appends a `plan_update` line
and emits `PlanUpdated`, and the sticky plan card is a projection of the latest list.

**The plan gate opens only when the plan is the user's to steer**: a request they typed, at
high or max thinking (an unset level keeps the gate). A skill the user chose has its plan in
the skill's own Plan section, and a plan at a light level arrives in seconds and is cheap to
stop; for both, `execute_plan` confirms the list itself (`plan_auto`, with the reason, and
no `plan` line, so replay raises no card that looks pending).

**The plan phase bends the thinking level.** With plan confirmation on, at high or max (a skill
turn included, though its plan confirms itself), the calls before the plan is settled go at
"low" (`light_effort`): naming steps needs no deep
thinking, and the design belongs after the user's decision — a high-effort plan call was
measured spending minutes reasoning before a three-line list. A light response is kept only
when every call in it is a planning move (`planning_move`); otherwise it is withheld, an
`effort` line is logged, and the request retaken at the session's level. No answer and no
write is ever produced at the light level.

### Block provenance

The gate is consent *before* a write; provenance is review *after* it. "Allow for this
session", a turn that writes five notes, an edit-then-apply where one line of fifty was read
— all land without being read line by line. So every block can say who wrote its current
text, and a block the agent wrote stays **unreviewed** until the user accepts it.

Nothing is added to the notes. `.vemry/provenance.jsonl` is an append-only ledger written
when text lands: an `agent` line for a write that passed without a dialog (state `pending`;
with the blocks the applied bytes added, and `hunks` so the write can be put back); an
`insert` line for text the user approved or placed — a write approved on the dialog, a chat
answer inserted, a clip, a capture — accepted from the start; an `accept` line (a later agent
write of the same hash makes the block pending again). Some origins need no ledger line
(`provenance::derive_origin`: the ledger first, then a capture stamp or a clip's credit line
in the text, then the note's `vemry.origin`, else the user).

**Identity is a hash of the block's normalized text** (markers, checkbox, callout head,
trailing ` ^id` and capture stamps stripped, whitespace folded), so provenance survives a
block being moved, indented or turned into another kind, and is dropped the moment its words
change — an edited AI block is the user's. What a write added is a multiset difference of
hashes, so a block merely moved is not new. The ledger keeps no note text beyond a pending
line's hunks, skips private paths, and above 5000 lines is compacted to the hashes that
still exist. In a note the ledger knows, a block without a record is the user's. A known
limit: a pending block moved into another note keeps its state but escapes the per-note count.

**Putting a write back.** `provenance_revert_plan` takes the editor's buffer and returns
byte replacements — the hunk whose added lines hold the block, put back to what it replaced;
a hunk not found exactly once is reported as missed, never guessed at. The command writes
nothing: the editor applies the plan as one undoable edit. The block is the unit of
accepting, the line the unit of showing, the hunk the unit of deciding. The editor
(`src/lib/editor/provenance.ts`) re-reads provenance only while the buffer is clean — a
dirty buffer's blocks are not the ledger's yet — so the marks show disk truth with the
autosave lag.

### Moving with links, undoing a turn, and restructuring

A tidy-up is the write path most likely to lose something, so three rules hold it:

- **A move rewrites the links that pointed at it, in the same write** — `move_note` every
  wikilink that resolved to the note, in the author's spelling; `move_file` embeds and
  Markdown links. The rewrites are planned at prepare time and are the move's diff in the
  dialog; the rename lands first and each rewrite second; a note changed meanwhile
  is skipped and reported, since a stale link is the lesser evil than a stranger's edit
  overwritten. A private note is never rewritten, so a link inside one goes stale.
- **A turn's writes can be undone** (`undo.rs`). Every landed write is journaled with what
  it replaced; undo walks the records backwards — a note still holding what the write left
  gets its previous text back, a created note goes to the trash, a move moves back — and
  skips and counts anything edited since. The journal is in memory with the session; after a
  restart the per-block revert and the note's history (`history.rs`) remain.
- **A reorganisation is a block-level operation** (`vemry_core::restructure`). A whole-note
  overwrite retypes every block, and is refused for a note holding a private block. So
  `restructure_note` and `move_blocks` move units (a heading, a paragraph, a list item with
  its subtree) byte for byte under three invariants: a **private block is an anchor** (it
  travels with the public unit it follows, never alone, never into another note); **ids
  travel with their blocks** (a cross-note move rewrites `[[from#^id]]` to `[[to#^id]]`);
  **nothing disappears silently** (a unit the outline does not name lands under "## Unassigned"
  unless it is listed as removed). Both carry
  a block-level diff (`block_diff`), which the dialog opens on.

### The Markdown formatter

Format is code's business, not the model's. `vemry_core::format_markdown` is deterministic
and idempotent (rules in `format.rs`'s header); it leaves code, maths, private and quote
callouts and frontmatter values alone, and changes prose characters only on opt-in, since
changed bytes move the hashes behind provenance and change times. It runs only where asked:
the format-note command; an agent's whole writes, before the gate shows the diff (an edit
fragment, a task line or a field is left as written, so an agent never reformats the user's
lines); a clip or a rich-text paste. Never on autosave.

### Citations, sources and the research loop

- **Block citations.** The model cites `[[path#^id]]`, or `[[path#bN]]` with the positional
  handle `query_blocks` returned. Positional ids are never written to a note, so every path
  out of the panel — copy, insert, pin, save as note — goes through `exportAnswer`, which
  mints a real id (`block_ensure_id`), links a heading by its text, and degrades a block
  that cannot carry an id to `[[Note]]`. A block may move between the answer and the click,
  so each positional citation's content is captured when the answer arrives and checked on
  use; a mismatch falls back to the one block that still holds that text (`src/lib/citation.ts`).
- **Web citations share numbers with the sources**: the panel numbers superscripts by the
  same rule the source table settles by (`extractCitations` in `lib/markdown.ts`, mirrored by
  `sources.rs::extract_citations`). Every link click goes through
  `url_open` (the reading projection by default; ⌘click a real browser tab), and
  `clips_lookup` turns a URL already clipped into its local note.
- **The source table** (`sources.rs`, rebuilt from the session log's `sources` lines). At
  the end of a turn the runner settles the turn's citations, plus every page the read gate
  fetched, with a verdict — verified when a search ran, the user handed the page over or the
  read gate fetched it; otherwise **unverified**, a URL from the model's prose alone. The
  card takes that list as authoritative, a regenerate drops its turn's sources, and
  `list_sources` reads the table back, beyond compaction's reach.
- **The agent fetches nothing by itself**: its ways online are the provider's search, the
  browser in the user's hands, and the read gate.

### Web search

Search is the **provider's server-side** `web_search` tool: no search vendor in the app, no
second key, and search stays inside the LLM channel. The carriers are `AnthropicTransport`,
because DeepSeek's Anthropic-format API and Anthropic's own are where those providers run
search, `ResponsesTransport` on OpenAI's, xAI's and Meta's, and Chat Completions on Zhipu's, which
takes a search tool of its own for GLM-5.3 (`ToolDef::Raw` passes the server tool through —
as the filtering variant on the Claude models that take it, in Zhipu's shape with a
prompt that has it cite by link; `ChatMessage.extra_items` replays its blocks across turns,
to the line that wrote them). One predicate, `ProviderConfig::web_search_available` — a
DeepSeek, Anthropic, OpenAI, xAI or Meta host, or GLM-5.3 on a Zhipu one, on a custom
provider, the catalog's `search` flag on the hosted one — drives the composer's switch (off
by default), the per-turn check and the transport choice. The session log keeps queries and links, never page
text. **A hit is not a source**: only the answer's citations settle. The tool body stays
parameterless — whether a domain filter takes effect is unmeasured, and a control that does
nothing is worse than none. Search passes no write gate: that boundary is the write path.

### Skills

Two layers of playbook, one listing. The built-ins ship compiled into vemry-agent
(`crates/vemry-agent/skills/*.md`); "customise" writes a built-in's file into
`.vemry/skills/`, and the copy shadows it **by listed name**. So a binary update reaches
every vault that has not customised the skill, while a customised copy is the user's for
good. A built-in is switched off by `skills_disabled` in the vault config, since there is no
file to edit. Every switch, copy and new skill drops the prompt snapshot.

VEMRY.md is the resident constitution; skills load on demand, two ways. The model's: the
prompt lists names and descriptions, with the rule that a matching skill is read **before
the plan** (its Plan section then is the plan), through `read_skill`. The user's: `/name` at
the head of the draft becomes a `ContextRef::Skill`, injected first among the envelopes.
Both write a `skill` line, and the method stays in hand until the task ends: compaction
never turns a `read_skill` result into a handle, and the summary keeps the skill and its
plan's unfinished steps. `read_skill` matches a name exactly against the scanned list and
never builds a path from caller input, so it is immune to traversal by construction.

### The session log, forks and evals

The session log (`.vemry/sessions/<id>.jsonl`, the interface's audit log) records every
message, tool call, diff and gate decision, and on each answer the model id that gave it (the
current model can change between turns; the endpoint is not recorded); it is immutable — a retry, an edit-and-resend, a
compaction and a fork are markers the presentation folds on. `fork_session` copies the log's
prefix up to a turn id; the fork command (`agent_session_fork`) writes the matching cut of the
sidecar (`history_fork_cut`).

The offline evals (`cargo test -p vemry-agent --test evals`) check what needs no provider:
citations resolve, sources carry the right verdict, the plan is state rather than prose,
tools succeed, and no write slips past the gate.

## Programmability

Scripts (palette commands, event workflows, cron schedules) and runnable `js` blocks, labs
among them, share one QuickJS sandbox on the Rust side (`crates/vemry-script`). What matters
most is not what it can do but three boundaries on what it cannot:

- **Arbitrary code never runs by itself.** Every execution starts from a user action — a
  command, a run button, a trigger the user installed. Opening a note is not running code,
  which defends against a poisoned note. A file in `.vemry/scripts/` is itself the
  installation: enabled by default, its top level runs, sandboxed, whenever the vault opens
  or the file is saved, so a vault copied from elsewhere brings its scripts along — but not
  their write grants, which live with the app.
- **The agent cannot trigger a script.** No run_code tool, no script-level skill: the "no
  bash" principle does not loosen because the app became programmable. A write the agent
  lands through the gate reaches the user's workflows like any edit, but which code runs is
  never the agent's to choose.
- **A script's writes structurally cannot trigger a workflow** (below).

**The sandbox boundary**: no fs, no net, no process, no require — QuickJS is the language
core only, and the host-injected `vemry.*` whitelist is the entire capability surface (a
unit test asserts it). Every binding takes the user's perspective (see "Scripts see what the
user sees").

### Scripts: discovery, declaration, lifecycle

`.vemry/scripts/*.js`, one level deep, the file stem as the script id. There is **no
manifest** — a top-level call is the declaration:

```js
vemry.commands.register({ id: "archive", title: "Archive completed tasks" }, (ctx) => { … });
vemry.events.on("note_added", (e) => { … });     // note_added/modified/removed/files_changed
vemry.schedule("0 9 * * *", () => { … });        // a standard 5-field cron
```

Saving a script hot-reloads it: the watcher raises `ScriptsChanged`, and the host drops
every context and reloads wholesale — no stale state. A broken script gets a `last_error`
and does not drag the others down. `enabled` and `write_grant` live in the app's
`script-state.json`, so a script arriving in a copied vault still asks before its first
write. Commands enter the palette through the registry and execute through
`script_command_run` back in the sandbox — the frontend holds the entry point, not the
ability to execute.

### Execution model and resource limits

One OS thread owns the runtime and all execution is serialised, so concurrent writes and
re-entrancy are structurally impossible. Every execution carries a wall-clock fuse: 2 s for
loading, 5 s for a command or workflow, 2 s for a js block, the clock paused while waiting
on the user. Memory is capped at 64 MB. `vemry.prompt` can be answered only in a command —
in a workflow or a schedule it returns null at once, because unattended work must not hang
on a dialog nobody is looking at.

### Write grants and auditing

A script's first `vemry.write` or `append` blocks on a one-time "allow this script to write"
grant (a denial throws, and the script can catch it); settings can revoke it. Every write —
and every grant decision, error, console line and missed schedule — is appended to
`.vemry/script-log.jsonl` with its trigger (`command`, `event:note_modified`, `cron`,
`load`). A script cannot write to any path with a dot-prefixed segment: `.vemry/` holds the vault's privacy
rules, block types and skills, which a script could rewrite to widen its own reach.

### Why workflows cannot loop (a structural guarantee, plus fuses)

A workflow's event source is **only** `commands.rs::broadcast`: the user's IPC writes, the
writes the agent landed through the gate, and external changes the watcher caught. A
script's own `vemry.write` goes through `Vault::write_note_as`, `append_note` or
`write_file` (the watcher swallows the echo — a note's because its text matches the index, a
file's within a short self-write window), and the resulting event goes straight to the
frontend through `UiBridge::vault_event`, **never into broadcast**. So a workflow that edits
a note in its own `note_modified` callback runs exactly once: the cascade does not exist
topologically. The fuses on top: the same (event, path) coalesces within 500 ms (a 1 s
autosave will not machine-gun it), the pending queue is capped at 100 (over that, a drop and
an error toast), and every execution has its fuse. A cron trigger missed while asleep is
skipped, leaving one `schedule_missed` line; one that fell due while the app was closed is
skipped silently (a schedule is armed afresh at load).

### Query blocks, js blocks and labs (the editor side)

- A ` ```vemry-query ` fence holds eight flat scalars —
  `kind/note/tag/text/task_state/due_before/field/limit`, `due_before: today` as sugar —
  plus `group` and `sort`, which the frontend applies (`views/queryrows.ts`) and never sends
  to Rust; a hand-written parser, no YAML dependency. With the cursor outside, the fence is
  a live-results widget (`queryblock.ts`) that `vault.indexVersion` re-runs. Elsewhere it is
  code.
- A `js` fence gets a Run button (`runblock.ts`): a click runs the block in a one-shot
  **read-only** context (`script_eval_block`, a 2 s fuse, `vemry.write` throws, no grant
  flow), and the output renders after the fence in editor state, never on disk. A grant is
  persisted against a script's identity, and a code block has none — its body can be edited
  freely or synced in — so a block that needs to write graduates to `.vemry/scripts/`.
- A block that calls `vemry.lab.*` is a lab (SYNTAX.md, "Lab blocks"): its calls are
  normalised into the capped spec of `lab.rs`, and while its card is open it re-runs with
  the controls' values on every move — the same one-shot context and fuse, the code read
  again from the document each time; the lab API exists only in a block's context, never a
  script's. "Pin", the lab's one write, is an ordinary editor transaction. The first run
  is a click, and a re-run only follows a person's move.

### Known limitations (deliberate)

- A script command's `hotkey` is display-only, like the registry's hotkey for every command.
- Renaming or deleting through the interface emits the `IndexReady` summary event, which
  does not trigger workflows (an external tool's change arrives through the watcher as a
  typed event and triggers normally).
- A callback does not see intermediate states that coalescing folded away.

## The block model

The parser's blocks are the atoms every projection is built from. Two mechanisms extend them
without adding a format or a database.

### Block types

`.vemry/types/*.md` (`vemry-core/src/types.rs`; regular `.md` files only, a symbolic link
refused, malformed frontmatter degrading to defaults) declares a kind of block, its inline
fields, an optional tag and a template. A typed block is ordinary Markdown with fields, and
no note points at a type. Enabled types appear in the editor's `/` menu and under "turn into
›" (one transaction turns the block into the kind and appends the fields it lacks), and
under `## Block types` in the system prompt, so the agent writes such blocks the same way. A
type file opens like a note, but `note_write` routes a `.vemry/` path to
`Vault::write_meta`: configuration is never indexed, and the watcher's `TypesChanged` (own
writes included) refreshes the menus and drops the prompt snapshot.

A field declared `type: state` is a **state field**: its `values`, in order, are the steps
(`lib/fields.ts::stateField`), and nothing of it reaches Rust beyond the `type` string; one
glyph draws it on every chip (`lib/chips.ts::stateGlyphSvg`), sorting and the board's
columns take the declared order, and the system prompt spells it `key (state: a → b → c)` so
the model moves blocks along it with `set_block_field`. How a decision, a requirement or a
card is written is in SYNTAX.md and the built-in skills.

### Block change times

A block has no modification time, yet the board's unscheduled and done columns want "most
recently changed first", and the note's mtime would move every task of a touched note
together. So the vault remembers when each block's current text appeared
(`vemry-core/src/blocktimes.rs`): a change key — FNV-1a over the block's trimmed lines, a
trailing ` ^id` dropped — seen before keeps its stamp, and a new one takes the note's mtime
(the file's time, so a change made while the app was closed is dated when it happened). A
toggled checkbox counts; minting an id or moving a block does not. This is deliberately not
provenance's hash, which strips the checkbox so authorship survives a toggle — here the
toggle is the point. `.vemry/block-times.json` is a **cache, not state**: without it every
block falls back to its note's mtime. Opening a vault stamps but never saves. The stamp
reaches every consumer as `Block.modified_ms` through `query()`.

## The editor and the app shell

### One live buffer, tabs and projections

The core invariant is **one live buffer**: only the active tab holds a live CodeMirror
instance and only it can be dirty. Background tabs keep a snapshot side table (EditorState,
scroll, line ending, the content as it would be written) that is **not reactive** — `$state`
would deep-proxy the EditorState and break its identity comparisons — and an LRU of 20 drops
the oldest, losing its undo history, cursor and scroll — never data. Activating a tab
re-reads disk and validates
lazily: identical, the snapshot is reused with its undo stack; different, it is rebuilt. So
every `=== currentPath` test keeps its meaning.

**Both uniqueness properties — one preview slot, one tab per path — are kept by the single
`openNote` / `openFile` funnel**; do not add another entry point. `activateTab` flushes
before it snapshots (`state.doc ≡ disk` is what makes lazy validation sound); deletion and
external removal go through `dropTabs`, which deliberately does **not** flush (it would
resurrect a dead file). The tab list persists per vault, restored in the same synchronous
batch as `adoptRoot`; on a vault switch a path the new listing does not know is confirmed on
disk once before it is dropped.

A **projection is a tab too** (a `VIEW_TAB_PREFIX` sentinel path with no buffer), so
`mainView` and the active tab are aligned both ways, and Escape, the view header's ✕, the
rail entry and ⌘W all end at `closeTab`. The reading projection and the translation overlay
are the two overlay states that are not tabs. While a projection has the main area **the
Editor stays mounted, hidden by CSS**: CodeMirror holds the unsaved buffer and undo stack.

The editor derives its own block model from CodeMirror's Lezer tree (`editor/blocks.ts`),
two projections of one text with the Rust parser's; where they differ, the editor never
predicts where a ` ^id` goes but asks Rust (`block_id_plan`).

### The flush funnel

Notes and plain-text files autosave one second after typing stops (the debounce lives in
`Editor.svelte`, which holds the `EditorView`, the content's truth). The invariants:

- **Write the old before reading the new.** `flushAutosave` registers as
  `vault.flushEditor`, and the store's funnels (`openNote`, `openFile`, `renamePath`,
  `openVault`, `activateTab`, `closeTab`, `openView`) await it **before** reading, moving or
  switching — otherwise a rename would move the old content off disk, a pending save would
  write a phantom file into the next vault, and reopening the note would reset the buffer
  from stale disk. The editor also flushes before an external reload and on window blur.
- **The pending timer is the dirty signal**, so it may be rescheduled but never dropped:
  saves pause while the agent runs but stay scheduled; a failed save reschedules; ⌘S cancels
  the timer before writing, or a flush during an external reload would write the stale
  buffer back. A save keeps dirty set when more input arrived during the write.
- **An existence guard** (`docExists`): after a rename or a delete the old path is gone from
  the listing, and writing would resurrect it. A delete also drops the pending timer on entry
  (`discardEditor`): an undo of a bulk delete does not refresh the listing path by path, so
  the guard alone would miss.
- **Save-echo suppression.** A save raises exactly one event — `note_write` broadcasts its
  own (the watcher reads the echo as no change), `file_write` suppresses its echo in Rust —
  and the store skips the reload when the disk equals the buffer, so a save never resets the
  cursor or the undo stack.
- **The quit handshake.** Every quit reaches Rust first; the first request is refused and
  the main webview asked (`vemry://quit-requested`) to flush, answering `quit_confirmed`; a
  second request passes regardless, so a webview that never answers cannot hold the app
  hostage. A relaunch after an update exits with a code the handshake lets pass, so it
  flushes the buffer itself first (`updater.svelte.ts`). Saves wait while the agent runs,
  so without this a quit after a long turn would
  lose those minutes of typing.

### Reloads, reveal and history

**A reload from disk is one edit.** When the open note changes on disk, `loadDoc` applies
the difference as one transaction rather than a fresh state: the undo stack survives, and ⌘Z
takes an agent write back the moment it lands.

**`reveal.ts` is the one definition of reveal** (when rendered syntax opens to its source),
shared by the live preview, tables, query blocks and frontmatter; a second copy would drift.
A freshly opened document or a programmatic jump reveals nothing until a real key or mouse
action.

**Back and forward** are recorded only in the store's one landing funnel (`settleTab` →
`landed`). Only documents are recorded — a projection is a mode left with Escape, not a
destination — except the reading projection, one sentinel entry (`READING_ENTRY`) so a clip
note landed from it can go back to it. The keys are ⌘⌥← and ⌘⌥→, because ⌘[ and ⌘] are
CodeMirror's indentation keys.

### Projections

Every view renders "a view is a query", each as a tab. `vault.indexVersion` increments on
every vault event and is the single re-query signal; a row click goes through `openNote` (a
task row `openBlockAt`), sharing the navigation contract and history.

| View | Data source |
|---|---|
| Agenda | one `query_blocks {kind: task}`, grouped by `placeTask` (`views/tasks.ts`); a checkbox rewrites the task's byte span, and a stale span refuses and re-queries |
| Board | the same query; its state columns are a coarser cut of `placeTask`, so the two views cannot disagree about a task (`views/kanban.ts`) |
| Timeline | `vault.notes`: frontmatter `created`, else mtime |
| Claims / Evidence | two `query_blocks` plus `resolve_link` for the `supports:` join, through the index's own resolver, so the join agrees with backlinks and the agent |
| Graph | `graph_get` (`BlockIndex::graph()`); interface-only, not an agent tool |
| Review | one `query_blocks {tag: card}`; a grade is a user field write on the card's span (`views/cards.ts`) — the note is the deck |
| Session replay | the session log's rows, re-read 300 ms after the agent store's activity counter last moved, so a replay grows with a running session |

Views are not kept mounted while another tab has the main area — a mounted view re-queries
on every vault event and stays registered as the AI's view context — so their place is kept
in memory instead (`viewmemory.ts`). Views **do not filter** private notes.

### Everything is a command

The activity bar, the menu bar and the context menus are entry points, and none grows a
second set of semantics: an item that is also a command ends at the same id in
`commands/registry.ts` (the menu bar bridges back through `menu://command`), while a row's
own actions (rename this file) call the store. A feature registered as a command is present
in all of them and the palette.

- **The menu bar** (`menu.rs`, macOS): an item's id is the command id, its label the
  command's title. Keys a focused page would swallow (⌘W, the browser's ⌘L ⌘T ⌘R) are
  menu-only, with the frontend branch gated on `!isMac`; the Edit menu's undo, cut, copy and
  paste go to the first responder, since an emit would steal CodeMirror's ⌘Z and ⌘C. Off
  macOS the menu is a stub and the frontend keydown chain is the only source of bindings.
- **Context menus** are data, computed at the moment of opening; one `<ContextMenu />` per
  window serves every mount point, and a dangerous item confirms by a second click.
- **Layout coupling.** The rail's width (`ABAR_W`) is a term of three minimum widths that
  change together: `window_geom.rs`'s `min_inner_size`, the browser store's
  `MAIN_MIN_BESIDE_PANEL`, and the sidebar resize ceiling.
- **Typography.** Only weights 400, 500 and 600 — the CJK system face has no others, and an
  off-scale weight makes a mixed line uneven (`src/app.css`).

### Attachments and remote images in a note

A note's pasted and dropped files live in its bucket (see the clipboard paste under "Key
flows"), private with the note. Its **remote images** load only with the user's leave,
always through the proxy (`src/lib/editor/remoteimages.ts`): opening a note asks the proxy's
disk cache alone (`c=1`), so an image seen in the reading projection or loaded before shows at once and
any other stays a placeholder; it loads (the same `k=note` request without `c=1`) on a
click, on "load every image in this
note", with the `note_remote_images` setting, or at once when pasted or dropped in (the same
trust as clipping its page). Opening a note therefore makes no network request — a tracking
pixel clipped with a page would otherwise report every re-reading of the note — and the CSP
makes that a rule rather than a habit.

### Persisting UI state

`localStorage["vemry-ui"]` holds the global presentational layout, kept out of the Rust
settings (not worth an IPC round trip); per-vault state — the expanded tree, the open tabs,
the pinned clip target, the board's grouping — goes through `persist.ts`'s bucketed table.
Pruning accepts only a recent-vault list that **actually loaded**: a startup race or a
failed IPC passes null and skips pruning, because an empty list would delete every other
vault's bucket.

## The built-in browser and the reading projection

A **reading browser**, not a general-purpose one: reading on the web inside the app — no
ads, fast, clean — and turning a page into a note when it is worth keeping. The browser is
the intake, the vault the settling tank; no extension ecosystem, no sync account.

### Shape: tabs as child webviews, docked or detached

Every page is a **tab**: a `browser-N` child webview (`Window::add_child`), moved between
windows with `Webview::reparent` — no reload; the page, its scroll and its history come
along. **Docked** (the default), the tabs lie over the main window's outermost column under
a chrome that is the main window's own DOM; the frontend reports the column's anchors only
when they change, and Rust re-places the tab on every resize with no round trip.
**Detached**, they move into `browserwin`, whose first child is `browserbar`, the same
`BrowserPanel` in bar mode. A **warm-up shell**, a hidden tab built after startup and
refilled after each claim, has its WKWebView and ad-block rules ready before a page is asked
for (`claim_tab`).

**A native view covers the DOM** — the one cost of same-window embedding — handled by three
rules with one collection point each: a full-window overlay **veils the tab** (App derives
`overlayOpen`, Rust hides the tab, the panel paints a placeholder); **popups stay left of
the panel** (`usableRight()` in `src/lib/viewport.ts`; the chrome's own menus are native);
**a DOM drag ends at the panel's edge**, so a sash drag veils the tab throughout. A focused
page swallows every key, so the browser's keys are menu-bar accelerators routed by where the
focus is (`focused_tab` walks the first responder's superview chain), and `browser_focus`
hands the keyboard back to the page after any chrome interaction.

**History and bookmarks** live in the app-config directory, shared across vaults, outside
`Settings` because `settings::load` is cloned on the navigation and image-proxy hot paths.
History has one writer, the tab's build hooks; the reader windows have none, so the reading
projection and the agent's reads stay out of history by construction.

### Ad blocking

adblock-rust compiles the subscribed filter lists into WKContentRuleList JSON — no JS
injection, no local proxy — attached before a tab is built, so its first frame is already
filtered (`browser/adblock.rs`'s header has the pipeline). **No filter list is bundled**:
EasyList is GPLv3+ / CC BY-SA 3.0+ and EasyList China states no licence at all, so shipping
one would be redistributing it, and a frozen snapshot rots anyway. The rules live in
`app_config_dir/adblock/*.txt` and arrive from a user action: exit 3's download — tried
automatically at most once per run, because a firewall that cannot reach the hosts must not
cost a timeout per page, or on demand from the settings — or files imported by hand
(`adblock_import_rules`), the path with no network at all.

### Clipping

- **Selected text** becomes a quote callout (`> [!quote] [title](url) · date` as its head
  line), inserted at the cursor, saved into `clips/`, or clipped into a chosen section of a
  chosen note. The landing rule (`insert.rs`, sharing one table of cases with
  `src/lib/insert.ts`): a named section → its end; a missing heading → the end with a toast,
  never silently; no section → the cursor in the open note, else the end.
- **A whole page** (⌘⇧S) → Readability → Markdown → `clips/<title>.md`. With
  `clip_localize_images` on (the default) its images are then **localised in the
  background** through the proxy's fetch into `clips/assets/<content hash>.<ext>` (format
  sniffed from the bytes; 10 MB per image, 30 images and 50 MB per clip; a failure keeps the
  remote URL, with `images: partial`), and the note rewritten — re-read after the downloads,
  replaced literally, code skipped, so an edit made meanwhile survives.
- **A link** → a `[title](url)` reference in the current note.

A clip lands through `Vault::write_note_as` plus `broadcast`: indexed the moment it lands,
and workflows under `clips/` fire. `clips/` is visible to the agent by default — it is
material to feed it — and making it private is one rule line. A target that is the open note
goes through the editor buffer (see "Landing text from another window").

### The reading projection

A citation click lands by default in the **extracted view in the main window's centre
pane**; a browser tab is the escape hatch for interaction (a paywall, a login, comments —
one click across, or ⌘click). A hidden `reader` window loads the page: no capability, the
same navigation guard, the ad-block rules **plus its own image, media and font blocking** on
macOS, so the page is ready far sooner; it is not in the browser's registry, so the chrome
and its clip and AI entry points cannot see it. Rust extracts the article with
`vemry_clip::extract_page`, without waiting for onload (`browser/reader.rs`), and
`renderReadingMarkdown` renders it into `mainView = "reading"`.

- **Images** go through the proxy (the page's URL as Referer) only once the user has opened
  the projection, and only from the proxy — the reader window blocks them itself.
- **Formulas and code cross htmd through sentinel elements** (`<x-vemry-math>`,
  `<x-vemry-code>`), so a clip carries standard `$…$`, `$$…$$` and fences
  (`browser/clip.rs`'s header documents the normalisers).
- **Straight back into the loop**: `reading_clip` lands the projection cache (an LRU of 32)
  in `clips/`, byte-identical to the screen, with no refetch; "add to context" hands a
  `PageContext` to the agent like the browser's AI button.
- **Privacy**: projected content is displayed only — never written to disk, never indexed,
  never in a prompt; anything settling into the vault goes through a clip or add-to-context.
- **Known compromises**: paywalls and heavy application pages extract poorly (a banner, and
  the escape hatch); an explicit "open URL" opens a browser tab — the intent to browse.

## Side-by-side translation

Reading in another language is the one everyday task where the model is a tool rather than a
collaborator: the text goes out, the translation comes back, and nothing about the vault
changes. So the translation is a **projection, not data** — `mainView = "translate"` is an
overlay like the reading projection, recorded nowhere in the document history, and it
becomes a note only when the user saves it.

- **One document model.** A source is cut into segments (`vemry_core::segment`), one row
  each, so source and translation stay aligned by construction.
- **The engine is not the agent.** `TranslateJob` sends batches as `<s id=N>…</s>` in the
  same tool-free single call the summary makes — no session, no tool table, no ledger — with
  thinking off where a level is sent at all: translation is transcription. Two batches are
  in flight and the first is cut to about 600 bytes of text, so text lands in seconds.
- **The reader first, then a budget.** The rows in view go first and are never limited; the
  rest is background work bounded by about 60 kB of source text, so a paper finishes and
  a book stops and offers to continue.
- **What leaves the app.** A private note or file is refused whole; a private block is never
  enqueued and is left out of a saved translation without a placeholder (a placeholder would
  say there is something to hide); captions and reference entries (`SegKind::Aside`) are
  never sent. The agent sees none of it. The status bar's offer to translate is judged
  locally (`lang.rs`), so nothing is sent to decide it.
- **The cache** (`.vemry/cache/translate/`) maps the hash of each segment's text to its
  translation, with no source text in the file. **Saving** first translates what is left (the
  budget lifted), so a saved note is complete, then writes a note beside the source
  (`translation_of: "[[source]]"`) through the same `write_note` any note takes — the user's
  action, no gate.
- **Four sources**: a note; a web page in the reading projection (translations in memory
  only, as the projection's content is); a browser tab in place (`browser/pagetr.rs`:
  everything through `eval`, each segment written as `textContent`, never HTML, cached in
  memory only); a PDF, its paragraphs rebuilt by `segment_pages`.

## The hosted account and the phone inbox

### The hosted account

The user's own providers are a list in `settings.json`, each an endpoint, a key and the model
ids it serves (`ProviderKind::Custom`). *vemry hosted* (`ProviderKind::Hosted`) is one more
provider, built in rather than listed, and not a special host name: the relay speaks the two
shapes the transports already speak, so the hosted `ProviderConfig` is the relay's `/v1` as
base URL, the account's **device token** as the key, and the **model catalog** the account
snapshot carries, which the provider predicates read. One picker lists both (`model_choices`,
computed in Rust so the detached window, which has no account store, gets the catalog too):
the current model is a `ModelRef` — a provider id (`hosted` or a custom provider's `p<N>`) and
a model id — resolved per request, so choosing a model is what decides whether a request goes
through the relay. Translation may name a model of its own from the same list; otherwise it
follows the chat's.

The account lives in `account.json` (mode 0600, apart from `settings.json` so an exported
settings file never carries the token). Sign-in is an e-mail address and a 6-digit code.
**Freshness is the offline rule**: a snapshot within ten minutes is fresh; within seven days
it is stale and still applies (a machine that cannot reach the service keeps its Pro); past
that the account counts as free and not hosted until a refresh succeeds. A hosted turn
refreshes a non-fresh snapshot first and once more when it ends (an unchanged one is a 304).
**A 401 from any request means the token was revoked**: the file is cleared and the frontend
told, in one place (`on_unauthorized`). A relay error carries a code, mapped in one place
(`localize_relay_error`) to the user's wording with an action.

A hosted request adds four headers and no body field, none sent to a custom provider:
`X-Vemry-Feature` (the relay's metering category — chat, study, research, translate,
compact; `cards` and `import` are reserved and not sent yet), `X-Vemry-Request-Id`,
`X-Vemry-Priority` (interactive for a turn the user waits
on, background for translation) and a `User-Agent` naming the client version. The
stream-idle limit is 600 s through the relay — the vendor's peak-time queue, relayed, sends
only keep-alive comment lines for up to ten minutes, and the read loop counts events, not
comments — and 120 s otherwise.

### The phone inbox

A capture made on a phone reaches the desk through the hosted service's inbox (`inbox.rs`;
the phone pairs once by a QR code the settings page shows). One background loop long-polls
it with the device token for as long as the app runs and the account is signed in. Items are
acked only once written, and a seen-list in the vault (`.vemry/inbox-seen.json`, written
before the ack) makes a lost ack, or a second computer sharing the vault, a no-op rather
than a duplicate. Rust works out each item's text, stamp (the phone's clock unless a day or
more out) and capture spot, so the two doors below cannot file it differently.

**The landing vault is fixed, not whichever is open.** A phone cannot see which vault is
open, and privacy zones belong to a vault: a private thought must not land in the work vault
because that one happened to be open. So the landing vault is an app setting (`inbox_vault`,
set to the open vault at the first pairing). With it open in the main window, the items go
through the window, whose buffer may hold the target note; otherwise Rust writes them itself
— the exception in "Landing text from another window" — at the same spot (the mirror of
`capture.ts`'s `defaultCaptureSpot`) with the same provenance record. The main window gets a
receipt naming the vault, and a vault that cannot be written is reported and retried.

## Opening a folder as a vault

**Whether a folder is a vault is the user's call, made before anything in it is read.**
Opening walks, reads and watches the whole tree, so a folder picked by mistake — a whole
drive — would freeze the app. The mark is a `.vemry/` directory: `vault_open` refuses a
folder without one, unread (one already on the recent list counts as answered and gets the
mark), and the picker first asks for the vault's name (`vault_inspect`
walks nothing); the answer creates `.vemry/`, Cancel leaves nothing behind. The commands
that act on a vault by path (`vault_rename`, `vault_show_folder`) accept only the open vault
or a recent one, so none can reach, or mark, an arbitrary folder.

## Internationalisation

**One generator, two complete catalogs, five overlays.** `scripts/gen-locales.py` holds
English and Chinese as one `key -> (en, zh)` table, so a new message cannot enter one and
miss the other. Japanese, Korean, French, German and Spanish are overlays
(`locales/src/<tag>.json`, conventions in `<tag>.md`), and all five are complete
translations: only `slash.*` stays English, by rule, and `langName.*` is never carried. The
generator validates an overlay — unknown keys, placeholder names, the CLDR plural forms the
language needs, where a form English lacks (French and Spanish `_many`) is accepted against
the English `_other` — and writes `locales/<tag>.json`; both consumers still fall back to
English key by key. Keys are flat and dotted (`cmd.open-note.title`), because a nested
object would let TypeScript's completeness check see only the top level. CI regenerates and
diffs `locales/`. The list of languages is mirrored in three places that must agree —
`LANGS` in the generator, `LANGS` in `src/lib/i18n/i18n.svelte.ts`, the `Lang` enum in
`src-tauri/src/i18n.rs` — and each entry needs its `langName.<tag>`.

| Consumer | Entry point | What a missing key costs |
|---|---|---|
| The frontend | `t()` / `tn()` in `src/lib/i18n/i18n.svelte.ts` | a `pnpm check` error (`Record<MessageKey, string>` on the two complete catalogs; an overlay may carry only keys English has, or the `_many` of a family whose `_other` English carries) |
| Rust (native menu, IPC errors, window titles) | `t()` / `tf()` in `src-tauri/src/i18n.rs` | a `cargo test -p vemry-app i18n` failure (each key Rust uses is asserted against the two complete catalogs) |
| The two READMEs and the two syntax references | by hand, against `docs/GLOSSARY.md` | nothing automatic — the glossary exists for this |

**Resolution.** "Match system" resolves the system language by primary subtag, by the same
rule in the frontend and in Rust; anything unsupported is English. The editor body carries
the note's own language (`lang:` frontmatter, else the one its letters give away), and
fonts, line breaking and hyphenation hang off that attribute in `app.css`.

**Locale-dependent behaviour goes through one door each**: dates and numbers through `Intl`
on `i18n.locale` (an amount by its currency code); orderings through `compareNames` (one
cached `Intl.Collator`); search folds both sides with NFKC and lowercase, while note names
and paths fold by NFC and case only (a compatibility character is a different file); plurals ask
`Intl.PluralRules`. Parsing tolerates full-width digits (an IME artefact); what is written
to disk stays ASCII. **What never follows the interface language** is the dialect's keywords
— ISO dates, `every 2 weeks`, `3d2h15m`, callout type names, the `/` commands — typed from
memory and read by other machines, so a translated spelling would split the format. Pickers
are translated; what they insert is not (SYNTAX.md, "Baseline").

Constraints that are not obvious:

- **Placeholder names must match across languages**: a mismatch renders a literal `{foo}`,
  invisible to key-set parity and to `pnpm check`. The generator refuses such an overlay,
  and a test in `i18n.rs` asserts it again for every language.
- **`t()` reads a `$state`**, so a template re-renders when the language changes — no
  restart. A string array computed at module evaluation must therefore be `$derived`, or it
  freezes in the language active at first mount.
- **Each webview holds its own language state**: the window that changes it emits
  `settings://language` and every window follows; the native menu is reinstalled wholesale
  (`menu::install()`) and the tray menu rebuilt with it. CodeMirror widgets already built
  keep the old language until the document re-renders — a known, accepted lag.
- **VEMRY.md's section name is stateful**: `memory_append` looks up every name vemry has
  written (`vault.rs::MEMORY_SECTIONS`), so a vault keeps appending to its own section.

**Who deliberately does not read the catalog**: copy sent to the model (prompt envelopes,
attachment placeholders, tool errors) is English unconditionally — the system prompt is
English and tells the model to answer in the user's language, so the protocol layer does not
switch with it. Errors from the `vemry.*` script API are English, a developer interface like
any JS runtime's. Built-in skills are English too, with `triggers` in every interface
language so the `/` picker finds them.

## Seams left for later

The study loop's own next steps come first (the README's "next", in its order):

| Future capability | The seam today |
|---|---|
| PDF annotation that lands in the source note | `PdfViewer` exposes page numbers and jump-to-page; `import_page` writes a source note whose blocks carry ids, so an annotation is a block reference plus a page number; translation's PDF alignment is the text-to-page map |
| Reference management on source notes (DOI, BibTeX export) | a source note's frontmatter carries `source` and the clipper's provenance; the session source table is the per-session list; an exporter is a query over notes with those fields |
| Reading across papers (one claim, every source that bears on it) | claims and evidence are indexed blocks with `supports:` edges; `query_blocks({kind: claim})` plus the link graph is the join, and `![[embed]]` is the rendering |

Then the rest:

| Future capability | The seam today |
|---|---|
| tantivy or semantic retrieval | the `SearchProvider` trait |
| vemry Publish | vemry-core has no Tauri dependency and can be reused server-side |
| PDF text in the block index, making a PDF one more source the projections are built from | the extraction cache under `.vemry/cache/pdf/` already holds each PDF's pages; `segment_pages` rebuilds its paragraphs |
| The agent reading an image or a scanned PDF by itself (`read_attachment`) | `read_file_bytes`; the single `inflate_images` function, also where resizing or transcoding (tiff→png, past 5 MiB) would go; `tool_defs()` is an extensible registry |
| The built-in browser: detaching a single tab, favicons, a page snapshot as the veil placeholder | Tauri 2 multi-webview is in use; `Webview::reparent` works per tab; the veil placeholder is a DOM slot that can take an image |
| The agent triggering scripts (needs fine-grained permissions first) | the `Perspective{hide_private}` every script binding takes, its filtering implemented and tested (see "Scripts see what the user sees") |
| Block-level backlinks ("which blocks reference this block") | `LinkEdge` already records the source block; the target fragment (`heading` / `block_ref`) survives on `Block.links` and would move into the edge |
| The agent reading an embed's content (embed expansion) | `Vault::resolve_ref_for_agent` (a target resolved with its private blocks removed); `read_block` already wraps the sibling block lookup (`Vault::block_for_agent`, the same redaction) behind the read gate |
| Rewriting links when the user renames or moves a note in the file tree (the agent's moves already do) | `vemry_core::relink` plans the rewrite (the author's spelling kept, private blocks left untouched; the agent's move tools drop private referrers themselves); `path_rename` does not call it yet |
| Provenance by similarity ("yours, edited") and a per-session "what did it write" view | the ledger is keyed by hash and ordered by time; `pending_notes` is the entry point of the latter |
| A per-block authorship history | the ledger's lines are timestamped and append-only (compaction above 5000 lines keeps only live hashes, so a history would need superseded lines kept) |
| Installable extensions and themes | an extension is a packaged user script behind the boundary scripts already have (the glossary keeps the word "extension" for it); a theme is a set of values for the colour tokens in `src/app.css` that passes `scripts/check-contrast.mjs` |
| Split-pane browsing in the main window | docking already moves tabs into the main window by `reparent`, without a reload, and places them where the frontend reports (`browser_panel_layout`); a split pane is the same webviews in an editor pane's rectangle |
| Search on Moonshot; thinking text from OpenAI's models | Moonshot's built-in search tool retires on 2026-10-20 and its successor is a set of search endpoints the client would call itself — a new kind of request on the network-exit list, so a design decision; OpenAI's reasoning summaries are one request field away, once an organisation that is not verified for them is not refused the whole request |
| A password lock for privacy zones (a design note, unscheduled: [design/privacy-zone-password-lock.md](design/privacy-zone-password-lock.md)) | content enters the WebView only through a closed list of IPC commands; the agent's named reads in `vemry-core` (`*_for_agent`) and the scripts' `hide_private` switch are the read-only view a lock would reuse |
