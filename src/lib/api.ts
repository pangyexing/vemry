// The single typed entry point for all IPC. Mirrors src-tauri/src/commands.rs.
import { invoke, Channel } from "@tauri-apps/api/core";
import { emit, emitTo, listen, type UnlistenFn } from "@tauri-apps/api/event";

// ---- types mirrored from Rust ----

export interface NoteSummary {
  id: string;
  title: string;
  aliases: string[];
  tags: string[];
  mtime_ms: number;
  /** frontmatter `created` as written (free-form date string) */
  created: string | null;
  /** frontmatter `private: true` — in the privacy zone, invisible to the AI */
  private: boolean;
  /** frontmatter `source:` — external origin URL of a clipped note */
  source: string | null;
}

export interface GraphNode {
  id: string;
  title: string;
}

export interface GraphEdge {
  source: string;
  target: string;
  /** resolved links source -> target, parallel links aggregated */
  count: number;
}

export interface GraphData {
  nodes: GraphNode[];
  edges: GraphEdge[];
}

export interface VaultInfo {
  root: string;
  note_count: number;
}

export interface SearchHit {
  note: string;
  title: string;
  snippet: string;
  line: number;
  score: number;
}

export type BlockKind =
  | { type: "paragraph" }
  | { type: "heading"; level: number }
  | { type: "list_item"; indent: number; ordered: boolean }
  | { type: "task"; state: "todo" | "doing" | "done"; due?: string; scheduled?: string }
  | { type: "claim" }
  | { type: "evidence"; supports: string[] }
  | { type: "code_block"; lang: string }
  | { type: "private" };

/** An inline `key:: value` field (mirrors vemry-core's `Field`); `span` covers the whole occurrence in bytes. */
export interface Field {
  key: string;
  value: string;
  span: { start: number; end: number };
}

export interface Block {
  local_id: string;
  kind: BlockKind;
  content: string;
  span: { start: number; end: number };
  tags?: string[];
  fields?: Field[];
  explicit_id?: boolean;
  /** When the block's current text appeared: its note's mtime (ms) at the first indexing that saw it — the nearest thing a line has to a modification time of its own (vemry-core's blocktimes.rs) */
  modified_ms?: number;
}

/** One inline field a block type carries (mirrors vemry-core's `FieldSpec`). */
export interface FieldSpec {
  key: string;
  default?: string;
  values?: string[];
  /** `date` / `number` / `text` (a hint). */
  type?: string;
}

/** What a scaffold command (`typeCreate`, `initResearchSkill`) hands back (mirrors commands.rs
 *  `Scaffolded`): the file's vault-relative path, and whether it was written just now —
 *  `false` means it was already there and was left untouched. */
export interface Scaffolded {
  path: string;
  created: boolean;
}

/** A block type from `.vemry/types/<file>` (mirrors vemry-core's `BlockType`). */
export interface BlockType {
  file: string;
  name: string;
  description: string;
  /** A turn-into target id (`paragraph`, `bullet`, `task`, …). */
  kind: string;
  callout?: string;
  tag?: string;
  icon?: string;
  fields: FieldSpec[];
  /** `{{key}}` = a field's default, `{{date}}` = today, `{{cursor}}` = where the caret lands. */
  template: string;
  disabled: boolean;
}

/** One inline-field key across the vault (mirrors vemry-core's `FieldStat`). */
export interface FieldStat {
  key: string;
  /** Notes carrying the key. */
  count: number;
  /** The most frequent values, most frequent first. */
  values: string[];
}

export type FieldOp = "eq" | "ne" | "has" | "lt" | "gt" | "contains";

/** One inline-field condition; the flat string form (`author = X`, `due < 2026-09-10`, `rating`) is parsed in Rust. */
export interface FieldFilter {
  key: string;
  op: FieldOp;
  value?: string;
}

/** The attachment-bucket migration report (mirroring vemry-core's attach_migrate::MigrationReport). */
export interface MigrationReport {
  /** [old bucket, new bucket] relative path pairs */
  moved: [string, string][];
  rewritten_notes: number;
  /** A mixed bucket, whose stem several notes share: kept in place */
  mixed: string[];
  /** An orphan bucket with no owning note: kept in place */
  orphaned: string[];
  /** The new bucket is already taken: skipped */
  conflicts: string[];
}

export interface BlockHit {
  note: string;
  note_title: string;
  block: Block;
}

export interface BlockQuery {
  kind?: "paragraph" | "heading" | "list_item" | "task" | "claim" | "evidence" | "code_block" | "private";
  /** Exact vault-relative path of one note (e.g. `projects/Alpha.md`), not a substring. */
  note?: string;
  tag?: string;
  text?: string;
  task_state?: "todo" | "doing" | "done";
  /** `YYYY-MM-DD` — tasks due on or before this date (mirrors Rust `NaiveDate`). */
  due_before?: string;
  /** Field conditions (all must hold): flat strings, typed objects, or a list of either. */
  field?: string | FieldFilter | (string | FieldFilter)[];
  limit?: number;
}

/** What a wikilink fragment points at (mirrors vemry-core's `RefTarget`). */
export type RefTarget =
  | { kind: "note" }
  | { kind: "heading"; text: string; level: number }
  | { kind: "block"; local_id: string; block_kind: BlockKind };

/** A resolved reference: `span` is a byte range like `Block.span`; `content` is the raw slice it denotes. */
export interface RefHit {
  note: string;
  note_title: string;
  target: RefTarget;
  span: { start: number; end: number };
  content: string;
}

export type RefResolution =
  | ({ state: "resolved" } & RefHit)
  | { state: "note_missing" }
  | { state: "anchor_missing"; note: string; note_title: string };

/** Which block to give an id: a query hit's local id (with its content as a guard) or a byte offset. */
export type BlockSelector = { by: "id"; local_id: string; guard?: string } | { by: "offset"; offset: number };

/** The single insert that gives a block its id (`offset` in bytes); absent when nothing needs writing. */
export interface BlockIdPlan {
  target: { kind: "heading"; text: string } | { kind: "block"; id: string };
  insert?: { offset: number; text: string };
}

/** Who authored a block's current text (mirrors vemry-core's provenance `Origin`). */
export type Origin =
  | { origin: "you" }
  | { origin: "agent"; session?: string; ts?: string; pending: boolean }
  | { origin: "clip"; url?: string; ts?: string }
  | { origin: "capture"; ts?: string };

export interface BlockProvenance {
  local_id: string;
  /** Byte range like `Block.span`. */
  span: { start: number; end: number };
  /** For an unreviewed block: the part of it the write added, when the ledger's hunk can be found (a block the model extended keeps its reviewed lines out of the mark). */
  changed?: { start: number; end: number };
  hash: string;
  origin: Origin;
}

export interface PendingNote {
  path: string;
  count: number;
}

/** How to undo the agent writes behind unreviewed blocks (mirrors vemry-core's `RevertPlan`):
 *  byte ranges of the text the plan was made for. */
export interface RevertPlan {
  replacements: { from: number; to: number; insert: string }[];
  /** Unreviewed blocks whose write could not be located (a ledger line older than the hunks, or surroundings that moved on). */
  missed: number;
}

export interface BacklinkEntry {
  source: string;
  source_title: string;
  snippet: string;
}

export interface NoteLinks {
  outgoing: { raw_target: string; resolved: string | null }[];
  backlinks: BacklinkEntry[];
}

export type LinkOpenMode = "reading" | "internal" | "system";

/** The interface language. Matches Rust's settings::LangSetting exactly. */
export type LangSetting = "auto" | "zh-CN" | "en";

export interface SettingsView {
  base_url: string;
  model: string;
  api_key: string;
  env_key_present: boolean;
  recent_vaults: string[];
  /** The interface language; auto = follow the system. */
  language: LangSetting;
  adblock_enabled: boolean;
  adblock_rule_urls: string[];
  open_links_in: LinkOpenMode;
  click_opens_links: boolean;
  /** A read-only mirror: the shape a browser window is built in (detached or docked).
   *  The write path is browserSetDocked (every toggle is persisted = remember the last
   *  choice), which does not go through SettingsUpdate and has no switch in the
   *  settings page. */
  browser_docked: boolean;
  web_search_enabled: boolean;
  /** Derived and read-only: the switch is on and the endpoint is DeepSeek (decided on the Rust side; do not reimplement it in the frontend). */
  web_search_active: boolean;
  /** The agent's plan confirmation (propose_plan plus a blocking gate); frozen per session, so a change takes effect from the next session. */
  plan_confirm: boolean;
  /** AI blocks awaiting review show an amber bar in the editor. */
  provenance_marks: boolean;
  reading_images_enabled: boolean;
  clip_localize_images: boolean;
  updater_auto_check: boolean;
  note_font_size: number;
  readable_line_width: boolean;
  /** The thinking depth level: none/low/high/max (DeepSeek's official default is high). */
  reasoning_effort: string;
  /** Derived and read-only: effort is only sent when the endpoint is DeepSeek (decided on the Rust side; do not reimplement it in the frontend). */
  reasoning_effort_active: boolean;
  /** Derived and read-only: the endpoint is DeepSeek (which is known to have no vision model; the composer warns once when an image is attached). */
  provider_deepseek: boolean;
}

/** A partial update: a field left out keeps its value. */
export interface SettingsUpdate {
  base_url?: string;
  language?: LangSetting;
  model?: string;
  api_key?: string;
  adblock_enabled?: boolean;
  adblock_rule_urls?: string[];
  open_links_in?: LinkOpenMode;
  click_opens_links?: boolean;
  web_search_enabled?: boolean;
  plan_confirm?: boolean;
  provenance_marks?: boolean;
  reading_images_enabled?: boolean;
  clip_localize_images?: boolean;
  updater_auto_check?: boolean;
  note_font_size?: number;
  readable_line_width?: boolean;
  reasoning_effort?: string;
}

export interface WriteRequest {
  id: string;
  tool: string;
  path: string;
  /** move_note's destination (absent for every other tool; the Rust side does not serialise the key when it is None). */
  to_path?: string;
  diff: string;
  /** The kind of write; the wording lives in the catalog
   *  (`permission.summary.<kind>`) — the Rust side does not know the user's language,
   *  so it sends the kind and never the wording. */
  kind: "create" | "overwrite" | "edit" | "move";
  new_content: string;
}

export interface WebSearchResult {
  title: string;
  url: string;
  snippet: string;
  publish_date: string | null;
}

export type AgentEvent =
  /** A new user turn begins: turn is the turn id (the same one on the audit user line
   *  and the model history's envelope), the timeline tags its user item with it, and
   *  forking locates by it. A regenerate does not emit it (it produces no new turn). */
  | { type: "turn_started"; turn: string }
  /** A write tool landed: the same vault events the watcher would have sent (the app also broadcasts them). */
  | { type: "vault_changed"; events: VaultEvent[] }
  | { type: "text_delta"; text: string }
  | { type: "thinking"; text: string }
  | { type: "tool_call_started"; id: string; name: string; args: string }
  | { type: "tool_call_finished"; id: string; name: string; ok: boolean; output: string }
  | { type: "permission_request"; request: WriteRequest }
  /** propose_plan is suspended awaiting the user's decision (the plan gate, of the same family as the permission gate). */
  | { type: "plan_request"; request: PlanRequest }
  | { type: "web_searching" }
  | { type: "web_search_queries"; queries: string[] }
  | { type: "web_search_results"; results: WebSearchResult[] }
  /** Per-iteration token counts (only when the provider reports them): they drive the panel's gauge. */
  | {
      type: "usage";
      prompt_tokens: number;
      completion_tokens: number;
      cache_hit_tokens: number;
      iteration: number;
      max_iterations: number;
    }
  /** The context was compacted: 1 = old tool results turned into handles, 2 = an LLM summary. */
  | { type: "compacted"; level: number }
  /** truncated: the output was cut off by the length limit (so "continue" can be offered). */
  | { type: "turn_finished"; truncated: boolean }
  | { type: "error"; message: string };

/** A session row in the research panel's list (mirroring vemry_agent::session::SessionMeta). */
export interface SessionMeta {
  id: string;
  /** The first line of the first user message, cut to 80 characters. */
  title: string;
  /** How many user messages there are. */
  turns: number;
  /** The RFC3339 timestamp of the last audit line; it may be an empty string. */
  last_ts: string;
  /** Having a history sidecar means the model context can be resumed; false ones are the per-turn fragments from before the upgrade. */
  resumable: boolean;
  /** A forked session (its title matches the source's — the first user message is the same, and the list UI tells them apart by the ⑂). */
  forked: boolean;
}

export type PermissionDecision =
  | { kind: "allow" }
  | { kind: "allow_session" }
  | { kind: "allow_always" }
  | { kind: "deny"; reason: string }
  | { kind: "allow_edited"; content: string };

/** The plan gate's (propose_plan) request and decision (mirroring vemry_agent::permission). */
export interface PlanRequest {
  /** The tool call id: the decision is paired back by it. */
  id: string;
  items: string[];
}

export type PlanDecision =
  | { kind: "approve" }
  | { kind: "approve_edited"; items: string[] }
  | { kind: "reject"; reason: string };

/** A row of the skills management list (mirroring commands.rs::SkillEntry; disabled ones included). */
export interface SkillEntry {
  file: string;
  name: string;
  description: string;
  disabled: boolean;
}

export type VaultEvent =
  | { type: "note_added"; id: string }
  | { type: "note_modified"; id: string }
  | { type: "note_removed"; id: string }
  | { type: "index_ready"; note_count: number }
  | { type: "files_changed" }
  | { type: "privacy_changed" }
  | { type: "scripts_changed" }
  | { type: "types_changed" };

// ---- the built-in knowledge browser (v0.4) ----

export interface BrowserWindowView {
  label: string;
  url: string;
  title: string;
  /** Snapped to the main window's right edge and following it (on macOS attached as a native child window); false after pressing "detach". */
  docked: boolean;
  /** How much of the main window's right edge it covers while docked (logical px, 0 =
   *  side by side with no overlap). It is > 0 when the main window is full-screen or
   *  the window is pinned to the screen's right edge, and App's layout gives way by
   *  that much (a merged split view). */
  inset: number;
}

export interface BrowserSnapshot {
  windows: BrowserWindowView[];
  active: string | null;
}

export interface ClipSelectionResult {
  /** The quote block's text (with save_to_clips=false the frontend inserts it into the current note). */
  text: string;
  /** The source page's URL: clipLocalizeNote is called with it once the insertion is done, to localise the images. */
  url: string;
  saved_path: string | null;
}

export interface ClipLinkResult {
  text: string;
  url: string;
  title: string;
}

/** The page body "add to context" extracted (browser_page_context). */
export interface PageContext {
  title: string;
  url: string;
  markdown: string;
  truncated: boolean;
}

/** An action from the detached chat window to the main window (run by host.ts; the editor, the clip destination and the command table live in the main window alone). */
export type HostAction =
  | { kind: "open-note"; path: string }
  /** guard: for a positional citation, the block's content when it was cited (a moved block is still found). */
  | { kind: "follow-link"; target: string; guard?: string }
  /** session: the chat session the text came from (recorded as the blocks' provenance). */
  | { kind: "insert"; payload: string; session?: string }
  | { kind: "pin"; payload: string; session?: string }
  | { kind: "memo"; text: string }
  | { kind: "command"; id: string };
/** An action from the clip toolbar (clipbar-N, batch D) to the main window (App.svelte
 *  listens and runs it; the clip flow, clipTarget and the editor buffer all live in the
 *  main window alone). The three clip actions carry a label = the browser window the
 *  toolbar is paired with, so with several windows open nothing is guessed from
 *  "focused last". */
export type ClipbarAction =
  | { kind: "clip-selection"; label: string }
  | { kind: "clip-page"; label: string }
  | { kind: "clip-link"; label: string }
  | { kind: "set-clip-target"; path: string; heading: string | null }
  | { kind: "clear-clip-target" }
  | { kind: "open-note"; path: string }
  /** Summon the detached AI chat window and bring the current page into its context
   *  automatically (the window opens first for immediate feedback, and the extracted
   *  page is attached afterwards through attach-page; it does not go through
   *  focus-agent — that branch would pull the user back to the main window's panel,
   *  while summoning from the browser always wants the floating chat window). */
  | { kind: "summon-agent"; label: string };
/** The clip receipt from the main window back to the toolbar (emitTo'd to
 *  clipbar://result): a toast in the main window is invisible to a toolbar user, so the
 *  receipt flashes on the bar itself. */
export interface ClipbarResult {
  ok: boolean;
  message: string;
}

/** A command from the main window to the detached chat window. send: a main-window
 *  command (drafting VEMRY.md, say) has to send a message while the conversation has
 *  moved to the detached window, so the text goes over with the main window's current
 *  note and is sent there; memo-result: the receipt after a `#` quick memory was
 *  relayed and written in the main window (with ok=false the chat window gives the
 *  input back to the user). */
export type AgentWindowCommand =
  | { kind: "new-session" }
  | { kind: "send"; text: string; currentNote: string | null }
  | { kind: "memo-result"; ok: boolean; error?: string }
  /** The page context attached after the toolbar's "summon the AI" (the window is
   *  already open, so the handoff package cannot reach it); page=null means the
   *  extraction failed — the error goes into the chat window too, since that is where
   *  the user is looking. */
  | { kind: "attach-page"; page: PageContext | null; error?: string };

/** An @ context reference: only the path identifier travels, and resolution (with the
 *  privacy gate) happens inside Rust's vault lock (the serde mirror of vemry-agent's
 *  context.rs; the two change together). */
export type ContextRef =
  /** image: an image in the vault, whose path alone travels; the bytes are filled in inside the privacy gate when Rust assembles the request. */
  | { kind: "note" | "folder" | "file" | "image"; path: string }
  /** An editor selection (`@ current selection`): the one vault reference whose body is
   *  handed over by the frontend — what is selected are the characters in the buffer
   *  (possibly unsaved). Rust still puts the path through the privacy gate.
   *  The line numbers are 1-based and inclusive. */
  | ({ kind: "selection" } & EditorSelection)
  /** One block of a note (the composer's `@Note#` stage): local_id is `^id` or `bN` without
   *  the caret; label is the chip's preview text and is stripped before sending. */
  | { kind: "block"; path: string; local_id: string; label?: string };

/** The image extensions that can be sent to the model as an image reference (mirroring
 *  `image_mime` in vemry-agent's context.rs; the two change together). Every other
 *  image format (tiff/svg/bmp…) is still stored in the vault but does not become a
 *  chip. */
export const AGENT_IMAGE_EXTS: ReadonlySet<string> = new Set(["png", "jpg", "jpeg", "gif", "webp"]);

/** Whether this path can be sent as an image reference (by extension; the caller also
 *  confirms through fileKind that it really is an image). Chip generation and the @
 *  candidates share it — missing one would mean either "a chip appeared but sending
 *  400s" or "the file silently never shows up among the candidates". */
export function isAgentImage(path: string): boolean {
  return AGENT_IMAGE_EXTS.has((path.split(".").pop() ?? "").toLowerCase());
}

/** The thinking depth level. The value range mirrors Rust's
 *  `settings::is_reasoning_effort` (the two change together), and the order is the
 *  display order. The wording is not here: the short name is the catalog's
 *  `settings.effort.<value>` and the parenthetical is the matching `…Hint` (an empty
 *  string = this level needs no explanation).
 *  Because the wording lives in the catalog, this has to be a literal union rather than
 *  string. */
export type EffortValue = "none" | "low" | "high" | "max";

export interface EffortOption {
  value: EffortValue;
  /** The level used when nothing is chosen (the settings page marks it "default"). */
  isDefault?: boolean;
}
export const EFFORT_OPTIONS: readonly EffortOption[] = [
  { value: "none" },
  { value: "low" },
  { value: "high", isDefault: true },
  { value: "max" },
];

/** The UTF-16 limit on an editor selection snapshot's body: slightly above Rust's
 *  FILE_CONTEXT_MAX_CHARS truncation line (30k characters) — the real truncation and
 *  the "[... truncated" marker are Rust's job, and this only bounds the string cut out
 *  on every drag when the whole document is selected (a selection change fires on
 *  mousemove). */
export const SELECTION_MAX_CHARS = 40_000;

/** Trim a lone high surrogate after truncating. A UTF-16 slice may cut through a
 *  surrogate pair, and a lone surrogate character makes the IPC's JSON deserialisation
 *  refuse it outright — both the editor's selection snapshot and the reading
 *  projection's pageContext have fallen for it, so the trimming is written once. */
export function trimLoneSurrogate(s: string): string {
  const last = s.charCodeAt(s.length - 1);
  return last >= 0xd800 && last <= 0xdbff ? s.slice(0, -1) : s;
}

/** A snapshot of the editor's current selection (recomputed by Editor as the selection changes; the main window pushes it to the chat window too). */
export interface EditorSelection {
  path: string;
  from_line: number;
  to_line: number;
  text: string;
  /** Touches a `> [!private]` block in the buffer: never offered as a context chip. */
  private?: true;
}

/** A reference tag in the audit log or the timeline: kind/path alone (plus the line
 *  range for a selection) and no body (mirroring the `refs` field of runner.rs's user
 *  line). */
export interface RefTag {
  kind: ContextRef["kind"];
  path: string;
  lines?: [number, number];
  /** A block reference's handle (`^id` / `bN`), as the audit log records it. */
  block?: string;
}

// ---- the reading projection (v0.5 P2) ----

/** page_read's projection result. `url` is the actual location.href at grab time
 *  (after redirects), and the display, the relative link resolution and the clip cache
 *  lookup all go by it. */
export interface ReadPage {
  url: string;
  title: string;
  byline: string | null;
  markdown: string;
  /** Readability's extraction failed and this took the whole-page fallback — a "the layout may be poor" banner is shown. */
  fallback: boolean;
}

/** `reading://event`: the projection request the Reading arm of the url_open funnel sends. */
export type ReadingEvent = { type: "open"; url: string };

export type AdblockStatusView =
  | { state: "unsupported" }
  | { state: "disabled" }
  /** On, but no rules are cached yet — nothing is bundled, so they are downloaded or imported. */
  | { state: "no_rules" }
  | { state: "downloading" }
  | { state: "compiling" }
  | { state: "ready"; identifier: string; rules: number }
  | { state: "failed"; error: string };

/** One entry of the application-level browsing history (browser_history.json, shared
 *  across vaults). ts and added are both unix milliseconds. */
export interface HistoryEntryView {
  url: string;
  title: string;
  ts: number;
  visits: number;
}

/** One bookmarked address (bookmarks.json, shared across vaults). */
export interface BookmarkView {
  url: string;
  title: string;
  added: number;
}

/** `browser://event` payloads — the browser windows' state pushes. */
export type BrowserEvent =
  | { type: "opened"; label: string; url: string; docked: boolean }
  | { type: "navigated"; label: string; url: string }
  | { type: "title_changed"; label: string; title: string }
  | { type: "focused"; label: string }
  | { type: "closed"; label: string }
  | { type: "dock_changed"; label: string; docked: boolean }
  | { type: "dock_layout"; label: string; inset: number }
  | { type: "adblock_status"; status: AdblockStatusView }
  | { type: "bookmarks_changed"; bookmarks: BookmarkView[] };

// ---- user scripts (v0.3 programmability) ----

export interface ScriptCommandInfo {
  id: string;
  title: string;
  description: string | null;
  hotkey: string | null;
}

export interface ScriptInfo {
  name: string;
  enabled: boolean;
  write_grant: boolean;
  commands: ScriptCommandInfo[];
  event_triggers: string[];
  schedules: string[];
  last_error: string | null;
}

/** `script://event` payloads: host pushes + the blocking dialog requests. */
export type ScriptEvent =
  | { type: "commands_changed"; scripts: ScriptInfo[] }
  | { type: "toast"; message: string; level: "info" | "error" }
  | { type: "error"; script: string; message: string }
  | { type: "grant_request"; id: string; script: string; path: string }
  | { type: "prompt_request"; id: string; script: string; message: string; placeholder: string | null };

// ---- commands ----

export const api = {
  vaultOpen: (path: string) => invoke<VaultInfo>("vault_open", { path }),
  vaultCreate: (path: string) => invoke<VaultInfo>("vault_create", { path }),
  vaultRecent: () => invoke<string[]>("vault_recent"),
  vaultRecentRemove: (path: string) => invoke<string[]>("vault_recent_remove", { path }),
  vaultLast: () => invoke<string | null>("vault_last"),
  /** The vault that is open (the second webview picks up the main window's vault without going through vault_open). */
  vaultCurrent: () => invoke<VaultInfo | null>("vault_current"),
  notesList: () => invoke<NoteSummary[]>("notes_list"),
  noteRead: (path: string) => invoke<{ id: string; content: string }>("note_read", { path }),
  noteWrite: (path: string, content: string) => invoke<void>("note_write", { path, content }),
  noteCreate: (title: string, folder?: string) => invoke<string>("note_create", { title, folder }),
  /** Located insertion: a matching section lands at the end of that section, and a
   *  missing or unspecified one at the end of the file (with blank lines normalised).
   *  It returns where it actually landed, which is how the caller knows to say "that
   *  section does not exist". */
  noteInsert: (path: string, text: string, section?: string) =>
    invoke<"section" | "tail">("note_insert", { path, text, section }),
  search: (query: string, limit?: number) => invoke<SearchHit[]>("search", { query, limit }),
  linksGet: (path: string) => invoke<NoteLinks>("links_get", { path }),
  queryBlocks: (query: BlockQuery) => invoke<BlockHit[]>("query_blocks", { query }),
  graphGet: () => invoke<GraphData>("graph_get"),
  linkTargets: () => invoke<string[]>("link_targets"),
  /** Inline-field keys in use and their frequent values (for `key::` completion and group-by menus). */
  fieldsList: () => invoke<FieldStat[]>("fields_list"),
  /** The vault's block types (`.vemry/types/*.md`), disabled ones included. */
  typesList: () => invoke<BlockType[]>("types_list"),
  /** Open-or-create for "block type…": `.vemry/types/<name>.md` is written when there is none (`book`
   *  gets the full example, any other name a skeleton); one already there is left as it is. */
  typeCreate: (name: string) => invoke<Scaffolded>("type_create", { name }),
  foldersList: () => invoke<string[]>("folders_list"),
  folderCreate: (path: string) => invoke<string>("folder_create", { path }),
  filesList: () => invoke<string[]>("files_list"),
  fileCreate: (path: string) => invoke<string>("file_create", { path }),
  fileRead: (path: string) => invoke<string>("file_read", { path }),
  fileReadBinary: (path: string) => invoke<ArrayBuffer>("file_read_binary", { path }),
  // The path travels in an IPC header (the body is left to the binary bytes). A header
  // value has to be ISO-8859-1: a non-ASCII file name placed there raw makes Windows
  // WebView2's new Headers() throw a TypeError outright, so it is percent-encoded and
  // commands.rs::attachment_save decodes it symmetrically.
  attachmentSave: (bytes: Uint8Array, path: string) =>
    invoke<string>("attachment_save", bytes, { headers: { path: encodeURIComponent(path) } }),
  /** The attachment-bucket migration (a one-off explicit command): dry_run produces the
   *  report for confirmation, and a real run moves the directories and rewrites the
   *  links vault-wide. Mirrors commands.rs::attachments_migrate. */
  attachmentsMigrate: (dryRun: boolean) => invoke<MigrationReport>("attachments_migrate", { dryRun }),
  clipboardPaste: (dest: string, stamp: string, reuseVaultFiles: boolean) =>
    invoke<string[]>("clipboard_paste", { dest, stamp, reuseVaultFiles }),
  /** Import absolute disk paths picked from the native file dialog into the vault
   *  folder `dest` (the agent's "+" attachment); a file already inside the vault has its
   *  path returned without being copied. Returns vault-relative paths. */
  filesImport: (paths: string[], dest: string) => invoke<string[]>("files_import", { paths, dest }),
  /** Plain text on the clipboard (the fallback leg of the editor's right-click paste); off macOS it returns null. */
  clipboardReadText: () => invoke<string | null>("clipboard_read_text"),
  clipboardCopyImage: (path: string) => invoke<void>("clipboard_copy_image", { path }),
  fileWrite: (path: string, content: string) => invoke<void>("file_write", { path, content }),
  pathRename: (from: string, to: string) => invoke<string>("path_rename", { from, to }),
  pathDelete: (path: string) => invoke<void>("path_delete", { path }),
  fileOpenSystem: (path: string) => invoke<void>("file_open_system", { path }),
  urlOpenSystem: (url: string) => invoke<void>("url_open_system", { url }),
  /** The single link-opening funnel: it branches by the setting into the built-in or the system browser; invert (⌘click) inverts it. */
  urlOpen: (url: string, invert = false, focus = true) => invoke<void>("url_open", { url, invert, focus }),
  browserOpen: (url: string, newWindow = false, focus = true) =>
    invoke<string>("browser_open", { url, newWindow, focus }),
  /** The DNS prefetch (called behind a debounce when ⌘L input pauses): it pushes the
   *  target domain's resolution into the system cache early, so WebKit skips the
   *  resolution step after return. Only a complete domain is sent (the Rust side checks
   *  for a dot), it is purely a side channel, it never creates a window or a
   *  navigation, and a failure goes unnoticed. */
  browserPrefetch: (url: string) => invoke<void>("browser_prefetch", { url }),
  browserSetDocked: (docked: boolean, label?: string) =>
    invoke<void>("browser_set_docked", { docked, label }),
  browserNavigate: (url: string, label?: string) => invoke<void>("browser_navigate", { url, label }),
  browserBack: (label?: string) => invoke<void>("browser_back", { label }),
  browserForward: (label?: string) => invoke<void>("browser_forward", { label }),
  browserReload: (label?: string) => invoke<void>("browser_reload", { label }),
  browserClose: (label?: string) => invoke<void>("browser_close", { label }),
  browserFocus: (label: string) => invoke<void>("browser_focus", { label }),
  browserState: () => invoke<BrowserSnapshot>("browser_state"),
  /** The single implementation of "open the browser": focus an existing window or
   *  reopen the most recent page from the history; null = no window and no history (the
   *  caller says so). */
  browserOpenLast: () => invoke<string | null>("browser_open_last"),
  browserHistoryList: () => invoke<HistoryEntryView[]>("browser_history_list"),
  browserHistoryRemove: (url: string) => invoke<void>("browser_history_remove", { url }),
  browserHistoryClear: () => invoke<void>("browser_history_clear"),
  bookmarkList: () => invoke<BookmarkView[]>("bookmark_list"),
  /** Toggle the ☆ bookmark and return the new state (true = bookmarked); the change is broadcast through bookmarks_changed. */
  bookmarkToggle: (url: string, title: string) => invoke<boolean>("bookmark_toggle", { url, title }),
  bookmarkRemove: (url: string) => invoke<void>("bookmark_remove", { url }),
  /** label: the target browser window (the toolbar points straight at the window it is paired with); omitted = the one focused last. */
  clipSelection: (saveToClips: boolean, label?: string) =>
    invoke<ClipSelectionResult>("clip_selection", { saveToClips, label }),
  /** Localise the images after a selection quote block was inserted into a note (a no-op on the backend when the setting is off). */
  clipLocalizeNote: (rel: string, url: string) => invoke<void>("clip_localize_note", { rel, url }),
  clipPage: (label?: string) => invoke<string>("clip_page", { label }),
  clipLink: (label?: string) => invoke<ClipLinkResult>("clip_link", { label }),
  /** The toolbar → main window action relay; focus is true for "open the note" alone
   *  (every other action leaves the browsing uninterrupted — not raising the main window
   *  is exactly what the toolbar is for). */
  clipbarToMain: (action: ClipbarAction, focus = false) =>
    invoke<void>("clipbar_to_main", { action, focus }),
  /** Expand or collapse the toolbar's destination picker (Rust changes the window's height anchored at the bottom edge). */
  clipbarSetExpanded: (expanded: boolean) => invoke<void>("clipbar_set_expanded", { expanded }),
  /** One-click clipping from a source card: the `[title](url)` reference entry lands in clips/ without loading the page. */
  clipUrl: (url: string, title: string) => invoke<string>("clip_url", { url, title }),
  /** The reverse lookup of where a citation landed: which URLs have been clipped (url → the clips/ note's path). */
  clipsLookup: (urls: string[]) => invoke<Record<string, string>>("clips_lookup", { urls }),
  /** "Add to context": extract the current page's body and hand it to the agent as an
   *  attachment of the next message.
   *  label works as in clipSelection: the toolbar points straight at the browser window
   *  it is paired with, and omitted means the one focused last. */
  browserPageContext: (label?: string) => invoke<PageContext>("browser_page_context", { label }),
  /** Fetching for the reading projection: the hidden reader window loads the page and
   *  the clipping pipeline extracts the body.
   *  When clicking citations in quick succession supersedes an older request it returns
   *  null, which the caller ignores silently. */
  pageRead: (url: string) => invoke<ReadPage | null>("page_read", { url }),
  /** Clipping from the projection: the Rust-side cache is used directly to land in clips/, byte-identical to the projection and with no refetch. */
  readingClip: (url: string) => invoke<string>("reading_clip", { url }),
  /** A projection selection → quote block text (landed by the frontend's insertAtCursor or capture). */
  readingQuote: (url: string, selection: string, html: string) =>
    invoke<string>("reading_quote", { url, selection, html }),
  /** Downloads every configured subscription; resolves with the per-source failures
   *  (empty = all of them came through). A partial download still counts as success. */
  adblockUpdateRules: () => invoke<string[]>("adblock_update_rules"),
  /** Imports rule files the user downloaded themselves — the escape hatch for a
   *  network that cannot reach the subscription hosts. Resolves with the file count. */
  adblockImportRules: (paths: string[]) => invoke<number>("adblock_import_rules", { paths }),
  resolveLink: (target: string, from?: string) => invoke<string | null>("resolve_link", { target, from }),
  /** `Note` / `Note#Heading` / `Note#^id` / `#^id` → the note and the raw slice it denotes. */
  resolveRef: (target: string, from?: string) => invoke<RefResolution>("resolve_ref", { target, from }),
  /** Disk path (the note is not open): mint or return the block's `^id`, writing the note when minting. */
  blockEnsureId: (path: string, localId: string, guard?: string) =>
    invoke<string>("block_ensure_id", { path, localId, guard }),
  /** Buffer path (the note is open — a disk write would be clobbered by autosave): send the buffer
   *  text, apply the one returned insert at `byteToChar(offset)`. Placement and id generation live
   *  only in Rust (`blockid.rs::plan_block_id`); the editor never computes where an id goes. */
  blockIdPlan: (source: string, selector: BlockSelector) => invoke<BlockIdPlan>("block_id_plan", { source, selector }),
  /** Who authored each block of a note. */
  provenanceGet: (path: string) => invoke<BlockProvenance[]>("provenance_get", { path }),
  /** Accept pending agent blocks (one by local id, or all); resolves to the count still pending. */
  provenanceAccept: (path: string, localId?: string) => invoke<number>("provenance_accept", { path, localId }),
  /** The revert plan for `text` (the editor's buffer): one unreviewed block by local id, or all; the editor applies it as one undoable edit. */
  provenanceRevertPlan: (text: string, localId?: string) => invoke<RevertPlan>("provenance_revert_plan", { text, localId }),
  /** Record text the user placed themselves (lands accepted); a private path or text without blocks records nothing. */
  provenanceRecord: (path: string, text: string, origin: "agent" | "clip" | "capture", session?: string, url?: string) =>
    invoke<void>("provenance_record", { path, text, origin, session, url }),
  /** Every note with agent blocks awaiting review. */
  provenancePending: () => invoke<PendingNote[]>("provenance_pending"),
  settingsGet: () => invoke<SettingsView>("settings_get"),
  settingsSet: (update: SettingsUpdate) => invoke<void>("settings_set", { update }),
  agentStart: (
    message: string,
    currentNote: string | null,
    pageContext: PageContext | null,
    contextRefs: ContextRef[],
    effort: string | null,
    onEvent: Channel<AgentEvent>,
  ) => invoke<string>("agent_start", { message, currentNote, pageContext, contextRefs, effort, onEvent }),
  /** Regenerate the last turn: cut the assistant/tool tail and replay the generation from the original envelope. */
  agentRetry: (effort: string | null, onEvent: Channel<AgentEvent>) =>
    invoke<string>("agent_retry", { effort, onEvent }),
  /** The first half of an edit and resend: drop the last user message and everything after it. */
  agentHistoryTruncate: () => invoke<void>("agent_history_truncate"),
  agentPermissionRespond: (requestId: string, decision: PermissionDecision) =>
    invoke<void>("agent_permission_respond", { requestId, decision }),
  /** Handing a plan-gate decision back (propose_plan): the loop continues within the same turn. */
  agentPlanRespond: (requestId: string, decision: PlanDecision) =>
    invoke<void>("agent_plan_respond", { requestId, decision }),
  agentCancel: () => invoke<void>("agent_cancel"),
  /** Cancelling only sets a flag: the turn is really over when agentIsRunning goes to false (see reattach). */
  agentIsRunning: () => invoke<boolean>("agent_is_running"),
  agentSessionNew: () => invoke<void>("agent_session_new"),
  agentSessionList: () => invoke<SessionMeta[]>("agent_session_list"),
  /** Returns that session's full jsonl, from which the frontend's parseSession rebuilds the timeline. */
  agentSessionSelect: (id: string) => invoke<string>("agent_session_select", { id }),
  /** The current session id (it does not switch the context; the chat window's startup and the main window taking over rebuild their timeline from it). */
  agentSessionCurrent: () => invoke<string | null>("agent_session_current"),
  // ---- the detached AI chat window (agent_window.rs): the handoff package is opaque JSON whose shape the frontend defines ----
  /** Returns whether a window was created (false = one was already open and only focused: the handoff was not delivered, so the main window's draft must not be cleared). */
  agentWindowOpen: (handoff: unknown, currentNote: string | null, selection: EditorSelection | null) =>
    invoke<boolean>("agent_window_open", { handoff, currentNote, selection }),
  agentWindowClose: (handback: unknown, discard: boolean) =>
    invoke<void>("agent_window_close", { handback, discard }),
  agentWindowIsOpen: () => invoke<boolean>("agent_window_is_open"),
  /** queued: the to-agent commands queued before the webview mounted (Rust queues them,
   *  emit does not — anything sent before mounting is dropped), handled one by one in
   *  the order they were sent as if they had just arrived. */
  agentWindowTakeHandoff: () =>
    invoke<{
      handoff: unknown;
      current_note: string | null;
      selection: EditorSelection | null;
      queued: AgentWindowCommand[];
    }>("agent_window_take_handoff"),
  agentWindowTakeHandback: () => invoke<unknown>("agent_window_take_handback"),
  agentWindowSetNote: (path: string | null) => invoke<void>("agent_window_set_note", { path }),
  agentWindowSetSelection: (selection: EditorSelection | null) =>
    invoke<void>("agent_window_set_selection", { selection }),
  agentWindowToMain: (action: HostAction, focus: boolean) =>
    invoke<void>("agent_window_to_main", { action, focus }),
  agentWindowToAgent: (action: AgentWindowCommand) => invoke<void>("agent_window_to_agent", { action }),
  agentSessionDelete: (id: string) => invoke<void>("agent_session_delete", { id }),
  /** Rename by hand: a title override line is appended to the audit jsonl (the last one wins), and it is allowed while the agent is running. */
  agentSessionRename: (id: string, title: string) =>
    invoke<void>("agent_session_rename", { id, title }),
  /** Fork a new session off the current one before one of its user messages and switch
   *  to it. The fork point is located by turn id first (turn, the one shared by all three
   *  layers); null (an existing old session with no ids) falls back to counting to the
   *  Nth user message (0-based). summarized_context=true: the fork point had already been
   *  swallowed into a summary by second-level compaction, so the context the new session
   *  carries covers more than the fork point did. */
  agentSessionFork: (turn: string | null, beforeUserTurn: number) =>
    invoke<{ id: string; summarized_context: boolean }>("agent_session_fork", {
      turn,
      beforeUserTurn,
    }),
  /** Read-only fetching for the replay view (agent_session_select switches the model context, so the replay does not use it). */
  sessionRead: (id: string) => invoke<string>("session_read", { id }),
  initMemoryFile: () => invoke<string>("init_memory_file"),
  /** The `#` quick-memory convention: one bullet lands in VEMRY.md's conventions section. */
  memoryAppend: (text: string) => invoke<void>("memory_append", { text }),
  /** Write the research skill (`.vemry/skills/research.md`); one already there is left as it is. */
  initResearchSkill: () => invoke<Scaffolded>("init_research_skill"),
  /** Install the research methodology skill automatically the first time the research
   *  panel is opened (idempotent; deleting it does not resurrect it); returns whether it
   *  was installed just now. */
  ensureResearchSkill: () => invoke<boolean>("ensure_research_skill"),
  skillsList: () => invoke<SkillEntry[]>("skills_list"),
  skillSetEnabled: (file: string, enabled: boolean) =>
    invoke<void>("skill_set_enabled", { file, enabled }),
  /** The vault's persistent write policy ("allow" = writes are pre-authorised). */
  vaultWritePolicyGet: () => invoke<"allow" | "ask">("vault_write_policy_get"),
  vaultWritePolicySet: (policy: "allow" | "ask") =>
    invoke<void>("vault_write_policy_set", { policy }),
  privacyList: () => invoke<string[]>("privacy_list"),
  privacyMark: (path: string, kind: "note" | "folder" | "file") => invoke<void>("privacy_mark", { path, kind }),
  privacyUnmark: (path: string, kind: "note" | "folder" | "file") => invoke<void>("privacy_unmark", { path, kind }),
  privacyRules: () => invoke<string[]>("privacy_rules"),
  privacyRuleAdd: (line: string) => invoke<void>("privacy_rule_add", { line }),
  privacyRuleRemove: (line: string) => invoke<void>("privacy_rule_remove", { line }),
  scriptsList: () => invoke<ScriptInfo[]>("scripts_list"),
  scriptsReload: () => invoke<void>("scripts_reload"),
  scriptSetEnabled: (name: string, enabled: boolean) => invoke<void>("script_set_enabled", { name, enabled }),
  scriptCommandRun: (script: string, command: string, currentNote: string | null) =>
    invoke<void>("script_command_run", { script, command, currentNote }),
  scriptGrantRespond: (requestId: string, allow: boolean) =>
    invoke<void>("script_grant_respond", { requestId, allow }),
  scriptGrantRevoke: (name: string) => invoke<void>("script_grant_revoke", { name }),
  scriptPromptRespond: (requestId: string, value: string | null) =>
    invoke<void>("script_prompt_respond", { requestId, value }),
  scriptEvalBlock: (code: string, currentNote: string | null) =>
    invoke<{ output: string; error: string | null }>("script_eval_block", { code, currentNote }),
};

export { Channel };

/** The cross-window broadcast of a language change. One bundle runs in three webviews
 *  — the main window, the chat window and the clip toolbar — and each has its own i18n
 *  state, so without pushing a settings change made in the main window, a window that
 *  is already open would stay in the old language. emit is a broadcast and the sender
 *  receives it too (which is idempotent). */
export function emitLanguage(lang: LangSetting) {
  void emit("settings://language", lang);
}

export function onLanguageChange(handler: (lang: LangSetting) => void): Promise<UnlistenFn> {
  return listen<LangSetting>("settings://language", (event) => handler(event.payload));
}

export function onVaultEvent(handler: (e: VaultEvent) => void): Promise<UnlistenFn> {
  return listen<VaultEvent>("vault://event", (event) => handler(event.payload));
}

export function onScriptEvent(handler: (e: ScriptEvent) => void): Promise<UnlistenFn> {
  return listen<ScriptEvent>("script://event", (event) => handler(event.payload));
}

export function onBrowserEvent(handler: (e: BrowserEvent) => void): Promise<UnlistenFn> {
  return listen<BrowserEvent>("browser://event", (event) => handler(event.payload));
}

/** The event bridge of the native menu bar (macOS): the payload is a registry command
 *  id — the menu never grows a second set of semantics (the payload convention of
 *  src-tauri/src/menu.rs). */
export function onMenuEvent(handler: (id: string) => void): Promise<UnlistenFn> {
  return listen<string>("menu://command", (event) => handler(event.payload));
}

export function onReadingEvent(handler: (e: ReadingEvent) => void): Promise<UnlistenFn> {
  return listen<ReadingEvent>("reading://event", (event) => handler(event.payload));
}

// ---- the cross-window events of the detached AI chat window (emitted by agent_window.rs, directed with emit_to) ----
/** Chat window: the main window's current note changed. */
export function onAgentWindowNote(handler: (path: string | null) => void): Promise<UnlistenFn> {
  return listen<string | null>("agent-window://note", (event) => handler(event.payload));
}
/** Chat window: the main window's editor selection changed (null = no selection). */
export function onAgentWindowSelection(handler: (sel: EditorSelection | null) => void): Promise<UnlistenFn> {
  return listen<EditorSelection | null>("agent-window://selection", (event) => handler(event.payload));
}
/** Chat window: Rust received a close request and waits for the draft to come back before closing. */
export function onAgentWindowCloseRequested(handler: () => void): Promise<UnlistenFn> {
  return listen<null>("agent-window://close-requested", () => handler());
}
/** Main window: the chat window was destroyed, so take the handback and rebuild the timeline. */
export function onAgentWindowClosed(handler: () => void): Promise<UnlistenFn> {
  return listen<null>("agent-window://closed", () => handler());
}
/** Main window: an action the chat window wants the main window to run. */
export function onAgentWindowAction(handler: (a: HostAction) => void): Promise<UnlistenFn> {
  return listen<HostAction>("agent-window://action", (event) => handler(event.payload));
}
/** Chat window: a command from the main window. */
export function onAgentWindowCommand(handler: (c: AgentWindowCommand) => void): Promise<UnlistenFn> {
  return listen<AgentWindowCommand>("agent-window://command", (event) => handler(event.payload));
}

// ---- the cross-window events of the clip toolbar (clipbar-N, batch D) ----
/** Main window: an action the toolbar wants the main window to run (relayed by clipbar_to_main). */
export function onClipbarAction(handler: (a: ClipbarAction) => void): Promise<UnlistenFn> {
  return listen<ClipbarAction>("clipbar://action", (event) => handler(event.payload));
}
/** Toolbar: the receipt after the main window ran a clip action. */
export function onClipbarResult(handler: (r: ClipbarResult) => void): Promise<UnlistenFn> {
  return listen<ClipbarResult>("clipbar://result", (event) => handler(event.payload));
}
/** Main window → a given toolbar window's receipt (directed, not broadcast). */
export function emitClipbarResult(label: string, r: ClipbarResult): Promise<void> {
  return emitTo(label, "clipbar://result", r);
}

/** The cross-window sync payload of the pinned destination (clipTarget). */
export interface ClipTargetSync {
  path: string | null;
  heading: string | null;
}
/** A non-main window (the chat window, the toolbar): the main window changed the pinned destination, so it is mirrored into this window's store. */
export function onClipTargetChanged(handler: (t: ClipTargetSync) => void): Promise<UnlistenFn> {
  return listen<ClipTargetSync>("clip-target://target", (event) => handler(event.payload));
}
/** Main window (the only writer): broadcast a change of the pinned destination. */
export function emitClipTargetChanged(t: ClipTargetSync): Promise<void> {
  return emit("clip-target://target", t);
}
