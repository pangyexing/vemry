<script module lang="ts">
  /** The short label of a selection's line range: `(12-20)`, or `(12)` for one line — shared
   *  by the candidate rows, the composer chips and a user reference in the timeline
   *  (AgentChat imports it from here). */
  export function linesLabel(sel: { from_line: number; to_line: number }): string {
    return sel.from_line === sel.to_line ? `(${sel.from_line})` : `(${sel.from_line}-${sel.to_line})`;
  }
</script>

<script lang="ts">
  // The composer's @ candidate overlay: the current selection, notes, folders, text
  // attachments, the current web page and the current reading page.
  // Every candidate comes from data the frontend already has in its stores, and private
  // paths are excluded by an exact privatePaths `.has()` (no prefix scan — that would
  // wrongly kill a `!` negative exemption; this is only a UI courtesy, and the security
  // boundary is the privacy gate on the Rust side at send time). Images (png/jpg/gif/webp)
  // enter as image references — only the path travels and Rust fills it in when assembling
  // the request; a PDF enters as a file reference (Rust extracts its text); no other binary
  // enters at all.
  // The focus always stays in the textarea — the keyboard events are forwarded by AgentChat
  // through handleKey, and a mouse press calls preventDefault to keep the focus.
  import { t } from "$lib/i18n/i18n.svelte";
  import { isAgentImage } from "$lib/api";
  import RefIcon, { type RefIconKind } from "./RefIcon.svelte";
  import { agent, type DraftRef } from "$lib/stores/agent.svelte";
  import { vault, baseOf, parentOf, fileKind } from "$lib/stores/vault.svelte";
  import type { EditorSelection } from "$lib/api";
  import { browser } from "$lib/stores/browser.svelte";
  import { reading } from "$lib/stores/reading.svelte";
  import { fuzzyFilter } from "$lib/commands/registry";
  import { isImeKeydown } from "$lib/ime";
  import { blocksOf } from "$lib/citation";
  import { blockLabel, isAnchorableHit } from "$lib/editor/blockref";
  import { blockHandle } from "$lib/stores/agent.svelte";
  import type { BlockHit } from "$lib/api";

  let {
    query,
    anchor,
    curSel,
    onPick,
    onPickFiles,
    onStage,
    onClose,
  }: {
    query: string;
    /** The composer's textarea: the candidate layer measures the viewport rect of the
     *  .composer it sits in and positions itself fixed — absolute would be clipped away
     *  entirely by the overlay container's overflow:hidden (while idle the overlay is only as
     *  tall as the composer, with no room above inside the container), and fixed is clipped by
     *  nothing.
     *  The precondition: no transform or filter on the ancestor chain (either would make that
     *  ancestor the containing block of a fixed element, shifting the coordinates and putting
     *  it back inside the overflow clipping) — so no host container of a conversation surface
     *  may position with translate or the like. */
    anchor: HTMLElement | null;
    /** The editor's current selection (the source of the `@ current selection` candidate).
     *  It is passed down by AgentChat — the "the main window reads the editor / the chat
     *  window reads the main window's push" branch is decided there, once. */
    curSel: EditorSelection | null;
    onPick: (ref: DraftRef) => void;
    /** The candidates' first row, "add a file from this computer…": the native file dialog is opened by AgentChat (which owns the writing and the chips). */
    onPickFiles: () => void;
    /** Tab (or `#`) on a note row: AgentChat rewrites the @token to `@<notePart>#`, which
     *  opens that note's block stage (the picker remembers which row was highlighted, see
     *  `staged`). notePart is the note's stem when it can live in a token, else what was typed. */
    onStage: (notePart: string) => void;
    onClose: () => void;
  } = $props();

  let pos = $state<{ left: number; width: number; bottom: number; maxH: number } | null>(null);
  function measure() {
    const box = anchor?.closest(".composer");
    if (!(box instanceof HTMLElement)) return;
    const r = box.getBoundingClientRect();
    pos = {
      left: r.left + 12, // aligned with the composer's padding (12)
      width: Math.max(220, r.width - 24),
      bottom: window.innerHeight - r.top + 4,
      // With less than 40vh of room above, it caps at what is really left so the candidate layer does not push out of the viewport
      maxH: Math.max(120, Math.min(window.innerHeight * 0.4, r.top - 12)),
    };
  }
  $effect(() => {
    void query; // remeasured whenever the filter text changes: auto-growing moves the composer's top edge
    measure();
  });

  const MAX_SHOW = 12;

  type Cand =
    | { kind: "disk"; title: string; sub: string }
    | { kind: "selection"; title: string; sub: string; sel: EditorSelection }
    | { kind: "page-browser"; title: string; sub: string }
    | { kind: "page-reading"; title: string; sub: string }
    | { kind: "note" | "folder" | "file" | "image"; title: string; sub: string; path: string }
    /** The `@Note#` stage: the whole note, then its headings and anchorable blocks. */
    | { kind: "whole"; title: string; sub: string; path: string }
    | { kind: "block"; title: string; sub: string; path: string; local_id: string; heading: boolean };

  /** The string fuzzy filtering matches against: a path-like candidate goes by path plus
   *  title, and the current page or selection by label plus title (a selection also carries
   *  its path and the words for "selection", so both `@选` and `@sel` match).
   *  The case is folded ahead of time: `base` is only recomputed when the vault's data
   *  changes, and each keystroke only runs the filtering. */
  function keyOf(c: Cand): string {
    if ("path" in c) return `${c.path} ${c.title}`;
    if (c.kind === "selection") return `选区 selection ${c.sel.path} ${c.title}`;
    return `${c.sub} ${c.title}`;
  }

  /** The dynamic candidates pinned to the top (the disk entry, the current selection, the
   *  current page): at most 4 rows, following the selection and the page, computed apart
   *  from the vault-wide table below — a drag must not drag a thousand rows through a
   *  rebuild. */
  const head = $derived.by(() => {
    const out: Cand[] = [];
    // The other half of "+": files outside the vault come in here
    out.push({ kind: "disk", title: t("atPicker.fromDisk"), sub: t("atPicker.fromDiskSub") });
    // "The current selection" comes first: having selected a passage and then typed @, it is
    // almost certainly what is wanted. With a selection the chips row already carries a live
    // chip — choosing it here pins it as an explicit chip (so it is not lost when the
    // selection changes). A selection inside a private note does not appear as a candidate at
    // all (a UI courtesy; the real gate is in Rust)
    if (curSel && !vault.privatePaths.has(curSel.path)) {
      out.push({ kind: "selection", title: `${baseOf(curSel.path)} ${linesLabel(curSel)}`, sub: t("atPicker.selectionSub"), sel: curSel });
    }
    // The two "current page" entries are pinned to the top: the context entry points of anywhere, filtered fuzzily like the rest
    if (browser.current) {
      out.push({ kind: "page-browser", title: browser.current.title || browser.current.url, sub: t("atPicker.pageBrowserSub") });
    }
    if (reading.current) {
      out.push({ kind: "page-reading", title: reading.current.title || reading.current.url, sub: t("atPicker.pageReadingSub") });
    }
    return out.map((c) => ({ c, key: keyOf(c) }));
  });
  /** The vault-wide part of the candidate set (before filtering): its only dependency is the
   *  vault's lists, and it is independent of `query` and the selection — with a thousand
   *  notes in a vault, rebuilding this table on every keystroke or every selection change
   *  costs visibly. */
  const base = $derived.by(() => {
    const out: Cand[] = [];
    const priv = vault.privatePaths;
    for (const n of vault.notes) {
      if (!priv.has(n.id)) out.push({ kind: "note", title: n.title, sub: parentOf(n.id), path: n.id });
    }
    for (const f of vault.folders) {
      if (!priv.has(f)) out.push({ kind: "folder", title: baseOf(f), sub: parentOf(f), path: f });
    }
    for (const f of vault.otherFiles) {
      if (priv.has(f)) continue;
      const kind = fileKind(f);
      if (kind === "text" || kind === "pdf") out.push({ kind: "file", title: baseOf(f), sub: parentOf(f), path: f });
      else if (kind === "image" && isAgentImage(f)) out.push({ kind: "image", title: baseOf(f), sub: parentOf(f), path: f });
    }
    return out.map((c) => ({ c, key: keyOf(c) }));
  });
  // ---- the `@Note#` stage: a `#` in the query switches to the blocks of one note ----
  type PathCand = Extract<Cand, { path: string }>;
  const hashAt = $derived(query.indexOf("#"));
  /** The row that Tab or `#` took into the stage: the highlighted note, remembered together
   *  with the note part written into the token. As long as that note part stays as written,
   *  the stage is about this note — a row moved to with ↓ has to be honoured, and the same
   *  stem can exist in two folders (or, when the stem could not be written, the note part is
   *  still the typed `手测`), so deriving it from the text again could bite another note.
   *  Editing the note part falls back to the derivation, and leaving the stage (deleting the
   *  `#`) forgets it. */
  let staged = $state<{ notePart: string; path: string } | null>(null);
  $effect(() => {
    if (hashAt < 0) staged = null;
  });
  /** The note the stage is about: the staged row, else an exact stem or title match, else
   *  the best fuzzy one; an empty note part (`@#`) means the current note. null = no such note. */
  const stageNote = $derived.by(() => {
    if (hashAt < 0) return null;
    if (staged && query.slice(0, hashAt) === staged.notePart) return staged.path;
    const q = query.slice(0, hashAt).trim().toLowerCase();
    if (!q) return vault.currentPath && !vault.privatePaths.has(vault.currentPath) ? vault.currentPath : null;
    const notes = base.filter((e) => e.c.kind === "note");
    const exact = notes.find((e) => {
      const c = e.c as PathCand;
      const stem = baseOf(c.path).replace(/\.md$/i, "").toLowerCase();
      return stem === q || c.title.toLowerCase() === q || c.path.toLowerCase() === q;
    });
    const hit = exact ?? fuzzyFilter(notes, q, (e) => e.key)[0];
    return hit ? (hit.c as PathCand).path : null;
  });
  let stageBlocks = $state<BlockHit[]>([]);
  $effect(() => {
    const path = stageNote;
    const version = vault.indexVersion;
    if (!path) {
      stageBlocks = [];
      return;
    }
    let live = true;
    void blocksOf(path, version).then((hits) => {
      if (live) stageBlocks = hits;
    });
    return () => {
      live = false;
    };
  });
  const stageCands = $derived.by((): Cand[] | null => {
    const path = stageNote;
    if (hashAt < 0) return null;
    if (!path) return [];
    const q = query.slice(hashAt + 1).trim();
    const out: { c: Cand; key: string }[] = [
      { c: { kind: "whole", title: t("atPicker.wholeNote"), sub: baseOf(path), path }, key: t("atPicker.wholeNote") },
    ];
    for (const h of stageBlocks) {
      if (!isAnchorableHit(h)) continue;
      const heading = h.block.kind.type === "heading";
      const title = blockLabel(h.block.content);
      out.push({
        c: { kind: "block", title, sub: heading ? "›" : `¶${blockHandle(h.block.local_id)}`, path, local_id: h.block.local_id, heading },
        key: title,
      });
    }
    // the whole-note row always stays on top
    const [whole, ...rest] = out;
    return [whole.c, ...fuzzyFilter(rest, q, (e) => e.key).map((e) => e.c)].slice(0, MAX_SHOW);
  });
  const cands = $derived(stageCands ?? fuzzyFilter([...head, ...base], query, (e) => e.key).slice(0, MAX_SHOW).map((e) => e.c));

  let sel = $state(0);
  $effect(() => {
    void query;
    sel = 0; // after typing more of the filter, the selection starts from the top again
  });
  const selIdx = $derived(Math.max(0, Math.min(sel, cands.length - 1)));
  /** The highlighted row when it is a note outside the stage — the one kind Tab can drill into. */
  const drillable = $derived.by(() => {
    const c = cands[selIdx];
    return hashAt < 0 && c?.kind === "note" ? c : null;
  });
  /** The key legend follows what the keys do right now: ⇥ is offered on a note row, and the
   *  stage says how to get back (deleting the `#` returns to the notes). */
  const keysHint = $derived(stageNote ? t("atPicker.keysStage") : drillable ? t("atPicker.keysDrill") : t("atPicker.keys"));

  let listEl = $state<HTMLDivElement | null>(null);
  $effect(() => {
    // Scroll along as the keyboard moves the selection (the list only scrolls internally once it is past its cap)
    void selIdx;
    const row = listEl?.children[selIdx];
    if (row instanceof HTMLElement) row.scrollIntoView({ block: "nearest" });
  });

  async function pick(c: Cand) {
    if (c.kind === "disk") {
      onPickFiles();
      return;
    }
    if (c.kind === "selection") {
      onPick({ kind: "selection", ...c.sel });
      return;
    }
    if (c.kind === "page-browser") {
      const page = await agent.grabPage();
      if (page) onPick({ kind: "page", page });
      else onClose();
      return;
    }
    if (c.kind === "page-reading") {
      const page = reading.pageContext();
      if (page) onPick({ kind: "page", page });
      else onClose();
      return;
    }
    if (c.kind === "whole") {
      onPick({ kind: "note", path: c.path });
      return;
    }
    if (c.kind === "block") {
      onPick({ kind: "block", path: c.path, local_id: c.local_id, label: c.title });
      return;
    }
    onPick({ kind: c.kind, path: c.path });
  }

  function iconKind(c: Cand): RefIconKind {
    if (c.kind === "disk") return "file";
    if (c.kind === "page-browser") return "page";
    if (c.kind === "page-reading") return "reading";
    if (c.kind === "whole") return "note";
    return c.kind;
  }

  /** Forwarded in from AgentChat's composer onkeydown; true = consumed.
   *  Enter and Esc both pass the IME guard: confirming or cancelling an input-method
   *  candidate is neither selecting nor closing. */
  export function handleKey(e: KeyboardEvent): boolean {
    if (isImeKeydown(e)) return false;
    if (e.key === "ArrowDown") {
      e.preventDefault();
      sel = Math.min(selIdx + 1, cands.length - 1);
      return true;
    }
    if (e.key === "ArrowUp") {
      e.preventDefault();
      sel = Math.max(selIdx - 1, 0);
      return true;
    }
    if (e.key === "Escape") {
      // stopPropagation: this Esc only closes the candidates and does not trigger App's layered Esc or collapse the overlay
      e.preventDefault();
      e.stopPropagation();
      onClose();
      return true;
    }
    // Tab on a note row drills into its blocks rather than confirming — the same convention as
    // the palette's pick-note stage; on any other row (a leaf: a folder, a file, the disk entry,
    // a block in the stage) there is nothing to drill into and Tab confirms like Enter.
    // Typing `#` after moving to a note row takes that row along too — otherwise the stage would
    // be derived from the text alone and open the first fuzzy hit.
    if ((e.key === "Tab" || e.key === "#") && drillable) {
      e.preventDefault();
      // The token is completed to the note's stem (`@手测` → `@手测夹具#`), like a shell
      // completion, so the composer shows which note the blocks belong to. A stem AT_RE cannot
      // hold — a `@` or `#` in it, or whitespace other than single inner spaces — leaves the
      // typed text in place; the stage is still right through `staged`.
      const stem = baseOf(drillable.path).replace(/\.md$/i, "");
      const notePart = /^[^\s@#]+(?: [^\s@#]+)*$/.test(stem) ? stem : query;
      staged = { notePart, path: drillable.path };
      onStage(notePart);
      return true;
    }
    if ((e.key === "Enter" || e.key === "Tab") && cands.length) {
      e.preventDefault();
      void pick(cands[selIdx]);
      return true;
    }
    if (e.key === "Enter") {
      onClose(); // with no candidate, the @token goes out with the message as written
      return false;
    }
    return false;
  }
</script>

<svelte:window onresize={measure} />

<!-- svelte-ignore a11y_no_static_element_interactions -->
<div
  class="at-picker"
  style={pos ? `left:${pos.left}px; width:${pos.width}px; bottom:${pos.bottom}px; max-height:${pos.maxH}px` : "visibility:hidden"}
  onmousedown={(e) => e.preventDefault()}
>
  <div class="at-head">
    <span
      >{stageNote
        ? t("atPicker.blocksOf", { note: baseOf(stageNote) })
        : query
          ? t("atPicker.headingWithQuery", { query })
          : t("atPicker.heading")}</span
    >
    <span class="at-keys">{keysHint}</span>
  </div>
  <div class="at-list" bind:this={listEl}>
    {#if cands.length === 0}
      <div class="at-empty">{t("atPicker.empty")}</div>
    {/if}
    {#each cands as c, i (c.kind === "block" ? `block:${c.path}#${c.local_id}` : "path" in c ? `${c.kind}:${c.path}` : c.kind)}
      <button
        class="at-row"
        class:sel={i === selIdx}
        title={"path" in c ? c.path : c.kind === "selection" ? c.sel.path : c.title}
        onmouseenter={() => (sel = i)}
        onclick={() => void pick(c)}
      >
        {#if c.kind === "disk"}
          <span class="at-icon at-plus" aria-hidden="true">+</span>
        {:else}
          <span class="at-icon" class:page={!("path" in c)}><RefIcon kind={iconKind(c)} size={14} /></span>
        {/if}
        <span class="at-title" class:heading={c.kind === "block" && c.heading}>{agent.grabbingPage && c.kind === "page-browser" ? t("atPicker.extracting") : c.title}{c.kind === "folder" ? "/" : ""}</span>
        {#if c.sub}<span class="at-sub" class:tag={!("path" in c)}>{c.sub}</span>{/if}
      </button>
    {/each}
  </div>
</div>

<style>
  .at-picker {
    position: fixed; /* the coordinates are computed by the script from .composer's viewport rect */
    z-index: 95; /* above the overlay (91), below the command palette (100) and the permission dialog (200) */
    background: var(--bg-panel);
    border: 1px solid var(--border);
    border-radius: 8px;
    box-shadow: var(--shadow);
    display: flex;
    flex-direction: column;
    overflow: hidden;
  }
  .at-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
    flex-shrink: 0;
    padding: 6px 10px 5px;
    border-bottom: 1px solid var(--border);
    font-size: 10px;
    color: var(--fg-faint);
    letter-spacing: 0.04em;
    -webkit-user-select: none;
    user-select: none;
    min-width: 0;
  }
  .at-head > span:first-child {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .at-keys {
    flex-shrink: 0;
    font-family: var(--font-mono);
    letter-spacing: 0;
  }
  .at-list {
    overflow-y: auto;
    padding: 4px;
    display: flex;
    flex-direction: column;
    gap: 1px;
    min-height: 0;
  }
  .at-empty {
    font-size: var(--fs-xs);
    color: var(--fg-faint);
    padding: 8px;
  }
  .at-row {
    display: flex;
    align-items: center;
    gap: 8px;
    width: 100%;
    text-align: left;
    padding: 5px 8px;
    border-radius: 6px;
    min-width: 0;
  }
  .at-row.sel {
    background: var(--accent-soft);
  }
  .at-icon {
    flex-shrink: 0;
    display: flex;
    color: var(--fg-muted);
  }
  .at-plus {
    justify-content: center;
    font-size: 15px;
    line-height: 1;
    color: var(--fg-muted);
  }
  .at-icon.page {
    color: var(--accent);
  }
  .at-title {
    flex-shrink: 1;
    min-width: 0;
    font-size: var(--fs-sm);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .at-sub {
    flex: 1;
    min-width: 0;
    font-size: 11px;
    color: var(--fg-faint);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .at-title.heading {
    font-weight: 600;
  }
  .at-sub.tag {
    flex: none;
    margin-left: auto;
    font-size: 10px;
    color: var(--accent);
  }
</style>
