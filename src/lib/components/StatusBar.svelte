<script lang="ts">
  // The bottom status bar (main window only; the welcome page and the detached chat
  // window do not render it). Its content is deliberately restrained: on the left the
  // vault name plus the privacy-zone badge; on the right the cursor's line and column,
  // the selection statistics and the document's word count.
  // Every figure comes from what the vault store already pushes — the editor is the
  // only producer, and this merely projects it.
  import { t, tn } from "$lib/i18n/i18n.svelte";
  import { stemOf, vault, vaultName } from "$lib/stores/vault.svelte";
  import { originLabel } from "$lib/editor/provenance";
  import { prefs } from "$lib/stores/prefs.svelte";
  import { executeCommand } from "$lib/commands/registry";
  import { contextMenu } from "$lib/stores/contextMenu.svelte";
  import { k } from "$lib/shortcut";
  import PrivacyIcon from "./PrivacyIcon.svelte";

  const inPrivate = $derived(vault.currentPath !== null && vault.privatePaths.has(vault.currentPath));
  /** Unreviewed AI blocks in notes other than the open one (the vault chip's count). */
  const others = $derived.by(() => {
    const rest = vault.pendingNotes.filter((p) => p.path !== vault.currentPath);
    return { notes: rest.length, blocks: rest.reduce((n, p) => n + p.blocks, 0) };
  });

  /** The vault chip's list: every note with unreviewed AI blocks and its count, a click
   *  opening the note on its first run; marking the whole vault sits at the bottom behind
   *  a second click, since it is the one action here that cannot be looked at first. */
  function pendingList(e: MouseEvent) {
    const r = (e.currentTarget as HTMLElement).getBoundingClientRect();
    contextMenu.openAt(r.left, r.top - 4, [
      { title: t("prov.pendingListTitle") },
      ...vault.pendingNotes.map((p) => ({
        label: stemOf(p.path),
        hint: String(p.blocks),
        checked: p.path === vault.currentPath,
        action: () => void vault.openForReview(p.path),
      })),
      { separator: true },
      { label: t("prov.acceptVault"), confirm: t("prov.acceptVaultConfirm"), action: () => void executeCommand("provenance-accept-vault") },
    ]);
  }
</script>

<footer class="statusbar">
  <span class="vault-name" title={vault.root}>{vaultName(vault.root)}</span>
  {#if inPrivate}
    <span class="zone" title={t("statusBar.privacyZoneHint")}>
      <PrivacyIcon />
      <span>{t("statusBar.privacyZone")}</span>
    </span>
  {:else if vault.editorBlockPrivate}
    <span class="zone" title={t("statusBar.privateBlockHint")}>
      <PrivacyIcon />
      <span>{t("statusBar.privateBlock")}</span>
    </span>
  {/if}
  {#if vault.editorOrigin && vault.editorOrigin.origin !== "you"}
    <span class="origin origin-{vault.editorOrigin.origin}" title={originLabel(vault.editorOrigin)}>{originLabel(vault.editorOrigin, { withState: false })}</span>
  {/if}
  {#if vault.pendingCount > 0 && prefs.provenanceMarks}
    <span class="pending-group">
      <button class="pending" title={t("prov.pendingHint")} onclick={() => void executeCommand("provenance-next-pending")}>{t("prov.pendingCount", { n: vault.pendingCount })}</button>
      <!-- the review's steps: the previous or next unreviewed change, on to the next note at either end -->
      <button class="pending step" title={t("prov.prevHint", { key: k("Mod+Alt+↑") })} onclick={() => void executeCommand("provenance-prev-pending")}>↑</button>
      <button class="pending step" title={t("prov.nextHint", { key: k("Mod+Alt+↓") })} onclick={() => void executeCommand("provenance-next-pending")}>↓</button>
    </span>
    <!-- one click for this note (the palette command carries the toast); the labels say "this note", not "all" — the vault chip beside them is the wider scope -->
    <button class="pending" title={t("prov.acceptAll")} onclick={() => void executeCommand("provenance-accept-all")}>{t("prov.acceptAllShort")}</button>
    <button class="pending" title={t("prov.rejectAll")} onclick={() => void executeCommand("provenance-reject-all")}>{t("prov.rejectAllShort")}</button>
  {/if}
  {#if prefs.provenanceMarks && others.notes > 0}
    <!-- unreviewed AI blocks in other notes too (an agent run that wrote many): the count opens the list of those notes -->
    <button class="pending pending-vault" title={t("prov.pendingListHint")} onclick={pendingList}
      >{t(vault.pendingCount > 0 ? "prov.pendingOthers" : "prov.pendingVault", { n: others.blocks, m: others.notes })} ▾</button
    >
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
    -webkit-user-select: none;
    user-select: none;
    overflow: hidden;
    white-space: nowrap;
  }
  .vault-name {
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .zone {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    color: var(--accent);
    flex-shrink: 0;
  }
  .zone :global(svg) {
    display: block;
  }
  .origin {
    max-width: 40vw;
    overflow: hidden;
    text-overflow: ellipsis;
    flex-shrink: 1;
  }
  .origin-agent {
    color: var(--prov-agent);
  }
  .origin-clip {
    color: var(--prov-clip);
  }
  .origin-capture {
    color: var(--prov-capture);
  }
  .pending {
    padding: 0 8px;
    border-radius: 999px;
    border: 1px solid var(--prov-agent);
    color: var(--prov-agent);
    background: none;
    font-size: var(--fs-xs);
    cursor: pointer;
    flex-shrink: 0;
  }
  .pending:hover {
    background: color-mix(in srgb, var(--prov-agent) 14%, transparent);
  }
  /* the count and its two steps read as one control */
  .pending-group {
    display: inline-flex;
    gap: 2px;
    flex-shrink: 0;
  }
  .pending.step {
    padding: 0 5px;
  }
  /* the vault chip is quieter than this note's: no ring until hovered */
  .pending-vault {
    border-color: transparent;
    opacity: 0.85;
  }
  .pending-vault:hover {
    border-color: var(--prov-agent);
    opacity: 1;
  }
  .right {
    margin-left: auto;
    display: inline-flex;
    gap: 12px;
    flex-shrink: 0;
    font-variant-numeric: tabular-nums;
  }
</style>
