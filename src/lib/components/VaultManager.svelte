<script lang="ts">
  // Manage vaults: every vault on the recent list (and the open one), each opened,
  // renamed, shown in the file manager or removed from the list here. A vault whose
  // folder has gone away can only be removed.
  import { t, tn, type MessageKey } from "$lib/i18n/i18n.svelte";
  import { api } from "$lib/api";
  import { executeCommand } from "$lib/commands/registry";
  import { contextMenu } from "$lib/stores/contextMenu.svelte";
  import { toast } from "$lib/stores/toast.svelte";
  import { vault } from "$lib/stores/vault.svelte";
  import { vaultDialog, vaultManager } from "$lib/stores/vaultDialog.svelte";
  import { isMac } from "$lib/shortcut";

  const isWindows = /Win/.test(navigator.platform || navigator.userAgent);
  const SHOW_FOLDER: MessageKey = isMac
    ? "vaultManager.showInFinder"
    : isWindows
      ? "vaultManager.showInExplorer"
      : "vaultManager.showInFileManager";

  /** The open vault is always listed, even before the recent list has it. */
  const rows = $derived(vault.root && !vault.recent.includes(vault.root) ? [vault.root, ...vault.recent] : vault.recent);

  // Re-read on every opening, so a folder moved away since shows as missing
  $effect(() => {
    if (vaultManager.open) void vault.loadRecent();
  });

  function close() {
    vaultManager.open = false;
  }

  function openVault(path: string) {
    close();
    void vault.openVault(path);
  }

  function run(id: string) {
    close();
    void executeCommand(id);
  }

  async function rename(path: string) {
    const name = await vaultDialog.ask("rename", path, vault.nameOf(path));
    if (name === null) return;
    try {
      await vault.renameVault(path, name);
    } catch (e) {
      toast.push(String(e), "error");
    }
  }

  function more(e: MouseEvent, path: string) {
    const missing = vault.recentInfo[path]?.missing ?? false;
    contextMenu.open(e, [
      { label: t("vaultManager.rename"), disabled: missing, action: () => void rename(path) },
      {
        label: t(SHOW_FOLDER),
        disabled: missing,
        action: () => void api.vaultShowFolder(path).catch((err) => toast.push(String(err), "error")),
      },
      { separator: true },
      // Disabled for the vault that is open: the next open would only add it back
      { label: t("fileTree.removeFromRecent"), disabled: path === vault.root, action: () => void vault.removeRecent(path) },
    ]);
  }

  function onkeydown(e: KeyboardEvent) {
    // A name question asked from here is answered first (VaultDialog closes it)
    if (vaultManager.open && !vaultDialog.pending && e.key === "Escape") close();
  }
</script>

<svelte:window {onkeydown} />

{#if vaultManager.open}
  <div class="overlay" role="presentation">
    <div class="dialog" role="dialog" aria-label={t("vaultManager.title")}>
      <h2>{t("vaultManager.title")}</h2>
      <ul class="list">
        {#each rows as path (path)}
          {@const current = path === vault.root}
          {@const missing = vault.recentInfo[path]?.missing ?? false}
          <li class="row">
            <div class="text">
              <span class="name-line">
                <span class="name" class:gone={missing}>{vault.nameOf(path)}</span>
                {#if current}
                  <span class="badge">{t("vaultManager.current")}</span>
                  <span class="count">{tn("fileTree.noteCount", vault.noteCount)}</span>
                {/if}
                {#if missing}<span class="badge">{t("vaultManager.missing")}</span>{/if}
              </span>
              <!-- lrm bookends: under the rtl truncation trick, the neutral characters at a path's ends would be reordered by bidi -->
              <span class="path" title={path}>&lrm;{path}&lrm;</span>
            </div>
            {#if !current && !missing}
              <button class="btn" onclick={() => openVault(path)}>{t("vaultManager.open")}</button>
            {/if}
            <button
              class="more"
              title={t("common.more")}
              aria-label={t("common.more")}
              onclick={(e) => {
                e.stopPropagation();
                more(e, path);
              }}
            >
              <svg width="14" height="14" viewBox="0 0 16 16" fill="currentColor" aria-hidden="true">
                <circle cx="3.5" cy="8" r="1.3" />
                <circle cx="8" cy="8" r="1.3" />
                <circle cx="12.5" cy="8" r="1.3" />
              </svg>
            </button>
          </li>
        {:else}
          <li class="empty">{t("vaultManager.empty")}</li>
        {/each}
      </ul>
      <footer>
        <button class="btn" onclick={() => run("open-vault")}>{t("palette.openOtherVault")}</button>
        <button class="btn" onclick={() => run("create-vault")}>{t("cmd.create-vault.title")}</button>
        <span class="spacer"></span>
        <button class="btn" onclick={close}>{t("common.close")}</button>
      </footer>
    </div>
  </div>
{/if}

<style>
  .overlay {
    position: fixed;
    inset: 0;
    background: var(--scrim);
    display: grid;
    place-items: center;
    z-index: 200;
  }
  .dialog {
    width: min(520px, 90vw);
    max-height: 80vh;
    background: var(--bg-panel);
    border: 1px solid var(--border);
    border-radius: 10px;
    box-shadow: var(--shadow);
    padding: 20px;
    display: flex;
    flex-direction: column;
    gap: 12px;
  }
  h2 {
    font-size: var(--fs-lg);
    font-weight: 600;
  }
  .list {
    list-style: none;
    margin: 0;
    padding: 0;
    overflow-y: auto;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .row {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 6px 8px;
    border-radius: 6px;
  }
  .row:hover {
    background: var(--bg-hover);
  }
  .text {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .name-line {
    display: flex;
    align-items: baseline;
    gap: 8px;
    min-width: 0;
  }
  .name {
    font-size: var(--fs-sm);
    color: var(--fg);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .name.gone {
    color: var(--fg-muted);
    text-decoration: line-through;
  }
  .badge,
  .count {
    font-size: var(--fs-xs);
    color: var(--fg-muted);
    white-space: nowrap;
    flex-shrink: 0;
  }
  .path {
    font-family: var(--font-mono);
    font-size: var(--fs-xs);
    color: var(--fg-muted);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    direction: rtl;
    text-align: left;
  }
  .empty {
    font-size: var(--fs-sm);
    color: var(--fg-muted);
    padding: 6px 8px;
  }
  .more {
    display: grid;
    place-items: center;
    width: 26px;
    height: 26px;
    border-radius: 5px;
    color: var(--fg-muted);
    flex-shrink: 0;
  }
  .more:hover {
    background: var(--bg-active);
    color: var(--fg);
  }
  footer {
    display: flex;
    gap: 8px;
    flex-wrap: wrap;
  }
  .spacer {
    flex: 1;
  }
  .btn {
    font-size: var(--fs-sm);
    padding: 6px 14px;
    border-radius: 6px;
    border: 1px solid var(--border);
    white-space: nowrap;
    flex-shrink: 0;
  }
</style>
