<script lang="ts">
  // In-app preview for images and PDFs. Loads its own bytes and owns its blob
  // URL, so multiple instances can coexist (a future split view: the PDF beside its
  // translation).
  import { t } from "$lib/i18n/i18n.svelte";
  import { api } from "$lib/api";
  import { fileKind, mimeOf } from "$lib/stores/vault.svelte";

  let { path }: { path: string } = $props();

  let url = $state<string | null>(null);
  let failed = $state<string | null>(null);

  $effect(() => {
    const p = path;
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
{:else if url && fileKind(path) === "pdf"}
  <iframe class="pdf" src={url} title={path}></iframe>
{:else if url}
  <div class="image-wrap">
    <img src={url} alt={path} />
  </div>
{/if}

<style>
  .pdf {
    flex: 1;
    min-height: 0;
    width: 100%;
    border: none;
    background: var(--bg-panel);
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
