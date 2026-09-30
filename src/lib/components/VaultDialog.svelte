<script lang="ts">
  // The vault name dialog (see stores/vaultDialog): the folder, the name it is shown by,
  // and for a folder being opened for the first time what opening it means.
  import { t } from "$lib/i18n/i18n.svelte";
  import { vaultDialog } from "$lib/stores/vaultDialog.svelte";
  import { folderName } from "$lib/stores/vault.svelte";
  import { isImeKeydown } from "$lib/ime";

  let name = $state("");

  $effect(() => {
    if (vaultDialog.pending) name = vaultDialog.pending.name;
  });

  const TITLE = { open: "vaultDialog.openTitle", create: "vaultDialog.createTitle", rename: "vaultDialog.renameTitle" } as const;
  const ACTION = { open: "vaultDialog.open", create: "vaultDialog.create", rename: "vaultDialog.rename" } as const;

  function onkeydown(e: KeyboardEvent) {
    if (vaultDialog.pending && e.key === "Escape") vaultDialog.answer(null);
  }
</script>

<svelte:window {onkeydown} />

{#if vaultDialog.pending}
  {@const req = vaultDialog.pending}
  <div class="overlay" role="presentation">
    <div class="dialog" role="dialog" aria-label={t(TITLE[req.mode])}>
      <h2>{t(TITLE[req.mode])}</h2>
      <!-- lrm bookends: under the rtl truncation trick, the neutral characters at a path's ends would be reordered by bidi -->
      <code class="folder" title={req.folder}>&lrm;{req.folder}&lrm;</code>
      <label>
        <span>{t("vaultDialog.name")}</span>
        <!-- svelte-ignore a11y_autofocus -->
        <input
          autofocus
          bind:value={name}
          placeholder={folderName(req.folder)}
          spellcheck="false"
          onfocus={(e) => e.currentTarget.select()}
          onkeydown={(e) => e.key === "Enter" && !isImeKeydown(e) && vaultDialog.answer(name)}
        />
      </label>
      <small>{t("vaultDialog.nameHint")}</small>
      {#if req.mode === "open"}<p>{t("vaultDialog.openBody")}</p>{/if}
      <footer>
        <button class="btn" onclick={() => vaultDialog.answer(null)}>{t("common.cancel")}</button>
        <button class="btn primary" onclick={() => vaultDialog.answer(name)}>{t(ACTION[req.mode])}</button>
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
    width: min(440px, 90vw);
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
  .folder {
    font-family: var(--font-mono);
    font-size: var(--fs-xs);
    color: var(--fg-muted);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    direction: rtl;
    text-align: left;
  }
  label {
    display: flex;
    flex-direction: column;
    gap: 6px;
    font-size: var(--fs-sm);
  }
  input {
    padding: 7px 10px;
    font-size: var(--fs-sm);
  }
  p {
    font-size: var(--fs-sm);
    line-height: 1.6;
  }
  small {
    font-size: var(--fs-xs);
    color: var(--fg-muted);
    line-height: 1.6;
    margin-top: -6px;
  }
  footer {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
  }
  .btn {
    font-size: var(--fs-sm);
    padding: 6px 14px;
    border-radius: 6px;
    border: 1px solid var(--border);
  }
  .btn.primary {
    background: var(--accent);
    border-color: var(--accent);
    color: var(--bg);
  }
</style>
