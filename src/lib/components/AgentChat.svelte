<script lang="ts">
  // The conversation body (the timeline plus the composer): one implementation shared by the
  // research panel and the detached AI chat window (the second webview) — the session state
  // lives in each window's own agent store, and the source of truth is Rust's session files
  // (the unity principle: one session ever has one visible composer). Only per-instance,
  // harmless presentation state is kept here (expansion, scrolling, the block cache). "The
  // current note" comes from the editor in the main window and from the main window's push in
  // the chat window (agent.hostNote); anything that has to happen in the main window goes
  // through host.ts.
  import { fmtCount } from "$lib/stats";
  import { api, type SkillEntry, AGENT_IMAGE_EXTS, EFFORT_OPTIONS, type EffortValue, isAgentImage, type WebSearchResult } from "$lib/api";
  import { open as openDialog } from "@tauri-apps/plugin-dialog";
  import { agent, blockHandle, type ComposerHandle, type DraftRef, type SourceEntry, type TimelineItem } from "$lib/stores/agent.svelte";
  import type { ActionItem, ReadRequest } from "$lib/api";
  import HoverCard, { type HoverCardData } from "./HoverCard.svelte";
  import { POSITIONAL_RE, TRANSIENT_CITE_RE, blocksOf, findBlock, materializeCitations, parseRefTarget } from "$lib/citation";
  import AtPicker, { linesLabel } from "./AtPicker.svelte";
  import SkillPicker from "./SkillPicker.svelte";
  import NoteSuggest from "./NoteSuggest.svelte";
  import { groupSkills, skillAskByName, skillDesc, skillTitle, skillTitleByName } from "$lib/skills";
  import RefIcon, { type RefIconKind } from "./RefIcon.svelte";
  import PrivacyIcon from "./PrivacyIcon.svelte";
  import { tick, untrack } from "svelte";
  import { hostDispatch } from "$lib/host";
  import { isAgentWindow } from "$lib/window";
  import { vault, baseOf, fileKind, noteBucketOf, parentOf, stemOf } from "$lib/stores/vault.svelte";
  import { clips } from "$lib/stores/clips.svelte";
  import { clipTarget } from "$lib/stores/clipTarget.svelte";
  import { toast } from "$lib/stores/toast.svelte";
  import { prefs } from "$lib/stores/prefs.svelte";
  import { account } from "$lib/stores/account.svelte";
  import { executeCommand } from "$lib/commands/registry";
  import { contextMenu, type MenuEntry } from "$lib/stores/contextMenu.svelte";
  import { t, i18n } from "$lib/i18n/i18n.svelte";
  import { PLAN_GLYPH, toolLabel } from "$lib/views/session";
  import {
    citedNotesHtml,
    domainOf,
    extractCitations,
    handleCodeCopyClick,
    plainText,
    renderAssistantMarkdownBlocks,
    stableBlockPrefix,
    streamingTail,
    TASK_PREFIX_RE,
    type Citation,
  } from "$lib/markdown";
  import { langAttrs, noteLanguage } from "$lib/notelang";
  import { cliptip } from "$lib/cliptip";
  import { tableFades } from "$lib/tablefade";
  import { localDate } from "$lib/views/tasks";
  import { isImeKeydown } from "$lib/ime";
  import { isFileLikePaste } from "$lib/editor/setup";
  import { isMac, k } from "$lib/shortcut";
  import { isUrlLike } from "$lib/urls";
  import GlobeIcon from "$lib/components/GlobeIcon.svelte";
  import { clipSource as clipSourceUrl, openCitation, openClipped } from "$lib/sources";
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
  /** The note open now came out of another AI session (its frontmatter records the id) and
   *  that session can still be resumed: the bar above the composer offers to switch to it. */
  const resumeSession = $derived.by(() => {
    const p = curNote;
    if (!p) return null;
    const sid = vault.notes.find((n) => n.id === p)?.agent_session;
    if (!sid || sid === agent.sessionId) return null;
    return agent.sessions.some((s) => s.id === sid && s.resumable) ? sid : null;
  });
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
    const explicit = materialRefs();
    if (explicit.length >= MAX_REFS) return null;
    if (explicit.some((d) => d.kind === "selection" && selKey(d) === selKey(s))) return null;
    // A stable identity: the same selection reuses the previous object, so downstream deriveds
    // (the chips) do not recompute for nothing on every new snapshot object from CodeMirror or
    // from a push
    if (lastSel && selKey(lastSel) === selKey(s) && lastSel.text === s.text) return lastSel;
    return (lastSel = s);
  });
  /** The chips that count against the reference cap: the vault material — not the attached web
   *  page (its own slot). The method (`/name` at the head of the draft) is no chip at all. */
  function materialRefs(): DraftRef[] {
    return agent.contextDraft.filter((d) => d.kind !== "page");
  }
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
  /** The uncited search hits of a source card, shown on demand (keyed by timeline index like expandedSources). */
  let expandedHits = $state<Record<number, boolean>>({});
  /** How the timeline follows what arrives. "answer": to the bottom, until the answer being
   *  written has its start at the top of the viewport — there it stops, so the text the
   *  reader came for is never pushed out by what arrives under it; "tail": to the bottom,
   *  unconditionally (the reader asked for the end: the ↓ button, or scrolling down to it);
   *  "off": never moves. A send and a session change start "answer"; the reader's own
   *  scrolling picks "tail" at the bottom and "off" anywhere else. */
  let follow = $state<"answer" | "tail" | "off">("answer");
  /** The viewport sits at the end of the timeline (the ↓ button is shown otherwise). */
  let atBottom = $state(true);
  /** Where the last programmatic scroll put the timeline: the scroll event it causes is
   *  told apart from the reader's own by it, so following never reads as the reader
   *  scrolling away, and never closes the hover card or the selection popover. */
  let expectTop = -1;
  /** The reasoning folded above an answer, opened by hand (keyed by timeline index). */
  let expandedThoughts = $state<Record<number, boolean>>({});
  /** Whether the plan card (the task list the assistant recites) is expanded. */
  let planOpen = $state(false);
  /** Whether the thinking stream is expanded into fully selectable text (a narrow, bottom-aligned strip by default). */
  let thinkingOpen = $state(false);
  /** The collapsed reasoning shows its tail from a sentence boundary rather than from the
   *  middle of a word (the buffer is a rolling window, so its start is arbitrary). */
  const thinkingTail = $derived.by(() => {
    const s = agent.thinking;
    if (s.length <= 300) return s;
    const tail = s.slice(-300);
    const m = /[。！？.!?\n]\s*/.exec(tail);
    const after = m ? m.index + m[0].length : -1;
    const from = after > 0 && after < tail.length - 40 ? after : Math.max(0, tail.indexOf(" ") + 1);
    return tail.slice(from);
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
    // The method rides as a reference: a head `/name` that names an enabled skill becomes a
    // ContextRef::Skill (its text joins the envelope first), while the message keeps the token
    // as typed — `/tidy-inbox 只处理今天的` reads the same in the transcript as in the input
    const method = skillHead(text);
    const refs = method ? [{ kind: "skill" as const, path: method }, ...draft] : draft;
    agent.contextDraft = [];
    await agent.send(text, noteForSend, refs);
    await vault.refreshNotes();
  }
  /** The head token of a message: `/name` as its first word (the memo's `#` and a URL are
   *  head-tested the same way). Whether it is a method is decided against the enabled skills. */
  const SKILL_HEAD_RE = /^\/([^\s/]+)(?=\s|$)/;
  /** The method a text leads with, when its head token names an enabled skill. */
  function skillHead(text: string): string | null {
    const m = SKILL_HEAD_RE.exec(text.trimStart());
    return m && knownSkills.has(m[1]) ? m[1] : null;
  }

  /** The one-shot highlight ⌘J lands with (class:summon, the same bool + setTimeout as
   *  SettingsModal's nudge) plus the "press ⌘J again for a detached window" hint (2s, dismissed
   *  by the first keystroke or a focus loss). A cursor landing quietly in the input gives no
   *  sign at all — the user would think nothing happened and go looking for a new window. */
  let summoning = $state(false);
  /** The composer dock's height, published on the root as --chat-dock-h: the detached chat
   *  window lifts its notices above the dock, where they would otherwise cover the send
   *  button (the main window keeps its notices off the panel altogether). */
  let dockH = $state(0);
  $effect(() => {
    document.documentElement.style.setProperty("--chat-dock-h", `${dockH}px`);
  });
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
    // A new, empty session shows the next hint; switching to a session with a history keeps it
    if (!untrack(() => agent.timeline.length)) phIdx = (phIdx + 1) % PH_HINTS.length;
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
    { text: string; blocks: string[]; cites: Citation[]; citeZone: boolean; notes: string[] }
  >();
  /** Each answer's language and writing system (the lang / data-script it is stamped with).
   *  Read once, as soon as there is enough text to tell (noteLanguage's 300 letters — the
   *  first few lines of a streaming answer), and never again: a verdict taken when the
   *  answer finishes would rewrap, in a wide window, a column the reader is halfway
   *  through. A finished answer too short to tell follows the interface. */
  const langCache = new Map<number, ReturnType<typeof langAttrs>>();
  function answerLang(i: number, text: string, streaming: boolean): ReturnType<typeof langAttrs> {
    const known = langCache.get(i);
    if (known) return known;
    const attrs = langAttrs(noteLanguage(text));
    if (attrs["data-script"] || !streaming) langCache.set(i, attrs);
    return attrs;
  }
  /** The render caches are keyed by timeline index: anything that rewrites the timeline drops them. */
  function clearRenderCaches() {
    blockCache.clear();
    langCache.clear();
  }

  /** Merging citations (deduplicated by URL, keeping the first non-empty title — the same rule as extractCitations'). */
  function mergeCites(into: Citation[], add: Citation[]) {
    for (const c of add) {
      const prev = into.find((p) => p.url === c.url);
      if (!prev) into.push({ ...c });
      else if (!prev.title && c.title) prev.title = c.title;
    }
  }

  /** `streaming`: the answer is still arriving — its unclosed tail is held back
   *  (streamingTail), and the cited-notes list waits for the last word. */
  function assistantBlocks(i: number, text: string, streaming: boolean): string[] {
    let cached = blockCache.get(i);
    if (!cached || !text.startsWith(cached.text)) {
      cached = { text: "", blocks: [], cites: [], citeZone: false, notes: [] };
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
      const st = { citeZone: cached.citeZone, notes: cached.notes };
      cached.blocks.push(...renderAssistantMarkdownBlocks(grown, numberOf, st));
      cached.citeZone = st.citeZone;
      cached.text += grown;
      tail = tail.slice(cut);
    }
    // The tail is not frozen, so it renders against copies and the cache is not polluted
    const notes = [...cached.notes];
    const out = [...cached.blocks];
    const shown = streaming ? streamingTail(tail) : tail;
    if (shown) {
      const cites = cached.cites.map((c) => ({ ...c }));
      mergeCites(cites, extractCitations(shown));
      const numberOf = new Map(cites.map((c, n) => [c.url, n + 1] as const));
      out.push(...renderAssistantMarkdownBlocks(shown, numberOf, { citeZone: cached.citeZone, notes }));
    }
    if (!streaming && notes.length) out.push(citedNotesHtml(notes));
    return out;
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
    return [...counts.entries()].map(([n, c]) => (c > 1 ? `${toolLabel(n)} ×${c}` : toolLabel(n))).join(" · ");
  }

  /** A tool call's key argument value (a path, a query and so on), for the run's summary; a
   *  list (clips_lookup's urls) shows its first entry and how many more there are. */
  function argValue(args: string): string {
    try {
      const v = JSON.parse(args) as Record<string, unknown>;
      for (const key of ["path", "query", "name", "note", "text", "folder", "url", "urls", "paths"]) {
        const val = v[key];
        if (typeof val === "string" && val) return val;
        if (Array.isArray(val) && val.length && typeof val[0] === "string") return val.length > 1 ? `${val[0]} +${val.length - 1}` : val[0];
      }
    } catch {
      // The runner previews args at 300 characters, so a long call is cut mid-JSON: the first
      // value of a key argument is still readable by regex ("…" marks a list's remainder)
      const m = /"(?:path|query|name|note|text|folder|url|urls|paths)"\s*:\s*(\[\s*)?"([^"]*)/.exec(args);
      if (m?.[2]) return m[1] ? `${m[2]} …` : m[2];
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

  /** A tool row's argument glance: the key value alone (a path, a query, a skill name), with the
   *  raw JSON left to the hover title; a call without arguments shows nothing rather than `{}`. */
  function argGlance(args: string): string {
    return argValue(args) || (/^\s*\{\s*\}\s*$/.test(args) ? "" : args);
  }

  type PlanItem = { done: boolean; text: string };
  /** One line of `- [ ] text` → `{done, text}`; null when it is not a list item. The plan
   *  card's edit box tolerates a checkbox prefix the user typed out of habit. The check
   *  grammar is shared with the rendering layer through TASK_PREFIX_RE. */
  function stripBullet(line: string): PlanItem | null {
    const li = /^\s*[-*]\s+(.*)$/.exec(line);
    const task = li && TASK_PREFIX_RE.exec(li[1]);
    if (!li || !task) return null;
    return { done: task[1].toLowerCase() === "x", text: li[1].slice(task[0].length) };
  }
  /** The sticky plan card's glyph per step state (the store holds the list; the model
   *  keeps it current through update_plan, and nothing is parsed out of the prose). */
  const planSettled = $derived(agent.plan ? agent.plan.filter((p) => p.status === "done" || p.status === "skipped").length : 0);
  /** The step the folded sticky card names: the one in progress, else the next pending one. */
  const planHeadStep = $derived(
    agent.plan ? (agent.plan.find((p) => p.status === "doing") ?? agent.plan.find((p) => p.status === "pending") ?? null) : null,
  );
  /** The transcript's decided plan cards opened by hand (a decided card folds to one line: the sticky card carries the live list). */
  let expandedPlans = $state<Record<number, boolean>>({});

  /** The tool families behind the status strip's verb; anything else shows its own label. */
  const READ_TOOLS = new Set(["read_note", "read_block", "read_file", "read_skill", "open_for_reading"]);
  const LOOKUP_TOOLS = new Set(["list_notes", "list_files", "query_blocks", "get_links", "clips_lookup", "list_sources", "recent_changes", "aggregate_blocks", "audit_links"]);
  // Every name tools.rs::is_write_tool answers for: a write missing here shows its raw
  // identifier in the status strip instead of "Writing …"
  const WRITE_TOOLS = new Set(["write_note", "edit_note", "move_note", "move_file", "append_note", "set_frontmatter", "update_task", "set_block_field", "restructure_note", "move_blocks"]);
  /** What the agent is doing right now, in words: a gate waiting on the user first, then the
   *  tool in flight (its verb and key argument), a web search, the answer streaming, and
   *  thinking otherwise. It reads this turn's tail — a tool in flight is always its last
   *  entry, since the model only continues once the result is back. */
  /** A gate waiting on the user (plan, pages to read, action items, a write): the status names
   *  it, and the reasoning tail is hidden meanwhile — it was produced before the call and, left
   *  under the card for minutes, reads as if the model were still talking. */
  const gateWaiting = $derived(!!(agent.pendingPlan || agent.pendingRead || agent.pendingActions || agent.pendingPermission));
  const runStatus = $derived.by(() => {
    if (!agent.running) return "";
    const by = agent.activeSkill ? `${t("chat.statusSkill", { name: skillTitleByName(agent.activeSkill) })} · ` : "";
    return by + runVerb();
  });
  const runVerb = () => {
    if (agent.pendingPlan) return t("chat.statusWaitingPlan");
    if (agent.pendingRead) return t("chat.statusWaitingRead");
    if (agent.pendingActions) return t("chat.statusWaitingActions");
    if (agent.pendingPermission) return t("chat.statusWaitingWrite");
    const tl = agent.timeline;
    for (let i = tl.length - 1; i >= 0; i--) {
      const it = tl[i];
      if (it.kind === "user") break;
      if (it.kind === "tool") {
        if (it.ok !== undefined) break;
        const v = argValue(it.args);
        const x = v.length > 40 ? `${v.slice(0, 40)}…` : v;
        if (READ_TOOLS.has(it.name)) return t("chat.statusReading", { x }).trim();
        if (it.name === "search_notes") return t("chat.statusSearching", { x }).trim();
        if (LOOKUP_TOOLS.has(it.name)) return t("chat.statusLooking", { x }).trim();
        if (WRITE_TOOLS.has(it.name)) return t("chat.statusWriting", { x }).trim();
        return t("chat.statusTool", { tool: toolLabel(it.name), x }).trim();
      }
      if (it.kind === "read") {
        if (!it.decision) return t("chat.statusReadingPages"); // the fetch itself (a gate still waiting was answered above)
        break;
      }
      if (it.kind === "sources" && it.searching) return t("chat.searching");
    }
    if (agent.phase === "text") return t("chat.statusAnswering");
    // A long think is a visible one: how much reasoning has come in so far, so a
    // four-minute wait reads as work rather than a hang. The running total, not
    // agent.thinking's length — that window is capped and stops moving at 8k
    const n = agent.thinkingChars;
    return n ? t("chat.thinkingCount", { n: fmtCount(n) }) : t("chat.thinking");
  };

  /** ---- The decision state of the plan gate's (propose_plan) inline card: at most one plan is
   *  pending at a time, so this is one piece of component-level state; a new request resets it on
   *  arrival (the same as PermissionDialog's $effect). */
  let planEditing = $state(false);
  let planEditText = $state("");
  $effect(() => {
    void agent.pendingPlan;
    planEditing = false;
    planEditText = "";
  });
  type PlanCard = Extract<TimelineItem, { kind: "plan" }>;
  function togglePlanEdit(item: PlanCard) {
    if (planEditing) {
      planEditing = false;
      return;
    }
    planEditText = item.items.join("\n");
    planEditing = true;
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
  /** One click and no reason asked: the reason field that used to sit between the click and the
   *  decision was skipped nearly every time. Sent empty, the rejection ends the turn on the Rust
   *  side (runner.rs, PlanRejected::EndTurn), and what should change is the next message. */
  function submitPlanReject() {
    void agent.respondPlan({ kind: "reject", reason: "" });
  }
  function planBadge(d: NonNullable<PlanCard["decision"]>): string {
    if (d.kind === "approve") return t("chat.planApproved");
    if (d.kind === "approve_edited") return t("chat.planApprovedEdited");
    if (d.reason === "cancelled") return t("chat.planCancelled");
    return `${t("chat.planRejected")}${d.reason ? ` · ${d.reason}` : ""}`;
  }

  /** ---- The action-list card (propose_actions): editable while pending — untick, fix the
   *  owner, the date and the target note — and the kept rows go back to the model as the
   *  list to write. `actionRows` is the working copy; a duplicate hint comes from a quick
   *  task query per row (a task with the same key words already in the vault). */
  type ActionsCard = Extract<TimelineItem, { kind: "actions" }>;
  type ActionRow = { keep: boolean; text: string; owner: string; due: string; target: string; source: string; dup: string | null; stay: boolean; fields: { key: string; value: string }[] };
  /** The note a source handle or a target path names: `[[inbox#^b1]]`, `inbox.md` and `inbox`
   *  all read as `inbox`. */
  function noteOf(s: string): string {
    return stemOf(s.replace(/^\[\[/, "").split(/[#|\]]/)[0].trim()).toLowerCase();
  }
  /** The block a source handle names (`[[inbox#^b1]]` → `^b1`, `[[inbox#b3]]` → `b3`), or null
   *  for a quoted sentence. */
  function sourceBlock(s: string): string | null {
    const m = /^\[\[[^#|\]]+#([^|\]]+)\]\]$/.exec(s.trim());
    return m ? blockHandle(m[1].trim()) : null;
  }
  /** A row the model meant to leave put: the explicit `stay` flag alone. A target that is the
   *  source's own note is not a stay — meeting-actions lands a new task line in the meeting note
   *  it was read from, and that is a write to keep ticked. */
  function staysPut(it: ActionItem): boolean {
    return !!it.stay;
  }
  let actionRows = $state<ActionRow[]>([]);
  $effect(() => {
    const req = agent.pendingActions;
    targetPick = null; // the boxes are rebuilt with the rows: no box of the old card has the focus
    if (!req) {
      actionRows = [];
      return;
    }
    // A staying row starts unticked with its target empty: the tick follows the target (a
    // home typed in makes it a move, cleared again it stays), so "write N" only ever counts
    // moves the user asked for
    actionRows = req.items.map((it) => {
      const stay = staysPut(it);
      return {
        keep: !stay,
        text: it.text,
        owner: it.owner ?? "",
        due: it.due ?? "",
        target: stay ? "" : (it.target ?? ""),
        source: it.source ?? "",
        dup: null,
        stay,
        fields: Object.entries(it.fields ?? {}).map(([key, value]) => ({ key, value })),
      };
    });
    // Untracked: the scan reads `actionRows`, which this effect has just replaced with a fresh
    // array — tracking that read would re-run the effect on its own write, without end.
    void untrack(() => findDuplicates(req.id));
  });
  /** A task already in the vault with the row's key words: the row is shown as a likely duplicate and unticked. */
  async function findDuplicates(reqId: string) {
    const rows = actionRows;
    for (let i = 0; i < rows.length; i++) {
      const key = rows[i].text.replace(/\s+/g, " ").trim().slice(0, 16);
      if (key.length < 4) continue;
      try {
        const hits = await api.queryBlocks({ kind: "task", text: key, limit: 2 });
        if (agent.pendingActions?.id !== reqId || actionRows !== rows) return;
        // The row's own source block is no duplicate of itself: a task-shaped capture proposed
        // as a move out of the inbox would otherwise come back unticked as "already in inbox.md"
        const own = sourceBlock(rows[i].source);
        const hit = hits.find((h) => !(own && noteOf(h.note) === noteOf(rows[i].source) && blockHandle(h.block.local_id) === own));
        if (hit) {
          rows[i].dup = hit.note;
          rows[i].keep = false;
        }
      } catch {
        /* a hint only */
      }
    }
  }
  const keptCount = $derived(actionRows.filter((r) => r.keep).length);
  /** Nothing ticked and every unticked row a staying one: the button says "leave them" rather
   *  than "keep none", which would read as throwing the captures away. */
  const allStay = $derived(actionRows.length > 0 && actionRows.every((r) => r.keep || r.stay));
  /** What a staying row's empty target box says: the note it stays in when the source names one. */
  function stayPlaceholder(r: ActionRow): string {
    const note = r.source ? noteOf(r.source) : "";
    return note ? t("chat.actionsStayTarget", { note }) : t("chat.actionsStayPut");
  }
  /** The target box with the focus: its row, the box, the row body its candidate list spans,
   *  and whether that list is open — Esc closes the list while the focus stays, so the box
   *  is still being edited (and shows its raw path rather than the veil). */
  let targetPick = $state.raw<{ row: number; el: HTMLInputElement; within: HTMLElement; open: boolean } | null>(null);
  let suggest = $state<NoteSuggest | null>(null);
  /** The homes already chosen on the card: the candidate list leads with them when the box is empty. */
  const cardTargets = $derived(actionRows.map((x) => x.target.trim()).filter(Boolean));
  function focusTarget(row: number, el: HTMLInputElement) {
    targetPick = { row, el, within: el.closest<HTMLElement>(".act-body") ?? el, open: true };
  }
  function openTargetList() {
    if (targetPick && !targetPick.open) targetPick = { ...targetPick, open: true };
  }
  function closeTargetList() {
    if (targetPick?.open) targetPick = { ...targetPick, open: false };
  }
  /** Typing in a staying row's box: the tick follows it (a home typed in makes it a move,
   *  cleared again it stays); typing in any box brings the list back after an Esc. */
  function typeTarget(r: ActionRow, value: string) {
    if (r.stay) r.keep = value.trim().length > 0;
    openTargetList();
  }
  function pickTarget(path: string) {
    if (!targetPick) return;
    const r = actionRows[targetPick.row];
    if (r) {
      r.target = path;
      if (r.stay) r.keep = true;
    }
    closeTargetList();
  }
  /** The box's keys go to the open list first; ↓ on a closed list reopens it. */
  function targetKey(e: KeyboardEvent) {
    if (suggest?.handleKey(e)) return;
    if (e.key === "ArrowDown" && targetPick && !targetPick.open) {
      e.preventDefault();
      openTargetList();
    }
  }
  function submitActions() {
    const items = actionRows
      .filter((r) => r.keep && r.text.trim())
      .map((r) => ({
        text: r.text.trim(),
        owner: r.owner.trim() || null,
        due: r.due.trim() || null,
        target: r.target.trim() || null,
        source: r.source.trim() || null,
        fields: r.fields.length ? Object.fromEntries(r.fields.filter((f) => f.key.trim()).map((f) => [f.key.trim(), f.value.trim()])) : undefined,
      }));
    void agent.respondActions({ kind: "approve", items });
  }
  /** The same one click as the plan's; the model is told to write nothing from the list and carries on. */
  function submitActionsReject() {
    void agent.respondActions({ kind: "reject", reason: "" });
  }
  function actionsBadge(d: NonNullable<ActionsCard["decision"]>): string {
    if (d.kind === "approve") return t("chat.actionsApproved", { n: d.items.length });
    if (d.reason === "cancelled") return t("chat.actionsCancelled");
    return t("chat.actionsRejected");
  }

  /** ---- The read gate's card (open_for_reading): tick the pages to open, or decline. The
   *  "from now on" checkbox is the session's standing consent: every page the model proposes
   *  afterwards is read without the card's buttons, for the rest of the session (kept by the
   *  app, src-tauri's consents.rs, not in the vault). */
  type ReadCard = Extract<TimelineItem, { kind: "read" }>;
  let readChecks = $state<boolean[]>([]);
  let readAuto = $state(false);
  $effect(() => {
    const req = agent.pendingRead;
    readChecks = req ? req.urls.map(() => true) : [];
    readAuto = false;
  });
  const readCount = $derived(readChecks.filter(Boolean).length);
  function submitRead(req: ReadRequest) {
    const urls = req.urls.filter((_, i) => readChecks[i]);
    void agent.respondRead({ kind: "allow", urls, auto_sources: readAuto });
  }
  function declineRead() {
    void agent.respondRead({ kind: "deny", reason: "" });
  }
  function readBadge(d: NonNullable<ReadCard["decision"]>): string {
    if (d.kind === "pages") return d.urls.length ? t(d.auto ? "chat.readDoneAuto" : "chat.readDone", { n: d.urls.length }) : t("chat.readNone");
    if (d.reason === "cancelled") return t("chat.actionsCancelled");
    return t("chat.readDeclined");
  }

  /** m:ss, h:mm:ss past an hour — the strip's clock. */
  function fmtDuration(ms: number): string {
    const s = Math.max(0, Math.floor(ms / 1000));
    const h = Math.floor(s / 3600);
    const m = Math.floor((s % 3600) / 60);
    return `${h ? `${h}:` : ""}${h ? String(m).padStart(2, "0") : m}:${String(s % 60).padStart(2, "0")}`;
  }
  /** The clock ticks only while a turn runs: started with the run, stopped with it. */
  let now = $state(Date.now());
  $effect(() => {
    if (!agent.running) return;
    now = Date.now();
    const it = setInterval(() => (now = Date.now()), 1000);
    return () => clearInterval(it);
  });
  const elapsedText = $derived(agent.running && agent.turnStartedAt != null ? fmtDuration(now - agent.turnStartedAt) : "");
  /** The iteration budget (a step = one model-tool round trip; the runner asks the model to
   *  wrap up with five left) is shown only once it is nearly spent — before that the count
   *  says nothing a user can act on. */
  const STEPS_LEFT_WARN = 5;
  /** Before a close reading on the hosted provider: the balance against what one reading
   *  draws (the server's estimate, doubled at peak). Shown while the draft leads with /study
   *  and the balance falls short, so a run is not cut off partway unannounced — a trial's
   *  last credits most of all. It warns and does not block: the estimate is a median. */
  const studyShort = $derived.by(() => {
    if (prefs.provider !== "hosted" || !account.hostedOk || skillHead(agent.draft) !== "study") return "";
    const me = account.me;
    if (!me) return "";
    const have = Math.max(0, me.quota ? me.quota.total_fen - me.quota.used_fen : 0) + me.pack_fen;
    const need = me.estimates.study_fen * (me.peak ? 2 : 1);
    if (need <= 0 || have >= need) return "";
    const count = new Intl.NumberFormat(i18n.locale, { maximumFractionDigits: 0 });
    return t(me.peak ? "chat.studyShortPeak" : "chat.studyShort", { have: count.format(Math.max(0, have)), need: count.format(need) });
  });
  const stepsLeftText = $derived.by(() => {
    const u = agent.usage;
    if (!agent.running || !u) return "";
    const left = u.max - u.iteration;
    return left <= STEPS_LEFT_WARN ? t("chat.stepsLeft", { n: left }) : "";
  });
  /** After the turn: how long it took and the context size; the calls, the output and the cache hits are in the hover. */
  const usageIdleText = $derived.by(() => {
    const u = agent.usage;
    if (!u) return "";
    const context = t("chat.usageContext", { tokens: fmtCount(u.prompt) });
    return agent.lastTurnMs != null ? t("chat.usageIdle", { t: fmtDuration(agent.lastTurnMs), context }) : context;
  });
  const usageIdleHint = $derived.by(() => {
    const u = agent.usage;
    if (!u) return "";
    const cache = u.prompt > 0 ? `${Math.round((u.cacheHit / u.prompt) * 100)}%` : "0%";
    const searches = u.searches ? t("chat.usageSearches", { n: u.searches }) : "";
    return `${t("chat.usageLastTurn", { calls: u.iteration, prompt: u.prompt.toLocaleString(i18n.locale), cache, completion: u.completion.toLocaleString(i18n.locale) })}${searches}`;
  });

  /** On a session change, clear the expansion state keyed by timeline index, so it is not attached to the new timeline at the wrong place. */
  function resetTimelineUi() {
    expandedTools = {};
    expandedToolRuns = {};
    expandedSources = {};
    expandedHits = {};
    expandedPlans = {};
    expandedThoughts = {};
    clearRenderCaches();
    follow = "answer"; // a session opens on its latest answer, read from its start
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
    fm.push("status: draft");
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
      // The saved note becomes the working note: the rest of the research lands in it without a dialog per paragraph
      await agent.setFollow(path);
      toast.push(t("chat.savedAs", { path }));
    } catch (e) {
      toast.push(String(e), "error");
    }
  }

  // ---- Retry / edit and resend / fork / continue ----

  function doRetry() {
    clearRenderCaches();
    run(agent.retry().then(() => vault.refreshNotes()));
  }

  async function editLast() {
    clearRenderCaches();
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
    clearRenderCaches();
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

  /** The turn id of the user message a bubble answers (undo is offered per turn). */
  function turnIdFor(i: number): string | undefined {
    for (let j = i; j >= 0; j--) {
      const it = agent.timeline[j];
      if (it.kind === "user") return it.turn;
    }
    return undefined;
  }
  /** Whether this bubble's turn landed writes that can still be undone. */
  function undoableTurn(i: number): string | null {
    const turn = turnIdFor(i);
    if (!turn || agent.undoneTurns[turn] || !(agent.writesByTurn[turn] > 0)) return null;
    return turn;
  }
  async function undoTurn(turn: string) {
    try {
      const r = await agent.undoTurn(turn);
      toast.push(t("chat.undone", { n: r.reverted, skipped: r.skipped ? t("chat.undoneSkipped", { n: r.skipped }) : "" }));
      await vault.refreshNotes();
    } catch (e) {
      toast.push(String(e), "error");
    }
  }
  /** The notes this bubble's turn wrote that still hold unreviewed AI blocks, with their
   *  count — the strip under the answer that starts the turn's review. Null when the turn
   *  wrote nothing, or everything it wrote has been looked at (or undone). */
  function turnReview(i: number): { paths: string[]; blocks: number } | null {
    const turn = turnIdFor(i);
    const written = turn && !agent.undoneTurns[turn] ? agent.notesByTurn[turn] : undefined;
    if (!written?.length) return null;
    const pending = vault.pendingNotes.filter((p) => written.includes(p.path));
    if (!pending.length) return null;
    return { paths: pending.map((p) => p.path), blocks: pending.reduce((n, p) => n + p.blocks, 0) };
  }

  const run = (p: Promise<unknown>) => void p.catch((e) => toast.push(String(e), "error"));

  const urlLike = $derived(isUrlLike(agent.draft.trim()));
  /** A leading `#` means recording one convention into VEMRY.md.
   *  The same dual mode as a URL's: ⌥↵ still sends it verbatim as a question. */
  const memoText = $derived(agent.draft.trim().startsWith("#") ? agent.draft.trim().replace(/^#\s*/, "") : "");
  const memoLike = $derived(memoText.length > 0);

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

  /** A block citation in the rendered answer: the numbered marker, and the chip form
   *  (an alias the model wrote) — both carry the fragment. */
  const CITE_SEL = "a[data-frag]";
  function onTimelineOver(e: MouseEvent) {
    if (!(e.target instanceof Element)) return;
    const el = e.target.closest<HTMLElement>(CITE_SEL);
    if (!el) return;
    if (hoverTimer) clearTimeout(hoverTimer);
    const target = el.dataset.wiki ?? "";
    hoverTimer = setTimeout(() => void showHover(el, target), 250);
  }
  function onTimelineOut(e: MouseEvent) {
    if (!(e.target instanceof Element) || !e.target.closest(CITE_SEL)) return;
    if (hoverTimer) clearTimeout(hoverTimer);
    hoverTimer = null;
    hover = null;
    hoverFor = null;
  }
  /** The citation the hover card is for: an answer that resolves after the pointer moved on is dropped. */
  let hoverFor: string | null = null;
  /** The card's head: the note's name, and the heading when the citation names a section. */
  function citeTitle(note: string, frag: string): string {
    const name = (note.split("/").pop() ?? note).replace(/\.md$/i, "") || t("chat.thisNote");
    return frag.startsWith("^") || POSITIONAL_RE.test(frag) ? name : `${name} › ${frag}`;
  }
  async function showHover(el: HTMLElement, target: string) {
    if (!el.isConnected) return;
    const r = el.getBoundingClientRect();
    const { note, frag } = parseRefTarget(target);
    hoverFor = target;
    hover = { x: r.left, y: r.bottom + 6, title: citeTitle(note, frag), text: t("hover.loading") };
    const text = await citationText(note, frag);
    if (!hover || hoverFor !== target) return;
    hover = { ...hover, text: text ?? t("hover.missing"), missing: text == null };
  }

  /** The one funnel for text leaving the panel into the vault or the clipboard:
   *  positional citations become stable ids written into the cited notes (a note that
   *  is open and dirty flushes first, or autosave would clobber the ` ^id`). */
  async function exportAnswer(text: string): Promise<string> {
    TRANSIENT_CITE_RE.lastIndex = 0;
    if (!TRANSIENT_CITE_RE.test(text)) return text;
    if (vault.currentPath && vault.dirty) await vault.flushEditor?.();
    const r = await materializeCitations(text, (note, frag) => citeGuards.get(`${note}#${frag}`), vault.currentPath ?? undefined, vault.indexVersion);
    if (r.minted) toast.push(t("chat.citationsMaterialized", { n: r.minted }));
    if (r.failed) toast.push(t("chat.citationsDropped", { n: r.failed }), "error");
    return r.text;
  }

  const clipSource = (r: WebSearchResult) => clipSourceUrl(r.url, r.title);

  function openPage(url: string) {
    agent.draft = "";
    run(api.browserOpen(url));
  }

  /** The enabled skills (built-in and the vault's own): the welcome's method chips by scene,
   *  the `/` picker's candidates, and the names that make a head `/name` the message's method.
   *  One list, fetched on mount, when the panel empties, and whenever the picker opens (a
   *  directory scan; the list is short) — the picker and the head test cannot disagree. */
  let skills = $state<SkillEntry[]>([]);
  const knownSkills = $derived(new Set(skills.map((x) => x.name)));
  function refreshSkills() {
    api
      .skillsList()
      .then((s) => (skills = s.filter((x) => !x.disabled)))
      .catch(() => {});
  }
  const welcome = $derived(agent.timeline.length === 0);
  $effect(() => {
    void welcome;
    refreshSkills();
  });
  const welcomeGroups = $derived(groupSkills(skills));
  /** The secondary scenes (daily, product) unfolded on the welcome; folded to start, and kept as the user left it while the panel lives. */
  let welcomeMoreOpen = $state(false);
  const welcomeShown = $derived(welcomeMoreOpen ? welcomeGroups : welcomeGroups.filter((g) => !g.secondary));
  const welcomeHidden = $derived(welcomeGroups.filter((g) => g.secondary).reduce((n, g) => n + g.skills.length, 0));
  /** A welcome chip toggles the head token: the chosen method reads as pressed, and clicking it
   *  again takes `/name ` back out of the draft — so a slip of the mouse costs one more click.
   *  Another method replaces the token; whatever was typed after it stays. */
  function useSkill(name: string) {
    const rest = draftSkill ? agent.draft.trimStart().replace(SKILL_HEAD_RE, "").trimStart() : agent.draft;
    agent.draft = draftSkill === name ? rest : `/${name} ${rest}`;
    const el = composerEl;
    el?.focus();
    queueMicrotask(() => el?.setSelectionRange(el.value.length, el.value.length));
  }

  // The placeholder: the main sentence, or one of the three syntaxes @ / # / a URL — each one
  // line, so it never grows into a two-line explanation. It holds still: a sentence sliding
  // in beside an answer being read pulls the eye away every few seconds, so each new session
  // shows the next one instead, and a new vault starts from the main sentence. While running
  // it says a message would be queued. A .ph layer over the textarea stands in for the
  // native placeholder (the textarea itself sets none).
  const PH_HINTS = $derived([
    t("chat.placeholder1"),
    t("chat.placeholder2"),
    t("chat.placeholder3"),
    t("chat.placeholder4"),
  ]);
  /** The message's method: the draft's head token when it names an enabled skill. It stays
   *  where it was typed, as the coding agents' slash commands do — the picker only completes
   *  it — rather than turning into a chip in the row meant for the material. */
  const draftSkill = $derived(skillHead(agent.draft));
  /** The hint strip while a method leads the draft: the method by its title, then what it asks
   *  for (a topic, a note to @, or nothing but Enter); a vault's own skill has no ask line. */
  const skillHint = $derived.by(() => {
    if (!draftSkill) return "";
    const name = skillTitleByName(draftSkill);
    const ask = skillAskByName(draftSkill);
    return ask ? `${t("chat.skillModeHint", { name })} · ${ask}` : t("chat.skillPlaceholder", { name });
  });
  let phIdx = $state(0);

  /** The stop button takes the send button's slot the moment a turn starts (send() clears the
   *  draft synchronously and runTurn sets running before its first await), so the second click
   *  of a double-click on ↑ would land on ■ and cancel the turn it just sent. A stop within the
   *  first moments of a turn is ignored; Enter is unaffected (it never reaches this button). */
  const STOP_GRACE_MS = 400;
  function stopTurn() {
    if (agent.turnStartedAt != null && Date.now() - agent.turnStartedAt < STOP_GRACE_MS) return;
    agent.cancel();
  }

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
    follow = "answer"; // a message you sent yourself always has to be visible, and its answer is read from the start
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
    // A method at the head (`/name`) changes nothing about ⏎; the strip then says what the method asks for (empty otherwise)
    queue: { hint: skillHint, title: t("chat.queueTitle"), aria: t("chat.queueAria"), run: () => void send() },
    send: { hint: skillHint, title: t("chat.sendTitle"), aria: t("chat.sendAria"), run: () => void send() },
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
   *  the URL and # memo dual modes, which test the whole string as a prefix.
   *  The one exception is a token Tab completed to `@Note name#…`: a note part with single
   *  spaces inside is accepted when a `#` follows it directly, since otherwise the completed
   *  token would close the candidate layer on the spot. Without the `#` a space still ends the
   *  token, so typing on in prose after an @ is unchanged. */
  const AT_RE = /(^|[\s(（【[])@([^\s@#]+(?: [^\s@#]+)*#[^\s@]*|[^\s@]*)$/;
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
    detectSlash(upto);
  }
  function closeAt() {
    atQuery = null;
    atStart = -1;
  }
  // ---- the / skill picker: the method for the message, chosen by name ----
  let skillPicker = $state<SkillPicker | null>(null);
  /** null = not triggered; non-null = the filter text between the / and the cursor. */
  let slashQuery = $state<string | null>(null);
  let slashStart = -1;
  /** The / triggers only as the draft's first token (the caret inside it), since the method is
   *  the head of the message — a URL (`https://…`) or a path (`research/x`) further in never
   *  opens it, and the memo (`#`) and URL modes, which test the whole draft, are untouched. */
  const SLASH_RE = /^\s*\/([^\s/]*)$/;
  function detectSlash(upto: string) {
    const m = SLASH_RE.exec(upto);
    if (m) {
      if (slashQuery == null) refreshSkills();
      slashQuery = m[1];
      slashStart = upto.length - m[1].length - 1;
    } else {
      closeSlash();
    }
  }
  function closeSlash() {
    slashQuery = null;
    slashStart = -1;
  }
  /** A skill was chosen: the token is completed in place to `/name ` with the caret after it,
   *  ready for what the method takes — nothing moves and nothing is renamed (the chip that used
   *  to replace the token jumped to the material row under a translated title, and read as odd). */
  function onSlashPick(name: string) {
    const el = composerEl;
    if (slashStart >= 0) {
      const end = slashStart + 1 + (slashQuery?.length ?? 0);
      const token = `/${name} `;
      agent.draft = agent.draft.slice(0, slashStart) + token + agent.draft.slice(end);
      const caret = slashStart + token.length;
      queueMicrotask(() => {
        el?.focus();
        el?.setSelectionRange(caret, caret);
      });
    }
    closeSlash();
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
    if (d.kind === "view") return d.label;
    if (d.kind === "block") return baseOf(d.path);
    const name = baseOf(d.path);
    return d.kind === "folder" ? `${name}/` : name;
  }
  /** A chip's hover: the path; a selection adds its line range and the start of its text (a glance at which passage is attached). */
  function refTitle(d: DraftRef): string {
    if (d.kind === "page") return d.page.url;
    if (d.kind === "view") return t("chat.viewTitle", { view: d.label, n: d.rows ?? 0 });
    // The image chip's permanent hint: no table of model capabilities is attempted (it cannot be judged reliably), and a text-only model will error on one
    if (d.kind === "image") return t("chat.needsMultimodal", { path: d.path });
    if (d.kind === "block") return `${d.path} ¶${blockHandle(d.local_id)}${d.label ? `\n\n${d.label}` : ""}`;
    if (d.kind !== "selection") return d.path;
    const head = d.text.length > 300 ? `${d.text.slice(0, 300)}…` : d.text;
    return `${d.path} ${linesLabel(d)}\n\n${head}`;
  }
  /** The identity shared by the chips and the deduplication: two different selections in one note are two chips. */
  function refKey(d: DraftRef): string {
    if (d.kind === "page") return `page:${d.page.url}`;
    if (d.kind === "view") return `view:${d.path}`;
    if (d.kind === "selection") return `selection:${selKey(d)}`;
    if (d.kind === "block") return `block:${d.path}#${d.local_id}`;
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
    /** A right-click menu (the working-note chip: auto-apply on or off, open, stop). */
    menu?: (e: MouseEvent) => void;
    /** A click on the label (the working-note chip opens the note). */
    open?: () => void;
  }
  /** The working-note chip's menu: whether its local writes apply without a dialog, open it, stop following. */
  function followMenu(e: MouseEvent) {
    const f = agent.follow;
    if (!f) return;
    contextMenu.open(e, [
      { title: t("chat.followMenuTitle") },
      { label: `${f.auto ? "✓ " : ""}${t("chat.followAutoOn")}`, action: () => run(agent.setFollow(f.path, true)) },
      { label: `${f.auto ? "" : "✓ "}${t("chat.followAutoOff")}`, action: () => run(agent.setFollow(f.path, false)) },
      { separator: true },
      { label: t("chat.followOpen"), action: () => void hostDispatch({ kind: "open-note", path: f.path }) },
      { label: t("chat.unfollow"), action: () => run(agent.setFollow(null).then(() => toast.push(t("chat.unfollowed")))) },
    ]);
  }
  const chips = $derived.by(() => {
    const out: Chip[] = [];
    const f = agent.follow;
    if (f) {
      out.push({
        key: "follow",
        kind: "note",
        cur: false,
        title: t(f.auto ? "chat.followTitle" : "chat.followTitleAsk", { note: f.path }),
        label: baseOf(f.path),
        lines: null,
        tag: t("chat.followTag"),
        removeLabel: t("chat.unfollow"),
        remove: () => run(agent.setFollow(null).then(() => toast.push(t("chat.unfollowed")))),
        menu: followMenu,
        open: () => void hostDispatch({ kind: "open-note", path: f.path }),
      });
    }
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
        lines: d.kind === "selection" ? linesLabel(d) : d.kind === "block" ? `¶${blockHandle(d.local_id)}` : null,
        tag: d.kind === "page" && d.page.truncated ? t("chat.tagTruncated") : d.kind === "view" ? t("chat.viewTag") : null,
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
  /** Tab on a note row: the typed note part (the text between the @ and the caret) is replaced
   *  by the completed one plus a `#`, the caret lands after the `#`, and detectAt reads the
   *  token back as `@Note#` — the same text a hand-typed `#` would have left. */
  async function onAtStage(notePart: string) {
    const el = composerEl;
    if (!el || atStart < 0) return;
    const caret = el.selectionStart ?? agent.draft.length;
    const text = `${notePart}#`;
    agent.draft = agent.draft.slice(0, atStart + 1) + text + agent.draft.slice(caret);
    await tick();
    el.focus();
    const at = atStart + 1 + text.length;
    el.setSelectionRange(at, at);
    detectAt();
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
    if (materialRefs().length + auto >= MAX_REFS) {
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
        if (pushRef({ kind: "image", path: rel }) && prefs.imagesUnsupported && !warnedVision) {
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
    const rels = await vault.importFiles(paths, bucket); // a failure lands in vault.error, which the app announces
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
    const rels = await vault.pasteIntoNote(Array.from(dt.files), bucket); // as in pickFiles, a failure is announced by the app
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
  /** The model chip's hover: the model (changed in Settings) and the thinking level; without a model, the level alone. */
  const modelChipHint = $derived(prefs.model ? t("chat.modelChipHint", { model: prefs.model }) : t("chat.effortHint"));

  /** ---- The web search switch: one item of the model menu (the settings page writes the same
   *  field), read afresh by every send, so a click takes effect from the next message. The item
   *  is disabled with the reason when the endpoint cannot search; while a search may run, a
   *  globe sits on the model label — what the user sees before sending is whether the message
   *  can leave the machine, and the switch itself stays out of the bar. */
  const searchOn = $derived(prefs.webSearchAvailable && prefs.webSearchEnabled);
  async function toggleSearch() {
    if (!prefs.webSearchAvailable) return;
    try {
      await api.settingsSet({ web_search_enabled: !prefs.webSearchEnabled });
      await prefs.refresh();
    } catch (err) {
      toast.push(String(err));
    }
  }
  /** The model chip's menu: the model with the way to Settings (the main window's — the
   *  detached window has no settings dialog to open), then the thinking level's quick switch. */
  function modelMenu(e: MouseEvent) {
    const entries: MenuEntry[] = [];
    if (prefs.model) {
      entries.push({ title: prefs.model });
      // The detached window has no settings of its own: hostDispatch relays it to the main window, brought to the front
      entries.push({ label: t("chat.changeModel"), action: () => void hostDispatch({ kind: "settings", id: "ai.model" }) });
    }
    if (prefs.reasoningEffortActive) {
      if (entries.length) entries.push({ separator: true });
      entries.push(
        { title: t("chat.effortMenuTitle") },
        {
          label: t("chat.effortFollowGlobal", { level: effortName(prefs.reasoningEffort) }),
          checked: agent.effortOverride === null,
          action: () => (agent.effortOverride = null),
        },
        { separator: true },
        ...EFFORT_OPTIONS.map((o) => {
          const hint = t(`settings.effort.${o.value}Hint`);
          return {
            label: `${effortName(o.value)}${hint ? `(${hint})` : ""}`,
            checked: agent.effortOverride === o.value,
            action: () => (agent.effortOverride = o.value),
          };
        }),
      );
    }
    if (entries.length) entries.push({ separator: true });
    entries.push(
      prefs.webSearchAvailable
        ? { label: t("chat.searchMenuItem"), checked: prefs.webSearchEnabled, action: () => void toggleSearch() }
        : { label: t("chat.searchUnavailable"), disabled: true },
    );
    contextMenu.open(e, entries);
  }

  function onkeydown(e: KeyboardEvent) {
    if (summonHint) hideSummonHint();
    // With the candidate overlay open the keyboard gives way first (↑/↓/Enter/Tab/Esc belong to it, and everything else types as usual)
    if (atQuery != null && picker?.handleKey(e)) return;
    if (slashQuery != null && skillPicker?.handleKey(e)) return;
    if (e.key !== "Enter" || e.shiftKey || isImeKeydown(e)) return;
    e.preventDefault();
    submit(e.altKey);
  }

  /** Within this distance of the end the viewport counts as at the bottom. */
  const BOTTOM_SLACK = 40;
  /** Where "answer" stops: the answer's start this far below the viewport's top edge (the column's own padding). */
  const ANSWER_TOP_GAP = 12;

  function onScroll() {
    const el = scroller;
    if (!el) return;
    // A scroll this component made (following): the mode stays, and the popovers have moved with the text
    if (expectTop >= 0 && Math.abs(el.scrollTop - expectTop) <= 1) {
      expectTop = -1;
      return;
    }
    expectTop = -1;
    // The reader's own scroll: the popovers are positioned against the wrap and the viewport
    // and do not scroll with the content, so they close (as selection popovers commonly do)
    hover = null;
    selPop = null;
    atBottom = el.scrollHeight - el.scrollTop - el.clientHeight < BOTTOM_SLACK;
    follow = atBottom ? "tail" : "off";
  }

  /** Scroll the timeline by program: the popovers ride along with the text they point at
   *  (and close once it has left the view), and the scroll event this causes is marked as ours. */
  function scrollTimeline(el: HTMLElement, top: number) {
    const before = el.scrollTop;
    el.scrollTop = top;
    const delta = el.scrollTop - before;
    if (!delta) return;
    expectTop = el.scrollTop;
    if (selPop) {
      const y = selPop.y - delta;
      selPop = y < 0 || y > el.clientHeight ? null : { ...selPop, y };
    }
    if (hover) {
      const y = hover.y - delta;
      const r = el.getBoundingClientRect();
      hover = y < r.top || y > r.bottom ? null : { ...hover, y };
    }
  }

  /** Where "answer" stops: the top of this turn's newest answer (its folded reasoning
   *  included), in the scroller's coordinates; null while the turn has no answer yet. */
  function answerTop(el: HTMLElement): number | null {
    const i = lastAssistantIndex;
    if (i < 0 || i < lastUserIndex) return null;
    const node = el.querySelector<HTMLElement>(`[data-answer="${i}"]`);
    if (!node) return null;
    return Math.max(0, node.getBoundingClientRect().top - el.getBoundingClientRect().top + el.scrollTop - ANSWER_TOP_GAP);
  }

  function scrollToBottom() {
    follow = "tail";
    if (scroller) scrollTimeline(scroller, scroller.scrollHeight - scroller.clientHeight);
  }

  $effect(() => {
    // Follow what arrives, per `follow`. Appending token by token changes only the last
    // item's text and not the length, so both have to be tracked.
    // The visibility switch is in the same list (the same reason as autogrow's): under
    // display:none scrolling is a no-op, and on becoming visible again the mode has to be
    // applied once more
    const last = agent.timeline[agent.timeline.length - 1];
    if (last && "text" in last) void last.text;
    void agent.timeline.length;
    void agent.thinking;
    void vault.rightTab;
    void vault.rightOpen;
    const mode = follow;
    const el = scroller;
    if (!el || el.clientHeight === 0) return;
    untrack(() => {
      const max = el.scrollHeight - el.clientHeight;
      if (mode !== "off") {
        let target = max;
        if (mode === "answer") {
          const top = answerTop(el);
          // The answer's start can reach the top: it goes there, and following ends
          if (top != null && top <= max) {
            target = top;
            follow = "off";
          }
        }
        if (Math.abs(target - el.scrollTop) >= 1) scrollTimeline(el, target);
      }
      atBottom = max - el.scrollTop < BOTTOM_SLACK;
    });
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
  <!-- The panel's disclosure caret: three collapsing heads (the sticky plan card, a decided
       plan card, a source card) turn the same drawing, each rotated by its own `.open` class. -->
  {#snippet sourceRow(r: SourceEntry, n: number)}
    <div class="source-row">
      <button class="source-main" title={r.publish_date ? `${r.url} · ${r.publish_date}` : r.snippet || r.url} onclick={(e) => openCitation(r.url, e.metaKey || e.ctrlKey)}>
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
  {/snippet}
  <!-- The answer's action icons (14px strokes on the 20 grid, the composer's own) -->
  {#snippet footIcon(kind: "copy" | "insert" | "pin" | "save" | "regenerate" | "undo")}
    <svg width="14" height="14" viewBox="0 0 20 20" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
      {#if kind === "copy"}
        <rect x="7" y="7" width="9.5" height="9.5" rx="2" /><path d="M13 4.5V4.4a1.9 1.9 0 0 0-1.9-1.9H5.4a1.9 1.9 0 0 0-1.9 1.9v5.7A1.9 1.9 0 0 0 5.4 12h.1" />
      {:else if kind === "insert"}
        <path d="M15.5 4v5.5a2 2 0 0 1-2 2h-9M8 8l-3.5 3.5L8 15" />
      {:else if kind === "pin"}
        <path d="M3.5 10h9M9 6.5l3.5 3.5L9 13.5M16.5 4v12" />
      {:else if kind === "save"}
        <path d="M11.5 2.5H6a2 2 0 0 0-2 2v11a2 2 0 0 0 2 2h8a2 2 0 0 0 2-2V7z" /><path d="M11.5 2.5V7H16M10 10v5M7.5 12.5h5" />
      {:else if kind === "regenerate"}
        <path d="M16 10a6 6 0 1 1-1.8-4.3" /><path d="M16.5 3.5v3.5H13" />
      {:else}
        <path d="M4 10a6 6 0 1 1 1.8 4.3" /><path d="M3.5 16.5V13H7" />
      {/if}
    </svg>
  {/snippet}
  {#snippet caret()}
    <svg width="12" height="12" viewBox="0 0 16 16" fill="none"><path d="M6 4l4 4-4 4" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"/></svg>
  {/snippet}
  {#if agent.plan}
    <!-- The step in progress is marked by its accent glyph and holds still: while a turn runs
         the status strip's dot is the one thing that moves -->
    <div class="plan-card">
      <button class="plan-head" title={t(planOpen ? "chat.planCollapseHint" : "chat.planExpandHint")} onclick={() => (planOpen = !planOpen)}>
        <span class="plan-caret" class:open={planOpen} aria-hidden="true">
          {@render caret()}
        </span>
        <span class="plan-head-count">{t("chat.planProgress", { done: planSettled, total: agent.plan.length })}</span>
        <!-- Folded (the default), the head names the step in progress or the next one: progress
             and the current activity at a glance, with the whole list one click away rather than
             taking a third of a narrow panel -->
        {#if !planOpen && planHeadStep}
          <span class="plan-head-step" class:doing={planHeadStep.status === "doing"}>
            <span class="plan-check">{PLAN_GLYPH[planHeadStep.status] ?? "☐"}</span><span class="plan-head-text" use:cliptip={planHeadStep.text}>{planHeadStep.text}</span>
          </span>
        {/if}
      </button>
      {#if planOpen}
        {#each agent.plan as p, pi (pi)}
          <div
            class="plan-item"
            class:done={p.status === "done"}
            class:doing={p.status === "doing"}
            class:skipped={p.status === "skipped"}
            title={p.status === "doing" ? t("chat.planStatusDoing") : p.status === "skipped" ? t("chat.planStatusSkipped") : undefined}
          >
            <span class="plan-check">{PLAN_GLYPH[p.status] ?? "☐"}</span>{p.text}
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
    use:tableFades
    onscroll={onScroll}
    onclick={onTimelineClick}
    onmouseup={onTimelineMouseUp}
    onmouseover={onTimelineOver}
    onmouseout={onTimelineOut}
  >
    <!-- .turns exists so that a press in the padding or in a flex gap never targets the scroll
      container itself: for that target WebKit resolves the selection anchor as if the container
      were not scrolled, and a drag started in a gap selects a turn that is scrolled far above -->
    <div class="turns">
    {#if agent.timeline.length === 0}
      <div class="welcome">
        <p class="welcome-hint">
          {t("chat.welcomeBody")}
        </p>
        {#if welcomeGroups.length}
          <p class="welcome-lead">
            {t("chat.welcomeSkills")}<span class="faint"> · {t("chat.welcomeSkillsHint")}</span>
          </p>
          <div class="welcome-groups">
            {#each welcomeShown as g (g.label)}
              <span class="welcome-group-label">{g.label}</span>
              <div class="welcome-examples">
                {#each g.skills as s (s.name)}
                  <button class="example" class:on={draftSkill === s.name} aria-pressed={draftSkill === s.name} title={`/${s.name} — ${skillDesc(s)}`} onclick={() => useSkill(s.name)}>{skillTitle(s)}</button>
                {/each}
              </div>
            {/each}
            {#if welcomeHidden > 0}
              <span class="welcome-group-label"></span>
              <div class="welcome-examples">
                <button class="example more" aria-expanded={welcomeMoreOpen} onclick={() => (welcomeMoreOpen = !welcomeMoreOpen)}>
                  {welcomeMoreOpen ? t("chat.welcomeLess") : t("chat.welcomeMore", { n: welcomeHidden })}
                </button>
              </div>
            {/if}
          </div>
        {/if}
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
        <!-- A mixed run's breakdown wraps rather than being cut: in a narrow panel the ledger's
             one line is not enough for four tool names, and a second line costs less than losing them -->
        <button class="tool-row" class:wrap={row.mixed} onclick={() => (expandedToolRuns[row.start] = !open)}>
          <span class="tool-status" class:err={errs > 0} class:pending>
            {pending ? "◌" : open ? "▾" : "▸"}
          </span>
          <span class="tool-name">{row.mixed ? t("tool.mixed") : toolLabel(row.name)}</span>
          <span class="tool-args">
            {row.mixed ? runLabel(row) : `×${row.items.length}`}{errs ? t("chat.toolFailures", { n: errs }) : ""}{row.mixed ? "" : runDigest(row)}
          </span>
        </button>
        {#if open}
          {#each row.items as it (it.i)}
            <button class="tool-row tool-sub" onclick={() => (expandedTools[it.i] = !expandedTools[it.i])}>
              <span class="tool-status" class:err={it.item.ok === false} class:pending={it.item.ok === undefined}>
                {it.item.ok === undefined ? "◌" : it.item.ok ? "▸" : "✕"}
              </span>
              {#if row.mixed}<span class="tool-name">{toolLabel(it.item.name)}</span>{/if}
              <span class="tool-args" title={it.item.args}>{argGlance(it.item.args)}</span>
            </button>
            {#if expandedTools[it.i] && it.item.output}
              <pre class="tool-output">{it.item.output}</pre>
            {/if}
          {/each}
        {:else if pending}
          <button class="tool-row tool-sub" onclick={() => (expandedToolRuns[row.start] = true)}>
            <span class="tool-status pending">◌</span>
            {#if row.mixed}<span class="tool-name">{toolLabel(last.item.name)}</span>{/if}
            <span class="tool-args" title={last.item.args}>{argGlance(last.item.args)}</span>
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
            {#each item.refs as r (`${r.kind}:${r.path}:${r.lines?.join("-") ?? ""}:${r.block ?? ""}`)}
              <!-- A reference marked private afterwards: its path is no longer shown (a placeholder
                   rather than hiding it — it really was sent at the time, and disappearing entirely
                   would distort the record the other way; a privacy change takes effect reactively
                   through privatePaths) -->
              {#if vault.privatePaths.has(r.path)}
                <span class="user-page" title={t("chat.privateRefHint")}><PrivacyIcon size={12} /> {t("chat.privateRef")}</span>
              {:else}
                <span class="user-page" title={r.kind === "skill" ? `/${r.path}` : r.path}
                  ><RefIcon kind={r.kind} size={12} /> {r.kind === "skill" ? skillTitleByName(r.path) : r.path}{r.lines ? ` ${linesLabel({ from_line: r.lines[0], to_line: r.lines[1] })}` : ""}{r.block ? ` ¶${r.block}` : ""}</span
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
        {@const body = item.text}
        {@const streaming = agent.running && i === agent.timeline.length - 1}
        {#if body || isTurnFinal(i)}
          <div class="answer" data-answer={i}>
          {#if item.thought}
            <!-- The reasoning that came before the answer, folded into one line above it the
                 moment the answer began: it holds its place, so nothing under it moves when it
                 folds, and it stays when the turn ends -->
            {@const thoughtOpen = !!expandedThoughts[i]}
            <button class="thought" aria-expanded={thoughtOpen} onclick={() => (expandedThoughts[i] = !thoughtOpen)}>
              {t("chat.thoughtChars", { n: fmtCount(item.thought.chars) })}
              <span class="thought-caret" class:open={thoughtOpen} aria-hidden="true">{@render caret()}</span>
            </button>
            {#if thoughtOpen}
              <pre class="thought-text">{item.thought.text}</pre>
            {/if}
          {/if}
          <div class="msg assistant md-body" {...answerLang(i, body, streaming)}>
            {#each assistantBlocks(i, body, streaming) as b, bi (bi)}{@html b}{/each}
            <!-- The action bar rests at the end of each turn's final answer (where the hand already
                 is after reading, and it covers no text): the newest turn keeps Insert in view and
                 shows the rest on hover, an older turn all of it on hover; an intermediate narration
                 bubble does not get one — select
                 its text and press a button here instead (the selection semantics are unchanged).
                 It is not shown while streaming. -->
            {#if isTurnFinal(i) && !(agent.running && i === lastAssistantIndex)}
              {@const undoable = undoableTurn(i)}
              {@const review = turnReview(i)}
              {#if review}
                <!-- The turn's writes await review: the strip stays visible (unlike the foot) until
                     they have been looked at, and its two actions run in the main window, where the
                     editor is -->
                <div class="review-strip">
                  <span class="review-text">{t("chat.turnWrote", { m: review.paths.length, n: review.blocks })}</span>
                  <button class="review-go" title={t("chat.reviewStartHint", { accept: k("Mod+Y"), next: k("Mod+Alt+↓") })} onclick={() => void hostDispatch({ kind: "review-start", paths: review.paths })}>{t("chat.reviewStart")}</button>
                  <button title={t("chat.reviewAcceptHint")} onclick={() => void hostDispatch({ kind: "review-accept", paths: review.paths })}>{t("chat.reviewAccept")}</button>
                </div>
              {/if}
              <!-- Icons, shown on hover or focus: under a finished answer six words were the
                   loudest thing in the column. Insert — the step that takes an answer back into
                   the note — keeps its word and stays on the newest answer -->
              <div class="msg-foot" class:current={i === lastAssistantIndex}>
                <button class="foot-insert" title={t("chat.insertAnswerHint")} onclick={() => void insertAnswer(body)}>
                  {@render footIcon("insert")}<span>{t("chat.insert")}</span>
                </button>
                <button class="foot-icon" title={t("chat.copyAnswerHint")} aria-label={t("chat.copy")} onclick={() => void copyAnswer(body)}>{@render footIcon("copy")}</button>
                <button
                  class="foot-icon"
                  title={clipTarget.path ? t("chat.sendToTargetPlain", { name: clipTarget.name ?? "" }) : t("chat.pinFirst")}
                  aria-label={clipTarget.path ? t("chat.sendToTargetPlain", { name: clipTarget.name ?? "" }) : t("chat.pinWord")}
                  onclick={() => void sendToPinned(body)}
                >{@render footIcon("pin")}</button>
                <button class="foot-icon" title={t("chat.saveAsNoteHint")} aria-label={t("chat.saveAsNote")} onclick={() => void saveAsNote(body, i)}>{@render footIcon("save")}</button>
                {#if i === lastAssistantIndex && !agent.running}
                  <button class="foot-icon" title={t("chat.regenerateHint")} aria-label={t("chat.regenerate")} onclick={doRetry}>{@render footIcon("regenerate")}</button>
                {/if}
                {#if undoable && !agent.running}
                  <button class="foot-icon undo" title={t("chat.undoTurnHint")} aria-label={t("chat.undoTurn")} onclick={() => void undoTurn(undoable)}>{@render footIcon("undo")}</button>
                {/if}
              </div>
            {/if}
          </div>
          </div>
        {/if}
      {:else if item.kind === "read"}
        {@const pendingRd = agent.pendingRead?.id === item.id}
        <div class="plan-inline read-card" class:rejected={item.decision?.kind === "deny"}>
          <div class="plan-inline-head">
            <span class="plan-inline-title">{t("chat.readTitle")}</span>
            {#if item.decision}
              <span class="plan-inline-badge" class:bad={item.decision.kind === "deny"}>{readBadge(item.decision)}</span>
            {:else if pendingRd}
              <span class="plan-inline-badge wait">{t("chat.readWaiting")}</span>
            {:else if item.auto || item.fetching}
              <!-- The pages are on their way: under the standing consent without a click, or after one -->
              <span class="plan-inline-badge wait"><span class="pulse"></span> {t(item.auto ? "chat.readAutoRunning" : "chat.readFetching")}</span>
            {/if}
          </div>
          {#each item.urls as u, ui (u)}
            <div class="read-row" class:off={pendingRd && !readChecks[ui]} class:failed={item.decision?.kind === "pages" && item.decision.failed.includes(u)}>
              {#if pendingRd}
                <label class="act-keep"><input type="checkbox" bind:checked={readChecks[ui]} /></label>
              {:else}
                <!-- A read page is reported under its final address (a redirect), so the requested URL
                     is not always in the decision's list: only `failed` marks a miss -->
                <span class="plan-check">{item.decision?.kind === "pages" ? (item.decision.failed.includes(u) ? "✕" : "✓") : "·"}</span>
              {/if}
              <button class="read-url" title={u} onclick={(e) => openCitation(u, e.metaKey || e.ctrlKey)}>
                <span class="read-domain">{domainOf(u)}</span><span class="read-path">{u.replace(/^https?:\/\/[^/]+/, "")}</span>
              </button>
            </div>
          {/each}
          {#if pendingRd}
            <div class="plan-actions">
              <label class="read-auto"><input type="checkbox" bind:checked={readAuto} />{t("chat.readAutoSources")}</label>
              <span class="plan-spacer"></span>
              <button class="btn subtle" onclick={declineRead}>{t("chat.readDecline")}</button>
              <button class="btn primary" onclick={() => submitRead(item)}>{readCount ? t("chat.readOpen", { n: readCount }) : t("chat.readOpenNone")}</button>
            </div>
          {/if}
        </div>
      {:else if item.kind === "actions"}
        {@const pendingActs = agent.pendingActions?.id === item.id}
        {@const shownActs = item.decision?.kind === "approve" ? item.decision.items : item.items}
        <div class="plan-inline actions-card" class:rejected={item.decision?.kind === "reject"}>
          <div class="plan-inline-head">
            <span class="plan-inline-title">{t("chat.actionsTitle")}</span>
            {#if item.decision}
              <span class="plan-inline-badge" class:bad={item.decision.kind === "reject"}>{actionsBadge(item.decision)}</span>
            {:else if pendingActs}
              <span class="plan-inline-badge wait">{t("chat.actionsWaiting")}</span>
            {/if}
          </div>
          {#if pendingActs}
            {#each actionRows as r, ri (ri)}
              {@const untargeted = r.stay && !r.target.trim()}
              {@const veiled = targetPick?.row !== ri && r.target.trim() !== ""}
              <div class="act-row" class:off={!r.keep} class:stay={r.stay}>
                <!-- A staying row cannot be ticked without a home: the tick follows the target box -->
                <label class="act-keep" title={untargeted ? t("chat.actionsStayTick") : undefined}>
                  <input type="checkbox" bind:checked={r.keep} disabled={untargeted} />
                </label>
                <div class="act-body">
                  <input class="act-text" bind:value={r.text} spellcheck="false" />
                  <div class="act-meta">
                    <input class="act-owner" bind:value={r.owner} placeholder={t("chat.actionsOwner")} spellcheck="false" />
                    <input class="act-due" type="date" bind:value={r.due} title={t("chat.actionsDue")} />
                    <!-- The box holds the path. Unfocused, a view over it shows the stem with the
                         folder dimmed after it — a long path is told apart by its stem, not by the
                         folder that repeats down the card — and the full path is the tooltip -->
                    <div class="act-target-wrap" title={veiled ? r.target.trim() : undefined}>
                      <input
                        class="act-target"
                        class:veiled
                        bind:value={r.target}
                        oninput={(e) => typeTarget(r, e.currentTarget.value)}
                        onfocus={(e) => focusTarget(ri, e.currentTarget)}
                        onblur={() => (targetPick = null)}
                        onclick={openTargetList}
                        onkeydown={targetKey}
                        placeholder={r.stay ? stayPlaceholder(r) : t("chat.actionsTarget")}
                        spellcheck="false"
                      />
                      {#if veiled}
                        {@const dir = parentOf(r.target.trim())}
                        <span class="act-target-view" aria-hidden="true"><span class="stem">{stemOf(r.target.trim())}</span>{#if dir}<span class="dir">{dir}</span>{/if}</span>
                      {/if}
                    </div>
                  </div>
                  {#if r.fields.length}
                    <!-- The extra fields (a scoring's numbers, say): each editable on the row, written afterwards as [key:: value] -->
                    <div class="act-meta act-fields">
                      {#each r.fields as f, fi (fi)}
                        <label class="act-field"><span>{f.key}</span><input bind:value={f.value} spellcheck="false" /></label>
                      {/each}
                    </div>
                  {/if}
                  {#if r.dup}
                    <div class="act-hint">{t("chat.actionsDuplicate", { path: r.dup })}</div>
                  {:else if r.stay && !r.keep}
                    <div class="act-hint">{t("chat.actionsStayHint")}</div>
                  {:else if r.source}
                    <div class="act-hint">{t("chat.actionsSource", { source: r.source })}</div>
                  {/if}
                </div>
              </div>
            {/each}
            <!-- One list for the focused box, mounted outside the rows: an unticked row's ink
                 steps back, which a list rendered inside it would inherit -->
            {#if targetPick?.open && actionRows[targetPick.row]}
              <NoteSuggest bind:this={suggest} anchor={targetPick.el} within={targetPick.within} query={actionRows[targetPick.row].target} pinned={cardTargets} onpick={pickTarget} onclose={closeTargetList} />
            {/if}
            <div class="plan-actions">
              <button class="btn subtle" onclick={submitActionsReject}>{t("chat.actionsReject")}</button>
              <span class="plan-spacer"></span>
              <button class="btn primary" onclick={submitActions}>{keptCount ? t("chat.actionsWrite", { n: keptCount }) : allStay ? t("chat.actionsStayAll") : t("chat.actionsWriteNone")}</button>
            </div>
          {:else}
            {#each shownActs as a, ai (ai)}
              <div class="plan-item">
                <span class="plan-check">{ai + 1}.</span>
                <span>{a.text}{a.owner ? ` · ${a.owner}` : ""}{a.due ? ` · ${a.due}` : ""}{a.fields ? Object.entries(a.fields).map(([k, v]) => ` · ${k} ${v}`).join("") : ""}{a.target ? ` → ${a.target}` : a.stay ? ` · ${t("chat.actionsStayShort")}` : ""}</span>
              </div>
            {/each}
          {/if}
        </div>
      {:else if item.kind === "plan"}
        <!-- It looks at pendingPlan alone and adds no running gate: in a window handed over with
             ⌘J the loop runs on the other window's channel and the local running is always false,
             while the gate really is pending.
             Every route by which the loop ends (send's finally, tool_call_finished, reset,
             reattach) clears pendingPlan, so no zombie buttons are left behind. -->
        {@const pendingPlan = agent.pendingPlan?.id === item.id}
        {@const shownItems = item.decision?.kind === "approve_edited" ? item.decision.items : item.items}
        {@const planFolded = !!item.decision && !expandedPlans[i]}
        <div class="plan-inline" class:rejected={item.decision?.kind === "reject"} class:folded={planFolded}>
          {#if item.decision}
            <!-- A decided card folds to one line, the verdict and the step count: the sticky card
                 carries the live list from here on, and the list as it was decided is a click away -->
            <button class="plan-inline-head plan-inline-toggle" title={t(planFolded ? "chat.planExpandHint" : "chat.planCollapseHint")} onclick={() => (expandedPlans[i] = planFolded)}>
              <span class="plan-caret" class:open={!planFolded} aria-hidden="true">
                {@render caret()}
              </span>
              <span class="plan-inline-title">{t("chat.planTitle")}</span>
              <span class="plan-inline-badge" class:bad={item.decision.kind === "reject"}>{planBadge(item.decision)}</span>
              {#if planFolded}<span class="plan-inline-count">· {t("chat.planSteps", { n: shownItems.length })}</span>{/if}
            </button>
          {:else}
            <div class="plan-inline-head">
              <span class="plan-inline-title">{t("chat.planTitle")}</span>
              {#if pendingPlan}
                <span class="plan-inline-badge wait">{t("chat.planWaiting")}</span>
              {/if}
            </div>
          {/if}
          {#if planFolded}
            <!-- folded: the head alone -->
          {:else if pendingPlan && planEditing}
            <!-- svelte-ignore a11y_autofocus -->
            <textarea class="plan-edit" bind:value={planEditText} spellcheck="false" autofocus></textarea>
          {:else}
            {#each shownItems as p, pi (pi)}
              <div class="plan-item"><span class="plan-check">{pi + 1}.</span>{p}</div>
            {/each}
          {/if}
          {#if pendingPlan}
            <div class="plan-actions">
              <button class="btn subtle" onclick={submitPlanReject}>{t("chat.planReject")}</button>
              <button class="btn subtle" onclick={() => togglePlanEdit(item)}>{planEditing ? t("chat.planRestore") : t("chat.planEdit")}</button>
              <span class="plan-spacer"></span>
              <button class="btn primary" onclick={() => submitPlanApprove(item)}>{t("chat.planApprove")}</button>
            </div>
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
          <span class="tool-args" title={item.args}>{argGlance(item.args)}</span>
        </button>
        {#if expandedTools[i] && item.output}
          <pre class="tool-output">{item.output}</pre>
        {/if}
      {:else if item.kind === "sources"}
        <!-- The citations lead; the search hits the model saw and did not cite stay behind a
             second toggle (a turn may see fifty and cite three). Before the settlement every
             entry is a hit, so the card reads "search hits" until the answer names its sources -->
        {@const cited = item.results.filter((r) => r.cited)}
        {@const hits = item.results.filter((r) => !r.cited)}
        {@const main = cited.length ? cited : hits}
        {@const extra = cited.length ? hits : []}
        {@const clippedCount = main.filter((r) => clips.pathFor(r.url)).length}
        {@const headText = cited.length
          ? `${t("chat.sourcesCount", { n: cited.length })}${clippedCount ? t("chat.sourcesClipped", { n: clippedCount }) : ""}`
          : hits.length
            ? t("chat.searchHits", { n: hits.length })
            : t("chat.searched")}
        {@const collapsible = !item.searching && main.length > 4}
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
                  {@render caret()}
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
            {#each main as r, n (r.url)}
              {@render sourceRow(r, n)}
            {/each}
            {#if extra.length}
              <button
                class="sources-head sources-toggle sources-more"
                onclick={() => (expandedHits[i] = !expandedHits[i])}
              >
                <span class="sources-caret" class:open={expandedHits[i]} aria-hidden="true">
                  {@render caret()}
                </span>
                {t("chat.moreHits", { n: extra.length })}
              </button>
              {#if expandedHits[i]}
                {#each extra as r, n (r.url)}
                  {@render sourceRow(r, main.length + n)}
                {/each}
              {/if}
            {/if}
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
    {#if agent.thinking && !gateWaiting}
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
        ><span>{thinkingTail}</span></div>
      {/if}
    {/if}
    </div>
  </div>
  </div>

  <!-- The dock is the panel's ground across the full width; the composer inside it is the
       timeline's column, centred with it in a wide window -->
  <div class="composer-dock" bind:clientHeight={dockH}>
  <div class="composer" class:summon={summoning}>
    <!-- The status strip: what the run is doing right now (a gate waiting, the tool in flight,
         the answer streaming), the last turn's usage when idle, and the way back to the bottom
         after scrolling up. It sits between the timeline and the input rather than floating over
         the text, and stays for the whole conversation so its coming and going never shifts the
         text being read -->
    {#if agent.timeline.length}
      <div class="status-strip">
        {#if agent.running}<span class="pulse"></span>{/if}
        <span class="status-text" use:cliptip={runStatus}>{runStatus}</span>
        {#if agent.running}
          {#if stepsLeftText}<span class="status-warn" title={t("chat.stepsLeftHint")}>{stepsLeftText}</span>{/if}
          {#if elapsedText}<span class="status-usage" title={t("chat.elapsedHint")}>{elapsedText}</span>{/if}
        {:else if usageIdleText}
          <span class="status-usage" title={usageIdleHint}>{usageIdleText}</span>
        {/if}
        {#if !atBottom}
          <!-- While a turn runs, what is below is what has arrived since following stopped -->
          <button class="to-bottom" onclick={scrollToBottom}>{t(agent.running ? "chat.newContent" : "chat.backToBottom")}</button>
        {/if}
      </div>
    {/if}
    {#if resumeSession && !agent.running}
      <div class="resume-bar">
        <span>{t("chat.resumeResearch")}</span>
        <button onclick={() => run(agent.selectSession(resumeSession))}>{t("chat.resumeButton")}</button>
      </div>
    {/if}
    <div class="box">
    <!-- The context chips (the current note / the selection / @ references) sit at the top of the
         input; with no chip at all the row takes no space. What can be seen can be dismissed with
         ✕, and what goes out is exactly this row -->
    {#if chips.length}
    <div class="ctx-row">
      {#each chips as c (c.key)}
        <!-- svelte-ignore a11y_no_static_element_interactions -->
        <span class="ctx-chip" class:cur={c.cur} class:follow={c.key === "follow"} title={c.title} oncontextmenu={c.menu}>
          <span class="ctx-chip-icon"><RefIcon kind={c.kind} size={12} /></span>
          {#if c.open}
            <button class="ctx-chip-label ctx-chip-open" onclick={c.open}>{c.label}</button>
          {:else}
            <span class="ctx-chip-label">{c.label}</span>
          {/if}
          {#if c.lines}<span class="ctx-chip-lines">{c.lines}</span>{/if}
          {#if c.tag}<span class="ctx-chip-tag">{c.tag}</span>{/if}
          <button aria-label={c.removeLabel} title={c.removeTitle} onclick={c.remove}>✕</button>
        </span>
      {/each}
    </div>
    {/if}
    {#if agent.queuedText}
      <div class="queue-chip">
        <span class="queue-label" use:cliptip={agent.queuedText}>{t("chat.queued", { text: agent.queuedText })}</span>
        <button aria-label={t("chat.cancelQueued")} onclick={() => (agent.queuedText = null)}>✕</button>
      </div>
    {/if}
    {#if studyShort}
      <div class="queue-chip balance-chip">
        <span class="queue-label" use:cliptip={studyShort}>{studyShort}</span>
        <button onclick={() => void executeCommand("settings-account")}>{t("chat.studyShortAction")}</button>
      </div>
    {/if}
    {#if atQuery != null}
      <AtPicker bind:this={picker} query={atQuery} anchor={composerEl} {curSel} onPick={onAtPick} onPickFiles={onAtPickFiles} onStage={onAtStage} onClose={closeAt} />
    {/if}
    {#if slashQuery != null}
      <SkillPicker bind:this={skillPicker} {skills} query={slashQuery} anchor={composerEl} onPick={onSlashPick} onClose={closeSlash} />
    {/if}
    <div class="input-wrap">
      {#if agent.draft === ""}
        <div class="ph" aria-hidden="true">
          {#if agent.running}
            <span>{t("chat.runningQueueHint")}</span>
          {:else}
            <span>{PH_HINTS[phIdx]}</span>
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
        onblur={() => { closeAt(); closeSlash(); hideSummonHint(); }}
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
      <!-- The model chip: the model's name and the thinking level together, next to the input
           where the request is composed, rather than a label crowding the session title out of
           the header -->
      {#if prefs.model || prefs.reasoningEffortActive}
        <button class="tool-btn model" title={modelChipHint} onclick={modelMenu}>
          <svg width="14" height="14" viewBox="0 0 20 20" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" aria-hidden="true">
            <path d="M3.5 6.5h7M14.5 6.5h2M3.5 13.5h2M9.5 13.5h7" />
            <circle cx="12.5" cy="6.5" r="1.8" />
            <circle cx="7.5" cy="13.5" r="1.8" />
          </svg>
          {#if prefs.model}<span class="model-name">{prefs.model}</span>{/if}
          {#if prefs.model && prefs.reasoningEffortActive}<span class="model-sep" aria-hidden="true">·</span>{/if}
          {#if prefs.reasoningEffortActive}<span class="model-effort">{effortLabel}</span>{/if}
          {#if searchOn}
            <!-- The globe: the search switch is on, so this message may leave the machine -->
            <span class="model-sep" aria-hidden="true">·</span>
            <span class="model-web" title={t("chat.searchOnHint")}>
              <svg width="13" height="13" viewBox="0 0 20 20" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" aria-hidden="true">
                <circle cx="10" cy="10" r="7" />
                <path d="M3 10h14M10 3c-2.5 2.3-2.5 11.7 0 14M10 3c2.5 2.3 2.5 11.7 0 14" />
              </svg>
            </span>
          {/if}
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
      {#if agent.running && !agent.draft.trim()}
        <!-- While a turn runs and nothing is typed, the send button is the stop button — always
             in reach, unlike a control at the end of the scrolling timeline; a draft turns it back
             into ↑, which queues -->
        <button class="send stop" title={t("chat.stopHint")} aria-label={t("chat.stop")} onclick={stopTurn}>
          <!-- A still ring: while a turn runs, the status strip's dot is the one thing that moves
               (a second animation here would pull the eye from the answer being read) -->
          <svg width="28" height="28" viewBox="0 0 28 28" fill="none" aria-hidden="true">
            <circle class="stop-track" cx="14" cy="14" r="13.25" stroke="currentColor" stroke-width="1.5" />
            <rect x="10.25" y="10.25" width="7.5" height="7.5" rx="1.25" fill="currentColor" />
          </svg>
        </button>
      {:else}
      <button
        class="send"
        title={modeInfo.title}
        aria-label={modeInfo.aria}
        onclick={() => submit(false)}
        disabled={!agent.draft.trim()}
      >
        {#if mode === "url"}
          <!-- A URL-shaped draft: ↵ opens the page in the browser, and the button says so -->
          <GlobeIcon size={16} strokeWidth={1.8} />
        {:else}
          <svg width="16" height="16" viewBox="0 0 20 20" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
            <path d="M10 15.5v-11M5 9.5l5-5 5 5" />
          </svg>
        {/if}
      </button>
      {/if}
    </div>
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
  /* The timeline is the page the answers are written on: the editor's ground, while the
     session bar above and the composer's dock below keep the panel's */
  .timeline {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    background: var(--bg);
  }
  /* The column itself is a child of the scroller, not the scroller (see the .turns comment in
     the markup). It is the body column the editor uses (--content-width, at the note's font
     size, so its em are the note's): in a narrow panel it is the panel, and in a wide window
     it is centred rather than running a line across the screen. The composer takes the
     same width */
  .turns {
    padding: 12px;
    display: flex;
    flex-direction: column;
    gap: 8px;
    width: 100%;
    max-width: var(--content-width);
    margin: 0 auto;
    font-size: var(--fs-note);
  }
  /* The timeline in the overlay needs some body: an empty flex container collapses into a strip
     when there is little content, and the second glance after summoning it (the streaming answer
     unfolding) needs a stable perceived height; the cap is left to the overlay container's 72vh */
  /* ---- the status strip (between the timeline and the input; see the markup) ---- */
  .status-strip {
    display: flex;
    align-items: center;
    gap: 6px;
    min-height: 18px;
    padding: 0 2px 4px;
    font-size: var(--fs-xs);
    color: var(--fg-muted);
  }
  .status-text {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .status-warn {
    flex-shrink: 0;
    color: var(--warning);
  }
  .status-usage {
    flex-shrink: 0;
    font-family: var(--font-mono);
  }
  .to-bottom {
    flex-shrink: 0;
    font-size: var(--fs-xs);
    color: var(--accent);
    padding: 0 5px;
    border-radius: 4px;
  }
  .to-bottom:hover {
    background: var(--bg-hover);
  }
  .welcome {
    padding: 24px 8px;
  }
  .welcome-hint {
    font-size: var(--fs-sm);
    color: var(--fg-muted);
    line-height: 1.6;
    margin-bottom: 6px;
  }
  /* the methods: a scene label in the left column, its chips wrapping beside it — one row a
     scene, so the eye reads "daily: start, close, report…" rather than an alphabet of names */
  .welcome-lead {
    font-size: var(--fs-sm);
    font-weight: 600;
    margin: 18px 0 8px;
  }
  .welcome-lead .faint {
    font-weight: 400;
  }
  .welcome-groups {
    display: grid;
    grid-template-columns: auto 1fr;
    column-gap: 10px;
    row-gap: 8px;
    align-items: baseline;
    margin-bottom: 18px;
  }
  .welcome-group-label {
    font-size: var(--fs-xs);
    color: var(--fg-muted);
    white-space: nowrap;
    padding-top: 5px;
  }
  .welcome-examples {
    display: flex;
    flex-wrap: wrap;
    align-items: flex-start;
    gap: 6px;
  }
  .example {
    font-size: var(--fs-xs);
    color: var(--fg);
    border: 1px solid var(--border);
    border-radius: 999px;
    padding: 3px 10px;
    text-align: left;
  }
  .example:hover {
    color: var(--accent);
    border-color: var(--accent);
    background: var(--bg-hover);
  }
  /* the method on the row: the same accent as its hover, filled — the click has visibly landed
     where the eye was, before it goes looking for the chip in the composer */
  .example.on {
    color: var(--accent);
    border-color: var(--accent);
    background: var(--accent-soft);
  }
  /* the fold of the secondary scenes: dashed and in the secondary ink, so it reads as a door, not a method */
  .example.more {
    color: var(--fg-muted);
    border-style: dashed;
  }
  .faint {
    color: var(--fg-muted);
  }
  /* The note's own size and line height (the reader's setting governs every long text); an
     answer's rhythm — paragraph gaps, the heading scale — is .md-body's */
  .msg {
    font-size: var(--fs-note);
    line-height: 1.75;
    -webkit-user-select: text;
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
  .turns > .msg.user:not(:first-child) {
    margin-top: 16px;
  }
  .user-page {
    display: block;
    margin-top: 4px;
    font-size: var(--fs-xs);
    line-height: 1.5;
    color: var(--fg-muted);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .user-page :global(svg) {
    vertical-align: -2px;
  }
  .msg.assistant {
    padding: 0 2px;
    min-width: 0;
  }
  /* An answer set in a Latin or other alphabet takes the Latin column (the column itself is
     the CJK-tuned width under a CJK interface); the readable-width switch lifts it */
  .msg.assistant[data-script="latin"],
  .msg.assistant[data-script="other"],
  .msg.assistant[data-script="rtl"] {
    max-width: var(--content-width-off, 44em);
  }
  /* An answer with the reasoning folded above it */
  .answer {
    display: flex;
    flex-direction: column;
    align-items: stretch;
    gap: 2px;
    min-width: 0;
  }
  .thought {
    align-self: flex-start;
    display: inline-flex;
    align-items: center;
    gap: 3px;
    font-size: var(--fs-xs);
    line-height: 1.5;
    color: var(--fg-muted);
    padding: 1px 4px;
    margin-left: -2px;
    border-radius: 4px;
  }
  .thought:hover {
    color: var(--fg);
    background: var(--bg-hover);
  }
  .thought-caret {
    display: flex;
    align-items: center;
    transition: transform 0.15s ease;
  }
  .thought-caret.open {
    transform: rotate(90deg);
  }
  .thought-text {
    font-family: inherit;
    font-size: var(--fs-xs);
    line-height: 1.6;
    color: var(--fg-muted);
    white-space: pre-wrap;
    max-height: 240px;
    overflow-y: auto;
    margin: 2px 0 6px;
    padding-left: 10px;
    border-left: 2px solid var(--border);
    -webkit-user-select: text;
    user-select: text;
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
  /* ◌ is a mark, not text: the faint ink clears the 3:1 a mark needs */
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
  .tool-row.wrap .tool-args {
    white-space: normal;
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
    -webkit-user-select: text;
    user-select: text;
    margin-left: 12px;
  }
  /* The reasoning still waiting for its answer. Bottom-aligned: what is clipped on overflow is
     the oldest beginning, so the newest thinking is always visible. Upright in the secondary
     ink: a slant is only a machine-sheared glyph in CJK, and it is text to be read */
  .thinking {
    display: flex;
    flex-direction: column;
    justify-content: flex-end;
    font-size: var(--fs-xs);
    line-height: 1.6;
    color: var(--fg-muted);
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
  /* The run's dot. Only the status strip's breathes: one thing moves while a turn runs, and
     the dots on a searching source card or a fetching read card hold still */
  .pulse {
    flex-shrink: 0;
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background: var(--accent);
  }
  .status-strip .pulse {
    animation: pulse 1.2s ease-in-out infinite;
  }
  @keyframes pulse {
    50% {
      opacity: 0.25;
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .status-strip .pulse {
      animation: none;
    }
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
  /* No capitals and no tracking: CJK has no case, and letter-spacing only pulls its characters apart */
  .sources-head {
    display: flex;
    align-items: center;
    gap: 6px;
    font-size: var(--fs-xs);
    color: var(--fg-muted);
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
    color: var(--fg);
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
  .sources-more {
    margin-top: 4px;
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
    color: var(--fg-muted);
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
    color: var(--fg-muted);
  }
  .source-clip {
    flex-shrink: 0;
    font-size: var(--fs-xs);
    color: var(--fg-muted);
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
    color: var(--fg-muted);
    overflow: hidden;
    display: -webkit-box;
    -webkit-box-orient: vertical;
    -webkit-line-clamp: 2;
    line-clamp: 2;
    padding: 0 6px 2px;
  }
  /* ---- the input area ---- */
  .composer-dock {
    flex-shrink: 0;
    background: var(--bg-panel);
    border-top: 1px solid var(--border);
  }
  /* The timeline's column (see .turns): the same width at the same font size, centred */
  .composer {
    position: relative; /* the @ candidate overlay anchors to its viewport rect (the padding of 12 lines up with the candidate layer) */
    padding: 4px 12px 10px;
    width: 100%;
    max-width: var(--content-width);
    margin: 0 auto;
    font-size: var(--fs-note);
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
  .resume-bar {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
    margin: 0 12px 6px;
    padding: 5px 10px;
    font-size: var(--fs-xs);
    color: var(--fg-muted);
    border: 1px dashed var(--border);
    border-radius: 6px;
  }
  .resume-bar button {
    flex-shrink: 0;
    color: var(--accent);
    font-size: var(--fs-xs);
  }
  .resume-bar button:hover {
    text-decoration: underline;
  }
  .ctx-chip.follow {
    border-color: var(--accent);
  }
  .ctx-chip-open {
    font: inherit;
    color: inherit;
    padding: 0;
    cursor: pointer;
  }
  .ctx-chip-open:hover {
    text-decoration: underline;
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
    font-size: 11px;
    color: var(--fg-muted);
    padding-left: 2px;
  }
  /* A selection's line range: it does not shrink, so however long the file name is, the name is truncated first and the (12-20) survives */
  .ctx-chip-lines {
    flex-shrink: 0;
    font-family: var(--font-mono);
    font-size: 11px;
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
     clicking the input). The secondary ink: a hint is still text to be read */
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
    color: var(--fg-muted);
    pointer-events: none;
    overflow: hidden;
    white-space: nowrap;
  }
  .ph span {
    display: block;
    overflow: hidden;
    text-overflow: ellipsis;
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
    font-size: var(--fs-xs);
    color: var(--fg-muted);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .url-hint {
    color: var(--accent);
    font-family: inherit;
  }
  .tool-btn.model {
    min-width: 0;
    flex-shrink: 1;
  }
  .model-web {
    display: inline-flex;
    align-items: center;
    flex-shrink: 0;
    color: var(--accent);
  }
  .model-name {
    min-width: 0;
    max-width: 130px;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .model-sep {
    color: var(--fg-faint);
  }
  .model-effort {
    flex-shrink: 0;
  }
  .send {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 28px;
    height: 28px;
    border-radius: 50%;
    background: var(--accent);
    color: var(--bg);
    flex-shrink: 0;
    transition: background 0.12s ease, color 0.12s ease;
  }
  /* A glyph on the accent is the ground's colour, never white: the dark theme's accent is
     light, and white on it is 2.6:1. Hover moves the accent away from that ink (towards --fg),
     so the glyph only gains contrast */
  .send:hover:not(:disabled) {
    background: color-mix(in srgb, var(--accent) 86%, var(--fg));
  }
  .send:disabled {
    background: transparent;
    color: var(--fg-faint);
    cursor: default;
  }
  .send.stop {
    background: transparent;
    color: var(--danger);
  }
  .send.stop svg {
    display: block;
  }
  .send.stop:hover {
    background: var(--danger);
    color: var(--bg);
  }
  /* Hovered, the button is a plain filled circle: the ring would only be a rim on red */
  .send.stop:hover .stop-track {
    opacity: 0;
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
  .plan-head-count {
    flex-shrink: 0;
  }
  .plan-head-step {
    flex: 1;
    min-width: 0;
    display: flex;
    align-items: baseline;
    gap: 5px;
    font-weight: 400;
    color: var(--fg);
  }
  .plan-head-text {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
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
    color: var(--fg-muted);
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
  .plan-item.doing .plan-check,
  .plan-head-step.doing .plan-check {
    color: var(--accent);
  }
  .plan-item.skipped {
    color: var(--fg-muted);
    text-decoration: line-through dotted;
    text-decoration-color: var(--fg-faint);
  }
  .act-row {
    display: flex;
    align-items: flex-start;
    gap: 6px;
    padding: 4px 0;
    border-top: 1px solid var(--border);
  }
  /* An unticked row steps back in ink (not opacity, which would fade its words below the secondary ink) */
  .act-row.off .act-body input {
    color: var(--fg-muted);
  }
  /* A staying row's target box invites a home: dashed until one is typed */
  .act-row.stay.off .act-target {
    border-style: dashed;
  }
  .act-keep {
    flex-shrink: 0;
    padding-top: 3px;
  }
  .act-body {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 3px;
  }
  .act-text,
  .act-owner,
  .act-due,
  .act-target {
    font-size: var(--fs-xs);
    padding: 2px 6px;
    border: 1px solid var(--border);
    border-radius: 4px;
    background: var(--bg);
    color: var(--fg);
    min-width: 0;
  }
  .act-text {
    width: 100%;
  }
  .act-meta {
    display: flex;
    gap: 4px;
  }
  .act-owner {
    width: 5em;
  }
  .act-due {
    width: 9.5em;
  }
  .act-target-wrap {
    position: relative;
    flex: 1;
    min-width: 0;
    display: flex;
  }
  .act-target {
    width: 100%;
  }
  /* Veiled: the box's own text is hidden and the view over it shows stem, then folder;
     the box keeps the value, the focus and the tooltip */
  .act-target.veiled {
    color: transparent;
    -webkit-text-fill-color: transparent;
  }
  .act-target-view {
    position: absolute;
    inset: 0;
    padding: 0 7px;
    display: flex;
    align-items: center;
    gap: 6px;
    min-width: 0;
    font-size: var(--fs-xs);
    white-space: nowrap;
    overflow: hidden;
    pointer-events: none;
  }
  /* The stem is what tells the notes apart, so the folder gives way first: it only takes
     what the stem leaves, and is ellipsized there */
  .act-target-view .stem {
    flex: 0 1 auto;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .act-target-view .dir {
    flex: 1 1 0;
    min-width: 0;
    color: var(--fg-muted);
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .act-hint {
    font-size: var(--fs-xs);
    color: var(--fg-muted);
  }
  .act-fields {
    flex-wrap: wrap;
  }
  .read-row {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 3px 0;
    border-top: 1px solid var(--border);
    font-size: var(--fs-xs);
  }
  .read-row.off .read-url,
  .read-row.off .read-domain {
    color: var(--fg-muted);
  }
  .read-row.failed .read-url,
  .read-row.failed .read-domain {
    text-decoration: line-through;
    color: var(--fg-muted);
  }
  .read-url {
    flex: 1;
    min-width: 0;
    display: flex;
    gap: 4px;
    text-align: left;
    overflow: hidden;
  }
  .read-domain {
    flex-shrink: 0;
    color: var(--fg);
  }
  .read-path {
    color: var(--fg-muted);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .read-auto {
    display: flex;
    align-items: center;
    gap: 4px;
    min-width: 0;
    font-size: var(--fs-xs);
    line-height: 1.3;
    color: var(--fg-muted);
  }
  .act-field {
    display: flex;
    align-items: center;
    gap: 3px;
    font-size: var(--fs-xs);
    color: var(--fg-muted);
  }
  .act-field input {
    width: 4.5em;
    font-size: var(--fs-xs);
    padding: 1px 4px;
    border: 1px solid var(--border);
    border-radius: 4px;
    background: var(--bg);
    color: var(--fg);
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
  /* A rejected card steps back in ink, not opacity */
  .plan-inline.rejected .plan-item,
  .plan-inline.rejected .read-domain,
  .plan-inline.rejected .plan-inline-title {
    color: var(--fg-muted);
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
  .plan-inline-toggle {
    width: 100%;
    text-align: left;
    flex-wrap: wrap;
    padding: 0;
    border-radius: 4px;
  }
  .plan-inline-toggle:hover {
    color: var(--fg);
  }
  .plan-inline.folded {
    padding-top: 6px;
    padding-bottom: 6px;
  }
  .plan-inline-count {
    font-weight: 400;
    color: var(--fg-muted);
  }
  .plan-inline-badge {
    font-weight: 400;
    color: var(--accent);
  }
  .plan-inline-badge.bad {
    color: var(--danger);
  }
  .plan-inline-badge.wait {
    color: var(--fg-muted);
  }
  .plan-inline-badge .pulse {
    display: inline-block;
    vertical-align: middle;
    margin-right: 2px;
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
  .plan-actions .btn {
    font-size: var(--fs-xs);
    padding: 4px 10px;
    border-radius: 6px;
    border: 1px solid var(--border);
    white-space: nowrap;
    flex-shrink: 0;
  }
  .plan-actions .btn:hover {
    background: var(--bg-hover);
  }
  .plan-actions .btn.primary {
    background: var(--accent);
    border-color: var(--accent);
    color: var(--bg);
  }
  .plan-actions .btn.primary:hover {
    background: color-mix(in srgb, var(--accent) 86%, var(--fg));
  }
  .plan-actions .btn.subtle {
    color: var(--fg-muted);
  }
  /* ---- The action bar at the end of an answer: the shortest path for getting an answer back
     into a note. It rests at the end of each turn's final answer — where the hand already is after
     reading; the newest turn keeps Insert in view, and every turn shows the rest on hover.
     Its space is constant, so hovering does not shift the layout ---- */
  /* The review strip under an answer whose writes await review: the amber of the
     editor's pending bar, and unlike the foot it does not wait to be hovered */
  .review-strip {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 6px;
    margin-top: 10px;
    padding: 4px 8px;
    border-left: 3px solid var(--prov-agent);
    background: color-mix(in srgb, var(--prov-agent) 9%, transparent);
    border-radius: 0 4px 4px 0;
    font-size: var(--fs-xs);
    color: var(--fg-muted);
    -webkit-user-select: none;
    user-select: none;
  }
  .review-strip .review-text {
    flex: 1;
    min-width: 0;
  }
  .review-strip button {
    font-size: var(--fs-xs);
    padding: 1px 8px;
    border-radius: 999px;
    border: 1px solid color-mix(in srgb, var(--prov-agent) 55%, transparent);
    color: var(--prov-agent);
    background: none;
    white-space: nowrap;
    cursor: pointer;
  }
  .review-strip button:hover {
    background: color-mix(in srgb, var(--prov-agent) 14%, transparent);
  }
  .review-strip .review-go {
    background: color-mix(in srgb, var(--prov-agent) 16%, transparent);
    border-color: var(--prov-agent);
  }
  .msg-foot {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 2px;
    margin-top: 8px;
    /* Not selectable although it sits inside the selectable bubble: its buttons are
       transparent until hovered, and a drag started on it would otherwise anchor in them */
    -webkit-user-select: none;
    user-select: none;
  }
  .msg-foot button {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    height: 24px;
    font-size: var(--fs-xs);
    color: var(--fg-muted);
    border-radius: 5px;
    white-space: nowrap;
    opacity: 0;
    transition: opacity 0.12s ease;
  }
  .msg-foot .foot-icon {
    width: 24px;
    justify-content: center;
  }
  .msg-foot .foot-insert {
    padding: 0 7px 0 5px;
  }
  /* Its space is constant, so showing the icons never shifts the text: the newest answer
     keeps Insert in view, and every answer shows the rest on hover or keyboard focus */
  .msg-foot.current .foot-insert,
  .msg.assistant:hover .msg-foot button,
  .msg-foot:focus-within button {
    opacity: 1;
  }
  .msg-foot button:hover {
    color: var(--fg);
    background: var(--bg-hover);
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
    font-size: var(--fs-xs);
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
    -webkit-user-select: none;
    user-select: none;
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
    font-size: var(--fs-xs);
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
    font-size: var(--fs-xs);
    color: var(--fg-muted);
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
    font-size: var(--fs-xs);
    color: var(--fg-muted);
  }
  .thinking-toggle:hover {
    color: var(--fg);
  }
  .thinking-full pre {
    font-family: inherit;
    font-size: var(--fs-xs);
    line-height: 1.6;
    color: var(--fg-muted);
    white-space: pre-wrap;
    max-height: 240px;
    overflow-y: auto;
    -webkit-user-select: text;
    user-select: text;
    margin: 0;
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
  .balance-chip .queue-label {
    color: var(--warning);
  }
  .balance-chip button {
    color: var(--accent);
  }
  .queue-chip button:hover {
    color: var(--fg);
  }
  .src-unverified {
    flex-shrink: 0;
    font-size: 11px;
    color: var(--warning);
    border: 1px solid currentColor;
    border-radius: 4px;
    padding: 0 4px;
  }
</style>
