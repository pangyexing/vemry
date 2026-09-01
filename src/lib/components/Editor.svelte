<script lang="ts">
  import { t } from "$lib/i18n/i18n.svelte";
  import { onDestroy } from "svelte";
  import { listen } from "@tauri-apps/api/event";
  import { EditorView } from "@codemirror/view";
  import type { EditorState } from "@codemirror/state";
  import { createEditor, resetDoc, languageFor } from "$lib/editor/setup";
  import { queryRefresh } from "$lib/editor/queryblock";
  import { suppressReveal } from "$lib/editor/reveal";
  import { ensureSyntaxTree } from "@codemirror/language";
  import { linkAt } from "$lib/editor/linkat";
  import { vault, fileKind, mimeOf } from "$lib/stores/vault.svelte";
  import { agent } from "$lib/stores/agent.svelte";
  import { prefs } from "$lib/stores/prefs.svelte";
  import { toast } from "$lib/stores/toast.svelte";
  import { api, SELECTION_MAX_CHARS, trimLoneSurrogate, type BlockQuery } from "$lib/api";
  import { planInsert, planCursorInsert } from "$lib/insert";
  import { extractOutline } from "$lib/outline";
  import { countWords } from "$lib/stats";
  import { k } from "$lib/shortcut";
  import { executeCommand } from "$lib/commands/registry";
  import FilePreview from "./FilePreview.svelte";
  import { contextMenu, type MenuEntry } from "$lib/stores/contextMenu.svelte";
  import { clearImageDims } from "$lib/editor/livepreview";

  let container = $state<HTMLDivElement | null>(null);
  let view: EditorView | null = null;
  let knownTargets = new Set<string>();
  let targetList: string[] = [];
  let tagList: string[] = [];
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
   *  the notes index, so otherFiles is consulted. */
  function docExists(path: string): boolean {
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
    view.dispatch({
      changes: { from: plan.from, to: plan.to, insert: plan.insert },
      selection: { anchor: plan.caret },
      scrollIntoView: true,
    });
    view.focus();
    return true;
  }
  vault.insertAtCursor = insertAtCursor;

  /** Insert at the end of a section (used when the target is the note that is open and a
   *  destination section was given; the section wins over the cursor).
   *  The positioning and separator rules live in insert.ts, on the same table of cases as
   *  Rust's insert.rs; a missing heading falls back to the end of the file, and the return
   *  value tells the caller to say so. */
  function insertAtSection(heading: string, text: string): "section" | "tail" | false {
    if (!view || !docPath?.endsWith(".md") || docPath !== vault.currentPath) return false;
    const plan = planInsert(view.state.doc.toString(), heading, text);
    view.dispatch({
      changes: { from: plan.from, to: plan.to, insert: plan.insert },
      selection: { anchor: plan.caret },
      scrollIntoView: true,
    });
    view.focus();
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
  function revealPos(pos: number): boolean {
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
    view.dispatch({
      selection: { anchor: p },
      effects: EditorView.scrollIntoView(p, { y: "start", yMargin: REVEAL_TOP_MARGIN }),
      annotations: suppressReveal.of(true),
    });
    view.focus();
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
    const cur = vault.editorSelection;
    if (cur && cur.path === docPath && cur.from_line === a.number && cur.to_line === b.number && cur.text === text) return;
    vault.editorSelection = { path: docPath, from_line: a.number, to_line: b.number, text };
  }

  /** The buffer sync for a clip's image localisation (clip://localized). The rewrite happens on
   *  disk, but a dirty buffer is not reloaded by NoteModified — the next autosave would quietly
   *  wipe the rewrite out. So the same substitution is applied to the live buffer (touching only
   *  the target of `![..](target)`, matching the Rust side's rewrite_images); a clean buffer has
   *  already been reloaded and the substitution naturally does nothing. */
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
    if (changes.length) view.dispatch({ changes });
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
    tagList = [...counts.keys()].sort((a, b) => counts.get(b)! - counts.get(a)! || a.localeCompare(b));
  });

  const host = {
    isKnown: (t: string) => knownTargets.has(t),
    targets: () => targetList,
    follow: (t: string) => void vault.followLink(t),
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
        .sort((a, b) => a.length - b.length || a.localeCompare(b))[0] ?? null;
    }
    return hit && fileKind(hit) === "image" ? hit : null;
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
    view.focus();
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
      view.focus();
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
    const priv = vault.privatePaths.has(path);
    entries.push(
      { label: t("editor.cut"), hint: k("Mod+X"), disabled: !hasSel, action: cutSelection },
      { label: t("editor.copy"), hint: k("Mod+C"), disabled: !hasSel, action: () => void copySelection() },
      { label: t("editor.paste"), hint: k("Mod+V"), action: () => void pasteFromMenu() },
      { separator: true },
      { label: t("tabBar.revealInTree"), action: () => void executeCommand("reveal-in-tree") },
      {
        label: t(priv ? "fileTree.unmarkPrivate" : "fileTree.markPrivate"),
        action: () => void executeCommand(priv ? "unmark-private" : "mark-private"),
      },
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

  /** Finishing a load: shared by the restore-a-suspended-tab path and the create-a-new-state
   *  path. Adding a store field derived from the editor changes this one place — adding
   *  editorLoc and editorSelStats in this batch would once have meant keeping four sites in
   *  step, and missing one is stale data. With head=0, lineAt(0) gives exactly 1:1, which
   *  matches the old hardcoded value. */
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
    vault.editorCursor = head;
    const ln = v.state.doc.lineAt(head);
    vault.editorLoc = { line: ln.number, col: head - ln.from + 1 };
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
    vault.editorSelection = null;
    vault.editorSelStats = null;
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
      onOpenLink: openMdLink,
      linkSingleClick: () => prefs.clickOpensLinks,
      loadAsset,
      copyImage,
      tags: () => tagList,
      queryHost: isNote
        ? {
            runQuery: (q: BlockQuery) => api.queryBlocks(q),
            openNote: (target: string) => void vault.openNote(target),
          }
        : null,
      runHost: isNote ? { evalBlock: (code: string) => api.scriptEvalBlock(code, path) } : null,
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
    } finally {
      loadingDoc = false;
    }
    finishLoad(path, !isNote && content.includes("\r\n") ? "\r\n" : "\n", 0);
  }

  // An index update → the query blocks requery (sharing indexVersion, the one refresh signal, with the projection views)
  $effect(() => {
    const version = vault.indexVersion;
    if (view && docPath?.endsWith(".md")) view.dispatch({ effects: queryRefresh.of(version) });
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
    if (vault.insertAtSection === insertAtSection) vault.insertAtSection = null;
    if (vault.revealPos === revealPos) vault.revealPos = null;
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
