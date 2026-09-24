<script lang="ts">
  // The "night reading" switch of a PDF view (PdfPage's pdfNight), shown only while the
  // system is dark — on a light theme the white page is already the right page.
  import { t } from "$lib/i18n/i18n.svelte";
  import { pdfNight, pdfScheme, setPdfNight } from "./PdfPage.svelte";
</script>

{#if pdfScheme.dark}
  <button class="night" class:on={pdfNight.on} aria-pressed={pdfNight.on} title={t("filePreview.nightHint")} onclick={() => setPdfNight(!pdfNight.on)}
    >{t("filePreview.night")}</button
  >
{/if}

<style>
  .night {
    font-size: var(--fs-xs);
    padding: 3px 8px;
    border: 1px solid var(--border);
    border-radius: 6px;
    color: var(--fg-muted);
    white-space: nowrap;
  }
  .night:hover {
    background: var(--bg-hover);
    color: var(--fg);
  }
  .night.on {
    color: var(--accent);
    border-color: var(--accent);
  }
</style>
