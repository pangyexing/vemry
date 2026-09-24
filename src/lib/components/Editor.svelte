<script lang="ts">
  import { t, compareNames } from "$lib/i18n/i18n.svelte";
  import { onDestroy } from "svelte";
  import { listen } from "@tauri-apps/api/event";
  import { EditorView } from "@codemirror/view";
  import { type EditorState, Text, Transaction } from "@codemirror/state";
  import { createEditor, resetDoc, languageFor } from "$lib/editor/setup";
  import { applyLangAttrs, noteLanguage } from "$lib/notelang";
  import { foldMarkedIn, foldOnLoad } from "$lib/editor/fold";
  import { type EditorBlock, blockAt, blockContentStart, extentTo } from "$lib/editor/blocks";
  import { blockMenuEntries, type BlockMenuHost } from "$lib/editor/blockmenu";
  import { selectedBlocks } from "$lib/editor/blocksel";
  import { focusEditor } from "$lib/editor/clickguard";
  import { deleteBlocks, duplicateBlocks, moveBlocks } from "$lib/editor/blockops";
  import type { BlockHost } from "$lib/editor/blockhandle";
  import { startBlockDrag } from "$lib/editor/blockdrag";
  import { byteSpansToChar, byteToChar, charToByte } from "$lib/bytes";
  import { copyText } from "$lib/copytext";
  import { blocksOf } from "$lib/citation";
  import { privateRangeAt, selectionTouchesPrivate } from "$lib/editor/privateblock";
  import { queryRefresh } from "$lib/editor/queryblock";
  import { suppressReveal } from "$lib/editor/reveal";
  import { ensureSyntaxTree } from "@codemirror/language";
  import { fencedCodeAt } from "$lib/editor/fenced";
  import { linkAt } from "$lib/editor/linkat";
  import { vault, fileKind, mimeOf, isMetaPath, metaDocExists } from "$lib/stores/vault.svelte";
  import { agent } from "$lib/stores/agent.svelte";
  import { prefs } from "$lib/stores/prefs.svelte";
  import { toast } from "$lib/stores/toast.svelte";
  import { type TaskState, api, SELECTION_MAX_CHARS, trimLoneSurrogate, type BlockHit, type BlockQuery, type BlockSelector } from "$lib/api";
  import { planInsert, planCursorInsert, type InsertPlan } from "$lib/insert";
  import { extractOutline } from "$lib/outline";
  import { countWords } from "$lib/stats";
  import { k } from "$lib/shortcut";
  import { executeCommand } from "$lib/commands/registry";
  import { privacyMenuItem } from "$lib/privacyMenu";
  import FilePreview from "./FilePreview.svelte";
  import { contextMenu, type MenuEntry } from "$lib/stores/contextMenu.svelte";
  import { clearImageDims } from "$lib/editor/livepreview";
  import { clearEmbedCaches, type EmbedHost } from "$lib/editor/embed";
  import {
    isPending,
    originAt,
    provenanceMarks as provMarksEffect,
    provenanceOverlay as provOverlayEffect,
    provenanceSet,
    pendingRuns,
    runAt,
    stepRun,
    type ReviewStep,
  } from "$lib/editor/provenance";
  import { flashRange } from "$lib/editor/flash";
  import { declaredField, fieldValueKind, scanFields, setFieldValue, stateField, stateStepOf } from "$lib/fields";
  import { DATE_FIELDS, rescheduleLine, TASK_LINE_RE, type DateField, updateTask } from "$lib/views/tasks";
  import { commitTaskWrite } from "$lib/views/taskwrite";
  import { writeBlockField } from "$lib/views/fieldwrite";
  import type { RowChipClick } from "$lib/editor/queryblock";
  import TaskDateMenu from "./views/TaskDateMenu.svelte";
  import FieldValueMenu from "./FieldValueMenu.svelte";
  import { types } from "$lib/stores/types.svelte";
  import type { FieldStat, LabParams, RevertPlan } from "$lib/api";
  import "katex/dist/katex.min.css";

  let container = $state<HTMLDivElement | null>(null);
  let view: EditorView | null = null;
  let knownTargets = new Set<string>();
  let targetList: string[] = [];
  let tagList: string[] = [];
  /** Inline-field keys and values across the vault (for `key::` completion and a chip's
   *  value menu). Read on demand — the first `::` after an index update fetches, the rest
   *  of that version reuse the list — rather than on every index update, most of which
   *  happen while no field is being typed. */
  let fieldStats: FieldStat[] = [];
  let fieldStatsVersion = -1;
  let fieldStatsPending: Promise<FieldStat[]> | null = null;
  function currentFieldStats(): FieldStat[] | Promise<FieldStat[]> {
    const version = vault.indexVersion;
    if (version === fieldStatsVersion) return fieldStats;
    if (!fieldStatsPending) {
      fieldStatsPending = api
        .fieldsList()
        .then((list) => {
          fieldStats = list;
          fieldStatsVersion = version;
          return list;
        })
        .catch(() => fieldStats) // the stale list is better than none
        .finally(() => (fieldStatsPending = null));
    }
    return fieldStatsPending;
  }
  /** The date menu raised by a chip: which line and key it edits. */
  let fieldMenu = $state<{ x: number; y: number; pos: number; key: string; current?: string; field: DateField; keyboard?: boolean } | null>(null);
  /** The value menu raised by any other chip: the candidates (the block types' declared
   *  values first, then the vault's), whether the input is numeric, and the vocabulary
   *  when the key is a state field. `pos` is the chip's line in this editor; a chip on a
   *  query row carries its `hit` instead, and the pick is written to that block's note. */
  let valueMenu = $state<{
    x: number;
    y: number;
    pos: number;
    hit?: BlockHit;
    key: string;
    current: string;
    values: string[];
    number: boolean;
    state: string[] | null;
    keyboard?: boolean;
  } | null>(null);
  /** How many candidate values the menu lists at most. */
  const MENU_VALUES_MAX = 12;

  /** A menu's pick lands on the line: a task's due/scheduled goes through rescheduleLine
   *  (it knows the emoji spellings); any other field is rewritten in place by
   *  setFieldValue, at the clicked chip (null removes it). An empty placeholder of a
   *  date key (`[due:: ]`, from a block type) is filled in place too, and so is a bare
   *  `due:: 2026-08-20` — a field to the parser, not the task's date: the task-date
   *  rewriter cannot see either and would leave it behind next to a fresh 📅. */
  function writeField(m: { pos: number; key: string }, value: string | null) {
    if (!view) return;
    const line = view.state.doc.lineAt(Math.min(m.pos, view.state.doc.length));
    const lower = m.key.toLowerCase();
    const marks = scanFields(line.text, 0, { empty: true });
    // the placeholder test is deliberately position-free: the `@due` flow raises the menu
    // at the marker it just typed, not at the placeholder it is about to fill
    const placeholder = marks.some((f) => f.value === "" && f.key.toLowerCase() === lower);
    const clicked = marks.find((f) => f.key.toLowerCase() === lower && f.from === m.pos - line.from);
    const isTaskDate = !placeholder && clicked?.form !== "bare" && TASK_LINE_RE.test(line.text) && m.key in DATE_FIELDS;
    const next = isTaskDate ? rescheduleLine(line.text, m.key as DateField, value) : setFieldValue(line.text, m.key, value, m.pos - line.from);
    focusEditor(view, line.from);
    if (next === line.text) return;
    view.dispatch({ changes: { from: line.from, to: line.to, insert: next }, userEvent: "input.field" });
  }
  /** The value menu's candidates for a key: the declared values first, then the vault's,
   *  deduplicated and capped; numeric when the type says so or the value is a number; and
   *  the vocabulary when the key is a state field. Shared by an editor chip and a query
   *  row's chip, so the two menus cannot disagree. */
  async function valueMenuFor(key: string, value: string, at: { x: number; y: number }) {
    const spec = declaredField(types.enabled, key);
    const seen = (await currentFieldStats()).find((s) => s.key.toLowerCase() === key.toLowerCase())?.values ?? [];
    const values = [...new Set([...spec.values.map((v) => v.value), ...seen])].slice(0, MENU_VALUES_MAX);
    const number = spec.kind === "number" || (value !== "" && fieldValueKind(value) === "number");
    return { x: at.x, y: at.y, key, current: value, values, number, state: stateField(types.enabled, key) };
  }
  /** A query row's chip pick, written to the row's block in its own note (the task
   *  views' funnel: the dirty guard, a stale span requeries). The index refresh that
   *  follows the write re-runs every query block on its own. */
  const writeRowField = (hit: BlockHit, key: string, value: string | null) =>
    commitTaskWrite(hit, t("queryBlock.act.setField"), () => writeBlockField(hit, key, value), requeryBlocks);
  /** A stale span: the query blocks re-run now rather than at the next index refresh
   *  (a fresh key, distinct from any index version, so the widgets do rebuild). */
  function requeryBlocks() {
    view?.dispatch({ effects: queryRefresh.of(-Date.now()) });
  }
  function applyFieldDate(date: string | null) {
    const m = fieldMenu;
    fieldMenu = null;
    if (m) writeField(m, date);
  }
  /** A chip menu closed without a pick: a menu that had taken the keyboard gives the
   *  focus back to the editor at the line it was raised on, so typing goes on there. A
   *  menu raised from a query row edits another note and has no line here. */
  function returnFocus(m: { keyboard?: boolean; pos: number; hit?: BlockHit } | null) {
    if (m?.keyboard && !m.hit && view && !view.hasFocus) focusEditor(view, Math.min(m.pos, view.state.doc.length));
  }
  function closeFieldMenu() {
    const m = fieldMenu;
    fieldMenu = null;
    returnFocus(m);
  }
  function closeValueMenu() {
    const m = valueMenu;
    valueMenu = null;
    returnFocus(m);
  }
  function applyFieldValue(value: string | null) {
    const m = valueMenu;
    valueMenu = null;
    if (!m) return;
    if (m.hit) void writeRowField(m.hit, m.key, value);
    else writeField(m, value);
  }
  let loadedFor = -1;

  // ---- Autosave: it writes to disk 1s after typing pauses. Notes and plain text files share
  // it — the old "only save .md" was because file_write had no self-write suppression on the
  // Rust side, and once the script writes gained it (vault.rs's self_file_writes) that was
  // lifted: a write now leaves only file_write's own explicit broadcast, exactly like a note's
  // NoteModified, with no second shot from a watcher echo. ----
  const AUTOSAVE_MS = 1000;
  let autosaveTimer: ReturnType<typeof setTimeout> | null = null;
  /** The document path the view currently holds (vault.currentPath changes first on a switch and cannot be relied on) */
  let docPath: string | null = null;
  /** Line-ending fidelity for non-md files: CM is always \n internally, so a CRLF is
   *  remembered on load and restored before writing — otherwise the autosave would silently
   *  normalise a whole .csv or .bat to LF (in the ⌘S era that was a deliberate user action;
   *  once automatic it has to be faithful). Mixed line endings are unified to CRLF, since
   *  determinism beats per-line fidelity; .md does not do this: the insertion and destination
   *  logic all count positions in \n, and normalising a note's line endings is the existing
   *  behaviour. */
  let docEol: "\n" | "\r\n" = "\n";
  const withEol = (text: string) => (docEol === "\r\n" ? text.replaceAll("\n", "\r\n") : text);
  /** A load or restore is in progress: both resetDoc and setState fire a false onDirty
   *  (docChanged), which must not be taken as "the user started editing" and promote a preview
   *  tab. */
  let loadingDoc = false;

  function cancelAutosave() {
    if (autosaveTimer != null) {
      clearTimeout(autosaveTimer);
      autosaveTimer = null;
    }
  }

  /** The existence guard: writing back in the window before note_removed/files_changed arrives
   *  after an external deletion would resurrect the file just deleted. A non-md file is not in
   *  the notes index, so otherFiles is consulted; a meta file (a block type) is in neither list
   *  and has its own guard. */
  function docExists(path: string): boolean {
    if (isMetaPath(path)) return metaDocExists(path);
    return path.endsWith(".md") ? vault.notes.some((n) => n.id === path) : vault.otherFiles.includes(path);
  }

  function scheduleAutosave() {
    cancelAutosave();
    autosaveTimer = setTimeout(() => {
      autosaveTimer = null;
      if (!view || !docPath || docPath !== vault.currentPath) return;
      // Nothing is written on a timer while the agent runs (a conflict with edits to the
      // current note is the user's to settle knowingly), but it has to be rescheduled rather
      // than dropped: a pending timer is flush's dirty signal, and cutting the signal would
      // have a document switch silently throw those edits away
      if (agent.running) return scheduleAutosave();
      if (!docExists(docPath)) return scheduleAutosave();
      // Not while an IME composition is open: the save's echo (dirty cleared, the index
      // bumped) dispatches into the editor, and WebKit re-plants the composition at
      // wherever the selection ends up — text lands at the line's start
      if (view.composing) return scheduleAutosave();
      void vault.saveCurrent(withEol(view.state.doc.toString())).then(() => {
        // A failed save keeps dirty: it reschedules and retries, so the dirty signal is not lost to one failure
        if (vault.dirty && autosaveTimer == null && docPath === vault.currentPath) scheduleAutosave();
      });
    }, AUTOSAVE_MS);
  }

  /** Write out the unsaved edits at once. A pending timer is itself the dirty signal — openNote
   *  resets vault.dirty first, so it cannot be relied on here. It returns the write's Promise,
   *  which the store's opening, renaming and vault-switching funnels await to guarantee the
   *  "write the old, then read the new" order. */
  function flushAutosave(): Promise<unknown> | undefined {
    if (autosaveTimer == null) return;
    cancelAutosave();
    if (!view || !docPath) return;
    const content = withEol(view.state.doc.toString());
    if (docPath === vault.currentPath && loadedFor === vault.reloadCounter) {
      // The steady state (the window losing focus, say): a full save that clears the dirty mark
      return vault.saveCurrent(content);
    } else if (docExists(docPath)) {
      // The document is being switched or reloaded: only the old document is written, and the store's current-document state is untouched
      return vault.writeDoc(docPath, content);
    }
  }
  // The store's opening, renaming, vault-switching and tab-switching funnels come through here to write the old buffer out before reading or moving
  vault.flushEditor = flushAutosave;
  // The snapshot of a suspended tab (the store takes it after the flush): the whole
  // EditorState plus the scroll and the line ending; content uses the on-disk form (withEol),
  // so on activation it compares byte for byte with what is read back.
  // An image or a PDF has no view and naturally returns null
  vault.snapshotEditor = () => {
    if (!view || !docPath) return null;
    return {
      state: view.state,
      scrollTop: view.scrollDOM.scrollTop,
      eol: docEol,
      content: withEol(view.state.doc.toString()),
    };
  };
  // Discard the pending autosave before a path is deleted: after deletePath (with refresh=false
  // during a bulk undo) the lists are stale, and the unmount branch's flush would run the
  // existence guard against them and write the file just moved to the trash straight back
  vault.discardEditor = (path) => {
    if (docPath === path || docPath?.startsWith(path + "/")) cancelAutosave();
  };

  // ---- The outline: recomputed 200ms after typing pauses (an O(n) pure string scan),
  // completely decoupled from the 1s autosave — the panel has to keep up with the hand and
  // cannot wait for query_blocks' on-disk index.
  // A non-md file sets it to null: an outline only means something for a note. ----
  const OUTLINE_MS = 200;
  let outlineTimer: ReturnType<typeof setTimeout> | null = null;
  function cancelOutline() {
    if (outlineTimer != null) {
      clearTimeout(outlineTimer);
      outlineTimer = null;
    }
  }
  function computeOutline() {
    // One toString feeds two consumers: the outline (md only) and the word count (md and plain text alike)
    const text = view ? view.state.doc.toString() : null;
    vault.outline = text != null && docPath?.endsWith(".md") ? extractOutline(text) : null;
    vault.docWords = text != null ? countWords(text) : null;
  }
  function scheduleOutline() {
    cancelOutline();
    outlineTimer = setTimeout(() => {
      outlineTimer = null;
      computeOutline();
    }, OUTLINE_MS);
  }

  /** Insert at the cursor (from an external source such as clipping). dispatch enters the undo
   *  stack automatically and triggers onDirty → autosave, so it goes down the same write chain
   *  as typing by hand.
   *  With block=true the payload is block-level Markdown (a quote block, a memo's list item):
   *  it aligns to line boundaries and does not overwrite the selection — inserting mid-line
   *  would twist a multi-line quote block into broken Markdown; an inline payload (a clipped
   *  link) keeps the original semantics and replaces the selection. */
  function insertAtCursor(text: string, block = false): boolean {
    if (!view || !docPath?.endsWith(".md") || docPath !== vault.currentPath) return false;
    const sel = view.state.selection.main;
    // The block level's positioning and separator rules all live in insert.ts (planCursorInsert); an inline payload replaces the selection
    const plan = block
      ? planCursorInsert(view.state.doc, sel.to, text)
      : { from: sel.from, to: sel.to, insert: text, caret: sel.from + text.length };
    landPlan(view, plan, block);
    return true;
  }
  /** Land an insert plan: the change and the caret, a `[!quote]-` clip folded as it would
   *  open (a block payload alone can carry one), and the focus. */
  function landPlan(v: EditorView, plan: Pick<InsertPlan, "from" | "to" | "insert" | "caret">, block: boolean) {
    v.dispatch({
      changes: { from: plan.from, to: plan.to, insert: plan.insert },
      selection: { anchor: plan.caret },
      scrollIntoView: true,
    });
    if (block) foldMarkedIn(v, plan.from, plan.from + plan.insert.length);
    focusEditor(v);
  }
  vault.insertAtCursor = insertAtCursor;

  /** The "format note" command: the whole document through the Markdown formatter, replaced
   *  as one transaction (undo restores it); the cursor keeps its offset, clamped. Returns
   *  whether anything changed. A document edited (or swapped) during the round trip is
   *  left alone: the formatted text belongs to the one that was sent. */
  async function formatDoc(): Promise<boolean> {
    if (!view) return false;
    const before = view.state.doc.toString();
    const after = await api.formatMarkdown(before);
    const v = view;
    if (!v || v.state.doc.toString() !== before || after === before) return false;
    const head = Math.min(v.state.selection.main.head, after.length);
    v.dispatch({ changes: { from: 0, to: v.state.doc.length, insert: after }, selection: { anchor: head }, scrollIntoView: true });
    return true;
  }
  vault.formatDoc = formatDoc;

  /** Insert at the end of a section (used when the target is the note that is open and a
   *  destination section was given; the section wins over the cursor).
   *  The positioning and separator rules live in insert.ts, on the same table of cases as
   *  Rust's insert.rs; a missing heading falls back to the end of the file, and the return
   *  value tells the caller to say so. */
  function insertAtSection(heading: string, text: string): "section" | "tail" | false {
    if (!view || !docPath?.endsWith(".md") || docPath !== vault.currentPath) return false;
    const plan = planInsert(view.state.doc.toString(), heading, text);
    landPlan(view, plan, true);
    return plan.landed;
  }
  vault.insertAtSection = insertAtSection;

  /** The space between the landing point and the top of the viewport, about two lines of body
   *  text. It used to be 8px: the landing point hugged the tab bar's lower edge with not one
   *  line of context above, while "is this the one I clicked" is judged from exactly that
   *  context; worse, any height measured only after the scroll (a decoration rebuild, an image
   *  placeholder becoming its real height) needed only to push down a little to cut the landing
   *  point in half. This margin solves both. */
  const REVEAL_TOP_MARGIN = 64;

  /** Jump to a document character position (clicked from the outline, the kanban or Agenda).
   *  The cursor lands and it scrolls to the top of the viewport (leaving REVEAL_TOP_MARGIN of
   *  context); it focuses, following insertAtCursor's precedent — clicking an entry means "go
   *  there and continue".
   *  suppressReveal: a programmatic jump is not a user's intent about the selection, so the
   *  landing line stays rendered (without the annotation a task line would burst into a bare
   *  `- [ ]`); any later key or mouse action by the user arms it again. */
  function revealPos(pos: number, flash = false): boolean {
    if (!view || !docPath?.endsWith(".md") || docPath !== vault.currentPath) return false;
    // The document may have shortened during the outline's debounce window, so it is clamped against a RangeError
    const p = Math.min(pos, view.state.doc.length);
    // The landing point may lie outside the first screen's parse budget: the syntax tree is
    // pushed past it synchronously first (with a screen of slack), or the viewport would fly to
    // an unparsed region, the missing decorations would flash the source for a frame, and it
    // would only render once the parse worker dispatched a transaction. Once ensure has
    // advanced the ParseContext, the dispatch below lands the new tree into the state through
    // the language field's apply (treeChanged), and livepreview, the tables and the query
    // blocks rebuild within that same transaction; on a timeout (a very large note) it falls
    // back to the old late rendering.
    ensureSyntaxTree(view.state, Math.min(p + 5000, view.state.doc.length), 100);
    // A landing point inside a folded section needs no unfolding here: CodeMirror's
    // fold field unfolds any fold the selection head enters
    view.dispatch({
      selection: { anchor: p },
      effects: EditorView.scrollIntoView(p, { y: "start", yMargin: REVEAL_TOP_MARGIN }),
      annotations: suppressReveal.of(true),
    });
    focusEditor(view);
    if (flash) {
      // The landing block (its whole extent) flashes so the eye finds it after the scroll
      const b = blockAt(view.state, p);
      flashRange(view, p, b ? extentTo(view.state, b) : view.state.doc.lineAt(p).to);
    }
    // One more beat: CM clears scrollTarget once it has used it (scrollTarget = null after
    // scrollIntoView inside the measure loop), and after that nobody corrects a transaction
    // that changes the height above the landing point. So the next frame scrolls the same way
    // again: with no displacement it is a no-op (nothing flickers), and with one it scrolls
    // right back.
    // It carries effects only and no selection — revealArmed only honours tr.selection, so this
    // shot does not reveal the landing line as source.
    const v = view;
    requestAnimationFrame(() => {
      if (view !== v || docPath !== vault.currentPath) return;
      const q = Math.min(p, v.state.doc.length);
      v.dispatch({ effects: EditorView.scrollIntoView(q, { y: "start", yMargin: REVEAL_TOP_MARGIN }) });
    });
    return true;
  }
  vault.revealPos = revealPos;

  /** The palette's block commands act on the selected run (block mode) or the block at the cursor. */
  function blockCommand(op: "move-up" | "move-down" | "duplicate" | "delete"): boolean {
    if (!view || !docPath?.endsWith(".md") || docPath !== vault.currentPath) return false;
    const state = view.state;
    let run = selectedBlocks(state);
    if (!run.length) {
      const b = blockAt(state, state.selection.main.head);
      if (!b) return false;
      run = [b];
    }
    const spec =
      op === "move-up"
        ? moveBlocks(state, run, -1)
        : op === "move-down"
          ? moveBlocks(state, run, 1)
          : op === "duplicate"
            ? duplicateBlocks(state, run)
            : deleteBlocks(state, run);
    if (!spec) return false;
    view.dispatch(spec);
    focusEditor(view);
    return true;
  }
  vault.blockCommand = blockCommand;
  /** Land the cursor on a pending run (its first line) and bring it into view. */
  function landOn(v: EditorView, pos: number) {
    // annotated like every other programmatic landing: the review steps onto a run, which
    // is not the reader asking to see its source (reveal.ts's suppressReveal)
    v.dispatch({ selection: { anchor: pos }, scrollIntoView: true, annotations: suppressReveal.of(true) });
    focusEditor(v);
  }
  const stepPending = (which: ReviewStep): boolean => {
    if (!view) return false;
    const pos = stepRun(view.state, view.state.selection.main.head, which);
    if (pos == null) return false;
    landOn(view, pos);
    return true;
  };
  vault.stepPending = stepPending;
  /** Where the review goes on after acting on these blocks: the start of the next run
   *  below the ones they belong to, else the note's first run still unreviewed (a review
   *  begun mid-note wraps before leaving it), else null — the note's end. Null ids (the
   *  whole note) go nowhere. */
  function afterRuns(state: EditorState, ids: string[] | null): number | null {
    if (!ids) return null;
    const runs = pendingRuns(state);
    const acted = (r: { ids: string[] }) => r.ids.some((id) => ids.includes(id));
    const end = runs.filter(acted).reduce((m, r) => Math.max(m, r.to), -1);
    if (end < 0) return null;
    const rest = runs.filter((r) => !acted(r));
    return (rest.find((r) => r.from >= end) ?? rest[0])?.from ?? null;
  }
  /** After a run is reviewed: the cursor moves to the next run of the note, or the review
   *  steps to the next note with unreviewed blocks. `null` ids (a whole-note action) stay. */
  function advance(v: EditorView, ids: string[] | null, next: number | null) {
    if (!ids) return;
    if (next != null) landOn(v, next);
    else void vault.reviewNextNote();
  }
  /** Mark blocks reviewed and step on (the run's ✓, ⌘Y, the palette). */
  async function acceptProvenance(ids: string[] | null) {
    const v = view;
    if (!v) return;
    const next = afterRuns(v.state, ids);
    const n = await vault.acceptProvenance(ids);
    if (view !== v || !n) return;
    advance(v, ids, next);
  }
  /** Put the agent write behind unreviewed blocks (local ids: one block, or the run the
   *  editor groups) or, with null, every unreviewed block of the note back: the ledger's
   *  record of what the write replaced goes in again, as one undoable edit in the buffer,
   *  which then saves like any edit (the mark goes with the block's hash). The buffer
   *  flushes first so the local ids the marks carry are disk's; the plan is made against
   *  the buffer and applied only if it is unchanged. Then the review steps on. */
  async function revertProvenance(ids: string[] | null) {
    const v = view;
    const path = docPath;
    if (!v || !path) return;
    if (vault.dirty) await flushAutosave();
    if (view !== v || docPath !== path) return;
    const text = v.state.doc.toString();
    let plan: RevertPlan;
    try {
      plan = await api.provenanceRevertPlan(text, ids ?? undefined);
    } catch (e) {
      toast.push(String(e), "error");
      return;
    }
    if (view !== v || docPath !== path || v.state.doc.toString() !== text) {
      toast.push(t("prov.revertStale"), "error");
      return;
    }
    if (!plan.replacements.length) {
      toast.push(t(plan.missed ? "prov.revertMissed" : "prov.noPendingHere"), "error");
      return;
    }
    // the specs, not a ChangeSet: whenNotComposing may defer the dispatch past an IME
    // composition that has since changed the document, and a ChangeSet built against the
    // old length would be refused outright
    const changes = plan.replacements.map((r) => ({ from: byteToChar(text, r.from), to: byteToChar(text, r.to), insert: r.insert }));
    const missed = plan.missed;
    // the run after the reverted one, carried across the edit
    const after = afterRuns(v.state, ids);
    const next = after != null ? v.state.changes(changes).mapPos(after, 1) : null;
    whenNotComposing(() => {
      if (view !== v) return;
      v.dispatch({ changes, annotations: Transaction.userEvent.of("prov.revert") });
      const n = plan.replacements.length;
      toast.push(missed ? t("prov.revertedPartly", { n, m: missed }) : t("prov.reverted", { n }));
      advance(v, ids, next);
    });
  }
  vault.rejectProvenance = revertProvenance;
  vault.reviewRun = (act) => {
    if (!view) return false;
    const run = runAt(view.state, view.state.selection.main.head);
    if (!run) return false;
    void (act === "accept" ? acceptProvenance(run.ids) : revertProvenance(run.ids));
    return true;
  };

  /** The snapshot source of `@ current selection`: with a non-empty main selection it updates
   *  vault.editorSelection with the line numbers (1-based, inclusive) and the text, and an
   *  empty selection or no document clears it to null. When the selection's end falls exactly
   *  at the start of a line (dragging to the next line's start, triple-clicking a whole line)
   *  that line holds nothing selected and is not counted in the range — matching the `(10-20)`
   *  annotation of other editors. The text is cut to SELECTION_MAX_CHARS without cutting
   *  through a surrogate pair (see trimLoneSurrogate in api.ts). */
  function syncSelection(from: number, to: number) {
    if (!view || !docPath || from === to) {
      if (vault.editorSelection) vault.editorSelection = null;
      if (vault.editorSelStats) vault.editorSelStats = null;
      return;
    }
    const doc = view.state.doc;
    const a = doc.lineAt(from);
    const tail = doc.lineAt(to);
    const b = to === tail.from && tail.number > a.number ? doc.line(tail.number - 1) : tail;
    // The status line's selection statistics: chars is computed from the doc
    // (editorSelection.text is truncated and cannot be counted). An unchanged value keeps the
    // same object, so nothing downstream spins during a drag
    const stats = vault.editorSelStats;
    if (!stats || stats.chars !== to - from || stats.lines !== b.number - a.number + 1)
      vault.editorSelStats = { chars: to - from, lines: b.number - a.number + 1 };
    const text = trimLoneSurrogate(doc.sliceString(from, Math.min(to, from + SELECTION_MAX_CHARS)));
    // The same selection keeps the same object: downstream deriveds (the chips, the @
    // candidates) track it by object identity, and handing them a new snapshot on every update
    // would have them recompute for nothing (all the more during a drag)
    // A selection touching a private block never becomes a chip: Rust cuts private
    // lines it can see on disk, but a block that only exists in this buffer is
    // invisible to it — the editor closes that gap (see privateblock.ts)
    const priv = selectionTouchesPrivate(doc, from, to);
    const cur = vault.editorSelection;
    if (
      cur &&
      cur.path === docPath &&
      cur.from_line === a.number &&
      cur.to_line === b.number &&
      cur.text === text &&
      (cur.private === true) === priv
    )
      return;
    vault.editorSelection = priv
      ? { path: docPath, from_line: a.number, to_line: b.number, text, private: true }
      : { path: docPath, from_line: a.number, to_line: b.number, text };
  }

  /** The buffer sync for a clip's image localisation (clip://localized). The rewrite happens on
   *  disk, but a dirty buffer is not reloaded by NoteModified — the next autosave would quietly
   *  wipe the rewrite out. So the same substitution is applied to the live buffer (touching only
   *  the target of `![..](target)`, matching the Rust side's rewrite_images).
   *  On a clean buffer the event can still arrive a beat before the reload that carries the
   *  same rewrite, and then the substitution is a real change — which must not count as the
   *  user's: marked dirty, the buffer would skip every NoteModified that follows and the
   *  autosave would write it, one step stale, over newer disk content (an agent's next edit
   *  vanished exactly so). It is applied as an external change, like a reload, and the buffer
   *  stays clean; a buffer the user had dirtied keeps its dirty mark and its autosave. */
  function applyLocalizedImages(path: string, pairs: [string, string][]) {
    if (!view || docPath !== path) return;
    const doc = view.state.doc.toString();
    const changes: { from: number; to: number; insert: string }[] = [];
    for (const [remote, local] of pairs) {
      const esc = remote.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
      const re = new RegExp(`(!\\[[^\\]]*\\]\\()${esc}(?=[\\s)])`, "g");
      for (const m of doc.matchAll(re)) {
        const start = m.index + m[1].length;
        changes.push({ from: start, to: start + remote.length, insert: local });
      }
    }
    if (!changes.length) return;
    const wasDirty = vault.dirty;
    loadingDoc = true;
    try {
      view.dispatch({ changes, annotations: Transaction.userEvent.of("external") });
    } finally {
      loadingDoc = false;
    }
    if (!wasDirty) {
      cancelAutosave();
      vault.dirty = false;
    }
  }
  let unlistenLocalized: (() => void) | null = null;
  void listen<{ path: string; replace: [string, string][] }>("clip://localized", (e) => {
    applyLocalizedImages(e.payload.path, e.payload.replace);
  }).then((u) => (unlistenLocalized = u));

  $effect(() => {
    // refresh completion targets whenever the note list changes
    const notes = vault.notes;
    const priv = vault.privatePaths;
    // privacy_list is already expanded note by note under Rust's semantics (globs, case, `!`
    // exemptions), so an exact has is enough; a prefix scan on top of it would also hide the
    // public notes exempted by `!diary/Public.md` from the completions
    const isPrivate = (id: string) => priv.has(id);
    const stems = (n: (typeof notes)[number]) => {
      const stem = n.id.replace(/\.md$/, "").split("/").pop() ?? n.id;
      return [stem, ...n.aliases];
    };
    // The completion candidates (link targets, tags) do not expose notes in a privacy zone
    const visible = notes.filter((n) => !isPrivate(n.id));
    targetList = visible.flatMap(stems);
    // The resolved check uses the full set: an existing link pointing at a private note should not be shown as unresolved
    knownTargets = new Set(notes.flatMap(stems).map((t) => t.toLowerCase()));
    // Tags are sorted by how often they are used, and alphabetically within the same count
    const counts = new Map<string, number>();
    for (const n of visible) for (const t of n.tags) counts.set(t, (counts.get(t) ?? 0) + 1);
    tagList = [...counts.keys()].sort((a, b) => counts.get(b)! - counts.get(a)! || compareNames(a, b));
  });

  /** Copy `[[Stem#^id]]` (or `![[…]]`) for a block: Rust decides the id and where it
   *  goes (`block_id_plan` with the block's content offset); the editor applies the one
   *  insert to its buffer and never guesses the placement. A heading needs no id. */
  async function copyBlockRef(embed: boolean, b: EditorBlock) {
    if (!view || !docPath) return;
    const source = view.state.doc.toString();
    let plan;
    try {
      plan = await api.blockIdPlan(source, { by: "offset", offset: charToByte(source, blockContentStart(view.state, b)) });
    } catch (e) {
      toast.push(String(e), "error");
      return;
    }
    if (plan.insert && view.state.doc.toString() === source) {
      view.dispatch({
        changes: { from: byteToChar(source, plan.insert.offset), insert: plan.insert.text },
        userEvent: "block.id",
      });
    } else if (plan.insert) {
      toast.push(t("block.changedRetry"), "error");
      return;
    }
    const stem = docPath.replace(/\.md$/, "");
    const short = stem.split("/").pop() ?? stem;
    // another note with the same stem: the path form resolves exactly (SYNTAX: resolution)
    const clash = vault.notes.some((n) => n.id !== docPath && (n.id.split("/").pop() ?? n.id).replace(/\.md$/, "").toLowerCase() === short.toLowerCase());
    const name = clash ? stem : short;
    const link = plan.target.kind === "heading" ? `[[${name}#${plan.target.text}]]` : `[[${name}#^${plan.target.id}]]`;
    const ok = await copyText(embed ? `!${link}` : link);
    toast.push(t(ok ? "editor.copied" : "editor.copyFailed"), ok ? "info" : "error");
  }

  const blockHost: BlockHost & BlockMenuHost = {
    gripTip: () => t("block.handleTip"),
    openMenu: (v, b, e) => contextMenu.open(e, blockMenuEntries(v, b, blockHost, { compact: false })),
    onGripPointerDown: (v, b, e, grip, done) => startBlockDrag(v, b, e, grip, done),
    copyRef: (_v, b, embed) => void copyBlockRef(embed, b),
    isPrivate: () => !!docPath && vault.privatePaths.has(docPath),
    pendingBlock: (v, b) => {
      const row = originAt(v.state, b.from);
      return row && isPending(row.origin) ? row.local_id : null;
    },
    pendingCount: () => vault.pendingCount,
    blockTypes: () => types.enabled,
    rejectProvenance: (ids) => void revertProvenance(ids),
    acceptProvenance: (ids) => void acceptProvenance(ids),
    // Select the block's lines (the composer's automatic selection chip picks the
    // selection up through syncSelection → vault.editorSelection) and summon the agent
    askAgent: (v, b) => {
      const from = b.from;
      const to = extentTo(v.state, b);
      v.dispatch({ selection: { anchor: from, head: to }, scrollIntoView: true });
      focusEditor(v);
      if (blockHost.isPrivate()) {
        toast.push(t("block.askPrivate"), "error");
        return;
      }
      void executeCommand("focus-agent");
    },
  };

  const embedHost: EmbedHost = {
    resolve: (target: string) => api.resolveRef(target, docPath ?? undefined),
    follow: (target: string) => void vault.followLink(target),
    openUrl: (url: string) => openMdLink(url, false),
    openBlockAt: (note: string, byteStart: number, content?: string) => void vault.openBlockAt(note, byteStart, content),
    currentNote: () => docPath,
    loadAsset,
  };

  const host = {
    isKnown: (t: string) => knownTargets.has(t),
    targets: () => targetList,
    follow: (t: string) => void vault.followLink(t),
    currentPath: () => docPath,
    resolveNote: async (target: string) => {
      const note = target.trim() ? await api.resolveLink(target.trim(), docPath ?? undefined) : docPath;
      return note && !vault.privatePaths.has(note) ? note : null;
    },
    // citation.ts's cache, the same one the `@Note#` picker, the hover card and the
    // click path read — a second copy here fetched every note twice
    blocksOf: (note: string) => blocksOf(note, vault.indexVersion),
    planId: (source: string, selector: BlockSelector) => api.blockIdPlan(source, selector),
    ensureId: (note: string, localId: string, guard: string) => api.blockEnsureId(note, localId, guard),
    toast: (message: string, level?: "info" | "error") => toast.push(message, level),
  };

  /** Where `text` sits in the open buffer, nearest to `near`: the index's spans lag an
   *  unsaved buffer by up to the autosave delay, so a jump verifies its landing point. */
  function locateText(text: string, near: number): number | null {
    if (!view || !text) return null;
    const doc = view.state.doc;
    const head = text.split("\n")[0] ?? text;
    if (doc.sliceString(near, near + head.length) === head) return near;
    const all = doc.toString();
    let best: number | null = null;
    for (let i = all.indexOf(head); i >= 0; i = all.indexOf(head, i + 1)) {
      if (best == null || Math.abs(i - near) < Math.abs(best - near)) best = i;
    }
    return best;
  }
  vault.locateText = locateText;
  vault.copyBlockRef = (embed: boolean) => {
    if (!view || !docPath?.endsWith(".md") || docPath !== vault.currentPath) return false;
    const b = blockAt(view.state, view.state.selection.main.head);
    if (!b) return false;
    void copyBlockRef(embed, b);
    return true;
  };

  // markdown link targets: CommonMark forbids raw spaces/parens in (…), and
  // "%" must be escaped so the target round-trips through decodeURIComponent
  const ENC: Record<string, string> = { "%": "%25", " ": "%20", "(": "%28", ")": "%29" };
  const encodeTarget = (rel: string) => rel.replace(/[% ()]/g, (c) => ENC[c]);

  /** Markdown reference for a pasted vault file: embed images, link the rest. */
  function mdRef(rel: string): string {
    const target = encodeTarget(rel);
    return fileKind(rel) === "image" ? `![](${target})` : `[${rel.split("/").pop()}](${target})`;
  }

  // The blob URL cache of embedded images (keyed by the resolved vault path), valid for the
  // vault's session — an image widget is rebuilt again and again as the cursor moves, and
  // without a cache the disk would be read repeatedly
  const assetUrls = new Map<string, Promise<string | null>>();

  // Editor stays mounted and a component-level cache would outlive a vault switch: the same
  // relative path (attachments/logo.png) would render the previous vault's image. On a switch
  // it is invalidated wholesale and the blobs are reclaimed
  $effect(() => {
    void vault.root;
    for (const p of assetUrls.values()) void p.then((u) => u && URL.revokeObjectURL(u));
    assetUrls.clear();
    clearImageDims(); // the size placeholder cache has the same trap: the previous vault's aspect ratio would skew a same-named image in the new one
    clearEmbedCaches(); // and the embed caches: the same target names another note's block in the new vault
  });

  /** An image embed's target → a vault-relative path. With a / it matches the vault-root-relative
   *  path exactly; a bare file name is looked up by name across the vault's files, and among
   *  matches the shortest path wins (the same resolution semantics as a wikilink's). Not found,
   *  or not an image, returns null. */
  function resolveAsset(target: string): string | null {
    let rel = target;
    try {
      rel = decodeURIComponent(target);
    } catch {
      /* not our encoding */
    }
    rel = rel.replace(/^\.\//, "");
    const lower = rel.toLowerCase();
    const files = vault.otherFiles;
    let hit = files.find((f) => f.toLowerCase() === lower) ?? null;
    if (!hit && !rel.includes("/")) {
      hit = files
        .filter((f) => (f.split("/").pop() ?? "").toLowerCase() === lower)
        .sort((a, b) => a.length - b.length || compareNames(a, b))[0] ?? null;
    }
    if (hit) return fileKind(hit) === "image" ? hit : null;
    // The file list is a projection that can lag behind the disk: the clip localiser
    // writes clips/assets/ and rewrites the note in one go, and clip://localized reaches
    // the buffer before the list refresh's round trip returns — a lookup at that moment
    // misses and the widget would be nailed as "missing" (its DOM is reused as long as
    // the target is the same). An explicit path (one with a /) needs no lookup at all:
    // it is read as written, and a file that is not there fails in the read itself. A
    // bare name has nothing to go on without the list.
    return rel.includes("/") && fileKind(rel) === "image" ? rel : null;
  }

  /** An image embed's target → a blob URL (resolved through resolveAsset, with the blob cached at component level). */
  function loadAsset(target: string): Promise<string | null> {
    const path = resolveAsset(target);
    if (!path) return Promise.resolve(null);
    let p = assetUrls.get(path);
    if (!p) {
      p = api
        .fileReadBinary(path)
        .then((buf) => URL.createObjectURL(new Blob([buf], { type: mimeOf(path) })))
        .catch(() => {
          // One transient read failure should not nail this image as "missing" for a whole session
          assetUrls.delete(path);
          return null;
        });
      assetUrls.set(path, p);
    }
    return p;
  }

  /** One-click image copy: resolved as loadAsset does, written to the system clipboard (a
   *  bitmap through NSPasteboard on the Rust side). Success or failure goes back to the button
   *  as feedback. */
  function copyImage(target: string): Promise<boolean> {
    const path = resolveAsset(target);
    if (!path) return Promise.resolve(false);
    return api.clipboardCopyImage(path).then(
      () => true,
      () => false,
    );
  }

  /** Click/⌘click on a markdown link: vault files open in-app; http(s) goes
   *  through url_open, which forks per settings (invert = ⌘click's "other way of opening it",
   *  which only means something for http(s)).
   *  Targets we didn't encode may contain raw "%" — keep them as-is. */
  function openMdLink(raw: string, invert = false) {
    if (/^https?:\/\//i.test(raw)) {
      api.urlOpen(raw, invert).catch((e) => (vault.error = String(e)));
      return;
    }
    let target = raw;
    try {
      target = decodeURIComponent(raw);
    } catch {
      /* not our encoding */
    }
    void vault.openFile(target.replace(/^\.\//, ""));
  }

  // ---- The editor body's context menu: it takes over contextmenu inside .cm-editor
  // (displacing WKWebView's native "look up / spelling / services" menu — a deliberate
  // trade-off, since the vemry items are used more often; FilePreview's images and PDFs are not
  // intercepted). Cut and copy go through navigator.clipboard.writeText (the write direction is
  // reliable in WKWebView); the paste's three fallbacks are in pasteFromMenu. ----

  /** The plain-text insertion of the menu's paste: not limited to .md (insertAtCursor serves notes alone), and it replaces the selection. */
  function rawInsert(text: string) {
    if (!view) return;
    const sel = view.state.selection.main;
    view.dispatch({
      changes: { from: sel.from, to: sel.to, insert: text },
      selection: { anchor: sel.from + text.length },
      scrollIntoView: true,
    });
    focusEditor(view);
  }

  async function copySelection(): Promise<boolean> {
    if (!view) return false;
    const sel = view.state.selection.main;
    if (sel.empty) return false;
    try {
      await navigator.clipboard.writeText(view.state.sliceDoc(sel.from, sel.to));
      return true;
    } catch {
      return false;
    }
  }

  function cutSelection() {
    if (!view) return;
    const doc = view.state.doc;
    const sel = view.state.selection.main;
    void copySelection().then((ok) => {
      if (!ok || !view) return; // a failed copy deletes nothing — a cut must not become a plain delete
      // Should the document have been swapped while waiting for the clipboard (an external
      // change's resetDoc, an agent write), give up: an old offset against a new document would
      // delete the wrong text or even throw a RangeError. Text is immutable, so comparing references is enough.
      if (view.state.doc !== doc) return;
      view.dispatch({ changes: { from: sel.from, to: sel.to, insert: "" }, selection: { anchor: sel.from } });
      focusEditor(view);
    });
  }

  /** Three fallbacks: ① a file or a screenshot goes through the same Rust pipeline as ⌘V
   *  (reference semantics, with mdRef landing the link); ② Rust's NSPasteboard plain text;
   *  ③ off mac it falls back to navigator.clipboard.readText. */
  async function pasteFromMenu() {
    if (vault.currentIsNote) {
      try {
        const rels = await vault.pasteIntoNote([]);
        if (rels.length) {
          insertAtCursor(rels.map(mdRef).join("\n"));
          return;
        }
      } catch (e) {
        vault.error = String(e);
        return;
      }
    }
    let text: string | null = null;
    try {
      text = await api.clipboardReadText();
    } catch {
      text = null;
    }
    if (text == null) {
      try {
        text = await navigator.clipboard.readText();
      } catch {
        text = null;
      }
    }
    if (text) rawInsert(text);
    else toast.push(t("editor.clipboardEmpty"));
  }

  function editorMenu(e: MouseEvent) {
    if (!view || !docPath || docPath !== vault.currentPath) return;
    const path = docPath;
    const isNote = path.endsWith(".md");
    const hasSel = !view.state.selection.main.empty;
    const entries: MenuEntry[] = [];
    // The link at the point (the same linkAt policy as ⌘click and ⌘Enter: a purely positional verdict, wiki before md)
    const pos = view.posAtCoords({ x: e.clientX, y: e.clientY });
    const found = isNote && pos != null ? linkAt(view.state, pos) : null;
    if (found) {
      const { target } = found.hit;
      entries.push(
        found.flavor === "wiki"
          ? { label: t("editor.openLinkNamed", { target }), action: () => void vault.followLink(target) }
          : { label: t("editor.openLink"), action: () => openMdLink(target, false) },
        { separator: true },
      );
    }
    // Cut, copy, paste and open-link are editor-local actions (they need the point and CM's
    // selection), and everything else goes through the command registry — the menu does not grow
    // a second set of semantics (editorMenu has already ensured docPath === vault.currentPath at
    // the top, the same scope those commands read)
    entries.push(
      { label: t("editor.cut"), hint: k("Mod+X"), disabled: !hasSel, action: cutSelection },
      { label: t("editor.copy"), hint: k("Mod+C"), disabled: !hasSel, action: () => void copySelection() },
      { label: t("editor.paste"), hint: k("Mod+V"), action: () => void pasteFromMenu() },
      { separator: true },
    );
    // The block under the point: copy its link, turn it into another kind, ask the AI
    // about it (the structural operations live on the grip and the keyboard)
    const block = isNote && pos != null ? blockAt(view.state, pos) : null;
    if (block) entries.push(...blockMenuEntries(view, block, blockHost, { compact: true }), { separator: true });
    if (isNote) entries.push({ label: t("cmd.translate-toggle.title"), hint: k("Mod+Shift+T"), action: () => void executeCommand("translate-toggle") });
    if (isNote) entries.push({ label: t("cmd.note-history.title"), action: () => void executeCommand("note-history") });
    entries.push(
      { label: t("tabBar.revealInTree"), action: () => void executeCommand("reveal-in-tree") },
      privacyMenuItem(path, (op) => void executeCommand(op === "mark" ? "mark-private" : "unmark-private")),
      { separator: true },
      {
        label: t(isNote ? "editor.deleteNote" : "editor.deleteFile"),
        danger: true,
        confirm: t("editor.confirmDelete"),
        action: () => void executeCommand("delete-current-note"),
      },
    );
    contextMenu.open(e, entries);
  }

  /** After an external change (an agent write landing, an edit in another app) is applied to
   *  the buffer: bring the changed span into view and put the cursor at its first line, so the
   *  reader lands where the note changed — the run's ✓ and ⌘Y are then one press away. The
   *  cursor stays out of a code fence (a bare cursor inside a lab block would unfold its card
   *  over the reader) and out of a range selection the user is holding.
   *  This is also what keeps the view from going blank. Measured on the reload with the view
   *  scrolled to the note's end: the change rebuilds the block widgets above it, the ones not
   *  yet re-measured fall back to their estimates, the height map shrinks by the difference (an
   *  embed of 1800px, say), and the viewport, still at the old scroll offset, ends up past the
   *  document — an empty viewport draws nothing, so nothing is re-measured, and the note stays
   *  blank until a scroll. Anchoring the view to the changed span pulls it back onto real
   *  content. */
  function revealExternalChange(v: EditorView, from: number, to: number) {
    const doc = v.state.doc;
    const start = Math.min(from, doc.length);
    const end = Math.max(start, Math.min(to, doc.length));
    const line = doc.lineAt(start);
    const held = !v.state.selection.main.empty;
    let anchor = line.from;
    // out of a fence: the card's reveal counts its first line as inside, so the cursor goes
    // to the line above (a change that starts with a lab block lands just before its card)
    for (let fence = fencedCodeAt(v.state, anchor); fence && fence.from > 0; fence = fencedCodeAt(v.state, anchor)) {
      anchor = doc.lineAt(fence.from - 1).from;
    }
    v.dispatch({
      ...(held ? {} : { selection: { anchor } }),
      effects: EditorView.scrollIntoView(start, { y: "center" }),
      annotations: suppressReveal.of(true), // an external write landing in view is not a request to see its source
    });
    if (end > start) flashRange(v, start, end);
  }

  /** Finishing a load: shared by the restore-a-suspended-tab path and the create-a-new-state
   *  path. Adding a store field derived from the editor changes this one place — adding
   *  editorLoc and editorSelStats in this batch would once have meant keeping four sites in
   *  step, and missing one is stale data. With head=0, lineAt(0) gives exactly 1:1, which
   *  matches the old hardcoded value. */
  /** Stamp what the note reads as on the editor body: `lang` (its declared `lang:`, else the
   *  language its letters give away) for faces, line breaking and emphasis marks, and
   *  `data-script` for the column width. A note too short to judge, and every non-note file,
   *  carries neither and follows the interface. Read once at load and never while the reader
   *  edits: a column that changed width mid-sentence would move the text under the cursor (a
   *  `lang:` edit takes effect at the next load). The head of the document is enough — the
   *  detector stops after a few thousand letters. */
  function applyNoteLang(v: NonNullable<typeof view>, path: string) {
    const nl = path.endsWith(".md") ? noteLanguage(v.state.doc.sliceString(0, Math.min(v.state.doc.length, 16000))) : { lang: null, script: null };
    applyLangAttrs(v.contentDOM, nl);
  }

  function finishLoad(path: string, eol: "\n" | "\r\n", head: number) {
    const v = view!; // both call paths run after view has been assigned
    docPath = path;
    docEol = eol;
    // resetDoc's docChanged fires one onDirty, so the false timer it scheduled is killed
    cancelAutosave();
    vault.dirty = false;
    // The outline timer that same onDirty scheduled is void too: it is computed directly for the
    // new document (md's loadDoc is a synchronous path, so the panel never flashes the previous
    // note's stale outline)
    cancelOutline();
    computeOutline();
    applyNoteLang(v, path);
    vault.editorCursor = head;
    const ln = v.state.doc.lineAt(head);
    vault.editorLoc = { line: ln.number, col: head - ln.from + 1 };
    vault.editorBlockPrivate = privateRangeAt(v.state.doc, head) != null;
    vault.editorOrigin = null;
    vault.pendingCount = 0;
    // Provenance for the document just loaded. The effect below keys on the store's
    // reactive signals only; view and docPath are plain variables, so at startup (and
    // on any load the effect ran ahead of) it had returned early and nothing re-ran it
    // until the next index event — marks appeared only after the next chat turn
    if (path.endsWith(".md")) {
      v.dispatch({ effects: [provOverlayEffect.of(vault.provenanceOverlay), provMarksEffect.of(prefs.provenanceMarks)] });
      whenNotComposing(() => void loadProvenance(path));
    }
    // The onSelection resetDoc fires still carries the previous note's docPath, so it is zeroed here against the new document
    vault.editorSelection = null;
    vault.editorSelStats = null;
  }

  /** Clear every store signal derived from the editor when there is no document (shared by the DOM unmount and the component's destruction). */
  function clearDocSignals() {
    cancelOutline();
    vault.outline = null;
    vault.docWords = null;
    vault.editorLoc = null;
    vault.editorBlockPrivate = false;
    vault.editorOrigin = null;
    vault.pendingCount = 0;
    vault.editorSelection = null;
    vault.editorSelStats = null;
  }

  /** Read the note's provenance from disk truth and hand it to the editor's field
   *  (byte spans → character ranges in one pass). Guarded against a stale reply: the
   *  note may have changed under the await. */
  async function loadProvenance(path: string) {
    let items;
    try {
      items = await api.provenanceGet(path);
    } catch {
      return;
    }
    if (!view || docPath !== path || view.composing) return;
    const text = view.state.doc.toString();
    // One pass for all three lists: byteSpansToChar dedupes and resolves an arbitrary set
    // of spans in a single walk, and a source note runs to hundreds of thousands of
    // characters — three calls walk it three times
    const n = items.length;
    const all = byteSpansToChar(text, [
      ...items.map((p) => p.span),
      ...items.map((p) => p.changed ?? p.span),
      ...items.map((p) => p.hunk ?? p.span),
    ]);
    const ranges = all.slice(0, n);
    const changed = all.slice(n, n * 2);
    const hunks = all.slice(n * 2);
    const rows = items.map((p, i) => ({
      ...ranges[i],
      local_id: p.local_id,
      origin: p.origin,
      changed: p.changed ? changed[i] : undefined,
      hunk: p.hunk ? hunks[i] : undefined,
      replaced: p.replaced,
    }));
    view.dispatch({ effects: provenanceSet.of(rows) });
    vault.pendingCount = rows.filter((r) => isPending(r.origin)).length;
    vault.editorOrigin = originAt(view.state, view.state.selection.main.head)?.origin ?? null;
    // A note opened for review lands on its first (or last) run once the rows are here
    const landing = vault.reviewLanding;
    if (landing?.path === path) {
      vault.reviewLanding = null;
      stepPending(landing.which);
    }
  }

  async function loadDoc(counter: number, path: string, content: string) {
    // ---- The fast path of restoring a suspended tab (activateTab's cache validation passed):
    // the whole EditorState is reused, and undo, the cursor and the selection survive with it.
    // The state was created for the same path under the same configuration (the docPath/docEol
    // the closure holds are component-level variables and are updated right after), so reusing
    // it is safe. One-shot: consumed and cleared, so an external reload cannot run into a stale
    // restore ----
    const restore = vault.pendingRestore;
    if (restore) {
      vault.pendingRestore = null;
      if (counter !== loadedFor || !container) return;
      const st = restore.state as EditorState;
      loadingDoc = true;
      try {
        if (view) view.setState(st);
        else view = new EditorView({ state: st, parent: container });
      } finally {
        loadingDoc = false;
      }
      // The cursor, line and column are filled in from the restored state (not zeroed); pushing the selection is left to the next selectionSet
      finishLoad(path, restore.eol, st.selection.main.head);
      // The query block result widgets inside a cached state may be stale: a requery signal is sent once more
      if (path.endsWith(".md")) view.dispatch({ effects: queryRefresh.of(vault.indexVersion) });
      // The scroll position is restored after the layout settles (setting it synchronously after setState would be reset by the measuring)
      const v = view;
      requestAnimationFrame(() => {
        if (view === v && docPath === path) v.scrollDOM.scrollTop = restore.scrollTop;
      });
      return;
    }
    // ---- The document already open, reloaded from disk (an agent write landing, an edit in
    // another app, reloadCurrent): applied as one edit instead of a fresh state. The undo
    // stack survives it and ⌘Z takes the change back — the buffer's own history is the first
    // "undo the AI" — and the view is then brought onto the changed span
    // (revealExternalChange). Only the changed span is
    // touched (the longest common prefix and suffix), so the marks the fields map through it
    // stay on their lines. The transaction's docChanged fires the same false onDirty as
    // resetDoc; finishLoad kills that timer the same way ----
    if (view && docPath === path) {
      const v = view;
      const cur = v.state.doc.toString();
      const next = Text.of(content.split(/\r\n?|\n/)).toString();
      if (cur !== next) {
        const max = Math.min(cur.length, next.length);
        let head = 0;
        while (head < max && cur.charCodeAt(head) === next.charCodeAt(head)) head++;
        let tail = 0;
        while (tail < max - head && cur.charCodeAt(cur.length - 1 - tail) === next.charCodeAt(next.length - 1 - tail)) tail++;
        const insert = next.slice(head, next.length - tail);
        loadingDoc = true;
        try {
          v.dispatch({
            changes: { from: head, to: cur.length - tail, insert },
            annotations: Transaction.userEvent.of("external"),
          });
        } finally {
          loadingDoc = false;
        }
        revealExternalChange(v, head, head + insert.length);
      }
      finishLoad(path, docEol, v.state.selection.main.head);
      return;
    }
    // notes get markdown+wikilinks; other files get their language by extension
    const isNote = path.endsWith(".md");
    const language = isNote ? null : await languageFor(path);
    if (counter !== loadedFor || !container) return; // superseded while loading
    const opts = {
      parent: container,
      doc: content,
      host: isNote ? host : null,
      language,
      onPasteMarkdown: async (files: File[]) => {
        const rels = await vault.pasteIntoNote(files);
        return rels.length ? rels.map(mdRef).join("\n") : null;
      },
      // A rich-text paste becomes Markdown (the clip layer's converter, formatted) when the
      // clip formatter is on; a plain-text alternative is the paste-plain-text command
      onPasteHtml: async (html: string, plain: string) => {
        if (!prefs.formatClips) return null;
        try {
          const md = await api.htmlToMarkdown(html);
          if (!md.trim() || md.trim() === plain.trim()) return null;
          // No notice: the converted text lands where the reader is looking, and a notice for
          // every routine paste would only cover the body column
          return md;
        } catch {
          return null;
        }
      },
      onOpenLink: openMdLink,
      linkSingleClick: () => prefs.clickOpensLinks,
      loadAsset,
      copyImage,
      openUrl: (url: string) => openMdLink(url, false),
      tags: () => tagList,
      fieldStats: currentFieldStats,
      blockTypes: () => types.enabled,
      // A chip's click: a date (a task date, a date-typed key, a YYYY-MM-DD value) gets the
      // date menu; everything else the value menu — the declared values first, then the
      // vault's, deduplicated, and a numeric input when the type says number or the value is one
      onFieldChip: async (_v: EditorView, pos: number, key: string, value: string, at: { x: number; y: number }, opts?: { keyboard?: boolean }) => {
        const spec = declaredField(types.enabled, key);
        if (key in DATE_FIELDS || spec.kind === "date" || fieldValueKind(value) === "date") {
          fieldMenu = { x: at.x, y: at.y, pos, key, current: value || undefined, field: (key in DATE_FIELDS ? key : "due") as DateField, keyboard: opts?.keyboard };
          return;
        }
        valueMenu = { ...(await valueMenuFor(key, value, at)), pos, keyboard: opts?.keyboard };
      },
      // A state field's chip draws its step (the same vocabulary the menu lists)
      fieldState: (key: string, value: string) => stateStepOf(types.enabled, key, value),
      blockHost: isNote ? blockHost : null,
      embedHost: isNote ? embedHost : null,
      queryHost: isNote
        ? {
            runQuery: (q: BlockQuery) => api.queryBlocks(q),
            // a row lands on its block (Agenda's jump), or on the note when the widget has no offset
            openNote: (target: string, byteStart?: number, content?: string) =>
              void (byteStart === undefined ? vault.openNote(target) : vault.openBlockAt(target, byteStart, content)),
            blockTypes: () => types.enabled,
            // a row's chip raises the same value menu as an editor chip; the pick goes to the row's note
            onRowChip: async (c: RowChipClick) => {
              valueMenu = { ...(await valueMenuFor(c.key, c.value, c.at)), pos: 0, hit: c.hit };
            },
            onRowState: (hit: BlockHit, state: TaskState) =>
              void commitTaskWrite(hit, t("task.act.state"), () => updateTask(hit, { state }), requeryBlocks),
          }
        : null,
      runHost: isNote ? { evalBlock: (code: string, params: LabParams | null) => api.scriptEvalBlock(code, path, params) } : null,
      // After ⌘S writes, a pending timer no longer stands for unsaved edits: keeping it would
      // have the flush write that stale buffer back when an external change triggers a reload,
      // covering the external edit
      onSave: (c: string) => {
        cancelAutosave();
        void vault.saveCurrent(withEol(c));
      },
      onDirty: () => {
        // Only a real edit promotes a preview tab (the loadingDoc gate stops the false
        // docChanged signals of resetDoc and setState — hanging this on the dirty setter is the
        // same race, so do not move it there)
        if (!loadingDoc) vault.pinActiveTab();
        vault.dirty = true;
        scheduleAutosave();
        scheduleOutline();
      },
      onCursor: (h: number, line: number, col: number) => {
        vault.editorCursor = h;
        vault.editorLoc = { line, col };
        const inPrivate = view != null && privateRangeAt(view.state.doc, h) != null;
        if (inPrivate !== vault.editorBlockPrivate) vault.editorBlockPrivate = inPrivate;
        const origin = view ? (originAt(view.state, h)?.origin ?? null) : null;
        if (origin !== vault.editorOrigin) vault.editorOrigin = origin;
      },
      onSelection: syncSelection,
    };
    loadingDoc = true;
    try {
      if (view) {
        resetDoc(view, opts);
      } else {
        view = createEditor(opts);
      }
      // `[!type]-` callouts start folded (a restored state keeps its own folds)
      if (isNote) foldOnLoad(view);
    } finally {
      loadingDoc = false;
    }
    finishLoad(path, !isNote && content.includes("\r\n") ? "\r\n" : "\n", 0);
  }

  /** Run `fn` now, or — while an IME composition is open — once it has ended. Every
   *  programmatic dispatch that is not part of the composition goes through here:
   *  WebKit treats a DOM or selection update under marked text as a reason to abandon
   *  the composition and re-plant it at the current caret. */
  function whenNotComposing(fn: () => void) {
    if (!view || !view.composing) return fn();
    const v = view;
    const run = () => {
      v.contentDOM.removeEventListener("compositionend", run);
      // the browser settles its own DOM first
      setTimeout(() => {
        if (view === v) fn();
      }, 0);
    };
    v.contentDOM.addEventListener("compositionend", run);
  }

  // An index update → the query blocks requery (sharing indexVersion, the one refresh signal, with the projection views)
  $effect(() => {
    const version = vault.indexVersion;
    if (view && docPath?.endsWith(".md")) whenNotComposing(() => view?.dispatch({ effects: queryRefresh.of(version) }));
  });

  // Provenance is disk truth: re-read when the index moves or an accept lands, and only
  // while the buffer is clean (a dirty buffer's blocks are not the ledger's blocks yet;
  // the field keeps mapping the last rows through the edits meanwhile)
  $effect(() => {
    void vault.indexVersion;
    void vault.provenanceEpoch;
    const path = vault.currentPath;
    const dirty = vault.dirty;
    if (!view || dirty || !path?.endsWith(".md") || docPath !== path) return;
    whenNotComposing(() => void loadProvenance(path));
  });
  $effect(() => {
    const overlay = vault.provenanceOverlay;
    const marks = prefs.provenanceMarks;
    if (view && docPath?.endsWith(".md")) whenNotComposing(() => view?.dispatch({ effects: [provOverlayEffect.of(overlay), provMarksEffect.of(marks)] }));
  });

  // Changing the body font size or the readable line width in the settings (CM cannot notice a CSS variable change) → remeasure the line heights and coordinates
  $effect(() => {
    void prefs.noteFontSize;
    void prefs.readableLineWidth;
    view?.requestMeasure();
  });

  $effect(() => {
    // (re)load the document when the open file changes or is reloaded externally
    const counter = vault.reloadCounter;
    const content = vault.currentContent;
    const path = vault.currentPath;
    if (!container || !path) {
      // the {#if} removed the editor DOM (open file closed/deleted); the view
      // is attached to that dead subtree, so drop it or later files render blank
      // flush rather than cancel: the ordinary note → image/PDF preview switch takes this branch
      // too, and cancel would silently drop the last second of input. The dangerous cases already
      // have their gates — after a deletion the old path is not in the index (the flush's
      // existence guard stops the write-back), and a vault switch has already flushed at the top
      // of openVault (by then the timer is empty and this is naturally a no-op)
      flushAutosave();
      view?.destroy();
      view = null;
      loadedFor = -1;
      docPath = null;
      clearDocSignals();
      return;
    }
    if (counter === loadedFor) return;
    flushAutosave(); // before switching documents or an external reload, write the old document's unsaved edits out
    loadedFor = counter;
    void loadDoc(counter, path, content);
  });

  onDestroy(() => {
    if (vault.flushEditor === flushAutosave) vault.flushEditor = null;
    vault.snapshotEditor = null;
    if (vault.insertAtCursor === insertAtCursor) vault.insertAtCursor = null;
    if (vault.formatDoc === formatDoc) vault.formatDoc = null;
    if (vault.insertAtSection === insertAtSection) vault.insertAtSection = null;
    if (vault.revealPos === revealPos) vault.revealPos = null;
    vault.blockCommand = null;
    vault.stepPending = null;
    vault.reviewRun = null;
    vault.rejectProvenance = null;
    vault.locateText = null;
    vault.copyBlockRef = null;
    clearDocSignals();
    unlistenLocalized?.();
    view?.destroy();
  });
</script>

<svelte:window onblur={flushAutosave} />

{#if vault.currentPath}
  <!-- The tab bar (TabBar) is not here: it lives permanently at the top of App's .center and
    does not disappear under a projection view (only this component is hidden with the
    editor-host) -->
  <div class="editor-wrap">
    {#if vault.currentKind === "image" || vault.currentKind === "pdf"}
      {#key vault.currentPath}
        <FilePreview path={vault.currentPath} />
      {/key}
    {:else}
      <!-- svelte-ignore a11y_no_static_element_interactions -->
      <div class="editor" bind:this={container} oncontextmenu={editorMenu}></div>
    {/if}
  </div>
{:else}
  <div class="empty">
    <p>{t("editor.welcomeKeys", { open: k("Mod+O"), palette: k("Mod+P") })}</p>
  </div>
{/if}

{#if fieldMenu}
  <TaskDateMenu
    x={fieldMenu.x}
    y={fieldMenu.y}
    field={fieldMenu.field}
    title={fieldMenu.key in DATE_FIELDS ? undefined : fieldMenu.key}
    current={fieldMenu.current}
    keyboard={fieldMenu.keyboard}
    onpick={applyFieldDate}
    onclose={closeFieldMenu}
  />
{/if}
{#if valueMenu}
  <FieldValueMenu
    x={valueMenu.x}
    y={valueMenu.y}
    key={valueMenu.key}
    current={valueMenu.current}
    values={valueMenu.values}
    number={valueMenu.number}
    steps={valueMenu.state}
    onpick={applyFieldValue}
    onclose={closeValueMenu}
  />
{/if}

<style>
  .editor-wrap {
    display: flex;
    flex-direction: column;
    height: 100%;
    min-width: 0;
  }
  .editor {
    flex: 1;
    min-height: 0;
  }
  .empty {
    height: 100%;
    display: grid;
    place-items: center;
    color: var(--fg-faint);
    font-size: var(--fs-sm);
  }
</style>
