<script lang="ts">
  // One page of a PDF, drawn when it comes near the viewport and redrawn when its
  // column changes width. The preview stacks these; the translation view puts one
  // in each row's left cell. The aspect ratio is reserved before the draw, so the
  // page list does not jump as pages come in.
  import { renderPage, textRects, type PageRect, type PDFDocumentProxy } from "$lib/pdf";

  let {
    doc,
    n,
    textLayer = false,
    root = null,
    highlight = null,
  }: {
    doc: PDFDocumentProxy;
    /** 1-based page number. */
    n: number;
    /** Lay a selectable text layer over the page (the preview does; the translation rows do not). */
    textLayer?: boolean;
    /** The scrolling ancestor the visibility is measured against (null = the viewport). */
    root?: HTMLElement | null;
    /** A passage of the page's text to light up (the translation row the reader picked); null = none. */
    highlight?: string | null;
  } = $props();

  let host = $state<HTMLDivElement | null>(null);
  let canvas = $state<HTMLCanvasElement | null>(null);
  let layer = $state<HTMLDivElement | null>(null);
  let ratio = $state(1.4142);
  let near = $state(false);
  let width = $state(0);
  let drawnWidth = 0;
  let drawing: Promise<unknown> | null = null;
  let rects = $state<PageRect[]>([]);

  // The passage to light up: its boxes are looked up by text, and the first one is brought into view
  $effect(() => {
    const text = highlight;
    if (!text) { rects = []; return; }
    let stale = false;
    void doc.getPage(n).then((p) => textRects(p, text)).then((r) => {
      if (stale) return;
      rects = r;
      queueMicrotask(() => host?.querySelector(".hl")?.scrollIntoView({ block: "nearest" }));
    });
    return () => { stale = true; };
  });

  // The page's shape first (cheap: no drawing), so the row has its height at once
  $effect(() => {
    let stale = false;
    void doc.getPage(n).then((p) => {
      if (stale) return;
      const v = p.getViewport({ scale: 1 });
      ratio = v.height / v.width;
    });
    return () => { stale = true; };
  });

  $effect(() => {
    if (!host) return;
    const io = new IntersectionObserver(([e]) => (near = e.isIntersecting), { root, rootMargin: "600px 0px" });
    io.observe(host);
    const ro = new ResizeObserver(([e]) => (width = Math.floor(e.contentRect.width)));
    ro.observe(host);
    return () => { io.disconnect(); ro.disconnect(); };
  });

  // Draw when near and the width is known; redraw on a width change of more than a few pixels
  $effect(() => {
    const w = width;
    if (!near || !canvas || w < 40 || Math.abs(w - drawnWidth) < 4) return;
    const target = canvas;
    const text = textLayer ? layer : null;
    const run = async () => {
      await drawing;
      const page = await doc.getPage(n);
      drawnWidth = w;
      await renderPage(page, target, w, text ?? undefined);
    };
    drawing = run().catch(() => { drawnWidth = 0; });
  });
</script>

<div class="page" bind:this={host} style:aspect-ratio={`1 / ${ratio}`}>
  <canvas bind:this={canvas}></canvas>
  {#if textLayer}
    <div class="textLayer" bind:this={layer}></div>
  {/if}
  {#each rects as r, i (i)}
    <div class="hl" style:left={`${r.left * 100}%`} style:top={`${r.top * 100}%`} style:width={`${r.width * 100}%`} style:height={`${r.height * 100}%`}></div>
  {/each}
</div>

<style>
  .page {
    position: relative;
    width: 100%;
    background: #fff;
    box-shadow: 0 0 0 1px var(--border);
    overflow: hidden;
  }
  canvas {
    display: block;
    width: 100%;
  }
  /* The lit passage: one box per text run, multiplied onto the page so overlaps do not darken */
  .hl {
    position: absolute;
    background: var(--mark-bg, rgba(255, 213, 79, 0.45));
    mix-blend-mode: multiply;
    border-radius: 2px;
    pointer-events: none;
    padding: 1px 0;
    margin-top: -1px;
  }
  /* pdf.js's text layer: transparent glyph boxes over the canvas, sized by --scale-factor, selectable */
  .textLayer {
    position: absolute;
    inset: 0;
    overflow: hidden;
    line-height: 1;
    text-size-adjust: none;
    -webkit-text-size-adjust: none;
    forced-color-adjust: none;
    transform-origin: 0 0;
    -webkit-user-select: text;
    user-select: text;
  }
  .textLayer :global(span),
  .textLayer :global(br) {
    color: transparent;
    position: absolute;
    white-space: pre;
    cursor: text;
    transform-origin: 0% 0%;
  }
  .textLayer :global(::selection) {
    background: rgba(0, 120, 215, 0.35);
  }
  .textLayer :global(.endOfContent) {
    display: block;
    position: absolute;
    inset: 100% 0 0;
    z-index: 0;
    cursor: default;
    -webkit-user-select: none;
    user-select: none;
  }
</style>
