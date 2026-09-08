<script lang="ts">
  // In-app preview for images and PDFs. An image loads its own bytes and owns its
  // blob URL; a PDF goes through pdf.js (PdfViewer, one parsed document shared with
  // the translation view), with a toolbar for reading it beside its translation.
  import { t } from "$lib/i18n/i18n.svelte";
  import { api } from "$lib/api";
  import { fileKind, mimeOf } from "$lib/stores/vault.svelte";
  import { executeCommand } from "$lib/commands/registry";
  import { k } from "$lib/shortcut";
  import PdfViewer from "./PdfViewer.svelte";

  let { path }: { path: string } = $props();

  let url = $state<string | null>(null);
  let failed = $state<string | null>(null);
  const isPdf = $derived(fileKind(path) === "pdf");
  const name = $derived(path.split("/").pop() ?? path);

  $effect(() => {
    const p = path;
    if (fileKind(p) === "pdf") {
      url = null;
      failed = null;
      return;
    }
    let revoked: string | null = null;
    let stale = false;
    url = null;
    failed = null;
    api
      .fileReadBinary(p)
      .then((buf) => {
        if (stale) return;
        revoked = URL.createObjectURL(new Blob([buf], { type: mimeOf(p) }));
        url = revoked;
      })
      .catch((e) => {
        if (!stale) failed = String(e);
      });
    return () => {
      stale = true;
      if (revoked) URL.revokeObjectURL(revoked);
    };
  });

  function openWithSystem() {
    api.fileOpenSystem(path).catch(() => {});
  }
</script>

{#if failed}
  <div class="fallback">
    <p class="reason">{failed}</p>
    <button onclick={openWithSystem}>{t("editor.openWithSystem")}</button>
  </div>
{:else if isPdf}
  <div class="bar">
    <span class="name" title={path}>{name}</span>
    <button title={t("filePreview.translateHint", { key: k("Mod+Shift+T") })} onclick={() => void executeCommand("translate-toggle")}>{t("filePreview.translate")}</button>
  </div>
  {#key path}
    <PdfViewer {path} onFailed={(r) => (failed = r)} />
  {/key}
{:else if url}
  <div class="image-wrap">
    <img src={url} alt={path} />
  </div>
{/if}

<style>
  .bar {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 6px 14px;
    border-bottom: 1px solid var(--border);
    flex-shrink: 0;
  }
  .bar .name {
    font-size: var(--fs-sm);
    color: var(--fg-muted);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    min-width: 0;
    flex: 1;
  }
  .bar button {
    font-size: var(--fs-xs);
    padding: 3px 8px;
    border: 1px solid var(--accent);
    border-radius: 6px;
    color: var(--accent);
  }
  .bar button:hover {
    background: var(--bg-hover);
  }
  .image-wrap {
    flex: 1;
    min-height: 0;
    overflow: auto;
    display: grid;
    place-items: center;
    padding: 24px;
  }
  img {
    max-width: 100%;
    max-height: 100%;
    object-fit: contain;
    border-radius: var(--radius);
  }
  .fallback {
    flex: 1;
    display: grid;
    place-content: center;
    gap: 12px;
    justify-items: center;
    color: var(--fg-muted);
  }
  .reason {
    font-size: var(--fs-sm);
  }
  .fallback button {
    padding: 6px 14px;
    border: 1px solid var(--border);
    border-radius: var(--radius);
    background: var(--bg-panel);
  }
  .fallback button:hover {
    background: var(--bg-hover);
  }
</style>
