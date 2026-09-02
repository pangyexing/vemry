# vemry architecture

## First principle: your knowledge stays in Markdown, and every view grows from it

```
Storage (single truth)     Semantics (one index)          Projections (a view is a query)
─────────────────         ─────────────────              ─────────────────
Markdown vault      →     typed block index          →   document view (the editor)
  *.md notes                Block{id,kind,span}            Claims / Evidence
  VEMRY.md                  Paragraph/Heading/Task/        Agenda / Timeline
  .vemry/sessions/          Claim/Evidence/CodeBlock       Graph (d3-force + Canvas)
                            + link graph / backlinks       Session replay (the agent's record)
                            + tag table
                            + one query()
```

Interface copy sits outside that chain: the pair `locales/{en,zh-CN}.json` is the
single source of every user-visible string, read by the frontend
(`src/lib/i18n/i18n.svelte.ts`) and by Rust (`src-tauri/src/i18n.rs`) alike.

Three constraints follow:

1. **Disk is the single source of truth.** The index is always derived and always
   fully rebuildable; there is no database.
2. **One `query()` serves three consumers**: UI views, user JS scripts (v0.3
   `vemry.query`), and the agent's `query_blocks` tool. Any projection a person can
   see, a script and the AI can compute — no AI-only capability, no UI-only data.
3. **Semantics are expressed as a superset of standard Markdown** (the dialect and
   its fallbacks are in [SYNTAX.md](SYNTAX.md)); no private formats. Take the vault
   away from vemry and it is still complete.

## Layers

### crates/vemry-core — the vault engine (no UI, AI or Tauri dependencies)

| Module | Responsibility |
|---|---|
| `privacy.rs` | The privacy zone: the glob half of path privacy (`PrivacyScope`), and the block half — removing, blanking and matching around `> [!private]` blocks, the functions every agent-visible read goes through |
| `parser.rs` | Line-level scanner: frontmatter, headings, list items, tasks, callouts (claim/evidence/private), code fences, `[[wikilink]]` (with the `![[…]]` embed flag), `#tag`, `^blockId`. Everything is a line-level or inline construct, which is why no full Markdown AST is needed |
| `block.rs` | The `Block` / `BlockKind` types: the atom every projection is built from |
| `locate.rs` | One definition of "which block / which section": heading lookup (exact, then case-insensitive; a trailing ` ^id` is not part of a title), section spans, block by local id (explicit beats positional), block by offset, a list item's subtree. The insert landing rule, reference resolution and id minting all go through it |
| `blockid.rs` | Block ids: the validity rule the parser applies, which blocks may carry one (not fences, table rows or rules), and minting — a random `a–z0–9` id unique within the note, written only on demand |
| `provenance.rs` | Who authored each block: a normalized-content hash as the block's identity, the `.vemry/provenance.jsonl` ledger (written when text lands, compacted on load), the multiset difference that names the blocks a write added, and the origin priority (ledger → capture stamp → clip credit line → frontmatter `vemry.origin` → the user) |
| `index.rs` | `BlockIndex`: note table, block table, link graph (backlinks derived), name resolution table (stem/alias, shortest path wins), tag table. Incremental update: reparse one file, then reparse links wholesale (O(total links), milliseconds at ten-thousand-note scale) |
| `query.rs` | `BlockQuery` → `Vec<BlockHit>`: the one query entry point. Inline-field conditions (`FieldFilter`, parsed from the flat `key = v` grammar shared by query blocks, the agent tool and scripts) consult a block's own fields first, then the note's frontmatter scalars |
| `search.rs` | The `SearchProvider` trait plus an in-memory multi-term AND substring match (no tokenisation, so CJK works by construction). tantivy or semantic retrieval would be another implementation of the same trait |
| `vault.rs` | Orchestrates disk ↔ parse ↔ index ↔ search; path validation (rejects `../` escapes); self-write suppression (watcher loop protection) |
| `watcher.rs` | notify + 500 ms debounce → a batch of paths → `handle_fs_change` |
| `event.rs` | `VaultEvent` (including the `ScriptsChanged` hot-reload signal). Since v0.3 workflows subscribe to these through the `broadcast` fan-out in src-tauri |

### crates/vemry-agent — the AI layer (depends on core, not on Tauri)

| Module | Responsibility |
|---|---|
| `provider.rs` | `ProviderConfig`: base_url + model + key. Zhipu (`glm-4.7-flash`) by default, swappable for any OpenAI-compatible endpoint; `is_deepseek()` decides whether the Responses transport is available |
| `types.rs` | OpenAI Chat Completions wire types plus the streaming tool_call assembler. `ToolDef` is an untagged enum (`Function` plus `Raw`, which passes a server-side tool through verbatim); `ChatMessage.extra_items` carries Responses output items across turns |
| `client.rs` | The `ChatTransport` trait (mockable) plus the reqwest SSE implementation — **the LLM channel boundary**. Since v0.4 the system has two further controlled network exits (see "The built-in knowledge browser"). v0.7: a process-wide shared connection pool, connect timeout, a 120 s stream-idle timeout and cancellation polling for a stalled stream; both transports share one read loop, `read_sse` |
| `responses.rs` | v0.4: the DeepSeek Responses API transport — a second implementation of the same `ChatTransport` trait. On the request side it maps messages→input items, system→instructions and flattens tools; on the response side it synthesises `StreamChunk` from semantic SSE events, so the runner and the mock need no changes. The server-side `web_search` tool is injected through here, and `web_search_call` items are replayed verbatim. Enabled only when web search is on **and** the endpoint is api.deepseek.com |
| `tools.rs` | Nine domain tools. The read-level ones (list/read/search/links/query_blocks/read_skill) execute directly; the write-level ones (write_note/edit_note/move_note) are first "prepared" into a `WriteRequest` — carrying the rendered diff and the exact content that would land on disk (a move is a path pair, `path → to_path`, with no diff) — and execute only after the gate. `move_note` goes through `Vault::rename_path`, the same implementation the file tree's move uses: rename and rescan, without rewriting links |
| `permission.rs` | The `PermissionGate` trait. The write gate is enforced in the Rust layer that owns the ability to write files — the frontend can render the decision and nothing else |
| `runner.rs` | The agent loop: stream → dispatch on `finish_reason` → execute tools → feed results back → repeat (25 iterations max). The `ChatTransport` mock makes every branch testable. v0.7 added three layers of resilience: transient errors (429/5xx/dropped stream) back off and retry as long as nothing was produced; a context overflow first compacts, then summarises, then retries, so a session is no longer written off; usage is reported per iteration to drive the frontend gauge and proactive compaction; a wrap-up hint is injected with five iterations left; `finish_reason=length` reports truncated; `reasoning_content` is kept in history and sent back (the DeepSeek thinking contract); tool results have a 32k backstop; `user_message=None` means regenerate — no user message is appended and the tail of history is replayed |
| `compact.rs` | v0.7 two-level context compaction, pure logic. Level one replaces older successful tool results with recoverable handles (the note is on disk; calling again fetches it back — ERROR lines are never cleared). Level two has the LLM summarise everything before the current turn, with a hard truncation as the fallback if summarising fails. `is_context_overflow` is the trigger for reactive self-healing |
| `session.rs` | The `.vemry/sessions/*.jsonl` audit log: every user message, model output, tool call, diff and permission decision. It is the data source for the v0.2 session-replay view. v0.8 added a title override row (manual rename; the last one wins, and a fork inherits it). v0.7 added retry/edit_resend/compact/fork marker rows (the audit is immutable; the presentation layer folds on the markers) and `fork_session` — a research-tree fork that copies the audit prefix. The fork point is located by turn id: the runner stamps one `turn` id into the audit's user row, the model-history envelope and the `TurnStarted` event at the top of each turn, because timeline, audit and sidecar each counting user ordinals independently falls out of step after a compaction synthesises a message or an edit-and-resend leaves a stale row. Older sessions with no ids fall back to ordinal counting |
| `prompt.rs` | The system prompt is fixed rules + VEMRY.md (8k cap; over that it truncates and points at indexing it) + a Skills summary (name — description) + a vault overview. v0.7: **frozen as a session-level snapshot** — the glue layer takes it once per session and reuses it, because rebuilding between turns guarantees a provider prefix-cache miss, and DeepSeek's hit/miss price difference is roughly 30×. Volatile information (the date, the current note) rides the turn-context tail in `compose_user_message` instead |
| `skills.rs` | Instruction-level skills (v0.3): listing and reading `.vemry/skills/*.md` by name (frontmatter `name`/`description`/`disabled`), and the implementation of the `read_skill` tool. See "Programmability" |

### crates/vemry-script — the scripting layer (depends on core, not on agent or Tauri)

| Module | Responsibility |
|---|---|
| `engine.rs` | A dedicated OS thread owns a QuickJS `Runtime` (64 MB memory cap, interrupt-based fuse), one `Context` per script. `ScriptHost` is a cloneable handle and tasks are strictly serialised through an mpsc channel — single-threaded execution makes concurrent writes and re-entrancy structurally impossible. The main loop doubles as the scheduler (`recv_timeout` drives both the coalesced event flush and cron firing) |
| `bindings.rs` | The `vemry.*` API surface: one `__vemry_call(method, json)` dispatch function crosses the JS boundary (a JSON envelope both ways), and JS glue builds the ergonomic API on top of it while keeping callbacks (commands, events, schedules) on the JS side. Every vault read and write goes through existing `Vault` methods, so path validation, self-write suppression and incremental indexing all come for free |
| `grants.rs` | `.vemry/scripts.json`: two state bits per script, `enabled` and `write_grant`, travelling with the vault |
| `audit.rs` | `.vemry/script-log.jsonl`: write/grant_*/error/console/schedule_missed appended line by line (the same JSONL style as sessions, but one file per vault — a script is a long-lived resident, unlike the agent's per-session logs) |
| `scheduler.rs` | Cron timing, pure logic: POSIX 5-field expressions (the host supplies the seconds field, and weekday 0/7 = Sunday maps onto the cron crate's 1–7), fired in local time. A missed trigger is skipped rather than replayed, and a run of them coalesces into one `schedule_missed`. Unit-tested against a synthetic clock |

The sandbox boundary: no fs, no net, no process, no require — QuickJS is the
language core only, and the host-injected `vemry.*` whitelist is the entire
capability surface (a unit test asserts this). Every binding carries a
`Perspective{hide_private}` parameter: in v0.3 it is always the user's perspective
(a script is as privileged as the UI and can see privacy zones), while the agent's
filtering is implemented and tested and waits for a future agent-triggered path —
see "Visibility follows the trigger".

### crates/vemry-clip — the clipping layer (v0.4; depends on core, no Tauri or UI)

Pure logic: `extract_page` (dom_smoothie Readability extracts the article →
htmd converts it to Markdown, falling back to converting the whole page),
`compose_clip_note` (reuses core's `Frontmatter`/`VemryMeta` so round-trips are
safe; `source:` and `clipped:` go in extra, and the provenance marker is
`vemry: origin: clip`), `clip_rel_path` (`clips/<sanitised title>.md`, flat, with
the domain in the frontmatter's `domain:`), plus `quote_block` and `link_entry`.
Every URL in the output is in `[..](url)` or `<url>` form — a bare URL's
`#fragment` would be misparsed as a tag, and there is a regression test for it.

### src-tauri — the glue layer

- `commands.rs`: every IPC command (the contract between the three layers).
  `broadcast(app, event)` is the single fan-out for vault events — the frontend
  always receives them, `ScriptsChanged` triggers a script hot reload, and note
  events dispatch to workflows.
- `agent_bridge.rs`: `AgentEvent` streams out through a `tauri::ipc::Channel`;
  permission requests go there and back through a oneshot (UI dialog →
  `agent_permission_respond` → the suspended loop wakes).
- `script_bridge.rs`: the script host's `UiBridge` implementation — `script://event`
  pushes (command-table changes, toasts, errors), and write grants plus
  `vemry.prompt` block on a round trip through a pending map (the same oneshot
  pattern as agent_bridge, in its std-channel form; a grant times out after 300 s
  and a prompt after 120 s, both denying).
- `browser/` (v0.4): `window.rs` creates, navigates and manages the lifecycle of
  the standalone browser window (`browser://event` pushes state to the control
  panel); `clip.rs` holds the clip commands (`eval_with_callback` plus a oneshot to
  fetch the selection or DOM; writing goes through `write_note` + `broadcast`, so
  indexing, frontend refresh and workflow triggering are all free); `adblock.rs` is
  the rule compilation pipeline (see "The built-in knowledge browser").
- `menu.rs` (macOS): the native window menu bar. **A menu item's id is the
  frontend registry's command id**, and a click bridges back through
  `menu://command` to `executeCommand` — the menu never grows a second set of
  semantics, and its labels come from the same catalog as the command titles
  (`cmd.<id>.title`). Accelerators are split three ways: menu-only (⌘, ⌘N ⌘O ⌘⇧F
  ⌘⇧C ⌘⇧S, with the matching frontend branch gated on `!isMac` so nothing fires
  twice), frontend-only (⌘P ⌘L ⌘B ⌘. ⌘⌥←→ — the menu item carries no accelerator
  but is still clickable), and Predefined-only (the Edit menu's
  undo/redo/cut/copy/paste go through native selectors straight to the first
  responder; turning them into emits would steal CodeMirror's ⌘Z and ⌘C). The
  whole module is `cfg(macos)` and compiles to a no-op stub elsewhere, leaving the
  frontend keydown chain as the single source of key bindings on those platforms.
- `pasteboard.rs`: system clipboard reads the web layer cannot reach (macOS
  NSPasteboard: the file-path list and raw image bytes). Other platforms return
  nothing and the frontend falls back to the `File` objects on the DOM paste event.
- `settings.rs`: model / endpoint / key, plus v0.4's browser and search switches
  (adblock, how links open, web search), v0.5's reader image proxy and clip image
  localisation, v0.6's check-for-updates-at-startup, v0.9's interface language, and
  appearance preferences (body text size, clamped to 15..=18 in `settings_set`).
  Stored in the OS app-config directory, never inside a vault.

### src — the Svelte 5 frontend

- `api.ts`: the single IPC entry point (a typed mirror of commands.rs).
- `i18n/i18n.svelte.ts`: `t()` / `tn()` over the shared catalog; see
  "Internationalisation".
- `shortcut.ts`: platform formatting for key hints (`k("Mod+Enter")` → mac
  "⌘Enter" / win & linux "Ctrl+Enter"). Copy only; the key handling itself is
  already cross-platform (`metaKey||ctrlKey`, CodeMirror's `Mod-`).
- `commands/registry.ts`: the command registry (everything is a command). UI
  features, hotkeys and user-script commands (the `script:<script>:<command>`
  namespace, replaced wholesale by `syncScriptCommands`) all go through this one
  entry point. A command carries an id, not a literal title: its copy comes from
  `cmd.<id>.title`, so the palette and the native menu cannot disagree.
- `persist.ts`: localStorage persistence bucketed per vault (the root's absolute
  path → that vault's bucket). `vemry-tree` (the file tree's expanded set) and
  `vemry-clip-target` (the pinned clip target) share one parse-fallback,
  write-through and pruning strategy. Pruning's `keep` accepts only a recent list
  that **actually loaded** — a startup race or a failed IPC passes null and skips
  pruning, because an empty array would delete every other vault's bucket.
- `stores/vault.svelte.ts`: the global state singleton. The single funnel for
  opening a document (`openNote`/`openFile`, where forward/back history is
  recorded), the save path, and vault event handling.
- `editor/`: CodeMirror 6. `livepreview.ts` is the live preview (hiding syntax;
  rendering callouts, images and checkboxes; distinguishing claim from evidence;
  block anchors and wikilink suffix badges). `table.ts` renders GFM tables (a
  StateField maintained incrementally, see below). `queryblock.ts` is the
  `vemry-query` block (v0.3, see "Programmability"). `runblock.ts` is the `js`
  block's run button and output (v0.3). `wikilink.ts` / `mdlink.ts` /
  `clickopen.ts` handle decoration, completion and click-to-follow — in the
  rendered state a plain single click opens (setting `click_opens_links`, on by
  default; ⌘click is promoted to "open the other way", and ⌘↩ opens the link at the
  cursor), while in the source state (the selection touching that link element,
  the same test the live preview's element-level reveal uses) a single click still
  edits, and code blocks, image embeds and drag-selection do not trigger it.
  `completions.ts` provides the `#` tag and `[!` callout completion sources.
  `blocks.ts` is the editor's block model, derived live from the Lezer tree
  (paragraph, heading, list item with nested items as children, quote or
  callout, fence, table, rule — an own range and a subtree each); `blockops.ts`
  the transforms over it (turn into, move with one seam rule, indent, duplicate,
  delete, copy), each one change and one undo step; `blocksel.ts` the block
  selection mode (Esc selects the block at the cursor; the mode's own field
  tints the run and collapses CodeMirror's selection to the head with
  suppressReveal, so tables and query blocks stay rendered) plus the text-mode
  block keys; `fold.ts` heading and callout folding (a hover chevron, a
  "n lines" chip, `[!type]-` folded on open; fold state lives in the editor
  state only); `blockhandle.ts` the hover grip in the left padding (one element
  in the scroller, placed through the height map, never a layout read per
  frame) and `blockmenu.ts` its menu — copy link or embed, turn into ›, move,
  indent, fold, duplicate, delete, ask the AI about the block (the text context
  menu shows the compact group). The context menu grew a second level for
  "turn into". `blockdrag.ts` drags a block by its grip (the file tree's
  pointer contract; the drop boundary comes from the height map and the depth
  from the horizontal travel, one level per 24px for list items) and lands it
  as one `moveBlockTo` transaction. `reveal.ts` is **the one definition of reveal** — inline and line-level
  constructs only accept a bare cursor (drag-selection stays rendered), block-level
  ones also open when `selectionTouches` intersects a range selection, and an armed
  gate keeps a newly opened document and a programmatic jump (the `suppressReveal`
  annotation) from revealing until a real key or mouse action re-arms it.
  livepreview, table, queryblock and frontmatter all share it; a second copy would
  drift. `linkat.ts` holds the three rules for "which link is at this point or
  cursor": wiki before md, not inside code, skip `![[embed]]` — shared by ⌘↩ and
  the editor's context menu. `embed.ts` renders a non-image `![[Note#^id]]` as a
  read-only projection of its source (a block widget alone on a line, a chip
  inside one): a StateField (a block-level replace may not come from a view
  plugin) resolves every target through `resolve_ref` with a per-index-version
  cache, and `renderNoteMarkdown` in `markdown.ts` folds nested embeds into ⧉
  links so depth is one by construction; an embed of the note itself shows a
  marker, and heights are remembered per target so a widget mounts at its last
  size instead of pushing the scroll position around. `provenance.ts` holds the
  block-provenance rows (byte spans mapped to positions once, then mapped
  through edits by the field) and draws the pending bar and the origin overlay
  as line decorations — see "Block provenance" below. `flash.ts` is the one-shot
  landing highlight after a jump (a citation, a query card, a fragment link).
  `src/lib/fields.ts` mirrors the Rust inline-field scanner (the same three
  forms and boundary rule, replayed against the Rust example table by the
  batch's harness): livepreview draws `key:: value` as a chip, a date-valued
  chip opens the same date menu the task chips use, `setFieldValue` rewrites a
  field in place, and `fieldCompletions` offers a key's values (from
  `fields_list`, the vault-wide key and value counts) after `key:: ` and the
  known keys after `key:`.
- `components/`: the activity bar and the three-column layout (both side panels
  collapse, resize and swap sides as a unit, with UI state in localStorage; the
  right side's two panels stay mounted and switch by CSS, so changing tab loses
  nothing), the command palette (grouped into sections on an empty query), the
  settings panel (five tabs, a fixed-height skeleton with a single scrolling
  surface), the outline panel, the research panel (AI conversation and browser
  control in one: the audit-stream timeline; assistant messages rendered by the
  own-subset renderer in `lib/markdown.ts`, which folds `[title](url)` and bare
  URLs into citation superscripts; source cards numbered to match and clippable in
  one click; a "current page" card at the top carrying clip, dock and add-to-context.
  `stores/browser.svelte.ts` mirrors the Rust-side BrowserState and
  `stores/clips.svelte.ts` caches the reverse lookup for a landed citation), and
  the permission diff dialog.

## Key flows

### The AI writes a note

```
the model streams a tool_call(edit_note)
  → runner: prepare_write() computes the diff in memory (nothing touches disk)
  → the audit log records write_prepared
  → PermissionGate.request() suspends the loop
  → a Tauri Channel pushes PermissionRequest{diff} → the frontend dialog
  → the user: allow / allow for this session / always allow in this vault /
    edit then apply / deny (with a reason)
  → a oneshot wakes the loop
  → allowed: vault.write_note() (registers self-write suppression → incremental reindex)
    → vault.record_provenance(the blocks the applied bytes added, under the session id) — pending review
    → AgentEvent::VaultChanged (the NoteModified / NoteAdded / NoteRemoved the write means) → the app broadcasts it like an IPC write — the watcher would have swallowed the change as a self-write, and without this the open note's buffer kept the pre-edit text until reopened
    denied: an error tool result goes back to the model, which adjusts
  → the audit log records permission + tool_result
```

Grant scope (v0.7): "allow for this session" lives in
`RunnerConfig.session_allow` (a shared AtomicBool) and survives across turns,
cleared with the session's lifecycle (new, switch, change vault, delete). It used
to be a local in `run()`, which meant the button said "session" while the grant
actually lasted one turn. "Always allow in this vault" is merged into the
`write_policy` key in `.vemry/config.json`, with a clickable revoke badge in
settings; revoking immediately withdraws the cross-turn grant from a session in
flight (a boolean already honoured inside the current turn is not retroactive, and
per-write approval resumes from the next turn).

### An external edit (another editor can be open at the same time)

```
a notify event → 500 ms debounce → handle_fs_change
  → inside a self-write window? ignore it (loop protection)
  → reparse the one file → update the index incrementally
  → emit("vault://event") → the UI refreshes locally
```

### A clipboard paste (a Finder file or a screenshot → the vault)

Two entry points share `clipboard_paste(dest, stamp, reuse_vault_files)` with
different meanings:

- **The file tree** (⌘V, or "Paste" from the context menu) is a **placement**:
  dest is the folder you clicked, and a file already inside the vault is copied
  too (an explicit duplicate).
- **A note's body** (⌘V) is a **reference**: dest is
  `attachments/<the note's relative path minus .md>/`. Since v0.9 buckets are keyed
  by the full path — every note's bucket is unique and notes sharing a stem no
  longer collide, while a root-level note keeps the path the old rule gave it. The
  frontend's `noteBucketOf` mirrors Rust's `attach_migrate::bucket_for`. Migrating
  existing vaults is the explicit `migrate-attachments` command: a dry-run report,
  then moving directories and rewriting links vault-wide, **moving only what can be
  attributed unambiguously** — same-stem mixed buckets and ownerless orphans stay
  where they are and go into the report. Known trade-offs: a bucket does not follow
  a note that is renamed, and a bucket does not inherit a privacy marking, both as
  under the old rule. A file already inside the vault is not copied; its existing
  path is returned. The frontend inserts the result as a link — `![](…)` for an
  image, `[filename](…)` otherwise, with spaces, parentheses and `%` escaped as
  `%XX`. The division of labour: `[[wikilinks]]` are semantic references between
  notes (resolved through the index and aliases), while a Markdown link is a
  literal reference to a file (a vault-root-relative path) — which also lets a
  future `read_attachment` discover attachments deterministically from body links.

```
clipboard_paste(dest, stamp, reuse)
  → does NSPasteboard hold file paths? if reuse and the source is already in the
    vault → return the relative path;
    otherwise Vault::import_files(): an fs copy (recursing into folders, without
    moving bytes through IPC, at any size), deduplicating names with -1/-2, and
    rescanning into the index if any .md is involved
  → otherwise, are there image bytes? save_attachment(dest/<stamp>.png|tiff)
  → neither → return empty, and the frontend falls back to the File the paste
    event carried (the non-macOS path);
    the editor entry point falls back again to plain text (when what was copied
    was only a URL)
  → the frontend calls refreshNotes; the tree entry point opens the first new
    file, the editor entry point inserts a link at the cursor
```

File paths take priority over image bytes: copying a file in Finder puts both a
file URL and a preview TIFF on the clipboard, and the other order would paste a
PDF as a screenshot of its icon. The editor's paste interception has to decide
`preventDefault` synchronously while the file path has to be asked for
asynchronously — so the interception test is deliberately loose (Files, uri-list
and image types are all intercepted) and, when the async answer turns out not to be
a file paste, the clipboard's plain text is inserted manually instead.
## Security boundaries

- **Paths**: every path handed in by the agent or the frontend goes through
  `Vault::note_id()`, which rejects absolute paths and `../` escapes — the agent
  cannot read or write any file outside the vault.
- **Write permission**: the gate is in Rust, right next to the ability to write
  files.
- **Privacy zones**: a folder or note marked private does not exist as far as the
  agent is concerned (see "Privacy isolation"), enforced in Rust at three funnel
  functions.
- **API key**: held only on the Rust side, never entering the WebView; stored in
  the OS config directory, never in a vault.
- **No bash tool**: every capability the agent has is a typed domain tool —
  interceptable, renderable, auditable.
- **The second webview**: the standalone AI chat window (label `agent`,
  `agent_window.rs`) runs our own frontend bundle, and `capabilities/agent.json`
  grants it the same core IPC as the main window, matched exactly on that label.
  This is a deliberate widening and does not conflict with the browser window's
  "remote pages get zero IPC" (`browser-*` still matches no capability).
  Capabilities always match a label exactly, never a wildcard — the one exception
  is `clipbar-*` in `capabilities/clipbar.json`: the clip bar's label is numbered
  dynamically alongside the browser window and there is no other way, but that
  prefix is minted only by Rust (clipbar.rs), while a window holding remote
  content is always `browser-*` or `reader` and can never match.
- **The script sandbox**: user scripts have no fs, net or process, only the
  `vemry.*` whitelist; writing needs a one-time grant and is fully audited; the
  agent cannot trigger a script and has no run_code tool — programmability does not
  loosen the "no bash" principle on the AI side; a js code block runs only when a
  person clicks it, and is read-only (see "Programmability").

## Block provenance (post-write review)

The permission gate is consent *before* a write; provenance is review *after*
it. "Allow for this session", a turn that writes five notes, an "edit, then
apply" where one line of fifty was actually read — those land on disk without
being read line by line. So every block can say who authored its current text,
and an agent-written block stays visibly **pending** until the user accepts it.

Nothing is added to the notes. `.vemry/provenance.jsonl` is an append-only
ledger written at the moment text lands (`vemry-core/src/provenance.rs`):

- `{"kind":"agent","ts","session","path","blocks":[{hash,kind}],"hunks":[…]}` —
  an agent write that passed the gate (`execute_write` records the blocks the
  applied bytes added, so an "edit, then apply" counts what the user applied,
  not what the model proposed); these are pending. `hunks` are the write's
  changed regions — the lines it removed and added, with up to three unchanged
  lines on either side — kept so the write can be put back (below).
- `{"kind":"insert","origin":"agent|clip|capture",…}` — text the user placed
  themselves (a chat answer inserted or pinned, a clip, a capture): accepted from
  the start, since the user saw it going in.
- `{"kind":"accept","hashes":[…]}` — flips pending blocks to accepted; a later
  agent write of the same hash makes it pending again.

Which agent writes are pending: only those a **standing grant** let through
without a dialog (allow for the session, allow always) — nobody looked at
that diff. A write the user allowed on the dialog, seen and approved (Allow,
or "edit, then apply"), is recorded as an `insert` line with origin `agent`:
accepted, because the gate was the review. Asking twice for the same diff
was the first thing the manual test flagged.

A block's identity is a hash of its **normalized** text: markers, checkbox,
callout head, trailing ` ^id` and capture stamps are stripped, whitespace folded
(a fenced block keeps its lines and loses only the fences). Provenance therefore
survives a block being moved, indented or turned into another kind, and is
dropped the moment its words change — an edited AI block is the user's. Which
blocks a write added is a multiset difference of hashes, so a block the model
merely moved is not "new". The ledger stores no note text beyond a pending
line's hunks (compaction drops them once the write is accepted) and skips
private paths; a damaged line is skipped on load, and above 5000 lines the file
is compacted to the hashes that still exist.

Origins that need no ledger: a `*(YYYY-MM-DD HH:MM)*` capture stamp, a
selection clip's `> — [title](url) · date` credit line, a whole-page clip's
`vemry.origin: clip` frontmatter, and a pre-ledger `vemry.origin: agent` note
(reported as agent, session unknown, not pending — in a note the ledger knows,
a block without a record is the user's). New agent-created notes now carry
`vemry.agent_session`, and the audit's `permission` row records the
`applied_diff` of an "edit, then apply". IPC: `provenance_get` (every block's
origin), `provenance_accept` (one block or the whole note; not broadcast, since
accepting is not a note event), `provenance_record` (the user-placed inserts),
`provenance_pending` (notes with pending blocks), `provenance_revert_plan`
(below). Known limits: a pending block
moved into another note keeps its state but escapes the per-note count; the
overlay is disk truth with the autosave lag; similarity matching ("yours,
edited") is a seam.

In the UI (`src/lib/editor/provenance.ts`): the editor re-reads
`provenance_get` whenever the index moves or an accept lands, but only while
the buffer is clean — a dirty buffer's blocks are not the ledger's blocks yet,
and the field keeps mapping the last rows through the edits meanwhile. Two
independent layers: an amber bar beside every AI block still awaiting review
(on by default, the `provenance_marks` setting) and a four-colour origin
overlay that a command toggles. The status bar names the origin of the block
under the cursor when it is not the user's, and shows "pending N" as a button
that jumps to the next unreviewed block; the block menu offers "accept AI
change" and "accept all in this note"; the sessions view's write rows open the
note with the overlay on ("view the written blocks") and show the
`applied_diff` of an edit-then-apply. The user-placed inserts are recorded at
their landing points — a chat answer inserted or pinned (`host.ts`, with the
session id), a selection clip (`clipTarget.deliver`, with the URL), a quick
capture (`vault.capture`), and "save as note" (which now also writes
`vemry.agent_session`) — all as accepted lines: the placing was the review.

Putting a write back. Marking reviewed is one half of review; the other is
undoing what the model wrote, block by block. `provenance_revert_plan(text,
local_id?)` takes the editor's buffer and returns byte replacements
(`provenance::plan_revert`): a pending block's write is the ledger hunk whose
added lines hold the block, located in the buffer by its context first and by
its added lines alone when the user has edited nearby, and put back to the
lines it replaced — blocks that arrived in one hunk go back together, and a
hunk that cannot be found exactly once (moved, copied, or from a line older
than the hunks) is reported as missed rather than guessed at. The command
writes nothing: the editor applies the plan as one undoable edit
(`prov.revert`) and saves as it does any edit, so the mark disappears with the
block's hash. The control on an unreviewed block's first line is therefore a
pair, ✓ and ✕; the status bar has "✓ All" and "✕ All"; the block menu and the
command palette carry both. Independently of the ledger, a reload of the open
note from disk — an agent write landing, an edit in another app — is applied to
the buffer as one edit rather than a fresh state (`Editor.svelte`'s `loadDoc`):
the undo stack survives it, the cursor stays, and ⌘Z takes an agent write back
the moment it lands.

## Seams left for later

| Future capability | The seam today |
|---|---|
| tantivy or semantic retrieval (unscheduled; the README roadmap's "next") | the `SearchProvider` trait |
| vemry Publish (distant) | vemry-core has no Tauri dependency and can be reused server-side |
| An AI gateway (distant) | the `ProviderConfig` abstraction |
| PDF and image Q&A, side-by-side translation (below) | `read_file_bytes` / `file_read_binary`; the `currentPath` context channel is type-agnostic; `FilePreview` is parameterised by path with a replaceable rendering engine; `tool_defs()` is an incrementally extensible registry |
| The built-in browser (v0.4, below) | Tauri 2 multi-webview; `FilePreview`'s split direction; the attachments and Markdown-link conventions already accept clipped material; a clip lands on disk and is indexed |
| The agent triggering scripts (distant; needs fine-grained permissions first) | every script binding already takes a `Perspective{hide_private}` parameter, with the filtering implemented and tested; script-level skills could grow on top |
| Block-level backlinks ("which blocks reference this block") | `LinkEdge` already records the source block; the target fragment (`heading` / `block_ref`) survives on `Block.links` and would move into the edge |
| The agent reading one block or an embed's content (`read_block`, embed expansion) | `Vault::resolve_ref` is the primitive; a tool wraps it behind the read-level privacy gate like every other read |
| Rewriting links when a note is renamed | `resolve_ref`'s note part is the only thing a rename changes — a minted `^id` never does |
| Provenance by similarity ("yours, edited") and a per-session "what did it write" view | the ledger is keyed by hash and ordered by time; `pending_notes` is the entry point of the latter |
| Git-blame-like block history | the ledger's lines are already timestamped and append-only |

## How PDF and image Q&A would evolve (a design note)

The target scenarios: asking the agent about the PDF or image currently open, and
side-by-side PDF translation (source on one side, translation on the other, aligned
by page).

The seams already in place are in the table above. Four gaps remain, in dependency
order:

1. ~~**The transport is text-only**~~ — **done (batch C)**: `ChatMessage.content` is
   `Option<Content>`, where `Content::Text(String) | Content::Parts(Vec<Part>)` is
   an untagged enum (plain text still serialises as a bare string, so old sidecars
   and request bodies are byte-identical). `Part::Text / ImageRef{path}` (the
   persisted form) `/ ImageUrl` (the wire form); Chat Completions goes through
   serde and Responses maps to `input_text`/`input_image`. Images become context
   references through `@`, "+" or a paste (see "Privacy isolation" point 4: stored
   by path, inflated when the request is assembled). **The model is still the
   precondition**: the default glm-4.7-flash and DeepSeek are both text-only and
   will reject an attached image — rather than maintain a model capability table,
   the error is shown as-is and the image chip carries a standing hint to switch to
   a multimodal model.
2. **Agent tools cannot read non-md files**: `read_note` goes through `note_id`,
   which forces a `.md`. It needs `read_file` (text — `Vault::read_file` can already
   take it directly) and `read_attachment` (an image becomes a content part; a PDF
   gets its text extracted).
3. **PDFs have no text extraction or indexing.** Extraction is half done: a PDF
   attached through `@` or "+" has its text pulled by `context.rs::read_pdf_text`
   using pdf-extract into `<file-context>`, with a 20 MiB cap, a panic guard around
   the library, and an explicit "no extractable text" note for scans. What is
   **missing** is caching into `.vemry/` and entry into the block index. The main
   path (usable with a text-only model) is extract (pdf-extract/pdfium) → cache in
   `.vemry/` → join the existing block index and search, making a PDF one more data
   source the projections are built from. Side-by-side translation fetching the
   source page by page depends on this too.
4. **The native PDF viewer is a black box**: the iframe's built-in viewer exposes
   no page number or selection and cannot be highlighted or scrolled
   programmatically. That is fine for reading; "ask about this page or this
   passage", page-aligned translation and jump-to-page from a citation all need
   pdf.js instead — a replacement inside `FilePreview` that nothing outside notices.

On the model side: the default glm-4.7-flash is text-only, so image Q&A means
switching to a multimodal endpoint (glm-4v, say). Settings already supports that;
there is no architectural obstacle.

**Suggested order**: the extraction route (PDF extraction + a `read_file` tool +
indexing, which also lays the groundwork for side-by-side translation) →
~~multimodal parts (images)~~ done, and a scanned PDF can reuse the same inflation
path → replacing the viewer with pdf.js (together with side-by-side translation, in
one go). If image resizing or transcoding is ever needed (tiff→png, compressing
past 5 MiB), the seam is the single `inflate_images` function.

## Programmability (v0.3, implemented)

Five capabilities share one QuickJS sandbox on the Rust side (`crates/vemry-script`,
layered as above). What matters most in the design is not what it can do but three
boundaries on what it cannot: **arbitrary code never runs by itself** (opening a
note is not running code, which defends against a poisoned note); **the agent
cannot trigger a script** (no run_code tool, no script-level skills — the "no bash"
principle does not loosen because the app became programmable); and **a script's
writes structurally cannot trigger a workflow** (cascade prevention, below).

### Scripts: discovery, declaration, lifecycle

`.vemry/scripts/*.js`, scanned one level deep, with the filename stem as the script
id. There is **no separate manifest** — a top-level call is the declaration:

```js
vemry.commands.register({ id: "archive", title: "Archive completed tasks" }, (ctx) => { … });
vemry.events.on("note_added", (e) => { … });     // note_added/modified/removed/files_changed
vemry.schedule("0 9 * * *", () => { … });        // a standard 5-field cron
```

Saving the file hot-reloads it: the watcher catches a change under
`.vemry/scripts/*.js` → `VaultEvent::ScriptsChanged` (the same hidden-directory
special case as `.vemry/privacy`) → the host drops every Context and reloads
wholesale. Simple semantics, no stale state. A broken script only gets a
`last_error` (visible in settings, plus an error toast) and does not drag the
others down. `.vemry/scripts.json` stores `enabled` and `write_grant` per script
and travels with the vault.

Commands enter the palette through the existing registry (namespaced
`script:<script>:<command>`) and execute through the `script_command_run` IPC back
into the Rust sandbox — the frontend holds the entry point and no ability to
execute.

### Execution model and resource limits

A single OS thread owns the Runtime and all execution is strictly serialised, so
concurrent writes and re-entrancy are structurally impossible. Every execution
carries a wall-clock fuse (the interrupt handler checks a deadline): 2 s for
loading, 5 s for a command or workflow, 2 s for a js block; the clock pauses while
waiting on the user (a grant or prompt dialog). Memory is capped at 64 MB.
`vemry.prompt` can only be answered in a command context — inside a workflow or a
schedule it returns null immediately, because unattended background work must not
hang on a dialog nobody is looking at.

### Write grants and auditing

The first time a script calls `vemry.write`/`append` it blocks on a one-time
"allow this script to write" grant (a denial throws, and the script can catch it).
Allowing persists it into `scripts.json` and subsequent writes go straight through;
settings can revoke it, after which the next write asks again. Every write — plus
grant_asked/given/denied/revoked, error, console and schedule_missed — is appended
line by line to `.vemry/script-log.jsonl` with its trigger (`command`,
`event:note_modified`, `cron`, `load`). A script cannot write to any dot-prefixed
path: if `.vemry/scripts.json` were writable, that would be self-granting.

### Why workflows cannot loop (a structural guarantee, plus fuses)

A workflow's event source is **only** `commands.rs::broadcast` — the user's IPC
write paths plus external changes the watcher caught. A script's own `vemry.write`
goes through `Vault::write_note` / `write_file` (both register self-write
suppression, silencing the watcher), and the resulting event goes straight to the
frontend through `UiBridge::vault_event` to refresh views, **never into broadcast**.
So "a workflow subscribed to note_modified edits a note in its callback" runs
exactly once: the cascade does not exist topologically. The fuses on top: the same
(event, path) coalesces within 500 ms (a 1 s autosave will not machine-gun it), the
pending queue is capped at 100 (over that, drop plus an error toast), and a single
execution has a 5 s fuse. A cron trigger missed while asleep or not running is
skipped rather than replayed, leaving one `schedule_missed` line.

### Query blocks and runnable js blocks (the editor side)

- A ` ```vemry-query ` fence with a YAML body (eight flat scalars —
  `kind/note/tag/text/task_state/due_before/field/limit` — with `due_before: today` as
  sugar, plus the presentation keys `group` and `sort`, which the frontend applies
  in `views/queryrows.ts` and never sends to Rust; a 20-line parser, no js-yaml
  dependency). When the cursor is outside the
  block, the whole fence is replaced by a live-results widget (`queryblock.ts`,
  the same StateField block-replace semantics as table.ts); clicking a row jumps
  through `openNote` and enters navigation history. A change to
  `vault.indexVersion` triggers a re-query through the `queryRefresh` effect — the
  same refresh signal the projections use. In any standard Markdown renderer it
  degrades to an ordinary code block. No lock-in.
- A `js` fence gets a Run button on its first line (`runblock.ts`, beside the copy
  button): clicking sends the block's contents into a one-shot read-only Context
  (`script_eval_block`, a 2 s fuse, `vemry.write` throws on call, no grant flow),
  and console output plus the return value render in a block widget after the
  closing fence. That output exists only in editor state — it is never written to
  disk and disappears when the document closes. Why a js block gets no write
  capability: a grant is persisted against a *script's identity*, and a code block
  has no stable identity (the body can be edited freely, or synced in from
  outside). If it needs to write, it graduates to `.vemry/scripts/`.

### Block types (typed blocks on Markdown)

`.vemry/types/*.md` (`vemry-core/src/types.rs`, the same loader shape as the
skills: regular `.md` files only, a symlink refused, malformed frontmatter
degrading to defaults) declares a kind of block plus its inline fields, tag
and template. The editor lists the enabled types in the `/` menu and under
"turn into ›" (`editor/blocktype.ts`: one transaction that turns the block
into the kind and appends the fields it lacks); the system prompt names them
under `## Block types` so the agent writes such blocks the same way; the
watcher emits `TypesChanged` for the directory, which refreshes the menus and
invalidates the prompt snapshot. IPC: `types_list`, `types_init` (the example
`book.md`). A typed block is ordinary Markdown with fields — no note points
at a type.

### Skills (instruction-level)

`.vemry/skills/*.md`, with frontmatter `name` (defaulting to the file stem),
`description` and `disabled`. VEMRY.md is the resident constitution; skills load on
demand: `build_system_prompt` injects a `## Skills` summary section each turn
(names and descriptions only) and the model calls the `read_skill` tool for the
full text when it wants it (read-level, truncated at 30k characters). `read_skill`
matches a name exactly against the already-scanned list and never concatenates
caller input into a path, so it is immune to traversal by construction.

**Privacy semantics (a deliberate trade-off)**: `.vemry` is a built-in privacy zone
as far as the agent is concerned, and the skills directory is a deliberate, narrow
exception — the whole point of a skill is for the agent to read it. The exception
covers the single `read_skill` tool and reaches only `.md` files directly under the
skills directory; `read_note` and every other tool still cannot touch anything
inside `.vemry` (a regression test pins this). To hide a skill from the agent: set
`disabled: true` or delete the file.

### Visibility follows the trigger (an API constraint pinned down for later)

Every script binding takes a `Perspective{hide_private}`. In v0.3 all script
execution is user-triggered (command, workflow, schedule, js block), so it is always
the user's perspective: privacy zones are visible, exactly as in the UI. The
`hide_private=true` filtering — across list/query/search/read/write, with error
messages byte-identical to a genuine absence — is implemented and tested but no
code path enables it. It is the parameter reserved for a future "the agent triggers
a script": when that arrives, the execution perspective follows the trigger instead
of someone reinventing the filtering.

### Known limitations (deliberate)

- A script command's `hotkey` field is display-only and binds nothing (built-in
  hotkeys are likewise hardcoded in App.svelte — the registry's hotkey has always
  been a display field).
- Renaming or deleting through the UI emits the `IndexReady` summary event, which
  does not trigger workflows (an external tool's change goes through the watcher as
  a typed event and triggers normally).
- Event callbacks do not see intermediate states that coalescing folded away
  (several edits to one path within 500 ms fire once).
- A script reads and writes the vault from the user's perspective: a script the
  user installed is trusted as the user is (with the grant gate and the audit log
  as the backstop for writes).
## Projections (v0.2, implemented)

All seven views are frontend rendering of "a view is a query", opened equivalently
three ways — an activity-bar icon, the menu bar, or the `view-*` commands in the
palette — and left with Escape or the ✕ in the view header
(`views/ViewClose.svelte`, shared by all seven). The host mechanism:
`vault.mainView` switches the main area while the Editor stays mounted and is
hidden by CSS — CodeMirror holds the unsaved buffer and the undo stack, and
unmounting would lose edits. `vault.indexVersion` increments on every `VaultEvent`
and is the single re-query signal for every view (an external file change → the
watcher → the view refreshes itself). Clicking any row inside a view calls
`openNote` and returns to the editor, sharing the navigation contract and entering
forward/back history. Components live in `src/lib/components/views/`, pure logic in
`src/lib/views/`.

| View | Data source | Notes |
|---|---|---|
| Agenda | one `query_blocks {kind:task}` for everything, grouped in the frontend | Overdue / today / next 7 days / later / in progress / no date / done. A checkbox rewrites disk precisely by the block's span (a span is a UTF-8 byte offset, sliced through TextEncoder; a stale span refuses the write and re-queries) |
| Board | the same `query_blocks {kind:task}` as Agenda | Five columns: in state mode, or — grouped by any inline field the tasks carry — one column per value plus "none", where a drop writes `[key:: value]` and a quick create carries it; the state columns are overdue / today / scheduled / unscheduled / done (the grouping mirrors Agenda's `whenOf`; `doing` is orthogonal to dates and gets a marker plus a pin to the top rather than a column of its own). Dragging a card between columns is one merged `updateTask` span write — state and date written at once, because two writes would leave the second span stale. Dropping into "scheduled" opens the date menu first. Clicking a card converts through `byteToChar` and jumps to the task line. Dragging is pointer events plus the suppressClick contract (the same as FileTree; no HTML5 DnD) |
| Timeline | `vault.notes` (no new IPC) | Frontmatter `created` is used when it parses (`NoteSummary.created` exists for this), otherwise mtime; grouped by day, newest first |
| Claims / Evidence | two `query_blocks` plus `resolve_link` for the supports join | Resolution goes through the index's own resolver (aliases, shortest path), so the join necessarily agrees with Backlinks and with the agent. Only claims with an `explicit_id` match; unresolved and no-matching-^id are shown distinctly |
| Graph | a new IPC, `graph_get` (`BlockIndex::graph()` aggregates resolved edges) | d3-force layout with hand-drawn Canvas; an incremental refresh keeps old node coordinates by id so the layout stays put; zoom, pan and drag are hand-written rather than pulling in d3-zoom; private nodes get a dashed outline. UI-only — it is not an agent tool |
| Session replay | `sessions_list` / `session_read` (no new IPC) | Replays seven JSONL row kinds: user, assistant, tool_call, tool_result, write_prepared (diff colouring reuses `views/diff.ts`), a permission badge and a highlighted privacy row; a bad row falls back to raw rather than crashing. The one refresh path that does not use `indexVersion`: the agent store's `activity` counter counts discrete events (tool start and end, permissions, turns — not per-token deltas) and the view re-reads after a 300 ms trailing debounce, so a replay grows along with a running session (Rust appends line by line), keeps expansion state across re-reads (the log only appends, so indices are stable), and follows the newest session unless the user picked an older one. A title can be renamed by hand (the audit appends a title override row, last one wins), and a forked session carries a ⑂ marker in the list (it shares the source's first user message, so derived titles are necessarily identical) |

Views **do not filter** private notes — you see everything, as you do in the file
tree, with the privacy badge on every private note (a view is a flat list, so each
note carries its own; the tree badges only the zone root and dims the rows inside).

## The app shell (activity bar, menu bar, context menus — implemented)

Three visible entry points, one shared principle: **none of them grows a second set
of semantics**. Every item ends at the same command id in `commands/registry.ts`
(the activity bar and context menus call `executeCommand`; the menu bar bridges
back through `menu://command` to the same function). Visible entry points exist for
people who do not know about ⌘P yet, not as a parallel feature set; conversely, a
new feature registered as a command is instantly present in all three plus the
palette.

- **Activity bar** (`ActivityBar.svelte`, a 44 px vertical rail): the file-tree
  toggle and full-text search at the top, the seven projections in the middle,
  settings pinned to the bottom. A full-height 2 px accent bar marks only the
  current view (the file-tree button is a toggle and merely brightens).
  Structurally it is a flex sibling **outside** `.layout` (a grid), both inside the
  `.shell` wrapper — which is why the `gridCols` source of truth and
  `.layout.swapped`'s order needed no changes, and why swapping sides is just
  `row-reverse` on `.shell.swapped`, leaving the rail naturally against the outer
  edge of the window. The `padding-right` that yields to a docked browser window
  hangs on `.frame` (the vertical wrapper around `.shell` plus the status bar), not
  on `.layout`: after a side swap the rail is at the right edge and the status
  bar's right segment is the cursor readout, and a docked window is the full height
  of the main window — miss either and the native child window covers it. Its width
  is the common term in three coupled minimum widths, and widening it means
  changing all three: `tauri.conf`'s `minWidth` 764, `window.rs`'s
  `MIN_MAIN_VISIBLE` 804, and the sidebar `startResize` ceiling minus `ABAR_W`.
- **Editor tabs** (`TabBar.svelte` plus the tab model in `vault.svelte.ts`): the
  core invariant is **one live buffer** — at any moment only the active tab holds a
  live CodeMirror instance and only it can be dirty. Background tabs hold a
  snapshot side table (EditorState, scroll position, eol, and the content as it
  would be written), which is **not reactive** — putting an EditorState into
  `$state` deep-proxies it and breaks its internal identity comparisons; an LRU of
  20 drops the oldest, losing only its undo history. The truth is on disk, and
  activating re-reads it and validates lazily (identical → `pendingRestore` reuses
  the whole state and keeps undo; different → rebuild). So the `=== currentPath`
  tests in `dirty`, dirtyGuard and insertBlock keep their original meaning, and
  `FilesChanged` needs no path and no Rust change. Opening semantics are preview
  tabs: a single click uses the preview slot (italic; at most one `pinned:false` in
  the bar, and later single clicks reuse it); a real edit (the `loadingDoc` gate
  filters out the spurious onDirty from resetDoc/setState), a double click on the
  tab title, or a double click in the file tree promotes it; ⌘click, middle click
  and "new" are pinned immediately. **Both uniqueness properties — one preview
  slot, one tab per path — are maintained by the single `openNote`/`openFile`
  funnel**; do not open another entry point elsewhere. `activateTab` is one of the
  flush funnel's call sites (snapshot after flush; `state.doc ≡ disk` is what makes
  lazy validation sound). Deletion and external removal go through `dropTabs`,
  which deliberately **does not flush** — flushing would resurrect a dead file. A
  rename rewrites the path in every tab in place and keeps the cache. On mac ⌘W is
  always taken by the native menu accelerator, so `menu.rs::close_focused_window`
  emits the `close-tab` command back to the frontend for the main window (non-mac
  goes through App's keydown); ⌘⇧[ / ⌘⇧] and ⌃Tab cycle. The tab list persists per
  vault (the `vemry-tabs` bucket) and must be restored in the same synchronous
  batch as `adoptRoot` — the same invariant the expanded set has.
- **Menu bar**: see `menu.rs` under Layers — macOS only, accelerators split three
  ways, compiled to a no-op stub elsewhere.
- **The standalone AI chat window** (⌘J / `focus-agent`,
  `src-tauri/src/agent_window.rs` + `src/AgentWindow.svelte`): when the panel's
  agent page is visible, ⌘J focuses the panel (the input flashes and hints "press
  ⌘J again to pop out a separate window"); pressing again with the cursor already
  in that input, or pressing it when the panel is hidden, opens (or focuses) a
  **native window** hosting the same `AgentPanel` — draggable out of the main
  window and onto another screen, which a DOM overlay physically cannot do since it
  cannot paint a webview. One bundle mounts different root components by
  `getCurrentWindow().label` (`main.ts`), not by a URL parameter. The two webviews
  are two stores, and **the truth for conversation state is always the session
  jsonl on the Rust side** — both windows are projections. On opening, the main
  window hands the composer state (draft, chips, effort level, dismissedNote) to
  Rust as opaque JSON (`agent_window_open`); the chat window takes it at startup and
  rebuilds its timeline from the jsonl through `agent_session_current`. Closing is a
  handshake: the native ✕ or ⌘W reaches `CloseRequested`, which first calls
  `prevent_close` and asks the webview; the webview cancels any running turn and
  calls `agent_window_close(handback)`, and the second pass is allowed through;
  `Destroyed` notifies the main window via `agent-window://closed`, which takes the
  handback and rebuilds (if the webview is already dead, clicking ✕ again just
  closes). The yielding invariant takes a new form: while the window is open, the
  panel's agent page renders only a "the conversation is open in a separate window"
  placeholder — the session list and page card are not rendered either, so nobody
  switches sessions in two places — and a session always has exactly one visible
  composer. "The current note" inside the chat window comes from the main window
  (`agent_window_set_note` → `agent.hostNote`; the chat window's
  `vault.currentPath` is permanently null); the editor's selection snapshot works
  the same way (`vault.editorSelection` is recomputed by the Editor as the
  selection changes → the main window debounces `agent_window_set_selection` →
  `agent.hostSelection`, and the value at the moment of opening rides the handoff).
  The `@` picker's "current selection" and the chips row's automatic selection chip
  (select-and-it-comes-along: the live chip follows the selection; ✕ records a
  `dismissedSelection` identity key and it returns as soon as the selection
  changes; picking it from `@` pins it as an explicit chip) each read from their own
  window's copy. Opening a note, inserting, pinning, the `#` convention and the page
  card's clip commands all have to happen in the main window (the editor and the
  clip target only exist there), and all go through `hostDispatch` in
  `src/lib/host.ts`: inside the chat window it posts to Rust, which relays to the
  main window (navigation actions bring the main window forward on the way); inside
  the main window it executes directly. There is only one `runHostAction`. The chat
  window's vault store joins the already-open vault through `attachCurrent()` (the
  listing plus event subscription, with expanded and root restored from the bucket
  in the same batch — otherwise the module-level persistence effect would write the
  main window's expanded set empty). On a vault switch the main window first calls
  `agent_window_close(discard=true)`: the handback belongs to the old vault and the
  whole package is dropped. Geometry (in logical pixels) is kept in memory on
  Moved/Resized and written to settings on Destroyed, so the next window opens in
  place.
- **Context menus** (`stores/contextMenu.svelte.ts` plus the **one** `<ContextMenu />`
  mounted in App): menu items **are data, not layout** — `entries` is computed for
  the context at the moment of opening (a dynamic undo label, paste's two states, a
  snapshot of the privacy toggle, delete-only for VEMRY.md). Six mount points (the
  file tree, the editor body, a research-panel session row, a replay-view session
  row, a backlink entry, the sidebar tab strip) share one implementation and one
  pair of window capture listeners — before it was made a singleton each mount
  point held its own pair, and since at most one menu is ever visible, those twelve
  resident listeners were pure waste. Dangerous items use the `confirm` field for a
  two-step confirmation (the first click changes the label, the second executes, and
  clicking away or Escape resets), which is why no extra dialog is needed.

### Type and typography

- **`-webkit-font-smoothing: antialiased` is deliberately not set** (it was a v0.1
  scaffold default, not a decision): it turns off macOS's weight-adding smoothing,
  and SF plus PingFang go thin and washed out, most visibly on a dark background.
  The evidence: measured across two mainstream desktop editors' shipped bundles,
  the property occurs zero times in one and only locally in the other, never on
  `body`. `body` uses `line-height: 1.5` plus `text-rendering: optimizeLegibility`,
  and `:root` declares `color-scheme: light dark` so native scrollbars and form
  controls follow the theme.
- **Only weights 400/500/600**: PingFang SC's weights are discrete and stop at
  Semibold (this machine's `PingFang.ttc` measures 100/200/300/400/500/600 with no
  700 face), so writing 550 or 650 snaps Chinese to a neighbouring weight while
  English (SF is variable) genuinely renders at 550/650, and a mixed line comes out
  unevenly weighted. `strong` is therefore 600 rather than 700, and there is not a
  single `font-weight: bold` in the repository. New code should not introduce
  off-scale weights.
- **Body width and size**: `--content-width: 42em` is shared by the editor's
  `.cm-content` and the reading projection. `--fs-note` is the repository's **only**
  precedent for writing a CSS variable at runtime (`prefs.applyNoteFontSize` writes
  it on `documentElement`); Rust's `settings_set` clamps it to 15..=18 and the
  frontend clamps the same way in case someone edits the JSON. CodeMirror cannot
  observe a CSS variable change, so `Editor.svelte` has to call
  `view.requestMeasure()` when the size changes.

## Editor experience (implemented)

### Autosave

Writes to disk one second after typing stops (the debounce lives in
`Editor.svelte` — the content's truth is the `EditorView` it holds, so every
lifecycle edge that needs a cancel or a flush is in its `$effect`). ⌘S still saves
immediately. The points that matter:

- **Notes and plain-text files both autosave** (since batch B): once `file_write`
  gained Rust-side self-write suppression (`self_file_writes` in `vault.rs`,
  added alongside script writes), the old "only save .md" restriction lifted —
  writing produces exactly one explicit `FilesChanged` broadcast, the same shape as
  a note's `NoteModified`. An open text file with a clean buffer re-reads and
  compares on that event and reloads only if it changed; external deletion or
  renaming (a failed read plus absence from the file list) closes the document the
  same way `note_removed` does, with a toast when there were unsaved changes. A
  non-md write restores the CRLF line ending remembered at load time (CM is always
  `\n` internally), normalising mixed endings to CRLF. Images and PDFs have no edit
  buffer and are not involved.
- **Write the old before reading the new**: `flushAutosave` returns the write
  promise and registers as `vault.flushEditor`, and the store's four funnels
  (`openNote`, `openFile`, `renamePath`, `openVault`) await it **before** reading,
  moving or switching — otherwise a rename would move the old content off disk, a
  pending save would write a phantom file into the new vault, and reopening the
  current note would reset the buffer from stale disk content. The editor's own
  flush points: before an external reload, on the container-unmount branch when a
  note becomes an image or PDF preview (flush, not cancel), and on window blur.
- **The dirty signal**: the pending timer *is* the flush's dirty signal
  (`openNote` resets `vault.dirty` first, so flush cannot depend on it), which
  means the timer may be rescheduled but never dropped — scheduled writes pause
  while the agent runs but the timer is rescheduled (a document the user switches
  away from still flushes through the funnel); a failed save reschedules a retry;
  and ⌘S cancels the timer after writing (otherwise, when an external change
  triggers a reload, the flush would write the stale buffer back over the external
  edit). `saveCurrent` carries an edit-sequence guard: if more input arrived while
  the write was in flight, dirty is not cleared.
- **An existence guard** (the same `docExists` for both timer and flush): after a
  rename or delete the old path is gone from the listing (`.md` checks the notes
  index, everything else checks `otherFiles`), and writing back would resurrect the
  old file. `deletePath` calls `discardEditor` on entry to drop the pending timer —
  an undo of a bulk delete does not refresh the listing one by one, so relying on
  the listing guard alone would miss.
- **Save-echo suppression**: `note_write` re-emits `NoteModified` unconditionally
  (the compensating channel for a privacy marking), and `handleEvent` re-reads and
  compares against `currentContent`, skipping `reloadCounter++` when they match —
  otherwise every save would go through `resetDoc`, snapping the cursor to 0 and
  clearing the undo stack. That was a latent bug back in the ⌘S-only days; autosave
  turned it into a hard blocker, which is how it got fixed. The Backlinks panel
  therefore does not watch `reloadCounter` and re-queries on `indexVersion`.
- Known trade-off: there is no Tauri close hook, so quitting the app can lose at
  most the last second of typing.

### Back and forward

Browser-style document history (`VaultStore.navStack`: a new visit truncates
forward, consecutive duplicates collapse, capped at 100). Recording is concentrated
in the store's open funnels (the success tail of `openNote` plus each success
branch of `openFile`), leaving about twenty callers untouched; `goBack`,
`goForward` and rename-following bypass recording through the `navigating` flag.
Deletion and external removal prune (including sub-paths); a rename rewrites the
path in place rather than adding a visit. Only documents are recorded, not
projections (a projection is a mode you leave with Escape, not a destination) —
with one exception, the reading projection: entering it records a sentinel entry in
navStack (`READING_ENTRY`, whose NUL prefix cannot be a path, so prune's path
predicate never matches it; adjacent projection entries merge when the documents
between them are pruned), so a clip note landed from the projection can go ← back
to it, while each page inside the projection stays with the reading store and never
enters navStack. The keys are ⌘⌥← and ⌘⌥→ — ⌘[ and ⌘] are taken by CodeMirror's
defaultKeymap for indentation, and binding them again at the window layer would
fire twice.

### Input completion

Three sources sit side by side in `autocompletion({override})`, each yielding when
its matchBefore misses: `[[` for note names (stem plus alias), `#` for tags (every
tag in the vault ordered by frequency; the Unicode class covers CJK, and a bare `#`
at the start of a line is probably a heading, so only ⌃Space triggers it there), and
`[!` for callout types (the full `CALLOUT_GROUP` including claim and evidence, with
the group shown as detail; a bare `[!` at the start of a line triggers too, and
accepting inserts the `> ` prefix). Cooperating with `closeBrackets()` is a
question every apply function has to answer: if `]]` or `]` already follows the
cursor, insert only the name, otherwise insert the closer too — a static apply
double-closes into `[[name]]]]`. Completion candidates (link targets, tags) filter
out private notes in the frontend; the resolved test (`knownTargets`) still uses the
full set, because an existing link pointing at a private note should not be shown as
unresolved.

### Rendering semantic markers in the live preview

claim and evidence are a first-class callout group (flag and scales icons, rose and
cyan) rather than falling into generic info. Block references are differentiated by
role: at the definition, the trailing ` ^id` renders as a mono anchor badge (a mark
decoration that survives on the cursor's own line); at the reference,
`supports:[[note#^id]]` renders as `supports → note ¶` (the block id is an internal
anchor name, hidden while reading, with ¶ meaning it points at a block rather than
the whole note); `[[note#heading]]` renders as `note › heading`. The cursor touching
an element's range restores the raw syntax through the shared reveal guard (an
element-level test, so the cursor elsewhere on the same line no longer explodes the
whole line; only a bare cursor counts — drag-selection and ⌘A stay rendered, and a
programmatic jump's landing point does not reveal either, thanks to the armed gate).

### Incremental table rendering (table.ts)

When the selection is outside a table, the whole GFM table renders as a real
`<table>`, and clicking a cell puts the cursor back at the matching position. A
block-level replace that spans lines can only come from a StateField (a ViewPlugin
is not allowed to), and a StateField does not know the viewport — so the naive
implementation rescans the entire document on every keystroke and cursor move. Two
things make it incremental:

- **Widgets are decoupled from absolute positions**: a cell's `data-pos` holds an
  offset relative to the table's start, and a click asks `posAtDOM` for the table's
  current position and adds. A widget's `eq` compares only the source text, so an
  edit elsewhere in the document only needs to `map` existing decorations rather
  than rebuild them.
- **Only the dirty region is recomputed**: each transaction collects the edited
  ranges, the old and new selections (a table expands or collapses as the selection
  enters and leaves), and the region incremental parsing newly advanced through;
  those are widened to whole lines, merged, then widened again to the full extent of
  any intersecting old decoration and Table node, looping until it converges — a
  table straddling a dirty-region boundary is either recomputed whole or left alone,
  because computing half of it produces overlapping block decorations and CodeMirror
  throws outright. Moving the cursor within an ordinary paragraph does not even
  change the RangeSet reference.

Rows are split from the source text on unescaped `|` rather than using the syntax
tree's TableCell — an empty cell produces no node, which would shift the columns. A
cell's inline Markdown goes through a tiny display-only renderer. Correctness is
backed by headless fuzzing: over random sequences of edits and selections, it
asserts step by step that the incremental result is identical to a full recompute.

### Persisting UI state

`localStorage["vemry-ui"]` holds `{leftOpen, rightOpen, rightTab, leftW, rightW,
swap}` (written through by `$effect.root`) — the first frontend persistence
precedent, deliberately kept out of Rust settings: purely presentational state is
not worth an IPC round trip. `leftW`/`rightW` are the sidebar widths the user
dragged the sash to (in px; the default is each panel's flexible width), and `swap`
exchanges the two sidebars. Bad data falls back to the defaults through try/catch.
Which documents are open is not persisted (startup restores only the vault).

That file is **global** state; **per-vault** state (the file tree's expanded set,
the pinned clip target) goes through the bucketed table in `persist.ts`, described
under Layers.
## Privacy isolation (v0.2, implemented)

Any folder, note or single block can be marked a "privacy zone": the AI never
reads it, never writes to it, and it never appears in the system prompt — as far
as the model is concerned that content **does not exist**. You keep seeing all of
it in the UI.

The principle: **privacy is a property of the vault, not of a session**. Rules live
in `.vemry/privacy` (a gitignore-syntax glob list, compiled by
`vemry-core/src/privacy.rs::PrivacyScope` with the `ignore` crate,
case-insensitively — APFS is case-insensitive, and a path `fs` can open while the
glob misses would be a leak channel), travel with the vault, and hold against any
model endpoint. Per-note marking with frontmatter `private: true` is also supported
(surfaced as `NoteSummary.private`). `Vault::is_private()` is the union of glob
matches and frontmatter markings, and any path segment starting with `.` is private
by construction (which covers `.vemry/privacy` itself, the session logs, and
`.obsidian`). The fourth marker is a block: a `> [!private]` callout is
`BlockKind::Private`, and the parser indexes none of its tags, links or fields
(so no aggregate — tag counts, backlinks, the graph — can carry a trace of it).

Enforcement points (all in Rust; the agent reaches the vault only along these
paths):

1. **The read-level semantics are "it does not exist", not "you may not"**: in
   `execute_read_tool`, list/search/query_blocks filter out private notes and their
   blocks; `get_links` presents a resolved link pointing at a private note as
   UNRESOLVED and drops backlinks coming from private notes; `read_note` returns a
   byte-identical `note not found` for a private path and a genuinely missing one
   (tightened along the way to "not in the index means it does not exist", which
   closed a bare `fs::read` gap for `.md` files in hidden directories). Reporting
   "no permission" would itself disclose that the file exists.
2. **Writes are refused at the `prepare_write` stage**: no diff is generated, no
   permission dialog appears, no `write_prepared` is recorded, and an error tool
   result goes straight back to the model, indistinguishable from `invalid path` or
   `note not found`. Glob rules apply to paths that do not exist yet, so "create a
   file inside a private directory" is refused too.
3. **The prompt does not leak**: `build_system_prompt` aggregates its note count,
   folder list and top tags from visible notes only; the currently open note is
   omitted entirely when it is private; and VEMRY.md is not injected at all if it is
   itself marked private.
4. **`@` context references the user hands in pass the same gate**
   (`vemry-agent/context.rs::resolve_context_refs`, inside the runner's vault lock,
   under the same lock and at the same moment as the current-note filtering —
   privacy can change between picking and sending, and only the state at resolution
   time counts): notes and folders reuse the `execute_read_tool` funnel verbatim;
   text attachments get their own gate of the same shape (privacy before existence,
   worded byte-identically to `read_file`'s not-found). **An editor selection**
   (the select-and-it-comes-along `@ current selection`) is the one vault reference
   whose body the frontend hands in — what was selected is text in a buffer, possibly
   unsaved, so disk line numbers may not match and Rust cannot read that buffer;
   it is still gated by path through `is_private_rel` (glob ∪ frontmatter), so a
   selection made inside a private note is dropped whole, the model sees the same
   wording as "attachment unavailable", and only the `privacy` row's filtered count
   moves. The audit's user row records only kind and path in `refs` (plus `lines`
   for a selection); the body, like web page context, goes only into model history.
   The text-attachment gate also covers **PDFs** (extension-dispatched to
   pdf-extract for text, still a `file` reference in a `<file-context>` envelope).
   **Image references** (`@` candidates, the composer's "+" attachment, a pasted
   screenshot — the latter two land in the current note's `attachments/<note>/`
   first and are then referenced by path) are the fourth read path, with the same
   gate and privacy before format, existence and size; but the bytes **do not enter
   the envelope at resolution time**: history stores only `Part::ImageRef{path}`
   (a sidecar never contains base64), and every time the runner assembles a request
   `context.rs::inflate_images` runs **the same gate again** inside the vault lock,
   reads the bytes and swaps a base64 data URL into the request's copy (at most six
   images across history, 5 MiB each, png/jpg/gif/webp only, neither resized nor
   transcoded — no image crate was introduced); anything unreadable becomes an
   "attachment unavailable" line. The side effect is that images are re-gated every
   turn: an image later deleted or marked private is only a placeholder in
   subsequent requests, which is stricter than text references (whose original
   envelope is frozen and replayed). Summarisation requests (level-two compaction)
   carry no images.

5. **History envelopes are re-gated every turn when a request is assembled**
   (`context.rs::scrub_private_refs`, at the same lock and the same point as
   `inflate_images`, plus one run over the original history at the top of each turn,
   so what `write_history` persists at the end of the turn is already clean; the
   **summarisation request** of level-two compaction is a separate outbound path and
   passes the same gate first): once a text reference's body has entered a user
   envelope it is frozen in history and the sidecar, so when it is marked private
   afterwards this gate replaces the now-private envelope wholesale with output
   byte-identical to what resolution time would have produced (a note, attachment or
   selection becomes the single "attachment unavailable" line; a folder's body
   becomes `(no notes)`; the turn tail's "currently open note" item is removed).
   Location is by the `ChatMessage.context_refs` metadata (kind and path, marked
   `serde(skip_serializing)` so it never enters a request body, and written into the
   sidecar by hand), with envelope-attribute scanning as the fallback for older
   sidecars. The transformation is deterministic: when privacy has not changed the
   output bytes do not change, and the prefix cache still hits. Retry replays of the
   original envelope, switching to an old session, and forking all pass this gate.

6. **Private blocks are cut out of every read, by one set of functions**
   (`privacy.rs`: `redact_private_blocks`, `blank_private_blocks`,
   `apply_redacted_edit`, `private_blocks_preserved`; the vault names the agent's reads `read_note_for_agent`,
   `search_for_agent`, `memory_file_for_agent`, `resolve_ref_for_agent` so the
   choke points can be grepped, and the raw `read_note` its many UI callers use is
   never handed to the agent by a wrong default). The table:

   | Agent-visible path | Function | What is removed |
   |---|---|---|
   | `read_note`, an `@note` reference | `read_note_for_agent` (disk-fresh, re-parsed; no placeholder; one adjacent blank line goes with the block) | the private blocks, counted as filtered |
   | `search_notes` | `search_for_agent` | a hit in a note with private blocks is re-matched over a blanked copy: the snippet cannot come from a private line, and a query satisfied only together with private text drops out (the AND matcher would otherwise confirm private words one term at a time) |
   | `query_blocks` | the tool's `retain` | private blocks (a `kind: private` query finds nothing); positional ids are **not** renumbered around the gap — one id space for the UI, the editor and id minting |
   | `get_links` | the index | nothing to do: a private block's links were never indexed |
   | an `@selection` | `gate_selection`, two layers | lines equal to a line of one of the note's private blocks on disk (by content, not line number, so an unsaved edit above cannot shift the cut and a selection starting mid-callout is caught), then the private blocks the selection text itself contains |
   | history envelopes, every turn | `scrub_private_refs` | a note envelope whose note now holds private blocks is rebuilt from the same read; a selection envelope with a line equal to a private line becomes "unavailable" |
   | the system prompt | `memory_file_for_agent` | VEMRY.md's private blocks; a VEMRY.md write also invalidates the prompt snapshot, so the change applies from the next turn |
   | a block reference or embed for the agent (seam) | `resolve_ref_for_agent` | a private target is a missing anchor; a section or subtree is handed over redacted |
   | scripts under `hide_private` | the same three vault functions | the same |
   | `write_note` | `prepare_write`, re-checked in `execute_write` | refused outright for a note holding a private block ("use edit_note"): the model never saw the block, so its "full content" would delete it |
   | `edit_note` | `apply_redacted_edit`, `private_blocks_preserved` | `old_str` is matched in the model's view of the note — the text `read_note` returned, private blocks cut out — so a passage the model copied always matches, even one spanning a hidden block; the replacement lands in the raw text, a private block that sat inside the replaced span is kept and re-inserted right after the replacement on lines of its own, a block at the edge stays put, and the edit is refused when what the model wrote around a block would stop it parsing as one (a fence opened above it and never closed, its line glued onto a quote) or when it changes nothing (the line break before a hidden block cannot be taken away, and a "wrote" with no change would send the model round again); the diff the user approves is over the raw text, and the size in the "wrote" result is the view's |

Auditing: when filtering happens the runner writes
`{"kind":"privacy","tool":…,"filtered":N}` into the session JSONL — a count only, no
paths; an `@` reference the privacy gate filtered out does not even get its kind and
path into the audit's user row `refs` (the ledger lives inside the vault, so
recording the path would write the private path back to disk). The session-replay
view lets you watch the rules working.

UI: a `> [!private]` block wears the privacy accent in the editor with the same
struck-out sparkle for its icon (the tint stops at the last `>` line, so a lazy
continuation line is visibly public), the status bar shows "private block" while
the cursor is inside one, and a selection touching one is never offered as a chat
chip. The file tree's context menu and the palette's "mark as private"; a private
path shows a badge — the AI's four-point sparkle struck out, "no agent here"
(visible to you, invisible to the AI). One drawing, `PrivacyIcon.svelte`, serves every site — the tree, the projection views, the
status bar, a private reference chip in the chat and the privacy row of the
session replay. In the tree the badge sits on the *zone root* only (a private path
with no private ancestor folder); the rows inside a private folder are dimmed
instead, and their tooltip names the folder that carries the badge. The zone root
is derived in the frontend from the flat `privacy_list` set (walk the ancestors),
which has one known degradation: a rule that names contents rather than the
folder (`diary/**`) leaves `diary` itself out of the set, so each direct child is a
root of its own — write `diary/` for one badge. Because `privacy_unmark` removes
only the exact rule line, the context menu offers "unmark" on a zone root only and
shows a disabled "inherited from …" item on a row inside one (the palette command
says the same in a toast); an unmark that leaves the path private (a broader glob
still covers it) also ends in a toast. The settings panel's "Privacy zones" page
lists the raw rule lines in `.vemry/privacy` (add / remove, `!` negations and
comments included) and the notes marked one by one through frontmatter (open /
unmark). Marking a `.md` note writes frontmatter (so it travels with the
file); marking a directory or a non-md file writes a rule line into
`.vemry/privacy`. IPC: `privacy_list` / `privacy_mark` / `privacy_unmark` /
`privacy_rules` / `privacy_rule_add` / `privacy_rule_remove`; editing
`.vemry/privacy` externally is caught by the watcher (a `PrivacyChanged` event) and
takes effect immediately. Marking or unmarking a note re-emits `note_modified` (the
write goes through self_writes, which silences the watcher, so the direct emit is
the compensating channel), and an open editor immediately re-reads the new
frontmatter rather than swallowing the marking on its next save; switching the
marking is refused while the current note is dirty (save first).

Test baseline: unit tests in core's `privacy.rs` and `vault.rs` (glob semantics,
case, NFC, negation rules, frontmatter round-trips, watcher reloads); direct tool
tests in the agent layer plus a full-chain `MockTransport` sweep — the model
actively reads, searches, queries, follows links, writes and edits private paths,
and the assertion is zero leakage of a sentinel string, checked across history,
events, request bodies and the JSONL.

Known limitations (deliberate):

- Content inside a summary that has **already been generated** (the product of
  level-two compaction) cannot be traced and scrubbed — a summary is model-synthesised
  text with no envelope structure to strip (the request that generated it had itself
  passed the gate, so only content that was not yet private at summarisation time
  can be baked in). Content read in through tool results is the same (a `tool`
  message carries no reference metadata; level-one compaction or a vault switch
  clears it). `scrub_private_refs` covers only `@` reference bodies inside user
  envelopes.
- `privacy_mark` does not scan every session sidecar (that is O(sessions × size) of
  synchronous disk IO; a sidecar sits on the same disk in plaintext as the private
  note itself, and the channel worth defending is the request body) — other
  sessions are cleaned automatically by the top-of-turn scrub the moment they are
  loaded to continue.
- An older sidecar (no `context_refs` metadata) combined with a pathological
  filename containing `"`, `<`, `>` or a newline defeats envelope-attribute
  scanning, and the scrub misses it. Metadata covers this from new sessions onward.
- A session log's `tool_call` rows contain path arguments the model itself supplied
  — that is the model's own speech; the `privacy` row carries no path.
- Unmarking removes only the exact rule line; if a broader rule still covers the
  path it stays private (and the badge says so honestly).
- After a rename the frontmatter marking follows the file; a glob rule does not.
- Writing to a private path errors while writing to a public one opens a dialog,
  which is a detectable behavioural difference — the error tool result is a design
  requirement, and this is accepted. The `write_note` refusal for a note holding a
  private block is the same class: the model learns that something is hidden,
  never what. So is a gap in positional block ids, and the "selection no longer
  available" line of a re-gated selection.
- A private block in a never-saved note, selected from its middle: Rust cannot
  see the block on disk and the selection has no `[!private]` head line to parse,
  so the editor refuses that selection itself. Rust stays the real gate; the
  editor's refusal covers this one gap.
- `search_notes` line numbers stay those of the raw text (frontmatter included)
  and match what `read_note` shows only when no private block precedes the hit;
  the model edits by text, not by line, so this is accepted.
- The search index keeps the raw text of every note in memory; what the agent
  receives is re-matched over a blanked copy, but the index itself is unfiltered.
- The audit log's `write_prepared` diff is over the raw text: the user sees the
  private block in the permission dialog and the replay, the model never does.
- A `> [!private]` block's tags and links are absent from the graph, backlinks
  and tag counts for the user too — the price of not indexing them anywhere.
- A private block typed into a note between `edit_note`'s prepare and execute makes
  the execute refuse ("changed while the edit was awaiting review" — the same
  `private_blocks_preserved` check, against the note as it is then), unless the user
  rewrote the proposal in the dialog, which is their own view of the raw text; a
  change to the public text in that window is the general concurrent-edit hazard,
  unchanged.
- The UI does not filter: an embed of a private block (`![[Note#^id]]`) renders in
  the editor, and the status bar names the block when the cursor is inside it.
- `.vemry/skills/` is a deliberate exception the agent can read (the `read_skill`
  tool only, and only `.md` directly under that directory — see "Programmability");
  user scripts read and write the vault from the user's perspective, privacy zones
  included, because a script is trusted as the user is. The agent cannot trigger a
  script, so the two trust domains do not connect.

## A password lock for privacy zones (a design note; optional, unscheduled)

The ask: private files should need a password before their **content** can be
viewed. Note that this is orthogonal to the privacy isolation already implemented —
that one governs "invisible to the AI", a password lock governs "not casually
visible to people either". The two stack without interfering.

**A fork that has to be settled before writing any code**, because the two paths
cost entirely different things:

- **An interface lock**: the password is verified in Rust, and while locked private
  content never enters the WebView. On disk it is still plaintext Markdown — this
  defends against someone idly looking, a screen recording, a shared screen; not
  against someone who has the disk. External editors, sync and `git diff` all keep
  working, so it does not conflict with "local-first, plain files you own".
- **Encryption at rest** (XChaCha20-Poly1305 with Argon2id, say): the disk holds
  ciphertext, which is real confidentiality. The cost is that external editors,
  sync and `git diff` stop working for those files, forgetting the password loses
  the content permanently, and indexing and search can only be built after
  unlocking — a head-on conflict with the product's positioning. If it is ever
  built it should be an explicit per-vault or per-directory switch, never the
  default.

If the interface lock is chosen, enforcement should still converge in Rust, the
locked state should live in memory (`AppState`, never on disk), and the password
should be stored only as an Argon2id hash in `.vemry/config.json` (travelling with
the vault). The only channel by which content enters the WebView is IPC (the
capability grants `core:default` plus `dialog:default`, with no assetProtocol and
no `convertFileSrc`), so the list is closed — but it is **far more than the
editor**:

- Full text: `note_read`, `file_read`, `file_read_binary` (the preview's blob URL),
  `resolve_ref` (the raw slice a `[[…]]` target denotes)
- Fragments: `search`'s snippets, `query_blocks`'s block content (the frontend does
  not consume it today; the v0.2 projections will)
- **`links_get`'s backlink snippet is the least obvious one**: the fragment comes
  from the *source* note, so opening a public note displays, in the panel, the body
  of the private notes that link to it
- Metadata is user prose: `notes_list`'s titles and aliases (a filename like
  `divorce-agreement.md` is itself content), and **the editor's `[[` and `#`
  completions are built from `notes_list` rather than `link_targets`** — the
  candidate list is filtered for private notes in the frontend, but that is UI
  courtesy and not a security boundary: the data still enters the WebView intact,
  so as far as a password lock is concerned the exposure is unchanged
- Existence confirmation and bypasses: `resolve_link` and `resolve_ref` (confirm a
  private note exists, and are the entry points from clicking a wikilink to its
  full text), `block_ensure_id` (writes an id into any note),
  `privacy_list` (enumerates private paths directly), `session_read` (the session
  JSONL — the tool side is filtered, but text the user pasted into the composer
  themselves is logged verbatim), and `file_open_system` ("open with the system
  app" in the context menu, one click that bypasses every IPC read layer's lock)
- Locking must also lock `privacy_unmark`, or unmarking from the UI is an unlock

Two more things Rust cannot do on its own: refusing subsequent IPC does **not**
clear content already inside the WebView (the CodeMirror document, `FilePreview`'s
blob URL and the palette's search results all need explicit clearing), and the
"reload on external change" driven by `vault://event` will pour content straight
back into the editor, so it has to check the lock state too.

Implementation-wise, the suggestion is to add a read-only view in `vemry-core`
carrying a `hide_private` switch (always true for the agent, following the lock
state for the UI) rather than scattering checks through a dozen commands — which is
consistent with "one `query()` serves three consumers". Error semantics should stay
"it does not exist" rather than "it is locked".

Still undecided: unlock granularity (once per vault, an auto-lock timer, or every
open), what recovery looks like after a forgotten password, and whether different
directories may have different passwords.
## The built-in knowledge browser (v0.4, implemented)

The target scenario: browsing knowledge on the internet inside the app — no ads,
fast, clean — and turning a page into a vault note whenever it is worth keeping.
The browser is the intake, the vault is the settling tank. It is positioned as a
**knowledge browser**, not a general-purpose one: reading, retrieval and clipping;
no extension ecosystem, no sync account.

### Shape: standalone and centred by default, docking demoted to a remembered switch (batch D)

External pages open in a standalone `browser-N` window (`WebviewWindowBuilder` +
`WebviewUrl::External`, all stable API). **Zero IPC for remote content is
guaranteed at the configuration layer**: the capability matches the main webview
exactly with `"webviews": ["main"]` (a defensive tightening, so a future
multi-webview main window cannot inherit indiscriminately), and a browser window's
label matches no capability at all → `window.__TAURI_INTERNALS__` does not exist.
The navigation guard is belt and braces: `on_navigation` allows only http(s) and
rejects the app origin (localhost:1420, tauri://); `on_new_window` always denies and
opens in the same window (popup suppression).

Window creation (batch D inverted v0.5.2's docked-by-default): the first window
opens centred and nearly full height in the visible area of **the display the main
window is on** (`initial_browser_rect` is a pure function — 1080 wide to preserve
desktop layout breakpoints, height filling the visible area capped at 1200;
computed explicitly from `current_monitor().work_area()` rather than `.center()`,
whose choice of display is not controllable and would open the window on a
different screen when the main window is on an external monitor). A standalone
window's geometry is kept in memory on Moved/Resized and written to
`settings.browser_window` on Destroyed, so the next one opens in place (the same as
the chat window; multiple windows cascade by count so they do not stack exactly).
"Dock" is demoted to a switch: `browser_set_docked` writes
`settings.browser_docked` on every toggle (remembering the last choice), and the
docking machinery (dock_geometry clamping, DockLayout yielding, the reattach
backstop) is preserved as-is and used only by docked windows. ⌘L opening a
standalone window **no longer auto-expands the research panel** (its clip entry
points are on the window's own toolbar, so the main window's layout is not changed
out from under you; a docked window keeps the auto-expand, because its control
panel is in the main window).

Browser controls merged into the right sidebar's **research panel** (v0.5; the
former "Browse" tab was removed): when a browser window is active, a "current page"
card is pinned above the conversation (title, domain, back/forward, three clip
buttons, add-to-context, the dock switch, a ☆ bookmark, adblock status), kept in
sync through `browser://event` with the Rust-side `BrowserState` (a label →
url/title/docked registry). The research panel's input is dual-mode — a URL shape
navigates on ↵, anything else goes to the AI (⌥↵ forces it to be a question) — and
⌘L in the palette also opens a URL. More windows means more tabs; **within** a
window, back and forward are left entirely to the webview's own history and never
mix with the document navStack (on macOS the browser window enables
`allowsBackForwardNavigationGestures`, so a two-finger swipe goes back and forward —
with zero IPC there is no way to inject a back button, and the native gesture is the
only navigation the window owns).

**App-level history and bookmarks** (v0.8): `browser_history.json` and
`bookmarks.json` under `app_config_dir` (the same "shared across vaults, never
inside a vault" decision as settings, with an RwLock in-process cache), deliberately
not stuffed into `Settings` — `settings::load` is cloned per request on the
navigation and image-proxy hot paths, and a large list in there would be copied over
and over. History has exactly one writer, the window-creation hooks:
`on_navigation` records the visit and `on_document_title_changed` fills in the title
by label → registry url (url and title arrive on separate beats). The same URL
**merges and moves to the top globally** (visits accumulate, capped at 500), so
"the last page" is simply the first row and the ⌘L list needs no second
deduplication — at the cost of exact ordering, which is fine because this is "places
you have been", not a forensic log. The hidden reader window has none of these
hooks, so the reading projection stays out of history by construction. Bookmark
changes (`bookmark_toggle`, `bookmark_remove`) broadcast a full `BookmarksChanged`
snapshot through `browser://event`, keeping the ★ mirrors in the main window's page
card and the clip toolbar in sync (single writer in Rust, the same "ride along on
something that is not window state" pattern as `AdblockStatus`). The UI converges on
⌘L: an empty query offers "return to the browser window / reopen the last page" plus
bookmarks and recents; with a query it fuzzy-filters bookmarks and history across
both title and url (the direct row preserves the "what I type is a URL" model), and a
bookmark row can be removed inline. **Direct invocation** (the menu's Research →
Open browser, ⌘P, and an empty ⌘L Enter) all share one Rust implementation,
`browser_open_last`: focus an existing window, or reopen the most recent page from
history if there is none — neither of which the frontend has to prompt for.

Window relationships (v0.5.2's "pseudo-merged docking", successor to v0.5's magnetic
docking; since batch D docking is an **optional** shape and standalone is the
default): a docked browser window has **no title bar** (on macOS,
`TitleBarStyle::Overlay` plus hiddenTitle, with the traffic lights floating over the
top-left of the page and only the red close button kept — a visible exit for a new
user at all times, and ⌘W also works; `setMovable(false)` prevents dragging it away
by the invisible title bar. Other platforms use `decorations(false)`). On macOS it is
attached as a **native child window** of the main window through
`NSWindow addChildWindow:` — dragging, stacking order and minimising are grouped by
AppKit (with none of the lag of replaying events), it does not appear separately in
the Window menu, and it reads visually as a panel the main window extended.
Navigation, detach and close UI all live in the main window's page card, because the
browser window has zero IPC and cannot grow a button of its own. The main window's
`Moved`/`Resized` → `redock_all` is still there: on a move it is usually a
same-value no-op, and its real job is syncing height and clamping to the screen edge
on a resize. Minimising on macOS breaks the child relationship, and
`reattach_docked` idempotently reattaches when the main window regains focus. When
the main window is **full-screen or flush against the right edge of the screen**
there is no room beside it and the browser window can only overlap the main window's
right edge (a native child window always covers DOM) — `dock_geometry` computes the
covered width (an inset, in logical px) and pushes a `DockLayout` event when it
changes; the App layout yields with a matching `padding-right`, so it reads as
"notes on the left, web on the right", with the research panel always visible.
"Detach" means removing the child relationship and switching back to a **regular
title bar** (batch D: a visible drag handle plus the page title,
`set_title_bar_style(Visible)` + `setTitleVisibility` — in the Overlay shape the page
runs up to the top of the window and WKWebView eats the drag, so the window simply
cannot be moved, and a standalone window must have a real title bar), free to be
placed anywhere; "dock" reverses it (a docked window cannot be dragged away, which is
why the old `expected_pos` drag detection was deleted). The window title follows the
page title so it can be told apart in Mission Control; a new window is hidden first,
positioned, attached as a child, and only then shown (no jump). Focus etiquette:
opening from a citation passes `focus=false` so keyboard focus is not stolen, and
closing the last browser window returns focus to the main window. Same-window
splitting (web on one side, notes on the other) still waits on Tauri's multi-webview
support stabilising; when it lands, the page card and the reading projection can move
across wholesale.

### The clip toolbar (batch D, implemented)

Clipping entry points moved into the browser window itself: a 40 logical-px **clip
toolbar** sits inside the bottom of each `browser-N` — a separate `clipbar-N` window
(`src-tauri/src/browser/clipbar.rs` + `src/ClipBar.svelte`, the same "our own bundle,
authorised by label" as the chat window; its capability is the wildcard exception
noted under Security boundaries). The browser window has zero IPC, so UI cannot grow
inside it; the toolbar overlays the bottom rather than the top, because the top has
the floating traffic lights (an Above-level child window would cover the close
button) and nearly every site has a sticky header — the top belongs to the page, the
bottom belongs to vemry (the positional language of a browser status bar). It is a
transparent window (macos-private-api) whose CSS bottom corners match the window's
own radius; on macOS it is attached as a native child of the **browser** window (in
docked mode that makes it a child of a child, with AppKit propagating moves; it
reattaches idempotently when the browser window is Focused), and elsewhere
`builder.parent` preserves z-order with the position following the browser window's
Moved/Resized manually. Its lifecycle follows the browser window entirely (Destroyed
closes it too) and it does not enter the `BrowserState` registry (the same as the
reader window — the registry means "pages").

On the bar: a URL input (`browser_navigate` opens the new page in place, URL-only,
the same semantics as ⌘L); an AI button (the dedicated `summon-agent` action: the
main window **opens the standalone chat window first** for immediate feedback, then
extracts the current page through `browser_page_context` by the paired window's
label and attaches it as a page chip through `agent_window_to_agent`'s `attach-page`
command — once the window is open the handoff package is out of reach, so everything
goes down this one path; attachPage replaces a single page, so invoking it again
just refreshes; an extraction failure is reported into the chat window. It
deliberately does not go through `focus-agent`, whose panel branch would pull the
person back to the main window); selection → the pinned target; a ▾ two-stage
landing picker in simplified form (`clipbar_set_expanded` grows the bar upward,
anchored at the bottom; the data path shares its source with CommandPalette's
pick-note — fuzzyFilter plus `query({kind:"heading"})` — and the component is not
reused because it is tightly coupled to App's pending state); page → clips; link →
note; a clipped badge; detach/dock; and a flashed receipt. **Every action is relayed
to the main window for execution** (`clipbar_to_main` → `clipbar://action` → App's
clip*Flow; delivery must go through the main window's editor buffer, and the landing
record is kept there too), with the receipt going back through a targeted `emitTo`
on `clipbar://result` to flash on the bar (it cannot see the main window's toast).
Only the "already clipped → open the note" relay brings the main window forward.
**The focus-return contract**: after any interaction, `browser_focus` returns focus
to the paired browser window — space-bar paging and keyboard scrolling never break,
and reading is not interrupted. The pinned target (clipTarget) syncs across windows:
**the main window is the only writer** and broadcasts `clip-target://target`
globally from its persistence effect, while non-main windows (chat, toolbar) are
read-only mirrors — so both labels agree in real time. The main window's research
panel keeps its page card and three clip buttons (two coexisting entry points with
identical command semantics).

**Returning to the landing note on close** (closing the loop): during a browsing
session (from the first browser window opening to the last one closing) the note
that the most recent clip landed in is recorded in the main window
(`lastClipLanding`, recorded only in clip*Flow and deliverPicked; the reading
projection, an agent pin and quick capture do not count). When the last window
closes, Rust has already returned focus to the main window and the frontend
navigates with `openNote`, then — if there is a section — scrolls to it with
`revealPos` using the frontend's `extractOutline` (a UTF-16 measure; `query_blocks`
spans are byte offsets and cannot be fed to CM directly). The close gesture: the
menu's Window → Close Window ⌘W is special-cased in Rust (menu.rs) — a focused
browser window closes, a focused toolbar closes its paired browser window, the chat
window goes through the existing CloseRequested handshake, and the main window does
nothing. On non-macOS a standalone window has a native close button, and a docked
window uses the page card's ✕.

### Ad blocking (macOS, native WebKit)

adblock-rust (the `content-blocking` feature, avoiding the heavy css-validation
dependency) compiles the subscribed filter lists into WKContentRuleList JSON — no JS
injection, no local proxy. The pipeline: a background thread parses the rules at
startup (pure Rust, CPU-heavy) → the main thread's `WKContentRuleListStore` checks
the disk cache first (**a content-addressed identifier** = the FNV hash of the
produced JSON, so a rule change rolls the generation automatically and old ones are
cleaned up; a second launch is instant) → only a miss actually compiles. The rule
list (`Retained<WKContentRuleList>`, !Send) lives in a main-thread thread_local, and
the Send-safe state mirror travels through `AdblockState` plus events. The rules are
attached to the `WKWebViewConfiguration` **before the window is built** (window
construction runs entirely on the main thread), so the first frame is already
filtered; if a window predates compilation finishing, the rules are attached
afterwards to the native WKWebView through `with_webview`. Truncated at a 50k rule
cap, with `ignore_previous_fp_documents` always last. **No filter list is bundled**:
third-party lists are copyleft (EasyList is GPLv3+/CC BY-SA 3.0+, EasyList China
states no licence at all), so shipping one inside the binary would be redistributing
it — and a frozen snapshot rots anyway (EasyList's own header says `Expires: 4
days`). The rules live in `app_config_dir/adblock/*.txt` and get there two ways, both
from a user action: `ensure_rules_soon` downloads the subscriptions when the first
browser or reader page of the run opens with nothing cached (**once per run** — a
firewall that cannot reach the hosts must not cost a timeout per page), and "update
now" in the settings. Sources are fetched **independently**: an unreachable host
costs its own list alone and leaves the previous copy in place, `Err` is reserved for
"not one source came through", and the failures are reported back so a partial update
cannot read as a clean one. Identical rule lines across sources are dropped before
compiling, so the same list from a mirror does not eat the 50k budget twice.
`adblock_import_rules` takes files the user downloaded by hand — the escape hatch for
a network that cannot reach the subscription hosts, and the one path that touches no
network at all. Non-macOS returns an Unsupported no-op (a CI backstop) and skips the
download entirely: nothing there would consume the rules.

### Clipping into notes, at three granularities

- **Selected text** → a quote block (`> quotation` plus `— [title](url) · date`),
  inserted at the cursor in the current note or saved into `clips/`; it can also be
  clipped **into a chosen section of a chosen note** (▾ or ⌥ opens the pick-note
  panel to choose a target, or create one; Enter picks straight through for a
  friction-free "end of note", and Tab or typing `#` enters the second stage's
  heading list — the same mental model as `[[Note#Heading]]`, and that heading list
  is literally a projection of `query({kind:"heading"})`. Choosing once **pins** it:
  the button's label becomes `note › section`, later single clicks go straight
  there, and ✕ unpins. The pinned target persists per vault in localStorage, so
  switching vaults restores each one's own). The landing rules: a named section →
  the **end of that section** (a section runs from its heading to the next heading
  at a level ≤ its own; a section takes priority over the cursor; a missing heading
  falls back to the end of the file plus a toast, never silently); no section → the
  cursor (only when the target is the note already open) or the end of the file. The
  write primitive is `note_insert` (vemry-core's `insert.rs`: the position is found
  by reparsing the content read in that same call, and separators are normalised —
  blank lines around a block payload, a single-line list item hugging the list, and
  exactly one blank line before what follows). When the target is the note currently
  open it must go through the editor buffer (`insertAtSection`/`insertAtCursor`;
  `src/lib/insert.ts` and insert.rs share one table of rule cases) — a disk write
  would be overwritten by the stale buffer's autosave. Quick capture shares this
  entire landing mechanism with clipping. A selection is taken not as plain text but
  as a Range-cloned **DOM fragment** (sharing the normalisation JS with full-page
  clipping: lazy-loaded images are repaired by pairing the src attribute against the
  live DOM, formulas are wrapped in sentinels, and relative addresses are completed
  in place through attribute getters) → `fragment_to_markdown` converts it into the
  quote block, so images, formulas and links survive; if nothing converts, it falls
  back to plain text. A selection in the reading projection works the same way (the
  frontend restores proxied images and placeholder chips by `data-url`, KaTeX output
  falls back to text, and `reading_quote` sends it down the same htmd pipeline).
  After landing, the frontend calls `clip_localize_note(rel, url)` to localise
  images (only the frontend knows where it landed; on the buffer path it calls
  flushEditor first, because the backend rewrites literally against disk content).
- **A whole page** (⌘⇧S) → Readability extracts the article → html→markdown →
  `clips/<title>.md` (flat, not foldered by domain — retrieval is by topic, not by
  source site), with `source:`, `domain:` and `clipped:` plus `vemry: origin: clip`
  in the frontmatter; "browse by site" is left to a `query()` projection. After it
  lands, **images are localised asynchronously** (v0.5.2, setting
  `clip_localize_images`, on by default): article images are downloaded through the
  same `assets.rs::fetch_image` pipeline (three layers of SSRF protection, no
  cookies, the page's Referer, a disk cache) into
  `clips/assets/<content hash>.<ext>`, and the note is rewritten to vault-root
  relative references — so reading is fully offline afterwards, and neither hotlink
  protection nor a dead image host loses a picture. The extension is sniffed from
  magic bytes (a whitelist of raster formats plus SVG; the URL is not trusted), and
  content-hash naming deduplicates by construction. The budget is 10 MB per image
  and 30 images / 50 MB per clip; a failure or an over-budget image keeps its remote
  URL and the frontmatter is marked `images: partial`. The rewrite re-reads the note
  after the downloads finish, replaces literally, and skips code blocks (a wiki's
  teaching example must not be touched), so an edit the user made in the meantime is
  not clobbered. The pure logic (scanning, rewriting, sniffing, marking) is in
  vemry-clip; the orchestration is in src-tauri.
- **A link** → a `[title](url)` reference inserted into the current note (into inbox
  when no note editor is open).

Fetching uses `eval_with_callback` (stable API is enough to get a JS return value)
plus a oneshot bridge with a 10 s timeout; the pure logic lives in
`crates/vemry-clip` with dense unit tests, including a regression that a `#fragment`
does not produce a spurious tag. Landing reuses `Vault::write_note` (with
`dedup_rel` for name collisions) plus `commands::broadcast` — **a clip is indexed
the moment it lands**, participating in search, the link graph and `query()` exactly
as a hand-written note does, and workflows under `clips/` fire for free (the v0.3
event bus). "Insert into the current note" goes through the `insertAtCursor` hook
the Editor registers (CodeMirror doc → onDirty → autosave, consistent with the
flushEditor contract; never writing to disk behind the editor's back). `clips/` is
visible to the agent by default (deliberately: it is material to feed it); making it
private is one line in the rules.

The security boundary: a remote page never gets IPC, and the clipping JS is a fixed
extraction script on the app side, pulled one way by a user command — Rust asks the
browser webview for the selection or DOM, and page scripts cannot write to the vault.

### Citations into the vault (block citations)

The model cites the block a fact came from, the way it cites a web source:
`[[path#^id]]` when the block already carries an id, otherwise `[[path#bN]]`
with the positional handle exactly as `query_blocks` returned it (an explicit
id is returned with its caret so the two are told apart without a second
field; the prompt rules ask for one citation per fact and forbid invented
ids). The panel renders a fragment link as a chip (`wikiAnchor` in
`markdown.ts`: a ¶ or › glyph and the note's name); hovering 250 ms shows a
text-only preview card (`HoverCard.svelte`; a positional citation is looked
up in the note's block list, a stable one through `resolve_ref`), and a click
opens the note on the block with a 1.2 s flash (`editor/flash.ts`). Positional
ids are transient — never written to a note — so every path out of the panel
(copy, insert, pin, save as note) goes through one funnel, `exportAnswer`,
which materializes them: `block_ensure_id` mints ` ^id` into the cited note
and the link becomes `[[path#^id]]`, or degrades to `[[path]]` when the block
is gone. Because a block may move between the answer and the click, the panel
captures each positional citation's content when the turn ends and passes it
as a guard: a mismatch falls back to the one block that still has that text.
The guard lives in `src/lib/citation.ts` next to the shared block cache.

The other direction — the user handing one block to the model — is the
composer's `@Note#` stage: typing `#` after a note name lists the whole note,
its headings and its anchorable blocks, and a pick becomes
`ContextRef::Block { path, local_id }`. Rust resolves it at send time through
`Vault::block_for_agent` (a heading hands over its section, a list item its
subtree, private blocks removed from the slice, a private block nonexistent;
the path goes through the same privacy gate as every attachment, worded like
a missing note), and the envelope is `<block-context path="…" block="^id">`
whose intro tells the model how to cite it. The audit records the handle, not
the text. A selection attachment that lies inside one block carries the same
handle (`<selection-context … lines="8-8" block="b1">`, the intro naming the
citation), so the block menu's "ask the AI about this block" — which arrives as
a selection — yields a citable answer; a passage spanning blocks, or one in a
private block, carries none, and the rules still forbid citing such a
selection. This is the seam an in-editor agent will use.

### AI web search (the DeepSeek Responses API)

Search uses the **provider's server-side** `web_search` built-in tool (the DeepSeek
Responses API, which as it happens supports only deepseek-v4-flash) — no new search
vendor inside the app, no second key, and search stays inside the existing LLM
channel. The implementation is `ResponsesTransport` (the second implementation of
the `ChatTransport` trait; see the vemry-agent table). It is enabled when
`web_search_enabled && base_url == api.deepseek.com`, off by default, and the
default path is unchanged.

Live findings (2026-08-11): the `/responses` path is correct; function tools need a
**flat** shape (a nested `{"function":{…}}` returns 400); the stream returns **no
structured citations** — `annotations` is always empty, and a `web_search_call` item
carries only `action.queries` (the actual search terms). A follow-up measurement
(2026-08-19, `live_responses_web_search_params_probe`): the OpenAI-style tool
parameter `filters.allowed_domains` is **accepted and then completely ignored** —
with no domain mentioned in the prompt, the search terms contain zero `site:`
operators and the source domains are unconstrained. So the Raw tool body stays
parameterless: the provider exposes no parameter surface, and adding parameters
would only fake a control that does not exist. Visibility is therefore designed as:
search terms are shown on the timeline through `AgentEvent::WebSearchQueries` and
recorded in the session log (terms and links only, never full text); sources are
harvested from the links in the answer body (Markdown or bare URLs) into source
cards, and clicking one lands in the main window's reading projection by default
(v0.5 P2; ⌘click opens the browser window) — completing the search → source → read →
clip → settle loop. Search does not touch disk, does not leave the LLM channel, and
does not pass through PermissionGate (the principle that the permission boundary is
the write path is unchanged).

### The research loop (v0.5: citations you can follow, context the user hands in)

v0.5 joined "search → citation → read → clip → feed back" into an unbroken loop.
Three designs matter:

- **Citation superscripts share numbers with the sources**: the frontend's
  `lib/markdown.ts` (an own-subset Markdown renderer that escapes all text first and
  allows only http(s) and `[[wikilink]]`, so the XSS surface is self-owned) folds
  `[title](url)` and bare URLs in the assistant's body into inline superscripts,
  numbered by `extractCitations`'s document order; the source harvesting on
  `turn_finished` uses **the same extraction**, so the two are always numbered
  alike. Every link click goes through the single `url_open` funnel (`open_links_in`
  has three states: reading projection — the P2 default — the built-in browser
  window, or the system browser; ⌘click's inversion maps Reading and System →
  Internal and Internal → System, so ⌘click always gives you a real browser window,
  and the editor and the AI panel are no longer two separate paths).
- **Citations landing**: `clips_lookup` reverse-looks-up whether a URL has been
  clipped by the `source:` frontmatter of notes in clips/ — a citation that has been
  clipped opens the local note directly (⌘click still opens the web page), and a
  source card clips a reference entry with one `clip_url`. External citations settle
  into in-vault links as they are used.
- **Page context is handed in by the user; the agent gets no fetch tool**: the page
  card's "add to context" extracts the article through `browser_page_context`
  (reusing the GRAB_PAGE → `extract_page` pipeline, truncated at 20 000 characters)
  and attaches it as a chip on the next message. At the LLM boundary the runner
  wraps it in `<web-page-context source=…>` and states that web content is untrusted
  and its instructions are not to be executed before splicing it into the model
  message (`vemry-agent/prompt.rs::compose_user_message` is the single place the
  envelope format is assembled). The audit jsonl's user row records only a
  structured `page: {url, title}` — the session title and the frontend chip read
  those fields rather than parsing the envelope string, and the full envelope exists
  only in the history sent to the model (and its sidecar). The agent's autonomous
  network capability remains zero: the only ways online are DeepSeek's server-side
  web_search and the browser in the user's own hands.

### The reading projection (v0.5 P2, implemented)

Clicking a citation lands by default in the **extracted view in the main window's
centre pane**, and the visible browser window is demoted to an **escape hatch for
interaction** (a paywall, a login, comments — one click across, or ⌘click). The
mechanism: a hidden `reader` window loads the page — its label matches no capability
(zero IPC), it uses the same navigation guards, and it gets the same adblock rules
attached at creation **plus reader-specific image, media and font blocking** (article
extraction does not need heavy assets, so onload arrives far earlier). It does
**not** enter the `BrowserState` registry (the page card, the clip commands and
magnetic docking cannot see it). Rust reuses `vemry_clip::extract_page` to extract
the article, and the frontend's `renderReadingMarkdown` (the own renderer's reading
mode: pipe tables, and relative links resolved against the page's actual URL while
still allowing only http(s)) renders it into `mainView = "reading"`.

- **Fetch timing**: `url_open`'s Reading arm emits `reading://event` → the frontend
  calls `page_read` → and **does not wait for onload**: it polls readyState after
  navigating (150 ms → 400 ms) and returns as soon as extraction quality is good
  enough (not a fallback, and the article over a threshold). Detecting a document
  change uses two criteria, an old-document marker and the URL — an eval that lands
  before the navigation commits still sees the old document, and readyState alone
  would return the old page as the new one. If it is still thin or a fallback after
  readyState reaches complete, it waits 1 s and grabs again, keeping the better
  result (SPA settle), with 15 s as the hard backstop. A request generation counter
  prevents rapid clicks from crossing numbers: an old request exits with a
  `superseded` sentinel and the frontend ignores it silently.
- **Projected images**: the renderer emits `![..](url)` as
  `<img src="vemry-asset://…">`, fetched by the custom protocol in
  `browser/assets.rs` — **no cookies, the page's URL as Referer (to get past hotlink
  protection), and a disk LRU cache**. Image URLs come from web content and are not
  trusted, so Rust validates against SSRF (http(s) only; every resolved IP must be
  public; the resolution is pinned into the request against DNS rebinding; each
  redirect hop is rechecked). The main webview therefore never connects to a remote
  host directly — the exit converges into this one Rust channel, and only after the
  user explicitly opened the projection (the reader window has just genuinely loaded
  the same page, so the privacy delta is minimal). A failed load falls back to a
  placeholder chip, and `reading_images_enabled` can turn it off. **A clipped note on
  disk does not go through the proxy**: opening an old note must stay at zero network,
  or a tracking pixel would leak reading behaviour. Lazy loading takes two moves:
  GRAB_PAGE first does a fixLazyImages-style attribute restore (data-src and
  friends); the deeper kind, where the real URL lives only in JS state and an
  IntersectionObserver callback writes it back to src, is handled by an
  **IntersectionObserver shim** injected at document-start in the reader window — a
  hidden window does not render, so IO never fires, and the shim makes a lazy-load
  library see "intersecting on registration" and swap immediately (injected only in
  the reader window; visible browser windows are unaffected).
- **Formulas**: a three-stage relay. `GRAB_PAGE` works on a cloned copy of the DOM
  and restores KaTeX (annotation), MediaWiki (alttext) and MathJax (script source)
  render output back to LaTeX source, wrapped in an `<x-vemry-math>` sentinel
  element (inlining `$…$` text directly would be mangled by htmd's escaping);
  vemry-clip's custom htmd handler emits `$…$` / `$$…$$` (so a clipped note carries
  standard `$` syntax that any maths renderer understands); and the frontend renders
  with KaTeX locally (bundled into the app, zero remote requests; `trust:false`,
  called only on fragments identified as formulas — the own renderer's single
  third-party HTML injection point).
- **Code blocks**: the same sentinel relay (`<x-vemry-code lang>` → vemry-clip emits
  a fenced block, with the fence length automatically exceeding any backtick run in
  the content). The motivation: a code viewer's rendered DOM (a lake card, a
  CodeMirror instance) has **no newline text nodes between lines**, so serialising
  outerHTML splices the code onto one line, and a shell comment landing at the start
  turns into a spurious heading. `fixCode` works in two steps: a lake card's original
  code, newlines included, is hidden in URL-encoded JSON in a `value` attribute and
  is decoded directly (the most faithful route, immune to render timing and to
  virtualisation of long blocks); everything else is identified on the **live DOM**
  by computed style (a monospace font plus pre-family white-space or a code class
  name — two gates, so body text laid out with `pre-wrap` is not mistaken for code),
  with `innerText` restoring newlines by CSS render boxes and the result written back
  into the clone paired by document order (the same as fixImgs). Anything not
  recognised keeps htmd's default path — never worse than not normalising at all. On
  the selection path the clone is a detached node with no computed style, so only the
  two degraded routes apply: card decoding and direct `pre` conversion.
- **Straight back into the loop**: `reading_clip` takes the Rust-side projection
  cache directly (an LRU of 32; reopening the same URL hits a non-fallback cache and
  `page_read` returns instantly) and lands it in `clips/` — no refetch, byte-identical
  to what was on screen. A selection → quote block goes through `reading_quote` plus
  `insertAtCursor` (the selection is inside the main webview, so none of the browser
  window's "preserve the selection across a blur" gymnastics is needed). "Add to
  context" assembles a `PageContext` purely in the frontend and is handed over
  through the same `agent_start` channel the page card uses.
- **History of its own**: the projection's history (20 pages in the frontend,
  including scroll position) does not mix with the document navStack, following
  v0.4's position that browsing history and document history stay separate. But the
  *projection itself* takes one slot in navStack (the `READING_ENTRY` sentinel), and
  entering it also records where it came from (`vault.enterReading` /
  `vault.exitReading`). Going back is a three-stage chain (the header's ← behaves
  like ⌘⌥←): the projection's own page stack → document history (which can pass back
  and forth through projection entries, so ← from a clip note lands back in the
  projection) → the originating view as a backstop, and ← is never greyed out. ✕ and
  Escape are a mode exit: straight back to where you came from, without moving
  history (when the index is sitting on the projection entry it aligns to the
  neighbour of the current document). Escape with focus in an input returns focus
  first and only leaves on a second press, so nobody is trapped in the projection.
- **Privacy**: projected content is displayed in the frontend only — never written to
  disk, never indexed, never in a prompt. The agent still has no tool that fetches a
  web page, and anything settling into the vault goes through an explicit clip or
  add-to-context.
- **Known compromises**: projection quality is poor on paywalls and heavy JS
  application pages (a fallback banner plus a permanently available escape hatch;
  the projection is not meant to be universal); the hidden window keeps the last page
  it loaded; an explicit "open URL" (⌘L, or a URL shape in the input) still opens a
  visible browser window directly, because that is the intent to *browse* and should
  not be hijacked; and existing settings upgrade unchanged (serde applies a new
  default only to a missing field, with a compatibility test).

### The network-exit list (after v0.4)

"Offline first" is stated not as "the only network call is the LLM API" but as five
**controlled exits**, with everything else fully usable offline:

1. The LLM channel (chat completions / responses, server-side search included) —
   vemry-agent, the only routine exit.
2. The browser window itself (remote web content) — opened explicitly by the user
   and physically isolated from IPC. The reading projection's hidden `reader` window
   is the **headless form of the same exit**, also triggered by a user click. The
   associated DNS prefetch (batch D, `browser/window.rs::prefetch_dns`) counts as a
   prelude to this exit: when ⌘L input pauses or a candidate row is highlighted, and
   on `browser_open`, the target hostname is handed to the system resolver
   (getaddrinfo; no HTTP is sent) — only fully qualified domains containing a dot,
   deduplicated per host for 30 s, so the exposure is "which domain is about to be
   or is being visited", without path or content.
3. Ad-block rule downloads — no filter list is bundled (they are copyleft; see "Ad
   blocking" above), so the subscriptions are fetched when the first browser or
   reader page of the run opens with nothing cached (once per run, riding on an exit
   the user just took at 2), and whenever the user clicks "update now" in settings.
   Importing rule files by hand skips this exit altogether.
4. The projection image proxy (`vemry-asset://`, browser/assets.rs) — only for images
   inside a projected page, without cookies, SSRF-validated, disk-cached, and
   switchable off in settings. Rendering a note on disk does not go through it
   (opening a note is zero network). Clip image localisation reuses the same fetch
   pipeline and downloads once, at the moment the user clips (a trust level equal to
   looking at that page), after which the note references local files.
5. App updates (tauri-plugin-updater, v0.6) — by default the check happens only when
   the user clicks "check for updates"; turning on "check at startup" in settings adds
   one silent query to GitHub Releases' `latest.json` at launch. Downloading and
   installing always need a click, and the package's minisign signature is verified
   (the public key is compiled into the app and the private key exists only in the
   publisher's CI secrets; the version's single source of truth is the workspace
   Cargo.toml, and the release process is in docs/RELEASING.md).
## The rest of the app shell (implemented)

"The app shell" covered the activity bar, the menu bar and context menus. These came
later, and each carries one non-obvious constraint.

### Tabs

**The one-live-buffer invariant**: tabs are a list of paths and content does not
live alongside them — only the active tab's content is loaded in the editor.
`mainView ⟺ the active tab` are aligned both ways: a projection is a tab too (its
path is a `VIEW_TAB_PREFIX` sentinel with no buffer), so "close the current view"
and "close the current tab" are the same act, and `closeActiveView`'s three entry
points (Escape, ✕, ⌘W) all end at `closeTab`. The reading projection is the one
overlay state that is not a tab, and while it is up the tab strip greys out rather
than disappearing.

**Preview semantics**: a tab opened with a single click is the preview slot
(italic), and the next single click reuses that same slot; double-clicking the title
promotes it. This keeps "clicking through a few things" from accumulating a dozen
tabs.

`tabs` persist per vault. On a vault switch, dead ones are dropped against the new
listing (orphans from an external delete or rename), and anything the listing does
not know about is read from disk once to confirm before being pruned — otherwise a
tab that was alive all session would vanish on the next launch.

### The status bar

Rendered in the main window only (not on the welcome page, not in the standalone
chat window). The content is deliberately restrained: on the left, the vault name
plus the privacy badge; on the right, the cursor's line and column, selection
statistics and the document's word count. All of it comes from pushes the vault
store already makes — the Editor is the sole producer and the status bar only
projects, never computing anything itself. Its height is `--statusbar-h`, and every
layout that needs it references that one variable.

### The outline panel

One of the right column's two tabs (Outline / Research). The heading outline follows
the document live, clicking jumps, and the section holding the cursor is
highlighted; backlinks and outgoing links are collapsed by default. Jumping goes
through `revealPos` — the first editor-jump primitive — which in a projection state
needs a switch back to the editor and a `tick()` before the coordinates can be
measured.

### The board

The second projection over the same task rules as Agenda (`views/tasks.ts`):
grouping reads the same `whenOf`, so the two views necessarily give the same answer
about the same task. A column id is a stable key (a drop target, the landing logic)
and a column name is copy (`kanban.col.<id>`). "Overdue" is not a legal drop target
— dragging a task into being overdue has no meaning.

### The plan confirmation gate (v0.9)

The `propose_plan` tool plus a blocking gate: a multi-step task presents a plan card
first and waits for "run it / edit / reject", then continues in the same turn once
approved. Like `web_search` it is a **session-frozen addition to the tool table** —
a change to the switch takes effect from the next session, because the tool table
freezes along with the system prompt when a session begins. The plan card is
`propose_plan`'s only projection: the recited list in the bubble's body is stripped
by `stripPlan`, or the same checklist would appear in three places in a single turn.

### The standalone AI chat window

The second webview ⌘J opens, running the conversation surface from the same bundle.
**The truth for state stays in Rust's session file** and both windows are
projections — which is why closing either one loses no context. Creation, handoff,
the close handshake and the relay live in `src-tauri/src/agent_window.rs` +
`src/AgentWindow.svelte` + `src/lib/host.ts`. A session has exactly one visible
composer, and switching sessions happens only on that side.

### Attachments bucketed by a note's full path

`attachments/<note name>` makes notes with the same name in different directories
share one bucket. The new convention is `attachments/<the note's full path>`.
Migrating an existing vault is an **explicit command** in the palette, with a
dry-run confirmation; it never runs automatically on upgrade, and same-name mixed
buckets and ownerless orphans are left in place rather than guessed at.

## Internationalisation (implemented)

Interface language is a cross-cutting concern belonging to none of the three
layers, which is why it has its own section.

**One pair of JSON files is the single source**: `locales/en.json` and
`locales/zh-CN.json`, generated by `scripts/gen-locales.py` from one
`key -> (en, zh)` table — a new message physically cannot enter one catalog and miss
the other. Keys are flat and dotted (`cmd.openNote`), because a nested object would
let TypeScript's completeness check see only the top level.

Three consumers:

| Consumer | Entry point | What a missing key costs |
|---|---|---|
| The frontend | `t()` / `tn()` in `src/lib/i18n/i18n.svelte.ts` | a `pnpm check` error (the two-way `Record<MessageKey, string>` constraint) |
| Rust (native menu, IPC errors, window titles) | `t()` / `tf()` in `src-tauri/src/i18n.rs` | a `cargo test -p vemry-app i18n` failure (the list of keys Rust uses is asserted one by one) |
| The two READMEs | by hand, against `docs/GLOSSARY.md` | nothing automatic — the glossary exists for exactly this |

Two non-obvious constraints:

- **Placeholder names must match across languages.** The caller passes one set of
  `{name}` values, so when the two disagree the other language renders a literal
  `{foo}` — key-set parity cannot see it and neither can `pnpm check`. The generator
  refuses to emit a mismatched pair, and a test in `i18n.rs` asserts it again in CI.
- **`t()` reads a `$state`**, so every call site in a template re-renders when the
  language changes: no restart, no re-registration. The cost is that any string
  array computed at module evaluation (the welcome examples, the composer's
  placeholder carousel, the composer mode table) must be `$derived`, or it freezes
  in whatever language was active at first mount.

**Cross-window propagation**: the same bundle runs in three webviews — the main
window, the standalone chat window and the clip toolbar — each holding its own
language state. `settings_set` broadcasts `settings://language` and each window
follows with `setLang()`; the native menu is replaced wholesale by calling
`install()` again on the Rust side (`set_menu` is a full replacement). CodeMirror
widgets that have already been built (query blocks, run blocks, live-preview
tooltips) keep the old language until the document re-renders — they read `t()` at
widget construction time, and this lag is known and accepted.

**Who deliberately does not read the catalog**: copy sent to the model (prompt
envelopes, attachment placeholder lines, tool errors) is English unconditionally —
the system prompt is itself English and already instructs the model to answer in the
user's language, so the protocol layer should not switch with it. Errors from the
`vemry.*` script API are English too: that is a developer interface, of the same
nature as any JS runtime's messages.

**VEMRY.md's section name is stateful**: the section `memory_append` appends into
changes with the language, but the lookup tries every name vemry has ever written
(`vault.rs::MEMORY_SECTIONS`) and only creation uses the current language's — so a
vault created under a Chinese interface and then used in English keeps appending to
the section it already has, rather than opening a second one or falling to the end
of the file.

**Byte-identical privacy wording is a security constraint, not a copy question**:
the "does not exist" returned for a private file and the one returned for a genuinely
missing file must be exactly the same, or the model can tell "hidden from me" from
"never there". Both come from one constructor
(`vemry_core::reject(PathReject::NotFound, …)`) rather than two literals kept in
step by hand.
