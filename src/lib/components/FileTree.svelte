<script lang="ts">
  import { t, tn } from "$lib/i18n/i18n.svelte";
  import { tick } from "svelte";
  import { api } from "$lib/api";
  import { executeCommand } from "$lib/commands/registry";
  import { vault, type FileOp, baseOf, vaultName } from "$lib/stores/vault.svelte";
  import { isImeKeydown } from "$lib/ime";
  import { isMac, k } from "$lib/shortcut";
  import type { NoteSummary } from "$lib/api";
  import { contextMenu, type MenuEntry } from "$lib/stores/contextMenu.svelte";
  import { toast } from "$lib/stores/toast.svelte";
  import { copyText } from "$lib/copytext";
  import Chevron from "./Chevron.svelte";

  interface TreeNode {
    name: string;
    path: string;
    folders: TreeNode[];
    notes: NoteSummary[];
    others: string[];
  }

  type EditMode = "create-note" | "create-file" | "create-folder" | "rename";
  type RowKind = "note" | "file" | "folder";
  interface Editing {
    mode: EditMode;
    parent: string | null;
    /** rename only: full relative path + what it is */
    target?: string;
    targetKind?: RowKind;
  }

  /** Whether the header's vault switcher dropdown is open */
  let switcher = $state(false);
  /** The switcher's list: when the current vault did not enter recent because of the
   *  entry check (a directory of nothing but pdf/txt, say), it still needs a row of its
   *  own — the tick and the note count hang on it */
  const switcherVaults = $derived(
    vault.root && !vault.recent.includes(vault.root) ? [vault.root, ...vault.recent] : vault.recent,
  );
  let treeEl = $state<HTMLElement | null>(null);
  /** Where a clipboard paste lands: the last clicked folder / a clicked
   *  file's parent / "" for the vault root. */
  let pasteDir = $state("");
  let lastPasteAt = 0;
  /** The context menu (sharing the contextMenu store): entries are computed for the
   *  context at the moment of openMenu (undoLabel, the paste wording and the privacy state
   *  are all snapshots), while closing, Esc and clamping belong to the component. */
  let editing = $state<Editing | null>(null);
  let editValue = $state("");
  let editInput = $state<HTMLInputElement | null>(null);
  /** The row ⌘X cut to be moved; cleared after a paste or Escape */
  let cut = $state<{ path: string; kind: RowKind } | null>(null);
  /** Pointer drag-moving: pressed is a candidate row held down but not yet past the movement threshold */
  let pressed: { path: string; kind: RowKind; x: number; y: number } | null = null;
  let drag = $state<{ path: string; kind: RowKind; name: string; x: number; y: number } | null>(null);
  /** The valid drop directory currently hovered while dragging ("" = the vault root; null = none) */
  let dropTarget = $state<string | null>(null);
  /** The click right after a drag no longer counts as opening or collapsing */
  let suppressClick = false;
  let hoverExpandTimer = 0;

  /** The agent's constitution file at the vault root (injected into the system prompt), pinned at the top of the tree */
  const MEMORY_ID = "VEMRY.md";
  let hasMemory = $derived(vault.notes.some((n) => n.id === MEMORY_ID));

  let tree = $derived.by(() => {
    const root: TreeNode = { name: "", path: "", folders: [], notes: [], others: [] };
    const nodeOf = (folderPath: string): TreeNode => {
      if (!folderPath) return root;
      const parts = folderPath.split("/");
      let node = root;
      for (let i = 0; i < parts.length; i++) {
        const path = parts.slice(0, i + 1).join("/");
        let child = node.folders.find((f) => f.path === path);
        if (!child) {
          child = { name: parts[i], path, folders: [], notes: [], others: [] };
          node.folders.push(child);
          node.folders.sort((a, b) => a.name.localeCompare(b.name, "zh"));
        }
        node = child;
      }
      return node;
    };
    for (const folder of vault.folders) nodeOf(folder);
    for (const note of vault.notes) {
      if (note.id === MEMORY_ID) continue; // the pinned row is rendered separately and does not enter the ordinary tree
      const idx = note.id.lastIndexOf("/");
      const node = nodeOf(idx === -1 ? "" : note.id.slice(0, idx));
      node.notes.push(note);
      node.notes.sort((a, b) => a.id.localeCompare(b.id, "zh"));
    }
    for (const file of vault.otherFiles) {
      const idx = file.lastIndexOf("/");
      const node = nodeOf(idx === -1 ? "" : file.slice(0, idx));
      node.others.push(file);
      node.others.sort((a, b) => a.localeCompare(b, "zh"));
    }
    return root;
  });

  // Opening any document (from the command palette, a backlink, the graph, the history, a
  // click in the tree) → expand its ancestors and scroll to that row.
  // It runs on the first mount too: opening a note with the left column closed and then
  // pressing ⌘B reveals it just the same.
  $effect(() => {
    const path = vault.currentPath; // these two are the only dependencies — expanded must not be read,
    void vault.revealTick; // the manual retrigger of "reveal in the file tree" (when currentPath did not change)
    if (!path) return; // or manually collapsing the current note's ancestors would be bounced straight back
    const dir = parentOf(path);
    if (dir) vault.expandFolder(dir);
    void tick().then(() => {
      // A linear dataset comparison rather than an attribute selector: a file name may hold
      // quotes or backslashes, and CSS.escape is not complete for the attribute-value
      // context; a tree is a few hundred rows at most, so the cost is negligible
      const row =
        treeEl &&
        [...treeEl.querySelectorAll<HTMLElement>("[data-path]")].find((el) => el.dataset.path === path);
      row?.scrollIntoView({ block: "nearest" });
    });
  });

  function stemOf(path: string): string {
    return (path.split("/").pop() ?? path).replace(/\.md$/, "");
  }
  function parentOf(path: string): string | null {
    const i = path.lastIndexOf("/");
    return i === -1 ? null : path.slice(0, i);
  }
  function extOf(path: string): string {
    const base = baseOf(path);
    const i = base.lastIndexOf(".");
    return i > 0 ? base.slice(i + 1).toUpperCase() : "";
  }
  /** An attachment row shows the stem without its extension, leaving the type to the badge on the right */
  function stemNoExt(path: string): string {
    const base = baseOf(path);
    const i = base.lastIndexOf(".");
    return i > 0 ? base.slice(0, i) : base;
  }

  function openMenu(e: MouseEvent, folder: string | null, rename?: { path: string; kind: "note" | "file" | "folder" }) {
    e.preventDefault();
    e.stopPropagation();
    focusRow(e);
    switcher = false;
    pasteDir = folder ?? ""; // the side effect is kept on purpose: a paste lands where the right-click was
    contextMenu.open(e, buildMenu(folder, rename));
  }

  /** The row and empty-space menu (behaving exactly as the old inline version, with the dynamic undo and paste wording). */
  function buildMenu(folder: string | null, rename?: { path: string; kind: "note" | "file" | "folder" }): MenuEntry[] {
    const entries: MenuEntry[] = [];
    if (folder) entries.push({ title: `${folder}/` });
    if (vault.undoOps.length) {
      entries.push(
        { label: undoLabel(vault.undoOps[vault.undoOps.length - 1]), hint: k("Mod+Z"), action: () => void vault.undoFileOp() },
        { separator: true },
      );
    }
    entries.push(
      { label: t("fileTree.newNote"), action: () => startCreate("create-note", folder) },
      { label: t("fileTree.newFile"), action: () => startCreate("create-file", folder) },
      { label: t("fileTree.newFolder"), action: () => startCreate("create-folder", folder) },
      {
        label: cut ? t("fileTree.pasteMove", { name: baseOf(cut.path) }) : t("fileTree.paste"),
        hint: k("Mod+V"),
        action: () => void pasteClipboard(),
      },
    );
    if (rename) {
      const r = rename;
      entries.push(
        { separator: true },
        { label: t("fileTree.rename"), hint: "F2", action: () => beginRename(r.path, r.kind) },
        { label: t("fileTree.cut"), hint: k("Mod+X"), action: () => { cut = { path: r.path, kind: r.kind }; } },
        { label: t("fileTree.copyPath"), action: () => void copyPath(r.path) },
        { label: t("fileTree.copyFullPath"), action: () => void copyPath(r.path, true) },
        { label: vault.privatePaths.has(r.path) ? t("fileTree.unmarkPrivate") : t("fileTree.markPrivate"), action: () => togglePrivacy(r) },
      );
      if (r.kind === "file") {
        entries.push({ label: t("fileTree.openWithSystemApp"), action: () => void api.fileOpenSystem(r.path).catch((err) => (vault.error = String(err))) });
      }
      entries.push(deleteEntry(r.path));
    }
    return entries;
  }

  /** The one definition of the "delete" menu item — shared by an ordinary row and the pinned
   *  VEMRY.md row, so the wording, the danger state and the shortcut hint change in one place. */
  function deleteEntry(path: string): MenuEntry {
    return { label: t("fileTree.delete"), danger: true, hint: isMac ? "⌘⌫" : "Delete", action: () => doDelete(path) };
  }

  function switchTo(path: string) {
    switcher = false;
    if (path !== vault.root) void vault.openVault(path);
  }

  /** Copy the vault-relative path or the absolute path. An absolute path is joined in the
   *  root's separator style: a Windows root is in backslash form (already normalised by
   *  dunce, with no `\\?\` verbatim prefix) while rel is always forward slashes, so it is
   *  converted before joining. A failure toasts rather than staying silent. */
  function copyPath(rel: string, full = false): Promise<void> {
    let text = rel;
    if (full && vault.root) {
      const win = vault.root.includes("\\");
      const sep = win ? "\\" : "/";
      const base = vault.root.endsWith(sep) ? vault.root.slice(0, -1) : vault.root;
      text = base + sep + (win ? rel.replaceAll("/", "\\") : rel);
    }
    return copyText(text).then((ok) => {
      if (!ok) toast.push(t("fileTree.copyFailed"));
    });
  }

  /** WebKit (Tauri on macOS) never focuses a <button> on click, so without
   *  this the row's onkeydown (⌘⌫/Delete/F2) can never fire. */
  function focusRow(e: Event) {
    (e.currentTarget as HTMLElement | null)?.focus?.();
  }

  function startCreate(mode: Exclude<EditMode, "rename">, parent: string | null) {
    if (parent) vault.expandFolder(parent);
    editing = { mode, parent };
    editValue = "";
    focusEdit(0);
  }

  function beginRename(path: string, kind: "note" | "file" | "folder") {
    editing = { mode: "rename", parent: parentOf(path), target: path, targetKind: kind };
    // Always edit the full raw name (extension included and
    // changeable); the stem is preselected so plain typing replaces the title
    editValue = baseOf(path);
    const dot = kind === "folder" ? -1 : editValue.lastIndexOf(".");
    focusEdit(dot > 0 ? dot : editValue.length);
  }

  function doDelete(path: string | undefined) {
    if (!path) return;
    vault.deletePath(path).catch((e) => (vault.error = String(e)));
  }

  /** Move from into targetDir ("" = the vault root). A name collision is reported by the backend and shown. */
  async function moveInto(from: string, kind: RowKind, targetDir: string) {
    const to = targetDir ? `${targetDir}/${baseOf(from)}` : baseOf(from);
    if (to === from) return;
    if (kind === "folder" && (targetDir === from || targetDir.startsWith(from + "/"))) {
      vault.error = t("fileTree.noFolderIntoItself");
      return;
    }
    try {
      await vault.renamePath(from, to);
      if (targetDir) vault.expandFolder(targetDir);
    } catch (e) {
      vault.error = String(e);
    }
  }

  /** The context menu's "undo X" label: the top of the stack described in plain words */
  function undoLabel(op: FileOp): string {
    if (op.kind === "rename")
      return t(parentOf(op.from) === parentOf(op.to) ? "fileTree.undoRename" : "fileTree.undoMove", { name: baseOf(op.to) });
    if (op.kind === "create")
      return op.paths.length > 1
        ? t("fileTree.undoImport", { n: op.paths.length })
        : t("fileTree.undoCreate", { name: baseOf(op.paths[0]) });
    return t("fileTree.undoDelete", { name: baseOf(op.path) });
  }

  /** Whether a drop makes sense: it does not fall back where it was, and a folder does not fall into itself or a descendant */
  function validTarget(from: string, kind: RowKind, dir: string): boolean {
    if (dir === (parentOf(from) ?? "")) return false;
    if (kind === "folder" && (dir === from || dir.startsWith(from + "/"))) return false;
    return true;
  }

  function pressRow(e: PointerEvent, path: string, kind: RowKind) {
    if (e.button !== 0) return;
    pressed = { path, kind, x: e.clientX, y: e.clientY };
    // capture: move/up still arrive when the pointer leaves the window, so a drag is never left hanging
    (e.currentTarget as HTMLElement | null)?.setPointerCapture?.(e.pointerId);
  }

  function dragMove(e: PointerEvent) {
    if (pressed && !drag) {
      if (Math.hypot(e.clientX - pressed.x, e.clientY - pressed.y) < 5) return;
      drag = { path: pressed.path, kind: pressed.kind, name: baseOf(pressed.path), x: e.clientX, y: e.clientY };
    }
    if (!drag) return;
    e.preventDefault();
    drag.x = e.clientX;
    drag.y = e.clientY;
    // The drop target is resolved from the coordinates: the ghost is pointer-events:none and does not block the sampling
    const el = document.elementFromPoint(e.clientX, e.clientY)?.closest<HTMLElement>("[data-drop]");
    const dir = el?.dataset.drop ?? null;
    const next = dir !== null && validTarget(drag.path, drag.kind, dir) ? dir : null;
    if (next !== dropTarget) {
      dropTarget = next;
      clearTimeout(hoverExpandTimer);
      // Hovering an unexpanded folder for a moment expands it automatically, which makes dropping into a deep directory easier
      if (next && !vault.expanded[next])
        hoverExpandTimer = window.setTimeout(() => vault.expandFolder(next), 600);
    }
  }

  function dragEnd() {
    pressed = null;
    clearTimeout(hoverExpandTimer);
    if (!drag) return;
    const d = drag;
    const target = dropTarget;
    drag = null;
    dropTarget = null;
    suppressClick = true;
    if (target !== null) {
      // The focus returns to the tree: only then does the ⌘Z right after a drag land on the file tree's undo
      if (!treeEl?.contains(document.activeElement)) treeEl?.focus();
      void moveInto(d.path, d.kind, target);
    }
  }

  function dragCancel() {
    pressed = null;
    clearTimeout(hoverExpandTimer);
    drag = null;
    dropTarget = null;
    // After a pointercancel no click comes to consume the one-shot flag (the Escape branch arms
    // it through the click on release that follows) — a leftover would swallow the user's next,
    // unrelated click
    suppressClick = false;
  }

  /** Mark or unmark a privacy zone: visible to a person (the 🔒 badge), nonexistent to the AI */
  function togglePrivacy(target: { path: string; kind: "note" | "file" | "folder" }) {
    const marked = vault.privatePaths.has(target.path);
    void (marked ? vault.unmarkPrivate(target.path, target.kind) : vault.markPrivate(target.path, target.kind));
  }

  /** Keys on a focused tree row: ⌘⌫/Delete deletes, F2 renames, ⌘X cuts. */
  function rowKeydown(e: KeyboardEvent, path: string, kind: RowKind) {
    if ((e.key === "Backspace" && (e.metaKey || e.ctrlKey)) || e.key === "Delete") {
      e.preventDefault();
      doDelete(path);
    } else if (e.key === "F2") {
      e.preventDefault();
      beginRename(path, kind);
    } else if (e.key === "x" && (e.metaKey || e.ctrlKey)) {
      e.preventDefault();
      cut = { path, kind };
    } else if (e.key === "v" && (e.metaKey || e.ctrlKey)) {
      // reaches us only when no native Paste menu item swallows ⌘V;
      // otherwise the DOM paste event below handles it
      void pasteClipboard();
    }
  }

  /** Paste clipboard files/images into `pasteDir`. Triggered by the DOM paste
   *  event, ⌘V keydown, or the context menu — the timestamp guard keeps a
   *  keydown + paste pair from importing twice. */
  async function pasteClipboard(files: File[] = []) {
    const now = performance.now();
    if (now - lastPasteAt < 400) return;
    lastPasteAt = now;
    if (cut) {
      // An internal cut wins over the system clipboard: pasting means moving into pasteDir
      const c = cut;
      cut = null;
      if (!treeEl?.contains(document.activeElement)) treeEl?.focus();
      await moveInto(c.path, c.kind, pasteDir);
      return;
    }
    const saved = await vault.pasteIntoFolder(pasteDir, files);
    if (pasteDir) vault.expandFolder(pasteDir);
    // reveal the result — unless it's a folder (opening one launches Finder)
    if (saved[0] && !vault.folders.includes(saved[0])) await vault.openFile(saved[0]);
  }

  /** Window-level so WebKit's paste (menu-driven ⌘V) reaches us; only claimed
   *  when the tree has focus and the user isn't typing in the inline input. */
  function handlePaste(e: ClipboardEvent) {
    if (editing || !treeEl?.contains(document.activeElement)) return;
    e.preventDefault();
    void pasteClipboard(Array.from(e.clipboardData?.files ?? []));
  }

  function focusEdit(selectEnd: number) {
    queueMicrotask(() => {
      editInput?.focus();
      editInput?.setSelectionRange(0, selectEnd || editValue.length);
    });
  }

  async function commitEdit() {
    const ed = editing;
    const name = editValue.trim().replace(/[/\\]/g, " ").trim();
    editing = null;
    if (!ed || !name) return;
    try {
      if (ed.mode === "create-folder") {
        await vault.createFolder(ed.parent, name);
        if (ed.parent) vault.expandFolder(ed.parent);
      } else if (ed.mode === "create-note") {
        // an explicit extension (test.py, data.csv) is respected verbatim;
        // otherwise the name is a note title and gets .md
        const hasExt = /\.[A-Za-z0-9]{1,8}$/.test(name);
        await vault.createFile(ed.parent, hasExt ? name : `${name}.md`);
      } else if (ed.mode === "create-file") {
        await vault.createFile(ed.parent, name);
      } else if (ed.mode === "rename" && ed.target) {
        // exactly as typed — extension changes are deliberate and allowed
        const to = ed.parent ? `${ed.parent}/${name}` : name;
        if (to !== ed.target) await vault.renamePath(ed.target, to);
      }
    } catch (e) {
      vault.error = String(e);
    }
  }

  function cancelEdit() {
    editing = null;
  }

  function editKeydown(e: KeyboardEvent) {
    if (isImeKeydown(e)) return; // Chinese IME: Enter confirms composition, not the edit
    if (e.key === "Enter") {
      e.preventDefault();
      void commitEdit();
    } else if (e.key === "Escape") {
      e.preventDefault();
      // It does not bubble to window: App's Escape would switch a projection view back to the
      // editor along the way, and cancelling one inline input should not change the main area
      e.stopPropagation();
      cancelEdit();
    }
  }

  const placeholders = $derived<Record<EditMode, string>>({
    "create-note": t("fileTree.promptNote"),
    "create-file": t("fileTree.promptFile"),
    "create-folder": t("fileTree.promptFolder"),
    rename: "",
  });
</script>

<svelte:window
  onclickcapture={(e) => {
    if (suppressClick) {
      suppressClick = false;
      e.stopPropagation();
      e.preventDefault();
    }
  }}
  onclick={() => {
    switcher = false;
  }}
  onkeydown={(e) => {
    if (e.key === "Escape") {
      // Layered exit: close an overlay first, then cancel a drag, and only then clear the cut
      // marker — the Escape that closes the context menu should not throw away a cut waiting to
      // be pasted
      // (with the context menu open, ContextMenu swallows Escape during capture and it never reaches here)
      if (switcher) {
        switcher = false;
      } else if (drag) {
        dragCancel();
        // The click on release lands back on the starting row (pointer capture semantics) and must not open it
        suppressClick = true;
      } else {
        cut = null;
      }
    }
  }}
  onpointermove={dragMove}
  onpointerup={dragEnd}
  onpointercancel={dragCancel}
  onpaste={handlePaste}
/>

{#snippet editRow(depth: number)}
  <div class="edit-row" style:padding-left="{8 + depth * 16 + 16}px">
    <input
      bind:this={editInput}
      bind:value={editValue}
      placeholder={editing ? placeholders[editing.mode] : ""}
      spellcheck="false"
      onkeydown={editKeydown}
      onblur={() => void commitEdit()}
    />
  </div>
{/snippet}

{#snippet indent(depth: number)}
  {#each Array(depth) as _, i (i)}<span class="guide"></span>{/each}
{/snippet}

{#snippet folder(node: TreeNode, depth: number)}
  {#if editing && editing.mode !== "rename" && editing.parent === (node.path || null)}
    {@render editRow(depth)}
  {/if}
  {#each node.folders as child (child.path)}
    {#if editing?.mode === "rename" && editing.target === child.path}
      {@render editRow(depth - 1 < 0 ? 0 : depth)}
    {:else}
      <button
        class="row folder-row"
        class:cut={cut?.path === child.path}
        class:drop-into={dropTarget === child.path}
        data-drop={child.path}
        onpointerdown={(e) => pressRow(e, child.path, "folder")}
        onclick={(e) => {
          focusRow(e);
          pasteDir = child.path;
          if (vault.expanded[child.path]) delete vault.expanded[child.path];
          else vault.expanded[child.path] = true;
        }}
        oncontextmenu={(e) => openMenu(e, child.path, { path: child.path, kind: "folder" })}
        onkeydown={(e) => rowKeydown(e, child.path, "folder")}
      >
        {@render indent(depth)}
        <span class="gutter chevron" class:open={!!vault.expanded[child.path]}><Chevron /></span>
        <span class="name">{child.name}</span>
        {#if vault.privatePaths.has(child.path)}<span class="lock" title={t("statusBar.privacyZoneHint")}>🔒</span>{/if}
      </button>
    {/if}
    {#if vault.expanded[child.path]}
      {@render folder(child, depth + 1)}
    {/if}
  {/each}
  {#each node.notes as note (note.id)}
    {#if editing?.mode === "rename" && editing.target === note.id}
      {@render editRow(depth)}
    {:else}
      <button
        class="row file-row"
        class:active={vault.currentPath === note.id}
        class:open={vault.isOpen(note.id)}
        class:cut={cut?.path === note.id}
        data-path={note.id}
        data-drop={parentOf(note.id) ?? ""}
        title={note.id}
        onpointerdown={(e) => pressRow(e, note.id, "note")}
        onclick={(e) => {
          focusRow(e);
          pasteDir = parentOf(note.id) ?? "";
          // ⌘click = a new tab (pinned); a single click = the preview slot (the store funnel's default)
          if (e.metaKey || e.ctrlKey) void vault.openInNewTab(note.id);
          else void vault.openNote(note.id);
        }}
        onauxclick={(e) => e.button === 1 && void vault.openInNewTab(note.id)}
        ondblclick={() => vault.pinActiveTab()}
        oncontextmenu={(e) => openMenu(e, parentOf(note.id), { path: note.id, kind: "note" })}
        onkeydown={(e) => rowKeydown(e, note.id, "note")}
      >
        {@render indent(depth)}
        <span class="gutter"></span>
        <span class="name">{stemOf(note.id)}</span>
        {#if vault.privatePaths.has(note.id)}<span class="lock" title={t("statusBar.privacyZoneHint")}>🔒</span>{/if}
      </button>
    {/if}
  {/each}
  {#each node.others as file (file)}
    {#if editing?.mode === "rename" && editing.target === file}
      {@render editRow(depth)}
    {:else}
      <button
        class="row file-row other"
        class:active={vault.currentPath === file}
        class:open={vault.isOpen(file)}
        class:cut={cut?.path === file}
        data-path={file}
        data-drop={parentOf(file) ?? ""}
        title={file}
        onpointerdown={(e) => pressRow(e, file, "file")}
        onclick={(e) => {
          focusRow(e);
          pasteDir = parentOf(file) ?? "";
          if (e.metaKey || e.ctrlKey) void vault.openInNewTab(file);
          else void vault.openFile(file);
        }}
        onauxclick={(e) => e.button === 1 && void vault.openInNewTab(file)}
        ondblclick={() => vault.pinActiveTab()}
        oncontextmenu={(e) => openMenu(e, parentOf(file), { path: file, kind: "file" })}
        onkeydown={(e) => rowKeydown(e, file, "file")}
      >
        {@render indent(depth)}
        <span class="gutter"></span>
        <span class="name">{stemNoExt(file)}</span>
        {#if vault.privatePaths.has(file)}<span class="lock" title={t("statusBar.privacyZoneHint")}>🔒</span>{/if}
        {#if extOf(file)}<span class="ext">{extOf(file)}</span>{/if}
      </button>
    {/if}
  {/each}
{/snippet}

<div class="pane-head">
  <button
    class="vault-switcher"
    title={`${vault.root} · ${tn("fileTree.noteCount", vault.noteCount)}`}
    aria-haspopup="menu"
    aria-expanded={switcher}
    onclick={(e) => {
      e.stopPropagation();
      contextMenu.close();
      switcher = !switcher;
    }}
  >
    <span class="vault-name">{vaultName(vault.root)}</span>
    <svg class="caret" width="9" height="9" viewBox="0 0 10 10" fill="none" aria-hidden="true">
      <path d="M2 3.5 L5 6.5 L8 3.5" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" stroke-linejoin="round" />
    </svg>
  </button>
  {#if switcher}
    <div class="switcher" role="menu">
      {#each switcherVaults as path (path)}
        <button
          role="menuitem"
          class="sw-item"
          class:current={path === vault.root}
          onclick={() => switchTo(path)}
          oncontextmenu={(e) => {
            e.stopPropagation();
            contextMenu.open(e, [
              // Disabled for the vault that is open: removing it would only have remember add it
              // back on the next open or note creation, which is a pure no-op
              { label: t("fileTree.removeFromRecent"), disabled: path === vault.root, action: () => void vault.removeRecent(path) },
            ]);
          }}
        >
          <span class="sw-name">
            {vaultName(path)}
            {#if path === vault.root}
              <svg width="11" height="11" viewBox="0 0 12 12" fill="none" aria-hidden="true">
                <path d="M2.5 6.5 L5 9 L9.5 3.5" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round" />
              </svg>
              <span class="sw-count">{tn("fileTree.noteCount", vault.noteCount)}</span>
            {/if}
          </span>
          <!-- lrm bookends: under the rtl truncation trick, the neutral characters at a path's ends (\ ? .) would be reordered by bidi -->
          <span class="sw-path">&lrm;{path}&lrm;</span>
        </button>
      {/each}
      {#if vault.recent.length}<div class="menu-sep"></div>{/if}
      <button
        role="menuitem"
        class="sw-item sw-pick"
        onclick={() => {
          switcher = false;
          void executeCommand("open-vault");
        }}
      >
        {t("palette.openOtherVault")}
      </button>
      <button
        role="menuitem"
        class="sw-item sw-pick"
        onclick={() => {
          switcher = false;
          void executeCommand("create-vault");
        }}
      >
        {t("cmd.create-vault.title")}
      </button>
    </div>
  {/if}
  <div class="head-actions">
    <button title={t("fileTree.newNote")} aria-label={t("fileTree.newNote")} onclick={() => startCreate("create-note", pasteDir || null)}>
      <svg width="16" height="16" viewBox="0 0 20 20" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
        <path d="M10.5 4 H5 A1.75 1.75 0 0 0 3.25 5.75 V15 A1.75 1.75 0 0 0 5 16.75 H14.25 A1.75 1.75 0 0 0 16 15 V9.5" />
        <path d="M15.3 3.05 A1.6 1.6 0 0 1 17.55 5.3 L10.4 12.45 L7.3 13.3 L8.15 10.2 Z" />
      </svg>
    </button>
    <button title={t("fileTree.newFolder")} aria-label={t("fileTree.newFolder")} onclick={() => startCreate("create-folder", pasteDir || null)}>
      <svg width="16" height="16" viewBox="0 0 20 20" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
        <path d="M2 5.25 A1.75 1.75 0 0 1 3.75 3.5 H7.1 L9 5.6 H16.25 A1.75 1.75 0 0 1 18 7.35 V14.75 A1.75 1.75 0 0 1 16.25 16.5 H3.75 A1.75 1.75 0 0 1 2 14.75 Z" />
        <path d="M10 8.25 V12.75 M7.75 10.5 H12.25" />
      </svg>
    </button>
  </div>
</div>

{#if hasMemory}
  <div class="pinned">
    <button
      class="row file-row memory-row"
      class:active={vault.currentPath === MEMORY_ID}
      class:open={vault.isOpen(MEMORY_ID)}
      title={t("fileTree.memoryHint")}
      onclick={(e) => {
        focusRow(e);
        pasteDir = "";
        if (e.metaKey || e.ctrlKey) void vault.openInNewTab(MEMORY_ID);
        else void vault.openNote(MEMORY_ID);
      }}
      onauxclick={(e) => e.button === 1 && void vault.openInNewTab(MEMORY_ID)}
      ondblclick={() => vault.pinActiveTab()}
      oncontextmenu={(e) => {
        e.preventDefault();
        e.stopPropagation();
        focusRow(e);
        switcher = false;
        // The pinned VEMRY.md row: only deleting is allowed, since renaming or moving would break the agreed path
        contextMenu.open(e, [{ title: t("fileTree.memoryMenuTitle") }, deleteEntry(MEMORY_ID)]);
      }}
      onkeydown={(e) => {
        if ((e.key === "Backspace" && (e.metaKey || e.ctrlKey)) || e.key === "Delete") {
          e.preventDefault();
          doDelete(MEMORY_ID);
        }
      }}
    >
      <span class="gutter memory-icon">
        <svg width="12" height="12" viewBox="0 0 12 12" fill="none" aria-hidden="true">
          <path d="M6 1.4 L7.15 4.85 L10.6 6 L7.15 7.15 L6 10.6 L4.85 7.15 L1.4 6 L4.85 4.85 Z" stroke="currentColor" stroke-width="1.2" stroke-linejoin="round" />
        </svg>
      </span>
      <span class="name">VEMRY.md</span>
      {#if vault.privatePaths.has(MEMORY_ID)}<span class="lock" title={t("statusBar.privacyZoneHint")}>🔒</span>{/if}
      <span class="ext agent">AGENT</span>
    </button>
  </div>
{/if}

<!-- tabindex -1: clicking empty space must hold focus inside the tree so a
  following ⌘V paste targets the vault root (rows handle their own focus) -->
<!-- svelte-ignore a11y_no_noninteractive_tabindex, a11y_no_noninteractive_element_interactions, a11y_click_events_have_key_events -->
<nav
  class="tree"
  class:dragging={!!drag}
  class:drop-root={dropTarget === ""}
  tabindex="-1"
  bind:this={treeEl}
  data-drop=""
  oncontextmenu={(e) => openMenu(e, null)}
  onclick={(e) => {
    if (e.target !== e.currentTarget) return;
    (e.currentTarget as HTMLElement).focus();
    pasteDir = "";
  }}
  onkeydown={(e) => {
    // With the focus inside the tree, ⌘Z undoes a file operation (a row's keydown bubbles here
    // when it did not handle it).
    // A ⌘Z inside an inline input belongs to the native text undo and is not intercepted
    if (editing) return;
    if (e.key === "z" && (e.metaKey || e.ctrlKey) && !e.shiftKey && !e.altKey) {
      e.preventDefault();
      void vault.undoFileOp();
    }
  }}
>
  {@render folder(tree, 0)}
  {#if vault.notes.length === 0 && !editing}
    <p class="hint">{t("fileTree.emptyVault", { key: k("Mod+N") })}</p>
  {/if}
</nav>

{#if drag}
  <div class="drag-ghost" style:left="{drag.x + 12}px" style:top="{drag.y + 10}px">{drag.name}</div>
{/if}

<style>
  .pane-head {
    position: relative;
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 6px 8px 6px 12px;
    border-bottom: 1px solid var(--border);
  }
  .vault-switcher {
    display: flex;
    align-items: center;
    gap: 4px;
    min-width: 0;
    padding: 3px 6px;
    margin-left: -6px;
    border-radius: 5px;
  }
  .vault-switcher:hover,
  .vault-switcher[aria-expanded="true"] {
    background: var(--bg-hover);
  }
  .vault-name {
    font-weight: 600;
    font-size: var(--fs-sm);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .caret {
    flex-shrink: 0;
    color: var(--fg-faint);
  }
  .vault-switcher:hover .caret {
    color: var(--fg-muted);
  }
  .switcher {
    position: absolute;
    top: calc(100% + 4px);
    left: 8px;
    z-index: 250;
    min-width: 220px;
    max-width: 300px;
    background: var(--bg-panel);
    border: 1px solid var(--border);
    border-radius: 8px;
    box-shadow: var(--shadow);
    padding: 4px;
    display: flex;
    flex-direction: column;
  }
  .sw-item {
    display: flex;
    flex-direction: column;
    align-items: stretch;
    gap: 1px;
    text-align: left;
    padding: 6px 10px;
    border-radius: 5px;
  }
  .sw-item:hover {
    background: var(--bg-hover);
  }
  .sw-name {
    display: flex;
    align-items: center;
    gap: 6px;
    font-size: var(--fs-sm);
    font-weight: 600;
  }
  .sw-item.current .sw-name {
    color: var(--accent);
  }
  .sw-path {
    font-family: var(--font-mono);
    font-size: var(--fs-xs);
    color: var(--fg-faint);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    /* rtl: an overlong path is truncated at the head, keeping the trailing directory names */
    direction: rtl;
    text-align: left;
  }
  .sw-pick {
    font-size: var(--fs-sm);
    color: var(--fg-muted);
  }
  .sw-pick:hover {
    color: var(--fg);
  }
  .sw-count {
    margin-left: auto;
    padding-left: 10px;
    font-size: var(--fs-xs);
    font-weight: 400;
    color: var(--fg-faint);
  }
  .head-actions {
    display: flex;
    align-items: center;
    gap: 1px;
    margin-left: auto;
  }
  .head-actions button {
    display: grid;
    place-items: center;
    width: 26px;
    height: 26px;
    border-radius: 6px;
    color: var(--fg-muted);
  }
  .head-actions button:hover {
    background: var(--bg-hover);
    color: var(--fg);
  }
  /* The pinned VEMRY.md: fixed above the tree and not scrolling with its content */
  .pinned {
    flex-shrink: 0;
    padding: 4px;
    border-bottom: 1px solid var(--border);
  }
  .memory-icon {
    color: var(--fg-faint);
  }
  .memory-row:hover .memory-icon,
  .memory-row.active .memory-icon {
    color: var(--accent);
  }
  .tree {
    overflow-y: auto;
    flex: 1;
    min-height: 0;
    padding: 6px 4px 24px;
  }
  .row {
    display: flex;
    align-items: center;
    width: 100%;
    min-height: 24px;
    padding: 3px 8px;
    border-radius: 4px;
    font-size: var(--fs-sm);
    color: var(--fg-muted);
    text-align: left;
  }
  .row:hover {
    background: var(--bg-hover);
    color: var(--fg);
  }
  /* The faint highlight: open in some tab (the text colour alone); the active tab is still highlighted with a background */
  .file-row.open {
    color: var(--fg);
  }
  .file-row.active {
    background: var(--accent-soft);
    color: var(--fg);
  }
  .name {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .ext {
    flex-shrink: 0;
    margin-left: 8px;
    font-family: var(--font-mono);
    font-size: 9px;
    line-height: 1;
    letter-spacing: 0.03em;
    color: var(--fg-faint);
    border: 1px solid var(--border);
    border-radius: 3px;
    padding: 1.5px 3px;
  }
  /* AGENT is not an extension — the semantic badge uses accent to set it apart from the grey file-type badges */
  .ext.agent {
    color: var(--accent);
    border-color: currentColor;
  }
  /* The privacy zone badge: visible to a person, invisible to the AI */
  .lock {
    flex-shrink: 0;
    margin-left: 6px;
    font-size: 9px;
    line-height: 1;
    opacity: 0.75;
  }
  .file-row.active .ext {
    color: var(--accent);
  }
  .folder-row {
    color: var(--fg);
    font-weight: 600;
  }
  /* The indent guides: 16px per level, with the vertical line falling in the centre of the level above's arrow slot */
  .guide {
    position: relative;
    flex: 0 0 16px;
    align-self: stretch;
  }
  .guide::before {
    content: "";
    position: absolute;
    left: 7.5px;
    top: -4px;
    bottom: -4px;
    width: 1px;
    background: var(--border);
  }
  .gutter {
    flex: 0 0 16px;
    display: flex;
    align-items: center;
    justify-content: center;
  }
  .chevron {
    color: var(--fg-faint);
    transition: transform 0.12s;
  }
  .chevron.open {
    transform: rotate(90deg);
  }
  .row:hover .chevron {
    color: var(--fg-muted);
  }
  .hint {
    padding: 16px 12px;
    font-size: var(--fs-xs);
    color: var(--fg-faint);
  }
  /* Cut and waiting to be pasted: semi-transparent (the file manager convention) */
  .row.cut {
    opacity: 0.5;
  }
  /* The folder row hovered as a drop target while dragging (hovering a file row lights up its parent folder's row) */
  .row.drop-into {
    background: var(--accent-soft);
    box-shadow: inset 0 0 0 1px var(--accent);
  }
  /* The drop target is the vault root: the whole tree panel is outlined */
  .tree.drop-root {
    box-shadow: inset 0 0 0 2px var(--accent);
    border-radius: 6px;
  }
  .tree.dragging,
  .tree.dragging .row {
    cursor: grabbing;
  }
  .drag-ghost {
    position: fixed;
    z-index: 300;
    pointer-events: none;
    max-width: 220px;
    padding: 3px 8px;
    border: 1px solid var(--border);
    border-radius: 5px;
    background: var(--bg-panel);
    box-shadow: var(--shadow);
    font-size: var(--fs-xs);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .edit-row {
    padding-top: 1px;
    padding-bottom: 1px;
    padding-right: 8px;
  }
  .edit-row input {
    width: 100%;
    font-size: var(--fs-sm);
    padding: 2px 6px;
    border-color: var(--accent);
    border-radius: 4px;
    background: var(--bg);
  }
  /* The context menu's styling travels with its component (ContextMenu.svelte); the vault switcher still uses this divider */
  .menu-sep {
    height: 1px;
    background: var(--border);
    margin: 4px 6px;
  }
</style>
