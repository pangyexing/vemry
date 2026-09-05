// Agent conversation state: streams events from the Rust runner, surfaces the
// audit timeline and pending permission requests.
import { t } from "$lib/i18n/i18n.svelte";
import {
  api,
  Channel,
  type AgentEvent,
  type ContextRef,
  type EditorSelection,
  type PageContext,
  type PlanItem,
  type RefTag,
  type SourceRecord,
  type PermissionDecision,
  type PlanDecision,
  type PlanRequest,
  type SessionMeta,
  type WebSearchResult,
  type WriteRequest,
} from "$lib/api";
import { POSITIONAL_RE } from "$lib/citation";
import { domainOf, extractCitations } from "$lib/markdown";
import { clips } from "$lib/stores/clips.svelte";
import { toast } from "$lib/stores/toast.svelte";
import { CARD_TOOLS, parseSession, planReconciled } from "$lib/views/session";

/** A source entry: a WebSearchResult plus the frontend's verification marker.
 *  unverified = there was no web search at all this turn and the link appears only in
 *  the model's text (so it may be a hallucination); an already clipped one is exempted
 *  in the rendering layer. */
export type SourceEntry = WebSearchResult & { unverified?: boolean };

/** One item of the composer's context draft: a web page (whose body the frontend has
 *  already grabbed, outside the privacy domain) or a vault reference picked with @
 *  (only the path, with the body resolved by Rust inside the privacy gate). */
export type DraftRef = { kind: "page"; page: PageContext } | ContextRef;

/** The composer state that travels between the main window and the detached chat window (Rust only relays it and never parses it). */
export interface AgentHandoff {
  draft: string;
  contextDraft: DraftRef[];
  /** The message queued while a turn is running: it travels with the window — without
   *  that it would stay in the old store nobody has mounted, the far window would not
   *  see the queue, and reattaching would raise it from the dead and send it with the
   *  context of that moment. */
  queuedText: string | null;
  effortOverride: string | null;
  dismissedNote: string | null;
  dismissedSelection: string | null;
  /** A pending permission or plan gate request: it travels with the window — the gate
   *  awaits on the Rust side and the request event only ever reached the old window's
   *  store; without carrying it, the card the new window rebuilds from the jsonl would
   *  have no decision state, neither window would render a button, and the loop would
   *  stand at the gate forever. */
  pendingPermission: WriteRequest | null;
  pendingPlan: PlanRequest | null;
}

/** The composer handle a mounted conversation surface registers with the store (at most
 *  one under the single-instance invariant; unregistering only clears the one still
 *  pointing at itself). */
export interface ComposerHandle {
  /** Place the cursor alone (used at the chat window's startup). */
  focus(): void;
  /** Cursor, scrolled into view, plus one highlight; inside the panel it adds the
   *  "press ⌘J again for a detached window" hint, and hint=false only flashes (used when
   *  a window closes and hands back: the user has just come from there). */
  summon(opts?: { hint?: boolean }): void;
  /** What focus-agent decides from: with the cursor already in the input, pressing ⌘J again escalates to opening a detached window. */
  isFocused(): boolean;
  /** The "+" attachment: opens the native file dialog, and the chosen files become chips once imported into the vault (the same entry point as the command palette's). */
  attachFiles(): void;
}

export type TimelineItem =
  /** turn: the turn id (stamped live by the turn_started event, and taken from the audit
   *  line on replay; older existing turns have none) — forking locates by it, so the
   *  three layers no longer each count user ordinals. */
  | { kind: "user"; text: string; turn?: string; page?: { title: string; url: string }; refs?: RefTag[] }
  | { kind: "assistant"; text: string }
  | { kind: "tool"; id: string; name: string; args: string; ok?: boolean; output?: string }
  /** settled: Rust settled this turn's sources (a `sources` line / event) — the card is
   *  authoritative from then on and the frontend's own link harvest stays out of it. */
  | { kind: "sources"; results: SourceEntry[]; queries: string[]; searching: boolean; settled?: boolean }
  /** propose_plan's plan card: id = the tool call id; while the decision is undecided
   *  (and pendingPlan points at it) the card carries the decision buttons, and once
   *  decided it renders the decision badge. */
  | { kind: "plan"; id: string; items: string[]; decision?: PlanDecision }
  /** A system notice line (a context compaction, a fork and the like), shown quietly. */
  | { kind: "notice"; text: string }
  | { kind: "error"; text: string };

/** The data behind the panel's gauge (the usage event, present only when the provider reports it). */
export interface TurnUsage {
  prompt: number;
  completion: number;
  cacheHit: number;
  iteration: number;
  max: number;
}

/** Search backwards from `from` (the turn's start) for this turn's source card — taking
 *  a card across turns would merge this turn's results into the previous turn's card.
 *  The turn boundary rule is written here alone: both the live path (findSources) and
 *  the replay (rebuildTimeline) go through it. */
function findSourcesIn(items: TimelineItem[], from: number) {
  for (let i = items.length - 1; i >= from; i--) {
    const t = items[i];
    if (t.kind === "sources") return t;
  }
  return undefined;
}

/** A reference at send time → the tag the timeline and the audit log share (a selection keeps its line range alone and the body is dropped). */
function refTag(r: ContextRef): RefTag {
  if (r.kind === "selection") return { kind: r.kind, path: r.path, lines: [r.from_line, r.to_line] };
  if (r.kind === "block") return { kind: r.kind, path: r.path, block: blockHandle(r.local_id) };
  return { kind: r.kind, path: r.path };
}

/** A block's handle as the audit and the bubble show it: `bN` stays, anything else gets its caret. */
export function blockHandle(localId: string): string {
  const id = localId.replace(/^\^/, "");
  return POSITIONAL_RE.test(id) ? id : `^${id}`;
}

/** The index of the last user item (-1 = there is none). */
function lastUserIndex(items: TimelineItem[]): number {
  for (let i = items.length - 1; i >= 0; i--) {
    if (items[i].kind === "user") return i;
  }
  return -1;
}

/** The compaction notice's wording (shared by the live compacted event and the replay's compact line). */
function compactNotice(level: number): string {
  return t("vaultStore.compacted", {
      level: t(level === 2 ? "sessionsView.compactSummary" : "sessionsView.compactHandles"),
    });
}

class AgentStore {
  timeline = $state<TimelineItem[]>([]);
  running = $state(false);
  /** The session's plan (the latest update_plan list, or the one confirmed at the plan
   *  gate); the sticky card is its only projection. A plan whose every step is done or
   *  skipped leaves with the next question; an unfinished one stays as a reminder. */
  plan = $state<PlanItem[] | null>(null);
  /** The note the session follows (the user's "keep your findings in this note"):
   *  with auto, local writes on it pass the gate without a dialog and show as unreviewed
   *  blocks in the editor. Rust keeps the same state and the jsonl's `follow` lines
   *  rebuild it; this mirror drives the chip. */
  follow = $state<{ path: string; auto: boolean } | null>(null);
  /** The session's source table (every web citation with its verdict), the union of
   *  what Rust settled turn by turn — the sources workbench lists it. */
  sourceTable = $state<SourceRecord[]>([]);
  thinking = $state("");
  /** The most recent usage event (the gauge of turns, context size and cache hit rate). */
  usage = $state<TurnUsage | null>(null);
  /** The last turn's output was cut off by the length limit (finish_reason=length) — the UI offers "continue". */
  lastTruncated = $state(false);
  /** The current session id; null = a new session awaiting its first send (the same semantics as the Rust side). */
  sessionId = $state<string | null>(null);
  /** The list of session rows (agent_session_list), refreshed when a send finishes or after an add or delete. */
  sessions = $state<SessionMeta[]>([]);
  pendingPermission = $state<WriteRequest | null>(null);
  /** The pending plan gate request (of the same family as the permission gate): AgentChat's plan card raises its decision buttons from it. */
  pendingPlan = $state<PlanRequest | null>(null);
  /** The context draft (chips) the next message will carry: web pages plus @ references.
   *  The research panel's "add to context", the reading projection and the @ picker all
   *  hand things over here, and the caller consumes and clears it on send. There is at
   *  most one page item (a newly grabbed one replaces the old). */
  contextDraft = $state<DraftRef[]>([]);
  /** The path of the "current note" the user dismissed with ✕ on the chips row: it is no
   *  longer sent as context until another note is opened (when the chip comes back by
   *  itself) or a session is created or switched. A path is stored rather than a boolean
   *  — whether it is still that note is decided where it is read, so switching notes
   *  needs no extra listener. */
  dismissedNote = $state<string | null>(null);
  /** The identity of a dismissed "current selection" auto chip (`path:from-to`,
   *  AgentChat's selKey): that passage is no longer carried automatically, and as soon
   *  as the selection changes (a different key) the chip comes back — the same semantics
   *  and the same clearing points as dismissedNote. */
  dismissedSelection = $state<string | null>(null);
  /** The shared input draft: the research panel and the detached chat window are one
   *  composer (they are never two input boxes). Cleared with reset() on a vault switch. */
  draft = $state("");
  /** A message submitted while a turn is running is queued (one, which can be
   *  overwritten) and sent automatically by the mounted AgentChat when the turn ends;
   *  with no conversation surface mounted it is kept and sent on the next mount. */
  queuedText = $state<string | null>(null);
  /** Chat-window side: the original input held while a `#` quick memory is relayed and
   *  written (given back to the user when the main window's memo-result says ok=false;
   *  the main-window path does not come through here — runHostAction throws directly). */
  pendingMemoDraft: string | null = null;
  /** Main-window side: whether the detached AI chat window is open (set true by a
   *  successful openWindow, and false by agent-window://closed). The giving-way
   *  invariant: while the window is open the panel's agent tab shows only a "the
   *  conversation is in the detached window" placeholder — one session never has two
   *  visible composers. */
  windowOpen = $state(false);
  /** Chat-window side: a mirror of the main window's current note path (handed over once
   *  when the window opens, pushed afterwards).
   *  The chat window has no editor, so vault.currentPath is always null there. */
  hostNote = $state<string | null>(null);
  /** Chat-window side: a mirror of the main window's editor selection snapshot (handed
   *  over once when the window opens, pushed afterwards); it is where the @ candidate
   *  layer's "current selection" comes from there, while the main window reads
   *  vault.editorSelection. */
  hostSelection = $state<EditorSelection | null>(null);
  /** Incremented when a session or a vault changes: AgentChat uses it to clear the
   *  expansion state and block caches keyed by timeline index (the presentation lives in
   *  the component and the data in the store, and this is the only gear-change signal
   *  between them). */
  viewEpoch = $state(0);
  /** The composer handle the currently mounted conversation surface registered (at most one under the single-instance invariant). */
  composer: ComposerHandle | null = null;
  /** The in-session override of the thinking level (null = follow the setting's default);
   *  the composer's quick switch writes it and retry carries it too — a regenerate keeps
   *  the override; creating or switching a session falls back to the global default. */
  effortOverride = $state<string | null>(null);
  /**
   * Discrete audit events this session (tool calls, turns, permissions — not
   * per-token deltas). SessionsView watches it to refresh the replay live
   * mid-run; Rust appends the session log line before the event reaches us.
   */
  activity = $state(0);
  /** Bumped by reset(); events from a turn started before a vault switch go quiet. */
  private epoch = 0;
  /** The live path's start of this turn (the user item's index). Finding and filling the
   *  source card are both bounded by it — without a boundary, a following turn with no
   *  search would pour its text links into the previous turn's source card, diverging
   *  from the replay path (rebuildTimeline's turnStart). */
  private liveTurnStart = 0;
  /** The merge buffer for text_delta and thinking: updating $state per token would
   *  re-render the whole message (and the 8k thinking window) per token (a Markdown
   *  reparse plus a whole DOM swap, O(n²)), so 40ms are accumulated before landing in
   *  the timeline. Any non-delta event flushes first, which keeps the relative order
   *  against the tool rows and the ending intact. */
  private pendingDelta = "";
  private pendingThinking = "";
  private flushTimer: ReturnType<typeof setTimeout> | null = null;

  /** The single-page semantics of the web context: it replaces the page item already in
   *  the draft (a rule shared by all three producers — the page card, the reading
   *  projection and the @ picker). */
  attachPage(page: PageContext) {
    this.contextDraft = [...this.contextDraft.filter((d) => d.kind !== "page"), { kind: "page", page }];
  }

  /** Extraction in progress (read by both the page card's button and the @ candidate layer: one extraction should not run twice concurrently). */
  grabbingPage = $state(false);

  /** The current browsed page → a PageContext. A failed extraction toasts here and the
   *  caller only deals with the page it got (null = failed, or one is already in flight)
   *  — the two producers no longer each write their own gate and error handling. */
  async grabPage(): Promise<PageContext | null> {
    if (this.grabbingPage) return null;
    this.grabbingPage = true;
    try {
      return await api.browserPageContext();
    } catch (e) {
      toast.push(String(e), "error");
      return null;
    } finally {
      this.grabbingPage = false;
    }
  }

  // ---- the detached chat window: handoff and handback (two webviews, two stores, with the source of truth in Rust's session files) ----

  /** The composer state that travels with the window when it opens or closes. The shape lives here and in applyHandoff and nowhere else. */
  handoff(): AgentHandoff {
    return {
      draft: this.draft,
      contextDraft: $state.snapshot(this.contextDraft) as DraftRef[],
      queuedText: this.queuedText,
      effortOverride: this.effortOverride,
      dismissedNote: this.dismissedNote,
      dismissedSelection: this.dismissedSelection,
      pendingPermission: $state.snapshot(this.pendingPermission) as WriteRequest | null,
      pendingPlan: $state.snapshot(this.pendingPlan) as PlanRequest | null,
    };
  }

  applyHandoff(raw: unknown) {
    if (!raw || typeof raw !== "object") return;
    const h = raw as Partial<AgentHandoff>;
    if (typeof h.draft === "string") this.draft = h.draft;
    if (Array.isArray(h.contextDraft)) this.contextDraft = h.contextDraft;
    this.queuedText = typeof h.queuedText === "string" && h.queuedText ? h.queuedText : null;
    this.effortOverride = typeof h.effortOverride === "string" ? h.effortOverride : null;
    this.dismissedNote = typeof h.dismissedNote === "string" ? h.dismissedNote : null;
    this.dismissedSelection = typeof h.dismissedSelection === "string" ? h.dismissedSelection : null;
    this.pendingPermission = typeof h.pendingPermission === "object" ? (h.pendingPermission ?? null) : null;
    this.pendingPlan = typeof h.pendingPlan === "object" ? (h.pendingPlan ?? null) : null;
  }

  /** Rebuild the timeline from Rust's current session (without switching the model
   *  context): shared by the chat window's startup and the main window taking over —
   *  whatever happened in the other window is read back here from the jsonl. */
  async loadCurrent() {
    try {
      const id = await api.agentSessionCurrent();
      if (id) {
        this.timeline = this.rebuildTimeline(await api.sessionRead(id));
        this.sessionId = id;
      } else {
        this.timeline = [];
        this.sessionId = null;
      }
    } catch {
      this.timeline = [];
      this.sessionId = null;
    }
    this.thinking = "";
    this.usage = null;
    this.lastTruncated = false;
    this.viewEpoch++;
    await this.refreshSessions();
  }

  /** Main window: open (or focus) the detached chat window, with the composer state
   *  travelling with it; the current note and the editor selection go over in the handoff
   *  package (and are pushed by App's effect afterwards). */
  async openWindow(currentNote: string | null, selection: EditorSelection | null) {
    const created = await api.agentWindowOpen(this.handoff(), currentNote, selection);
    this.windowOpen = true;
    if (created) {
      // The handoff package was delivered: the composer state travels with the window,
      // and a pending gate request travels with it too — keeping the main window's copy
      // would raise the decision UI in both windows, and whichever answered second would
      // certainly report "no pending request"
      this.draft = "";
      this.contextDraft = [];
      this.queuedText = null;
      this.pendingPermission = null;
      this.pendingPlan = null;
    }
    // created=false: the window was already open and was only focused, so the handoff
    // was not delivered — the draft and the references stay in the main window's store
    // and are aligned with the handback when it comes back
  }

  /** Main window: the chat window closed — take the composer state handed back and rebuild the timeline from Rust's current session. */
  async reattach() {
    if (!this.windowOpen) return; // the vault-switch path already reset: a late closed event must not read the old vault's session back
    this.windowOpen = false;
    try {
      // Should a turn still be finishing when the chat window closes (cancel only sets a
      // flag), wait for Rust to finish writing the session jsonl before reading, or the
      // timeline would be missing its tail (capped at 5s; a failed query takes over anyway)
      for (let i = 0; i < 50 && (await api.agentIsRunning()); i++) {
        await new Promise((r) => setTimeout(r, 100));
      }
    } catch {
      /* the query failed: take over anyway */
    }
    try {
      const hb = await api.agentWindowTakeHandback();
      if (hb) this.applyHandoff(hb);
    } catch {
      /* there is no handback package (a forced close, or discarded on a vault switch) */
    }
    // The take-over direction keeps no gate request: the close handshake cancelled the
    // running turn first (so the gate was already refused by the fallback), and a pending
    // item in the handback is the after-image of that cancellation racing the packaging —
    // answering it would only error
    this.pendingPermission = null;
    this.pendingPlan = null;
    await this.loadCurrent();
  }

  /** Follow a note (null stops). Rust records it (and its `follow` line); the chip follows this mirror. */
  async setFollow(path: string | null, auto: boolean = this.follow?.auto ?? true) {
    await api.agentFollowSet(path, auto);
    this.follow = path ? { path, auto } : null;
    this.activity++;
  }

  /** A turn is being abandoned (a regenerate, an edit-and-resend): its sources leave the table, as Rust drops them. */
  private dropTurnSources(turn: string | undefined) {
    if (!turn) return;
    this.sourceTable = this.sourceTable.filter((s) => s.turn !== turn);
  }

  async send(message: string, currentNote: string | null, draft: DraftRef[] = []) {
    if (this.running || !message.trim()) return;
    const page = draft.find((d): d is Extract<DraftRef, { kind: "page" }> => d.kind === "page")?.page ?? null;
    // A block's label is presentation only and stays out of the request
    const refs: ContextRef[] = draft.flatMap((d): ContextRef[] =>
      d.kind === "page" ? [] : d.kind === "block" ? [{ kind: "block", path: d.path, local_id: d.local_id }] : [d],
    );
    this.liveTurnStart = this.timeline.length;
    this.timeline.push({
      kind: "user",
      text: message,
      page: page ? { title: page.title, url: page.url } : undefined,
      // The timeline keeps the tags alone (the same shape as the audit replay): a selection's body does not enter the presentation layer
      refs: refs.length ? refs.map(refTag) : undefined,
    });
    this.activity++; // the new session's file exists by now, so the replay list sees it as soon as possible
    await this.runTurn((channel) => api.agentStart(message, currentNote, page, refs, this.effortOverride, channel));
  }

  /** Regenerate the last turn: the UI truncates to the last user message, and Rust cuts
   *  the model history's tail and replays the generation from the original envelope (web
   *  context included). */
  async retry() {
    if (this.running) return;
    const idx = lastUserIndex(this.timeline);
    if (idx < 0) return;
    // Keep a copy before truncating: agent_retry can fail on the spot (an old session
    // with no model-history sidecar, an empty API key), and when it fails with not one
    // character regenerated the old answer is put back verbatim — one failure must not
    // eat an answer already on screen.
    const removed = this.timeline.splice(idx + 1);
    const abandoned = this.timeline[idx];
    if (abandoned?.kind === "user") this.dropTurnSources(abandoned.turn);
    this.liveTurnStart = idx;
    this.activity++;
    const epoch = this.epoch;
    const ok = await this.runTurn((channel) => api.agentRetry(this.effortOverride, channel));
    if (!ok && epoch === this.epoch) {
      const tail = this.timeline.slice(idx + 1);
      if (tail.every((t) => t.kind === "error")) {
        this.timeline.splice(idx + 1, 0, ...removed);
      }
    }
  }

  /** The first half of an edit and resend: the whole turn (the user message included) is
   *  taken out of the UI and the model history, and the original text and @ references
   *  are returned for the caller to put back into the input and the chips (the bodies of
   *  the web context and the selection are not in the timeline and cannot be recovered,
   *  so they are not returned — the selection is probably still in the editor and can be
   *  @-ed again).
   *  The audit jsonl keeps the original lines and appends an edit_resend marker. */
  async editResend(): Promise<{ text: string; refs?: ContextRef[] } | null> {
    if (this.running) return null;
    const idx = lastUserIndex(this.timeline);
    if (idx < 0) return null;
    const item = this.timeline[idx];
    if (item.kind !== "user") return null;
    const refs = item.refs?.flatMap((r): ContextRef[] =>
      r.kind === "selection"
        ? []
        : r.kind === "block"
          ? [{ kind: "block", path: r.path, local_id: (r.block ?? "").replace(/^\^/, "") }]
          : [{ kind: r.kind, path: r.path }],
    );
    const restored = { text: item.text, refs: refs?.length ? refs : undefined };
    await api.agentHistoryTruncate();
    this.dropTurnSources(item.turn);
    this.timeline.splice(idx);
    this.activity++;
    return restored;
  }

  /** Fork a new session (the research tree) before timeline item atIndex (a user item)
   *  and switch to it.
   *  The backend locates by that item's turn id (so neither a compaction's synthesised
   *  message nor an edit-resend's leftover line can cut in the wrong place); an existing
   *  old turn with no id falls back to ordinals. When the fork point precedes a
   *  second-level compaction, what the new session carries is a synthesised summary
   *  covering more — and the notice line says so plainly. */
  async fork(atIndex: number): Promise<void> {
    if (this.running) return;
    const item = this.timeline[atIndex];
    if (item?.kind !== "user") return;
    let ordinal = 0;
    for (let i = 0; i < atIndex; i++) {
      if (this.timeline[i].kind === "user") ordinal++;
    }
    const { id, summarized_context } = await api.agentSessionFork(item.turn ?? null, ordinal);
    this.timeline.splice(atIndex);
    this.timeline.push({
      kind: "notice",
      text: t(summarized_context ? "vaultStore.forkedHereCompacted" : "vaultStore.forkedHere"),
    });
    this.sessionId = id;
    this.activity++;
    await this.refreshSessions();
  }

  /** One turn's channel and ending (shared by send and retry): the epoch guard, errors
   *  landing in the timeline, resetting running/thinking/the permission state, and
   *  refreshing the session list. Returns whether the turn ended normally (false =
   *  start() threw), from which retry decides whether to put the old answer back. */
  private async runTurn(start: (channel: Channel<AgentEvent>) => Promise<string>): Promise<boolean> {
    const epoch = this.epoch;
    this.running = true;
    this.thinking = "";
    this.lastTruncated = false;
    const channel = new Channel<AgentEvent>();
    channel.onmessage = (e) => {
      if (epoch === this.epoch) this.handle(e);
    };
    try {
      const sid = await start(channel);
      if (epoch === this.epoch) this.sessionId = sid;
      return true;
    } catch (err) {
      const msg = String(err);
      // runner already emitted a matching error event for most failures
      if (epoch === this.epoch && !this.timeline.some((t) => t.kind === "error" && t.text === msg)) {
        this.flushDelta(); // the text buffer lands first, so the error line does not slip in front of it
        this.timeline.push({ kind: "error", text: msg });
      }
      return false;
    } finally {
      if (epoch === this.epoch) this.flushDelta();
      this.running = false;
      this.thinking = "";
      this.pendingPermission = null;
      this.pendingPlan = null;
      this.activity++;
      void this.refreshSessions();
    }
  }

  /** Vault switched: Rust restarts the new vault with fresh history — drop the UI's too. */
  reset() {
    this.epoch++;
    this.pendingDelta = ""; // leftovers of an old turn must not land in a new timeline
    this.pendingThinking = "";
    if (this.flushTimer != null) {
      clearTimeout(this.flushTimer);
      this.flushTimer = null;
    }
    this.timeline = [];
    this.plan = null;
    this.follow = null;
    this.sourceTable = [];
    this.thinking = "";
    this.pendingPermission = null;
    this.pendingPlan = null;
    this.sessionId = null;
    this.sessions = [];
    this.usage = null;
    this.lastTruncated = false;
    this.draft = "";
    this.queuedText = null;
    this.contextDraft = []; // the old vault's paths do not cross into another vault
    this.resetSessionScoped();
    void this.refreshSessions();
  }

  /** Resetting the session-scoped state: shared by a vault switch, a create and a switch.
   *  `loadCurrent()` only bumps viewEpoch and does not come through here — reattach has
   *  just applied the handback, which must not be wiped out again. */
  private resetSessionScoped() {
    this.dismissedNote = null;
    this.dismissedSelection = null;
    this.effortOverride = null;
    this.viewEpoch++;
  }

  // ---- session management (v0.5.1): the UI timeline, Rust's model context and the audit jsonl change gear together ----

  async refreshSessions() {
    try {
      this.sessions = await api.agentSessionList();
    } catch {
      this.sessions = []; // no vault open
    }
  }

  /** A new session: clear Rust's in-memory context and the UI timeline; the id is allocated on the first send. */
  async newSession() {
    if (this.running) return;
    await api.agentSessionNew();
    this.timeline = [];
    this.plan = null;
    this.follow = null;
    this.sourceTable = [];
    this.thinking = "";
    this.sessionId = null;
    this.resetSessionScoped();
  }

  /** Switch sessions: Rust loads the sidecar context and the frontend rebuilds the timeline from the jsonl. */
  async selectSession(id: string) {
    if (this.running || id === this.sessionId) return;
    const jsonl = await api.agentSessionSelect(id);
    this.timeline = this.rebuildTimeline(jsonl);
    this.sessionId = id;
    this.resetSessionScoped();
  }

  /** Rename a session: a title override line is appended to the jsonl without touching
   *  the model context — so it is allowed while running.
   *  activity lets the replay view's debounced refresh see the new title. */
  async renameSession(id: string, title: string) {
    await api.agentSessionRename(id, title);
    this.activity++;
    await this.refreshSessions();
  }

  /** Delete a session (the audit record goes with it, so the caller confirms first);
   *  deleting the current one returns to a new session.
   *  It throws while running rather than returning silently: the replay view's
   *  right-click delete has no disabled gate like the panel's dropdown, and silently
   *  not deleting after a confirmation is worse than an error. */
  async deleteSession(id: string) {
    if (this.running) throw new Error(t("vaultStore.cannotDeleteWhileRunning"));
    await api.agentSessionDelete(id);
    if (id === this.sessionId) {
      this.timeline = [];
      this.plan = null;
      this.follow = null;
      this.sourceTable = [];
      this.sessionId = null;
    }
    await this.refreshSessions();
  }

  /** The audit jsonl → the timeline. A user line's page field restores the 📄 chip
   *  (content is the user's own text, and the envelope only ever enters the model
   *  history); tool_call and tool_result are paired in order; a web_search line
   *  reassembles the source card and fills in the text's citations at the end of each
   *  turn (the same logic as the live path's harvestLinksFromAnswer).
   *  write_prepared/permission/privacy are left to the replay view and are not shown in
   *  the timeline. */
  private rebuildTimeline(jsonl: string): TimelineItem[] {
    const items: TimelineItem[] = [];
    let plan: PlanItem[] | null = null;
    let follow: { path: string; auto: boolean } | null = null;
    this.sourceTable = [];
    let turnStart = 0;
    const finishTurn = () => {
      this.harvestTurn(items, turnStart);
      turnStart = items.length;
    };
    let replaySeq = 0;
    for (const e of parseSession(jsonl)) {
      switch (e.kind) {
        case "user": {
          finishTurn();
          if (plan && planReconciled(plan)) plan = null; // the same rule as the live turn_started
          items.push({
            kind: "user",
            text: e.content,
            turn: e.turn,
            page: e.page ? { title: e.page.title, url: e.page.url } : undefined,
            refs: e.refs?.length ? e.refs : undefined,
          });
          break;
        }
        case "assistant": {
          if (e.content) items.push({ kind: "assistant", text: e.content });
          break;
        }
        case "tool_call": {
          // A tool that carries its own card projection pushes no raw tool row (the same verdict as the live path)
          if (CARD_TOOLS.has(e.tool)) break;
          items.push({
            kind: "tool",
            id: `replay-${replaySeq++}`,
            name: e.tool,
            args: typeof e.args === "string" ? e.args : JSON.stringify(e.args),
          });
          break;
        }
        case "tool_result": {
          // Search backwards for a tool row of the same name that has not been filled in: in a sequential log that is the line above, and a batch of calls still pairs correctly
          for (let i = items.length - 1; i >= turnStart; i--) {
            const item = items[i];
            if (item.kind === "tool" && item.name === e.tool && item.ok === undefined) {
              item.ok = e.ok;
              item.output = e.output;
              break;
            }
          }
          break;
        }
        case "web_search": {
          // Two payloads: {queries} and {count, links}; one turn shares one source card
          let card = findSourcesIn(items, turnStart);
          if (!card) {
            card = { kind: "sources", results: [], queries: [], searching: false };
            items.push(card);
          }
          const queries = card.queries;
          if (e.queries) queries.push(...e.queries.filter((q) => !queries.includes(q)));
          if (e.links) {
            const known = new Set(card.results.map((r) => r.url));
            for (const l of e.links) {
              if (known.has(l.url)) continue;
              card.results.push({ title: l.title, url: l.url, snippet: "", publish_date: null });
            }
          }
          break;
        }
        case "retry": {
          // Regenerate: the abandoned assistant/tool tail is no longer shown (the audit lines remain)
          const idx = lastUserIndex(items);
          const abandoned = items[idx];
          if (abandoned?.kind === "user") this.dropTurnSources(abandoned.turn);
          if (idx >= 0) items.splice(idx + 1);
          turnStart = Math.min(turnStart, items.length);
          break;
        }
        case "edit_resend": {
          // Edit and resend: the whole turn (the user message included) is taken out of the display, and the new question follows
          const idx = lastUserIndex(items);
          const abandoned = items[idx];
          if (abandoned?.kind === "user") this.dropTurnSources(abandoned.turn);
          if (idx >= 0) items.splice(idx);
          turnStart = Math.min(turnStart, items.length);
          break;
        }
        case "plan": {
          items.push({ kind: "plan", id: e.id, items: e.items });
          break;
        }
        case "plan_update": {
          plan = e.items;
          break;
        }
        case "follow": {
          follow = e.path ? { path: e.path, auto: e.auto ?? true } : null;
          break;
        }
        case "sources": {
          this.settleSourcesInto(items, turnStart, e.items);
          break;
        }
        case "plan_decision": {
          // Search backwards for the plan card with the same id to fill in the decision (the same as filling in a tool_result)
          for (let i = items.length - 1; i >= turnStart; i--) {
            const item = items[i];
            if (item.kind === "plan" && item.id === e.id && item.decision === undefined) {
              item.decision =
                e.decision === "approve"
                  ? { kind: "approve" }
                  : e.decision === "approve_edited"
                    ? { kind: "approve_edited", items: e.items ?? [] }
                    : { kind: "reject", reason: e.reason ?? "" };
              break;
            }
          }
          break;
        }
        case "compact": {
          items.push({ kind: "notice", text: compactNotice(e.level) });
          break;
        }
        case "fork": {
          items.push({ kind: "notice", text: t("vaultStore.forkedFromAnother") });
          break;
        }
        case "title":
          break; // a rename's title override line: pure metadata, not rendered in the timeline
        default:
          break; // write_prepared / permission / privacy / raw → the replay view
      }
    }
    finishTurn();
    this.plan = plan;
    this.follow = follow;
    const urls = items.flatMap((t) => (t.kind === "sources" ? t.results.map((r) => r.url) : []));
    if (urls.length) void clips.lookup(urls);
    return items;
  }

  /** Rust's settlement of a turn's sources (the `sources` line on replay, the
   *  sources_updated event live): the turn's card takes these entries with their
   *  verification and becomes authoritative — the frontend's own harvest of the
   *  answer's links no longer touches it. An entry already on the card (a structured
   *  search result) keeps its snippet and gains the verdict. */
  private settleSourcesInto(items: TimelineItem[], from: number, records: SourceRecord[]) {
    let card = findSourcesIn(items, from);
    if (!card) {
      card = { kind: "sources", results: [], queries: [], searching: false };
      items.push(card);
    }
    for (const r of records) {
      const known = this.sourceTable.find((x) => x.url === r.url);
      if (known) {
        if (r.verified) known.verified = true;
        if (!known.title && r.title) known.title = r.title;
      } else {
        this.sourceTable.push({ ...r });
      }
      const prev = card.results.find((x) => x.url === r.url);
      if (prev) {
        prev.unverified = r.verified ? undefined : true;
        if (!prev.title && r.title) prev.title = r.title;
        continue;
      }
      const entry: SourceEntry = { title: r.title || domainOf(r.url), url: r.url, snippet: "", publish_date: null };
      if (!r.verified) entry.unverified = true;
      card.results.push(entry);
    }
    card.searching = false;
    card.settled = true;
  }

  /** Land the accumulated thinking/text_delta into $state (an empty buffer is a no-op). */
  private flushDelta() {
    if (this.flushTimer != null) {
      clearTimeout(this.flushTimer);
      this.flushTimer = null;
    }
    if (this.pendingThinking) {
      // Keep a sizeable window to expand, read and copy; the 8k cap stops one turn's thinking from growing without bound
      this.thinking = (this.thinking + this.pendingThinking).slice(-8000);
      this.pendingThinking = "";
    }
    if (!this.pendingDelta) return;
    const text = this.pendingDelta;
    this.pendingDelta = "";
    const last = this.timeline[this.timeline.length - 1];
    if (last?.kind === "assistant") {
      last.text += text;
    } else {
      this.timeline.push({ kind: "assistant", text });
    }
  }

  private handle(e: AgentEvent) {
    // thinking and text_delta arrive per token and do not count as discrete events, or the refresh debounce would be meaningless
    if (e.type !== "thinking" && e.type !== "text_delta") {
      this.flushDelta(); // the buffered text lands first, so the timeline's order matches the event stream's
      this.activity++;
    }
    switch (e.type) {
      case "turn_started": {
        // The user item send() just pushed gets its turn id here (the same one Rust stamps
        // on the audit line and the model history) — the precondition for forking by id
        const idx = lastUserIndex(this.timeline);
        const item = this.timeline[idx];
        if (item?.kind === "user") item.turn = e.turn;
        // A finished plan belongs to the task that is over; an unfinished one stays
        if (this.plan && planReconciled(this.plan)) this.plan = null;
        break;
      }
      case "plan_updated": {
        this.plan = e.items;
        break;
      }
      case "sources_updated": {
        this.settleSourcesInto(this.timeline, this.liveTurnStart, e.items);
        void clips.lookup(e.items.map((r) => r.url));
        break;
      }
      case "thinking": {
        this.pendingThinking += e.text;
        if (this.flushTimer == null) {
          this.flushTimer = setTimeout(() => this.flushDelta(), 40);
        }
        break;
      }
      case "text_delta": {
        this.pendingDelta += e.text;
        if (this.flushTimer == null) {
          this.flushTimer = setTimeout(() => this.flushDelta(), 40);
        }
        break;
      }
      case "tool_call_started": {
        // A tool that carries its own card projection pushes no raw JSON tool row — the plan card is propose_plan's only projection
        if (CARD_TOOLS.has(e.name)) break;
        this.timeline.push({ kind: "tool", id: e.id, name: e.name, args: e.args });
        break;
      }
      case "tool_call_finished": {
        const item = [...this.timeline].reverse().find((t) => t.kind === "tool" && t.id === e.id);
        if (item && item.kind === "tool") {
          item.ok = e.ok;
          item.output = e.output;
        }
        this.pendingPermission = null;
        this.pendingPlan = null; // defensive: it is already null once the user has answered
        break;
      }
      case "permission_request": {
        this.pendingPermission = e.request;
        break;
      }
      case "plan_request": {
        this.timeline.push({ kind: "plan", id: e.request.id, items: e.request.items });
        this.pendingPlan = e.request;
        break;
      }
      case "web_searching": {
        // Several searches in one turn share one source card (the same semantics as the
        // replay path's rebuildTimeline, and the end-of-turn fill-in also only honours
        // this turn's last card) — an existing card lights up its pulse, and only without
        // one is a new card opened
        this.lastSources().searching = true;
        break;
      }
      case "web_search_queries": {
        const item = this.lastSources();
        item.queries.push(...e.queries.filter((q) => !item.queries.includes(q)));
        item.searching = false;
        break;
      }
      case "web_search_results": {
        // Structured results (DeepSeek currently returns none; kept defensively); several batches in one turn are merged and deduplicated
        const item = this.lastSources();
        const known = new Set(item.results.map((r) => r.url));
        item.results.push(...e.results.filter((r) => !known.has(r.url)));
        item.searching = false;
        break;
      }
      case "usage": {
        this.usage = {
          prompt: e.prompt_tokens,
          completion: e.completion_tokens,
          cacheHit: e.cache_hit_tokens,
          iteration: e.iteration,
          max: e.max_iterations,
        };
        break;
      }
      case "compacted": {
        this.timeline.push({ kind: "notice", text: compactNotice(e.level) });
        break;
      }
      case "turn_finished": {
        // DeepSeek returns no structured citations, and the sources live in the Markdown
        // links of its text — so they are extracted at the end and filled into the source
        // card, where a click opens the built-in browser
        this.harvestLinksFromAnswer();
        this.settleSources();
        this.thinking = "";
        this.lastTruncated = e.truncated;
        break;
      }
      case "vault_changed": {
        // The app broadcasts these as vault events (the store's handleEvent reloads the
        // open note and moves the index version); nothing to do on the agent side
        break;
      }
      case "error": {
        // A turn that errored or was cancelled will get no more search results, so the source card must not spin on "searching the web…" forever
        this.settleSources();
        this.timeline.push({ kind: "error", text: e.message });
        break;
      }
    }
  }

  /** This turn's source card (bounded by liveTurnStart; the rule lives in findSourcesIn). */
  private findSources(): Extract<TimelineItem, { kind: "sources" }> | undefined {
    return findSourcesIn(this.timeline, this.liveTurnStart);
  }

  /** This turn's source card, created when there is none (the fallback for out-of-order or missing events). */
  private lastSources(): Extract<TimelineItem, { kind: "sources" }> {
    const found = this.findSources();
    if (found) return found;
    const fresh: TimelineItem = { kind: "sources", results: [], queries: [], searching: false };
    this.timeline.push(fresh);
    return fresh as Extract<TimelineItem, { kind: "sources" }>;
  }

  /** The end-of-turn fallback: when this turn's source card is still marked searching (an error, a cancellation, an empty run), put it down. */
  private settleSources() {
    const sources = this.findSources();
    if (sources) sources.searching = false;
  }

  /** Merge the text's citation links into the source card. It shares one extraction with
   *  the text's markers through extractCitations: the number is the index plus 1, so the
   *  two always agree. The live ending and a session rebuild use the same logic.
   *  unverified: there was no web search at all this turn, so a link in the text may be
   *  something the model produced from memory — unless it is in the exemption set (this
   *  turn's web context) it is marked "unverified"; an already clipped one is exempted in
   *  the rendering layer. */
  private harvestInto(
    sources: Extract<TimelineItem, { kind: "sources" }>,
    text: string,
    unverified = false,
    exempt?: Set<string>,
  ) {
    const known = new Set(sources.results.map((r) => r.url));
    for (const c of extractCitations(text)) {
      if (known.has(c.url)) continue;
      known.add(c.url);
      const entry: SourceEntry = { title: c.title || domainOf(c.url), url: c.url, snippet: "", publish_date: null };
      if (unverified && !exempt?.has(c.url)) entry.unverified = true;
      sources.results.push(entry);
    }
    sources.searching = false;
  }

  /** The end-of-turn fill-in: the citations in every assistant bubble of this turn (from
   *  `from` onwards) are merged into this turn's source card — a tool call or a second
   *  search splits the text into several bubbles, and filling in only the last one would
   *  lose sources.
   *  With no search this turn but links in the text, a card is raised all the same and
   *  each entry is marked "unverified" (the exemption being this turn's web context) — a
   *  URL given from memory is the hallucination risk that most deserves to be seen. Both
   *  the live path (harvestLinksFromAnswer) and the replay (rebuildTimeline's finishTurn)
   *  come through here, so the unverified and exemption rules are written once. Returns
   *  this turn's source card (undefined when there is no card to raise). */
  private harvestTurn(items: TimelineItem[], from: number) {
    let sources = findSourcesIn(items, from);
    // Rust settled this turn (the normal path since the source table moved into the
    // session): the card is authoritative, and the fallback harvest below is only for
    // a turn that ended without a settlement (an error, an older session's replay)
    if (sources?.settled) {
      sources.searching = false;
      return sources;
    }
    const searched = !!sources; // until now a card could only have been created by a search event
    const pageUrls = new Set<string>();
    const texts: string[] = [];
    for (let i = from; i < items.length; i++) {
      const t = items[i];
      if (t.kind === "user" && t.page) pageUrls.add(t.page.url);
      if (t.kind === "assistant") texts.push(t.text);
    }
    if (!sources) {
      if (!texts.some((t) => extractCitations(t).length)) return undefined;
      const fresh: Extract<TimelineItem, { kind: "sources" }> = {
        kind: "sources",
        results: [],
        queries: [],
        searching: false,
      };
      items.push(fresh);
      sources = fresh;
    }
    for (const t of texts) this.harvestInto(sources, t, !searched, pageUrls);
    return sources;
  }

  /** The live ending: fill in this turn's sources and look up which of them have been clipped (the citation-landed badge). */
  private harvestLinksFromAnswer() {
    const sources = this.harvestTurn(this.timeline, this.liveTurnStart);
    if (sources) void clips.lookup(sources.results.map((r) => r.url));
  }

  async respond(decision: PermissionDecision) {
    const req = this.pendingPermission;
    if (!req) return;
    this.pendingPermission = null;
    try {
      await api.agentPermissionRespond(req.id, decision);
      this.activity++; // a permission decision enters the session log too
    } catch (err) {
      this.timeline.push({ kind: "error", text: String(err) });
    }
  }

  /** The plan gate's decision (the same shape as respond): the plan card's decision is filled in optimistically, and the loop continues within the same turn. */
  async respondPlan(decision: PlanDecision) {
    const req = this.pendingPlan;
    if (!req) return;
    this.pendingPlan = null;
    const card = [...this.timeline].reverse().find((t) => t.kind === "plan" && t.id === req.id);
    if (card && card.kind === "plan") card.decision = decision;
    try {
      await api.agentPlanRespond(req.id, decision);
      this.activity++; // a plan decision enters the session log too
    } catch (err) {
      this.timeline.push({ kind: "error", text: String(err) });
    }
  }

  async cancel() {
    try {
      await api.agentCancel();
    } catch {
      /* not running */
    }
  }
}

export const agent = new AgentStore();
