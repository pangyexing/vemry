<script lang="ts">
  // The conversation body (the timeline plus the composer): one implementation shared by the
  // research panel and the detached AI chat window (the second webview) — the session state
  // lives in each window's own agent store, and the source of truth is Rust's session files
  // (the unity principle: one session ever has one visible composer). Only per-instance,
  // harmless presentation state is kept here (expansion, scrolling, the block cache). "The
  // current note" comes from the editor in the main window and from the main window's push in
  // the chat window (agent.hostNote); anything that has to happen in the main window goes
  // through host.ts.
  import { api, AGENT_IMAGE_EXTS, EFFORT_OPTIONS, type EffortValue, isAgentImage, type WebSearchResult } from "$lib/api";
  import { open as openDialog } from "@tauri-apps/plugin-dialog";
  import { agent, type ComposerHandle, type DraftRef, type TimelineItem } from "$lib/stores/agent.svelte";
  import HoverCard, { type HoverCardData } from "./HoverCard.svelte";
  import { POSITIONAL_RE, TRANSIENT_CITE_RE, blocksOf, findBlock, materializeCitations, parseRefTarget } from "$lib/citation";
  import AtPicker, { linesLabel } from "./AtPicker.svelte";
  import RefIcon, { type RefIconKind } from "./RefIcon.svelte";
  import PrivacyIcon from "./PrivacyIcon.svelte";
  import { tick } from "svelte";
  import { hostDispatch } from "$lib/host";
  import { isAgentWindow } from "$lib/window";
  import { vault, baseOf, fileKind, noteBucketOf } from "$lib/stores/vault.svelte";
  import { clips } from "$lib/stores/clips.svelte";
  import { clipTarget } from "$lib/stores/clipTarget.svelte";
  import { toast } from "$lib/stores/toast.svelte";
  import { prefs } from "$lib/stores/prefs.svelte";
  import { contextMenu } from "$lib/stores/contextMenu.svelte";
  import { t } from "$lib/i18n/i18n.svelte";
  import { toolLabel } from "$lib/views/session";
  import {
    domainOf,
    extractCitations,
    handleCodeCopyClick,
    plainText,
    renderAssistantMarkdownBlocks,
    stableBlockPrefix,
    TASK_PREFIX_RE,
    type Citation,
  } from "$lib/markdown";
  import { localDate } from "$lib/views/tasks";
  import { isImeKeydown } from "$lib/ime";
  import { isFileLikePaste } from "$lib/editor/setup";
  import { isMac, k } from "$lib/shortcut";
  // Chat bodies now render $…$ formulas too (markdown.ts's renderInline), so the styling cannot
  // depend on ReadingView having been opened at some point
  import "katex/dist/katex.min.css";

  /** The same number as Rust's MAX_CONTEXT_REFS (a mirrored constant; the two change together). */
  const MAX_REFS = 8;
  let scroller = $state<HTMLDivElement | null>(null);
  let wrapEl = $state<HTMLDivElement | null>(null);
  let composerEl = $state<HTMLTextAreaElement | null>(null);
  /** The current note carried with a message: it can be dismissed with ✕ on the chips row
   *  (store.dismissedNote records the path) and comes back by itself once another note is
   *  opened. Both send and the queued resend read it from here — what the chip shows is what
   *  goes out. */
  const curNote = $derived(isAgentWindow ? agent.hostNote : vault.currentPath);
  // The privacy gate matches the selection chip's (selForSend): a UI courtesy, with the real
  // gate in Rust (the runner's is_private_rel filtering of current_note) — the chip and the send
  // parameter come from the same place, so one change has both effects
  const noteForSend = $derived(
    curNote && curNote !== agent.dismissedNote && !vault.privatePaths.has(curNote) ? curNote : null,
  );
  /** Where a paste or a "+" attachment lands: the current note's attachments/<note name> (the
   *  same convention as the editor's paste), computed in the detached window from the hostNote
   *  that was pushed. */
  const bucket = $derived(noteBucketOf(curNote));
  /** The editor's current selection ("select it and it comes along"): the live chip follows the
   *  selection, and its label says "selection" rather than "current" — it sits beside the
   *  "current note" chip, and two "current" labels would look like two copies being sent (in
   *  fact the note chip sends only a path pointer, and only the selection chip carries the
   *  text). Dismissing it with ✕ records its identity key, and it comes back by itself as soon
   *  as the selection changes. It does not appear when: the note is private (a UI courtesy,
   *  with the real gate in Rust), the same passage has already been pinned as an explicit chip
   *  with @ (avoiding a duplicate), or the explicit references already number 8 (Rust truncates
   *  at 8, and only what is visible can be sent). Both send and the queued resend read
   *  selForSend — the same contract as noteForSend's. */
  const curSel = $derived(isAgentWindow ? agent.hostSelection : vault.editorSelection);
  const selKey = (s: { path: string; from_line: number; to_line: number }) => `${s.path}:${s.from_line}-${s.to_line}`;
  let lastSel: { path: string; from_line: number; to_line: number; text: string } | null = null;
  const selForSend = $derived.by(() => {
    const s = curSel;
    if (!s || s.private || vault.privatePaths.has(s.path) || selKey(s) === agent.dismissedSelection) return null;
    const explicit = agent.contextDraft.filter((d) => d.kind !== "page");
    if (explicit.length >= MAX_REFS) return null;
    if (explicit.some((d) => d.kind === "selection" && selKey(d) === selKey(s))) return null;
    // A stable identity: the same selection reuses the previous object, so downstream deriveds
    // (the chips) do not recompute for nothing on every new snapshot object from CodeMirror or
    // from a push
    if (lastSel && selKey(lastSel) === selKey(s) && lastSel.text === s.text) return lastSel;
    return (lastSel = s);
  });
  /** The reference draft at send time: the explicit chips plus the automatic selection (last;
   *  the envelope is ordered exactly as the chips row is). The caller clears contextDraft after
   *  consuming it, while the automatic chip lives with the selection. */
  function draftForSend(): DraftRef[] {
    // A reference marked private between pick and send is no longer handed over (a chip is dead
    // data and does not disappear by itself): a UI courtesy, with the real gate in Rust at
    // resolution time (resolve_context_refs)
    const explicit = agent.contextDraft.filter((d) => d.kind === "page" || !vault.privatePaths.has(d.path));
    return selForSend ? [...explicit, { kind: "selection", ...selForSend }] : explicit;
  }
  /** The selection popover: it appears beside a selection made inside an assistant bubble (the
   *  actions for the whole answer are still in the action bar at the end of the message). Its
   *  coordinates are relative to timeline-wrap. */
  let selPop = $state<{ x: number; y: number } | null>(null);
  let expandedTools = $state<Record<number, boolean>>({});
  /** A run of same-named tool calls is collapsed by default, keyed by the timeline index of the run's first entry. */
  let expandedToolRuns = $state<Record<number, boolean>>({});
  /** A source card is collapsed by default (past 4 entries), keyed by its timeline index. */
  let expandedSources = $state<Record<number, boolean>>({});
  /** Whether the user is stuck to the bottom of the timeline — only then does it follow the streaming output, so reading back is not yanked away. */
  let atBottom = $state(true);
  /** Whether the plan card (the task list the assistant recites) is expanded. */
  let planOpen = $state(true);
  /** Whether the thinking stream is expanded into fully selectable text (a narrow, bottom-aligned strip by default). */
  let thinkingOpen = $state(false);
  /** The research methodology skill is installed automatically the first time a conversation surface opens (idempotent per vault). */
  let ensuredSkillFor: string | null = null;
  $effect(() => {
    const root = vault.root;
    if (!root || ensuredSkillFor === root) return;
    ensuredSkillFor = root;
    api
      .ensureResearchSkill()
      .then((installed) => {
        if (installed) toast.push(t("chat.skillInstalled"));
      })
      .catch(() => {});
  });
  // The queued message is sent automatically when the turn ends (the queue itself lives in the
  // store: whichever surface is mounted sends it; with none mounted the queue is kept and the
  // next conversation surface to mount sends it. The effect is not moved up into the store — it
  // needs vault.currentPath, and the vault store already refers to agent, so referring back
  // would make a cycle)
  $effect(() => {
    if (!agent.running && agent.queuedText) {
      const it = agent.queuedText;
      agent.queuedText = null;
      void dispatch(it);
    }
  });
  /** The one implementation of the send contract: consume the chips draft → send → refresh the
   *  note list (the AI may just have written to disk). Direct sending and the queued resend
   *  share it, so the two paths do not grow apart. */
  async function dispatch(text: string) {
    const draft = draftForSend();
    agent.contextDraft = [];
    await agent.send(text, noteForSend, draft);
    await vault.refreshNotes();
  }

  /** The one-shot highlight ⌘J lands with (class:summon, the same bool + setTimeout as
   *  SettingsModal's nudge) plus the "press ⌘J again for a detached window" hint (2s, dismissed
   *  by the first keystroke or a focus loss). A cursor landing quietly in the input gives no
   *  sign at all — the user would think nothing happened and go looking for a new window. */
  let summoning = $state(false);
  let summonHint = $state(false);
  let summonTimer: ReturnType<typeof setTimeout> | null = null;
  let hintTimer: ReturnType<typeof setTimeout> | null = null;

  function hideSummonHint() {
    if (hintTimer) { clearTimeout(hintTimer); hintTimer = null; }
    summonHint = false;
  }

  // At most one conversation surface is mounted under the single-instance invariant: the
  // composer handle is registered with the store, and the focus-agent command, the chat window's
  // ⌘J and the take-over on close all send the cursor into the input through it.
  // The effect body reads no reactive state (it is all in the closure), so it runs once per mount
  $effect(() => {
    const handle: ComposerHandle = {
      focus: () => composerEl?.focus(),
      isFocused: () => !!composerEl && document.activeElement === composerEl,
      attachFiles: () => void pickFiles(),
      summon: (opts) => {
        const el = composerEl;
        if (!el) return;
        el.focus();
        el.scrollIntoView({ block: "nearest" });
        summoning = true;
        if (summonTimer) clearTimeout(summonTimer);
        summonTimer = setTimeout(() => (summoning = false), 500);
        // The hint appears in the panel alone: a detached window is itself the end of "popping out", with nothing left to escalate to
        if (!isAgentWindow && opts?.hint !== false) {
          summonHint = true;
          if (hintTimer) clearTimeout(hintTimer);
          hintTimer = setTimeout(hideSummonHint, 2000);
        }
      },
    };
    agent.composer = handle;
    return () => {
      // When the panel and the chat window change gear a new instance may register first: only a
      // registration still pointing at this one is cleared, and the handle somebody else just
      // wrote is not overwritten
      if (agent.composer === handle) agent.composer = null;
      if (summonTimer) clearTimeout(summonTimer);
      if (hintTimer) clearTimeout(hintTimer);
    };
  });

  // A session or vault change (the store's viewEpoch increments) → clear the expansion state and block caches keyed by timeline index
  let seenEpoch = agent.viewEpoch;
  $effect(() => {
    if (agent.viewEpoch === seenEpoch) return;
    seenEpoch = agent.viewEpoch;
    citeGuards.clear();
    citeCache.clear();
    hover = null;
    resetTimelineUi();
  });

  /** The streaming block-render cache: an already-closed prefix block is rendered once, and
   *  every 40ms only the unclosed tail block is recomputed and remounted — a long answer is no
   *  longer re-rendered O(n²) as a whole, and text in what came before can be selected normally
   *  while streaming.
   *  The key is the timeline index; citation numbers only ever grow in document order, so a
   *  prefix block's markers stay valid.
   *  cites is the citation-extraction cache of the frozen prefix: each flush scans only the
   *  unfrozen tail rather than the whole body again (the split point never falls inside a fence
   *  or a paragraph, and a citation across the boundary could not be rendered anyway). */
  const blockCache = new Map<
    number,
    { text: string; blocks: string[]; cites: Citation[]; citeZone: boolean }
  >();

  /** Merging citations (deduplicated by URL, keeping the first non-empty title — the same rule as extractCitations'). */
  function mergeCites(into: Citation[], add: Citation[]) {
    for (const c of add) {
      const prev = into.find((p) => p.url === c.url);
      if (!prev) into.push({ ...c });
      else if (!prev.title && c.title) prev.title = c.title;
    }
  }

  function assistantBlocks(i: number, text: string): string[] {
    let cached = blockCache.get(i);
    if (!cached || !text.startsWith(cached.text)) {
      cached = { text: "", blocks: [], cites: [], citeZone: false };
      blockCache.set(i, cached);
    }
    let tail = text.slice(cached.text.length);
    const cut = stableBlockPrefix(tail);
    if (cut > 0) {
      const grown = tail.slice(0, cut);
      mergeCites(cached.cites, extractCitations(grown));
      const numberOf = new Map(cached.cites.map((c, n) => [c.url, n + 1] as const));
      // The block-level state (citeZone) travels across calls: a "## References" heading is often
      // frozen into the prefix while the list falls in the tail, and without carrying the state
      // the tail would merge a deliberate bibliography into a string of markers
      const st = { citeZone: cached.citeZone };
      cached.blocks.push(...renderAssistantMarkdownBlocks(grown, numberOf, st));
      cached.citeZone = st.citeZone;
      cached.text += grown;
      tail = tail.slice(cut);
    }
    if (!tail) return [...cached.blocks];
    const cites = cached.cites.map((c) => ({ ...c })); // the tail is not frozen, so the cache is not polluted
    mergeCites(cites, extractCitations(tail));
    const numberOf = new Map(cites.map((c, n) => [c.url, n + 1] as const));
    return [...cached.blocks, ...renderAssistantMarkdownBlocks(tail, numberOf, { citeZone: cached.citeZone })];
  }

  /** A timeline render row: a run of ≥3 tool calls is merged into one summary (interleaved calls
   *  merge too — a real investigation of read→search→read used not to fold at all, and a long
   *  one still flooded the screen), and everything else is left as it is. Folding loses no audit
   *  trail — expanding the summary is still the entry-by-entry ledger, and a run in progress
   *  keeps its current entry as live feedback. */
  type ToolItem = Extract<TimelineItem, { kind: "tool" }>;
  type TimelineRow =
    | { kind: "one"; item: TimelineItem; i: number }
    | { kind: "run"; name: string; mixed: boolean; start: number; items: { item: ToolItem; i: number }[] };
  const RUN_MIN = 3;
  const rows = $derived.by(() => {
    const out: TimelineRow[] = [];
    const tl = agent.timeline;
    let i = 0;
    while (i < tl.length) {
      const item = tl[i];
      if (item.kind === "tool") {
        const run: { item: ToolItem; i: number }[] = [];
        for (let j = i; j < tl.length; j++) {
          const it = tl[j];
          if (it.kind !== "tool") break;
          run.push({ item: it, i: j });
        }
        if (run.length >= RUN_MIN) {
          const mixed = run.some((it) => it.item.name !== item.name);
          out.push({ kind: "run", name: item.name, mixed, start: i, items: run });
          i += run.length;
          continue;
        }
      }
      out.push({ kind: "one", item, i });
      i++;
    }
    return out;
  });

  /** The summary label of an interleaved tool run: counted by name (read ×2 · search ×3). */
  function runLabel(row: Extract<TimelineRow, { kind: "run" }>): string {
    const counts = new Map<string, number>();
    for (const it of row.items) counts.set(it.item.name, (counts.get(it.item.name) ?? 0) + 1);
    return [...counts.entries()].map(([n, c]) => `${toolLabel(n)} ×${c}`).join(" · ");
  }

  /** A tool call's key argument value (a path, a query and so on), for the run's summary. */
  function argValue(args: string): string {
    try {
      const v = JSON.parse(args) as Record<string, unknown>;
      for (const key of ["path", "query", "name", "note", "text", "folder"]) {
        if (typeof v[key] === "string" && v[key]) return v[key] as string;
      }
    } catch {
      /* a truncated args preview is not valid JSON */
    }
    return "";
  }

  /** The run summary's argument glance: the first few key values, so what the run is doing is clear without expanding it. */
  function runDigest(row: Extract<TimelineRow, { kind: "run" }>): string {
    const vals: string[] = [];
    for (const it of row.items) {
      const v = argValue(it.item.args);
      if (v && !vals.includes(v)) vals.push(v);
      if (vals.length >= 3) break;
    }
    if (!vals.length) return "";
    const more = row.items.length > 3 ? "…" : "";
    return ` · ${vals.map((v) => (v.length > 18 ? `${v.slice(0, 18)}…` : v)).join(" / ")}${more}`;
  }

  /** The plan card: the task list the most recent assistant message recites.
   *  It is recomputed every 40ms while streaming, locating the heading by regex first — the vast
   *  majority of messages have no plan, and splitting the whole message into an array of lines
   *  does not belong on a hot path. The check grammar is shared with the rendering layer through
   *  TASK_PREFIX_RE.
   *  Under BASE_RULES the model recites the list every turn, and the plan card at the top is its
   *  only projection: that section is stripped out of the bubble's body (stripPlan), or one turn
   *  would show the same list three times ("the plan card + the first answer's 0/N + the
   *  follow-up's 2/N"). */
  // Anchored at the end of a line rather than at the start: the model often sticks the heading to
  // the end of a paragraph ("…the details. ## Plan"), where a start-of-line anchor would miss it
  // and the plan card would fall back to this turn's earlier 0/N version, out of step with the
  // ticks in the body
  // `#{2,}`: when the model writes ###/#### the whole heading is swallowed with it, or stripping
  // would leave a few bare #s at the top of the body
  const PLAN_HEAD_RE = /#{2,}[ \t]*(?:计划|plan)[ \t\r]*$/im;
  /** A list line not yet fully received while streaming ("- [", "- [x"…): it counts as part of
   *  the plan section, so half a line does not flash in the body and vanish. It is only judged
   *  on the last line when there is no trailing newline. */
  const PLAN_PARTIAL_RE = /^\s*[-*](\s+(\[( |x|X)?(\]\s*)?)?)?$/;
  type PlanItem = { done: boolean; text: string };
  /** One line of `- [ ] text` → `{done, text}`; null when it is not a list item. Parsing the plan
   *  (locatePlan) and filling the plan card's edit box share this one grammar — two regexes in
   *  two places would have the same text disagree between two parsers. */
  function stripBullet(line: string): PlanItem | null {
    const li = /^\s*[-*]\s+(.*)$/.exec(line);
    const task = li && TASK_PREFIX_RE.exec(li[1]);
    if (!li || !task) return null;
    return { done: task[1].toLowerCase() === "x", text: li[1].slice(task[0].length) };
  }

  /** Locate the plan section: `start` is where the heading begins and `end` is just past the list's last line (its newline included). */
  function locatePlan(text: string): { start: number; end: number; items: PlanItem[] } | null {
    const head = PLAN_HEAD_RE.exec(text);
    if (!head) return null;
    const bodyAt = head.index + head[0].length + 1;
    const lines = text.slice(bodyAt).split("\n");
    const items: PlanItem[] = [];
    let end = Math.min(bodyAt, text.length);
    let cursor = bodyAt;
    for (let n = 0; n < lines.length; n++) {
      const line = lines[n];
      const next = cursor + line.length + 1;
      const item = stripBullet(line);
      if (item) {
        items.push(item);
        end = Math.min(next, text.length);
      } else if (n === lines.length - 1 && PLAN_PARTIAL_RE.test(line)) {
        end = text.length; // a half line while streaming
      } else if (items.length || line.trim()) {
        break;
      } else {
        end = Math.min(next, text.length); // the blank line after the heading
      }
      cursor = next;
    }
    return items.length ? { start: head.index, end, items } : null;
  }
  /** The bubble's body = the text with the recited plan section removed (the plan card is its
   *  only projection). With the heading stuck to the end of a paragraph only the heading itself
   *  is cut and the sentence before it stays; while streaming, a heading not yet fully received
   *  ("## Pl") is hidden too, so an empty heading does not flash. */
  function stripPlan(text: string): string {
    const loc = locatePlan(text);
    if (!loc) {
      // The heading has arrived but the list has not started ("## Plan\n"), or the heading itself is incomplete
      return text.replace(/\n?#{2,}[ \t]*(计划?|p(?:l(?:a(?:n)?)?)?)?[ \t\r]*\n?\s*$/i, "");
    }
    const before = text.slice(0, loc.start).replace(/[ \t]+$/, "");
    const after = text.slice(loc.end).replace(/^\n+/, "");
    if (!before.trim()) return after;
    if (!after.trim()) return before.replace(/\n+$/, "");
    return `${before.replace(/\n+$/, "")}\n\n${after}`;
  }
  const plan = $derived.by(() => {
    // Only this turn is considered (after the last user message): the model recites the plan
    // every turn, so while the task continues the card stays; once the topic changes, an old plan
    // no longer hangs there
    let from = 0;
    for (let i = agent.timeline.length - 1; i >= 0; i--) {
      if (agent.timeline[i].kind === "user") {
        from = i;
        break;
      }
    }
    for (let i = agent.timeline.length - 1; i >= from; i--) {
      const it = agent.timeline[i];
      if (it.kind !== "assistant") continue;
      const items = locatePlan(it.text)?.items ?? null;
      if (items) return items;
    }
    // The structured version underneath, taken over by the recitation: in the gap after a
    // confirmation and before the model has recited it, the sticky card uses the confirmed list
    // (an edited one takes precedence); a pending or rejected plan does not enter the sticky card
    // — its inline card is the focus of the viewport
    for (let i = agent.timeline.length - 1; i >= from; i--) {
      const it = agent.timeline[i];
      if (it.kind !== "plan" || !it.decision || it.decision.kind === "reject") continue;
      const items = it.decision.kind === "approve_edited" ? it.decision.items : it.items;
      return items.map((text) => ({ done: false, text }));
    }
    return null;
  });

  /** ---- The decision state of the plan gate's (propose_plan) inline card: at most one plan is
   *  pending at a time, so this is one piece of component-level state; a new request resets it on
   *  arrival (the same as PermissionDialog's $effect). */
  let planEditing = $state(false);
  let planEditText = $state("");
  let planRejecting = $state(false);
  let planRejectReason = $state("");
  $effect(() => {
    void agent.pendingPlan;
    planEditing = false;
    planEditText = "";
    planRejecting = false;
    planRejectReason = "";
  });
  type PlanCard = Extract<TimelineItem, { kind: "plan" }>;
  function togglePlanEdit(item: PlanCard) {
    if (planEditing) {
      planEditing = false;
      return;
    }
    planEditText = item.items.join("\n");
    planEditing = true;
    planRejecting = false;
  }
  function submitPlanApprove(item: PlanCard) {
    if (planEditing) {
      // One per line; a `- [ ]` prefix the user typed out of habit is tolerated
      const items = planEditText
        .split("\n")
        .map((s) => (stripBullet(s)?.text ?? s.replace(/^\s*[-*]\s+/, "")).trim())
        .filter(Boolean);
      const changed = items.length !== item.items.length || items.some((s, idx) => s !== item.items[idx]);
      void agent.respondPlan(changed && items.length ? { kind: "approve_edited", items } : { kind: "approve" });
    } else {
      void agent.respondPlan({ kind: "approve" });
    }
  }
  function submitPlanReject() {
    void agent.respondPlan({ kind: "reject", reason: planRejectReason.trim() || t("chat.planRejectedByUser") });
  }
  function planBadge(d: NonNullable<PlanCard["decision"]>): string {
    if (d.kind === "approve") return t("chat.planApproved");
    if (d.kind === "approve_edited") return t("chat.planApprovedEdited");
    if (d.reason === "cancelled") return t("chat.planCancelled");
    const own = t("chat.planRejectedByUser");
    return `${t("chat.planRejected")}${d.reason && d.reason !== own ? ` · ${d.reason}` : ""}`;
  }

  /** The thousands abbreviation (for the usage gauge). */
  function fmtK(n: number): string {
    return n >= 10_000 ? `${Math.round(n / 1000)}k` : n >= 1000 ? `${(n / 1000).toFixed(1)}k` : String(n);
  }

  /** The status line's text, on the same template while running and after finishing (bb0eb0a
   *  requires the two to match word for word).
   *  A "step" is one model-tool round trip within a turn (the runner's iteration budget); the
   *  session list's "N turns" counts the user's questions — two quantities, two words. */
  const usageText = $derived.by(() => {
    const u = agent.usage;
    if (!u) return "";
    const cache = u.prompt > 0 && u.cacheHit > 0 ? t("chat.usageCache", { pct: Math.round((u.cacheHit / u.prompt) * 100) }) : "";
    return `${t("chat.usageStep", { n: u.iteration, max: u.max, tokens: fmtK(u.prompt) })}${cache}`;
  });

  /** On a session change, clear the expansion state keyed by timeline index, so it is not attached to the new timeline at the wrong place. */
  function resetTimelineUi() {
    expandedTools = {};
    expandedToolRuns = {};
    expandedSources = {};
    blockCache.clear();
    atBottom = true;
  }

  // ---- Getting an answer back into a note (the last metre of the research loop) ----

  /** The passage of an answer selected in the timeline (it only counts when it exists and falls inside an assistant bubble). */
  function selectedInTimeline(): string {
    const sel = window.getSelection();
    if (!sel || sel.isCollapsed) return "";
    const anchor = sel.anchorNode;
    const el = anchor instanceof Element ? anchor : (anchor?.parentElement ?? null);
    if (!el?.closest(".msg.assistant")) return "";
    return sel.toString();
  }

  /** The selection popover appears once mouseup settles the selection (the rectangle is computed
   *  here alone; the hot selectionchange path only does the cheap check of dismissing it when the
   *  selection collapses — the performance lesson of the v0.6 clipping side). */
  function onTimelineMouseUp() {
    setTimeout(() => {
      const sel = window.getSelection();
      if (!sel || sel.isCollapsed || !wrapEl) {
        selPop = null;
        return;
      }
      const anchor = sel.anchorNode;
      const el = anchor instanceof Element ? anchor : (anchor?.parentElement ?? null);
      if (!el?.closest(".msg.assistant")) {
        selPop = null;
        return;
      }
      const rect = sel.getRangeAt(0).getBoundingClientRect();
      if (!rect.width && !rect.height) {
        selPop = null;
        return;
      }
      const wrap = wrapEl.getBoundingClientRect();
      const x = Math.min(Math.max(rect.left + rect.width / 2 - wrap.left, 84), Math.max(wrap.width - 84, 84));
      let y = rect.top - wrap.top - 32;
      if (y < 4) y = rect.bottom - wrap.top + 8;
      selPop = { x, y };
    }, 0);
  }
  $effect(() => {
    const onSelChange = () => {
      const sel = window.getSelection();
      if (selPop && (!sel || sel.isCollapsed)) selPop = null;
    };
    document.addEventListener("selectionchange", onSelChange);
    return () => document.removeEventListener("selectionchange", onSelChange);
  });

  /** A popover action: confirm the selection is still there (the button's mousedown already
   *  called preventDefault to keep it) and dismiss the popover afterwards. The actions themselves
   *  are the same set as the end-of-message bar's (the selection semantics live in
   *  answerPayload). */
  function popAction(fn: (fullText: string) => unknown) {
    if (!selectedInTimeline()) {
      selPop = null;
      return;
    }
    void fn("");
    selPop = null;
  }

  /** A selection → a quote block (with the research session's signature); a whole answer lands as its original Markdown. */
  function answerPayload(fullText: string): string {
    const sel = selectedInTimeline();
    if (!sel) return `${fullText.trim()}\n`;
    const body = sel
      .trim()
      .split("\n")
      .map((l) => `> ${l}`)
      .join("\n");
    return `${body}\n> — ${t("chat.answerSignature")} *(${localDate()})*\n`;
  }

  async function copyAnswer(text: string) {
    const sel = selectedInTimeline();
    try {
      await navigator.clipboard.writeText(await exportAnswer(sel || text));
      toast.push(t(sel ? "chat.copiedSelection" : "chat.copiedAnswer"));
    } catch (e) {
      toast.push(String(e), "error");
    }
  }

  /** Insert at the cursor in the current note. It has to pass the canInsertIntoVisibleNote guard
   *  — under a projection the editor is only hidden by CSS, and a bare insertAtCursor cannot tell
   *  a visible note from a hidden one. */
  async function insertAnswer(text: string) {
    const payload = await exportAnswer(answerPayload(text));
    void hostDispatch({ kind: "insert", payload, session: agent.sessionId ?? undefined });
  }

  /** Send it to the pinned destination (sharing clipTarget.deliver's pipeline and destination contract with a clipped selection). */
  async function sendToPinned(text: string) {
    const payload = await exportAnswer(answerPayload(text));
    void hostDispatch({ kind: "pin", payload, session: agent.sessionId ?? undefined });
  }

  /** This turn's context: look back to the most recent user question and forward to the next turn to collect the sources. */
  function turnInfoFor(i: number): { question: string | null; sources: string[] } {
    const tl = agent.timeline;
    let question: string | null = null;
    let start = 0;
    for (let j = i; j >= 0; j--) {
      const it = tl[j];
      if (it.kind === "user") {
        question = it.text;
        start = j;
        break;
      }
    }
    const sources: string[] = [];
    for (let j = start; j < tl.length; j++) {
      const it = tl[j];
      if (j > start && it.kind === "user") break;
      if (it.kind === "sources") for (const r of it.results) if (!sources.includes(r.url)) sources.push(r.url);
    }
    return { question, sources };
  }

  /** Save as a new note: it lands in research/, with the frontmatter recording the question and this turn's sources. */
  async function saveAsNote(rawText: string, i: number) {
    const text = await exportAnswer(rawText);
    const { question, sources } = turnInfoFor(i);
    // The fallback comes after the cleaning: a question like "???" or "[link]" is empty once
    // cleaned, and a fallback before it would be displaced by the truncated original, landing as
    // a hidden research/.md file (which the list does not show at all)
    const title =
      (question?.split("\n")[0].slice(0, 30) ?? "")
        .replace(/[\\/:*?"<>|#[\]]/g, " ")
        .replace(/\s+/g, " ")
        .trim() || t("chat.researchNoteName", { date: localDate() });
    let path = `research/${title}.md`;
    if (vault.notes.some((n) => n.id === path)) {
      const it = new Date();
      const pad = (n: number) => String(n).padStart(2, "0");
      path = `research/${title} ${pad(it.getHours())}${pad(it.getMinutes())}${pad(it.getSeconds())}.md`;
    }
    const fm = ["---", "vemry:", "  origin: agent"];
    if (agent.sessionId) fm.push(`  agent_session: ${agent.sessionId}`);
    if (question) fm.push(`question: ${JSON.stringify(question.split("\n")[0])}`);
    if (sources.length) {
      fm.push("sources:");
      for (const u of sources) fm.push(`  - ${JSON.stringify(u)}`);
    }
    fm.push("---", "");
    try {
      await api.noteWrite(path, `${fm.join("\n")}${text.trim()}\n`);
      // Saved by the user's hand: the AI's blocks, accepted by the saving itself
      void api.provenanceRecord(path, text, "agent", agent.sessionId ?? undefined).catch(() => undefined);
      await vault.refreshNotes();
      await hostDispatch({ kind: "open-note", path });
      toast.push(t("chat.savedAs", { path }));
    } catch (e) {
      toast.push(String(e), "error");
    }
  }

  // ---- Retry / edit and resend / fork / continue ----

  function doRetry() {
    blockCache.clear();
    run(agent.retry().then(() => vault.refreshNotes()));
  }

  async function editLast() {
    blockCache.clear();
    try {
      const restored = await agent.editResend();
      if (restored != null) {
        agent.draft = restored.text;
        // The @ references are restored as chips (a web context's body is not in the timeline and cannot be restored)
        if (restored.refs?.length) agent.contextDraft = [...restored.refs];
        composerEl?.focus();
      }
    } catch (e) {
      toast.push(String(e), "error");
    }
  }

  /** This user item's ordinal among all user items (used only to decide whether the fork button
   *  is shown, since the first turn gets none; forking itself locates by the timeline index plus
   *  the turn id and counts no ordinals). */
  function userOrdinal(i: number): number {
    let n = 0;
    for (let j = 0; j < i; j++) if (agent.timeline[j].kind === "user") n++;
    return n;
  }

  async function forkAt(i: number) {
    blockCache.clear();
    const item = agent.timeline[i];
    const text = item.kind === "user" ? item.text : "";
    try {
      await agent.fork(i);
      toast.push(t("chat.forked"));
      if (text) {
        agent.draft = text;
        composerEl?.focus();
      }
    } catch (e) {
      toast.push(String(e), "error");
    }
  }

  /** The last turn's output was cut off by the length limit: sending "continue" asks for the rest. */
  function continueTruncated() {
    void agent.send(t("chat.continueWord"), noteForSend).then(() => vault.refreshNotes());
  }

  const lastAssistantIndex = $derived.by(() => {
    for (let i = agent.timeline.length - 1; i >= 0; i--) {
      if (agent.timeline[i].kind === "assistant") return i;
    }
    return -1;
  });

  /** Whether this assistant bubble is the turn's final answer (with no other assistant between it
   *  and the next user message) — the action bar hangs here alone and not on an intermediate
   *  narration bubble. */
  function isTurnFinal(i: number): boolean {
    const tl = agent.timeline;
    for (let j = i + 1; j < tl.length; j++) {
      const kind = tl[j].kind;
      if (kind === "user") return true;
      if (kind === "assistant") return false;
    }
    return true;
  }
  const lastUserIndex = $derived.by(() => {
    for (let i = agent.timeline.length - 1; i >= 0; i--) {
      if (agent.timeline[i].kind === "user") return i;
    }
    return -1;
  });

  const run = (p: Promise<unknown>) => void p.catch((e) => toast.push(String(e), "error"));

  /** Whether the input looks like a URL: it has a scheme, or a domain (with an alphabetic .tld) plus an optional path. */
  function isUrlLike(it: string): boolean {
    if (!it || /\s/.test(it)) return false;
    return /^https?:\/\/\S+$/i.test(it) || /^[\w-]+(\.[\w-]+)*\.[a-z]{2,}(\/\S*)?$/i.test(it);
  }
  const urlLike = $derived(isUrlLike(agent.draft.trim()));
  /** A leading `#` means recording one convention into VEMRY.md.
   *  The same dual mode as a URL's: ⌥↵ still sends it verbatim as a question. */
  const memoText = $derived(agent.draft.trim().startsWith("#") ? agent.draft.trim().replace(/^#\s*/, "") : "");
  const memoLike = $derived(memoText.length > 0);

  /** One way of opening a citation: already clipped → the local note (⌘click forces the web page);
   *  everything else goes through the url_open funnel (respecting the setting and not stealing the
   *  focus — reading is something the user deliberately switched to). */
  function openCitation(url: string, invert: boolean) {
    const local = clips.pathFor(url);
    if (local && !invert) {
      void hostDispatch({ kind: "open-note", path: local });
      return;
    }
    run(api.urlOpen(url, invert, false));
  }

  /** Every rendered link goes through click delegation (generated HTML cannot carry component events). */
  function onTimelineClick(e: MouseEvent) {
    if (!(e.target instanceof Element)) return;
    if (handleCodeCopyClick(e)) return; // the code block's copy button (static HTML the renderer generated)
    const el = e.target.closest("a[data-url], a[data-wiki]");
    if (!(el instanceof HTMLElement)) return;
    e.preventDefault();
    if (el.dataset.url) openCitation(el.dataset.url, e.metaKey || e.ctrlKey);
    else if (el.dataset.wiki) {
      const { note, frag } = parseRefTarget(el.dataset.wiki);
      hover = null;
      void hostDispatch({ kind: "follow-link", target: el.dataset.wiki, guard: citeGuards.get(`${note}#${frag}`) });
    }
  }

  // ---- Block citations: the guard captured at turn end, the hover card, the export funnel ----

  /** A positional citation's content the moment the answer arrived (key `note#bN`, the note
   *  as the model wrote it): a block that moves before the click or the export is still
   *  found by its words. Cleared with the session. */
  const citeGuards = new Map<string, string>();
  /** The hover card's resolved texts, per citation; cleared when the index moves. */
  const citeCache = new Map<string, Promise<string | null>>();
  let hover = $state<HoverCardData | null>(null);
  let hoverTimer: ReturnType<typeof setTimeout> | null = null;

  $effect(() => {
    if (agent.running) return;
    const i = lastAssistantIndex;
    if (i < 0) return;
    const item = agent.timeline[i];
    if (item?.kind !== "assistant") return;
    const seen = new Set<string>();
    TRANSIENT_CITE_RE.lastIndex = 0;
    let m: RegExpExecArray | null;
    while ((m = TRANSIENT_CITE_RE.exec(item.text))) {
      const note = m[1].trim();
      const key = `${note}#${m[2]}`;
      if (seen.has(key) || citeGuards.has(key)) continue;
      seen.add(key);
      void captureGuard(note, m[2], key);
    }
  });
  $effect(() => {
    void vault.indexVersion;
    citeCache.clear();
  });

  async function captureGuard(note: string, bN: string, key: string) {
    try {
      const path = await api.resolveLink(note, vault.currentPath ?? undefined);
      if (!path) return;
      const hit = findBlock(await blocksOf(path, vault.indexVersion), bN);
      if (hit) citeGuards.set(key, hit.block.content);
    } catch {
      // the guard is a nicety: without it the click still lands by id
    }
  }

  async function citationText(note: string, frag: string): Promise<string | null> {
    const key = `${note}#${frag}`;
    let p = citeCache.get(key);
    if (!p) {
      p = (async () => {
        if (POSITIONAL_RE.test(frag)) {
          const path = note ? await api.resolveLink(note, vault.currentPath ?? undefined) : vault.currentPath;
          if (!path) return null;
          const hit = findBlock(await blocksOf(path, vault.indexVersion), frag, citeGuards.get(key));
          return hit ? hit.block.content : null;
        }
        const r = await api.resolveRef(`${note}#${frag}`, vault.currentPath ?? undefined);
        return r.state === "resolved" ? r.content : null;
      })().catch(() => null);
      citeCache.set(key, p);
    }
    const text = await p;
    return text == null ? null : plainText(text).slice(0, 300);
  }

  function onTimelineOver(e: MouseEvent) {
    if (!(e.target instanceof Element)) return;
    const el = e.target.closest<HTMLElement>("a.md-blockref");
    if (!el) return;
    if (hoverTimer) clearTimeout(hoverTimer);
    const target = el.dataset.wiki ?? "";
    hoverTimer = setTimeout(() => void showHover(el, target), 250);
  }
  function onTimelineOut(e: MouseEvent) {
    if (!(e.target instanceof Element) || !e.target.closest("a.md-blockref")) return;
    if (hoverTimer) clearTimeout(hoverTimer);
    hoverTimer = null;
    hover = null;
  }
  async function showHover(el: HTMLElement, target: string) {
    if (!el.isConnected) return;
    const r = el.getBoundingClientRect();
    hover = { x: r.left, y: r.bottom + 6, title: target, text: t("hover.loading") };
    const { note, frag } = parseRefTarget(target);
    const text = await citationText(note, frag);
    if (!hover || hover.title !== target) return;
    hover = { ...hover, text: text ?? t("hover.missing"), missing: text == null };
  }

  /** The one funnel for text leaving the panel into the vault or the clipboard:
   *  positional citations become stable ids written into the cited notes (a note that
   *  is open and dirty flushes first, or autosave would clobber the ` ^id`). */
  async function exportAnswer(text: string): Promise<string> {
    TRANSIENT_CITE_RE.lastIndex = 0;
    if (!TRANSIENT_CITE_RE.test(text)) return text;
    if (vault.currentPath && vault.dirty) await vault.flushEditor?.();
    const r = await materializeCitations(text, (note, frag) => citeGuards.get(`${note}#${frag}`), vault.currentPath ?? undefined);
    if (r.minted) toast.push(t("chat.citationsMaterialized", { n: r.minted }));
    if (r.failed) toast.push(t("chat.citationsDropped", { n: r.failed }), "error");
    return r.text;
  }

  async function clipSource(r: WebSearchResult) {
    try {
      const path = await api.clipUrl(r.url, r.title);
      clips.markClipped(r.url, path);
      toast.push(t("chat.clipped", { path }));
    } catch (e) {
      toast.push(String(e), "error");
    }
  }

  function openClipped(url: string) {
    const p = clips.pathFor(url);
    if (p) void hostDispatch({ kind: "open-note", path: p });
  }

  function openPage(url: string) {
    agent.draft = "";
    run(api.browserOpen(url));
  }

  const welcomeExamples = $derived([t("chat.starter1"), t("chat.starter2"), t("chat.starter3")]);

  function fillExample(text: string) {
    agent.draft = text;
    composerEl?.focus();
  }

  // The placeholder carousel: the main sentence leads, and the three syntaxes @ / # / a URL take
  // turns showing themselves — each is one line, so the placeholder never grows into a two-line
  // explanation. The native placeholder attribute cannot do transitions, so a .ph layer over the
  // textarea stands in for it (the textarea itself sets no placeholder).
  // It only rotates while the draft is empty and nothing is running; one keystroke stops it and
  // resets it, so it starts from the main sentence again next time; while running it becomes a
  // static "running"; under reduced-motion it does not rotate and shows the main sentence
  const PH_HINTS = $derived([
    t("chat.placeholder1"),
    t("chat.placeholder2"),
    t("chat.placeholder3"),
    t("chat.placeholder4"),
  ]);
  const PH_INTERVAL = 4000;
  let phIdx = $state(0);
  $effect(() => {
    if (agent.draft !== "" || agent.running) {
      phIdx = 0;
      return;
    }
    if (window.matchMedia("(prefers-reduced-motion: reduce)").matches) return;
    const it = setInterval(() => (phIdx = (phIdx + 1) % PH_HINTS.length), PH_INTERVAL);
    return () => clearInterval(it);
  });

  async function send() {
    const text = agent.draft.trim();
    if (!text) return;
    // Nothing is muted while a turn runs: one message is queued (sending again overwrites it) and sent automatically when the turn ends
    if (agent.running) {
      agent.queuedText = text;
      agent.draft = "";
      return;
    }
    agent.draft = "";
    atBottom = true; // a message you sent yourself always has to be visible
    await dispatch(text);
  }

  /** A `#` convention goes straight into VEMRY.md (a deliberate user action, past no agent or permission gate). */
  async function appendMemo() {
    const text = memoText; // captured before the draft is cleared, since the derived empties with it
    const original = agent.draft;
    agent.draft = "";
    // The chat window's path: the relay is one-way, and the result comes back from the main
    // window as a memo-result receipt (AgentWindow handles it: on failure pendingMemoDraft goes
    // back into the input)
    if (isAgentWindow) agent.pendingMemoDraft = original;
    try {
      // Run in the main window (with VEMRY.md open and dirty it has to flush first)
      await hostDispatch({ kind: "memo", text });
    } catch {
      agent.draft = original; // do not lose the user's input on a failure (the main-window path, or the relay itself failing)
      agent.pendingMemoDraft = null;
    }
  }

  /** The input's current meaning. What ⏎ does, what the hint strip says and what the send
   *  button's title and aria are — four places read this one table rather than each copying the
   *  same `urlLike ? … : memoLike ? …` chain (the title and the aria-label used to be kept
   *  consistent by eye). */
  const mode = $derived(urlLike ? "url" : memoLike ? "memo" : agent.running ? "queue" : "send");
  const MODES = $derived<Record<string, { hint: string; title: string; aria: string; run: () => void }>>({
    url: {
      hint: t(isMac ? "chat.urlHintMac" : "chat.urlHintPc"),
      title: t("chat.openPageTitle"),
      aria: t("chat.openPageAria"),
      run: () => openPage(agent.draft.trim()),
    },
    memo: {
      hint: t(isMac ? "chat.memoHintMac" : "chat.memoHintPc"),
      title: t("chat.memoTitle"),
      aria: t("chat.memoAria"),
      run: () => void appendMemo(),
    },
    queue: { hint: "", title: t("chat.queueTitle"), aria: t("chat.queueAria"), run: () => void send() },
    send: { hint: "", title: t("chat.sendTitle"), aria: t("chat.sendAria"), run: () => void send() },
  });
  const modeInfo = $derived(MODES[mode]);

  function submit(asQuestion: boolean) {
    if (asQuestion) void send();
    else modeInfo.run();
  }

  // ---- the @ context picker ----
  let picker = $state<AtPicker | null>(null);
  /** null = not triggered; non-null = the filter text between the @ and the cursor. */
  let atQuery = $state<string | null>(null);
  /** The index of the @ character in the draft (used to delete the token once something is chosen). */
  let atStart = -1;
  /** The @ only triggers at the start of a line or after whitespace or a common opening bracket,
   *  and the token is a run of non-whitespace — triggering mid-string, it does not interfere with
   *  the URL and # memo dual modes, which test the whole string as a prefix. */
  const AT_RE = /(^|[\s(（【[])@([^\s@]*)$/;
  function detectAt() {
    const el = composerEl;
    if (!el) return;
    const upto = agent.draft.slice(0, el.selectionStart ?? agent.draft.length);
    const m = AT_RE.exec(upto);
    if (m) {
      atQuery = m[2];
      atStart = upto.length - m[2].length - 1;
    } else {
      atQuery = null;
      atStart = -1;
    }
  }
  function closeAt() {
    atQuery = null;
    atStart = -1;
  }
  /** The toolbar's "+" button: it inserts an @ at the cursor to trigger the same candidate layer
   *  (going through exactly the same detectAt/onAtPick path as typing @ by hand, with the token
   *  deleted after a choice just the same); the candidates' first row, "add a file from this
   *  computer…", opens the native file dialog — the one entry point for context from inside and
   *  outside the vault. When the preceding character is not whitespace or an opening bracket a
   *  space is added, or AT_RE would not recognise it. */
  async function openAt() {
    const el = composerEl;
    if (!el) return;
    if (atQuery != null) {
      closeAt();
      return;
    }
    const before = agent.draft.slice(0, el.selectionStart ?? agent.draft.length);
    await insertAtCaret((before && !/[\s(（【[]$/.test(before) ? " " : "") + "@");
    detectAt();
  }
  /** The short name shown on a chip: only a path's last segment (the hover title gives the full
   *  path), with a folder carrying a trailing slash.
   *  A selection's line range is not part of it — that is its own non-shrinking span (Chip.lines),
   *  so when a long file name is eaten by the ellipsis the `(12-20)` is still there, without which
   *  it could not be told apart from the same-named "current note" chip. */
  function refLabel(d: DraftRef): string {
    if (d.kind === "page") return d.page.title || d.page.url;
    const name = baseOf(d.path);
    return d.kind === "folder" ? `${name}/` : name;
  }
  /** A chip's hover: the path; a selection adds its line range and the start of its text (a glance at which passage is attached). */
  function refTitle(d: DraftRef): string {
    if (d.kind === "page") return d.page.url;
    // The image chip's permanent hint: no table of model capabilities is attempted (it cannot be judged reliably), and a text-only model will error on one
    if (d.kind === "image") return t("chat.needsMultimodal", { path: d.path });
    if (d.kind !== "selection") return d.path;
    const head = d.text.length > 300 ? `${d.text.slice(0, 300)}…` : d.text;
    return `${d.path} ${linesLabel(d)}\n\n${head}`;
  }
  /** The identity shared by the chips and the deduplication: two different selections in one note are two chips. */
  function refKey(d: DraftRef): string {
    if (d.kind === "page") return `page:${d.page.url}`;
    if (d.kind === "selection") return `selection:${selKey(d)}`;
    return `${d.kind}:${d.path}`;
  }
  /** One chip on the context row. The three sources — the current note, the current selection and
   *  an @ reference — converge here, so the template exists once: adding a hover state, changing
   *  a truncation rule or adding an aria touches one place (with three copies, the "current note"
   *  one had already lost the line-range slot). */
  interface Chip {
    key: string;
    kind: RefIconKind;
    /** Carried automatically (the current note or selection): one shade lighter, to set it apart from an explicit @ reference. */
    cur: boolean;
    title: string;
    label: string;
    lines: string | null;
    tag: string | null;
    removeLabel: string;
    removeTitle?: string;
    remove: () => void;
  }
  const chips = $derived.by(() => {
    const out: Chip[] = [];
    const note = noteForSend;
    if (note) {
      out.push({
        key: "cur:note",
        kind: "note",
        cur: true,
        title: t("chat.currentNoteContext", { note }),
        label: baseOf(note),
        lines: null,
        tag: t("chat.tagCurrent"),
        removeLabel: t("chat.dropCurrentNote"),
        removeTitle: t("chat.dropCurrentNoteHint"),
        remove: () => (agent.dismissedNote = note),
      });
    }
    const s = selForSend;
    if (s) {
      const d: DraftRef = { kind: "selection", ...s };
      out.push({
        key: "cur:selection",
        kind: "selection",
        cur: true,
        title: refTitle(d),
        label: refLabel(d),
        lines: linesLabel(s),
        tag: t("chat.tagSelection"),
        removeLabel: t("chat.dropSelection"),
        removeTitle: t("chat.dropSelectionHint"),
        remove: () => (agent.dismissedSelection = selKey(s)),
      });
    }
    agent.contextDraft.forEach((d, di) => {
      out.push({
        key: refKey(d),
        kind: d.kind,
        cur: false,
        title: refTitle(d),
        label: refLabel(d),
        lines: d.kind === "selection" ? linesLabel(d) : null,
        tag: d.kind === "page" && d.page.truncated ? t("chat.tagTruncated") : null,
        removeLabel: t("chat.removeContext"),
        remove: () => agent.contextDraft.splice(di, 1),
      });
    });
    return out;
  });

  /** A candidate was chosen: delete the @token, restore the cursor and close the candidate layer
   *  (shared by choosing a vault reference and by "add a file from this computer"). */
  function consumeAtToken() {
    const el = composerEl;
    // The candidate layer was already closed by a blur or Esc (during the await of grabbing "the
    // current web page", say): there is no @token left in the draft to delete, and a negative-index
    // slice would double the draft
    if (atStart < 0) return;
    const end = atStart + 1 + (atQuery?.length ?? 0);
    const caret = atStart;
    agent.draft = agent.draft.slice(0, atStart) + agent.draft.slice(end);
    closeAt();
    queueMicrotask(() => {
      el?.focus();
      el?.setSelectionRange(caret, caret);
    });
  }
  function onAtPick(ref: DraftRef) {
    consumeAtToken();
    pushRef(ref);
  }
  function onAtPickFiles() {
    consumeAtToken();
    void pickFiles();
  }
  /** Add one item to the chips: the deduplication and the cap shared by the three producers (the @
   *  candidates, the "+" attachment and pasting). Returns whether it was really added. */
  function pushRef(ref: DraftRef): boolean {
    if (ref.kind === "page") {
      agent.attachPage(ref.page);
      return true;
    }
    if (agent.contextDraft.some((d) => refKey(d) === refKey(ref))) return false;
    // The automatic selection chip takes a slot (choosing it pins the live chip as an explicit one and takes no new slot)
    const auto = selForSend && !(ref.kind === "selection" && selKey(ref) === selKey(selForSend)) ? 1 : 0;
    if (agent.contextDraft.filter((d) => d.kind !== "page").length + auto >= MAX_REFS) {
      toast.push(t("chat.maxRefs", { n: MAX_REFS }));
      return false;
    }
    agent.contextDraft.push(ref);
    return true;
  }

  // ---- The "+" attachment and pasting images: they land through the same path as the editor's paste, and the landed path becomes a chip ----
  /** Warned once only: the DeepSeek endpoint is known to have no vision model, so sending an attached image errors (the user decides to send anyway). */
  let warnedVision = false;
  /** Common binary extensions (paired with the "all files" filter): sending one as a file
   *  reference would only earn a "not UTF-8 text", so it is said plainly at attach time. */
  const BINARY_EXTS = new Set(["doc", "docx", "xls", "xlsx", "ppt", "pptx", "key", "pages", "numbers", "zip", "gz", "tgz", "bz2", "xz", "7z", "rar", "tar", "exe", "dll", "dmg", "iso", "bin", "so", "dylib", "class", "jar", "o", "a", "woff", "woff2", "ttf", "otf", "eot", "mp3", "mp4", "m4a", "mov", "avi", "mkv", "wav", "flac", "ogg", "webm", "epub", "sqlite", "db"]);
  /** A freshly landed vault path → chips. Routed by fileKind: an image (png/jpg/gif/webp only)
   *  becomes an image reference, a .md a note reference, and a PDF or other text a file reference
   *  (Rust extracts a PDF's text); every other image format is stored in the vault but becomes no
   *  chip (the wire cannot take it), which a toast explains. */
  function addRefs(rels: string[]) {
    for (const rel of rels) {
      const name = baseOf(rel);
      const kind = fileKind(rel);
      if (kind === "image") {
        if (!isAgentImage(rel)) {
          toast.push(t("chat.unsupportedImage", { name }));
          continue;
        }
        if (pushRef({ kind: "image", path: rel }) && prefs.providerDeepseek && !warnedVision) {
          warnedVision = true;
          toast.push(t("chat.deepseekNoImages"));
        }
      } else if (kind !== "note" && kind !== "pdf" && BINARY_EXTS.has((rel.split(".").pop() ?? "").toLowerCase())) {
        // A known binary format is explained on the spot: sent, Rust would only answer "not UTF-8
        // text", the model would see one "attachment unavailable" line, and the user would get no
        // hint at all. An unknown extension still goes through (source code, configuration and
        // other plain text), and something genuinely unreadable is covered by the envelope line at
        // send time
        toast.push(t("chat.unreadableFile", { name }));
      } else {
        pushRef({ kind: kind === "note" ? "note" : "file", path: rel });
      }
    }
  }
  /** "+": the native file dialog → import into the current note's attachment bucket → chips. A
   *  chosen file already inside the vault is referenced rather than copied (Rust's
   *  reuse_in_vault). */
  async function pickFiles() {
    let picked: string | string[] | null;
    try {
      picked = await openDialog({
        multiple: true,
        title: t("cmd.agent-attach-files.title"),
        filters: [
          { name: t("chat.fileImages"), extensions: [...AGENT_IMAGE_EXTS] },
          { name: "PDF", extensions: ["pdf"] },
          { name: t("chat.fileText"), extensions: ["md", "txt", "csv", "tsv", "json", "yaml", "yml", "toml", "xml", "html", "css", "js", "ts", "py", "rs", "log"] },
          { name: t("chat.fileAll"), extensions: ["*"] },
        ],
      });
    } catch (e) {
      toast.push(String(e), "error");
      return;
    }
    const paths = picked == null ? [] : Array.isArray(picked) ? picked : [picked];
    if (!paths.length) return;
    vault.error = null; // a sticky field: an earlier, unrelated error must not be taken for this import's result
    const rels = await vault.importFiles(paths, bucket);
    if (vault.error) toast.push(vault.error, "error");
    addRefs(rels);
    composerEl?.focus();
  }
  /** Pasting: the gate matches the editor setup.ts's filePaste — it only takes over when the
   *  clipboard holds a file or an image (a plain-text paste is not intercepted and keeps its
   *  native behaviour); it lands through pasteIntoNote (reading files and screenshots natively,
   *  falling back to DOM Files), with the bucket taken from the current note. When nothing landed
   *  but there is text, the text is inserted back at the cursor, so a paste never vanishes into
   *  nothing. */
  async function onPaste(e: ClipboardEvent) {
    const dt = e.clipboardData;
    if (!dt || !isFileLikePaste(dt)) return;
    const text = dt.getData("text/plain");
    e.preventDefault();
    vault.error = null; // as in pickFiles: a sticky error must not pose as this paste's result
    const rels = await vault.pasteIntoNote(Array.from(dt.files), bucket);
    if (vault.error) toast.push(vault.error, "error");
    if (rels.length) addRefs(rels);
    else if (text) insertAtCaret(text);
  }
  /** Insert a piece of text at the composer's cursor and put the cursor after it. */
  async function insertAtCaret(str: string) {
    const el = composerEl;
    const at = el?.selectionStart ?? agent.draft.length;
    agent.draft = agent.draft.slice(0, at) + str + agent.draft.slice(at);
    await tick();
    el?.focus();
    el?.setSelectionRange(at + str.length, at + str.length);
  }

  // ---- The thinking level's quick switch (an in-session override; the settings hold the global default, and the level table lives in api.ts) ----
  const effortName = (v: string) =>
    EFFORT_OPTIONS.some((o) => o.value === v) ? t(`settings.effort.${v as EffortValue}`) : v;
  const effortLabel = $derived.by(() => {
    const cur = agent.effortOverride ?? prefs.reasoningEffort;
    return `${t("chat.effortLabel", { level: effortName(cur) })}${agent.effortOverride ? t("chat.effortThisSession") : ""}`;
  });
  function effortMenu(e: MouseEvent) {
    const check = (v: string | null) => (agent.effortOverride === v ? "✓ " : "");
    contextMenu.open(e, [
      { title: t("chat.effortMenuTitle") },
      { label: `${check(null)}${t("chat.effortFollowGlobal", { level: effortName(prefs.reasoningEffort) })}`, action: () => (agent.effortOverride = null) },
      { separator: true },
      ...EFFORT_OPTIONS.map((o) => {
        const hint = t(`settings.effort.${o.value}Hint`);
        return {
          label: `${check(o.value)}${effortName(o.value)}${hint ? `(${hint})` : ""}`,
          action: () => (agent.effortOverride = o.value),
        };
      }),
    ]);
  }

  function onkeydown(e: KeyboardEvent) {
    if (summonHint) hideSummonHint();
    // With the candidate overlay open the keyboard gives way first (↑/↓/Enter/Tab/Esc belong to it, and everything else types as usual)
    if (atQuery != null && picker?.handleKey(e)) return;
    if (e.key !== "Enter" || e.shiftKey || isImeKeydown(e)) return;
    e.preventDefault();
    submit(e.altKey);
  }

  function onScroll() {
    hover = null;
    if (!scroller) return;
    atBottom = scroller.scrollHeight - scroller.scrollTop - scroller.clientHeight < 40;
    // The popover is positioned against the wrap and does not scroll with the content, so any scroll dismisses it (as selection popovers commonly do)
    selPop = null;
  }

  function scrollToBottom() {
    atBottom = true;
    if (scroller) scroller.scrollTop = scroller.scrollHeight;
  }

  $effect(() => {
    // While stuck to the bottom it follows the streaming output. Appending token by token changes
    // only the last item's text and not the length, so both have to be tracked.
    // The visibility switch is in the same list (the same reason as autogrow's): under
    // display:none scrolling is a no-op, and on becoming visible again the stuck-to-bottom state
    // has to be applied once more
    const last = agent.timeline[agent.timeline.length - 1];
    if (last && "text" in last) void last.text;
    void agent.timeline.length;
    void agent.thinking;
    void vault.rightTab;
    void vault.rightOpen;
    if (scroller && atBottom) scroller.scrollTop = scroller.scrollHeight;
  });

  $effect(() => {
    // The input starts at one line and grows with its content, capping at about six lines and
    // scrolling internally after that.
    // The panel's tabs are switched with display:none: while hidden, a mount or a remeasure sees a
    // scrollHeight of 0, which once nailed the input to an invisible 0px seam (unclickable and
    // unfocusable = "I cannot type"), and becoming visible again did not rerun the effect — so ①
    // the visibility switch is a dependency and it remeasures on appearing;
    // ② a measurement of 0 (still hidden) does not set a height, keeping the CSS default of one line.
    void agent.draft;
    void vault.rightTab;
    void vault.rightOpen;
    const el = composerEl;
    if (!el) return;
    el.style.height = "auto";
    if (el.scrollHeight === 0) return;
    el.style.height = `${Math.min(el.scrollHeight, 120)}px`;
  });

</script>

<div class="chat">
  <div class="timeline-wrap" bind:this={wrapEl}>
  {#if selPop}
    <!-- svelte-ignore a11y_no_static_element_interactions -->
    <div
      class="sel-pop"
      style={`left:${selPop.x}px; top:${selPop.y}px`}
      onmousedown={(e) => e.preventDefault()}
    >
      <button title={t("chat.copySelection")} onclick={() => popAction(copyAnswer)}>{t("chat.copy")}</button>
      <button title={t("chat.insertAsQuote")} onclick={() => popAction(insertAnswer)}>{t("chat.insert")}</button>
      <button
        title={clipTarget.path ? t("chat.sendToTarget", { name: clipTarget.name ?? "" }) : t("chat.pinFirst")}
        onclick={() => popAction(sendToPinned)}
      >→{clipTarget.name ?? t("chat.pinWord")}</button>
    </div>
  {/if}
  {#if plan}
    <div class="plan-card">
      <button class="plan-head" onclick={() => (planOpen = !planOpen)}>
        <span class="plan-caret" class:open={planOpen} aria-hidden="true">
          <svg width="12" height="12" viewBox="0 0 16 16" fill="none"><path d="M6 4l4 4-4 4" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"/></svg>
        </span>
        {t("chat.planProgress", { done: plan.filter((p) => p.done).length, total: plan.length })}
      </button>
      {#if planOpen}
        {#each plan as p, pi (pi)}
          <div class="plan-item" class:done={p.done}>
            <span class="plan-check">{p.done ? "☑" : "☐"}</span>{p.text}
          </div>
        {/each}
      {/if}
    </div>
  {/if}
  <HoverCard card={hover} />
  <!-- svelte-ignore a11y_click_events_have_key_events, a11y_mouse_events_have_key_events, a11y_no_static_element_interactions -->
  <div
    class="timeline"
    bind:this={scroller}
    onscroll={onScroll}
    onclick={onTimelineClick}
    onmouseup={onTimelineMouseUp}
    onmouseover={onTimelineOver}
    onmouseout={onTimelineOut}
  >
    {#if agent.timeline.length === 0}
      <div class="welcome">
        <p class="welcome-title">{t("chat.welcomeTitle")}</p>
        <p class="welcome-hint">
          {t("chat.welcomeBody")}
        </p>
        <div class="welcome-examples">
          {#each welcomeExamples as ex (ex)}
            <button class="example" onclick={() => fillExample(ex)}>{ex}</button>
          {/each}
        </div>
        <p class="welcome-hint faint">
          {t("chat.welcomeHint", { key: k("Mod+L") })}
        </p>
      </div>
    {/if}
    {#each rows as row (row.kind === "run" ? `run-${row.start}` : `i-${row.i}`)}
      {#if row.kind === "run"}
        {@const last = row.items[row.items.length - 1]}
        {@const pending = last.item.ok === undefined}
        {@const errs = row.items.reduce((n, it) => n + (it.item.ok === false ? 1 : 0), 0)}
        {@const open = !!expandedToolRuns[row.start]}
        <button class="tool-row" onclick={() => (expandedToolRuns[row.start] = !open)}>
          <span class="tool-status" class:err={errs > 0} class:pending>
            {pending ? "◌" : open ? "▾" : "▸"}
          </span>
          <span class="tool-name">{row.mixed ? t("tool.mixed") : toolLabel(row.name)}</span>
          <span class="tool-args">
            {row.mixed ? runLabel(row) : `×${row.items.length}`}{errs ? t("chat.toolFailures", { n: errs }) : ""}{runDigest(row)}
          </span>
        </button>
        {#if open}
          {#each row.items as it (it.i)}
            <button class="tool-row tool-sub" onclick={() => (expandedTools[it.i] = !expandedTools[it.i])}>
              <span class="tool-status" class:err={it.item.ok === false} class:pending={it.item.ok === undefined}>
                {it.item.ok === undefined ? "◌" : it.item.ok ? "▸" : "✕"}
              </span>
              {#if row.mixed}<span class="tool-name">{toolLabel(it.item.name)}</span>{/if}
              <span class="tool-args">{it.item.args}</span>
            </button>
            {#if expandedTools[it.i] && it.item.output}
              <pre class="tool-output">{it.item.output}</pre>
            {/if}
          {/each}
        {:else if pending}
          <button class="tool-row tool-sub" onclick={() => (expandedToolRuns[row.start] = true)}>
            <span class="tool-status pending">◌</span>
            {#if row.mixed}<span class="tool-name">{toolLabel(last.item.name)}</span>{/if}
            <span class="tool-args">{last.item.args}</span>
          </button>
        {/if}
      {:else}
      {@const item = row.item}
      {@const i = row.i}
      {#if item.kind === "user"}
        <div class="msg user">
          {item.text}
          {#if item.page}
            <span class="user-page" title={item.page.url}><RefIcon kind="page" size={12} /> {item.page.title}</span>
          {/if}
          {#if item.refs?.length}
            {#each item.refs as r (`${r.kind}:${r.path}:${r.lines?.join("-") ?? ""}`)}
              <!-- A reference marked private afterwards: its path is no longer shown (a placeholder
                   rather than hiding it — it really was sent at the time, and disappearing entirely
                   would distort the record the other way; a privacy change takes effect reactively
                   through privatePaths) -->
              {#if vault.privatePaths.has(r.path)}
                <span class="user-page" title={t("chat.privateRefHint")}><PrivacyIcon size={12} /> {t("chat.privateRef")}</span>
              {:else}
                <span class="user-page" title={r.path}
                  ><RefIcon kind={r.kind} size={12} /> {r.path}{r.lines ? ` ${linesLabel({ from_line: r.lines[0], to_line: r.lines[1] })}` : ""}</span
                >
              {/if}
            {/each}
          {/if}
          {#if !agent.running}
            <span class="msg-actions user-actions">
              {#if i === lastUserIndex}
                <button title={t("chat.editResendHint")} onclick={() => void editLast()}>{t("chat.editResend")}</button>
              {/if}
              {#if userOrdinal(i) >= 1}
                <button title={t("chat.forkHint")} onclick={() => void forkAt(i)}>{t("chat.fork")}</button>
              {/if}
            </span>
          {/if}
        </div>
      {:else if item.kind === "assistant"}
        {@const body = stripPlan(item.text)}
        {#if body || isTurnFinal(i)}
          <div class="msg assistant">
            {#each assistantBlocks(i, body) as b, bi (bi)}{@html b}{/each}
            <!-- The action bar rests at the end of each turn's final answer (where the hand already
                 is after reading, and it covers no text): the newest turn shows it permanently and
                 an older turn on hover; an intermediate narration bubble does not get one — select
                 its text and press a button here instead (the selection semantics are unchanged).
                 It is not shown while streaming. -->
            {#if isTurnFinal(i) && !(agent.running && i === lastAssistantIndex)}
              <div class="msg-foot" class:current={i === lastAssistantIndex}>
                <button title={t("chat.copyAnswerHint")} onclick={() => void copyAnswer(body)}>{t("chat.copy")}</button>
                <button title={t("chat.insertAnswerHint")} onclick={() => void insertAnswer(body)}>{t("chat.insert")}</button>
                <button
                  title={clipTarget.path ? t("chat.sendToTargetPlain", { name: clipTarget.name ?? "" }) : t("chat.pinFirst")}
                  onclick={() => void sendToPinned(body)}
                >→{clipTarget.name ?? t("chat.pinWord")}</button>
                <button title={t("chat.saveAsNoteHint")} onclick={() => void saveAsNote(body, i)}>{t("chat.saveAsNote")}</button>
                {#if i === lastAssistantIndex && !agent.running}
                  <button title={t("chat.regenerateHint")} onclick={doRetry}>{t("chat.regenerate")}</button>
                {/if}
              </div>
            {/if}
          </div>
        {/if}
      {:else if item.kind === "plan"}
        <!-- It looks at pendingPlan alone and adds no running gate: in a window handed over with
             ⌘J the loop runs on the other window's channel and the local running is always false,
             while the gate really is pending.
             Every route by which the loop ends (send's finally, tool_call_finished, reset,
             reattach) clears pendingPlan, so no zombie buttons are left behind. -->
        {@const pendingPlan = agent.pendingPlan?.id === item.id}
        {@const shownItems = item.decision?.kind === "approve_edited" ? item.decision.items : item.items}
        <div class="plan-inline" class:rejected={item.decision?.kind === "reject"}>
          <div class="plan-inline-head">
            <span class="plan-inline-title">{t("chat.planTitle")}</span>
            {#if item.decision}
              <span class="plan-inline-badge" class:bad={item.decision.kind === "reject"}>{planBadge(item.decision)}</span>
            {:else if pendingPlan}
              <span class="plan-inline-badge wait">{t("chat.planWaiting")}</span>
            {/if}
          </div>
          {#if pendingPlan && planEditing}
            <!-- svelte-ignore a11y_autofocus -->
            <textarea class="plan-edit" bind:value={planEditText} spellcheck="false" autofocus></textarea>
          {:else}
            {#each shownItems as p, pi (pi)}
              <div class="plan-item"><span class="plan-check">{pi + 1}.</span>{p}</div>
            {/each}
          {/if}
          {#if pendingPlan}
            {#if planRejecting}
              <div class="plan-actions">
                <!-- svelte-ignore a11y_autofocus -->
                <input class="plan-reason" bind:value={planRejectReason} placeholder={t("chat.planRejectPlaceholder")} autofocus />
                <button class="btn danger" onclick={submitPlanReject}>{t("chat.planConfirmReject")}</button>
              </div>
            {:else}
              <div class="plan-actions">
                <button class="btn subtle" onclick={() => (planRejecting = true)}>{t("chat.planReject")}</button>
                <button class="btn subtle" onclick={() => togglePlanEdit(item)}>{planEditing ? t("chat.planRestore") : t("chat.planEdit")}</button>
                <span class="plan-spacer"></span>
                <button class="btn primary" onclick={() => submitPlanApprove(item)}>{t("chat.planApprove")}</button>
              </div>
            {/if}
          {/if}
        </div>
      {:else if item.kind === "notice"}
        <div class="notice">{item.text}</div>
      {:else if item.kind === "tool"}
        <button class="tool-row" onclick={() => (expandedTools[i] = !expandedTools[i])}>
          <span class="tool-status" class:err={item.ok === false} class:pending={item.ok === undefined}>
            {item.ok === undefined ? "◌" : item.ok ? "▸" : "✕"}
          </span>
          <span class="tool-name">{toolLabel(item.name)}</span>
          <span class="tool-args">{item.args}</span>
        </button>
        {#if expandedTools[i] && item.output}
          <pre class="tool-output">{item.output}</pre>
        {/if}
      {:else if item.kind === "sources"}
        {@const clippedCount = item.results.filter((r) => clips.pathFor(r.url)).length}
        {@const headText = item.results.length
          ? `${t("chat.sourcesCount", { n: item.results.length })}${clippedCount ? t("chat.sourcesClipped", { n: clippedCount }) : ""}`
          : t("chat.searched")}
        {@const collapsible = !item.searching && item.results.length > 4}
        {@const collapsed = collapsible && !expandedSources[i]}
        <div class="sources">
          {#if item.searching}
            <div class="sources-head"><span class="pulse"></span> {t("chat.searching")}</div>
          {:else}
            {#if collapsible}
              <button
                class="sources-head sources-toggle"
                title={t(collapsed ? "chat.expandSources" : "chat.collapseSources")}
                onclick={() => (expandedSources[i] = !expandedSources[i])}
              >
                <span class="sources-caret" class:open={!collapsed} aria-hidden="true">
                  <svg width="12" height="12" viewBox="0 0 16 16" fill="none"><path d="M6 4l4 4-4 4" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"/></svg>
                </span>
                {headText}
              </button>
            {:else}
              <div class="sources-head">{headText}</div>
            {/if}
            {#if !collapsed}
            {#if item.queries.length}
              <div class="sources-queries" title={item.queries.join("\n")}>
                {t("chat.queries", { list: item.queries.join(" · ") })}
              </div>
            {/if}
            {#each item.results as r, n (r.url)}
              <div class="source-row">
                <button class="source-main" title={r.snippet || r.url} onclick={(e) => openCitation(r.url, e.metaKey || e.ctrlKey)}>
                  <span class="source-n">{n + 1}</span>
                  <span class="source-title">{r.title || r.url}</span>
                  {#if r.unverified && !clips.pathFor(r.url)}
                    <span class="src-unverified" title={t("chat.unverifiedHint")}>{t("chat.unverified")}</span>
                  {/if}
                  <span class="source-domain">{domainOf(r.url)}</span>
                </button>
                {#if clips.pathFor(r.url)}
                  <button class="source-clip clipped" title={t("reader.alreadyClippedHint")} onclick={() => openClipped(r.url)}>{t("reader.alreadyClipped")}</button>
                {:else}
                  <button class="source-clip" title={t("chat.clipSourceHint")} onclick={() => void clipSource(r)}>{t("reader.clip")}</button>
                {/if}
              </div>
            {/each}
            {/if}
          {/if}
        </div>
      {:else if item.kind === "error"}
        <div class="msg error">
          {item.text}
          {#if i === agent.timeline.length - 1 && !agent.running && lastUserIndex >= 0}
            <button class="err-retry" onclick={doRetry}>{t("chat.retry")}</button>
          {/if}
        </div>
      {/if}
      {/if}
    {/each}
    {#if agent.lastTruncated && !agent.running}
      <div class="truncated-bar">
        {t("chat.truncated")}
        <button onclick={continueTruncated}>{t("chat.continueGenerating")}</button>
      </div>
    {/if}
    {#if agent.thinking}
      {#if thinkingOpen}
        <div class="thinking-full">
          <button class="thinking-toggle" onclick={() => (thinkingOpen = false)}>{t("chat.collapseThinking")}</button>
          <pre>{agent.thinking}</pre>
        </div>
      {:else}
        <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
        <div
          class="thinking"
          class:long={agent.thinking.length > 200}
          title={t("chat.expandThinking")}
          onclick={() => (thinkingOpen = true)}
        ><span>{agent.thinking}</span></div>
      {/if}
    {/if}
    {#if agent.running}
      <div class="running-bar">
        <span class="pulse"></span> {t("chat.thinking")}
        {#if agent.usage}
          <span class="usage-inline" title={t("chat.usageHint", { prompt: agent.usage.prompt.toLocaleString(), cache: agent.usage.cacheHit.toLocaleString() })}>
            {usageText}
          </span>
        {/if}
        <button class="cancel" onclick={() => agent.cancel()}>{t("chat.stop")}</button>
      </div>
    {/if}
  </div>
  {#if !atBottom}
    <button class="to-bottom" onclick={scrollToBottom}>
      {#if agent.running}<span class="pulse"></span>{/if}
      {t("chat.backToBottom")}
    </button>
  {/if}
  </div>

  <div class="composer" class:summon={summoning}>
    {#if agent.usage && !agent.running}
      <div class="usage-line" title={t("chat.usageLastTurn", { prompt: agent.usage.prompt.toLocaleString(), cache: agent.usage.cacheHit.toLocaleString(), completion: agent.usage.completion.toLocaleString() })}>
        {usageText}
      </div>
    {/if}
    <div class="box">
    <!-- The context chips (the current note / the selection / @ references) sit at the top of the
         input; with no chip at all the row takes no space. What can be seen can be dismissed with
         ✕, and what goes out is exactly this row -->
    {#if chips.length}
    <div class="ctx-row">
      {#each chips as c (c.key)}
        <span class="ctx-chip" class:cur={c.cur} title={c.title}>
          <span class="ctx-chip-icon"><RefIcon kind={c.kind} size={12} /></span>
          <span class="ctx-chip-label">{c.label}</span>
          {#if c.lines}<span class="ctx-chip-lines">{c.lines}</span>{/if}
          {#if c.tag}<span class="ctx-chip-tag">{c.tag}</span>{/if}
          <button aria-label={c.removeLabel} title={c.removeTitle} onclick={c.remove}>✕</button>
        </span>
      {/each}
    </div>
    {/if}
    {#if agent.queuedText}
      <div class="queue-chip">
        <span class="queue-label" title={agent.queuedText}>{t("chat.queued", { text: agent.queuedText })}</span>
        <button aria-label={t("chat.cancelQueued")} onclick={() => (agent.queuedText = null)}>✕</button>
      </div>
    {/if}
    {#if atQuery != null}
      <AtPicker bind:this={picker} query={atQuery} anchor={composerEl} {curSel} onPick={onAtPick} onPickFiles={onAtPickFiles} onClose={closeAt} />
    {/if}
    <div class="input-wrap">
      {#if agent.draft === ""}
        <div class="ph" aria-hidden="true">
          {#if agent.running}
            <span>{t("chat.runningQueueHint")}</span>
          {:else}
            {#key phIdx}<span class="ph-item">{PH_HINTS[phIdx]}</span>{/key}
          {/if}
        </div>
      {/if}
      <textarea
        bind:this={composerEl}
        bind:value={agent.draft}
        {onkeydown}
        onpaste={onPaste}
        oninput={detectAt}
        onclick={detectAt}
        onkeyup={detectAt}
        onblur={() => { closeAt(); hideSummonHint(); }}
        aria-label={t("chat.composerAria")}
        rows="1"
      ></textarea>
    </div>
    <div class="composer-bar">
      <button
        class="tool-btn add"
        title={t("chat.addContextHint")}
        aria-label={t("atPicker.heading")}
        onmousedown={(e) => e.preventDefault()}
        onclick={() => void openAt()}
      >
        <svg width="16" height="16" viewBox="0 0 20 20" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" aria-hidden="true">
          <path d="M10 4.5v11M4.5 10h11" />
        </svg>
      </button>
      {#if prefs.reasoningEffortActive}
        <button class="tool-btn effort" title={t("chat.effortHint")} onclick={effortMenu}>
          <svg width="14" height="14" viewBox="0 0 20 20" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" aria-hidden="true">
            <path d="M3.5 6.5h7M14.5 6.5h2M3.5 13.5h2M9.5 13.5h7" />
            <circle cx="12.5" cy="6.5" r="1.8" />
            <circle cx="7.5" cy="13.5" r="1.8" />
          </svg>
          <span>{effortLabel}</span>
        </button>
      {/if}
      {#if summonHint}
        <span class="ctx url-hint">{t("chat.popOutHint", { key: k("Mod+J") })}</span>
      {:else if modeInfo.hint}
        <span class="ctx url-hint">{modeInfo.hint}</span>
      {:else}
        <!-- A spacer, so the send button on the right hugs the edge -->
        <span class="ctx"></span>
      {/if}
      <button
        class="send"
        title={modeInfo.title}
        aria-label={modeInfo.aria}
        onclick={() => submit(false)}
        disabled={!agent.draft.trim()}
      >
        <svg width="16" height="16" viewBox="0 0 20 20" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
          <path d="M10 15.5v-11M5 9.5l5-5 5 5" />
        </svg>
      </button>
    </div>
    </div>
  </div>
</div>

<style>
  .chat {
    display: flex;
    flex-direction: column;
    flex: 1;
    min-height: 0;
  }
  /* ---- the timeline ---- */
  .timeline-wrap {
    position: relative;
    flex: 1;
    min-height: 0;
    display: flex;
    flex-direction: column;
  }
  .timeline {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    padding: 12px;
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  /* The timeline in the overlay needs some body: an empty flex container collapses into a strip
     when there is little content, and the second glance after summoning it (the streaming answer
     unfolding) needs a stable perceived height; the cap is left to the overlay container's 72vh */
  .to-bottom {
    position: absolute;
    bottom: 10px;
    left: 50%;
    transform: translateX(-50%);
    display: flex;
    align-items: center;
    gap: 6px;
    font-size: var(--fs-xs);
    color: var(--fg-muted);
    background: var(--bg);
    border: 1px solid var(--border);
    border-radius: 999px;
    padding: 4px 12px;
    box-shadow: 0 2px 8px rgba(0, 0, 0, 0.12);
  }
  .to-bottom:hover {
    color: var(--fg);
    background: var(--bg-hover);
  }
  .welcome {
    padding: 24px 8px;
  }
  .welcome-title {
    font-weight: 600;
    margin-bottom: 8px;
  }
  .welcome-hint {
    font-size: var(--fs-sm);
    color: var(--fg-muted);
    line-height: 1.6;
    margin-bottom: 6px;
  }
  .welcome-examples {
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    gap: 6px;
    margin: 10px 0 14px;
  }
  .example {
    font-size: var(--fs-xs);
    color: var(--fg-muted);
    border: 1px solid var(--border);
    border-radius: 999px;
    padding: 4px 12px;
    text-align: left;
  }
  .example:hover {
    color: var(--accent);
    border-color: var(--accent);
    background: var(--bg-hover);
  }
  .faint {
    color: var(--fg-faint);
  }
  .msg {
    font-size: 15px;
    line-height: 1.65;
    user-select: text;
    word-break: break-word;
  }
  .msg.user {
    background: var(--accent-soft);
    border-radius: 8px;
    padding: 8px 10px;
    align-self: flex-end;
    max-width: 92%;
    white-space: pre-wrap;
  }
  /* Turn separation: room to breathe before a new turn's user message */
  .timeline > .msg.user:not(:first-child) {
    margin-top: 16px;
  }
  .user-page {
    display: block;
    margin-top: 4px;
    font-size: var(--fs-xs);
    color: var(--fg-faint);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .user-page :global(svg) {
    vertical-align: -2px;
  }
  .msg.assistant {
    padding: 0 2px;
  }
  /* The tags the renderer generates are injected with {@html}, so the selectors need :global */
  .msg.assistant :global(p) {
    margin: 0 0 8px;
  }
  .msg.assistant :global(p:last-child) {
    margin-bottom: 0;
  }
  .msg.assistant :global(ul),
  .msg.assistant :global(ol) {
    margin: 0 0 8px;
    padding-left: 20px;
  }
  .msg.assistant :global(li) {
    margin: 3px 0;
  }
  .msg.assistant :global(li::marker) {
    color: var(--fg-faint);
  }
  /* A weight of 700 in the CJK face looks muddy on a dark ground, and 600 already stands out from the body */
  .msg.assistant :global(strong) {
    font-weight: 600;
  }
  .msg.assistant :global(blockquote) {
    margin: 0 0 8px;
    padding: 2px 10px;
    border-left: 2px solid var(--border);
    color: var(--fg-muted);
  }
  .msg.assistant :global(code) {
    font-family: var(--font-mono);
    font-size: 0.92em;
    background: var(--bg-panel);
    border-radius: 3px;
    padding: 0 4px;
  }
  .msg.assistant :global(.md-code) {
    font-family: var(--font-mono);
    font-size: var(--fs-xs);
    background: var(--bg-panel);
    border-radius: 6px;
    padding: 8px;
    overflow-x: auto;
    margin: 0 0 8px;
  }
  .msg.assistant :global(.md-code code) {
    background: none;
    padding: 0;
  }
  /* The hierarchy comes from whitespace and structural lines rather than swelling font sizes — the panel is narrow, and a large heading would eat the line width */
  .msg.assistant :global(.md-h) {
    font-weight: 600;
    margin: 16px 0 6px;
  }
  .msg.assistant :global(.md-h:first-child) {
    margin-top: 2px;
  }
  .msg.assistant :global(.md-h1) {
    font-size: 1.1em;
  }
  .msg.assistant :global(.md-h2) {
    font-size: 1.05em;
  }
  .msg.assistant :global(.md-h1),
  .msg.assistant :global(.md-h2) {
    padding-bottom: 5px;
    border-bottom: 1px solid var(--border);
  }
  .msg.assistant :global(.md-hr) {
    border: none;
    border-top: 1px solid var(--border);
    margin: 8px 0;
  }
  .msg.assistant :global(.md-cite) {
    display: inline-block;
    font-size: 10px;
    font-weight: 600;
    font-family: var(--font-mono);
    color: var(--accent);
    background: var(--bg-hover);
    border-radius: 4px;
    padding: 0 4px;
    margin: 0 1px;
    vertical-align: super;
    line-height: 1.5;
    cursor: pointer;
  }
  .msg.assistant :global(.md-cite:hover) {
    background: var(--accent);
    color: var(--bg);
  }
  .msg.assistant :global(.md-link),
  .msg.assistant :global(.md-wiki) {
    color: var(--accent);
    cursor: pointer;
  }
  .msg.assistant :global(.md-link:hover),
  .msg.assistant :global(.md-wiki:hover) {
    text-decoration: underline;
  }
  /* A block citation: a chip with a ¶ / › glyph (the styles are global in app.css so the replay view shares them) */
  .msg.assistant :global(.md-blockref:hover) {
    text-decoration: none;
  }
  /* The table skeleton lives in app.css's global .md-tablewrap section (shared with ReadingView);
     only the chat bubble's own paragraph spacing stays here */
  .msg.assistant :global(.md-tablewrap) {
    margin: 2px 0 10px;
  }
  /* The image placeholder chip: zero remote requests, and a click opens a browser window to see the original (the same fallback as the reading projection's) */
  .msg.assistant :global(.md-img) {
    display: inline-block;
    font-size: 0.85em;
    color: var(--fg-muted);
    background: var(--bg-panel);
    border: 1px dashed var(--border);
    border-radius: 5px;
    padding: 1px 8px;
    cursor: pointer;
    max-width: 100%;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    vertical-align: bottom;
  }
  .msg.assistant :global(.md-img:hover) {
    color: var(--fg);
    background: var(--bg-hover);
  }
  .msg.error {
    color: var(--danger);
    font-size: var(--fs-xs);
    border: 1px solid var(--border);
    border-left: 2px solid var(--danger);
    border-radius: 4px;
    padding: 6px 8px;
    white-space: pre-wrap;
  }
  /* the audit ledger — vemry's signature element */
  .tool-row {
    display: flex;
    align-items: baseline;
    gap: 6px;
    width: 100%;
    text-align: left;
    font-family: var(--font-mono);
    font-size: var(--fs-xs);
    color: var(--fg-muted);
    padding: 2px 4px;
    border-left: 2px solid var(--border);
    border-radius: 0;
  }
  .tool-row:hover {
    background: var(--bg-hover);
  }
  /* A run's detail rows: they keep the ledger's left spine and are indented to show they belong to it */
  .tool-row.tool-sub {
    padding-left: 20px;
  }
  .tool-status {
    color: var(--accent);
    flex-shrink: 0;
  }
  .tool-status.err {
    color: var(--danger);
  }
  .tool-status.pending {
    color: var(--fg-faint);
  }
  .tool-name {
    color: var(--fg);
    flex-shrink: 0;
  }
  .tool-args {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .tool-output {
    font-family: var(--font-mono);
    font-size: var(--fs-xs);
    color: var(--fg-muted);
    background: var(--bg-panel);
    border-radius: 4px;
    padding: 8px;
    overflow-x: auto;
    max-height: 200px;
    user-select: text;
    margin-left: 12px;
  }
  /* Bottom-aligned: what is clipped on overflow is the oldest beginning, so the newest thinking is always visible */
  .thinking {
    display: flex;
    flex-direction: column;
    justify-content: flex-end;
    font-size: var(--fs-xs);
    color: var(--fg-faint);
    font-style: italic;
    max-height: 60px;
    overflow: hidden;
  }
  .thinking span {
    white-space: pre-wrap;
  }
  .thinking.long {
    -webkit-mask-image: linear-gradient(to bottom, transparent 0, #000 16px);
    mask-image: linear-gradient(to bottom, transparent 0, #000 16px);
  }
  .running-bar {
    display: flex;
    align-items: center;
    gap: 6px;
    font-size: var(--fs-xs);
    color: var(--fg-muted);
  }
  .pulse {
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background: var(--accent);
    animation: pulse 1.2s ease-in-out infinite;
  }
  @keyframes pulse {
    50% {
      opacity: 0.25;
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .pulse {
      animation: none;
    }
  }
  .cancel {
    margin-left: auto;
    color: var(--danger);
    font-size: var(--fs-xs);
  }
  /* ---- the source cards ---- */
  .sources {
    display: flex;
    flex-direction: column;
    gap: 2px;
    border: 1px solid var(--border);
    border-radius: 8px;
    padding: 8px;
    background: var(--bg);
  }
  .sources-head {
    display: flex;
    align-items: center;
    gap: 6px;
    font-size: var(--fs-xs);
    color: var(--fg-faint);
    text-transform: uppercase;
    letter-spacing: 0.05em;
    margin-bottom: 2px;
  }
  .sources-toggle {
    width: 100%;
    text-align: left;
    padding: 2px 4px;
    margin-bottom: 0;
    border-radius: 5px;
  }
  .sources-toggle:hover {
    color: var(--fg-muted);
    background: var(--bg-hover);
  }
  .sources-caret {
    flex-shrink: 0;
    display: flex;
    align-items: center;
    transition: transform 0.15s ease;
  }
  .sources-caret.open {
    transform: rotate(90deg);
  }
  .source-row {
    display: flex;
    align-items: baseline;
    gap: 4px;
    min-width: 0;
  }
  .source-main {
    flex: 1;
    display: flex;
    align-items: baseline;
    gap: 6px;
    text-align: left;
    padding: 3px 6px;
    border-radius: 5px;
    min-width: 0;
  }
  .source-main:hover {
    background: var(--bg-hover);
  }
  .source-n {
    flex-shrink: 0;
    min-width: 2ch;
    text-align: right;
    font-size: var(--fs-xs);
    color: var(--fg-faint);
    font-family: var(--font-mono);
  }
  .source-title {
    flex: 1;
    min-width: 0;
    font-size: var(--fs-xs);
    color: var(--fg);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .source-domain {
    flex-shrink: 0;
    font-size: var(--fs-xs);
    color: var(--fg-faint);
  }
  .source-clip {
    flex-shrink: 0;
    font-size: 10px;
    color: var(--fg-faint);
    padding: 1px 6px;
    border-radius: 4px;
    border: 1px solid transparent;
  }
  /* "Clip" is an action and appears on row hover; "clipped" is a state and is always shown */
  .source-clip:not(.clipped) {
    visibility: hidden;
  }
  .source-row:hover .source-clip:not(.clipped),
  .source-row:focus-within .source-clip:not(.clipped) {
    visibility: visible;
  }
  .source-clip:hover {
    color: var(--fg);
    border-color: var(--border);
  }
  .source-clip.clipped {
    color: var(--accent);
  }
  .sources-queries {
    font-size: var(--fs-xs);
    color: var(--fg-faint);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    padding: 0 6px 2px;
  }
  /* ---- the input area ---- */
  .composer {
    position: relative; /* the @ candidate overlay anchors to its viewport rect (the padding of 12 lines up with the candidate layer) */
    padding: 4px 12px 10px;
  }
  /* The input box: one rounded bordered container — the chips on top, the body in the middle and
     the toolbar at the bottom; with the cursor inside it is outlined in the accent colour (the
     textarea's own focus-visible outline is turned off) */
  .box {
    border: 1px solid var(--border);
    border-radius: 10px;
    background: var(--bg);
    transition: border-color 0.12s ease, box-shadow 0.12s ease;
  }
  .composer:focus-within .box {
    border-color: var(--accent);
    box-shadow: 0 0 0 2px var(--accent-soft);
  }
  /* The one-shot highlight of ⌘J landing in the panel: an accent inner outline plus a faint tint
     that flashes and fades. Only box-shadow and background may move — a transform or filter must
     not, or the @ candidate layer's fixed coordinates, computed from a viewport rect, would be
     thrown off (see AtPicker's anchoring precondition) */
  .composer.summon .box {
    animation: summon 0.5s ease-out;
  }
  @keyframes summon {
    0% {
      box-shadow: inset 0 0 0 2px var(--accent);
      background: var(--accent-soft);
    }
    100% {
      box-shadow: inset 0 0 0 2px transparent;
      background: var(--bg);
    }
  }
  @media (prefers-reduced-motion: reduce) {
    /* No animation: for the 0.5s the class is applied it lays a static faint accent tint, which still gives the "it is here" cue */
    .composer.summon .box {
      animation: none;
      background: var(--accent-soft);
    }
  }
  /* The context row: the @ entry point and the chips share a height and a column, with wrapping added */
  .ctx-row {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 4px;
    padding: 8px 10px 0;
    min-width: 0;
  }
  .ctx-chip {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    height: 20px;
    max-width: 240px;
    padding: 0 4px 0 6px;
    border: 1px solid var(--border);
    border-radius: 5px;
    background: var(--bg-panel);
    font-size: var(--fs-xs);
    color: var(--fg-muted);
    min-width: 0;
  }
  .ctx-chip.cur {
    color: var(--fg);
  }
  .ctx-chip-icon {
    display: flex;
    flex-shrink: 0;
    color: var(--fg-faint);
  }
  .ctx-chip-label {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .ctx-chip-tag {
    flex-shrink: 0;
    font-size: 10px;
    color: var(--fg-faint);
    padding-left: 2px;
  }
  /* A selection's line range: it does not shrink, so however long the file name is, the name is truncated first and the (12-20) survives */
  .ctx-chip-lines {
    flex-shrink: 0;
    font-family: var(--font-mono);
    font-size: 10px;
    color: var(--fg-muted);
  }
  .ctx-chip button {
    flex-shrink: 0;
    width: 14px;
    height: 14px;
    line-height: 14px;
    font-size: 10px;
    border-radius: 3px;
    color: var(--fg-faint);
    padding: 0;
  }
  .ctx-chip button:hover {
    color: var(--fg);
    background: var(--bg-hover);
  }
  textarea {
    display: block;
    width: 100%;
    resize: none;
    border: none;
    background: transparent;
    padding: 10px 12px 2px;
    font-size: var(--fs-md);
    line-height: 1.5;
    max-height: 120px;
    overflow-y: auto;
  }
  textarea:focus-visible {
    outline: none;
  }
  /* The overlay placeholder: strictly aligned with the textarea's first-line padding and line
     height, single-line and truncated when too wide; it takes no mouse events (clicking it is
     clicking the input). The animation lives on the span inside .ph — which is not an ancestor of
     the textarea and does not disturb AtPicker's viewport-rect anchoring */
  .input-wrap {
    position: relative;
  }
  .ph {
    position: absolute;
    top: 10px;
    left: 12px;
    right: 12px;
    font-size: var(--fs-md);
    line-height: 1.5;
    color: var(--fg-faint);
    pointer-events: none;
    overflow: hidden;
    white-space: nowrap;
  }
  .ph span {
    display: block;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .ph-item {
    animation: ph-in 0.3s ease;
  }
  @keyframes ph-in {
    from {
      opacity: 0;
      transform: translateY(8px);
    }
    to {
      opacity: 1;
      transform: none;
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .ph-item {
      animation: none;
    }
  }
  /* The bottom toolbar: "+", the level capsule and the hint text from the left; the round send button on the right */
  .composer-bar {
    display: flex;
    align-items: center;
    gap: 2px;
    padding: 2px 6px 6px 6px;
  }
  .tool-btn {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    height: 26px;
    padding: 0 6px;
    border-radius: 6px;
    color: var(--fg-muted);
    font-size: var(--fs-xs);
    white-space: nowrap;
    flex-shrink: 0;
  }
  .tool-btn:hover:not(:disabled) {
    color: var(--fg);
    background: var(--bg-hover);
  }
  .tool-btn.add {
    width: 26px;
    padding: 0;
    justify-content: center;
  }
  .ctx {
    flex: 1; /* it fills what is left, so the send button on the right hugs the edge */
    margin-left: 6px;
    min-width: 0;
    font-size: 10px;
    font-family: var(--font-mono);
    color: var(--fg-faint);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .url-hint {
    color: var(--accent);
    font-family: inherit;
  }
  .effort:disabled {
    opacity: 0.5;
    cursor: default;
  }
  .send {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 28px;
    height: 28px;
    border-radius: 50%;
    background: var(--accent);
    color: #fff;
    flex-shrink: 0;
    transition: background 0.12s ease, color 0.12s ease;
  }
  .send:hover:not(:disabled) {
    filter: brightness(1.08);
  }
  .send:disabled {
    background: transparent;
    color: var(--fg-faint);
    cursor: default;
  }
  /* ---- The plan card: the recited task list pinned at the top of the timeline, progress at a glance ---- */
  .plan-card {
    flex-shrink: 0;
    border-bottom: 1px solid var(--border);
    background: var(--bg-panel);
    padding: 4px 12px 6px;
    max-height: 30%;
    overflow-y: auto;
  }
  .plan-head {
    display: flex;
    align-items: center;
    gap: 5px;
    width: 100%;
    text-align: left;
    font-size: var(--fs-xs);
    font-weight: 600;
    color: var(--fg-muted);
    padding: 2px 0;
  }
  .plan-head:hover {
    color: var(--fg);
  }
  .plan-caret {
    display: flex;
    align-items: center;
    transition: transform 0.15s ease;
  }
  .plan-caret.open {
    transform: rotate(90deg);
  }
  .plan-item {
    display: flex;
    align-items: baseline;
    gap: 6px;
    font-size: var(--fs-xs);
    color: var(--fg);
    padding: 1px 0 1px 4px;
  }
  .plan-item.done {
    color: var(--fg-faint);
    text-decoration: line-through;
    text-decoration-color: var(--fg-faint);
  }
  .plan-check {
    flex-shrink: 0;
    color: var(--fg-faint);
    text-decoration: none;
  }
  .plan-item.done .plan-check {
    color: var(--accent);
  }
  /* ---- The plan gate's inline card (propose_plan): the decision point in the timeline. The
     sticky card is the progress projection, and this card carries "confirm / edit / reject" —
     after a decision the buttons disappear and a badge remains ---- */
  .plan-inline {
    border: 1px solid var(--border);
    border-radius: 8px;
    background: var(--bg-panel);
    padding: 8px 12px;
    margin: 4px 0;
  }
  .plan-inline.rejected {
    opacity: 0.65;
  }
  .plan-inline-head {
    display: flex;
    align-items: center;
    gap: 8px;
    font-size: var(--fs-xs);
    font-weight: 600;
    color: var(--fg-muted);
    padding-bottom: 2px;
  }
  .plan-inline-badge {
    font-weight: 400;
    color: var(--accent);
  }
  .plan-inline-badge.bad {
    color: var(--danger);
  }
  .plan-inline-badge.wait {
    color: var(--fg-faint);
  }
  .plan-edit {
    width: 100%;
    min-height: 96px;
    font-size: var(--fs-xs);
    font-family: inherit;
    line-height: 1.6;
    padding: 6px 8px;
    resize: vertical;
    margin: 4px 0;
  }
  .plan-actions {
    display: flex;
    align-items: center;
    gap: 8px;
    padding-top: 8px;
  }
  .plan-spacer {
    flex: 1;
  }
  .plan-reason {
    flex: 1;
    padding: 5px 8px;
    font-size: var(--fs-xs);
  }
  .plan-actions .btn {
    font-size: var(--fs-xs);
    padding: 4px 10px;
    border-radius: 6px;
    border: 1px solid var(--border);
  }
  .plan-actions .btn:hover {
    background: var(--bg-hover);
  }
  .plan-actions .btn.primary {
    background: var(--accent);
    border-color: var(--accent);
    color: #fff;
  }
  .plan-actions .btn.primary:hover {
    opacity: 0.9;
    background: var(--accent);
  }
  .plan-actions .btn.subtle {
    color: var(--fg-muted);
  }
  .plan-actions .btn.danger {
    color: var(--danger);
  }
  /* ---- The action bar at the end of an answer: the shortest path for getting an answer back
     into a note. It rests at the end of each turn's final answer — where the hand already is after
     reading; the newest turn shows it permanently (quietened), and an older turn on hover.
     Its space is constant, so hovering does not shift the layout ---- */
  .msg-foot {
    display: flex;
    flex-wrap: wrap;
    gap: 2px;
    margin-top: 8px;
    opacity: 0;
    transition: opacity 0.12s ease;
  }
  .msg-foot.current,
  .msg.assistant:hover .msg-foot,
  .msg-foot:focus-within {
    opacity: 1;
  }
  .msg-foot button {
    font-size: 10px;
    color: var(--fg-faint);
    padding: 1px 7px;
    border: 1px solid transparent;
    border-radius: 4px;
    white-space: nowrap;
    max-width: 150px;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .msg-foot.current button {
    color: var(--fg-muted);
  }
  .msg-foot button:hover {
    color: var(--fg);
    background: var(--bg-hover);
    border-color: var(--border);
  }
  /* The selection popover: it appears as soon as something is selected, so the hand does not have to travel to the end of the message */
  .sel-pop {
    position: absolute;
    transform: translateX(-50%);
    z-index: 20;
    display: flex;
    gap: 2px;
    background: var(--bg);
    border: 1px solid var(--border);
    border-radius: 6px;
    padding: 2px 3px;
    box-shadow: 0 2px 10px rgba(0, 0, 0, 0.14);
  }
  .sel-pop button {
    font-size: 10px;
    color: var(--fg-muted);
    padding: 1px 7px;
    border-radius: 4px;
    white-space: nowrap;
    max-width: 130px;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .sel-pop button:hover {
    background: var(--bg-hover);
    color: var(--fg);
  }
  /* The user bubble's action bar (edit and resend, fork): the bubble is short, so it hugs the bottom corner on hover */
  .msg.user {
    position: relative;
  }
  .msg-actions {
    position: absolute;
    right: 2px;
    bottom: -10px;
    z-index: 2;
    display: flex;
    gap: 3px;
    opacity: 0;
    transition: opacity 0.12s ease;
    background: var(--bg);
    border: 1px solid var(--border);
    border-radius: 6px;
    padding: 1px 3px;
  }
  .msg:hover .msg-actions,
  .msg-actions:focus-within {
    opacity: 1;
  }
  .msg-actions button {
    font-size: 10px;
    color: var(--fg-muted);
    padding: 1px 6px;
    border-radius: 4px;
    white-space: nowrap;
  }
  .msg-actions button:hover {
    background: var(--bg-hover);
    color: var(--fg);
  }
  /* ---- the system notice rows (compaction, forking) ---- */
  .notice {
    align-self: center;
    font-size: 10px;
    color: var(--fg-faint);
    border: 1px dashed var(--border);
    border-radius: 999px;
    padding: 1px 10px;
  }
  .err-retry {
    display: block;
    margin-top: 4px;
    color: var(--accent);
    font-size: var(--fs-xs);
  }
  .truncated-bar {
    display: flex;
    align-items: center;
    gap: 8px;
    font-size: var(--fs-xs);
    color: var(--fg-muted);
    border: 1px dashed var(--border);
    border-radius: 6px;
    padding: 4px 10px;
  }
  .truncated-bar button {
    color: var(--accent);
    font-size: var(--fs-xs);
  }
  .thinking {
    cursor: pointer;
  }
  .thinking-full {
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .thinking-toggle {
    align-self: flex-start;
    font-size: 10px;
    color: var(--fg-faint);
  }
  .thinking-toggle:hover {
    color: var(--fg-muted);
  }
  .thinking-full pre {
    font-size: var(--fs-xs);
    color: var(--fg-faint);
    font-style: italic;
    white-space: pre-wrap;
    max-height: 240px;
    overflow-y: auto;
    user-select: text;
    margin: 0;
  }
  .usage-inline {
    font-size: 10px;
    font-family: var(--font-mono);
    color: var(--fg-faint);
  }
  .usage-line {
    font-size: 10px;
    font-family: var(--font-mono);
    color: var(--fg-faint);
    text-align: right;
    margin-bottom: 4px;
  }
  .queue-chip {
    display: flex;
    align-items: center;
    gap: 6px;
    font-size: var(--fs-xs);
    color: var(--fg-muted);
    background: var(--bg-hover);
    border-radius: 6px;
    padding: 3px 8px;
    margin: 8px 10px 0;
    min-width: 0;
  }
  .queue-label {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .queue-chip button {
    color: var(--fg-faint);
    flex-shrink: 0;
  }
  .queue-chip button:hover {
    color: var(--fg);
  }
  .src-unverified {
    flex-shrink: 0;
    font-size: 10px;
    color: var(--warning, #b58900);
    border: 1px solid currentColor;
    border-radius: 4px;
    padding: 0 4px;
    opacity: 0.85;
  }
</style>
