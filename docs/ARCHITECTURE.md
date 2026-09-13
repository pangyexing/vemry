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
| `history.rs` | The text of a note before each write, kept on disk under `.vemry/history/<note path>/` so it outlives the session that wrote over it (undo.rs is in memory). One file per write, named by the moment and by who wrote next (`20260914-081522-417-agent.md`: editor / agent / script / clip / restore / app / external); every write that is not the user's own typing is kept exactly, the editor's autosaves are coalesced to one in five minutes — unless the text they land over was written by someone else, the stale-buffer case, which is always kept. Sixty per note, thirty days, pruned at open; a move takes them along; a restore writes through the vault, so the text it replaces is kept first and a restore is itself undoable |
| `index.rs` | `BlockIndex`: note table, block table, link graph (backlinks derived), name resolution table (stem/alias, shortest path wins), tag table. Incremental update: reparse one file, then reparse links wholesale (O(total links), milliseconds at ten-thousand-note scale) |
| `query.rs` | `BlockQuery` → `Vec<BlockHit>`: the one query entry point. Inline-field conditions (`FieldFilter`, parsed from the flat `key = v` grammar shared by query blocks, the agent tool and scripts) consult a block's own fields first, then the note's frontmatter scalars |
| `search.rs` | The `SearchProvider` trait plus an in-memory multi-term AND substring match (no tokenisation, so CJK works by construction). tantivy or semantic retrieval would be another implementation of the same trait |
| `segment.rs` | The translation units of a document: the parser's blocks sorted into what the model translates (prose, headings, list items, quotes, tables) and what is copied through (code, display maths, images, rules, private blocks), each with its structural marker (`## `, `- [ ] `) split off into a `prefix` so the translation keeps the heading level and the list shape by construction; and `segment_pages`, which rebuilds a PDF's paragraphs from extracted lines (the justification signal: padded lines are full, an unpadded one ends the paragraph). Pure; the byte span of each segment is the "jump to the source" anchor, a PDF segment's page its row |
| `vault.rs` | Orchestrates disk ↔ parse ↔ index ↔ search; path validation (rejects `../` escapes). A note write names its writer (`write_note_as`) so the history knows whose text it keeps, lands through a hidden temporary file renamed over the note (a crash between truncating and writing leaves the previous text, never an empty one — the rename replaces the inode, so a hard link or a tag an OS stores on the file itself does not follow), and the watcher tells the echo of the vault's own write from an outside change by content: the file holding what the index holds is an echo, anything else is a change and its previous text is kept. Only `write_file` (non-Markdown) still swallows its echo by a time window |
| `watcher.rs` | notify + 500 ms debounce → a batch of paths → `handle_fs_change` |
| `event.rs` | `VaultEvent` (including the `ScriptsChanged` hot-reload signal). Since v0.3 workflows subscribe to these through the `broadcast` fan-out in src-tauri |

### crates/vemry-agent — the AI layer (depends on core, not on Tauri)

| Module | Responsibility |
|---|---|
| `provider.rs` | `ProviderConfig`: base_url + model + key. Zhipu (`glm-4.7-flash`) by default, swappable for any OpenAI-compatible endpoint; `is_deepseek()` decides whether the Anthropic-format transport (and with it web search) is available |
| `types.rs` | OpenAI Chat Completions wire types plus the streaming tool_call assembler. `ToolDef` is an untagged enum (`Function` plus `Raw`, which passes a server-side tool through verbatim); `ChatMessage.extra_items` carries the Anthropic-format line's replay blocks across turns |
| `client.rs` | The `ChatTransport` trait (mockable) plus the reqwest SSE implementation — **the LLM channel boundary**. Since v0.4 the system has two further controlled network exits (see "The built-in knowledge browser"). v0.7: a process-wide shared connection pool, connect timeout, a 120 s stream-idle timeout and cancellation polling for a stalled stream; both transports share one read loop, `read_sse` |
| `anthropic.rs` | The Anthropic-format transport (DeepSeek's `/anthropic/v1/messages`) — a second implementation of the same `ChatTransport` trait, and the carrier of web search. On the request side it maps messages→content blocks, system→`system`, tool results→`tool_result` blocks and function tools→`input_schema` tools, spelling the runner's `{"type":"web_search"}` as the `web_search_20250305` server tool; on the response side it synthesises `StreamChunk` from the semantic SSE events, so the runner and the mock need no changes. `server_tool_use` / `web_search_tool_result` blocks (the results as `encrypted_content`) and a thinking block's signature are replayed through `extra_items`. Used for a turn only when web search is on **and** the endpoint is api.deepseek.com |
| `tools.rs` | The domain tools, in three classes. Read-level (list_notes / read_note / read_file / read_block / search_notes / get_links / clips_lookup / recent_changes / aggregate_blocks / query_blocks / read_skill) execute directly behind the privacy gate; write-level (write_note / edit_note / append_note / set_frontmatter / update_task / set_block_field / restructure_note / move_blocks / move_note / move_file) are first "prepared" into a `WriteRequest` — carrying the rendered diff and the exact content that would land on disk (a move is a path pair, `path → to_path`, with no diff) — and execute only after the gate; session-level (`is_session_tool`: update_plan / list_sources / propose_plan / propose_actions / open_for_reading) act on the session's own state, touch no note and pass no write gate (open_for_reading passes the read gate instead). `import_page` is the one tool of both kinds: the page passes the read gate, the app composes the source note from it (`import.rs`: the page's sections as headings, one block per paragraph with a stable `^s<section>p<n>` id, equations as `$$…$$` with their numbers, figures with their captions as `^figN`, table captions as `^tabN`, the reference list left out) and the note then lands as a write_note — the same diff, gate and provenance — with the outline of ids as the receipt, so a paper is archived without its text passing through the model as output tokens. `update_task` (state and dates in one rewrite of the task's own line) and `set_block_field` (one inline `[key:: value]` on a block's first line) go through `vemry_core::taskedit`, the Rust copy of the editor's `tasks.ts` / `fields.ts` grammar, so a task the AI ticks lands on disk byte for byte as one dragged on the board; `recent_changes` reads the block change times (`blocktimes.rs`) for "what changed since". `move_note` and `move_file` go through `Vault::rename_path`, the same implementation the file tree's move uses, **and rewrite the links that pointed at the moved thing in the same write** (`vemry_core::relink`: the plan is computed at prepare time, shown as the move's diff, and landed after the rename — rename first, rewrite second, a note edited meanwhile skipped rather than overwritten; private referrers are never rewritten). `list_files` and `audit_links` (unresolved links, orphans, dangling `supports::`, duplicate clips — `vemry_core::audit`, index only) are the read side of a tidy-up. `PlanItem`/`PlanStatus` and `parse_plan_items` are update_plan's grammar. `append_note` lands by the vault's own rule (`insert.rs`: the end of a heading's section, or the content end — the same landing as a clip or a quick capture; a missing heading is an error, never a silent fallback; a missing note is a Create) and its diff is only the addition; `set_frontmatter` goes through `vemry_core::set_frontmatter_key` (one top-level key: a scalar, a list, or removal; `private` and `vemry` are refused). `is_local_write` names the three that a followed note takes without a dialog |
| `undo.rs` | The write journal and "undo this turn's writes": every landed write is recorded with what it replaced (the note's previous text; a move's path pair and link rewrites), and undoing a turn walks its records backwards — a note still holding what the write left gets its previous text back, a created note goes to the trash, a move moves back when nothing has taken its place, a rewrite is restored when its note is untouched; anything edited since is skipped and counted. The journal lives with the session in memory; the jsonl gets an `undo` marker (counts only). Not a snapshot, not git |
| `sources.rs` | The session's web source table: `extract_citations` (Markdown links and bare ASCII URLs, in document order, deduplicated — the same rule the frontend numbers its superscripts by), `merge_turn` (the verification verdict: a search ran this turn, or the user handed the page over; otherwise the URL came out of the model's prose alone and shows as "unverified"), `drop_turn` (a regenerate or an edit-and-resend takes its turn's sources back) and `read_sources` (the table rebuilt from the audit's `sources` lines with the retry / edit_resend markers applied — the replay and the live path agree by construction). The runner settles every turn's citations into it at the end of the turn; `list_sources` reads it back to the model; compaction can no longer lose it |
| `testing.rs` | `ScriptedTransport` and chunk constructors for the offline evaluation suite (`tests/evals.rs`): every `tests/evals/*.json` fixture builds a small vault, replays scripted model output through the real runner and checks the product standards that need no provider — citations resolve, sources carry the right verdict, the plan is state rather than prose, tools succeed, nothing reaches the write gate |
| `permission.rs` | The `PermissionGate` trait. The write gate is enforced in the Rust layer that owns the ability to write files — the frontend can render the decision and nothing else |
| `runner.rs` | The agent loop: stream → dispatch on `finish_reason` → execute tools → feed results back → repeat (25 iterations max). Session-level tools are executed here (update_plan publishes a `plan_update` line and a `PlanUpdated` event; the plan gate's approval publishes the confirmed list the same way, so the sticky plan card is a projection of `plan_update` alone), and the end of every turn settles the turn's citations into the source table (`SourcesUpdated` + a `sources` line). A turn id is made unique against the history (a `-n` suffix when two turns land in one millisecond). The `ChatTransport` mock makes every branch testable. v0.7 added three layers of resilience: transient errors (429/5xx/dropped stream) back off and retry as long as nothing was produced; a context overflow first compacts, then summarises, then retries, so a session is no longer written off; usage is reported per iteration to drive the frontend gauge and proactive compaction; a wrap-up hint is injected with five iterations left; `finish_reason=length` reports truncated; `reasoning_content` is kept in history and sent back (the DeepSeek thinking contract); tool results have a 32k backstop; `user_message=None` means regenerate — no user message is appended and the tail of history is replayed |
| `compact.rs` | v0.7 two-level context compaction, pure logic. Level one replaces older successful tool results with recoverable handles (the note is on disk; calling again fetches it back — ERROR lines are never cleared). Level two has the LLM summarise everything before the current turn, with a hard truncation as the fallback if summarising fails. `is_context_overflow` is the trigger for reactive self-healing |
| `translate.rs` | Side-by-side translation (see "Side-by-side translation" under the app shell): a `TranslateJob` sends a document's segments in batches of about 2k characters as `<s id=N>…</s>`, `StreamParser` closes each segment the moment its `</s>` arrives (the row fills in while the batch is still streaming), a segment the model leaves out is resent alone once and then reported, and `TranslationCache` files the translations by the hash of each segment's text. The same tool-free single call the summary makes (`tools: None`, thinking off), not an agent turn: no session, no tool table, no ledger |
| `session.rs` | The `.vemry/sessions/*.jsonl` audit log: every user message, model output, tool call, diff and permission decision. It is the data source for the v0.2 session-replay view. v0.8 added a title override row (manual rename; the last one wins, and a fork inherits it). v0.7 added retry/edit_resend/compact/fork marker rows (the audit is immutable; the presentation layer folds on the markers) and `fork_session` — a research-tree fork that copies the audit prefix. The fork point is located by turn id: the runner stamps one `turn` id into the audit's user row, the model-history envelope and the `TurnStarted` event at the top of each turn, because timeline, audit and sidecar each counting user ordinals independently falls out of step after a compaction synthesises a message or an edit-and-resend leaves a stale row. Older sessions with no ids fall back to ordinal counting |
| `prompt.rs` | The system prompt is fixed rules + VEMRY.md (8k cap; over that it truncates and points at indexing it) + a Skills summary (name — description) + a vault overview. v0.7: **frozen as a session-level snapshot** — the glue layer takes it once per session and reuses it, because rebuilding between turns guarantees a provider prefix-cache miss, and DeepSeek's hit/miss price difference is roughly 30×. Volatile information (the date, the current note) rides the turn-context tail in `compose_user_message` instead |
| `skills.rs` | Instruction-level skills: the built-in playbooks (`crates/vemry-agent/skills/*.md`, compiled in, parsed once per process) and the vault's `.vemry/skills/*.md` in one listing by name — a vault file shadows the built-in of the same name; frontmatter `name`/`description`/`disabled`/`version` — and the implementation of the `read_skill` tool. See "Skills (instruction-level)". The glue layer's `type_templates.rs` holds the example block types Settings can install (never overwriting a file that is there) |

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
- `browser/` (v0.4; the same-window panel since v0.9): `window.rs` builds the
  tabs (child webviews), places them, moves them between the main window and the
  detached window and manages their lifecycle (`browser://event` pushes state to
  the chrome); `browserbar.rs` is the detached window's chrome relay; `clip.rs`
  holds the clip commands (`eval_with_callback` plus a oneshot to fetch the
  selection or DOM; writing goes through `write_note` + `broadcast`, so indexing,
  frontend refresh and workflow triggering are all free); `adblock.rs` is the rule
  compilation pipeline (see "The built-in knowledge browser").
- `translate.rs`: side-by-side translation's glue — the source (a note, the reader's
  page, a PDF) is cut into segments here (one in the privacy zone is refused whole), one job at a time is held,
  its events are relayed as `translate://event`, and `translate_save` writes the
  translations as a note through `write_note` + `broadcast` like any other note.
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
  block's run button and output (v0.3). `math.ts` typesets `$…$` and `$$` blocks
  with KaTeX (the grammar is `$lib/math.ts`, shared with the projection renderer so
  an embed and the editor agree on what is a formula). `wikilink.ts` / `mdlink.ts` /
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
  → the user: allow / allow for this session (everything, or only this folder,
    or only this note) / always allow in this vault / edit then apply / deny
    (with a reason)
  → a oneshot wakes the loop
  → allowed: the note is still the text the request was prepared against? (`WriteRequest.baseline`;
    the gate awaits with the vault lock released, and a blur saves the buffer, a sync client
    or a checkout replaces the note, a note appears where one was to be created — the
    approved diff would land over text nobody reviewed, so the write is refused and the
    model reads again)
    → vault.write_note_as(Agent) (keeps the previous text in .vemry/history → incremental reindex)
    → vault.record_provenance(the blocks the applied bytes added, under the session id) — pending review
    → AgentEvent::VaultChanged (the NoteModified / NoteAdded / NoteRemoved the write means) → the app broadcasts it like an IPC write — the watcher would have swallowed the change as a self-write, and without this the open note's buffer kept the pre-edit text until reopened
    denied: an error tool result goes back to the model, which adjusts
  → the audit log records permission + tool_result
```

Grant scope (v0.7): "allow for this session" lives in
`RunnerConfig.session_allow` (a shared `WriteGrants`: the `all` flag plus a list
of path prefixes) and survives across turns; leaving a session clears it and
reopening one restores it from the jsonl's `permission` lines (decided
allow_session / allow_scope — `session::read_session_state`, one read of the
jsonl that also carries the source table and the followed note, applied by
`AgentState::load_session` from `agent_session_select` and a fork), so coming back to a session, or restarting,
does not ask again for what the session was already granted. It used to be a local in `run()`, which
meant the button said "session" while the grant actually lasted one turn. The
scoped form (the dropdown on the same button) grants a prefix instead of
everything — the folder the note is in, with its trailing slash, or the note's
own path — so a tidy-up of one project runs without a dialog per note while a
write anywhere else still asks; a move is covered only when both of its paths
are. The ledger records the grant (`decision: allow_scope`, `scope`) and marks
each write it later covers `via: scope`, the way the working note's local writes
are marked `via: follow`; the writes stay pending in the editor like every
standing-grant write. "Always allow in this vault" is merged into the
`write_policy` key in `.vemry/config.json`, with a clickable revoke badge in
settings; revoking immediately withdraws the cross-turn grant from a session in
flight (a boolean already honoured inside the current turn is not retroactive, and
per-write approval resumes from the next turn).

### An external edit (another editor can be open at the same time)

```
a notify event → 500 ms debounce → handle_fs_change
  → the file holds what the index holds? it is the echo of our own write: ignore it
    (by content, not by a time window — a sync client writing the same note right
    after us is a change, and a window would swallow it)
  → keep the indexed text in .vemry/history (who: external) → reparse the one file
  → update the index incrementally
  → emit("vault://event") → the UI refreshes locally (a clean buffer reloads; a
    dirty one keeps typing and is told, once, that its next save lands over that
    change and where the change is kept)
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
  This is a deliberate widening and does not conflict with the browser tabs'
  "remote pages get zero IPC" (`browser-*` still matches no capability). The
  third of our own webviews, the detached browser window's chrome bar
  (`browserbar`, `capabilities/browserbar.json`), is matched exactly too.
  Capabilities always match a label exactly, never a wildcard, and they are
  scoped to **webviews**, not windows — the main window has had child webviews
  (the browser tabs) since v0.9, and a window-scoped capability would have let
  every remote page inherit the main webview's IPC.
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

- `{"kind":"agent","ts","session","path","blocks":[{hash}],"hunks":[…]}` —
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
selection clip's source line (`> [!quote] [title](url) · date` as the callout's
head, or the `> — [title](url) · date` credit line older clips end with), a whole-page clip's
`vemry.origin: clip` frontmatter, and a pre-ledger `vemry.origin: agent` note
(reported as agent, session unknown, not pending — in a note the ledger knows,
a block without a record is the user's). New agent-created notes now carry
`vemry.agent_session`, and the audit's `permission` row records the
`applied_diff` of an "edit, then apply". IPC: `provenance_get` (every block's
origin, with the located hunk and what it replaced for a pending one),
`provenance_accept` (a set of blocks or the whole note; not broadcast, since
accepting is not a note event), `provenance_record` (the user-placed inserts),
`provenance_pending_notes` (the notes with pending blocks, in path order, each
with its count — the one list the status bar's chip, the file tree's marks and
the review's note-to-note step read), `provenance_accept_notes` (every pending
block of a set of notes, or of the vault), `provenance_revert_plan` (below).
Known limits: a pending block
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
under the cursor when it is not the user's, and shows "unreviewed N" with a
step either way; the block menu offers "accept AI change" and "accept all in
this note"; the sessions view's write rows open the note on its first
unreviewed change with the overlay on ("view the written blocks") and show the
`applied_diff` of an edit-then-apply.

Review is a walk, in runs. The blocks one hunk produced (a heading and the
paragraphs under it) are one **run** with one pair of controls on its first
line — ✓ marks the run reviewed, ✕ puts its write back, and a run whose write
replaced lines has ⇄, which unfolds what it replaced above the run, struck
through and read-only; a block inside a run of several keeps its own pair,
shown when its line is hovered, and the block menu and the palette still act
per block. Either decision **steps on**: the cursor lands on the next run
below, else the note's first run still unreviewed, else the next note with
unreviewed blocks opens on its first run (`vault.reviewNextNote`), and when no
note has any left a toast says so. ⌘Y and ⌘⇧Y are the two decisions from the
keyboard (bound inside the editor, falling through off a run, so Ctrl+Y stays
redo where it is one); ⌘⌥↓ and ⌘⌥↑ step without deciding, across notes at
either end (`vault.reviewStep`). The note-to-note order is path order over
`provenance_pending_notes`; a note opened for review lands once the editor has
its rows (`vault.reviewLanding`, consumed by `loadProvenance`), since the rows
come from an async read after the open resolves. The same list marks each
note in the file tree with an amber dot, fills the status bar's vault chip
("other notes M · N blocks"), whose click lists those notes with their counts
(a click opens one on its first run; marking the whole vault sits at the
bottom, behind a second click), and puts a strip under a turn's final answer
in the chat when notes it wrote still await review — "Review" opens the first
of them, "mark all as reviewed" takes just that turn's notes; both run in the
main window (`HostAction` review-start / review-accept), the turn's notes
coming from `write_landed` events (`agent.notesByTurn`). Vault-wide revert is
not a separate action: the turn's own "undo this turn's writes" is that. The user-placed inserts are recorded at
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
block's hash. The mark itself follows the hunk: `provenance_get` reports, for
a pending block whose hunk can be found, the part of the block that write
added (`changed`), and the amber bar and the controls sit on those lines — a
block the model merely extended keeps its reviewed lines bare, the block stays
the unit of accepting, the line the unit of showing, the hunk the unit of
deciding (the run above). The status bar has "✓ This note" and "✕ This note";
the block menu and the command palette carry both per block. Independently of the ledger, a reload of the open
note from disk — an agent write landing, an edit in another app — is applied to
the buffer as one edit rather than a fresh state (`Editor.svelte`'s `loadDoc`):
the undo stack survives it, and ⌘Z takes an agent write back the moment it
lands. The view then lands on the change: the changed span is scrolled into
view and the cursor goes to its first line — the line above a lab block, since
a card's reveal counts its first line as inside — unless the reader holds a
range selection (`revealExternalChange`). The landing is also what keeps the
note from going blank. Measured on a chapter with a figure embed: a reload
with the view at the note's end rebuilt the widgets above it, the embed
measured a fraction of its height before its figure loaded, the height map
shrank by the difference, the browser clamped the scroll offset past the
document, and an empty viewport renders nothing and so never measures again —
the note stayed blank until a scroll. Two guards close that path for good: an
embed keeps its last measured height as its body's minimum while its images
load (`embed.ts`, the `heights` cache, filled on the synchronous fill path
too), and the editor pulls a viewport that is empty while the document is not
back onto the document's end (`setup.ts`, `emptyViewportGuard`).

## Block change times (a cache of observations)

A note has a modification time; a block does not, and a task line carries no
creation or completion date. The board's unscheduled and done columns (and
Agenda's matching groups) still want "most recently changed first", and "the
note's mtime" would move every task of a touched note together. So the vault
remembers when each block's current text appeared (`vemry-core/src/blocktimes.rs`):
before a note is indexed, every block gets a change key — FNV-1a over its trimmed
lines, a trailing ` ^id` dropped — and a block whose key was seen before keeps its
stamp while a new key is stamped with the note's mtime. The note's mtime rather
than the clock, so a change made while the app was closed is dated when the file
actually changed. A toggled checkbox, an edited word, a changed date all count as
a change; minting an id, re-indenting or moving a block does not. This is
deliberately not provenance's identity hash, which strips the checkbox so a block
keeps its authorship across a toggle — here the toggle is the point.

The table lives in `.vemry/block-times.json` and is a **cache, not state**:
without it every block falls back to its note's mtime, which is what the views
showed before it existed, so deleting the file loses nothing but precision.
Opening a vault stamps but never saves (a directory merely being browsed must not
grow a `.vemry`, the rule `Vault::meta_dir` states); the vault's own writes save
and may create the directory, an external change seen by the watcher saves only
where `.vemry/` already exists, a rename carries the stamps over, and a rescan
prunes the notes that are gone. A save is throttled to one write in five seconds
(the table is rewritten whole, under the vault lock, and an autosave arrives about
once a second while the user types); a change left in memory goes out with the
next save — the watcher's echo of the write, the next autosave — or when the vault
is dropped, and what a crash inside the interval loses is again only precision.
The stamp reaches every consumer as
`Block.modified_ms` through the same `query()` as everything else; the agent's
`query_blocks` renders its own fields and does not print it.

## Seams left for later

| Future capability | The seam today |
|---|---|
| tantivy or semantic retrieval (unscheduled; the README roadmap's "next") | the `SearchProvider` trait |
| vemry Publish (distant) | vemry-core has no Tauri dependency and can be reused server-side |
| An AI gateway (distant) | the `ProviderConfig` abstraction |
| PDF and image Q&A (below; side-by-side translation is done, see "Side-by-side translation") | `read_file_bytes` / `file_read_binary`; the `currentPath` context channel is type-agnostic; `FilePreview` renders a PDF through pdf.js (`PdfViewer`, page numbers and jump-to-page exposed); `tool_defs()` is an incrementally extensible registry |
| The built-in browser: detaching a single tab, favicons, a page snapshot as the veil placeholder (below) | Tauri 2 multi-webview is in use (v0.9); `Webview::reparent` works per tab; the veil placeholder is a DOM slot that can take an image |
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
   serde and the Anthropic-format line maps to `text`/`image` blocks. Images become context
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
3. ~~**PDFs have no text extraction or indexing.**~~ — **extraction and caching
   done**: `context.rs::pdf_pages` pulls a PDF's text page by page with pdf-extract
   (a 20 MiB cap, a panic guard around the library, an explicit "no extractable
   text" note for scans) and caches it under `.vemry/cache/pdf/<hash of the
   bytes>` — the pages, the flat text with `[p. N]` markers that `read_file` hands
   the model and `search_notes` searches, and the path it came from. What is still
   **missing** is entry into the block index, which would make a PDF one more data
   source the projections are built from.
4. ~~**The native PDF viewer is a black box**~~ — **done**: `FilePreview` renders a
   PDF through pdf.js (`PdfViewer` over `PdfPage`: pages drawn as they come near, a
   selectable text layer, the page in view reported, `scrollToPage` exposed), so
   "ask about this page", page-aligned translation and jump-to-page from a
   citation have what they need; nothing outside `FilePreview` noticed the change.

On the model side: the default glm-4.7-flash is text-only, so image Q&A means
switching to a multimodal endpoint (glm-4v, say). Settings already supports that;
there is no architectural obstacle.

**What remains**: indexing PDF text into the block index, and image Q&A on a
multimodal endpoint (a scanned PDF can reuse the same inflation path). If image
resizing or transcoding is ever needed (tiff→png, compressing past 5 MiB), the seam
is the single `inflate_images` function.

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
  to its block through `openBlockAt` and enters navigation history. A row is a
  `BlockRow` (`views/blockrow.ts`: the one derivation of words vs. chips from a
  `BlockHit` — a task's checkbox, the text without markers and fields, the field
  chips with a state field's step on its chip, the tags minus the query's own, the dates),
  drawn with the chip builders of `lib/chips.ts`; the head sums the rows over
  their state field. A row's chips write back through the task views' funnel
  (`views/taskwrite.ts`; a field through `views/fieldwrite.ts`, which rewrites
  the field's own indexed byte span after checking the bytes still read as that
  field), and the index refresh that follows re-runs the block. A change to
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
- A block that calls `vemry.lab.*` is a lab (docs/SYNTAX.md, "Lab blocks"): the
  glue in `bindings.rs` normalises its calls into the canonical spec of
  `vemry-script/src/lab.rs` (four controls, six views, hard caps), which travels
  back in `BlockEvalResult.lab`; the editor shows the block as its card rather
  than its code (a block-level replace in `runblock.ts`, the same reveal
  semantics as a query block: the source only while the cursor is inside; the
  session survives the reveal in a state field keyed by the fence's start),
  renders the session as a live card (`editor/lab.ts`, the charts in
  `lib/lab/charts.ts`) and, while the card is open,
  re-runs the block with the controls' values as `params` on every move — the
  same one-shot Context, the same fuse, the code read again from the document
  each time. The lab API exists only in a block Context (the params global the
  engine sets is what enables it; a script never has it). "Pin" (`lib/lab/pin.ts`)
  is the lab's one write: it rewrites the declared literals in the fence through
  an ordinary editor transaction. None of this changes the entry rule — the first
  run is a click, and a re-run only follows a person's move.

### Block types (typed blocks on Markdown)

`.vemry/types/*.md` (`vemry-core/src/types.rs`, the same loader shape as the
skills: regular `.md` files only, a symlink refused, malformed frontmatter
degrading to defaults) declares a kind of block plus its inline fields, tag
and template. The editor lists the enabled types in the `/` menu and under
"turn into ›" (`editor/blocktype.ts`: one transaction that turns the block
into the kind and appends the fields it lacks); the system prompt names them
under `## Block types` so the agent writes such blocks the same way; the
watcher emits `TypesChanged` for the directory, which refreshes the menus and
invalidates the prompt snapshot. IPC: `types_list`, `type_create` (the "block
type…" command's open-or-create: a name with no file gets the skeleton, `book`
the full example, and an existing file is opened in the editor, never
overwritten), `type_set_disabled` (the same command's row button, or ⌘↩ on
the row: the file's top-level `disabled:` line is rewritten in place and
written through `write_meta`, so the menus and the prompt drop the type while
"block type…" keeps listing it for re-enabling).
A type file opens through the same `note_read`/`note_write` as a note, but
`note_write` routes a `.vemry/` path to `Vault::write_meta`: configuration is
never indexed, and the watcher's `TypesChanged` (own writes included) is how
the menus and the open editor follow it. A typed block is ordinary Markdown
with fields — no note points at a type.
A field declared `type: state` is a state field (`lib/fields.ts::stateField`:
the `values` in declared order are the steps; `stateStepOf` places a value).
Nothing about it reaches Rust beyond the type file's `type` string, which the
loader has always carried through: the glyph on the editor's chip
(`livepreview.ts`, through the `fieldState` option), on an embed's and the
reader's chip (`markdown.ts`), on a query row, an Agenda row and a board card
(`views/blockrow.ts`, `views/kanban.ts`) is one drawing, `lib/chips.ts`'s
`stateGlyphSvg(step, of)` — the task views' three-state icon is that drawing
at three steps, so a task reads as a state field with a fixed vocabulary and
checkbox syntax. `queryrows.ts`'s sort and group and the board's columns take
the declared order (every declared value a column, empty ones too); the value
menu (`FieldValueMenu.svelte`) lists the steps with "Advance to …" first; and
the system prompt spells a state field as `key (state: a → b → c)` so the
model moves blocks along it with `set_block_field` — the same schema
exception as before, one string longer.

### Skills (instruction-level)

Two layers of playbook, one listing. The built-in skills ship inside vemry-agent
(`crates/vemry-agent/skills/*.md`, `include_str!` into the binary, parsed once per
process) and never land in the vault by themselves: a vault that wants one changed
takes a copy ("customise" in Settings writes the built-in's file into
`.vemry/skills/`), and from then on the copy shadows the built-in **by listed name**
(frontmatter `name`, defaulting to the file stem). The vault's other files are its own
skills. So an update to the binary reaches every vault that has not customised the
skill; a customised copy is the user's for good, and Settings flags it as "built-in
updated" when the `version` in its frontmatter is behind the built-in's (a copy without
one counts as 0) — "restore built-in" trashes the copy. Switching off: a vault file by
its `disabled:` frontmatter line (rewritten in place); a built-in by its name in the
vault config's `skills_disabled` list, since there is no file to write. A disabled
override hides the built-in behind it too — the row the user switched off is the
skill. Every switch, copy and new skill drops the prompt snapshot, so it applies from
the next turn; a file's text is read live.

VEMRY.md is the resident constitution; skills load on demand, by two paths. The model's:
`build_system_prompt` injects a `## Skills` summary section (names and descriptions
only, with the rule that a matching skill is read **before the plan** — the plan rules
say the same, and the skill's Plan section is then the list) and the model calls the
`read_skill` tool for the full text (read-level, truncated at 30k characters). The
user's: `/` in the composer opens the skill picker (the enabled skills, matched on
name, description and the frontmatter `triggers`), and the chosen skill becomes a chip
— a `ContextRef::Skill { path: name }`, riding the same pipeline as every other
reference (the audit's `refs`, the window handoff, edit-and-resend) — whose text Rust
injects into the turn's user envelope as `<skill-context path="name">` with an intro
that says it is the method to follow, first among the envelopes and outside the
reference cap. A message with a skill chip and nothing typed goes out as `/name`: a
routine is a whole request by itself. Both paths write a `skill` line to the ledger
(`via: attached` / `via: read`) and raise `SkillActivated`, which the status strip
shows as "by name" for the rest of the turn. Two things keep the method in hand until
the task ends: the first-level compaction never turns a `read_skill` result into a
handle (an attached skill's text sits in the user envelope, which compaction does not
touch at all), and the second-level summary is told to keep the skill in effect and the
steps of its plan not yet done. The system prompt also states whether web search is
available in the session (RunnerConfig.web_search is fixed per session, so the sentence
sits in the cached prefix), which is the one fact about the tool table a skill cannot
know by itself. `read_skill` matches a name exactly against the already-scanned
list — the vault's file first, then the built-in — and never concatenates caller
input into a path, so it is immune to traversal by construction. A frontmatter serde
cannot parse (an unquoted description with a colon in it is the usual slip) falls
back to a line scan for the four keys the loader reads, so a skill never loses its
description, and a `disabled: true` never loses its effect, to a YAML edge. A new
skill from Settings starts from the seven-section skeleton every built-in follows
(When / Plan / Inputs / Do / Landing / Shape / Never).

**Privacy semantics (a deliberate trade-off)**: `.vemry` is a built-in privacy zone
as far as the agent is concerned, and two directories are deliberate, narrow
exceptions: the skills directory — the whole point of a skill is for the agent to
read it — and the types directory, whose schema (never its files) the system
prompt lists so the agent writes typed blocks the way the user does. The exception
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
  edge of the window. The docked browser panel is a column of `.frame` (which is a
  row: `.frame-main` — the vertical wrapper around `.shell` plus the status bar —
  then the panel), not of `.layout`: it never swaps sides with the sidebars and it
  is the full height of the main window, the native tab view laid over it below the
  panel's chrome. Its width is the common term in three coupled minimum widths, and
  widening it means
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
  `getCurrentWebview().label` (`main.ts`; the webview's label, since the browser
  bar's window is not the bar), not by a URL parameter. The two webviews
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
- **The quit handshake**: ⌘Q, the main window's close button and the last window
  going all reach Rust first (`RunEvent::ExitRequested` without a code, the main
  window's `CloseRequested`); the first request is refused and the main webview is
  asked (`vemry://quit-requested`) to flush its buffer, and it answers with
  `quit_confirmed`, which exits with a code the handler lets pass. A second request
  passes regardless (`AppState::quit_asked`), so a webview that never answers
  cannot hold the app hostage; a relaunch after an update flushes on its own,
  since a restart's exit cannot be held. Without this, the timer deferring every
  save while the agent runs meant a quit right after a long turn took those
  minutes of typing with it.

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
double-closes into `[[name]]]]`. Enter and Tab both accept; for a note name Enter
lands after the `]]` while Tab keeps the caret inside the brackets, so `#` goes on
to the note's headings and blocks (`blockref.ts`) and `|` to an alias — the drill-in
convention the `@` picker and the palette use. Completion candidates (link targets, tags) filter
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
chip. The block menu toggles a block between private and plain ("make
private" / "remove privacy"), and `/private` in the slash menu does the same from
the keyboard. The file tree's context menu and the palette's "mark as private"; a private
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
  tool only, and only `.md` directly under that directory — see "Programmability"),
  and `.vemry/types/` is the other: the system prompt lists each enabled block
  type's schema — name, kind, tag, field keys, description — never a file's body,
  and `disabled: true` takes a type out of it (see "Block types");
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

### Shape: one browser surface with two homes (v0.9)

Every page is a **tab**: a `browser-N` child webview (`Window::add_child`, Tauri 2's
`unstable` multi-webview API), built hidden as a child of the main window and moved
between windows with `Webview::reparent` — no reload; the page, its scroll position
and its history come along. The surface has two shapes, remembered through
`settings.browser_docked` (docked by default):

- **Docked**: the tabs are children of the main window, laid over the **browser
  panel column** at its right edge (`src/lib/components/BrowserPanel.svelte`, the
  outermost column of `.frame`, the window's full height). The chrome — the tab
  strip, the address row, the clip and AI entry points — is the main window's own
  DOM, and the native tab view is placed under it. Only the active tab is shown
  (`show`/`hide`); the others stay alive hidden. The panel column is there exactly
  while there are tabs: closing the last one (×, ⌘W) is how the panel goes away,
  and opening a page (⌘L, a link, "open browser") is how it comes back — there is
  no hide switch, twice tried and twice a button nobody could place; "out of the
  layout but kept" is what detaching is for.
- **Detached**: the tabs move into a standalone window `browserwin` — a plain
  `WindowBuilder` window with no webview of its own — whose first child is
  `browserbar`, our bundle mounting the same `BrowserPanel` in bar mode as a 72 px
  chrome across the top (`capabilities/browserbar.json`, matched exactly on that
  label). The tab strip is the window's drag region; on macOS the title bar is
  Overlay with the traffic lights floating over the bar's left end (a page is a
  WKWebView that swallows a drag, so the chrome has to be the handle). The window's
  title follows the active tab. Closing the window (the red light, ⌘W on the last
  tab) closes every tab still living in it and returns the focus to the main window;
  the shape stays remembered, so the next page opens detached again.

**Zero IPC for remote content is guaranteed at the configuration layer**: the
capabilities match exactly on `main`, `agent` and `browserbar`, and a tab's label
matches none → `window.__TAURI_INTERNALS__` does not exist. The navigation guard is
belt and braces: `on_navigation` allows only http(s) and rejects the app origin
(localhost:1420, tauri://); `on_new_window` always denies and opens in the same tab
(popup suppression). A **warm-up shell** — a hidden about:blank child webview of the
main window built 3 s after startup and refilled after each claim — has its
WKWebView, its subprocess and its ad-blocking rules in place before a page is asked
for, so opening one is left with navigating, placing and showing (`claim_tab`; in
the detached shape it is reparented first).

**Geometry — who computes the bounds.** Docked: the frontend reports the panel's
anchors (`browser_panel_layout`: mounted or not, the column's width, the
chrome's height) only when they change; on a window resize Rust re-places the tab
from the anchors with no round trip (`relayout`, the main window's Resized
hook), so a live resize does not lag. The width is the user's (a sash on the notes
side of the boundary; 360 ≤ width ≤ innerWidth − 804, the same floor as the
layout's three minimum columns plus the activity bar). Detached: the bar is (0, 0,
w, 72) and the page (0, 72, w, h − 72), re-placed on that window's Resized; the
window geometry restores `settings.browser_window` or is centred at nearly full
height on the main window's screen (`initial_browser_rect`; a remembered position
that lands on no monitor is discarded, so it never opens off-screen).

**A native view covers the DOM** — the one cost of same-window embedding, handled
by three rules with one collection point each:

1. *Full-window overlays veil the tab.* App derives `overlayOpen` from the
   palette, the settings and permission dialogs, a script grant or prompt, the image
   lightbox and a panel-sash drag, and sends `browser_panel_veil`; Rust hides the
   active tab, the panel paints a placeholder (title and domain) that dims with the
   backdrop, and the tab is shown again once the overlay closes.
2. *Popups stay left of the panel.* `usableRight()` (`src/lib/viewport.ts`)
   replaces `window.innerWidth` in every popup clamp (popmenu, ContextMenu,
   HoverCard) and in the toast stack's inset; the chrome's own menus (the clip ▾,
   ⋯, a tab's context menu) are **native** popup menus (`@tauri-apps/api/menu`),
   which float above every view.
3. *A DOM drag ends at the panel's edge.* The pointer is lost to the native view the
   moment it crosses into the page, so the panel sash veils the tab for the whole
   drag (the width is live in the DOM, the final value reaches Rust when the drag
   ends), and any other drag simply stops at the boundary.

**Keyboard and focus.** A page holding the first responder swallows every key before
the main window's keydown chain sees it, so the browser's keys are menu-bar
accelerators (class 1 in menu.rs): ⌘L open URL, ⌘T new tab, ⌘R reload, and ⌘W. They
are routed by where the keyboard focus is: `focused_tab` walks the first responder's
superview chain against the native view pointers recorded at build
(`BrowserInner.views`) — inside a page ⌘W closes the tab, elsewhere it keeps its
editor meaning; with the detached window focused, ⌘L and ⌘T go to the bar
(`browserbar://command`). The focus-return contract stays: after any chrome
interaction `browser_focus` hands the keyboard back to the page, so space-bar paging
and keyboard scrolling never break. Switching is `browser_activate`; closing follows
the open order (the next tab to the right, else the nearest to the left).

`get_webview_window("main")` is None as soon as the main window has a child webview
(a WebviewWindow is a window whose only webview carries its own label), so the main
window is reached through `browser::window::main_window` (`get_webview("main")` →
`.window()`) everywhere, and `webview_windows()` never lists it; with the warm-up
shell that is true for the whole session.

**App-level history and bookmarks** (v0.8): `browser_history.json` and
`bookmarks.json` under `app_config_dir` (the same "shared across vaults, never
inside a vault" decision as settings, with an RwLock in-process cache), deliberately
not stuffed into `Settings` — `settings::load` is cloned per request on the
navigation and image-proxy hot paths, and a large list in there would be copied over
and over. The palette's two "find" spaces — notes (⌘⇧F) and the web (⌘L) — share
one entry, the activity bar's magnifier, and a segmented head at the top of the
palette switches between them with the typed text carried over; the lists never
mix (a query is either a note search or an address), so the browser needs no
launcher of its own on the rail. History has exactly one writer, the tab's build hooks: `on_navigation`
records the visit and `on_document_title_changed` fills in the title by label →
registry url (url and title arrive on separate beats; the title hook is also the SPA
backstop, reading the real URL back when a pushState changed the page without a
navigation). The same URL **merges and moves to the top globally** (visits
accumulate, capped at 500), so "the last page" is simply the first row and the ⌘L
list needs no second deduplication — at the cost of exact ordering, which is fine
because this is "places you have been", not a forensic log. The hidden reader window
has none of these hooks, so the reading projection stays out of history by
construction. Bookmark changes (`bookmark_toggle`, `bookmark_remove`) broadcast a
full `BookmarksChanged` snapshot through `browser://event`, keeping the ★ mirrors of
the panel chrome and the detached bar in sync (single writer in Rust, the same "ride
along on something that is not tab state" pattern as `AdblockStatus`). The UI
converges on ⌘L: an empty query offers "back to the browser / reopen the last page"
plus bookmarks and recents; with a query it fuzzy-filters bookmarks and history
across both title and url (the direct row preserves the "what I type is a URL"
model), and a bookmark row can be removed inline. **Direct invocation** (the menu's
Research → Open browser, ⌘P, and an empty ⌘L Enter) all share one Rust
implementation, `browser_open_last`: bring the active tab out, or reopen the most
recent page from history if there is none — neither of which the frontend has to
prompt for.

### The chrome: tab strip, address row, clip and AI entry points

One component for both shapes (`BrowserPanel.svelte`; `mode="docked"` in the panel
column, `mode="bar"` as the root of the `browserbar` webview). The tab strip: a tab
per `browser-N` (title, else domain; × closes, middle click too, a native context
menu), `+` for a new tab — a **blank tab in place** (`browser_new_tab`): its page stays
hidden, the address row's input opens on its own and takes the keyboard, and the slot
shows the panel's own new-tab page under it (bookmarks and recents, filtered by what is
typed — the address row is its one input; ↵ opens the address or the selected row, Esc
abandons the tab), so opening a tab never sends the eye to a popup — then detach (docked) or dock back (bar), and × which closes the
browser outright (`browser_close_all`; the plain "close a window" gesture, since there
is no hide-but-keep state). The
address row: ← → ↻ (history.back/forward sent blind — canGoBack is unknowable), the
address (domain plus title; a click edits it inline: ↵ navigates the active tab,
⌘↵ or an address opened by `+` opens a new one; in the main window ⌘L opens the
palette's URL mode with its recents and bookmarks, and `+`/⌘T open it with a new-tab
intent), ★, **AI**, the **clip** split button and ⋯ (the ad-blocking status, open in
the system browser, clear history).

**AI** follows the shape: docked, the page joins the research panel's conversation
as context (the panel is brought out); detached, the floating chat window is
summoned with the page attached (relayed as `summon-agent`: the main window opens
the window first for immediate feedback, then extracts the page through
`browser_page_context` and attaches it through `agent_window_to_agent`'s
`attach-page`). **Clip**: the main button is selection → the pinned target (the label
shows the destination; `clipTarget` is written by the main window alone and
mirrored to the bar), and ▾ is a native menu: pick a destination…, page → clips,
link → note, unpin, open the clipped note. Docked, every action runs in place
through the registry commands (`clip-selection`, `clip-page`, `clip-link`,
`clip-selection-to-note`); in the bar **every action is relayed to the main window
for execution** (`browserbar_to_main` → `browserbar://action` → App's clip*Flow;
delivery must go through the main window's editor buffer, and the landing record is
kept there too), with the receipt going back through a targeted `emitTo` on
`browserbar://result` to flash on the bar (it cannot see the main window's toast).
The bar's destination picker is the simplified two-stage pick-note (the data
contract shared with the palette in `$lib/pickNote`); it grows the bar downwards
over the page (`browserbar_set_expanded`; Rust re-adds the bar above the tabs after
every reparent, since a reparented view lands on top of its siblings). Only "already
clipped → open the note" brings the main window forward. The research panel no
longer carries a page card: the browser's state lives in one place, the panel
chrome, and the @ picker's "the page in the browser" is the conversation's other
door.

**Returning to the landing note on close** (closing the loop): during a browsing
session (from the first tab opening to the last one closing) the note that the most
recent clip landed in is recorded in the main window (`lastClipLanding`, recorded
only in clip*Flow and deliverPicked; the reading projection, an agent pin and quick
capture do not count). When the last tab closes, Rust has already returned the focus
to the main window and the frontend navigates with `openNote`, then — if there is a
section — scrolls to it with `revealPos` using the frontend's `extractOutline` (a
UTF-16 measure; `query_blocks` spans are byte offsets and cannot be fed to CM
directly). The close gesture is ⌘W inside the page (see keyboard and focus above);
off macOS the panel's × and the detached window's native close button.

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

- **Selected text** → a quote callout (`> [!quote] [title](url) · date` as the head
  line, the quotation below it; past a few lines it opens folded, so the source is
  the one line the note shows), inserted at the cursor in the current note or saved
  into `clips/`; it can also be
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
and the link becomes `[[Note#^id]]` — a heading, which cannot carry an id, is
linked by its text as `[[Note#Heading]]`, and a block that is gone or cannot
be anchored (a table, a code block) degrades to `[[Note]]`. The note part is the
note's stem when the stem alone resolves to it, the path otherwise, and the
stable citations the model wrote itself get the same treatment so one answer
links a note one way. Because a block may move between the answer and the click, the panel
captures each positional citation's content when the turn ends and passes it
as a guard: a mismatch falls back to the one block that still has that text.
The guard lives in `src/lib/citation.ts` next to the shared block cache.

The other direction — the user handing one block to the model — is the
composer's `@Note#` stage: typing `#` after a note name lists the whole note,
its headings and its anchorable blocks, and a pick becomes
`ContextRef::Block { path, local_id }`. Tab on a highlighted note row enters the
stage the same way, completing the token to the note's stem (`@手测` becomes
`@手测夹具#`) and remembering the row, so a note moved to with ↓ is the one
staged rather than the first fuzzy hit of the typed text — the same convention
as the palette's pick-note stage; on any other row Tab confirms like Enter, and
deleting the `#` returns to the notes. The trigger regex accepts single inner
spaces in the note part only when a `#` follows, so a completed `@block 方案#`
stays a token while a space after a plain `@word` still ends one. Rust resolves it at send time through
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

### AI web search (DeepSeek's Anthropic-format line)

Search uses the **provider's server-side** `web_search` tool — no new search vendor
inside the app, no second key, and search stays inside the existing LLM channel.
The carrier is `AnthropicTransport` (the second implementation of the
`ChatTransport` trait; see the vemry-agent table): DeepSeek's Anthropic-format API
is where the provider keeps the search (its `server_tool_use` /
`web_search_tool_result` blocks are documented as supported), while its Responses
API, the carrier until 2026-09-14, began listing every built-in tool as ignored
that day. A turn searches when `web_search_enabled && base_url == api.deepseek.com`
(`ProviderConfig::web_search_available`, the one predicate behind the composer's
web search switch, the per-turn check and the transport choice), on any DeepSeek model; the
switch is off by default and the default path is unchanged.

**The switch is per turn, not per session.** The chat composer's model menu carries
the switch (an item beside the thinking level; a globe on the model label says it
is on, so what the user sees before sending is whether the message can leave the
machine), the settings page writes the same field, and `agent_send` reads it afresh
on every send: a click takes effect from the next message. The system prompt does not mention the
switch (one sentence, true either way, says search exists only when the tool is in
the list), so switching changes nothing but the tool table — the one message after
a switch misses the prefix cache, and the next one hits again. That is the whole
price of not freezing it, and it buys "click it and it is on" instead of "start a
new session". The item is disabled, with the reason, when the endpoint cannot
search; there is no after-the-fact "search inactive" state to explain.

Measured live (2026-09-14, deepseek-flash and deepseek-v4-pro,
`live_anthropic_web_search`): the results come back **structured** — title, url,
page age, the body only as `encrypted_content` — and the usage counts the
searches (`server_tool_use.web_search_requests`); one message can carry a search
and a function call together. A session moves between the lines when the switch
changes (`live_history_moves_between_the_lines`): a thinking block rebuilt from
`reasoning_content` with no signature is accepted, and the Chat Completions line
ignores the search blocks. What a search returns is shown as it is: the queries on
the timeline through `AgentEvent::WebSearchQueries`, the hits on the source card
through `WebSearchResults` before the answer is written, and the count in the
turn's usage. The session log keeps queries and links only, never page text. A
hit is not a source: the end-of-turn settlement (`settle_sources`) records the
answer's citations (plus the pages read through the gate), a hit only lends its
title to a citation that names the same URL bare, and the card lists the citations
first with the uncited hits behind a second toggle (a turn may see fifty hits and
cite three). Clicking a source lands in the main window's reading projection by
default (v0.5 P2; ⌘click opens the browser window) — completing the search → source
→ read → clip → settle loop. The Raw tool body stays parameterless: on the
Responses line the OpenAI-style `filters.allowed_domains` was accepted and ignored
(2026-08-19), and whether the Anthropic-style `allowed_domains` takes effect here
is not yet measured (`live_anthropic_web_search_params_probe`); a control that does
nothing is worse than none. Search does not touch disk, does not leave the LLM
channel, and does not pass through PermissionGate (the principle that the
permission boundary is the write path is unchanged).

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

1. The LLM channel (chat completions / the Anthropic-format line, server-side search included) —
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
   one silent query to GitHub Releases' `latest.json` at launch. Either way the whole
   flow runs in one dialog: the version found with skip / later / install, the
   download's progress, then the restart ("skip this version" only mutes the silent
   check; a check the user asks for shows the version again). Downloading and
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

Inside a column the cards follow one order the user picks in the header (`CardOrder`
in `views/kanban.ts`). The default, "auto", is each column's own natural order and
has no direction: the task's date (Agenda's `whenOf`), soonest first, where a column
has one; and for the two columns that have no date to read — an unscheduled task has
none, and a finished task's due date says nothing about when it was finished — the
task's own change time, most recently changed first, the way a task manager's inbox
and completed list read (`byModified` in `views/tasks.ts`, reading
`Block.modified_ms` — see "Block change times"; Agenda's no-date and done groups
read the same rule). Checking one task moves that task alone.

Every other key — the date, the change time, the source order, the display
text, any inline field's value — applies to every column alike, ascending or
descending, so the label never hides a per-column special case; picking a key starts
it in its natural direction (newest first for the modification time, ascending for
the rest) and only then is there an arrow to flip it. A task in progress stays on top
under every key, a card without the key's value goes last whichever the direction
(the same convention as a query block's `sort`), and equal keys fall back to the
source order — path, then position — so a requery never shuffles cards that compare
equal. The order is persisted per vault together with the grouping and the collapsed
columns.

### The plan as state (update_plan), and the plan confirmation gate (v0.9)

The plan is **session state the model keeps through a tool**, never text it recites:
`update_plan` takes the whole step list each time (pending / doing / done / skipped),
the runner appends a `plan_update` line to the audit jsonl and emits `PlanUpdated`,
and the sticky plan card in the panel is a projection of the latest list — live from
the event, on replay from the lines. The system prompt forbids restating the list in
prose, so the regex that used to strip a recited "## Plan" out of every bubble is
gone with the token cost of the recitation. A plan whose every step is done or
skipped leaves the card with the next question; an unfinished one stays as a
reminder (the rule lives in `planReconciled`, applied by the live `turn_started`
and the replay's user line alike).

The `propose_plan` tool plus a blocking gate is the confirmation in front of that: a
multi-step task presents a plan card first and waits for "run it / edit / reject",
then continues in the same turn once approved — and the approval publishes the
confirmed list as the first `plan_update` (every step pending), so the two paths
share one projection. Like `web_search` it is a **session-frozen addition to the
tool table** — a change to the switch takes effect from the next session, because
the tool table freezes along with the system prompt when a session begins.

The plan phase is also where the thinking level bends. Measured on a real
turn (2026-09-08, deepseek-v4-flash at high): 96K characters of reasoning
went into the call that proposed the plan — the whole experiment designed and
its edits drafted in the model's head — and the user waited four minutes for a
three-line list. So in a plan-gated turn at high or max, the runner sends the
calls before the plan is settled at "low" (`light_effort`, the summary's level
too): reading and naming steps need no deep thinking, and the prompt now says
the design comes after the decision. A light response is kept only when every
call in it is a planning move (a read, a skill, the plan itself —
`planning_move`); one that answers, or goes to work without a plan, is a probe:
nothing of it enters the history or the UI (its text is withheld until the
response is known to be kept), an `effort` line records it in the session log,
and the same request is retaken at the session's level — so no answer and no
write is ever produced at the light level, and a turn that needed no plan
costs one short call more.

### Following a note (the working note)

A session can **follow** a note: the user's "keep your findings in this one"
(`agent_follow_set`, from the file tree's context menu, the palette, or automatically
when an answer is saved as a research note). Rust owns the state with the session
(`AgentState.follow`, a `follow` line in the jsonl — the last one wins on replay, and
a fresh ledger gets the line when it is created); the runner names the note in the
turn tail (`Working note: …`, privacy-filtered and scrubbed like the current note)
and, with `auto` on, its **local writes pass the gate without a dialog**: append_note,
set_frontmatter and edit_note on that one path (`is_local_write`), never a whole-note
overwrite, a restructure or a move, and never another note. The follow is the user's
explicit grant for one note, so the writes land as *unreviewed* blocks (the same
standing-grant rule as "allow for the session") and the editor's provenance bar is
the review afterwards — accept or revert per block. The permission line records
`via: follow`. The chip in the composer opens the note, toggles auto off (every write
asks again) and stops the follow. A note saved from the panel carries
`vemry.agent_session` in its frontmatter (now on `NoteSummary.agent_session`), and
opening it later offers to resume that session.

### The action-list gate (propose_actions)

The second gate of the plan gate's family. When the model has extracted a list of
things to write — action items from meeting notes, tasks out of an inbox, later the
moves of a tidy-up — it calls `propose_actions` once with the whole list (text,
owner, due as `YYYY-MM-DD`, target note, the source sentence) and the runner suspends
(`ActionsGate`, `ActionsRequest`/`ActionsDecision`, an `actions` line in the jsonl).
The card in the panel is the review: rows are unticked, dates, owners and targets
corrected (a quick task query marks rows that look like an existing task and unticks
them), and "write N items" hands the kept list back as the tool result — the model
then writes each item into its target through the ordinary write tools, which have
their own gate or the working note's grant. Discarding tells the model to write
nothing. The proposal itself touches no note: the dry run is free, and the only
things that land are the rows the user kept.

### Reading input: files, PDFs, and the read gate

`read_file` gives the agent the vault's non-note files as text — any UTF-8 file, and a
PDF through the same pdf-extract route an `@` attachment takes (`context.rs`,
privacy-gated by path, 30k cap). A PDF's extraction is **cached** under
`.vemry/cache/pdf/<hash of the bytes>.txt` (with a `.path` sidecar naming the file),
so a PDF read once costs nothing the next time, and `search_notes` searches the
cached texts as well — a PDF becomes searchable once it has been read (attached or
through read_file); until then `list_files` is the inventory. A private PDF's cache
entry is skipped and counted like any private read.

The **read gate** (`open_for_reading`) is the decision this design took over the
v0.5 stance of "the agent has no fetch tool": the agent's autonomous network
capability is still zero — it can only *propose* up to five URLs, and each fetch is
the user's click. The runner suspends at `ReadGate` (the plan and actions gates'
family; a `read` line in the jsonl), the card in the panel lists the URLs with
checkboxes, and the glue's `ChannelReadGate` fetches the allowed ones through the
reader window's extraction (`page_read`, the same pipeline a clip uses: no cookies,
the adblock rules, SSRF-checked images) and hands them back. Each page reaches the
model in the same untrusted-content envelope an attached page gets
(`prompt::page_envelope`), the page joins the session's source table as a verified
source with its title whether or not the answer cites it (the deliverable may be a
note, and the table is what `list_sources` and the sources workbench read), and the
ledger records URLs only — the tool_result line keeps the first line, never a page
body. The card's checkbox "from now on this session, read the pages it proposes
without asking" is the session's standing consent (`read_auto_sources`: cleared
when a session is left, and rebuilt from the `read_auto` line the runner writes
when it is given, so a reopened session keeps it): from then on the gate answers itself — `ReadGate::standing` is
asked before the card is raised, so the card lists the URLs with a "reading without
asking" badge and no buttons while they are fetched. The consent is not limited to
URLs already in the source table: the table is settled from citations at the end
of a turn and the server-side search returns no structured results, so every URL
proposed mid-turn would count as new and ask again, which made such a limit a
dead letter. The trade-off is stated rather than hidden: with the consent given, a
page the agent read could steer it into fetching a URL it composed, and what
remains is the card listing every URL as it is fetched, the stop button beside it,
and the consent ending with the session. A fetch's outcome — the pages read and
the ones that failed to load — comes back as the `read_decided` event, so the
card's badge reports what happened rather than what was clicked. Declining tells
the model to work from what it has.

### Side-by-side translation (v0.10, implemented)

Reading in another language is the one everyday task where the model is a tool
rather than a collaborator: the text goes out, the translation comes back, and
nothing about the vault changes. So the translation is a **projection, not
data** — `mainView = "translate"` is an overlay over the current view like the
reading projection (`vault.enterTranslate` / `exitTranslate`; it records nothing
in navStack, since it is a way of looking at the current document rather than a
place), its content lives in the translate store, and it becomes a note only when
the user presses "save as note".

- **One document model.** A source is cut into segments (`vemry_core::segment`):
  the parser's blocks sorted into what the model translates and what is copied
  through, each with its structural marker split off into `prefix`. The view is a
  grid of rows, one per segment, the source on the left and the translation on the
  right — a row takes the taller side's height, so the columns stay aligned by
  construction and nothing synchronises scrolling. A pane narrower than 880px
  interleaves the rows (translation under source) on its own; the reader can also
  choose interleaved or translation-only. Both columns go through
  `renderNoteMarkdown`, so a wikilink, a callout, a formula or a local image looks
  the same on either side.
- **The engine is not the agent.** `vemry_agent::translate::TranslateJob` makes the
  same tool-free single call the summary makes, through the same transport, with
  thinking off; batches of about 2k characters go out as
  `<s id=N>…</s>` and come back the same way, so alignment is by id and a segment
  the model drops is resent alone once and then reported as failed (the provider's
  message as-is). `StreamParser` closes a segment the moment its `</s>` arrives,
  which is what fills the right column while the batch is still streaming. Two
  batches are in flight at once, and the job's first batch is cut to about 600
  characters so the first text lands in a couple of seconds.
- **Where it reads, then ahead, then a budget.** The job keeps two queues. What the
  reader asked for comes first and is never limited: the view reports the rows in
  a window of a screen behind and two ahead (an IntersectionObserver, batched at
  150ms), the browser pane a screen behind and two and a half ahead (a page scrolls
  faster), and `translate_request` moves them to the front. The rest of the
  document is background work in order, bounded by a budget of about 60k
  characters of source text: a paper or a long article finishes on its own, a book
  or a thread of five hundred comments stops there and says so — the row where it
  stopped offers "continue" (another budget) and "translate everything"
  (`translate_continue`), and rows beyond it show a still placeholder until they
  come into view. So short and long documents follow one rule with two outcomes,
  rather than two modes. Saving translates what is left first (`translate_save`
  lifts the budget and waits for idle), so the note is complete.
- **What leaves the app.** A note in the privacy zone is refused whole
  (`translate_open` says so); a `> [!private]` block in a visible note is marked by
  the segmenter, never enqueued by the job, shown on the left and "not translated"
  on the right, and left out of a saved translation without a placeholder (a
  placeholder would say there is something to hide). The settings page's
  translation section carries the disclosure that the text is sent to the model.
  The agent sees none of it: no context reference is registered for the view, and
  a translation the agent should read is a saved note.
- **The cache.** `.vemry/cache/translate/<hash of the note path>.<target>.json`
  maps the hash of each segment's text to its translation, no source text in the
  file; an edited paragraph misses on its own and everything else hits, so a
  document is paid for once and reopening it is instant. Retranslate (↻ on a row)
  bypasses it. The cache is a projection like the block times: deleting the
  directory costs a re-translation and nothing else.
- **Saving.** `translate_save` composes the translations in document order (a
  copied-through block as it is, a segment not translated yet as its source),
  files it as `<title> (中译).md` beside the source with `translation_of:
  "[[source]]"`, `lang`, `translator` (the model) and `vemry.origin: translation`,
  formatted like a clip, through the same `write_note` an ordinary note takes — the
  user's action, no gate.
- **A web page in the reading projection.** The projection's page is a source too
  (`TranslateSource::Page`): the segments are cut from the Markdown the reader
  window's cache holds (the same text the projection renders; a page that has left
  the 32-page cache says so and asks to be reopened), the overlay comes up over the
  projection and Esc lands back on it. A page's translations are **never written to
  disk** — the projection's content is not, and neither is its translation: the
  cache is an in-memory `TranslationCache` parked in `TranslateState.web_caches`
  (32 pages, like the reader's) when the overlay closes and taken back when the
  same page and language open again. Saving clips the original first (or reuses
  the clip the page already has) and files the translation beside it in `clips/`,
  with the same `source:` and `domain:` and `translation_of` pointing at the clip.
- **A browser tab, where it is.** The browser panel's "translate" button does not
  leave the tab: a pane opens *inside the page* — the page on the left, every
  text block's translation on the right at the height of its block — docked or
  detached alike, so the reader's eyes stay where they were (the first version
  opened the overlay in the main window, and the jump was the complaint).
  `browser/pagetr.rs`: the tab is a native webview with zero IPC, so everything
  goes through `eval` — `COLLECT_JS` installs the pane (a fixed element on the
  right; the document gets a matching right margin so a page that reflows does;
  once the page has reflowed the pane's width is settled from the median block
  width — an entry gets about three quarters of its block's width, which is what
  a CJK translation of an English paragraph needs to end near where the
  paragraph ends — and an entry is set a touch smaller and tighter than its
  block) and tags the page's text blocks (a block with running text and no block
  children, outside `nav`/`header`/`footer`/`aside`, captions, tables, reference
  lists and bylines, not under the pane — a page's right-hand column and its
  promoted items are not what is being read — and not already in the target
  language: for a CJK target by the text's own script, for any target by a lang
  attribute below the document root, since the root's lang is the interface's
  and says nothing about a quoted post) with ids, the same `TranslateJob` translates them, and each
  finished segment is written into its pane entry with `textContent`. The pane
  lays its entries out from the blocks' positions on every scroll and resize: a
  forward pass puts each entry at its block or under the one above, and a
  backward pass lifts an entry that got pushed by letting the one above it rise
  into the slack over its own block (by at most three tenths of that block's
  height), so a long translation's overflow is shared with its neighbour rather
  than accumulated down the page; a placeholder is as tall as its block, so a
  translation landing above the reader does not push the rows below it; a poll
  every second collects blocks that appeared since (an endless feed; the job's
  segments are growable, `TranslateJob::extend`); moving to another page — a
  navigation, a reload, a single-page app's route change (the poll looks first,
  without touching the page: the pane's document token is gone, or the URL's
  path moved; an anchor jump stays) — ends the translation rather than carrying
  it over, and the next page is translated only when asked. The pane takes its
  colours from the first block's ancestors, not the body (an app shell often
  paints a dark body under a light column). Clicking an entry scrolls the page to its
  block. The rows the reader is looking at come first: every poll reports the
  untranslated blocks in and near the viewport and the job moves them to the
  front of its queue; the rest is the same budgeted background work, and the
  budget's stop is an entry in the pane that offers to go on. Until a translation
  lands, an entry is a placeholder of as many shimmering lines as its block has
  (still, beyond the window), the text streams into it as it arrives, and a
  progress line runs along the top of the pane. Picking an entry, or clicking its block in the page, keeps the
  block lit on the page and the entry lit in the pane until another is picked
  (the same lit row the PDF view has), scrolling only when the block is off
  screen. A rendered formula (MathML, KaTeX, MathJax) travels as a placeholder
  (`⟦m0⟧`, which the prompt says to keep) and comes back as a clone of the
  page's own rendering in the translation. Only what the reader can type into
  counts as editable and is skipped — a read-only editor (`contenteditable="false"`,
  how a long-form article is rendered) is body text. The chrome hears
  `BrowserEvent::Translate` (on/off, done/total) and the button shows it;
  pressing again removes the pane. The translations are cached in memory by URL
  and language across a close and a reopen, never on disk.
- **What is not body text.** A caption (`Figure 3:`, `表 1`), a reference entry
  (`[12] …`, a numbered author-year line), everything under a heading named
  References / Bibliography / Acknowledgements, a paper's author and address
  lines between the title and the abstract, an e-mail, an arXiv stamp, a footnote
  opened by a marker, a row of figures — `SegKind::Aside`: shown dimmed on the
  left, never sent, no cell on the right, copied as it is into a saved
  translation. The browser pane's collector skips the same things by DOM shape.
- **A PDF.** The file's text comes page by page from the same extraction and cache
  the agent's `read_file` uses (`context.rs::pdf_pages`), and
  `vemry_core::segment_pages` rebuilds paragraphs from what pdf-extract gives —
  lines, not paragraphs: on a justified page every full line is padded with double
  spaces and a paragraph's last line is not, so an unpadded line that ends a
  sentence ends the paragraph; elsewhere a line clearly shorter than the column
  does; a short line standing alone without a full stop is a heading, a hyphen
  broken across lines is mended, a bare page number is dropped. The rows are
  **pages**: the page drawn on the left (`PdfPage`, the same pdf.js document the
  preview parsed), its paragraphs as cells on the right, each cell a segment with
  the same states as a row (the page column three fifths wide — the page is small
  type, its translation is not — and the rows interleave below 1000px rather than
  880px), **each cell at the height of its paragraph on the page**: `segmentTops` finds where each segment's text starts among pdf.js's text
  items (one text-content read per page), and a cell gets the top margin that
  puts it there, or sits right under the cell above when that one is taller than
  its paragraph — cells never overlap, and the eye crosses the gutter horizontally.
  A file in a privacy zone is refused whole; the cache
  is filed by the bytes' hash, like the extraction. Saving groups the translations
  under a heading per page ("第 N 页" / "Page N", in the target language) beside
  the file, `translation_of` naming it. Clicking a translated paragraph lights its
  passage on the page: `textRects` looks the segment's text up among pdf.js's text
  items (whitespace ignored on both sides; the head and the tail when the whole
  run is not found as one) and the boxes are laid over the canvas as fractions of
  the page.
- **What it costs is shown.** The job sums the provider's usage counts from each
  stream's tail (`TranslateEvent::Usage`; DeepSeek sends them when asked, the
  same `include_usage` the agent uses), and the view's model chip and the
  browser button's hint carry the tokens spent on the document so far — the
  reader can see what the background work costs, and the budget stays a number
  rather than a setting.
- **Thinking off.** A translation is transcription, not reasoning: on a thinking
  model the thinking is most of the latency (measured on a 2.4k-character page:
  51s at the light level, 6s with thinking disabled), so the request disables it
  where the level is sent at all (a DeepSeek endpoint) and sends nothing elsewhere.
- **Entry points.** ⌘⇧T (`translate-toggle`, the same key leaves; over the reading
  projection it translates the page, else the open note), the View menu,
  the editor's context menu, and a "translate" chip in the status bar that appears
  when the open note reads as written in another script than the target language's
  (`langdetect.ts`: a ratio of CJK to Latin letters over 300 letters, computed
  locally — nothing is sent to decide it). Double-clicking a source block leaves
  the overlay and lands on that block in the editor (`openBlockAt` with the
  segment's byte offset); J/K step the lit row.
- **Settings.** `translate_target` (empty = the interface language; the header's
  language menu writes it and reopens the job), `translate_model` (empty = the chat
  model; the endpoint and key are always the chat's — a fast, cheap model is
  enough here) and `translate_addendum` (one line of the user's own rule appended
  to the prompt).

### Product documents and iterations: blocks and fields, not a system

A PRD, a decision, a piece of feedback and an iteration are all **typed blocks and
fields on ordinary Markdown**, so the existing projections are the product views:
the board grouped by `status` is the requirements board, grouped by `iteration` the
iteration board; the Claims view is the decision log, because the parser indexes a
`> [!decision]` callout **as a claim** — evidence supports it with `supports:` like
any claim, `audit_links` names decisions whose supports reach nothing, and the view
badges it. Four example block types install from Settings (`type_examples_list` /
`type_install_example`, the demo vault's `.vemry/types/*.md` as the one copy):
requirement (`priority / status / owner`), decision (`date / supersedes`), question
(`owner / status`) and hypothesis — a **task** whose due date is its review date,
so the Agenda surfaces "review this hypothesis" when it falls due, with `metric /
expected / baseline / result / status` on the line.

`aggregate_blocks` counts instead of listing (per field value, per note, per ISO
week of last change) — query_blocks is capped at 200 rows and cannot count, and a
retro or a feedback triage needs the totals. `propose_actions` items carry free
`fields` the card edits in place (a scoring's reach / impact / confidence /
effort), which the model then writes with `set_block_field` — the scoring card is
the action card. Six skills carry the method: prd, decision-log, feedback-triage,
iteration-plan, retro, brief (an implementation brief exported for a coding agent
as plain text, and the record filled back in from its summary — two pastes by the
user; vemry never runs the coding agent, and automatic dispatch stays a separate
design).

### Learning: cards, the Review view, and the tutor

A flashcard is a `> [!card]` callout tagged `#card` — the parser indexes it as a
paragraph with a tag and inline fields, nothing was added to the grammar. The first
line is the question, the quoted lines below are the answer, and the schedule
(`due / interval / ease / reviews`) lives in fields on the callout's last line. The
Review view (`ReviewView.svelte`, a projection in the tab system like the Agenda) is
one `query_blocks({tag: card})` over the vault: it shows the cards due today or never
graded one at a time, flips on Space, and takes one of four grades (`1`–`4`) with the
resulting interval previewed on each button. A grade is a **user field write** on the
block's span (`views/cards.ts`: read the note, check the callout's content against the
index as tasks.ts does, rewrite exactly the span, requery when stale) — the note is the
deck, there is no study database, and the agent's gate is not involved because the
agent did not write it. The scheduling rule is the two-factor spaced-repetition one
(interval grows by the ease; Again resets to today and the card returns at the end of
the session; Hard ×1.2, Good 1 → 3 → ×ease, Easy ×ease ×1.3), measured from the day of
the review so a late review is not penalised and studying ahead counts. A card graded
in a note the view has written this session may carry a stale span: the view requeries
before grading it.

The `card` example type installs from Settings (`.vemry/types/card.md`), the `tutor`
skill carries the method — explain from what the notes already say and end with one
check question; make 5–12 one-fact cards per note into `cards/<topic>.md` (the working
note when following), each answer citing its source block, `due` today and no other
schedule field; quiz from the due cards one per message without writing grades (chat
is practice, the Review view is the review); a knowledge check counts decks with
`aggregate_blocks` and lists the cards whose `ease` keeps falling. The
`explain-selection` command sends the editor's selection as a question at once, the
passage attached as the same selection reference the composer's automatic chip sends.
The view registers itself as a view reference (`view:review`), so "Ask AI" on its
header attaches the card table.

### Restructuring a note by its blocks (restructure.rs)

A whole-note overwrite of a note holding a private block is refused by the gate
(the model never saw the block, so its "full content" would delete it), and even
without one it retypes every block. So a reorganisation is a **block-level
operation**: `vemry_core::restructure` cuts a note into units (a heading, a
paragraph, a list item with its subtree, a code block…) and moves them byte for
byte, under three invariants — a **private block is an anchor** (it travels with the
public unit it follows, is never moved on its own or into another note, and survives
every removal), **ids travel with their blocks** (`^id` stays on its unit; a
cross-note move rewrites the vault's `[[from#^id]]` to `[[to#^id]]` through
`plan_block_move_links`), and **nothing disappears silently** (a unit the outline
does not name lands under "## Unassigned" and the tool result says so; a removal
must be listed). `restructure_note(path, outline, removed)` rearranges one note
under an outline (an existing heading is reused, id included; a new one is
written); `move_blocks(from, blocks, to, anchor, leave_link)` reorders within a note
or moves into another note's section, the destination and the link rewrites landing
as side writes of the same gated request. Seams follow the formatter's rule, so a
restructured note is already in format-note shape.

Both requests carry a **block-level diff** (`block_diff`: blocks matched by hash —
kept, moved when out of the common order, added, removed, and a removed/added pair
sharing most of its words as one changed block), and the permission dialog opens on
it: "moved 12 · changed 3 · added 2 · removed 0", the removed group open, the moved
group folded, the raw line diff one click away. `audit_links` gained a dangling
anchors section (`[[note#heading]]` / `[[note#^id]]` whose note exists but whose
anchor does not). The tidy-note skill: read whole, propose the outline in prose,
restructure once, wording fixes afterwards as separate edits.

### The Markdown formatter (format.rs)

Format is code's business, not the model's. `vemry_core::format_markdown` is a
deterministic, idempotent pass that normalises how a note is written without
changing what it says: one blank line between blocks and none between the items
of a list (the editor's seam rule), trailing whitespace off (a hard break's two
spaces kept), one newline at the end, `-` for every bullet and ordered lists
renumbered, `# ` with one space, `[X]` → `[x]`, `[!Note]` → `[!note]`, one space
before a trailing ` ^id`, table columns padded, a bare URL with a `#` wrapped in
`<…>` (a fragment reads as a tag otherwise), frontmatter keys in the order title /
tags / aliases / created with the rest as written. It leaves fenced code, `$$`
math, `> [!private]` and `> [!quote]` blocks verbatim, never touches a frontmatter
value, and never changes a character of prose unless an opt-in asks (CJK spacing,
full-width punctuation — they move the block hashes behind provenance and change
times, so they are off by default; `#word` at a line start is a tag and is never
turned into a heading).

Three entry points, all explicit: the **format-note command** runs it over the
whole open note as one undoable transaction; the **agent's whole writes** — a new
note's content, an appended section — go through it in `prepare_write` before the
gate shows the diff (`RunnerConfig.format`; an edit fragment, a task line or a
field is left as written, so an agent write never reformats the user's own lines);
a **clip note** and a **rich-text paste** (the clipboard's HTML through the clip
layer's converter, `html_to_markdown`, with `paste-plain-text` as the way around
it) are formatted when the clip formatter is on. It does not run on autosave: a
user's own text changes only when they ask. Settings: `format_agent_writes`,
`format_clips` (both on), `format_cjk_spacing`, `format_punctuation` (both off).

### Moving with links, and undoing a turn

A tidy-up is the write path most likely to lose something, so two rules hold it:

- **A move rewrites the links that pointed at it, in the same write.** `move_note`
  respells every `[[wikilink]]` that resolved to the moved note (the author's form
  kept: a bare name stays bare while it still resolves uniquely to the moved note,
  a path stays a path, headings, block refs and aliases carried along; the moved
  note's own self-links included); `move_file` rewrites embeds and Markdown links
  (`![[img.png]]` by bare name only when that name was unique). The plan is
  computed at prepare time (`Vault::plan_note_move_links` / `plan_file_move_links`)
  and shown in the permission dialog as the move's diff with a rewrite count; at
  execute time the rename lands first and each rewrite second, and a note that
  changed meanwhile is skipped and reported — a stranger's edit is never
  overwritten, a stale link is the lesser evil. Private notes are never rewritten
  (the model does not know they exist), so a link inside a private note goes stale
  for the user to fix.
- **A turn's writes can be undone.** Every write that passes the gate is journaled
  (`undo.rs`) with what it replaced, and the action bar of a turn that wrote offers
  "undo this turn's writes": records are walked backwards, reverted where the note
  still holds what the write left, skipped (and counted) where it was edited since;
  a created note goes to the trash, never straight to deletion. The journal is
  in memory with the session — after a restart the button is gone and the
  provenance bar's per-block revert remains.

The tidy skill puts the two together with the action-list gate: audit, propose every
move as a list the user reviews, execute only what came back, audit again.

### The view reference (@ the Agenda, @ the board)

The sixth kind of `@` reference: what a projection view shows right now. The view
registers its live query and row count (`stores/viewContext`) and the @ picker's
"current view" candidate or the header's "Ask AI" button attaches it. Rust re-runs the
query through `query_for_agent` — the same privacy-filtered query API the view and
the `query_blocks` tool use — and renders one block per line with its citation
handle, task state and dates (`<view-context>`). It is not a new read path: the
user's table, made citable, minus the private blocks. The audit records `view:agenda`.

### The source table (sources.rs)

Every web citation the model produces in a session lives in one table on the Rust
side, owned by `AgentState` with the session (rebuilt from the jsonl wherever
`current_session` changes: select, fork, delete, a vault switch). At the end of a
turn the runner extracts the citations from the turn's assistant text (plus any
structured search results), settles them into the table with a verdict — verified
when a search ran that turn or the user handed the page over, otherwise
**unverified** — appends a `sources` line and emits `SourcesUpdated`. The frontend
card for the turn takes that list as authoritative (`settled`); its own link
harvest remains only as the fallback for a turn that ended without a settlement.
A regenerate or an edit-and-resend drops the abandoned turn's sources
(`drop_turn`), and the replay applies the same rule from the marker lines. The
model can read the table back with `list_sources`, and `clips_lookup` tells it which
of those URLs already have a clipped note to cite as a wikilink instead. The frontend
mirrors the table (`agent.sourceTable`, the union of the turn settlements) for the
sources workbench in the panel's session bar: every source of the session, numbered,
with its verdict — verified, unverified, or clipped — and the three actions a source
takes (open, clip / open the clip, copy the link), shared with the per-turn cards
through `lib/sources.ts`.

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
window, the standalone chat window and the detached browser window's chrome bar —
each holding its own
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
