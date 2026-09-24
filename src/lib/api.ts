// The single typed entry point for all IPC. Mirrors src-tauri/src/commands.rs.
import { invoke, Channel } from "@tauri-apps/api/core";
import { emit, emitTo, listen, type UnlistenFn } from "@tauri-apps/api/event";

// ---- types mirrored from Rust ----

import type { NoteScript } from "./notelang";

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
  /** frontmatter `vemry.agent_session` — the chat session a note saved from the research panel came from (so it can be resumed) */
  agent_session?: string | null;
  /** what the note reads as (vemry-core lang.rs, the mirror of notelang.ts noteLanguage):
   *  its declared or detected language, null when unknown or too short to say */
  lang: string | null;
  /** its writing system, which sets the column width and the line-breaking rules */
  script: NoteScript | null;
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

/** A task's checkbox (mirrors vemry-core's `TaskState`): `[ ]`, `[/]`, `[x]`, `[-]`. */
export type TaskState = "todo" | "doing" | "done" | "cancelled";
/** A task's priority (mirrors `Priority`); `normal` is the unwritten default and never serialised. */
export type Priority = "highest" | "high" | "normal" | "low" | "lowest";

export type BlockKind =
  | { type: "paragraph" }
  | { type: "heading"; level: number }
  | { type: "list_item"; indent: number; ordered: boolean }
  | {
      type: "task";
      state: TaskState;
      due?: string;
      scheduled?: string;
      start?: string;
      created?: string;
      /** ✅ — the day it was ticked. */
      done?: string;
      /** ❌ — the day it was cancelled. */
      cancelled?: string;
      priority?: Priority;
      /** 🔁 — the rule's text as written. */
      repeat?: string;
      /** 🚮 — the line is removed once done. */
      delete_on_done?: boolean;
      /** 🆔 — the handle other tasks depend on. */
      task_id?: string;
      /** ⛔ — task ids that must be closed first. */
      depends_on?: string[];
      /** 🕐 — minutes. */
      estimate?: number;
    }
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
  /** `date` / `number` / `text` / `state` (a hint). `state` makes `values` a vocabulary
   *  in order — the first value is where a block starts, the last where it ends. */
  type?: string;
}

/** What a scaffold command (`typeCreate`, `skillCreate`, `skillCustomize`) hands back (mirrors commands.rs
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
  task_state?: TaskState;
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
  /** Where that write's hunk sits in the note: blocks sharing it arrived in one hunk and are reviewed as one run. */
  hunk?: { start: number; end: number };
  /** The lines that hunk replaced, when the write replaced rather than added. */
  replaced?: string;
  hash: string;
  origin: Origin;
}

/** One note with unreviewed AI blocks (mirroring commands.rs::PendingNote); the list comes in path order. */
export interface PendingNote {
  path: string;
  blocks: number;
}

/** What a note-level accept did (mirroring commands.rs::AcceptedTotal). */
export interface AcceptedTotal {
  blocks: number;
  notes: number;
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

/** The interface language. Matches Rust's settings::LangSetting exactly; the list
 *  of tags is LANGS in i18n.svelte.ts and the Lang enum in src-tauri/src/i18n.rs. */
export type LangSetting = "auto" | "en" | "zh-CN" | "ja" | "ko" | "fr" | "de" | "es";

/** Which provider the AI talks to (Rust's settings::ProviderSetting): the user's own endpoint
 *  and key, or vemry hosted (the relay, the account's device token, the catalog's models). */
export type ProviderSetting = "custom" | "hosted";

/** One row of the hosted model catalog (mirroring vemry_agent::provider::ModelCap). */
export interface ModelCap {
  id: string;
  default: boolean;
  /** Web search is available on this model. */
  search: boolean;
  /** Image input is accepted. */
  images: boolean;
  max_tokens: number | null;
}

/** The entitlement snapshot of the hosted account (mirroring account.rs::Me; the shape of `GET /v1/me`). */
export interface Me {
  email: string;
  plan: "free" | "pro" | "founder" | string;
  pro_until: string | null;
  /** Hosted model access is open to this account. */
  hosted: boolean;
  quota: { total_fen: number; used_fen: number; resets_at: string | null } | null;
  /** The credit pack in fen; negative while an overdraft awaits the next credit. */
  pack_fen: number;
  subscription: {
    status: "active" | "canceling" | "canceled" | "past_due" | string;
    cancel_at_period_end: boolean;
    next_charge_at: string | null;
    price: string | null;
  } | null;
  estimates: { study_fen: number; translate_fen: number };
  models: ModelCap[];
  workflow_free_uses: number;
  portal_url: string;
  server_time: string;
  peak: boolean;
}

/** How old the account snapshot is: fresh (≤ 10 min), stale (≤ 7 days, the offline grace — it
 *  still applies), expired (past that: the account counts as free until a refresh succeeds). */
export type Freshness = "fresh" | "stale" | "expired";

/** What the account module reports (mirroring account.rs::AccountStatus). */
export interface AccountStatus {
  logged_in: boolean;
  /** The last snapshot, possibly stale; null when signed out. */
  me: Me | null;
  fetched_at: string | null;
  freshness: Freshness;
  /** Hosted requests may go out: signed in, hosted access, and the snapshot not expired. */
  hosted_ok: boolean;
  /** floor(quota remaining / study estimate); null without a quota or an estimate. */
  study_left: number | null;
}

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
  /** Derived and read-only: the provider runs the search (the DeepSeek endpoint; decided on the Rust side, do not reimplement it in the frontend). The composer's web search switch (an item of its model menu) is enabled by it. */
  web_search_available: boolean;
  /** Derived and read-only: the switch is on and the provider runs the search (the same source as the per-turn check in agent_send). */
  web_search_active: boolean;
  /** The agent's plan confirmation (propose_plan plus a blocking gate); frozen per session, so a change takes effect from the next session. */
  plan_confirm: boolean;
  /** AI blocks awaiting review show an amber bar in the editor. */
  provenance_marks: boolean;
  reading_images_enabled: boolean;
  clip_localize_images: boolean;
  format_agent_writes: boolean;
  format_clips: boolean;
  format_cjk_spacing: boolean;
  format_punctuation: boolean;
  updater_auto_check: boolean;
  /** The version "skip this version" muted for the silent startup check (null = none). */
  updater_skipped_version: string | null;
  note_font_size: number;
  readable_line_width: boolean;
  /** The thinking depth level: none/low/high/max (DeepSeek's official default is high). */
  reasoning_effort: string;
  /** Derived and read-only: effort is only sent when the endpoint is DeepSeek (decided on the Rust side; do not reimplement it in the frontend). */
  reasoning_effort_active: boolean;
  /** Derived and read-only: the selected model has no image understanding (DeepSeek's pro model; the composer warns once when an image is attached). */
  images_unsupported: boolean;
  /** Side-by-side translation: the target language ("" = the interface language), the model ("" = the chat model), the user's extra rule. */
  translate_target: string;
  translate_model: string;
  translate_addendum: string;
  /** Which provider the AI talks to, and the hosted model choice ("" = the catalog's default). */
  provider: ProviderSetting;
  hosted_model: string;
  /** Derived and read-only: the model requests actually name (the hosted choice or the catalog's default in hosted mode, `model` otherwise). */
  active_model: string;
  /** Quick capture: the global shortcut in the shortcut plugin's grammar ("Alt+Shift+Space"; capture.ts renders it),
   *  the registration's failure (derived and read-only; null = the shortcut works), and where Enter lands a capture.
   *  The pin is the open vault's (`capture_pin` in its .vemry/config.json; empty with no vault open): served in this
   *  one view and written back through settingsSet into that vault, so each vault keeps its own. */
  capture_hotkey: string;
  capture_hotkey_error: string | null;
  capture_target: CaptureTarget;
  capture_pinned_path: string;
  capture_pinned_heading: string;
  /** The vault a capture from a phone lands in (its root; null = whichever vault is open). Set to the open vault when
   *  the first pairing code is shown (inbox.rs, "The landing vault"). */
  inbox_vault: string | null;
  /** The last landing of captures from a phone (derived and read-only). */
  inbox_last_landed: InboxLanding | null;
  /** The resident state (closing the main window hides it behind a menu bar / tray icon; tray.rs), and the icon's
   *  creation failure (derived and read-only; null = the icon is there, or the switch is off). */
  tray_resident: boolean;
  tray_error: string | null;
}

/** The capture target's wire values (settings.rs CaptureTarget): inbox.md, today's journal note, or the pinned note and section. */
export type CaptureTarget = "inbox" | "journal" | "pinned";

/** A partial update: a field left out keeps its value. */
export interface SettingsUpdate {
  base_url?: string;
  provider?: ProviderSetting;
  hosted_model?: string;
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
  format_agent_writes?: boolean;
  format_clips?: boolean;
  format_cjk_spacing?: boolean;
  format_punctuation?: boolean;
  updater_auto_check?: boolean;
  /** Set-only (see commands.rs): the update dialog's "skip this version". */
  updater_skipped_version?: string;
  note_font_size?: number;
  readable_line_width?: boolean;
  reasoning_effort?: string;
  translate_target?: string;
  translate_model?: string;
  translate_addendum?: string;
  /** Refused (nothing else in the update is lost) when the combination does not parse or another app holds it. */
  capture_hotkey?: string;
  capture_target?: CaptureTarget;
  /** Written into the open vault's .vemry/config.json (see SettingsView); ignored with no vault open. */
  capture_pinned_path?: string;
  capture_pinned_heading?: string;
  /** "" = whichever vault is open. */
  inbox_vault?: string;
  tray_resident?: boolean;
}

/** Who wrote over a kept state of a note (history.rs). */
export type HistoryWriter = "editor" | "agent" | "script" | "clip" | "restore" | "app" | "external";
export interface HistorySnapshot {
  id: string;
  ts_ms: number;
  writer: HistoryWriter;
  bytes: number;
}
export interface HistoryEntry {
  content: string;
  diff: string;
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
  kind: "create" | "overwrite" | "edit" | "move" | "append";
  new_content: string;
  /** A move's link rewrites, one per note that changes (the diffs are in `diff`); absent for every other write. */
  rewrites?: { path: string; links: number }[];
  /** A block-level rewrite's summary (a restructure, a block move): how the note compares block by block. */
  block_diff?: BlockDiff;
}

/** How two versions of a note compare block by block (mirroring vemry_core::restructure::BlockDiff). */
export interface BlockDiff {
  moved: number;
  changed: number;
  added: number;
  removed: number;
  items: { kind: "moved" | "changed" | "added" | "removed"; text: string; old?: string }[];
}

/** What "undo this turn's writes" did (mirroring vemry_agent::undo::UndoReport). */
export interface UndoReport {
  reverted: number;
  skipped: number;
}

// ---- the lab of a js block (crates/vemry-script/src/lab.rs is the source of these shapes) ----

/** A control the block declared, with the value the last run used. */
export type LabControl =
  | { kind: "slider"; name: string; label: string | null; min: number; max: number; step: number; value: number }
  | { kind: "toggle"; name: string; label: string | null; value: boolean }
  | { kind: "select"; name: string; label: string | null; options: string[]; value: string }
  | { kind: "matrix"; name: string; label: string | null; rows: number[][]; min: number | null; max: number | null; step: number | null }
  | { kind: "presets"; name: string; label: string | null; items: LabPreset[]; value: string | null };

/** One preset button: its label and the controls it sets, by name. */
export interface LabPreset {
  label: string;
  set: Record<string, number | boolean | string | number[][]>;
}

export interface LineSeries {
  name: string | null;
  y: (number | null)[];
  x: (number | null)[] | null;
}
export interface BarSeries {
  name: string | null;
  values: (number | null)[];
}
export interface PointSeries {
  name: string | null;
  points: [number, number][];
}
export type LabCell = number | string | null;
export type StepValue = number | (number | null)[] | (number | null)[][];
export interface StepItem {
  label: string;
  text: string | null;
  value: StepValue | null;
}

export type LabView =
  | {
      kind: "heatmap";
      title: string;
      data: (number | null)[][];
      rows: string[] | null;
      cols: string[] | null;
      min: number | null;
      max: number | null;
      digits: number | null;
      diverging: boolean;
    }
  | {
      kind: "line";
      title: string;
      series: LineSeries[];
      x: (number | null)[] | null;
      x_label: string | null;
      y_label: string | null;
      y_min: number | null;
      y_max: number | null;
    }
  | { kind: "bars"; title: string; labels: string[]; series: BarSeries[]; y_label: string | null; horizontal: boolean }
  | { kind: "scatter"; title: string; series: PointSeries[]; x_label: string | null; y_label: string | null }
  | {
      kind: "table";
      title: string;
      columns: string[];
      rows: LabCell[][];
      digits: number | null;
      highlight: { row: number | null; col: number | null } | null;
    }
  | { kind: "step"; title: string; steps: StepItem[] };

export interface LabSpec {
  controls: LabControl[];
  views: LabView[];
}

/** The controls' current values, keyed by name, handed back to the block on a re-run. */
export type LabParams = Record<string, number | boolean | string | number[][]>;

export interface BlockEvalResult {
  output: string;
  error: string | null;
  /** Present when the block called vemry.lab.* and ran without error. */
  lab: LabSpec | null;
}

export interface WebSearchResult {
  title: string;
  url: string;
  snippet: string;
  publish_date: string | null;
}

/** One step of the session's plan (mirroring vemry_agent::tools::PlanItem): the sticky
 *  plan card is a projection of the latest list the model sent through update_plan. */
export type PlanStatus = "pending" | "doing" | "done" | "skipped";
export interface PlanItem {
  text: string;
  status: PlanStatus;
}

/** One entry of the session's source table (mirroring vemry_agent::sources::Source):
 *  verified = a search ran in its turn, or the user handed the page over. */
export interface SourceRecord {
  url: string;
  title: string;
  verified: boolean;
  turn?: string | null;
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
  /** The model's step list (update_plan, or the list confirmed at the plan gate): replaces the sticky plan card. */
  | { type: "plan_updated"; items: PlanItem[] }
  /** propose_actions is suspended awaiting the user's review of the list (the actions gate). */
  | { type: "actions_request"; request: ActionsRequest }
  /** open_for_reading is suspended awaiting the user's click on the URLs (the read gate), or fetching under the standing consent (request.auto). */
  | { type: "read_request"; request: ReadRequest }
  /** The read gate's outcome after the fetch: the pages read, the ones that failed, and whether the standing consent answered it. */
  | { type: "read_decided"; id: string; urls: string[]; failed: string[]; auto: boolean }
  /** A write landed in this turn: the UI counts them per turn to offer "undo this turn's writes", and keeps the notes touched so the turn's review can start from them. */
  | { type: "write_landed"; turn: string | null; path: string }
  /** The web sources this turn cited, settled by Rust at the end of the turn with their verification (authoritative for the turn's source card). */
  | { type: "sources_updated"; turn: string | null; items: SourceRecord[] }
  | { type: "web_searching" }
  | { type: "web_search_queries"; queries: string[] }
  | { type: "web_search_results"; results: WebSearchResult[] }
  /** Per-iteration token counts (only when the provider reports them): they drive the panel's gauge. */
  | {
      type: "usage";
      prompt_tokens: number;
      completion_tokens: number;
      cache_hit_tokens: number;
      /** The server-side searches this call ran (billed separately; 0 on a line that reports none). */
      web_search_requests: number;
      iteration: number;
      max_iterations: number;
    }
  /** The context was compacted: 1 = old tool results turned into handles, 2 = an LLM summary. */
  | { type: "compacted"; level: number }
  /** truncated: the output was cut off by the length limit (so "continue" can be offered). */
  | { type: "turn_finished"; truncated: boolean }
  /** A skill is in effect for the turn: attached by the user (`/name`) or read by the model. */
  | { type: "skill_activated"; name: string }
  | { type: "error"; message: string };

/** A session row in the research panel's list (mirroring vemry_agent::session::SessionMeta). */
export interface SessionMeta {
  id: string;
  /** The first line of the first user message, cut to 80 characters. */
  title: string;
  /** How many user messages there are. */
  turns: number;
  /** The note the session produced (the last write the user allowed, else the note it follows); absent when it only answered. */
  note?: string;
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
  /** Allow, and for the rest of the session stop asking under `prefix` alone (a folder with its trailing slash, or one note's path). */
  | { kind: "allow_scope"; prefix: string }
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

/** One proposed action item (mirroring vemry_agent::permission::ActionItem): the card lets the user correct every field before anything is written. */
export interface ActionItem {
  text: string;
  owner?: string | null;
  due?: string | null;
  target?: string | null;
  source?: string | null;
  /** Extra editable fields (a scoring's reach / impact / confidence / effort), key → value. */
  fields?: Record<string, string>;
  /** Stays where it already is (a capture with no home): shown as staying and unticked; a target the user gives it makes it a move. */
  stay?: boolean;
}

/** The actions gate's request and decision (propose_actions; the same family as the plan gate). */
export interface ActionsRequest {
  id: string;
  items: ActionItem[];
}

export type ActionsDecision = { kind: "approve"; items: ActionItem[] } | { kind: "reject"; reason: string };

/** The read gate's request (open_for_reading; mirroring vemry_agent::permission::ReadRequest) and the user's answer (agent_bridge::ReadAnswer). */
export interface ReadRequest {
  id: string;
  urls: string[];
  /** The session's standing consent answers it: the card lists the URLs without buttons while they are fetched. */
  auto?: boolean;
}
export type ReadAnswer = { kind: "allow"; urls: string[]; auto_sources: boolean } | { kind: "deny"; reason: string };

/** A row of the example block types catalogue in Settings (mirroring commands.rs::TypeExample). */
export interface TypeExample {
  name: string;
  description: string;
  installed: boolean;
}

/** A row of the skills list in Settings (mirroring commands.rs::SkillEntry; disabled ones included).
 *  `source`: "builtin" ships with vemry and has no file in the vault; "vault" is the vault's own;
 *  "override" is a vault file shadowing a built-in of the same name. `file` is the vault file
 *  name — for a built-in, the name a customised copy takes. */
export interface SkillEntry {
  file: string;
  name: string;
  description: string;
  disabled: boolean;
  source: "builtin" | "vault" | "override";
  version: number;
  /** An override whose version is behind the built-in it shadows. */
  outdated: boolean;
  /** The words the `/` picker matches besides the name and the description. */
  triggers: string[];
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

// ---- the built-in browser (v0.4) ----

/** One browser tab: a `browser-N` child webview (of the main window while docked, of the detached window otherwise). */
export interface BrowserTabView {
  label: string;
  url: string;
  title: string;
}

/** Where the tabs live: the panel column of the main window, or the standalone browser window. */
export type BrowserShape = "docked" | "detached";

export interface BrowserSnapshot {
  tabs: BrowserTabView[];
  active: string | null;
  shape: BrowserShape;
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
  | { kind: "command"; id: string }
  /** Open the settings at one setting (an id of the settings index) or a page. */
  | { kind: "settings"; id: string }
  /** Start reviewing the AI blocks of these notes (a turn's writes): the first still unreviewed opens on its first run. */
  | { kind: "review-start"; paths: string[] }
  /** Mark every unreviewed AI block of these notes as reviewed. */
  | { kind: "review-accept"; paths: string[] };
/** An action from the detached browser window's chrome bar (`browserbar`) to the main
 *  window (App.svelte listens and runs it; the clip flow, clipTarget and the editor
 *  buffer all live in the main window alone). The clip actions carry the tab's label. */
export type BrowserbarAction =
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
   *  while summoning from the detached browser always wants the floating chat window). */
  | { kind: "summon-agent"; label: string };
/** The clip receipt from the main window back to the bar (emitTo'd to
 *  browserbar://result): a toast in the main window is invisible to the bar's user, so
 *  the receipt flashes on the bar itself. */
export interface BrowserbarResult {
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

/** What the quick capture window asks the main window to land (capture://action; the
 *  relay of capture_window.rs). id pairs the receipt with the request — the window
 *  hides on Enter and a second capture may be in flight before the first is answered.
 *  text is as typed — the lines after the first become continuation lines of the list
 *  item; attachments are vault-relative paths already saved into the target note's
 *  bucket, embedded as continuation lines. */
export interface CaptureAction {
  id: number;
  text: string;
  target: string;
  heading: string | null;
  attachments: string[];
}
/** The main window's receipt (capture://result), echoing the request's id. A success carries
 *  nothing more: the landing spot was written in the window's foot before Enter, and a section
 *  that was not found is reported by the main window on its side (insertBlock). */
export interface CaptureResult {
  id: number;
  ok: boolean;
  error?: string;
}

/** Captures pulled from the phone inbox for the main window to land (inbox.rs WindowBatch): each item's finished
 *  text and the moment its stamp names (RFC 3339), and the spot they all go to with its label. Rust works these out
 *  for both of its doors, so the window only writes. */
export interface InboxBatch {
  items: { id: string; text: string; at: string }[];
  path: string;
  heading: string | null;
  target: string;
  /** The root of the vault the spot was worked out for: a window on another vault by now lands nothing. */
  root: string;
}
/** A paired phone (inbox.rs InboxDoor): kind is pwa | shortcut | other, name what the phone called itself. */
export interface InboxDoor {
  id: number;
  kind: string;
  name: string;
  created_at: string;
  last_seen_at: string | null;
}
/** A pairing code to show: the QR code as an SVG in currentColor, and when the code stops working. */
export interface PairStart {
  url: string;
  expires_at: string;
  svg: string;
  /** The landing vault after this code was shown (the open vault became it when none was fixed yet). */
  inbox_vault: string | null;
}

/** One landing of captures from a phone (settings.rs InboxLanding): when, the spot's label, and the vault's folder
 *  name when they were written straight into a vault other than the open one (null through the window). */
export interface InboxLanding {
  at: string;
  target: string;
  vault: string | null;
}
/** Captures written straight into the landing vault while another vault (or none) was open (inbox://landed): the
 *  receipt names the vault by its folder name and the spot as capture.ts's spotLabel spells it. */
export interface InboxLanded {
  count: number;
  vault: string;
  target: string;
}

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
  | { kind: "block"; path: string; local_id: string; label?: string }
  /** What a projection view shows right now (the Agenda, the board): Rust re-runs `query`
   *  through the privacy-filtered query API. `path` is the view's id (`view:agenda`), the
   *  audit's spelling; `label` is the chip's wording; `rows` is the count shown. */
  | { kind: "view"; path: string; label: string; query: BlockQuery; rows?: number }
  /** A skill chosen in the composer (`/research`): `path` is the skill's listed name. Rust
   *  injects its text as the method for the message — first among the envelopes, outside the
   *  reference cap, never compacted away — and reports it as skill_activated. */
  | { kind: "skill"; path: string };

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

// ---- side-by-side translation (src-tauri/src/translate.rs) ----
/** What is being translated. */
export type TranslateSource = { kind: "note"; path: string } | { kind: "page"; url: string } | { kind: "pdf"; path: string };
/** A segment's kind (vemry_core::segment::SegKind). */
export type SegKind =
  | { type: "heading"; level: number }
  | { type: "para" }
  | { type: "list_item"; ordered: boolean; task: boolean }
  | { type: "quote" }
  | { type: "table" }
  | { type: "code" }
  | { type: "math" }
  | { type: "image" }
  | { type: "hr" }
  | { type: "private" }
  /** Not body text (a caption, a reference entry, a byline, a stamp): shown, never translated. */
  | { type: "aside" };
export interface TranslateSegment {
  id: number;
  kind: SegKind;
  /** The structural marker (`## `, `- [ ] `); source and translation both render as prefix + text. */
  prefix: string;
  text: string;
  /** False for the kinds copied through untouched (code, maths, images, rules, private). */
  translate: boolean;
  private: boolean;
  /** The block's byte offset in the note (openBlockAt's unit); 0 for a source with no anchors. */
  byte_start: number;
  /** The 1-based page of a PDF's segment; null for the other sources. */
  page: number | null;
  /** The translation the cache already held. */
  cached: string | null;
}
export interface TranslateDocView {
  job: string;
  title: string;
  source: TranslateSource;
  target: string;
  model: string;
  segments: TranslateSegment[];
}
export type TranslateEvent =
  | { job: string; type: "segment"; id: number; text: string; done: boolean }
  | { job: string; type: "failed"; ids: number[]; message: string }
  /** The background translation stopped at its budget before segment `at`; what is looked at is still translated. */
  | { job: string; type: "paused"; at: number }
  /** Tokens spent on this document so far (cumulative). */
  | { job: string; type: "usage"; prompt: number; completion: number }
  | { job: string; type: "idle" };

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

/** `browser://event` payloads — the browser tabs' state pushes. */
export type BrowserEvent =
  | { type: "opened"; label: string; url: string }
  | { type: "navigated"; label: string; url: string }
  | { type: "title_changed"; label: string; title: string }
  | { type: "activated"; label: string }
  | { type: "closed"; label: string }
  | { type: "shape_changed"; shape: BrowserShape }
  | { type: "adblock_status"; status: AdblockStatusView }
  | { type: "bookmarks_changed"; bookmarks: BookmarkView[] }
  /** A tab's in-page translation pane: on or off, and how many blocks have their translation. */
  | { type: "translate"; label: string; active: boolean; done: number; total: number; tokens: number };

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
  /** The kept states of a note (the text before each write that replaced it), newest first. */
  noteHistoryList: (path: string) => invoke<HistorySnapshot[]>("note_history_list", { path }),
  /** One kept state, with its difference from the note as it is now (what a restore would change). */
  noteHistoryRead: (path: string, snapshot: string) => invoke<HistoryEntry>("note_history_read", { path, snapshot }),
  /** Put a kept state back as the note's text (the text it replaces is kept first, so this is itself undoable). */
  noteHistoryRestore: (path: string, snapshot: string) => invoke<void>("note_history_restore", { path, snapshot }),
  /** The answer to `vemry://quit-requested`: the editor buffer is on disk, the app may exit. */
  quitConfirmed: () => invoke<void>("quit_confirmed"),
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
  /** Flip a block type's `disabled:` line (`file` as typesList reports it); types_changed follows. */
  typeSetDisabled: (file: string, disabled: boolean) => invoke<void>("type_set_disabled", { file, disabled }),
  foldersList: () => invoke<string[]>("folders_list"),
  folderCreate: (path: string) => invoke<string>("folder_create", { path }),
  filesList: () => invoke<string[]>("files_list"),
  fileCreate: (path: string) => invoke<string>("file_create", { path }),
  fileRead: (path: string) => invoke<string>("file_read", { path }),
  fileReadBinary: (path: string) => invoke<ArrayBuffer>("file_read_binary", { path }),
  /** Each vault image's `[width, height]` from its header (nothing is decoded), in the
   *  order asked; null for a file that is missing or unreadable. */
  imageSizes: (paths: string[]) => invoke<([number, number] | null)[]>("image_sizes", { paths }),
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
  /** The Markdown formatter over a whole text (the editor's "format note"). */
  formatMarkdown: (text: string) => invoke<string>("format_markdown", { text }),
  /** A rich-text paste's HTML → Markdown (empty when nothing converts; the caller falls back to the plain text). */
  htmlToMarkdown: (html: string) => invoke<string>("html_to_markdown", { html }),
  clipboardReadText: () => invoke<string | null>("clipboard_read_text"),
  clipboardCopyImage: (path: string) => invoke<void>("clipboard_copy_image", { path }),
  fileWrite: (path: string, content: string) => invoke<void>("file_write", { path, content }),
  pathRename: (from: string, to: string) => invoke<string>("path_rename", { from, to }),
  pathDelete: (path: string) => invoke<void>("path_delete", { path }),
  fileOpenSystem: (path: string) => invoke<void>("file_open_system", { path }),
  urlOpenSystem: (url: string) => invoke<void>("url_open_system", { url }),
  /** The single link-opening funnel: it branches by the setting into the built-in or the system browser; invert (⌘click) inverts it. */
  urlOpen: (url: string, invert = false, focus = true) => invoke<void>("url_open", { url, invert, focus }),
  /** Navigate the active tab, or open a new one (newTab, or none open yet); returns the tab's label. */
  browserOpen: (url: string, newTab = false, focus = true) =>
    invoke<string>("browser_open", { url, newTab, focus }),
  /** A blank tab (the tab strip's +, ⌘T): it shows the panel's new-tab page until its first navigation; returns the label. */
  browserNewTab: () => invoke<string>("browser_new_tab"),
  /** Close every tab — the browser goes away (and the detached window with it). */
  browserCloseAll: () => invoke<void>("browser_close_all"),
  /** The DNS prefetch (called behind a debounce when ⌘L input pauses): it pushes the
   *  target domain's resolution into the system cache early, so WebKit skips the
   *  resolution step after return. Only a complete domain is sent (the Rust side checks
   *  for a dot), it is purely a side channel, it never creates a window or a
   *  navigation, and a failure goes unnoticed. */
  browserPrefetch: (url: string) => invoke<void>("browser_prefetch", { url }),
  /** The shape switch: docked = the panel in the main window, detached = the standalone window (the tabs move without reloading). */
  browserSetDocked: (docked: boolean) => invoke<void>("browser_set_docked", { docked }),
  /** Show a tab (the tab strip). */
  browserActivate: (label: string) => invoke<void>("browser_activate", { label }),
  browserCloseTab: (label?: string) => invoke<void>("browser_close_tab", { label }),
  /** The docked panel column's anchors (logical px): sent when they change, never per frame — Rust re-places the tab from them on a window resize. */
  browserPanelLayout: (visible: boolean, width: number, chromeH: number) =>
    invoke<void>("browser_panel_layout", { visible, width, chromeH }),
  /** A full-window overlay is open in the main window: the active tab hides underneath it (a native view would cover the overlay). */
  browserPanelVeil: (veiled: boolean) => invoke<void>("browser_panel_veil", { veiled }),
  browserNavigate: (url: string, label?: string) => invoke<void>("browser_navigate", { url, label }),
  browserBack: (label?: string) => invoke<void>("browser_back", { label }),
  browserForward: (label?: string) => invoke<void>("browser_forward", { label }),
  /** Open a translation pane inside the tab (the page on the left, each paragraph's translation aligned on the right), or close it when it is open; resolves with whether it is open afterwards. */
  browserTranslateToggle: (label?: string) => invoke<boolean>("browser_translate_toggle", { label }),
  browserReload: (label?: string) => invoke<void>("browser_reload", { label }),
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
  /** The detached bar → main window action relay; focus is true for "open the note" alone
   *  (every other action leaves the browsing uninterrupted — not raising the main window
   *  is exactly what the bar is for). */
  browserbarToMain: (action: BrowserbarAction, focus = false) =>
    invoke<void>("browserbar_to_main", { action, focus }),
  /** Expand or collapse the bar's destination picker (Rust grows the bar's bounds downwards over the page). */
  browserbarSetExpanded: (expanded: boolean) => invoke<void>("browserbar_set_expanded", { expanded }),
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
  /** Side-by-side translation: cut the source into segments and start translating (one job at a time; a previous job is cancelled). */
  translateOpen: (source: TranslateSource) => invoke<TranslateDocView>("translate_open", { source }),
  /** Ask for these segments next, in this order (the visible rows first); force = retranslate. */
  translateRequest: (job: string, ids: number[], force = false) => invoke<void>("translate_request", { job, ids, force }),
  translateCancel: (job: string) => invoke<void>("translate_cancel", { job }),
  /** Go on with the background translation past its stop: another budget's worth, or all of it. */
  translateContinue: (job: string, all: boolean) => invoke<void>("translate_continue", { job, all }),
  /** Save the translations as a note beside the source; resolves with the note's path. */
  translateSave: (job: string) => invoke<string>("translate_save", { job }),
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
  /** Accept pending agent blocks (those with these local ids — a run — or all); resolves to how many were accepted. */
  provenanceAccept: (path: string, localIds?: string[]) => invoke<number>("provenance_accept", { path, localIds }),
  /** The notes with unreviewed AI blocks across the vault, in path order (the status bar's list, the file tree's marks, the review's note-to-note step). */
  provenancePendingNotes: () => invoke<PendingNote[]>("provenance_pending_notes"),
  /** Mark every unreviewed AI block in these notes (every note, when omitted) as reviewed. */
  provenanceAcceptNotes: (paths?: string[]) => invoke<AcceptedTotal>("provenance_accept_notes", { paths }),
  /** The revert plan for `text` (the editor's buffer): one unreviewed block by local id, or all; the editor applies it as one undoable edit. */
  provenanceRevertPlan: (text: string, localIds?: string[]) => invoke<RevertPlan>("provenance_revert_plan", { text, localIds }),
  /** Record text the user placed themselves (lands accepted); a private path or text without blocks records nothing. */
  provenanceRecord: (path: string, text: string, origin: "agent" | "clip" | "capture", session?: string, url?: string) =>
    invoke<void>("provenance_record", { path, text, origin, session, url }),
  settingsGet: () => invoke<SettingsView>("settings_get"),
  settingsSet: (update: SettingsUpdate) => invoke<void>("settings_set", { update }),
  // ---- the hosted account (account.rs); every change also arrives on onAccountChanged ----
  accountStatus: () => invoke<AccountStatus>("account_status"),
  /** Ask for a sign-in code by e-mail (the server answers the same whether or not the address is known). */
  accountLoginStart: (email: string) => invoke<void>("account_login_start", { email }),
  /** Turn the code into this device's token; the account becomes the current one. */
  accountLoginVerify: (email: string, code: string) => invoke<AccountStatus>("account_login_verify", { email, code }),
  accountLogout: () => invoke<void>("account_logout"),
  /** Fetch the entitlement snapshot again (a 304 when nothing changed). */
  accountRefresh: () => invoke<AccountStatus>("account_refresh"),
  accountRedeem: (code: string) => invoke<AccountStatus>("account_redeem", { code }),
  // ---- the phone inbox (inbox.rs): items arrive on onInboxItems; the main window lands them and acks ----
  /** The main window wrote these items into the vault (the ledger first, then the service); `target` is the
   *  spot's label (spotLabel), for the settings page's "last one filed". */
  inboxAck: (ids: string[], target?: string) => invoke<void>("inbox_ack", { ids, target: target ?? null }),
  /** These could not land (no vault open, a note that cannot be written); they are offered again in a few minutes. */
  inboxFailed: (ids: string[]) => invoke<void>("inbox_failed", { ids }),
  /** This window is listening and has a vault open: items may be handed over, and what could not land is offered again now. */
  inboxReady: () => invoke<void>("inbox_ready"),
  /** A fresh single-use pairing code, as the QR code the phone scans. */
  inboxPairStart: () => invoke<PairStart>("inbox_pair_start"),
  inboxDoors: () => invoke<InboxDoor[]>("inbox_doors"),
  /** The phone's token stops working; what it already sent stays. */
  inboxDoorRevoke: (id: number) => invoke<void>("inbox_door_revoke", { id }),
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
  agentActionsRespond: (requestId: string, decision: ActionsDecision) =>
    invoke<void>("agent_actions_respond", { requestId, decision }),
  /** The read gate's answer: which URLs to fetch, and whether source-table URLs may be fetched without asking from now on. */
  agentReadRespond: (requestId: string, answer: ReadAnswer) => invoke<void>("agent_read_respond", { requestId, answer }),
  /** Put back every write one turn landed (skipping notes edited since); refused while the agent runs. */
  agentUndoTurn: (turn: string) => invoke<UndoReport>("agent_undo_turn", { turn }),
  /** The example block types and their installer (a file already there is never overwritten). */
  typeExamplesList: () => invoke<TypeExample[]>("type_examples_list"),
  typeInstallExample: (name: string) => invoke<Scaffolded>("type_install_example", { name }),
  agentCancel: () => invoke<void>("agent_cancel"),
  /** Follow a note (null stops): the session keeps its findings there and, with auto, local writes on it pass the gate without a dialog. */
  agentFollowSet: (path: string | null, auto: boolean) => invoke<void>("agent_follow_set", { path, auto }),
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
  // ---- the quick capture window (capture_window.rs): shown by a global shortcut or these entry points; landing is relayed to the main window, whose receipt comes back ----
  captureWindowShow: () => invoke<void>("capture_window_show"),
  captureWindowHide: () => invoke<void>("capture_window_hide"),
  /** The capture webview has mounted and listens: a show that arrived earlier is replayed. */
  captureWindowReady: () => invoke<void>("capture_window_ready"),
  /** The window follows the composer's content height (logical px). */
  captureWindowResize: (height: number) => invoke<void>("capture_window_resize", { height }),
  captureToMain: (action: CaptureAction) => invoke<void>("capture_to_main", { action }),
  captureReply: (result: CaptureResult) => invoke<void>("capture_reply", { result }),
  /** What the system clipboard holds, for the capture window's paste: files as absolute paths (a file manager's
   *  copy), or an image's format as the extension it is saved under (the pasteboard type, as the editor's own
   *  paste names its file). Empty off macOS, where the DOM paste data is the fallback. */
  clipboardProbe: () => invoke<{ files: string[]; image: string | null }>("clipboard_probe"),
  /** Raw image bytes on the system clipboard (a screenshot), once clipboardProbe said there is one. */
  clipboardImage: async () => new Uint8Array(await invoke<ArrayBuffer>("clipboard_image")),
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
  /** The skills list: built-ins and the vault's own files, by listed name. */
  skillsList: () => invoke<SkillEntry[]>("skills_list"),
  /** Switch one skill on or off by name (a built-in by the vault config, a vault file by its frontmatter); applies from the next turn. */
  skillSetEnabled: (name: string, enabled: boolean) => invoke<void>("skill_set_enabled", { name, enabled }),
  /** A customised copy of a built-in in `.vemry/skills/` (shadows it by name; never overwrites one that is there). */
  skillCustomize: (name: string) => invoke<Scaffolded>("skill_customize", { name }),
  /** Trash the customised copy so the built-in of that name shows again. */
  skillRestore: (name: string) => invoke<void>("skill_restore", { name }),
  /** A new vault skill from the skeleton; one already there is left as it is. */
  skillCreate: (name: string) => invoke<Scaffolded>("skill_create", { name }),
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
  /** Run a note's js block; `params` are the lab controls' current values on a re-run (null on the first run). */
  scriptEvalBlock: (code: string, currentNote: string | null, params: LabParams | null = null) =>
    invoke<BlockEvalResult>("script_eval_block", { code, currentNote, params }),
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

/** Every change of account state (sign-in, sign-out, a refresh, a revoked token noticed by any request). */
export function onAccountChanged(handler: (s: AccountStatus) => void): Promise<UnlistenFn> {
  return listen<AccountStatus>("account://changed", (event) => handler(event.payload));
}

/** Items pulled from the phone inbox (inbox://items): the main window lands them and acks or fails each. */
export function onInboxItems(handler: (batch: InboxBatch) => void): Promise<UnlistenFn> {
  return listen<InboxBatch>("inbox://items", (event) => handler(event.payload));
}
/** Rust landed captures straight into the phone's vault (it was not the open one): show the receipt. */
export function onInboxLanded(handler: (landed: InboxLanded) => void): Promise<UnlistenFn> {
  return listen<InboxLanded>("inbox://landed", (event) => handler(event.payload));
}
/** The phone's vault could not take the captures (inbox://home-failed); Rust offers them again in a few minutes. */
export function onInboxHomeFailed(handler: (failure: { vault: string; error: string }) => void): Promise<UnlistenFn> {
  return listen<{ vault: string; error: string }>("inbox://home-failed", (event) => handler(event.payload));
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

/** Main window: ⌘Q or the close button — Rust holds the exit until `quitConfirmed` (the editor buffer is written out first). */
export function onQuitRequested(handler: () => void): Promise<UnlistenFn> {
  return listen<null>("vemry://quit-requested", () => handler());
}

export function onReadingEvent(handler: (e: ReadingEvent) => void): Promise<UnlistenFn> {
  return listen<ReadingEvent>("reading://event", (event) => handler(event.payload));
}

export function onTranslateEvent(handler: (e: TranslateEvent) => void): Promise<UnlistenFn> {
  return listen<TranslateEvent>("translate://event", (event) => handler(event.payload));
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

// ---- the cross-window events of the quick capture window (emitted by capture_window.rs, directed with emit_to) ----
/** Capture window: it was just shown — focus the text, refresh the settings and the notes. */
export function onCaptureShown(handler: () => void): Promise<UnlistenFn> {
  return listen<null>("capture://shown", () => handler());
}
/** Main window: a capture to land (answered with captureReply). */
export function onCaptureAction(handler: (a: CaptureAction) => void): Promise<UnlistenFn> {
  return listen<CaptureAction>("capture://action", (event) => handler(event.payload));
}
/** Capture window: the main window's receipt. */
export function onCaptureResult(handler: (r: CaptureResult) => void): Promise<UnlistenFn> {
  return listen<CaptureResult>("capture://result", (event) => handler(event.payload));
}

// ---- the cross-window events of the detached browser window's chrome bar (`browserbar`) ----
/** Main window: an action the bar wants the main window to run (relayed by browserbar_to_main). */
export function onBrowserbarAction(handler: (a: BrowserbarAction) => void): Promise<UnlistenFn> {
  return listen<BrowserbarAction>("browserbar://action", (event) => handler(event.payload));
}
/** Bar: the receipt after the main window ran a clip action. */
export function onBrowserbarResult(handler: (r: BrowserbarResult) => void): Promise<UnlistenFn> {
  return listen<BrowserbarResult>("browserbar://result", (event) => handler(event.payload));
}
/** Main window → the bar's receipt (directed, not broadcast). */
export function emitBrowserbarResult(r: BrowserbarResult): Promise<void> {
  return emitTo("browserbar", "browserbar://result", r);
}
/** Bar: a menu-bar command routed to the detached window because it held the focus (open-url, browser-new-tab). */
export function onBrowserbarCommand(handler: (id: string) => void): Promise<UnlistenFn> {
  return listen<string>("browserbar://command", (event) => handler(event.payload));
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
