// Session JSONL parsing for the Agent-review (replay) view. One line per
// audit event, written by vemry-agent's SessionLog (runner.rs).

import { t } from "$lib/i18n/i18n.svelte";
import { isImeKeydown } from "$lib/ime";
import type { ActionItem, PlanItem, RefTag, SourceRecord } from "$lib/api";

/** Every line carries `ts` (RFC3339) + `kind`; `raw` is the bad-line fallback. */
export type SessionEntry =
  /** turn: the turn id (stamped by the runner since v0.6; older existing lines do not have one) — forking locates by it. */
  | {
      kind: "user";
      ts: string;
      content: string;
      turn?: string;
      page?: { url: string; title: string };
      /** @ references (since v0.9): the audit records kind/path alone (plus lines for
       *  a selection), while the body lives in the model history's envelope. */
      refs?: RefTag[];
    }
  | { kind: "assistant"; ts: string; content: string; tool_calls: number }
  | { kind: "tool_call"; ts: string; tool: string; args: unknown }
  | { kind: "tool_result"; ts: string; tool: string; ok: boolean; output: string }
  /** to: move_note's destination (since v0.9; the lines of every other tool have no such key). */
  | { kind: "write_prepared"; ts: string; tool: string; path: string; diff: string; to?: string }
  /** applied_diff (since v0.9): for "edit, then apply", the diff between the proposed content and what the user actually applied. */
  | { kind: "permission"; ts: string; path: string; decision: string; applied_diff?: string; via?: string }
  | { kind: "privacy"; ts: string; tool: string; filtered: number }
  | { kind: "web_search"; ts: string; queries?: string[]; count?: number; links?: { title: string; url: string }[] }
  /** The plan propose_plan raised (id = the tool call id, which the decision line pairs by). */
  | { kind: "plan"; ts: string; id: string; items: string[] }
  /** The plan gate's decision: approve / approve_edited (items = the edited list) / reject (reason). */
  | {
      kind: "plan_decision";
      ts: string;
      id: string;
      decision: "approve" | "approve_edited" | "reject";
      items?: string[];
      reason?: string;
    }
  /** The action list propose_actions raised (id = the tool call id, which the decision line pairs by). */
  | { kind: "actions"; ts: string; id: string; items: ActionItem[] }
  /** The actions gate's decision: approve (items = what the user kept, as edited) / reject (reason). */
  | { kind: "actions_decision"; ts: string; id: string; decision: "approve" | "reject"; items?: ActionItem[]; reason?: string }
  /** "Undo this turn's writes" ran: how many writes were put back and how many skipped (edited since). */
  | { kind: "undo"; ts: string; turn: string; reverted: number; skipped: number }
  /** The session follows a note (path null = stopped); the last line wins. auto: local writes on it pass the gate without a dialog. */
  | { kind: "follow"; ts: string; path: string | null; auto?: boolean }
  /** The model's step list (update_plan, or the list confirmed at the plan gate); the latest one is the sticky plan card. */
  | { kind: "plan_update"; ts: string; items: PlanItem[] }
  /** The web sources one turn cited, settled with their verification (the turn's source card on replay). */
  | { kind: "sources"; ts: string; turn?: string | null; items: SourceRecord[] }
  /** Regenerate: the lines after it belong to a replay of the same turn, and the assistant/tool tail before it was abandoned. */
  | { kind: "retry"; ts: string }
  /** Edit and resend: the previous user message's whole turn was abandoned (the audit lines stay, and the display drops them). */
  | { kind: "edit_resend"; ts: string }
  /** Context compaction: 1 = old tool results turned into handles, 2 = an LLM summary. */
  | { kind: "compact"; ts: string; level: number; saved_chars?: number }
  /** The fork marker (the new session's first line): from is the source session's id;
   *  turn is the fork point's turn id (present only when forking by id; the existing
   *  ordinal-fallback path has before_turn alone). */
  | { kind: "fork"; ts: string; from: string; before_turn: number; turn?: string }
  /** A manual rename's title override line (the last one wins) — pure metadata, skipped by the replay and the timeline. */
  | { kind: "title"; ts: string; title: string }
  | { kind: "raw"; ts: string; text: string };

/** Parse a session's JSONL. Malformed lines become `raw` entries — a corrupt
 *  log line must never make the replay view crash. */
export function parseSession(jsonl: string): SessionEntry[] {
  const out: SessionEntry[] = [];
  for (const line of jsonl.split("\n")) {
    if (!line.trim()) continue;
    try {
      const obj = JSON.parse(line) as Record<string, unknown>;
      if (obj && typeof obj === "object" && typeof obj.kind === "string") out.push(obj as unknown as SessionEntry);
      else out.push({ kind: "raw", ts: "", text: line });
    } catch {
      out.push({ kind: "raw", ts: "", text: line });
    }
  }
  return out;
}

/** Session ids are UTC timestamps ("20260810-142305.123") — show local time. */
export function formatSessionId(id: string): string {
  const m = id.match(/^(\d{4})(\d{2})(\d{2})-(\d{2})(\d{2})(\d{2})/);
  if (!m) return id;
  const d = new Date(Date.UTC(+m[1], +m[2] - 1, +m[3], +m[4], +m[5], +m[6]));
  const pad = (n: number) => String(n).padStart(2, "0");
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())} ${pad(d.getHours())}:${pad(d.getMinutes())}:${pad(d.getSeconds())}`;
}

/** When a title has fallen back to the session id (a UTC timestamp string) it is shown
 *  as local time instead — the session dropdown, the current title and the replay list
 *  share this one "an id means untitled" verdict. */
export function displayTitle(s: { id: string; title: string }): string {
  return s.title === s.id ? formatSessionId(s.id) : s.title;
}

/** The prefilled value of the rename input: an empty string when it is untitled (the
 *  title===id fallback) rather than a UTC timestamp string offered as an editable
 *  title — the same verdict displayTitle uses. */
export function editableTitle(s: { id: string; title: string }): string {
  return s.title === s.id ? "" : s.title;
}

/** The keys of an inline rename input, shared by the session dropdown and the replay
 *  list: Enter goes through blur for a single submit path; Esc cancels first (cancel
 *  clears the id, and onblur therefore does not submit by mistake) and stops
 *  propagation — so App's Escape does not switch the projection view away. An
 *  Enter or Esc during an IME composition belongs to the input method (confirming or
 *  cancelling a candidate) and must neither submit nor cancel the rename — the same
 *  guard as FileTree's rename. */
export function renameKeydown(e: KeyboardEvent, cancel: () => void): void {
  if (isImeKeydown(e)) return;
  if (e.key === "Enter") {
    e.preventDefault();
    (e.currentTarget as HTMLInputElement).blur();
  } else if (e.key === "Escape") {
    e.preventDefault();
    e.stopPropagation();
    cancel();
  }
}

/** RFC3339 timestamp -> local HH:MM:SS gutter label. */
export function hhmmss(ts: string): string {
  const d = new Date(ts);
  if (isNaN(d.getTime())) return "";
  const pad = (n: number) => String(n).padStart(2, "0");
  return `${pad(d.getHours())}:${pad(d.getMinutes())}:${pad(d.getSeconds())}`;
}

/** Display names for agent tools — shared by AgentPanel and the replay view.
 *  A tool with no catalog entry falls back to its wire name, which is what an
 *  unknown tool from a future version should show anyway. */
export function toolLabel(name: string): string {
  const key = `tool.${name}`;
  const s = t(key as never);
  return s === key ? name : s;
}

/** The replay wording of a plan-gate decision (of the same family as
 *  `decisionLabels`): the audit view's vocabulary lives here rather than scattered
 *  through the markup. */
export function planDecisionLabel(decision: string): string {
  const key = `planDecision.${decision}`;
  const s = t(key as never);
  return s === key ? decision : s;
}

/** Tools that carry their own card projection: the conversation timeline no longer
 *  pushes their raw JSON tool line (the plan card is propose_plan's projection). The
 *  live view and the replay share this one verdict. */
export const CARD_TOOLS = new Set(["propose_plan", "update_plan", "propose_actions"]);

/** Whether a plan is settled: nothing pending or in progress — a finished plan leaves the sticky card on the next question. */
export function planReconciled(items: PlanItem[]): boolean {
  return items.every((p) => p.status === "done" || p.status === "skipped");
}

export function decisionLabel(decision: string): string {
  const key = `decision.${decision}`;
  const s = t(key as never);
  return s === key ? decision : s;
}
