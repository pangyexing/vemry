<script lang="ts">
  import { t } from "$lib/i18n/i18n.svelte";
  import { agent } from "$lib/stores/agent.svelte";
  import { classifyDiff } from "$lib/views/diff";
  import { contextMenu, type MenuEntry } from "$lib/stores/contextMenu.svelte";
  import Chevron from "./Chevron.svelte";

  let editing = $state(false);
  /** A block-level request (a restructure, a block move) opens on its block diff; this switches to the raw line diff. */
  let rawDiff = $state(false);
  let editedContent = $state("");
  let allowBtn = $state<HTMLButtonElement | null>(null);

  let req = $derived(agent.pendingPermission);

  $effect(() => {
    if (req) {
      editing = false;
      rawDiff = false;
      editedContent = req.new_content;
      // The initial focus lands on "allow": return approves, and Tab reaches the other buttons
      queueMicrotask(() => allowBtn?.focus());
    }
  });

  let diffLines = $derived.by(() => {
    if (!req) return [];
    // A move has no content diff: two lines of paths, borrowing the diff's add and delete
    // colours — the rendering still goes through the single {#each} below, so how a line is
    // drawn is decided in one place
    if (req.to_path) {
      const paths = [{ cls: "del", text: `- ${req.path}` }, { cls: "add", text: `+ ${req.to_path}` }];
      // The links the move rewrites follow as an ordinary diff (one hunk per note)
      return req.diff ? [...paths, { cls: "ctx", text: "" }, ...classifyDiff(req.diff)] : paths;
    }
    if (!req.diff) return [{ cls: "ctx", text: t("permission.newFile") }, ...req.new_content.split("\n").map((l) => ({ cls: "add", text: `+ ${l}` }))];
    return classifyDiff(req.diff);
  });

  /** The scoped variants of "allow for this session", as a dropdown under that button:
   *  only this note, or only the folder the note is in (for a move, the folder it comes
   *  from). A scope is a standing grant like the session one, narrowed to a prefix — the
   *  agent still asks for anything outside it, so a tidy-up of one project can run without
   *  a dialog per note while the rest of the vault stays reviewed. */
  function openScopeMenu(e: MouseEvent) {
    if (!req) return;
    const path = req.path;
    const slash = path.lastIndexOf("/");
    const folder = slash > 0 ? path.slice(0, slash + 1) : null;
    const name = path.slice(slash + 1);
    const entries: MenuEntry[] = [{ title: t("permission.scopeTitle") }];
    if (folder) entries.push({ label: t("permission.scopeFolder", { folder }), action: () => void agent.respond({ kind: "allow_scope", prefix: folder }) });
    entries.push({ label: t("permission.scopeNote", { name }), action: () => void agent.respond({ kind: "allow_scope", prefix: path }) });
    const r = (e.currentTarget as HTMLElement).getBoundingClientRect();
    contextMenu.openAt(r.left, r.bottom + 4, entries);
  }

  function allow() {
    if (editing && editedContent !== req?.new_content) {
      void agent.respond({ kind: "allow_edited", content: editedContent });
    } else {
      void agent.respond({ kind: "allow" });
    }
  }
  /** One click, no reason asked (the reason field that used to follow the click was skipped
   *  nearly every time); the model reads a plain "denied by user" and carries on. */
  function deny() {
    void agent.respond({ kind: "deny", reason: "" });
  }

  /** The keyboard channel: Enter allows, Esc denies. It is attached with capture and stops
   *  propagation: the dialog is the topmost overlay, and Esc must not also be consumed by a
   *  layer beneath (the settings modal, the App layer) — closing the settings while
   *  silently denying a write on one key press is a double consumption that really
   *  happened.
   *  With the focus on a button, Enter belongs to that button's native activation (tabbing
   *  to "deny" and pressing Enter has to deny rather than approve); an Enter inside an
   *  editing area or an input belongs to the input itself, and a contenteditable
   *  (CodeMirror) likewise. */
  function onkeydown(e: KeyboardEvent) {
    if (!req) return;
    // With the scope dropdown open, Esc closes the dropdown alone (the menu's own handler is
    // beneath this capture and never sees the key); a second Esc denies as before
    if (contextMenu.pos) {
      if (e.key === "Escape" || e.key === "Enter") {
        e.preventDefault();
        e.stopPropagation();
        if (e.key === "Escape") contextMenu.close();
      }
      return;
    }
    if (e.key === "Escape") {
      e.preventDefault();
      e.stopPropagation();
      deny();
      return;
    }
    if (e.key === "Enter") {
      const t = e.target instanceof HTMLElement ? e.target : null;
      if (t && (t.closest("button, textarea, input") != null || t.isContentEditable)) return;
      e.preventDefault();
      e.stopPropagation();
      allow();
    }
  }
</script>

<svelte:window onkeydowncapture={onkeydown} />

{#if req}
  <div class="overlay" role="presentation">
    <div class="dialog" role="dialog" aria-label={t("permission.title")}>
      <header>
        <span class="summary">{t(`permission.summary.${req.kind}`, { path: req.path, toPath: req.to_path ?? "" })}</span>
        <span class="path">{req.path}</span>
        {#if req.rewrites?.length}
          <span class="path">{t("permission.rewrites", { notes: req.rewrites.length, links: req.rewrites.reduce((n, r) => n + r.links, 0) })}</span>
        {/if}
      </header>

      {#if editing}
        <textarea class="edit-area" bind:value={editedContent} spellcheck="false"></textarea>
      {:else if req.block_diff && !rawDiff}
        <!-- A block-level write is read block by block: what moved is folded (moving is the point of a
             restructure and not a risk), what was removed is open (the one thing worth a second look) -->
        {@const bd = req.block_diff}
        <div class="bdiff">
          <div class="bdiff-sum">
            {t("permission.blockDiff", { moved: bd.moved, changed: bd.changed, added: bd.added, removed: bd.removed })}
            <button class="bdiff-raw" onclick={() => (rawDiff = true)}>{t("permission.rawDiff")}</button>
          </div>
          {#each ["removed", "changed", "added", "moved"] as kind (kind)}
            {@const rows = bd.items.filter((i) => i.kind === kind)}
            {#if rows.length}
              <details class="bdiff-group" open={kind !== "moved"}>
                <summary>{t(`permission.blockKind.${kind}` as never, { n: rows.length })}</summary>
                {#each rows as r, ri (ri)}
                  <div class="bdiff-row {kind}">
                    {#if r.old}<span class="bdiff-old">{r.old}</span><span class="bdiff-arrow">→</span>{/if}
                    <span>{r.text}</span>
                  </div>
                {/each}
              </details>
            {/if}
          {/each}
        </div>
      {:else}
        <pre class="diff">{#each diffLines as line, i (i)}<span class={line.cls}>{line.text}
</span>{/each}</pre>
      {/if}

      <footer>
        {#if !req.to_path}
          <button class="btn subtle" onclick={() => (editing = !editing)}>
            {editing ? t("permission.viewDiff") : t("permission.editThenApply")}
          </button>
        {/if}
        <span class="spacer"></span>
        <button class="btn subtle" onclick={deny} title={t("permission.denyHint")}>{t("permission.deny")}</button>
        <button
          class="btn subtle"
          onclick={() => agent.respond({ kind: "allow_always" })}
          title={t("permission.alwaysAllowVaultHint")}
        >
          {t("permission.alwaysAllowVault")}
        </button>
        <span class="split">
          <button class="btn subtle" onclick={() => agent.respond({ kind: "allow_session" })} title={t("permission.allowSessionHint")}>
            {t("permission.allowSession")}
          </button>
          <button class="btn subtle more" onclick={openScopeMenu} title={t("permission.scopeHint")} aria-label={t("permission.scopeHint")}>
            <Chevron size={9} />
          </button>
        </span>
        <button class="btn primary" bind:this={allowBtn} onclick={allow} title={t("permission.allowHint")}>{t("permission.allow")}</button>
      </footer>
    </div>
  </div>
{/if}

<style>
  .overlay {
    position: fixed;
    inset: 0;
    background: rgba(0, 0, 0, 0.35);
    display: grid;
    place-items: center;
    z-index: 200;
  }
  .dialog {
    width: min(680px, 92vw);
    max-height: 80vh;
    background: var(--bg-panel);
    border: 1px solid var(--border);
    border-radius: 10px;
    box-shadow: var(--shadow);
    display: flex;
    flex-direction: column;
    overflow: hidden;
  }
  header {
    display: flex;
    align-items: baseline;
    gap: 10px;
    padding: 14px 16px 10px;
  }
  .summary {
    font-weight: 600;
  }
  .path {
    font-family: var(--font-mono);
    font-size: var(--fs-xs);
    color: var(--fg-muted);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .diff {
    flex: 1;
    overflow: auto;
    margin: 0 16px;
    border: 1px solid var(--border);
    border-radius: 6px;
    font-family: var(--font-mono);
    font-size: var(--fs-xs);
    line-height: 1.6;
    -webkit-user-select: text;
    user-select: text;
    min-height: 120px;
  }
  .diff span {
    display: block;
    padding: 0 10px;
    white-space: pre-wrap;
    word-break: break-all;
  }
  .diff .add {
    background: var(--diff-add-bg);
    color: var(--accent);
  }
  .diff .del {
    background: var(--diff-del-bg);
    color: var(--danger);
  }
  .diff .hunk {
    color: var(--fg-faint);
  }
  .diff .meta {
    color: var(--fg-faint);
  }
  .diff .ctx {
    color: var(--fg-muted);
  }
  .bdiff {
    flex: 1;
    overflow: auto;
    margin: 0 16px;
    border: 1px solid var(--border);
    border-radius: 6px;
    font-size: var(--fs-xs);
    min-height: 120px;
    padding: 6px 10px;
  }
  .bdiff-sum {
    display: flex;
    align-items: center;
    justify-content: space-between;
    font-weight: 600;
    color: var(--fg-muted);
    padding-bottom: 4px;
    border-bottom: 1px solid var(--border);
  }
  .bdiff-raw {
    font-size: var(--fs-xs);
    color: var(--accent);
  }
  .bdiff-group {
    margin-top: 6px;
  }
  .bdiff-group summary {
    cursor: pointer;
    color: var(--fg-muted);
  }
  .bdiff-row {
    display: flex;
    gap: 6px;
    padding: 1px 0 1px 14px;
    font-family: var(--font-mono);
    white-space: pre-wrap;
    word-break: break-all;
  }
  .bdiff-row.removed {
    color: var(--danger);
  }
  .bdiff-row.added {
    color: var(--accent);
  }
  .bdiff-row.moved {
    color: var(--fg-faint);
  }
  .bdiff-old {
    color: var(--fg-faint);
    text-decoration: line-through;
  }
  .bdiff-arrow {
    color: var(--fg-faint);
  }
  .edit-area {
    flex: 1;
    margin: 0 16px;
    min-height: 240px;
    font-family: var(--font-mono);
    font-size: var(--fs-xs);
    line-height: 1.6;
    padding: 10px;
    resize: none;
  }
  footer {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 12px 16px;
  }
  .spacer {
    flex: 1;
  }
  .btn {
    font-size: var(--fs-sm);
    padding: 6px 12px;
    border-radius: 6px;
    border: 1px solid var(--border);
  }
  .btn:hover {
    background: var(--bg-hover);
  }
  .btn.primary {
    background: var(--accent);
    border-color: var(--accent);
    color: #fff;
  }
  .btn.primary:hover {
    opacity: 0.9;
    background: var(--accent);
  }
  .btn.subtle {
    color: var(--fg-muted);
  }
  /* "Allow for this session" and its scope dropdown are one split button: the chevron half
     shares the border and only opens the menu */
  .split {
    display: inline-flex;
  }
  .split .btn:first-child {
    border-top-right-radius: 0;
    border-bottom-right-radius: 0;
  }
  .split .more {
    border-top-left-radius: 0;
    border-bottom-left-radius: 0;
    border-left: none;
    padding-left: 6px;
    padding-right: 6px;
    display: inline-flex;
    align-items: center;
  }
</style>
