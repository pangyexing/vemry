<script lang="ts">
  // The PDF preview: pdf.js pages stacked at the pane's width, each drawn as it
  // comes near, with a selectable text layer. Replaces the iframe's built-in
  // viewer, which exposed no page number and could not be scrolled from outside;
  // this one reports the page in view and can be sent to a page.
  import { t } from "$lib/i18n/i18n.svelte";
  import { loadPdf, type PDFDocumentProxy } from "$lib/pdf";
  import PdfPage from "./PdfPage.svelte";

  let {
    path,
    onFailed,
  }: {
    path: string;
    /** The file could not be parsed: the host falls back (open with the system). */
    onFailed?: (reason: string) => void;
  } = $props();

  let doc = $state<PDFDocumentProxy | null>(null);
  let root = $state<HTMLDivElement | null>(null);
  /** The page most in view (the header's counter). */
  let current = $state(1);

  $effect(() => {
    const p = path;
    let stale = false;
    doc = null;
    loadPdf(p).then(
      (d) => { if (!stale) doc = d; },
      (e) => { if (!stale) onFailed?.(String(e)); },
    );
    return () => { stale = true; };
  });

  function onscroll() {
    if (!root) return;
    const mid = root.scrollTop + root.clientHeight / 3;
    let n = 1;
    for (const el of root.querySelectorAll<HTMLElement>(".slot")) {
      if (el.offsetTop <= mid) n = Number(el.dataset.n);
      else break;
    }
    current = n;
  }

  /** Scroll page `n` to the top (a citation's "jump to page", the translation's row). */
  export function scrollToPage(n: number) {
    root?.querySelector<HTMLElement>(`.slot[data-n="${n}"]`)?.scrollIntoView({ block: "start" });
  }
</script>

<div class="viewer" bind:this={root} {onscroll}>
  {#if doc}
    <div class="counter">{t("filePreview.pageOf", { n: current, total: doc.numPages })}</div>
    <div class="pages">
      {#each Array.from({ length: doc.numPages }, (_, i) => i + 1) as n (n)}
        <div class="slot" data-n={n}>
          <PdfPage {doc} {n} textLayer {root} />
        </div>
      {/each}
    </div>
  {:else}
    <div class="loading">{t("filePreview.loading")}</div>
  {/if}
</div>

<style>
  .viewer {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    background: var(--bg-panel);
    position: relative;
  }
  .counter {
    position: sticky;
    top: 8px;
    float: right;
    margin-right: 14px;
    z-index: 1;
    font-size: var(--fs-xs);
    font-variant-numeric: tabular-nums;
    color: var(--fg-muted);
    background: var(--bg);
    border: 1px solid var(--border);
    border-radius: 999px;
    padding: 1px 8px;
  }
  .pages {
    max-width: 920px;
    margin: 0 auto;
    padding: 16px 24px 48px;
    display: flex;
    flex-direction: column;
    gap: 14px;
  }
  .loading {
    padding: 40px;
    text-align: center;
    color: var(--fg-faint);
    font-size: var(--fs-sm);
  }
</style>
