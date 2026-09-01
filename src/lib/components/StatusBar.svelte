<script lang="ts">
  // The bottom status bar (main window only; the welcome page and the detached chat
  // window do not render it). Its content is deliberately restrained: on the left the
  // vault name plus the privacy-zone shield; on the right the cursor's line and column,
  // the selection statistics and the document's word count.
  // Every figure comes from what the vault store already pushes — the editor is the
  // only producer, and this merely projects it.
  import { t, tn } from "$lib/i18n/i18n.svelte";
  import { vault, vaultName } from "$lib/stores/vault.svelte";

  const inPrivate = $derived(vault.currentPath !== null && vault.privatePaths.has(vault.currentPath));
</script>

<footer class="statusbar">
  <span class="vault-name" title={vault.root}>{vaultName(vault.root)}</span>
  {#if inPrivate}
    <span class="shield" title={t("statusBar.privacyZoneHint")}>
      <svg width="14" height="14" viewBox="0 0 20 20" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
        <path d="M10 2.5 L16.5 5 v4.5 c0 4 -2.8 6.7 -6.5 8 c-3.7 -1.3 -6.5 -4 -6.5 -8 V5 Z" />
        <path d="M7.4 9.9 l1.8 1.8 3.4 -4" />
      </svg>
      <span>{t("statusBar.privacyZone")}</span>
    </span>
  {/if}
  <span class="right">
    {#if vault.editorLoc}
      <span>{t("statusBar.cursor", { line: vault.editorLoc.line, col: vault.editorLoc.col })}</span>
      {#if vault.editorSelStats}
        <span
          >{t("statusBar.selectedChars", { n: vault.editorSelStats.chars })}{vault.editorSelStats.lines > 1
            ? ` · ${tn("statusBar.selectedLines", vault.editorSelStats.lines)}`
            : ""}</span
        >
      {/if}
    {/if}
    {#if vault.docWords != null}
      <span>{tn("statusBar.words", vault.docWords)}</span>
    {/if}
  </span>
</footer>

<style>
  .statusbar {
    flex: 0 0 var(--statusbar-h);
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 0 12px;
    font-size: var(--fs-xs);
    color: var(--fg-muted);
    background: var(--bg-panel);
    border-top: 1px solid var(--border-strong);
    user-select: none;
    overflow: hidden;
    white-space: nowrap;
  }
  .vault-name {
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .shield {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    color: var(--accent);
    flex-shrink: 0;
  }
  .shield svg {
    display: block;
  }
  .right {
    margin-left: auto;
    display: inline-flex;
    gap: 12px;
    flex-shrink: 0;
    font-variant-numeric: tabular-nums;
  }
</style>
