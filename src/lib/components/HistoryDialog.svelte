<script lang="ts">
  // The kept states of a note (history.rs): a list of moments on the left, each
  // labelled by who wrote over it, and on the right what putting that state back
  // would change, read as a diff from the note as it is now. Restore writes the
  // kept text to disk through the vault (which keeps the current text first, so
  // a restore is itself in the list a moment later); an open, dirty buffer of the
  // note is written out before, or its next autosave would land over the restore.
  import { t } from "$lib/i18n/i18n.svelte";
  import { i18n } from "$lib/i18n/i18n.svelte";
  import { api, type HistoryEntry, type HistorySnapshot } from "$lib/api";
  import { classifyDiff } from "$lib/views/diff";
  import { history } from "$lib/stores/history.svelte";
  import { vault } from "$lib/stores/vault.svelte";
  import { toast } from "$lib/stores/toast.svelte";
  import DiffLines from "./DiffLines.svelte";

  let snapshots = $state<HistorySnapshot[]>([]);
  let selected = $state<string | null>(null);
  let entry = $state<HistoryEntry | null>(null);
  let loading = $state(false);
  let restoring = $state(false);
  let root = $state<HTMLElement | null>(null);

  const when = $derived(new Intl.DateTimeFormat(i18n.locale, { dateStyle: "medium", timeStyle: "medium" }));
  const diffLines = $derived(entry ? classifyDiff(entry.diff) : []);
  const kb = (bytes: number) => (bytes < 1024 ? `${bytes} B` : `${(bytes / 1024).toFixed(1)} KB`);

  /** Opening fetches the list and selects the newest; the focus moves onto the dialog so
   *  the keys stop reaching the editor behind the overlay. */
  $effect(() => {
    const path = history.path;
    if (!path) {
      snapshots = [];
      selected = null;
      entry = null;
      return;
    }
    void load(path);
    root?.focus();
  });

  async function load(path: string) {
    loading = true;
    try {
      snapshots = await api.noteHistoryList(path);
      selected = snapshots[0]?.id ?? null;
    } catch (e) {
      toast.push(String(e), "error");
      snapshots = [];
      selected = null;
    } finally {
      loading = false;
    }
  }

  $effect(() => {
    const path = history.path;
    const id = selected;
    if (!path || !id) {
      entry = null;
      return;
    }
    void api.noteHistoryRead(path, id).then(
      (e) => {
        if (history.path === path && selected === id) entry = e;
      },
      (err) => toast.push(String(err), "error"),
    );
  });

  async function restore() {
    const path = history.path;
    const id = selected;
    if (!path || !id || restoring) return;
    restoring = true;
    try {
      // The buffer first: a dirty buffer is not reloaded by the restore's event, and its
      // autosave would write the pre-restore text back
      if (vault.currentPath === path && vault.dirty) await vault.flushEditor?.();
      await api.noteHistoryRestore(path, id);
      toast.push(t("history.restored", { path }));
      history.close();
    } catch (e) {
      toast.push(String(e), "error");
    } finally {
      restoring = false;
    }
  }

  async function copyText() {
    if (!entry) return;
    try {
      await navigator.clipboard.writeText(entry.content);
      toast.push(t("history.copied"));
    } catch (e) {
      toast.push(String(e), "error");
    }
  }

  function onkeydown(e: KeyboardEvent) {
    if (!history.path) return;
    if (e.key === "Escape") {
      e.preventDefault();
      e.stopPropagation();
      history.close();
    }
  }
</script>

<svelte:window onkeydowncapture={onkeydown} />

{#if history.path}
  <div class="overlay" role="presentation" onclick={(e) => { if (e.target === e.currentTarget) history.close(); }}>
    <div class="dialog" role="dialog" aria-label={t("history.title")} tabindex="-1" bind:this={root}>
      <header>
        <span class="summary">{t("history.title")}</span>
        <span class="path">{history.path}</span>
      </header>
      <div class="body">
        <ul class="list" role="listbox" aria-label={t("history.versions")}>
          {#if loading}
            <li class="empty">{t("history.loading")}</li>
          {:else if !snapshots.length}
            <li class="empty">{t("history.empty")}</li>
          {:else}
            {#each snapshots as s (s.id)}
              <li>
                <button class="row" class:selected={s.id === selected} role="option" aria-selected={s.id === selected} onclick={() => (selected = s.id)}>
                  <span class="time">{when.format(new Date(s.ts_ms))}</span>
                  <span class="meta"><span class="writer {s.writer}">{t(`history.writer.${s.writer}`)}</span> · {kb(s.bytes)}</span>
                </button>
              </li>
            {/each}
          {/if}
        </ul>
        <div class="pane">
          {#if entry}
            {#if entry.diff}
              <pre class="diff"><DiffLines lines={diffLines} /></pre>
            {:else}
              <div class="same">{t("history.sameAsNow")}</div>
            {/if}
          {:else if selected}
            <div class="same">{t("history.loading")}</div>
          {/if}
        </div>
      </div>
      <footer>
        <span class="hint">{t("history.hint")}</span>
        <span class="spacer"></span>
        <button class="btn subtle" onclick={() => history.close()}>{t("common.close")}</button>
        <button class="btn subtle" disabled={!entry} onclick={copyText}>{t("history.copy")}</button>
        <button class="btn primary" disabled={!entry || !entry.diff || restoring} onclick={restore}>{t("history.restore")}</button>
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
    width: min(860px, 94vw);
    height: min(640px, 84vh);
    background: var(--bg-panel);
    border: 1px solid var(--border);
    border-radius: 10px;
    box-shadow: var(--shadow);
    display: flex;
    flex-direction: column;
    overflow: hidden;
    outline: none;
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
  .body {
    flex: 1;
    min-height: 0;
    display: grid;
    grid-template-columns: 260px 1fr;
    gap: 10px;
    margin: 0 16px;
  }
  .list {
    list-style: none;
    margin: 0;
    padding: 0;
    overflow: auto;
    border: 1px solid var(--border);
    border-radius: 6px;
  }
  .row {
    display: flex;
    flex-direction: column;
    gap: 2px;
    width: 100%;
    padding: 8px 10px;
    border: 0;
    border-bottom: 1px solid var(--border);
    background: none;
    color: var(--fg);
    text-align: left;
    cursor: pointer;
    font: inherit;
  }
  .row:hover {
    background: var(--bg-hover);
  }
  .row.selected {
    background: var(--bg-active);
  }
  .time {
    font-size: var(--fs-sm);
  }
  .meta {
    font-size: var(--fs-xs);
    color: var(--fg-muted);
  }
  .writer.agent {
    color: var(--accent);
  }
  .writer.external {
    color: var(--danger);
  }
  .empty {
    padding: 14px 12px;
    font-size: var(--fs-sm);
    color: var(--fg-muted);
    line-height: 1.5;
  }
  .pane {
    min-height: 0;
    display: flex;
  }
  .diff {
    flex: 1;
    margin: 0;
    overflow: auto;
    border: 1px solid var(--border);
    border-radius: 6px;
    -webkit-user-select: text;
    user-select: text;
  }
  .same {
    flex: 1;
    display: grid;
    place-items: center;
    color: var(--fg-muted);
    font-size: var(--fs-sm);
    border: 1px dashed var(--border);
    border-radius: 6px;
  }
  footer {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 12px 16px 14px;
  }
  .hint {
    font-size: var(--fs-xs);
    color: var(--fg-muted);
  }
  .spacer {
    flex: 1;
  }
  /* The footer's buttons: the permission dialog's shapes (they had none of their own and
     drew as bare text) */
  .btn {
    font-size: var(--fs-sm);
    padding: 6px 12px;
    border-radius: 6px;
    border: 1px solid var(--border);
    white-space: nowrap;
  }
  .btn:hover:not(:disabled) {
    background: var(--bg-hover);
  }
  .btn.subtle {
    color: var(--fg-muted);
  }
  .btn.primary {
    background: var(--accent);
    border-color: var(--accent);
    color: var(--bg);
  }
  .btn.primary:hover:not(:disabled) {
    background: color-mix(in srgb, var(--accent) 88%, var(--fg));
    border-color: color-mix(in srgb, var(--accent) 88%, var(--fg));
  }
  /* Unavailable: an outline only, so no label is ever greyed out by transparency */
  .btn.primary:disabled {
    background: transparent;
    border-color: var(--border);
    color: var(--fg-muted);
  }
</style>
