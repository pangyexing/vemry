// Vault state (Svelte 5 runes). One instance for the whole app.
import { t } from "$lib/i18n/i18n.svelte";
import { tick, untrack } from "svelte";
import { confirm } from "@tauri-apps/plugin-dialog";
import type { UnlistenFn } from "@tauri-apps/api/event";
import { api, onVaultEvent, type EditorSelection, type NoteSummary, type Origin, type VaultEvent } from "$lib/api";
import { agent } from "$lib/stores/agent.svelte";
import { clips } from "$lib/stores/clips.svelte";
import { toast } from "$lib/stores/toast.svelte";
import { POSITIONAL_RE, blocksOf, findBlock } from "$lib/citation";
import { types } from "$lib/stores/types.svelte";
import { k } from "$lib/shortcut";
import { byteToChar, ymd } from "$lib/views/tasks";
import { loadBucketMap, saveBucket } from "$lib/persist";
import type { OutlineItem } from "$lib/outline";

/** What the center pane shows: the editor or one of the projection views.
 *  "reading" is the web reading projection (v0.5 P2) — its content and page history
 *  come from the reading store; navStack only records the one stop "went to the
 *  projection" (READING_ENTRY) and not each page inside it (following v0.4's position
 *  that browsing history does not mix into document history). */
export type MainView =
  | "editor" | "agenda" | "kanban" | "timeline" | "blocks" | "claims" | "evidence" | "graph" | "sessions" | "reading";

/** An editor tab. The id is an incrementing stable identity (the path changes on a
 *  rename, and both the caches and the keyed each go by id); pinned=false means the
 *  "preview slot" (italic, at most one per column, reused by a single click). */
export interface Tab {
  id: number;
  path: string;
  pinned: boolean;
}

/** How a path stands to the privacy zone (see VaultStore.privacyStatus). */
export type PrivacyStatus = { kind: "none" } | { kind: "root" } | { kind: "inherited"; zone: string };

/** The snapshot of a suspended Editor. state is opaque to the store (a CodeMirror
 *  EditorState — the store does not import CM, to keep the dependency out; and it must
 *  never enter $state, since the deep proxy would break CM's internal identity
 *  comparisons). content is the serialisation in the on-disk form (withEol), compared
 *  against the disk on activation: only when they are equal is the state reused (so
 *  undo, the cursor and the scroll survive), and otherwise the cache is dropped and it
 *  is rebuilt. */
export interface EditorSnapshot {
  state: unknown;
  scrollTop: number;
  eol: "\n" | "\r\n";
  content: string;
}

// UI state (panel open/closed, the tab selected) is stored in localStorage rather than
// Rust's settings: it is purely presentational and not worth an IPC; bad data falls
// back to the defaults
const UI_KEY = "vemry-ui";
// rightTab is stored as a string: an older version's "browser" tab was folded into the
// research panel ("agent"), and the "links" tab was renamed "outline" (the panel's main
// identity moved from links to the outline, with the links collapsed at the bottom)
// leftW/rightW: the sidebar widths (px) the user set by dragging the sash; absent =
// each one's default flexible width
// swap: the two sidebars change sides
// blOpen/outOpen: whether the outline panel's bottom "backlinks / outgoing links"
// sections are expanded; collapsed by default and not persisted
type UiState = {
  leftOpen?: boolean; rightOpen?: boolean; rightTab?: string;
  leftW?: number; rightW?: number; swap?: boolean;
  blOpen?: boolean; outOpen?: boolean;
};
function loadUi(): UiState {
  try {
    return JSON.parse(localStorage.getItem(UI_KEY) ?? "{}") as UiState;
  } catch {
    return {};
  }
}
const ui = loadUi();

/** A persisted sidebar width is only trusted when finite, and is clamped to that
 *  column's minimum; bad data falls back to null (the default width).
 *  The upper bound is not handled here — the grid's content-area min catches it, and one
 *  more drag returns it to the legal range. */
function loadWidth(v: unknown, min: number): number | null {
  return typeof v === "number" && Number.isFinite(v) ? Math.round(Math.max(min, v)) : null;
}

// The file tree's expanded state is persisted per vault (the root's absolute path → an
// array of expanded folder paths).
// The same decision as vemry-ui: purely presentational state goes to localStorage and is
// not worth an IPC
const TREE_KEY = "vemry-tree";
/** The editor tabs are persisted per vault (path + pinned + the active index). Undo and
 *  the scroll are not stored (an EditorState is not serialisable); after a restart the
 *  tab list comes back as it was and the active one is loaded again. */
const TABS_KEY = "vemry-tabs";
interface TabsBucket {
  tabs: { path: string; pinned: boolean }[];
  active: number;
}

/** The "reading projection" sentinel entry in navStack — beginning with NUL, it cannot
 *  be a file path, so pruneHistory's path predicate never matches it (and once the
 *  documents between them are pruned, two adjacent projection entries are merged along
 *  the way). The page history inside the projection is still the reading store's own,
 *  and this only records the one stop "went to the projection", which lets the
 *  document ←/→ pass back and forth through it. */
const READING_ENTRY = "\u0000reading";

/** The sentinel path prefix of a projection view tab (the same NUL convention as
 *  READING_ENTRY: it cannot be a file path, so the path predicates of rename rewriting,
 *  dropTabs and the file events never match it), followed by the view's name. A view tab
 *  has no buffer and no snapshot cache, and its preview slot, promotion, closing and
 *  ordering semantics are exactly a document tab's; both the rail and the command
 *  palette open one into the tab system through openView, and
 *  mainView ∈ views ⟺ the active tab is that view (maintained jointly by openView,
 *  activateTab and closeTab; never write mainView directly around them). The reading
 *  projection (reading) does not enter the tabs: it is an overlay state with its own way
 *  in and out (enterReading/exitReading). */
const VIEW_TAB_PREFIX = "\u0000view:";
export type ProjectionView = Exclude<MainView, "editor" | "reading">;
/** The full set of projection views. Their names are not here — the rail, the tab bar
 *  and the command palette share the catalog's `view.<id>`, so a view has one display
 *  name. */
export const PROJECTION_VIEWS: Record<ProjectionView, true> = {
  agenda: true, kanban: true, timeline: true, blocks: true, claims: true,
  evidence: true, graph: true, sessions: true,
};
/** A tab path → a view name. A non-view tab returns null; bad data with an unknown view
 *  name returns null too — it is treated as an ordinary path, culled on a vault switch,
 *  and dies out by itself. */
export function viewOfTab(path: string): ProjectionView | null {
  if (!path.startsWith(VIEW_TAB_PREFIX)) return null;
  const v = path.slice(VIEW_TAB_PREFIX.length);
  return v in PROJECTION_VIEWS ? (v as ProjectionView) : null;
}

/** One record of the file-operation undo stack. Only operations the user started
 *  themselves are recorded (from the tree, a panel, or creating through a link); the
 *  agent's and external changes do not enter it — they are not "the step I just took". */
export type FileOp =
  | { kind: "rename"; from: string; to: string }
  | { kind: "create"; paths: string[] }
  | { kind: "delete"; path: string };

const UNDO_MAX = 50;

class VaultStore {
  root = $state<string | null>(null);
  noteCount = $state(0);
  notes = $state<NoteSummary[]>([]);
  folders = $state<string[]>([]);
  /** non-Markdown files in the vault (attachments, data files, …) */
  otherFiles = $state<string[]>([]);
  recent = $state<string[]>([]);
  /** Whether recent has really been loaded from the backend — the persistence trimming
   *  (saveBucket's keep) only trusts a loaded value, and the empty array of a startup
   *  race or a failed IPC must not be read as "no other vault exists" (which would
   *  wrongly delete the stale buckets). */
  recentLoaded = false;
  /** privacy-zone paths (notes, files, folders) — the badges in the tree */
  privatePaths = $state<Set<string>>(new Set());
  /** The file tree's expanded set (empty = all collapsed, which is the state on first open), persisted per vault to vemry-tree */
  expanded = $state<Record<string, true>>({});

  /** The file-operation undo stack (LIFO, this session only, cleared on a vault switch).
   *  It is $state only so a menu item's "undo X" label can reflect the top of the stack
   *  in real time. */
  undoOps = $state<FileOp[]>([]);

  private recordOp(op: FileOp) {
    this.undoOps.push(op);
    if (this.undoOps.length > UNDO_MAX) this.undoOps.shift();
  }

  /** Undo the most recent file operation. A rename is renamed back (renamePath handles
   *  the navigation stack, the expanded set and the open document following it); a create
   *  or an import is moved to the trash (the content can still be recovered, which is
   *  safer than an outright rm — a file edited after being created does not vanish into
   *  thin air either); a delete cannot be undone automatically (macOS has no
   *  restore-from-trash API), so it only says where it went.
   *  On failure (the path was taken or moved by an outside change) the entry has already
   *  been popped, and it reports the error without retrying. */
  async undoFileOp() {
    const op = this.undoOps.pop();
    if (!op) return;
    try {
      if (op.kind === "rename") {
        await this.renamePath(op.to, op.from, false);
        toast.push(t("vaultStore.undoneMovedBack", { path: op.from }));
      } else if (op.kind === "create") {
        // A bulk undo refreshes once at the end: a refreshNotes per item inside the loop would be N×4 IPCs for nothing
        for (const p of [...op.paths].reverse()) await this.deletePath(p, false, false);
        await this.refreshNotes();
        toast.push(
        op.paths.length > 1
          ? t("vaultStore.undoneImport", { n: op.paths.length })
          : t("vaultStore.undoneCreate", { path: op.paths[0] }),
      );
      } else {
        toast.push(t("vaultStore.alreadyInTrash", { path: op.path }));
      }
    } catch (e) {
      this.error = String(e);
    }
  }

  /** Expand folder and every one of its ancestors. Written and never read (an
   *  unconditional assignment): when the reveal $effect calls it, no dependency on
   *  expanded is registered — reading expanded here to make a decision would have the
   *  effect bounce an ancestor of the current note straight back open the moment the user
   *  collapsed it (making it uncollapsible). */
  expandFolder(folder: string) {
    const parts = folder.split("/");
    for (let i = 1; i <= parts.length; i++) this.expanded[parts.slice(0, i).join("/")] = true;
  }

  // ---- The editor tabs. The core invariant is the **single live buffer**: at any
  // moment only the active tab holds a live EditorView and only it can be dirty; a
  // background tab holds a cached snapshot alone (undo, cursor, scroll), the content's
  // source of truth is the disk, and activation reads it back and validates lazily (same
  // content → reuse the cache and keep undo; different → drop the cache and rebuild).
  // That is what keeps the `=== currentPath` tests of dirty/dirtyGuard/insertBlock in
  // their original meaning, and what makes it harmless that FilesChanged carries no path.
  // Preview tab semantics: a single click opens into the preview slot (at most one
  // pinned:false tab, reused by later single clicks); editing or a double click promotes
  // it. Both uniqueness properties (one preview slot, one tab per path) are maintained by
  // the single openNote/openFile funnel — do not open another entry point elsewhere. ----
  tabs = $state<Tab[]>([]);
  activeTabId = $state<number | null>(null);
  private nextTabId = 1;
  /** The side table of background tabs' editor snapshots. Deliberately not $state: an
   *  EditorState would be wrapped in Svelte's deep proxy and every internal identity
   *  comparison inside CM would break. The key is tab.id, and it only belongs to a
   *  suspended tab (deleted on activation, replacement or closing). */
  private tabCaches = new Map<number, EditorSnapshot>();

  currentPath = $state<string | null>(null);
  currentContent = $state("");
  private _dirty = $state(false);
  /** Incremented on every transition into the dirty state: saveCurrent uses it to
   *  recognise "more input arrived while the write was awaited", so a buffer still
   *  differing from the disk is not marked saved. */
  private editSeq = 0;
  get dirty(): boolean {
    return this._dirty;
  }
  set dirty(v: boolean) {
    if (v) this.editSeq++;
    this._dirty = v;
  }
  /** The "write out unsaved edits now" hook Editor registers. Opening, renaming, switching
   *  vaults or switching tabs — anything that takes an old buffer's ownership away —
   *  awaits it first, to avoid reading stale content or losing an edge write. */
  flushEditor: (() => Promise<unknown> | undefined) | null = null;
  /** The "snapshot the current editor" hook Editor registers (used when suspending a
   *  tab). null = there is no snapshottable text editor (an image, a PDF, nothing).
   *  **It has to be taken after flushEditor** — only then is state.doc ≡ the disk, which
   *  is what makes the content field a trustworthy baseline for the lazy validation. */
  snapshotEditor: (() => EditorSnapshot | null) | null = null;
  /** The one-shot restore payload: set when activateTab's validation passes and cleared
   *  the moment Editor's loadDoc consumes it — a leftover would feed a stale state to the
   *  next unrelated load. */
  pendingRestore: EditorSnapshot | null = null;
  /** Discard the pending save timer of a given path (or of anything under that directory)
   *  in the editor. deletePath calls it on the way in: otherwise the flush triggered by
   *  the unmount or switch after a deletion would run the existence guard against a stale
   *  file list from before the deletion and write the file just deleted back (a bulk undo
   *  delete passes refresh=false, so the list is certainly stale). */
  discardEditor: ((path: string) => void) | null = null;
  /** The hooks that keep outside state in step with a path being deleted or renamed (the
   *  assigner is the clipTarget store: a pinned destination is invalidated by a deletion
   *  and follows a rename — ensureLive only covers the moment of use, and a label still
   *  pointing at a deleted note would mislead). The optional-hook pattern matches
   *  discardEditor, so nothing is imported back and no cycle forms. */
  pathGone: ((path: string) => void) | null = null;
  pathMoved: ((from: string, to: string) => void) | null = null;
  /** The "insert text at the cursor" hook Editor registers (shared by clipping and a
   *  future script insertion).
   *  It goes through the CodeMirror doc → onDirty → autosave, which is naturally
   *  consistent with the flushEditor contract; a bare noteWrite around the editor would
   *  be overwritten by the stale buffer's autosave.
   *  block=true = a block-level payload (a quote block, a list item), aligned to line
   *  boundaries and never overwriting the selection.
   *  Returns false = there is no note editor to insert into. */
  insertAtCursor: ((text: string, block?: boolean) => boolean) | null = null;
  /** The "insert at the end of a section" hook Editor registers (used when the target is
   *  the note that is open and a destination section was given; as above it has to go
   *  through the buffer). It returns where it actually landed ("tail" = the section was
   *  missing and it fell back to the end of the file, which the caller reports), and
   *  false = the editor is unavailable. */
  insertAtSection: ((heading: string, text: string) => "section" | "tail" | false) | null = null;
  /** The "jump to a document character position and scroll it into view" hook Editor
   *  registers (used by the outline's clicks).
   *  Returns false = there is no note editor to jump in. */
  revealPos: ((pos: number, flash?: boolean) => boolean) | null = null;
  /** Block commands from the palette (move / duplicate / delete the selected run or the
   *  block at the cursor); registered by the editor, null when no note is open. */
  blockCommand: ((op: "move-up" | "move-down" | "duplicate" | "delete") => boolean) | null = null;
  /** Under a projection view it returns to the editor before scrolling into view — the
   *  editor host needs a tick to leave display:none before CM can measure the scroll.
   *  The outline's jumps and "land back after closing a browser window" share this one
   *  contract (change it in one place and both entry points get it). */
  async revealInEditor(pos: number, flash = false) {
    // With no active document (a view tab is active, or there is no tab): there is
    // nowhere to jump, and mainView must not be switched either — that would produce the
    // decoupled state "the editor's empty view is shown while the active tab is a view".
    // Every current caller comes in after a successful openNote, and this is the guard rail
    if (this.currentPath == null) return;
    if (this.mainView !== "editor") this.mainView = "editor";
    await tick();
    this.revealPos?.(pos, flash);
  }

  /** Open a note and jump to a **byte** span (the units of Rust's block.span). The one
   *  primitive behind "jump to the source" on a query result card: openNote may fail or
   *  land elsewhere, so `currentPath` has to be rechecked after the await; the byte →
   *  UTF-16 conversion also lives here alone. */
  async openBlockAt(note: string, byteStart: number, content?: string, flash = true) {
    await this.openNote(note);
    if (this.currentPath !== note) return;
    let pos = byteToChar(this.currentContent, byteStart);
    // the index may lag an unsaved buffer: the block's text, if given, corrects the landing point
    if (content && this.locateText) pos = this.locateText(content, pos) ?? pos;
    await this.revealInEditor(pos, flash);
  }
  /** Editor hook: where `text` sits in the open buffer, nearest to `near` (exact at
   *  `near` first); null when the buffer no longer holds it. */
  locateText: ((text: string, near: number) => number | null) | null = null;
  /** Editor hook: copy `[[Stem#^id]]` (or the embed) for the block at the cursor. */
  copyBlockRef: ((embed: boolean) => boolean) | null = null;
  /** The current note's heading outline, recomputed by Editor behind an input debounce
   *  (it does not wait for a disk write, which is why it does not go through
   *  query_blocks). null = not a note, or no document open. */
  outline = $state<OutlineItem[] | null>(null);
  /** The editor's cursor position (a character offset), from which the outline panel highlights the current section. */
  editorCursor = $state(0);
  /** The cursor's 1-based line:column (for the status line). null = there is no text editor right now (an image, a PDF). */
  editorLoc = $state<{ line: number; col: number } | null>(null);
  /** The cursor sits inside a `> [!private]` block (the editor's own judgement of the
   *  buffer; the status bar shows the block-level privacy badge). */
  editorBlockPrivate = $state(false);
  /** Provenance of the block under the cursor (null = no text editor, or nothing known yet). */
  editorOrigin = $state<Origin | null>(null);
  /** AI blocks of the current note still awaiting review (the status bar's "pending N"). */
  pendingCount = $state(0);
  /** The four-colour origin overlay (a command toggles it; per process, not persisted). */
  provenanceOverlay = $state(false);
  /** Bumped after an accept so the editor re-reads the ledger (accepting is not a note event and is not broadcast). */
  provenanceEpoch = $state(0);
  /** Editor hook: move the cursor to the next pending block; false when there is none. */
  nextPending: (() => boolean) | null = null;
  /** Editor hook: the pending block under the cursor (its local id), or null. */
  pendingAtCursor: (() => string | null) | null = null;
  /** The selection's statistics (for the status line). chars is computed from the doc
   *  (in UTF-16 units) — editorSelection.text is truncated at SELECTION_MAX_CHARS and
   *  cannot be counted. */
  editorSelStats = $state<{ chars: number; lines: number } | null>(null);
  /** The current document's word count (for the status line), recomputed by Editor on the
   *  same 200ms debounce as the outline — currentContent is stale while dirty, so it has
   *  to come from the live buffer. null = not text. */
  docWords = $state<number | null>(null);
  /** A snapshot of the editor's current selection (null = no selection or no document),
   *  recomputed by Editor as the selection changes; both the @ candidate layer's "current
   *  selection" and the main window → chat window push read it. The text is already
   *  truncated at SELECTION_MAX_CHARS, so selecting a whole document does not carry a
   *  huge string on every drag. */
  editorSelection = $state<EditorSelection | null>(null);
  /** Bumped when the open note is replaced from outside (agent/external edit). */
  reloadCounter = $state(0);

  leftOpen = $state(ui.leftOpen ?? true);
  rightOpen = $state(ui.rightOpen ?? true);
  rightTab = $state<"outline" | "agent">(ui.rightTab === "links" || ui.rightTab === "outline" ? "outline" : "agent");
  /** Whether the outline panel's bottom link sections are expanded (rarely used, collapsed by default, with the count badge always visible). */
  backlinksOpen = $state(ui.blOpen === true);
  outgoingOpen = $state(ui.outOpen === true);
  /** The sidebar widths (px), null = the default flexible width (the tree 220 / the panel
   *  min(30%, 560)).
   *  The minimums of 140/280 go with App.svelte's grid comment and Rust's
   *  MIN_MAIN_VISIBLE.
   *  left always means the file tree and right always the links/research panel, whichever
   *  side each is docked on at the moment (swapping affects presentation alone, and the
   *  open/close shortcuts and the state's meaning do not change). */
  leftWidth = $state<number | null>(loadWidth(ui.leftW, 140));
  rightWidth = $state<number | null>(loadWidth(ui.rightW, 280));
  /** The sash being dragged (App renders the handle's highlight and the cursor overlay
   *  from it). While dragging, persisting the UI state is paused — a synchronous
   *  localStorage write at pointermove frequency stalls the live layout, and when it goes
   *  back to null on release the persistence effect reruns and writes the final width. */
  resizing = $state<"tree" | "panel" | null>(null);
  /** true = the file tree is docked on the right and the links/research panel on the left. */
  swapSides = $state(ui.swap === true);
  mainView = $state<MainView>("editor");
  /** The manual trigger for "reveal in the file tree": FileTree's reveal effect has to
   *  rerun even when currentPath did not change (revealing the current note), and
   *  incrementing this triggers it. */
  revealTick = $state(0);

  /** Expand the left column and reveal the current note in the file tree (the context menu's "reveal in file tree"). */
  revealInTree() {
    this.leftOpen = true;
    this.revealTick++;
  }
  /** Which view the main pane held before entering the reading projection — the return
   *  address of Esc and the ✕ "leave this mode" (the ⌘⌥← / ← timeline back goes through
   *  navStack's projection entry, see enterReading).
   *  The only path that makes mainView "reading" is enterReading, so what exitReading
   *  consumes is necessarily fresh (landed and openVault writing "editor" directly only
   *  deny an old value the chance to be consumed, and the next entry overwrites it). */
  private readingOrigin: MainView | null = null;

  /** Switch to the reading projection: record where it came from and record "the
   *  projection" in navStack as one stop — so after landing on a note from the projection
   *  (a clip, an already clipped link), the document ← returns to the projection.
   *  Already inside the projection nothing is rewritten (changing pages inside it is the
   *  reading store's own business).
   *  goBack/goForward landing on a projection entry re-enter with record=false. */
  enterReading(record = true) {
    if (this.mainView !== "reading") this.readingOrigin = this.mainView;
    this.mainView = "reading";
    if (record) this.recordVisit(READING_ENTRY);
  }

  /** Leave the projection and return where it came from (which cannot be "reading", and an empty value falls back to the editor). */
  exitReading() {
    if (this.mainView !== "reading") return;
    // Should it have come from a projection view, that view's tab has to still be there —
    // a ⌘W while reading may have closed it (the close-to-empty branch leaves reading
    // alone). When they do not line up it returns to the editor's empty view rather than
    // producing the residue of "a view with no tab"
    const origin = this.readingOrigin ?? "editor";
    const t = this.tabs.find((t) => t.id === this.activeTabId);
    const backed = t ? viewOfTab(t.path) : null;
    this.mainView = origin === "editor" || origin === backed ? origin : "editor";
    this.readingOrigin = null;
    // Lining up the timeline: Esc and ✕ leave a mode rather than moving through history,
    // but what is shown is already the current document — so when the index still sits on
    // the projection entry it moves to the matching neighbour, and ←/→ neither slips nor
    // stops working
    if (this.navStack[this.navIndex] === READING_ENTRY && this.currentPath !== null) {
      if (this.navStack[this.navIndex - 1] === this.currentPath) this.navIndex--;
      else if (this.navStack[this.navIndex + 1] === this.currentPath) this.navIndex++;
    }
  }
  /** The return path of clicking the active tab (activateTab exits early on the active
   *  slot, and landed cannot be reached): a document tab shows the editor, and a view tab
   *  shows that view (usually it is already shown, and the only case with anything to do
   *  is being covered by reading). The reading projection goes through exitReading first,
   *  to get its timeline lining up; a document tab that came from the projection is still
   *  pushed back to the editor — clicking a tab says "show me this", not "go back one
   *  level". */
  revealActiveTab() {
    const t = this.tabs.find((t) => t.id === this.activeTabId);
    const view = t ? viewOfTab(t.path) : null;
    if (this.mainView === "reading") this.exitReading();
    this.mainView = view ?? "editor";
  }

  /** Close the current projection view tab (shared by Esc, the view header's ✕ and
   *  pressing the rail entry again).
   *  closeTab activates the neighbour: a document neighbour returns to the editor through
   *  landed, a view neighbour is switched to, and closing to empty has closeTab fall back
   *  to the editor's empty view itself. When the active tab is not a view this is the
   *  guard-rail branch (the residue of mainView and the tabs having come apart), and it
   *  pushes straight back to the editor. */
  async closeActiveView() {
    const t = this.tabs.find((t) => t.id === this.activeTabId);
    if (t && viewOfTab(t.path)) await this.closeTab(t.id);
    else if (this.mainView !== "reading") this.mainView = "editor";
  }

  /** Bumped on every vault event — the requery signal for projection views. */
  indexVersion = $state(0);

  error = $state<string | null>(null);

  // ---- Forward and back: the visit history (browser semantics: a new visit truncates
  //      the forward side, and repeats are deduplicated).
  //      An entry is a document path or READING_ENTRY (the stop of entering the reading
  //      projection) ----
  navStack = $state<string[]>([]);
  navIndex = $state(-1);

  private unlistenVault: UnlistenFn | null = null;

  get canGoBack(): boolean {
    return this.navIndex > 0;
  }

  get canGoForward(): boolean {
    return this.navIndex >= 0 && this.navIndex < this.navStack.length - 1;
  }

  private recordVisit(path: string) {
    if (this.navStack[this.navIndex] === path) return;
    const stack = [...this.navStack.slice(0, this.navIndex + 1), path];
    if (stack.length > 100) stack.shift();
    this.navStack = stack;
    this.navIndex = stack.length - 1;
  }

  /** The single ending after a successful open: return to the editor and record it in the visit history when asked. */
  private landed(path: string, record: boolean) {
    // shared navigation contract: clicking an entry in any projection
    // view lands back in the editor
    this.mainView = "editor";
    if (record) this.recordVisit(path);
  }

  async goBack() {
    if (!this.canGoBack) return;
    this.navIndex--;
    await this.openFromStack();
  }

  async goForward() {
    if (!this.canGoForward) return;
    this.navIndex++;
    await this.openFromStack();
  }

  private async openFromStack() {
    const path = this.navStack[this.navIndex];
    // A projection entry: return to the reading projection (showing the reading store's
    // current page), without recording it again and without touching the editor (which
    // stays mounted, its buffer intact, so no flush is needed)
    if (path === READING_ENTRY) {
      this.enterReading(false);
      return;
    }
    await this.openFile(path, false);
    // The open failed (deleted, unreadable): it is pruned from the history, and openNote has already toasted the error
    if (this.currentPath !== path) this.pruneHistory((p) => p === path);
  }

  /** Prune the paths matching a predicate from the history and keep navIndex on the
   *  surviving current item.
   *  Adjacent duplicates produced by the pruning are merged along the way (otherwise one
   *  back would "navigate" to the same note and reset the cursor and the undo stack for
   *  nothing). The anchoring priority: the current item recorded in the history → the
   *  document actually on screen (a failed goBack/goForward ends up here) → the top of
   *  the stack. */
  private pruneHistory(gone: (p: string) => boolean) {
    const current = this.navStack[this.navIndex];
    const stack: string[] = [];
    for (const p of this.navStack) {
      if (gone(p) || stack[stack.length - 1] === p) continue;
      stack.push(p);
    }
    const anchor =
      current !== undefined && !gone(current)
        ? stack.lastIndexOf(current)
        : this.currentPath !== null
          ? stack.lastIndexOf(this.currentPath)
          : -1;
    this.navStack = stack;
    this.navIndex = anchor >= 0 ? anchor : stack.length - 1;
  }

  /** Whether the open document is a note (markdown editing + links) or a plain file. */
  get currentIsNote(): boolean {
    return this.currentPath?.endsWith(".md") ?? false;
  }

  /** Whether a clip or a quote insertion would land in the note the user is looking at.
   *  Editor stays mounted (under a projection or another view it is only hidden by CSS),
   *  and insertAtCursor itself cannot tell "insert into the visible note" from "insert
   *  into the old note behind the projection" — so every entry point that inserts into
   *  "the current note" passes this guard first, and what fails it lands in the capture
   *  (inbox) and says so. */
  get canInsertIntoVisibleNote(): boolean {
    return this.mainView === "editor" && this.currentIsNote && this.insertAtCursor != null;
  }

  /** How the open document renders: note editor, plain editor, or preview. */
  get currentKind(): "note" | "text" | "image" | "pdf" | null {
    return this.currentPath ? fileKind(this.currentPath) : null;
  }

  async loadRecent() {
    try {
      this.recent = await api.vaultRecent();
      this.recentLoaded = true;
    } catch {
      // Keep the previous values (initially empty); recentLoaded is not set, so persistence skips the trimming on failure
    }
  }

  /** The context menu's "remove from the recent list": a settings-level operation that
   *  does not touch the disk. The command returns the updated list directly, saving a
   *  round trip; the tree bucket's allowlist (saveBucket's keep) narrowing with it is
   *  deliberate — removing it from recent means "this vault no longer matters", and its
   *  expanded state goes too. */
  async removeRecent(path: string) {
    try {
      this.recent = await api.vaultRecentRemove(path);
      this.recentLoaded = true;
    } catch (e) {
      this.error = String(e);
    }
  }

  async openVault(path: string) {
    await this.enterVault(() => api.vaultOpen(path));
  }

  /** Create a vault (a path that does not exist or an empty directory → the .vemry
   *  skeleton plus the VEMRY.md template); once Rust has created it, it goes through the
   *  same chain as vault_open, so everything after entering matches opening exactly. */
  async createVault(path: string) {
    await this.enterVault(() => api.vaultCreate(path));
  }

  private async enterVault(open: () => Promise<{ root: string; note_count: number }>) {
    this.error = null;
    try {
      if (agent.running) {
        const ok = await confirm(t("vaultStore.switchWhileRunning"), {
          title: t("cmd.switch-vault.title"),
          kind: "warning",
        });
        if (!ok) return;
        await agent.cancel();
      }
      // The detached chat window belongs to the old vault: close it, and the draft and
      // references it hands back are discarded with it (discard).
      // It has to really close first (the handshake is asynchronous and the close command
      // only starts it): while the webview is still alive it is subscribed to vault
      // events, would refresh the old window with the new vault's lists, and its session
      // reads and writes would land in the new vault's meta directory
      if (agent.windowOpen) {
        await api.agentWindowClose(null, true).catch(() => {});
        for (let i = 0; i < 50 && (await api.agentWindowIsOpen().catch(() => false)); i++) {
          await new Promise((r) => setTimeout(r, 100));
        }
        // A late closed event no longer reads the old session back (reattach gates on windowOpen)
        agent.windowOpen = false;
      }
      // Write the unsaved edits into the *old* vault first: once Rust has switched, a
      // pending autosave would write the old note as a phantom file in the new vault
      await this.flushEditor?.();
      const info = await open();
      this.adoptRoot(info); // the tabs are restored from the bucket (a list of paths, with no content loaded)
      this.currentPath = null;
      this.currentContent = "";
      this.dirty = false;
      this.mainView = "editor";
      this.navStack = [];
      this.navIndex = -1;
      // The relative paths in the undo stack belong to the old vault
      this.undoOps = [];
      // A conversation does not cross vaults: once Rust has switched, its history is void, and the frontend's timeline is cleared with it
      agent.reset();
      // The paths in the citation-landed cache belong to the old vault
      clips.reset();
      await this.refreshNotes();
      // folders is fresh at this point: prune the expanded entries that no longer exist (orphans of an external or agent rename)
      const live = new Set(this.folders);
      for (const p of Object.keys(this.expanded)) if (!live.has(p)) delete this.expanded[p];
      // The watcher and the recent list are in place before the tabs are loaded:
      // activateTab reads the disk, and putting it first would leave the file watcher a
      // whole IO round late, missing every external change in that window
      await this.watchVault();
      await this.loadRecent();
      // The restored tabs are culled against the new lists (orphans of an external delete
      // or rename) before the active tab's content is loaded; activateTab requires the id
      // to change, so the active slot is cleared first. A note or text file missing from
      // the lists is read once to confirm before being cut — the same fallback as the
      // files_changed delete check (a hidden path, or one whose case does not match, is
      // not in the lists to begin with, and one that reads is left alone) — or a tab alive
      // for the whole session would vanish on a restart
      const savedActive = this.activeTabId;
      const keeps = await Promise.all(this.tabs.map(async (t) => {
        if (viewOfTab(t.path) !== null) return true; // a view tab is not a file and lives forever (activateTab lines mainView back up)
        const kind = fileKind(t.path);
        const listed = kind === "note" ? this.notes.some((n) => n.id === t.path) : this.otherFiles.includes(t.path);
        if (listed) return true;
        if (kind !== "note" && kind !== "text") return false; // image/pdf have no delete detection, so the lists decide (reading the disk cannot judge a binary either)
        return (kind === "note" ? api.noteRead(t.path) : api.fileRead(t.path)).then(() => true, () => false);
      }));
      this.tabs = this.tabs.filter((_, i) => keeps[i]);
      this.activeTabId = null;
      const act = this.tabs.find((t) => t.id === savedActive) ?? this.tabs[this.tabs.length - 1];
      if (act) await this.activateTab(act.id, true);
    } catch (e) {
      this.error = String(e);
    }
  }

  /** Startup: reopen the last session's vault; fall back to the welcome page. */
  async restoreLast() {
    try {
      const last = await api.vaultLast();
      if (last) await this.openVault(last);
    } catch {
      /* welcome page handles it */
    }
  }

  /** A second webview taking over the vault already open. `watch=false` does not
   *  subscribe to the vault events: handleEvent refetches all four vault-wide lists on
   *  any event, and an autosave fires every second — so each extra subscribed window is
   *  another "four IPCs per keystroke". There is one clip toolbar per browser window, and
   *  it only wants the note list at the moment the destination picker expands, where
   *  fetching it then is enough (and fresher);
   *  `notes=false` also saves those four round trips at mount time — right then the
   *  toolbar only needs the root (for clipTarget to restore its pin), and since the lists
   *  are fetched in openPicker anyway, fetching them twice would simply discard the
   *  first.
   *  It touches no editor state — the chat window has no editor, currentPath is always
   *  null there, and "the current note" is pushed by the main window (agent.hostNote).
   *  expanded is restored from the bucket in the same batch as root: a module-level
   *  persistence effect seeing (a new root, an empty expanded) would write the main
   *  window's expanded set away as empty. */
  async attachCurrent(watch = true, notes = true) {
    const info = await api.vaultCurrent();
    if (!info) return;
    this.adoptRoot(info);
    if (notes) await this.refreshNotes();
    if (watch) await this.watchVault();
  }

  /** Adopt a vault's root: shared by a vault switch (enterVault) and a second webview
   *  taking over (attachCurrent). The three assignments have to be **one synchronous
   *  batch** — should the persistence $effect see the combination (a new root, an
   *  old/empty expanded) it would write that state into the new vault's bucket, ahead of
   *  the load, wiping out what was stored. This invariant lives here alone; do not
   *  rewrite it at a call site. */
  private adoptRoot(info: { root: string; note_count: number }) {
    this.root = info.root;
    this.expanded = Object.fromEntries((loadBucketMap<string[]>(TREE_KEY)[info.root] ?? []).map((p) => [p, true]));
    this.noteCount = info.note_count;
    // The tabs are restored in the same synchronous batch as root (the same invariant as
    // expanded): should the persistence effect see (a new root, empty tabs) it would write
    // the empty value into the new vault's bucket and wipe out what was stored — the
    // detached chat window's attachCurrent comes through here too, writing back the very
    // values just restored, which is harmless.
    // Dead paths are not culled here (the lists have not refreshed yet); enterVault tidies
    // up after refreshNotes
    const saved = loadBucketMap<TabsBucket>(TABS_KEY)[info.root];
    this.tabCaches.clear();
    this.pendingRestore = null;
    this.tabs = (saved?.tabs ?? []).map((t) => ({ id: this.nextTabId++, path: t.path, pinned: t.pinned }));
    this.activeTabId = this.tabs[saved?.active ?? -1]?.id ?? null;
  }

  /** Subscribe to the vault events (re-entrant: the old subscription is dropped first). */
  private async watchVault() {
    this.unlistenVault?.();
    this.unlistenVault = await onVaultEvent((e) => this.handleEvent(e));
  }

  async refreshNotes() {
    try {
      // Every vault event (the echo of every autosave included) comes through here: the
      // four read-only queries are independent, and sending them in parallel saves three
      // IPC round trips
      const [notes, folders, files, priv] = await Promise.all([
        api.notesList(),
        api.foldersList(),
        api.filesList(),
        api.privacyList(),
      ]);
      this.notes = notes;
      this.noteCount = notes.length;
      this.folders = folders;
      this.otherFiles = files.filter((f) => !f.endsWith(".md"));
      this.privatePaths = new Set(priv);
    } catch (e) {
      this.error = String(e);
    }
  }

  /** The outermost private ancestor folder of a path — the folder that carries the badge
   *  in the file tree — or null when no folder above it is in the set. Walks from the
   *  root down, so for nested zones (`a/` and `a/b/` both private) `a/b/c.md` reports `a`. */
  privacyZoneOf(path: string): string | null {
    for (let i = path.indexOf("/"); i !== -1; i = path.indexOf("/", i + 1)) {
      const ancestor = path.slice(0, i);
      if (this.privatePaths.has(ancestor)) return ancestor;
    }
    return null;
  }

  /** "root": private with no private ancestor folder — the row that shows the badge and
   *  the one whose unmark means something. "inherited": private because a folder above it
   *  is; Rust removes only the exact rule line, so unmarking it would be a silent no-op.
   *  "none": not private — an exempted path (`!diary/Public.md`) inside a zone lands here
   *  on purpose and renders as an ordinary row, which is what makes it visibly public.
   *  Known degradation: a rule that names contents rather than the folder (`diary/**`)
   *  leaves `diary` itself out of the set, so each direct child is a root of its own. */
  privacyStatus(path: string): PrivacyStatus {
    if (!this.privatePaths.has(path)) return { kind: "none" };
    const zone = this.privacyZoneOf(path);
    return zone === null ? { kind: "root" } : { kind: "inherited", zone };
  }

  async markPrivate(path: string, kind: "note" | "folder" | "file") {
    if (this.refusePrivacyToggleWhileDirty(path, kind)) return;
    try {
      await api.privacyMark(path, kind);
    } catch (e) {
      this.error = String(e);
    }
  }

  async unmarkPrivate(path: string, kind: "note" | "folder" | "file") {
    if (this.refusePrivacyToggleWhileDirty(path, kind)) return;
    try {
      await api.privacyUnmark(path, kind);
      // Rust removes only the exact rule line / the frontmatter flag: a path also covered
      // by a broader glob stays private, and the broadcast would merely redraw the same
      // badge. Refetch here rather than waiting for privacy_changed, so the verdict reads
      // the new state and not a race with handleEvent.
      await this.refreshNotes();
      if (this.privatePaths.has(path)) toast.push(t("vaultStore.stillPrivate"));
    } catch (e) {
      this.error = String(e);
    }
  }

  /** Toggling a note's frontmatter `private` flag rewrites the file on disk,
   *  but a dirty editor buffer won't reload (handleEvent skips dirty notes) —
   *  the next ⌘S would silently overwrite the flag. Refuse instead. */
  private refusePrivacyToggleWhileDirty(path: string, kind: string): boolean {
    if (kind === "note" && path === this.currentPath && this.dirty) {
      this.error = t("vaultStore.dirtyBeforePrivacy", { key: k("Mod+S") });
      return true;
    }
    return false;
  }

  async createNoteIn(folder: string | null, title: string) {
    const id = await api.noteCreate(title, folder ?? undefined);
    this.recordOp({ kind: "create", paths: [id] });
    await this.refreshNotes();
    // A newly created file gets a pinned tab: it is about to be edited (and editing would promote it anyway), so pinning it avoids a flash of italics
    await this.openNote(id, true, true);
  }

  async createFolder(parent: string | null, name: string) {
    const path = parent ? `${parent}/${name}` : name;
    const rel = await api.folderCreate(path);
    this.recordOp({ kind: "create", paths: [rel] });
    await this.refreshNotes();
  }

  /** Explicit-extension semantics: the extension you type is what you get. */
  async createFile(parent: string | null, name: string) {
    const path = parent ? `${parent}/${name}` : name;
    const rel = await api.fileCreate(path);
    this.recordOp({ kind: "create", paths: [rel] });
    await this.refreshNotes();
    await this.openFile(rel, true, true); // a newly created file gets a pinned tab
  }

  /** Move a file/folder to the OS trash and refresh.
   *  record=false is for the undo's own use (the delete that undoes a "create" does not
   *  enter the stack); refresh=false is for a bulk delete, where the caller runs one
   *  refreshNotes at the end. */
  async deletePath(path: string, record = true, refresh = true) {
    this.discardEditor?.(path);
    await api.pathDelete(path);
    this.pathGone?.(path);
    if (record) this.recordOp({ kind: "delete", path });
    if (refresh) await this.refreshNotes();
    this.pruneHistory((p) => p === path || p.startsWith(path + "/"));
    for (const p of Object.keys(this.expanded))
      if (p === path || p.startsWith(path + "/")) delete this.expanded[p];
    // Every matching tab (a directory prefix included) is closed. It does not go through
    // closeTab's flush — discardEditor has already dropped the pending timer, so a flush
    // here would have nothing to do anyway, and dropTabs' "no flush" semantics happen to
    // be exactly right for a deletion
    await this.dropTabs((t) => t.path === path || t.path.startsWith(path + "/"));
  }

  /** record=false is for the undo's own use (the reverse rename does not enter the stack,
   *  or ⌘Z⌘Z would bounce back and forth on the last step and never reach anything
   *  earlier). */
  async renamePath(from: string, to: string, record = true) {
    // Flush before renaming: what the move on disk carries away is the old content, and a
    // flush afterwards would be stopped by the "the old path is no longer in the index"
    // existence guard, so the pending edit would simply vanish
    await this.flushEditor?.();
    const rel = await api.pathRename(from, to);
    this.pathMoved?.(from, rel);
    if (record) this.recordOp({ kind: "rename", from, to: rel });
    await this.refreshNotes();
    // The old path is rewritten in place in the history (it is not a new visit)
    this.navStack = this.navStack.map((p) =>
      p === from ? rel : p.startsWith(from + "/") ? rel + p.slice(from.length) : p,
    );
    // The expanded set's keys follow the rename too, so the collapsed state is not lost to one
    for (const p of Object.keys(this.expanded)) {
      if (p === from || p.startsWith(from + "/")) {
        delete this.expanded[p];
        this.expanded[rel + p.slice(from.length)] = true;
      }
    }
    // Every tab's path is rewritten in place (the caches are keyed by id and a rename on
    // disk does not change the content, so a background tab's undo survives a rename — on
    // activation the new path is read and the content still equals the cache)
    this.tabs = this.tabs.map((t) =>
      t.path === from
        ? { ...t, path: rel }
        : t.path.startsWith(from + "/")
          ? { ...t, path: rel + t.path.slice(from.length) }
          : t,
    );
    // follow the open document across file or ancestor-folder renames;
    // openFile switches between note and plain-text mode as the extension
    // says (after the rewrite openFile's deduplication matches the active tab itself and takes the read-and-reload branch)
    if (this.currentPath === from) {
      await this.openFile(rel, false);
    } else if (this.currentPath?.startsWith(from + "/")) {
      await this.openFile(rel + this.currentPath.slice(from.length), false);
    }
  }

  private async handleEvent(e: VaultEvent) {
    this.indexVersion++;
    await this.refreshNotes();
    const changed = e.type === "note_modified" || e.type === "note_added" ? e.id : null;
    if (changed && changed === this.currentPath && !this.dirty) {
      // reload externally-updated open note (agent edits land here too)
      try {
        const { content } = await api.noteRead(changed);
        // Unchanged content is not reloaded: the echo of our own save (⌘S or the autosave)
        // comes through here, and reloadCounter++ would trigger resetDoc, throwing the
        // cursor back to 0 and clearing the undo stack
        if (content !== this.currentContent) {
          this.currentContent = content;
          this.reloadCounter++;
        }
      } catch {
        /* removed under us */
      }
    }
    if (e.type === "files_changed" && this.currentPath && fileKind(this.currentPath) === "text") {
      const path = this.currentPath;
      const listed = this.otherFiles.includes(path);
      // Dirty and still in the list (which refreshNotes has just refreshed): it is not
      // reloaded (a conflict is the user's to settle, as with a note) and there is no
      // suspicion of deletion — so the disk is not read. The echo of our own autosave is a
      // files_changed too (file_write broadcasts it explicitly), and reading the whole file
      // back every second while typing only to throw it away is pure waste
      if (!this.dirty || !listed) {
        try {
          const content = await api.fileRead(path);
          // The same semantics as note_modified's reload: an external change lands in a
          // clean buffer — otherwise the autosave that follows would write the stale
          // buffer back over the external edit. The content comparison stops the echo of
          // our own save (so the cursor is not moved and undo is not cleared); the
          // document may have been switched or gone dirty during the await, so it is
          // rechecked
          if (!this.dirty && this.currentPath === path && content !== this.currentContent) {
            this.currentContent = content;
            this.reloadCounter++;
          }
        } catch {
          // files_changed carries no path: a failed read plus absence from the list = an
          // external delete or rename (a hidden path, or one whose case does not match, is
          // not in the list to begin with, and one that reads is left alone). It ends the
          // same way as note_removed — without clearing currentPath, the editor's steady
          // flush branch (which has no existence guard) would write the buffer back and
          // resurrect a dead file; unsaved changes must not be dropped silently, so a toast
          // says so plainly
          if (this.currentPath === path && !this.otherFiles.includes(path)) {
            if (this.dirty) toast.push(t("vaultStore.deletedExternally", { path }));
            this.pruneHistory((p) => p === path);
            await this.dropTabs((t) => t.path === path);
          }
        }
      }
    }
    if (e.type === "types_changed") {
      void types.refresh();
      return;
    }
    if (e.type === "files_changed") {
      // The external-delete check for background text tabs: it only consults the lists
      // (just refreshed in refreshNotes) and does not read each tab from disk —
      // FilesChanged carries no path, and reading N background tabs in full would be pure
      // waste. One missing from the lists is read once to confirm (a hidden path, or one
      // whose case does not match, is not in the lists to begin with, and one that reads is
      // left alone). An image or pdf tab has no buffer and, as before, gets no delete
      // detection.
      // A view tab has to be culled first: a sentinel path has no extension, fileKind would
      // call it "text", and it can never be in the lists → the read would necessarily fail
      // and a background view tab would be silently closed as a dead file
      const suspects = this.tabs.filter(
        (t) => t.id !== this.activeTabId && viewOfTab(t.path) === null
          && fileKind(t.path) === "text" && !this.otherFiles.includes(t.path),
      );
      for (const t of suspects) {
        const alive = await api.fileRead(t.path).then(() => true, () => false);
        if (!alive) await this.dropTabs((x) => x.id === t.id);
      }
    }
    if (e.type === "note_removed") {
      this.pruneHistory((p) => p === e.id);
      await this.dropTabs((t) => t.path === e.id);
    }
  }

  /** The explicit alignment for when the disk's source of truth changed but no event
   *  matched it (the attachment migration rewrites notes through write_note, the watcher's
   *  NoteModified is swallowed by the self-write suppression, and the only broadcast is
   *  IndexReady): a clean buffer rereads what is on disk, and a dirty one is left alone (a
   *  conflict is the user's to settle, the same semantics as handleEvent's reload). Not
   *  rereading would leave a stale buffer — and the next edit or autosave would write the
   *  pre-migration links back verbatim, silently turning them into dead links. */
  async reloadCurrent() {
    const path = this.currentPath;
    if (!path || this.dirty) return;
    const kind = fileKind(path);
    if (kind === "image" || kind === "pdf") return;
    try {
      const content = kind === "note"
        ? (await api.noteRead(path)).content.replace(/\r\n/g, "\n")
        : await api.fileRead(path);
      // The document may have been switched or gone dirty during the await, so it is
      // rechecked; unchanged content is not reloaded (reloadCounter would move the cursor
      // and clear undo)
      if (this.currentPath === path && !this.dirty && content !== this.currentContent) {
        this.currentContent = content;
        this.reloadCounter++;
      }
    } catch {
      /* removed under us — files_changed/note_removed will handle it */
    }
  }

  // ---- the tab mechanism (the lower half of the opening funnel) ----

  /** The deduplicating activation for something already open in another tab. Returns true = handled. */
  private async activateExisting(path: string, record: boolean, pinned: boolean): Promise<boolean> {
    const existing = this.tabs.find((t) => t.path === path);
    if (!existing || existing.id === this.activeTabId) return false;
    if (pinned) existing.pinned = true;
    await this.activateTab(existing.id, record);
    return true;
  }

  /** Placing a tab under the preview semantics. Called only after a successful disk read
   *  (read first, then create, so a failure leaves no dead tab): an open path matches (one
   *  tab per path globally — two tabs holding caches of the same path would overwrite each
   *  other); a single click (pinned=false) reuses the one preview slot; otherwise a new
   *  tab is created to the right of the active one. */
  private placeTab(path: string, pinned: boolean): Tab {
    const existing = this.tabs.find((t) => t.path === path);
    if (existing) {
      if (pinned) existing.pinned = true;
      return existing;
    }
    const preview = this.tabs.find((t) => !t.pinned);
    if (!pinned && preview) {
      // Reuse the preview slot by swapping its path. A preview was never edited (editing
      // promotes it), so its cache holds only the cursor and the scroll and is dropped
      // outright
      this.tabCaches.delete(preview.id);
      preview.path = path;
      return preview;
    }
    const tab: Tab = { id: this.nextTabId++, path, pinned };
    const idx = this.tabs.findIndex((t) => t.id === this.activeTabId);
    const next = [...this.tabs];
    next.splice(idx < 0 ? next.length : idx + 1, 0, tab);
    this.tabs = next;
    return tab;
  }

  /** Suspend the current active tab: its editor snapshot goes into the side table. Every
   *  call site is after a flush (only then does the snapshot's content match the disk and
   *  the lazy validation hold). */
  private stashActive() {
    const id = this.activeTabId;
    if (id == null) return;
    const snap = this.snapshotEditor?.();
    if (snap) {
      this.tabCaches.set(id, snap);
      this.pruneCaches();
    }
  }

  /** The side table holds 20 at most: an EditorState carries the whole document plus its
   *  undo, and dozens of tabs would weigh on memory.
   *  Over the limit the oldest is dropped (a Map's insertion order is the suspension
   *  order), and a dropped one is rebuilt on activation — losing undo, never data. */
  private pruneCaches() {
    for (const key of this.tabCaches.keys()) {
      if (this.tabCaches.size <= 20) break;
      this.tabCaches.delete(key);
    }
  }

  /** The **one** funnel that finishes a landing (openIntoTab, both branches of activateTab
   *  and openView all come through here): suspend the old active tab → clear that tab's
   *  stale snapshot → move the active slot onto the tab.
   *  The order is part of the invariant: the stash has to precede the delete, or the
   *  snapshot just taken would delete itself. The payload comes in two kinds, differing
   *  only in that a document needs path/reloadCounter/landed while a view needs mainView
   *  lined up; `pendingRestore` is settled by the caller before coming in — activateTab
   *  has to pass the validated cached snapshot down, and it must not be cleared here. */
  private settleTab(tab: Tab, payload: { view: ProjectionView } | { content: string; record: boolean }) {
    if (tab.id !== this.activeTabId) this.stashActive();
    this.tabCaches.delete(tab.id);
    this.activeTabId = tab.id;
    this.dirty = false;
    if ("view" in payload) {
      // A view tab: no buffer, no disk read, and it does not enter navStack
      this.currentPath = null;
      this.currentContent = "";
      this.mainView = payload.view;
      return;
    }
    this.currentPath = tab.path;
    this.currentContent = payload.content;
    this.reloadCounter++;
    this.landed(tab.path, payload.record);
  }

  /** Clear the active slot. Closing or deleting a tab **has to** call it before
   *  activating the neighbour — otherwise the neighbour's stashActive would put the
   *  snapshot of the document just closed back into the side table. */
  private clearActive() {
    this.activeTabId = null;
    this.currentPath = null;
    this.currentContent = "";
    this.dirty = false;
  }

  /** Finishing a landing after a successful disk read: place the tab, suspend the old active tab, and load the new content. */
  private openIntoTab(path: string, content: string, record: boolean, pinned: boolean) {
    this.pendingRestore = null;
    this.settleTab(this.placeTab(path, pinned), { content, record });
  }

  /** Activate an existing tab (the core path of switching tabs, and one of the flush
   *  funnel's call sites).
   *  Read the disk → compare against the cached snapshot: equal → pendingRestore reuses it
   *  (undo, cursor and scroll survive); different (changed externally, by the agent or by a
   *  script) → rebuild. A failed read means a dead path, and closing the tab handles it
   *  automatically. */
  async activateTab(id: number, record = true) {
    if (id === this.activeTabId) return;
    const tab = this.tabs.find((t) => t.id === id);
    if (!tab) return;
    await this.flushEditor?.();
    // A view tab: no buffer and no disk read, so the current document is suspended and the
    // view is shown outright. It does not go through landed (a view does not enter
    // navStack, so record means nothing), and mainView is lined up with the active tab here
    const view = viewOfTab(tab.path);
    if (view) {
      this.pendingRestore = null;
      this.settleTab(tab, { view });
      return;
    }
    const kind = fileKind(tab.path);
    let content = "";
    if (kind === "image" || kind === "pdf") {
      this.pendingRestore = null;
    } else {
      try {
        // A note is normalised to LF as it is read back: a note's docEol is hardcoded "\n"
        // (CM normalises the same way), and without normalising, a CRLF note written by an
        // external tool would never match the snapshot comparison below — so undo, the
        // cursor and the scroll would be silently rebuilt on every tab switch. A text file
        // is kept as it is (docEol is decided from the content, so the snapshot and the disk
        // naturally take the same shape)
        content = kind === "note"
          ? (await api.noteRead(tab.path)).content.replace(/\r\n/g, "\n")
          : await api.fileRead(tab.path);
      } catch {
        toast.push(t("vaultStore.unreadableTabClosed", { path: tab.path }));
        await this.closeTab(id);
        return;
      }
      const cache = this.tabCaches.get(id);
      this.pendingRestore = cache && cache.content === content ? cache : null;
    }
    this.settleTab(tab, { content, record });
  }

  /** Close one tab. The active tab: flush first (a pending timer is itself a dirty signal)
   *  and then activate the neighbour to the right (or to the left when there is none), until
   *  it closes to empty; a background tab is simply removed. */
  async closeTab(id: number) {
    const idx = this.tabs.findIndex((t) => t.id === id);
    if (idx < 0) return;
    if (id !== this.activeTabId) {
      this.tabs = this.tabs.filter((t) => t.id !== id);
      this.tabCaches.delete(id);
      return;
    }
    await this.flushEditor?.();
    const wasReading = this.mainView === "reading";
    const rest = this.tabs.filter((t) => t.id !== id);
    this.tabs = rest;
    this.tabCaches.delete(id);
    this.clearActive();
    const next = rest[Math.min(idx, rest.length - 1)];
    if (next) {
      await this.activateTab(next.id, false);
      // reading does not hang off a tab, and closing the tab underneath should not leave
      // the reading view along the way (the same as the close-to-empty branch) — but
      // activating the neighbour goes through settleTab/landed, which writes mainView
      // directly and covers the overlay state. So it is restored (without recording history
      // again), and where it came from is updated to the view the neighbour landed on
      if (wasReading) this.enterReading(false);
    }
    // Closing to empty: when what was closed was a view tab, mainView still sits on that
    // view, so it is pushed back to the editor's empty state
    // (reading is the exception — it does not hang off a tab, and closing the document
    // underneath should not leave the reading view along the way)
    else if (this.mainView !== "editor" && this.mainView !== "reading") this.mainView = "editor";
  }

  async closeOtherTabs(id: number) {
    if (this.activeTabId !== id) await this.activateTab(id);
    if (this.activeTabId !== id) return; // the activation failed (a dead path has been closed)
    this.tabs = this.tabs.filter((t) => t.id === id);
    for (const key of [...this.tabCaches.keys()]) if (key !== id) this.tabCaches.delete(key);
  }

  async closeTabsToRight(id: number) {
    const idx = this.tabs.findIndex((t) => t.id === id);
    if (idx < 0) return;
    const gone = this.tabs.slice(idx + 1);
    if (gone.some((t) => t.id === this.activeTabId)) {
      await this.activateTab(id);
      if (this.activeTabId !== id) return;
    }
    this.tabs = this.tabs.slice(0, idx + 1);
    for (const t of gone) this.tabCaches.delete(t.id);
  }

  /** The sequential cycling of ctrl+Tab / ⌘⇧[]. */
  async cycleTab(delta: 1 | -1) {
    const n = this.tabs.length;
    if (n < 2) return;
    const idx = this.tabs.findIndex((t) => t.id === this.activeTabId);
    await this.activateTab(this.tabs[(idx + delta + n) % n].id);
  }

  /** Drag-reordering in the tab bar (the id-keyed each guarantees the DOM is not rebuilt). */
  moveTab(id: number, toIndex: number) {
    const from = this.tabs.findIndex((t) => t.id === id);
    if (from < 0) return;
    const next = [...this.tabs];
    const [t] = next.splice(from, 1);
    next.splice(Math.max(0, Math.min(toIndex, next.length)), 0, t);
    this.tabs = next;
  }

  /** The set of open paths behind the file tree's faint highlight. It is a $derived rather
   *  than a some() per row: every row of the file tree reads it and tabs changes once per
   *  frame while dragging — an O(rows × tabs) linear scan would degrade, so a Set is
   *  rebuilt whenever tabs changes and each row is O(1). */
  readonly openPaths: Set<string> = $derived(new Set(this.tabs.map((t) => t.path)));

  /** The file tree's faint highlight: this path is open in some tab. */
  isOpen(path: string): boolean {
    return this.openPaths.has(path);
  }

  /** Promote a preview: on the first real edit (Editor's onDirty, with resetDoc's false
   *  signal already stopped by the loadingDoc gate), a double click on the tab title, or a
   *  double click in the file tree. */
  pinActiveTab() {
    const t = this.tabs.find((t) => t.id === this.activeTabId);
    if (t && !t.pinned) t.pinned = true;
  }

  /** How a tab ends on an external deletion: **no flush** (which would write the buffer
   *  back and resurrect a dead file) and no cache kept; when the active tab is among them,
   *  a surviving neighbour is activated. */
  private async dropTabs(hit: (t: Tab) => boolean) {
    const gone = this.tabs.filter(hit);
    if (!gone.length) return;
    const activeGone = gone.some((t) => t.id === this.activeTabId);
    const idx = this.tabs.findIndex((t) => t.id === this.activeTabId);
    this.tabs = this.tabs.filter((t) => !hit(t));
    for (const t of gone) this.tabCaches.delete(t.id);
    if (!activeGone) return;
    const wasReading = this.mainView === "reading";
    this.clearActive();
    const next = this.tabs[Math.min(Math.max(idx, 0), this.tabs.length - 1)];
    if (next) {
      await this.activateTab(next.id, false);
      // The same as closeTab: the reading overlay does not leave because the tab underneath was deleted from outside
      if (wasReading) this.enterReading(false);
    }
  }

  /** Open a projection view (from the rail or the command palette): it opens into the tab
   *  system. The single-click semantics match a document's — an open one is activated,
   *  otherwise it lands in the one preview slot (which the next open can replace, and a
   *  double click or the context menu promotes); there is no disk read, and the ending
   *  mirrors openIntoTab (when placeTab reuses the active preview slot the id is the same
   *  and nothing is stashed — the document it replaced has already been flushed, and its
   *  snapshot has nowhere to belong and no need to). */
  async openView(view: ProjectionView) {
    const path = VIEW_TAB_PREFIX + view;
    const existing = this.tabs.find((t) => t.path === path);
    if (existing) {
      if (existing.id !== this.activeTabId) await this.activateTab(existing.id);
      else if (this.mainView !== view) {
        // The tab is active but not on screen (covered by reading, or the residue of
        // mainView having come apart): pressing the rail or ⌘P again says "show me this" —
        // the same lining-up as revealActiveTab, with the reading projection going through
        // exitReading first to get its timeline lined up
        if (this.mainView === "reading") this.exitReading();
        this.mainView = view;
      }
      return;
    }
    await this.flushEditor?.();
    this.pendingRestore = null;
    this.settleTab(this.placeTab(path, false), { view });
  }

  async openNote(path: string, record = true, pinned = false) {
    // Already open in another tab → activate it (activateTab does its own disk read and cache validation)
    if (await this.activateExisting(path, record, pinned)) return;
    // Flush before reading the disk: only then does reopening the current note avoid
    // resetting the buffer with stale disk content and throwing input from the last second
    // into an echo race
    await this.flushEditor?.();
    try {
      const { id, content } = await api.noteRead(path);
      this.openIntoTab(id, content, record, pinned);
    } catch (e) {
      this.error = String(e);
    }
  }

  /** Open any vault file: notes in the note editor, images/PDF in the in-app
   *  preview, text files in a plain editor, and remaining binaries
   *  (non-UTF-8 / oversized) with the OS default app. */
  async openFile(path: string, record = true, pinned = false) {
    const kind = fileKind(path);
    if (kind === "note") {
      await this.openNote(path, record, pinned);
      return;
    }
    if (await this.activateExisting(path, record, pinned)) return;
    await this.flushEditor?.();
    if (kind === "image" || kind === "pdf") {
      // the preview component loads the bytes itself (it stays reusable for
      // a future split view: one instance per pane, keyed by path);
      // no edit buffer → no snapshot, so the blob is reread on every activation, exactly as switching documents does
      this.openIntoTab(path, "", record, pinned);
      return;
    }
    try {
      const content = await api.fileRead(path);
      this.openIntoTab(path, content, record, pinned);
    } catch {
      try {
        await api.fileOpenSystem(path);
      } catch (e) {
        this.error = String(e);
      }
    }
  }

  /** An explicit "open in a new tab" (⌘click, the middle button, the tab context menu):
   *  it opens a pinned tab directly and does not take the preview slot. */
  async openInNewTab(path: string, record = true) {
    await this.openFile(path, record, true);
  }

  /** Write any document path to disk, toasting a failure. The autosave flushing an old document comes through here too. */
  async writeDoc(path: string, content: string): Promise<boolean> {
    try {
      if (path.endsWith(".md")) {
        await api.noteWrite(path, content);
      } else {
        await api.fileWrite(path, content);
      }
      return true;
    } catch (e) {
      this.error = String(e);
      return false;
    }
  }

  async saveCurrent(content: string) {
    const path = this.currentPath;
    if (!path) return;
    const seq = this.editSeq;
    const ok = await this.writeDoc(path, content);
    // The document may have been switched during the await; on failure dirty stays and the next input or ⌘S retries
    if (ok && this.currentPath === path) {
      this.currentContent = content;
      // More input arrived while writing (editSeq changed): the buffer still differs from
      // the disk, so dirty is not cleared — otherwise guards that rely on dirty, such as
      // refusePrivacyToggleWhileDirty, would miss it
      if (this.editSeq === seq) this.dirty = false;
    }
  }

  async createAndOpen(title: string) {
    const id = await api.noteCreate(title);
    this.recordOp({ kind: "create", paths: [id] });
    await this.refreshNotes();
    await this.openNote(id, true, true); // a newly created file gets a pinned tab
  }

  /** Follow a wikilink target: open if resolved, create if not. A fragment lands on the
   *  block (with a flash); guard is a positional citation's content when it was cited. */
  async followLink(target: string, opts: { guard?: string } = {}) {
    const hash = target.indexOf("#");
    const notePart = hash >= 0 ? target.slice(0, hash).trim() : target.trim();
    const fragment = hash >= 0 ? target.slice(hash + 1).trim() : "";
    if (POSITIONAL_RE.test(fragment)) {
      // `Note#bN`: a positional id from an AI answer — the index knows it, resolve_ref does not
      const note = notePart ? await api.resolveLink(notePart, this.currentPath ?? undefined) : this.currentPath;
      if (!note) {
        toast.push(t("vaultStore.fragmentMissing", { path: notePart, fragment }));
        return;
      }
      const hit = findBlock(await blocksOf(note, this.indexVersion), fragment, opts.guard);
      if (hit) {
        await this.openBlockAt(note, hit.block.span.start, hit.block.content);
        return;
      }
      await this.openNote(note);
      toast.push(t("vaultStore.fragmentMissing", { path: note, fragment }));
      return;
    }
    if (fragment) {
      // `Note#Heading` / `Note#^id` / `#^id`: land on the block, not at position 0
      const r = await api.resolveRef(target, this.currentPath ?? undefined);
      if (r.state === "resolved") {
        await this.openBlockAt(r.note, r.span.start, r.content);
        return;
      }
      if (r.state === "anchor_missing") {
        await this.openNote(r.note);
        toast.push(t("vaultStore.fragmentMissing", { path: r.note, fragment }));
        return;
      }
      // note missing: fall through with the note part alone
    }
    const resolved = notePart ? await api.resolveLink(notePart, this.currentPath ?? undefined) : null;
    if (resolved) {
      await this.openNote(resolved);
      return;
    }
    const clean = notePart;
    if (!clean) return;
    // [[attachment.png]] / ⌘-clicking ![[img.png]]: when the target is a non-note file that
    // already exists in the vault, it is opened — falling into "unresolved, so create it"
    // would conjure up a junk note called img.png.md. The resolution rule matches an image
    // embed's (an exact path match, and a bare file name takes the shortest path)
    const lower = clean.toLowerCase();
    const file =
      this.otherFiles.find((f) => f.toLowerCase() === lower) ??
      (clean.includes("/")
        ? undefined
        : this.otherFiles
            .filter((f) => (f.split("/").pop() ?? "").toLowerCase() === lower)
            .sort((a, b) => a.length - b.length || a.localeCompare(b))[0]);
    if (file) {
      await this.openFile(file);
      return;
    }
    await this.createAndOpen(clean);
    // Clicking an unresolved link creates a note — a misclick is more likely now that a
    // single click opens links, so this gives a receipt the user can notice
    toast.push(t("vaultStore.created", { path: clean }));
  }

  /** Attachment bucket for the open note: attachments/<note stem>. */
  get noteBucket(): string {
    return noteBucketOf(this.currentPath);
  }

  /** Tree paste — position semantics: everything lands in `folder` ("" =
   *  vault root); vault-internal sources get duplicated (explicit copy).
   *  A paste inside the editor (pasteIntoNote) does not enter the undo stack: an imported
   *  attachment and the link to it in the body are a pair, undoing the file alone would
   *  leave a dead link, and undoing the body is CodeMirror's business. */
  async pasteIntoFolder(folder: string, files: File[] = []): Promise<string[]> {
    const saved = await this.pasteClipboard(folder, files, false);
    if (saved.length) this.recordOp({ kind: "create", paths: saved });
    return saved;
  }

  /** Editor paste — reference semantics: external files copy into
   *  attachments/<note>/, files already in the vault are only referenced,
   *  screenshots save as pasted-<stamp>. Returns paths to link in the text. */
  async pasteIntoNote(files: File[] = [], bucket = this.noteBucket): Promise<string[]> {
    return this.pasteClipboard(bucket, files, true);
  }

  /** Import files picked from the native file dialog into `dest` (the agent composer's "+"
   *  attachment): one already inside the vault is referenced rather than copied (the same
   *  semantics as pasting into a note). Returns vault-relative paths. */
  async importFiles(paths: string[], dest: string): Promise<string[]> {
    try {
      const saved = await api.filesImport(paths, dest);
      if (saved.length) await this.refreshNotes();
      return saved;
    } catch (e) {
      this.error = String(e);
      return [];
    }
  }

  /** The native side handles real files (Finder copies — fs copy, original
   *  names, any size) and raw image bytes (screenshots); `files` from the DOM
   *  paste event is the fallback for platforms without a native reader. */
  private async pasteClipboard(folder: string, files: File[], reuseVaultFiles: boolean): Promise<string[]> {
    try {
      const saved = await api.clipboardPaste(folder, pasteStamp(), reuseVaultFiles);
      if (!saved.length) {
        for (const file of files) {
          // generic names ("image.png", empty) come from raw-bytes pastes —
          // stamp those; real filenames are kept and deduped by the backend
          const generic = !file.name || /^image\.\w+$/i.test(file.name);
          const name = generic ? `${pasteStamp()}.${extOf(file.type)}` : file.name.replace(/[/\\]/g, " ").trim();
          const rel = folder ? `${folder}/${name}` : name;
          if (rel.endsWith(".md")) {
            const taken = new Set([...this.notes.map((n) => n.id), ...this.otherFiles]);
            let target = rel;
            for (let n = 1; taken.has(target); n++) target = `${rel.slice(0, -3)}-${n}.md`;
            await api.noteWrite(target, await file.text());
            saved.push(target);
          } else {
            const bytes = new Uint8Array(await file.arrayBuffer());
            saved.push(await api.attachmentSave(bytes, rel));
          }
        }
      }
      if (saved.length) await this.refreshNotes();
      return saved;
    } catch (e) {
      this.error = String(e);
      return [];
    }
  }

  /** The destination dispatch shared by clipping and quick memos. When the target is
   *  exactly the note that is open it has to go through the editor buffer (with a heading
   *  given, the section wins over the cursor, and this holds for a CSS-hidden editor too) —
   *  a disk write would be overwritten by the stale buffer's autosave; when it cannot be
   *  inserted (the editor is switching or unmounting) it throws rather than writing, since
   *  not landing is better than losing it silently. Any other note is not being edited and
   *  goes through note_insert (which means "create and write" for a note that does not
   *  exist). When a heading was given but the section is missing, it falls back to the end
   *  of the file and toasts about it here, once, rather than silently. */
  async insertBlock(
    target: string,
    text: string,
    heading: string | null,
  ): Promise<{ intoEditor: boolean; landed: "section" | "tail" }> {
    if (target === this.currentPath) {
      const landed = heading ? this.insertAtSection?.(heading, text) : this.insertAtCursor?.(text, true);
      if (!landed) throw new Error(t("vaultStore.insertFailedSwitching", { path: target }));
      if (heading && landed === "tail") toast.push(t("vaultStore.headingMissingTail", { heading }));
      return { intoEditor: true, landed: landed === "tail" ? "tail" : "section" };
    }
    const landed = await api.noteInsert(target, text, heading ?? undefined);
    if (heading && landed === "tail") toast.push(t("vaultStore.headingMissingTailIn", { heading, path: target }));
    return { intoEditor: false, landed };
  }

  /** Capture one quick memo: a list item with a timestamp, inserted into target (inbox.md
   *  by default); the destination dispatch and the section-fallback message live in
   *  insertBlock. */
  /** Accept one pending AI block (a local id) or, with null, every pending block of the
   *  current note. The buffer flushes first: the ledger is flipped by the hash of the
   *  block as it is on disk, and a stale disk copy would accept the wrong text. Returns
   *  how many blocks were accepted. */
  async acceptProvenance(localId: string | null): Promise<number> {
    const path = this.currentPath;
    if (!path) return 0;
    if (this.dirty) await this.flushEditor?.();
    const n = await api.provenanceAccept(path, localId ?? undefined);
    this.provenanceEpoch++;
    return n;
  }

  async capture(text: string, target = "inbox.md", heading: string | null = null) {
    // Local time (toISOString is UTC, which is hours out east of Greenwich); the format has
    // to stay `YYYY-MM-DD HH:MM` — views/tasks.ts's STAMP_RE strips the signature by it
    const t = new Date();
    const pad = (n: number) => String(n).padStart(2, "0");
    const line = `- ${text} *(${ymd(t)} ${pad(t.getHours())}:${pad(t.getMinutes())})*`;
    const r = await this.insertBlock(target, line, heading);
    // The stamp alone would already name the origin; the ledger line adds the exact time and survives a stamp the user trims
    void api.provenanceRecord(target, line, "capture").catch(() => undefined);
    if (!r.intoEditor) await this.refreshNotes();
    // landed is passed through to the caller: on a section fallback the success receipt must not still claim it landed in the section
    return r;
  }
}

/** A path's last segment (the file name). The chip labels, the @ candidate rows and the attachment routing all care about this alone. */
export function baseOf(path: string): string {
  return path.slice(path.lastIndexOf("/") + 1);
}

/** A path's last segment without the .md — the rule for "a note's display name". The file
 *  tree's rows, the tab titles and the kanban's destination button all read this one, so
 *  three places do not each write a slice/replace and slowly drift apart. */
export function stemOf(path: string): string {
  const b = baseOf(path);
  return b.endsWith(".md") ? b.slice(0, -3) : b;
}

/** The display name of a vault's root path. A Windows path uses backslashes;
 *  filter(Boolean) copes with a drive-root vault such as "D:\". The FileTree switcher and
 *  the status line share this one. */
export function vaultName(root: string | null): string {
  return root?.split(/[\\/]/).filter(Boolean).pop() ?? root ?? "";
}

/** The directory a path is in (an empty string at the root). */
export function parentOf(path: string): string {
  const i = path.lastIndexOf("/");
  return i < 0 ? "" : path.slice(0, i);
}

/** A note's attachment bucket: attachments/<the note's relative path minus .md> (a note's
 *  path is unique → so is the bucket, and notes sharing a stem no longer share one; a
 *  root-level note keeps the same path as under the old rule). With no note it is
 *  attachments. In the detached chat window vault.currentPath is always null and the
 *  current note comes from agent.hostNote — which is why this is a pure function. Rust's
 *  attach_migrate::bucket_for mirrors it, and existing data is migrated by the
 *  migrate-attachments command. */
export function noteBucketOf(notePath: string | null): string {
  const rel = (notePath ?? "").replace(/\.md$/, "");
  return rel ? `attachments/${rel}` : "attachments";
}

/** `pasted-YYYYMMDD-HHMMSS` name for clipboard content with no filename. */
function pasteStamp(): string {
  const t = new Date();
  const pad = (n: number) => String(n).padStart(2, "0");
  return `pasted-${t.getFullYear()}${pad(t.getMonth() + 1)}${pad(t.getDate())}-${pad(t.getHours())}${pad(t.getMinutes())}${pad(t.getSeconds())}`;
}

/** File extension for a clipboard MIME type. */
function extOf(mime: string): string {
  return (mime.split("/")[1] ?? "png").replace("jpeg", "jpg").replace("svg+xml", "svg");
}

// tiff: WebKit renders it natively, and clipboard pastes can produce it
const IMAGE_EXTS = new Set(["png", "jpg", "jpeg", "gif", "webp", "svg", "bmp", "ico", "avif", "tiff", "tif"]);

const MIME: Record<string, string> = {
  png: "image/png",
  jpg: "image/jpeg",
  jpeg: "image/jpeg",
  gif: "image/gif",
  webp: "image/webp",
  svg: "image/svg+xml",
  bmp: "image/bmp",
  ico: "image/x-icon",
  avif: "image/avif",
  tiff: "image/tiff",
  tif: "image/tiff",
  pdf: "application/pdf",
};

/** MIME type for a vault path, by extension. */
export function mimeOf(path: string): string {
  const ext = (path.split(".").pop() ?? "").toLowerCase();
  return MIME[ext] ?? "application/octet-stream";
}

/** Classify a vault path by how it should open. */
export function fileKind(path: string): "note" | "text" | "image" | "pdf" {
  if (path.endsWith(".md")) return "note";
  const ext = (path.split(".").pop() ?? "").toLowerCase();
  if (IMAGE_EXTS.has(ext)) return "image";
  if (ext === "pdf") return "pdf";
  return "text";
}

export const vault = new VaultStore();

// The UI state is written through to localStorage (a module-level $effect needs its own root)
$effect.root(() => {
  $effect(() => {
    // Nothing is persisted while a sash is dragged (after the early return this effect
    // depends on resizing alone, so a pointermove changing the width no longer triggers
    // it); back at null it reruns and writes the final width
    if (vault.resizing) return;
    const snapshot = JSON.stringify({
      leftOpen: vault.leftOpen, rightOpen: vault.rightOpen, rightTab: vault.rightTab,
      // stringify drops undefined — "the default width / the default side" is not persisted
      leftW: vault.leftWidth ?? undefined, rightW: vault.rightWidth ?? undefined,
      swap: vault.swapSides || undefined,
      blOpen: vault.backlinksOpen || undefined, outOpen: vault.outgoingOpen || undefined,
    });
    try {
      localStorage.setItem(UI_KEY, snapshot);
    } catch {
      /* storage unavailable */
    }
  });
  // The file tree's expanded set is written through per vault. keys has to be read before
  // the root null check, to register the dependency.
  // recent is only the trimming allowlist, and untrack keeps every loadRecent from
  // rewriting the storage; while it is not loaded null is passed (nothing is trimmed),
  // which stops a startup race from wrongly deleting another vault's bucket
  $effect(() => {
    const root = vault.root;
    const keys = Object.keys(vault.expanded);
    if (!root) return;
    saveBucket(TREE_KEY, root, keys, untrack(() => (vault.recentLoaded ? vault.recent : null)));
  });
  // The editor tabs are written through per vault. The restore happens in adoptRoot in the
  // same batch as root (the same invariant); the dependencies are the tabs' path and
  // pinned plus the active index
  $effect(() => {
    const root = vault.root;
    const tabs = vault.tabs.map((t) => ({ path: t.path, pinned: t.pinned }));
    const active = vault.tabs.findIndex((t) => t.id === vault.activeTabId);
    if (!root) return;
    saveBucket(TABS_KEY, root, { tabs, active }, untrack(() => (vault.recentLoaded ? vault.recent : null)));
  });
});
