<script lang="ts">
  // Side-by-side translation: the source on the left, its translation on the right,
  // one grid row per segment — the row takes the taller side's height, so the two
  // columns stay aligned by construction and nothing has to synchronise scrolling.
  // Rows fill in as the model streams; the rows on screen are asked for first.
  // The store holds the document and the translations; this only renders and
  // reports what is visible. Leaving goes through vault.exitTranslate (Esc, ✕),
  // which is what closes the job.
  //
  // The text the reader is on does not move while the translation arrives: a streaming
  // cell renders through the same renderer as the finished one (only its unclosed tail is
  // held back), a cell not translated yet reserves about the height its translation will
  // take, images keep their size from the first sight, and whatever still changes height
  // above the viewport is paid back by holding the first visible row where it was.
  import { untrack } from "svelte";
  import { i18n, t, tn } from "$lib/i18n/i18n.svelte";
  import { api, type TranslateSegment } from "$lib/api";
  import { translate, targetLabel, fmtTokens, TARGET_TAGS, type TranslateMode } from "$lib/stores/translate.svelte";
  import { vault } from "$lib/stores/vault.svelte";
  import { toast } from "$lib/stores/toast.svelte";
  import { executeCommand } from "$lib/commands/registry";
  import { handleCodeCopyClick, renderNoteMarkdown, streamingTail } from "$lib/markdown";
  import { langAttrs, noteLanguage, scriptOfLang, type NoteLanguage, type NoteScript } from "$lib/notelang";
  import { captureAnchor, restoreAnchor, type ScrollAnchor } from "$lib/scrollanchor";
  import { vaultImages } from "$lib/views/assets";
  import { selectingIn } from "$lib/views/select";
  import { cliptip } from "$lib/cliptip";
  import { k } from "$lib/shortcut";
  import { loadPdf, segmentTops, type PDFDocumentProxy } from "$lib/pdf";
  import PdfPage from "$lib/components/PdfPage.svelte";
  import PdfNightToggle from "$lib/components/PdfNightToggle.svelte";
  import ViewHeader from "./ViewHeader.svelte";
  import "katex/dist/katex.min.css";

  /** A column's cap in em at the body size, by the writing system of the text in it: about
   *  36 ideographs to a CJK line, about 65 letters to a Latin one. Each column is capped by
   *  its own text, so an English source beside a Chinese translation gets two widths. */
  const colEm = (s: NoteScript) => (s === "cjk" ? 36 : 32);
  /** A PDF page is not a line of text: it wants the room to be legible as a picture. */
  const PAGE_EM = 50;
  /** The space between the two columns: the centre rule and a row's tools live in it. */
  const GUTTER_PX = 44;
  /** Visibility reports are batched: a scroll fires many intersections in a row. */
  const REPORT_MS = 150;

  const doc = $derived(translate.doc);
  let root = $state<HTMLDivElement | null>(null);
  const isPdf = $derived(doc?.source.kind === "pdf");
  /** The layout is the reader's choice alone: a pane too narrow for two columns scrolls
   *  sideways instead of re-flowing every row into the other layout under the reader. */
  const mode = $derived<TranslateMode>(translate.mode);
  const total = $derived(translate.toTranslate.length);
  const done = $derived(translate.doneCount);
  const failed = $derived(translate.failedIds.length);
  /** Tokens spent on this document so far (0 until the provider reports any). */
  const spent = $derived(translate.usage.prompt + translate.usage.completion);
  const targets = $derived.by(() => {
    const cur = doc?.target ?? "";
    const list: string[] = [...TARGET_TAGS];
    if (cur && !list.includes(cur)) list.unshift(cur);
    return list;
  });

  const run = (p: Promise<unknown>) => void p.catch((e) => toast.push(String(e), "error"));

  // ---- languages: the source reads as the note does, the translation as its target ----
  const NONE: NoteLanguage = { lang: null, script: null };
  /** Judged once per document (R1: a column that changed width mid-read would move the text).
   *  A note's verdict comes from the index, which also reads a declared `lang:` in the
   *  frontmatter the segments do not carry; a page or a PDF is judged from its text. */
  const srcLang = $derived.by<NoteLanguage>(() => {
    const d = doc;
    if (!d) return NONE;
    if (d.source.kind === "note") {
      const path = d.source.path;
      const n = untrack(() => vault.noteById.get(path));
      if (n && (n.lang || n.script)) return { lang: n.lang, script: n.script };
    }
    return noteLanguage(d.segments.filter((s) => !s.private).map((s) => s.prefix + s.text).join("\n\n"));
  });
  const trLang = $derived<NoteLanguage>(doc ? { lang: doc.target, script: scriptOfLang(doc.target) } : NONE);
  const srcAttrs = $derived(langAttrs(srcLang));
  const trAttrs = $derived(langAttrs(trLang));
  /** A text too short to judge follows the interface, as the rest of the app does. */
  const uiScript = $derived(scriptOfLang(i18n.locale));
  const srcScript = $derived(srcLang.script ?? uiScript);
  const trScript = $derived(trLang.script ?? uiScript);
  /** Side by side: the two capped columns (their proportions kept as the pane narrows) and
   *  the gutter, centred as a whole; stacked, the one column is capped by the source's text
   *  and translation-only by the translation's (app.css [data-script] → --content-width). */
  const layout = $derived.by(() => {
    const a = isPdf ? PAGE_EM : colEm(srcScript);
    const b = colEm(trScript);
    return { cols: `minmax(0, ${a}fr) ${GUTTER_PX}px minmax(0, ${b}fr)`, max: `calc(${a + b}em + ${GUTTER_PX}px)` };
  });
  const rowsScript = $derived(mode === "side" ? undefined : mode === "stack" ? srcScript : trScript);

  // ---- a PDF: rows are pages, the page drawn on the left and its paragraphs on the right ----
  let pdfDoc = $state<PDFDocumentProxy | null>(null);
  $effect(() => {
    const src = doc?.source;
    if (src?.kind !== "pdf") { pdfDoc = null; return; }
    let stale = false;
    loadPdf(src.path).then(
      (d) => { if (!stale) pdfDoc = d; },
      (e) => { if (!stale) toast.push(String(e), "error"); },
    );
    return () => { stale = true; };
  });
  /** The text of the lit segment when it sits on this page — what the page lights up. */
  function litOn(g: { page: number; segs: TranslateSegment[] }): string | null {
    const id = translate.selected;
    if (id == null) return null;
    return g.segs.find((s) => s.id === id)?.text ?? null;
  }
  /** A PDF's segments by page, in order (a page with no text still gets a row: the picture is the content). */
  const pageGroups = $derived.by(() => {
    if (doc?.source.kind !== "pdf") return [];
    const total = pdfDoc?.numPages ?? Math.max(1, ...doc.segments.map((s) => s.page ?? 1));
    const groups: { page: number; segs: TranslateSegment[] }[] = Array.from({ length: total }, (_, i) => ({ page: i + 1, segs: [] }));
    for (const s of doc.segments) {
      const g = groups[(s.page ?? 1) - 1];
      if (g) g.segs.push(s);
    }
    return groups;
  });
  /** The segments a page shows as cells beside it. */
  const cellsOf = (g: { segs: TranslateSegment[] }) => g.segs.filter((s) => s.kind.type !== "aside" && !s.private);

  /** Where each PDF segment starts on its page (a fraction of the page height), by segment
   *  id, and which pages have been looked up. A page's cells are only drawn once its
   *  positions are known, so each cell appears where it belongs rather than at the top and
   *  then sliding down; pages are looked up as they come near, not the whole document first
   *  (on a long PDF that held every cell back, then moved them all at once). */
  let tops = $state<Record<number, number>>({});
  let measured = $state<Record<number, true>>({});
  let measuring = new Set<number>();
  let measureGen = 0;
  $effect(() => {
    void pdfDoc;
    void doc;
    tops = {};
    measured = {};
    measuring = new Set();
    measureGen++;
    // The pages already near were reported before there was a document to look them up in;
    // observing them afresh reports them again
    const obs = untrack(() => pageIo);
    if (obs) for (const node of pageNodes.keys()) { obs.unobserve(node); obs.observe(node); }
  });
  async function measurePage(g: { page: number; segs: TranslateSegment[] }) {
    const d = pdfDoc;
    if (!d || measured[g.page] || measuring.has(g.page)) return;
    measuring.add(g.page);
    const gen = measureGen;
    const segs = cellsOf(g);
    try {
      if (segs.length) {
        const page = await d.getPage(g.page);
        const found = await segmentTops(page, segs.map((s) => s.text));
        if (gen !== measureGen) return;
        const next = { ...tops };
        segs.forEach((s, i) => { const f = found[i]; if (f != null) next[s.id] = f; });
        tops = next;
      }
    } catch {
      // no positions: the cells stack from the page's top, the same as a page with no text layer
    }
    if (gen === measureGen) measured = { ...measured, [g.page]: true };
  }
  /** The measured heights: the page cell per page, each translation cell per segment (bind:clientHeight). */
  let pageHeights = $state<Record<number, number>>({});
  let cellHeights = $state<Record<number, number>>({});
  /** Each cell's top margin so it sits at its source's height on the page — or right under the cell above when that one is taller than its block (side mode only). */
  const cellGaps = $derived.by(() => {
    const gaps: Record<number, number> = {};
    if (mode !== "side") return gaps;
    for (const g of pageGroups) {
      const ph = pageHeights[g.page] ?? 0;
      let bottom = 0;
      for (const s of cellsOf(g)) {
        const want = tops[s.id] != null ? tops[s.id] * ph : bottom;
        const gap = Math.max(0, want - bottom);
        gaps[s.id] = gap;
        bottom += gap + (cellHeights[s.id] ?? 0);
      }
    }
    return gaps;
  });

  // ---- rendering ----
  /** One segment's HTML from prefix + body: the note renderer (wikilinks, callouts, KaTeX, local images as data-asset). */
  function html(prefix: string, body: string): string {
    return renderNoteMarkdown(prefix + body);
  }

  const images = vaultImages();
  $effect(() => () => images.dispose());

  /** How wide a segment's text runs, in em at the body size: an ideograph is a whole em,
   *  a Latin letter a little over half, a space a third. Memoised per segment: the
   *  placeholders read it on every render. */
  const widthMemo = new Map<number, number>();
  const WIDE = /[\p{Script=Han}\p{Script=Hiragana}\p{Script=Katakana}\p{Script=Hangul}　-〿＀-￯]/u;
  function textEm(seg: TranslateSegment): number {
    let w = widthMemo.get(seg.id);
    if (w == null) {
      w = 0;
      for (const ch of seg.text) w += WIDE.test(ch) ? 1 : ch === " " ? 0.3 : 0.55;
      widthMemo.set(seg.id, w);
    }
    return w;
  }
  /** The translation column's width in em, measured (rounded, so the estimates below only
   *  change on a real change of width). */
  let trColEm = $state(30);
  /** The lines a translation not here yet will take: the source's run scaled by how the two
   *  writing systems compare (a Chinese rendering of English prose takes about 0.6 of its
   *  width, the other way round about 1.7), over the column's width. A heading's line is
   *  taller. The row then keeps its height when the text arrives. */
  function estLines(seg: TranslateSegment): number {
    const ratio = srcScript === "cjk" && trScript !== "cjk" ? 1.7 : srcScript !== "cjk" && trScript === "cjk" ? 0.6 : 1;
    const lines = Math.max(1, Math.ceil((textEm(seg) * ratio) / Math.max(8, trColEm)));
    return seg.kind.type === "heading" ? lines * 1.4 : lines;
  }
  function measureColumn() {
    const cell = root?.querySelector<HTMLElement>(".row .tr, .cell");
    if (!cell) return;
    const px = parseFloat(getComputedStyle(cell).fontSize) || 16;
    const em = Math.round(cell.clientWidth / px);
    if (em > 0 && em !== trColEm) trColEm = em;
  }

  // ---- the reader's row holds still ----
  let anchor: ScrollAnchor | null = null;
  function onscroll() {
    if (root) anchor = captureAnchor(root, true);
  }
  /** Rows change height after layout and before paint (a token arrived, an image loaded, a
   *  page learned its shape): one observer on every row, and each batch of changes puts the
   *  anchored row back before the frame is drawn. A change below the reader's row moves
   *  nothing above it, so the restore finds no drift and does nothing. */
  let ro: ResizeObserver | null = null;
  const held = new Set<HTMLElement>();
  function hold(node: HTMLElement) {
    held.add(node);
    ro?.observe(node);
    return { destroy: () => { held.delete(node); ro?.unobserve(node); } };
  }
  $effect(() => {
    const scroller = root;
    if (!scroller) return;
    ro = new ResizeObserver(() => {
      restoreAnchor(scroller, anchor);
      anchor = captureAnchor(scroller, true);
    });
    for (const n of held) ro.observe(n);
    const size = new ResizeObserver(() => measureColumn());
    size.observe(scroller);
    return () => { ro?.disconnect(); ro = null; size.disconnect(); };
  });
  // A layout switch changes the column's width without resizing the scroller
  $effect(() => {
    void mode;
    queueMicrotask(measureColumn);
  });

  // ---- what is on screen is asked for first ----
  const visible = new Set<number>();
  let reportTimer: ReturnType<typeof setTimeout> | null = null;
  function scheduleReport() {
    if (reportTimer) return;
    reportTimer = setTimeout(() => {
      reportTimer = null;
      const d = translate.doc;
      if (!d) return;
      const ids = d.segments
        .filter((s) => visible.has(s.id) && s.translate && !s.private)
        .filter((s) => {
          const r = translate.rows[s.id];
          return !r || (!r.done && r.text == null && !r.error);
        })
        .map((s) => s.id);
      if (ids.length) translate.request(ids);
    }, REPORT_MS);
  }
  let io: IntersectionObserver | null = null;
  let pageIo: IntersectionObserver | null = null;
  /** The rows on screen right now, so a rebuilt observer picks them all up again. */
  const observed = new Set<HTMLElement>();
  function observe(node: HTMLElement, id: number) {
    observed.add(node);
    io?.observe(node);
    return { destroy: () => { observed.delete(node); io?.unobserve(node); visible.delete(id); } };
  }
  /** A PDF page row: its positions are looked up as it comes within two screens. */
  const pageNodes = new Map<HTMLElement, { page: number; segs: TranslateSegment[] }>();
  function nearPage(node: HTMLElement, g: { page: number; segs: TranslateSegment[] }) {
    pageNodes.set(node, g);
    pageIo?.observe(node);
    return {
      update: (next: { page: number; segs: TranslateSegment[] }) => void pageNodes.set(node, next),
      destroy: () => { pageNodes.delete(node); pageIo?.unobserve(node); },
    };
  }
  // The observers measure against the scroller, so they are built here rather than in the
  // actions: reopening the document (a changed target language) mounts a new scroller,
  // and an observer left on the old, detached one never reports an intersection again —
  // the rows on screen would stop being asked for first.
  $effect(() => {
    const scroller = root;
    if (!scroller) return;
    visible.clear();
    io = new IntersectionObserver(
      (entries) => {
        for (const e of entries) {
          const sid = Number((e.target as HTMLElement).dataset.id);
          if (e.isIntersecting) visible.add(sid);
          else visible.delete(sid);
        }
        scheduleReport();
      },
      // the prefetch window: a screen behind the reader, two ahead
      { root: scroller, rootMargin: "100% 0px 200% 0px" },
    );
    for (const node of observed) io.observe(node);
    pageIo = new IntersectionObserver(
      (entries) => {
        for (const e of entries) {
          const g = pageNodes.get(e.target as HTMLElement);
          if (e.isIntersecting && g) void measurePage(g);
        }
      },
      { root: scroller, rootMargin: "200% 0px" },
    );
    for (const node of pageNodes.keys()) pageIo.observe(node);
    return () => { io?.disconnect(); io = null; pageIo?.disconnect(); pageIo = null; };
  });
  $effect(() => () => { if (reportTimer) clearTimeout(reportTimer); });

  // ---- interactions ----
  function select(id: number) {
    translate.selected = translate.selected === id ? null : id;
  }
  /** Double-clicking a source block: back to the editor, on that block. */
  function openSource(seg: TranslateSegment) {
    const d = translate.doc;
    if (!d || d.source.kind !== "note") return;
    const { path } = d.source;
    vault.exitTranslate();
    void vault.openBlockAt(path, seg.byte_start, seg.text);
  }
  async function copyRow(seg: TranslateSegment) {
    const r = translate.rows[seg.id];
    if (!r?.done || r.text == null) return;
    await navigator.clipboard.writeText(seg.prefix + r.text);
    toast.push(t("translate.copied"));
  }
  async function copyAll() {
    await navigator.clipboard.writeText(translate.allText());
    toast.push(t("translate.copiedAll"));
  }
  async function save() {
    if (!translate.doc || translate.saving) return;
    try {
      const path = await translate.save();
      toast.push(t("translate.saved", { path }));
    } catch (e) {
      toast.push(String(e), "error");
    }
  }
  /** How many translatable segments have no translation yet (the save button says so while it translates them). */
  const remaining = $derived(total - done);
  async function changeTarget(tag: string) {
    if (!doc || tag === doc.target) return;
    try {
      await api.settingsSet({ translate_target: tag });
      await translate.reopen();
    } catch (e) {
      toast.push(String(e), "error");
    }
  }
  /** Links inside a rendered cell: a wikilink follows in the editor, a web link opens by the link setting, a code block's copy button copies. */
  function onCellClick(e: MouseEvent) {
    if (handleCodeCopyClick(e)) return;
    const a = (e.target as HTMLElement).closest<HTMLElement>("a[data-wiki], a.md-link");
    if (!a) return;
    e.preventDefault();
    if (a.dataset.wiki) {
      vault.exitTranslate();
      void vault.followLink(a.dataset.wiki);
    } else if (a.dataset.url) {
      run(api.urlOpen(a.dataset.url, false));
    }
  }
  /** A click on a translation lights its row — unless it ended a drag that selected text there. */
  function onTrClick(e: MouseEvent, id: number) {
    onCellClick(e);
    if (!selectingIn(e.currentTarget as Element)) select(id);
  }
  /** J/K step the lit row; the keys are free in this view (no editor has the focus). */
  function onkeydown(e: KeyboardEvent) {
    if (!doc || e.metaKey || e.ctrlKey || e.altKey) return;
    if (e.target instanceof HTMLElement && e.target.closest("input, textarea, select, [contenteditable='true']")) return;
    const key = e.key.toLowerCase();
    if (key !== "j" && key !== "k") return;
    e.preventDefault();
    const ids = doc.segments.filter((s) => !s.private).map((s) => s.id);
    const at = translate.selected == null ? -1 : ids.indexOf(translate.selected);
    const next = ids[Math.min(ids.length - 1, Math.max(0, at + (key === "j" ? 1 : -1)))];
    if (next == null) return;
    translate.selected = next;
    root?.querySelector<HTMLElement>(`[data-id="${next}"]`)?.scrollIntoView({ block: "nearest" });
  }
</script>

<svelte:window {onkeydown} />

{#snippet status()}
  {#if doc}
    {#if failed > 0}
      <button class="chip warn" title={t("translate.retryFailed")} onclick={() => translate.retryFailed()}>
        {tn("translate.failedCount", failed)} · {t("translate.retryFailed")}
      </button>
    {:else if done >= total}
      <span class="status ok">{t("translate.complete")}</span>
    {:else}
      <span class="status" title={t("translate.progressHint")}>{t("translate.progress", { done, total })}</span>
    {/if}
  {/if}
{/snippet}

{#snippet tools()}
  {#if doc}
    <span class="actions">
      {#if isPdf}<PdfNightToggle />{/if}
      <select
        class="chip"
        title={t("translate.targetHint")}
        value={doc.target}
        onchange={(e) => void changeTarget((e.currentTarget as HTMLSelectElement).value)}
      >
        {#each targets as tag (tag)}
          <option value={tag}>{targetLabel(tag)}</option>
        {/each}
      </select>
      <select
        class="chip"
        title={t("translate.modeHint")}
        value={translate.mode}
        onchange={(e) => translate.setMode((e.currentTarget as HTMLSelectElement).value as TranslateMode)}
      >
        <option value="side">{t("translate.mode.side")}</option>
        <option value="stack">{t("translate.mode.stack")}</option>
        <option value="target">{t("translate.mode.target")}</option>
      </select>
      <button
        class="chip model"
        title={spent > 0 ? `${t("translate.modelHint")}\n${t("translate.usage", { n: fmtTokens(spent), input: fmtTokens(translate.usage.prompt), output: fmtTokens(translate.usage.completion) })}` : t("translate.modelHint")}
        onclick={() => void executeCommand("settings-ai")}
      >{doc.model}{#if spent > 0} · {fmtTokens(spent)}{/if}</button>
      <button title={t("translate.copyAll")} onclick={() => void copyAll()}>{t("translate.copy")}</button>
      <button class="primary" title={t("translate.saveHint")} disabled={translate.saving} onclick={() => void save()}
        >{translate.saving && remaining > 0 ? t("translate.savingRemaining", { n: remaining }) : t("translate.save")}</button
      >
    </span>
  {/if}
{/snippet}

{#snippet rowTools(seg: TranslateSegment, copy: boolean)}
  <span class="tools">
    <button title={t("translate.retranslate")} aria-label={t("translate.retranslate")} onclick={(e) => { e.stopPropagation(); translate.request([seg.id], true); }}>↻</button>
    {#if copy}
      <button title={t("translate.copy")} aria-label={t("translate.copy")} onclick={(e) => { e.stopPropagation(); void copyRow(seg); }}>⧉</button>
    {/if}
  </span>
{/snippet}

{#snippet trCell(seg: TranslateSegment)}
  {@const row = translate.rows[seg.id]}
  {#if seg.private}
    <span class="priv">{t("translate.privateSegment")}</span>
  {:else if seg.kind.type === "aside"}
    <span class="aside-mark" title={t("translate.asideHint")}>·</span>
  {:else if !seg.translate}
    {@html html(seg.prefix, seg.text)}
  {:else if row?.text != null}
    <!-- Streaming or done, the same renderer: a finished cell is the streamed one plus the
      tail that was held back while its syntax was still open -->
    {@html html(seg.prefix, row.done ? row.text : streamingTail(row.text))}
    {#if row.done}{@render rowTools(seg, true)}{/if}
  {:else if row?.error}
    <p class="failed" title={row.error}>{t("translate.untranslated")} · {row.error}</p>
    {@render rowTools(seg, false)}
  {:else if translate.pausedAt === seg.id}
    <!-- the background translation stopped here at its budget: the reader decides how much further -->
    <p class="paused">
      <span>{t("translate.pausedHere")}</span>
      <button onclick={(e) => { e.stopPropagation(); translate.continueBackground(false); }}>{t("translate.continue")}</button>
      <button onclick={(e) => { e.stopPropagation(); translate.continueBackground(true); }}>{t("translate.translateAll")}</button>
    </p>
  {:else}
    <!-- The room the translation will take, held from the start. Bars only on the rows asked
      for right now; a row not asked for yet (translated when it comes into view, or with
      "translate everything") keeps its room empty. Nothing here moves: the progress in the
      header is the one sign that the translation is running -->
    <div
      class="skel"
      class:asked={translate.requested[seg.id] && !translate.idle}
      style:--lines={estLines(seg)}
      title={translate.requested[seg.id] ? undefined : t("translate.notYet")}
    ></div>
  {/if}
{/snippet}

<div class="view">
  <ViewHeader
    title={doc ? doc.title : t("view.translate")}
    clipTitle
    left={status}
    right={tools}
    onclose={() => vault.exitTranslate()}
    closeHint={t("translate.leaveHint")}
  />

  {#if translate.loading}
    <div class="state"><span class="pulse"></span>{t("translate.waiting")}</div>
  {:else if translate.error}
    <div class="state error"><p>{translate.error}</p></div>
  {:else if doc}
    <div class="scroller" bind:this={root} {onscroll}>
      <div
        class="rows mode-{mode}"
        class:pdf={isPdf}
        style:--cols={layout.cols}
        style:--side-max={layout.max}
        data-script={rowsScript}
      >
        {#if mode === "side"}
          {@const head = `${t("translate.original")}${doc.source.kind === "note" ? ` · ${t("translate.sourceHint")}` : doc.source.kind === "pdf" ? ` · ${t("translate.pdfHint")}` : ""}`}
          <div class="cols">
            <div use:cliptip={head}>{head}</div>
            <div class="gut" aria-hidden="true"></div>
            <div>{t("translate.translation")}</div>
          </div>
        {/if}
        {#if doc.source.kind === "pdf"}
          {#each pageGroups as g (g.page)}
            {@const cells = cellsOf(g)}
            {@const ready = !!measured[g.page]}
            <div class="row page" use:hold use:nearPage={g} data-anchor={ready && cells.length ? undefined : `p${g.page}`}>
              <div class="src pdfpage">
                <div class="pagebox" bind:clientHeight={pageHeights[g.page]}>
                  {#if pdfDoc}<PdfPage doc={pdfDoc} n={g.page} {root} highlight={litOn(g)} />{/if}
                </div>
              </div>
              {#if mode === "side"}<div class="gut" aria-hidden="true"></div>{/if}
              <div class="tr md-body md-wide pagecells" {...trAttrs}>
                <div class="pagelabel">{t("translate.pageN", { n: g.page })}</div>
                {#if ready}
                  {#each cells as seg (seg.id)}
                    <!-- svelte-ignore a11y_no_static_element_interactions, a11y_click_events_have_key_events -->
                    <div
                      class="cell"
                      class:lit={translate.selected === seg.id}
                      style:margin-top={`${cellGaps[seg.id] ?? 0}px`}
                      bind:clientHeight={cellHeights[seg.id]}
                      use:observe={seg.id}
                      use:hold
                      use:images.fill
                      data-id={seg.id}
                      data-anchor={`s${seg.id}`}
                      onclick={(e) => onTrClick(e, seg.id)}
                    >
                      {@render trCell(seg)}
                    </div>
                  {/each}
                {/if}
              </div>
            </div>
          {/each}
        {:else}
          {#each doc.segments as seg (seg.id)}
            <!-- svelte-ignore a11y_no_static_element_interactions, a11y_click_events_have_key_events -->
            <div
              class="row kind-{seg.kind.type}"
              class:lit={translate.selected === seg.id}
              use:observe={seg.id}
              use:hold
              data-id={seg.id}
              data-anchor={`s${seg.id}`}
            >
              <div class="src md-body md-wide" {...srcAttrs} use:images.fill ondblclick={() => openSource(seg)} onclick={onCellClick}>
                {@html html(seg.prefix, seg.text)}
              </div>
              {#if mode === "side"}<div class="gut" aria-hidden="true"></div>{/if}
              <div class="tr md-body md-wide" {...trAttrs} use:images.fill onclick={(e) => onTrClick(e, seg.id)}>
                {@render trCell(seg)}
              </div>
            </div>
          {/each}
        {/if}
      </div>
    </div>
  {:else}
    <div class="state">{t("translate.noSource")} · {k("Mod+Shift+T")}</div>
  {/if}
</div>

<style>
  .view {
    height: 100%;
    display: flex;
    flex-direction: column;
    min-height: 0;
  }
  .status {
    font-size: var(--fs-xs);
    color: var(--fg-muted);
    font-variant-numeric: tabular-nums;
    white-space: nowrap;
  }
  .status.ok {
    color: var(--co-success);
  }
  .actions {
    display: flex;
    align-items: center;
    gap: 4px;
    flex-wrap: wrap;
  }
  .actions button,
  .actions .chip,
  .chip.warn {
    font-size: var(--fs-xs);
    padding: 3px 8px;
    border: 1px solid var(--border);
    border-radius: 6px;
    color: var(--fg-muted);
    background: transparent;
    white-space: nowrap;
  }
  .actions select.chip {
    appearance: none;
    -webkit-appearance: none;
    padding-right: 18px;
    background-image: linear-gradient(45deg, transparent 50%, var(--fg-faint) 50%), linear-gradient(135deg, var(--fg-faint) 50%, transparent 50%);
    background-position: calc(100% - 10px) 55%, calc(100% - 6px) 55%;
    background-size: 4px 4px, 4px 4px;
    background-repeat: no-repeat;
    cursor: pointer;
  }
  .actions button:hover,
  .actions select.chip:hover {
    background-color: var(--bg-hover);
    color: var(--fg);
  }
  .actions .model {
    font-family: var(--font-mono);
    max-width: 230px;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .chip.warn {
    color: var(--co-warn);
    border-color: var(--co-warn);
  }
  .actions .primary {
    color: var(--accent);
    border-color: var(--accent);
  }
  .state {
    padding: 40px 24px;
    font-size: var(--fs-sm);
    color: var(--fg-muted);
    display: flex;
    align-items: center;
    gap: 10px;
    justify-content: center;
  }
  .state.error {
    color: var(--danger);
  }
  .pulse {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    background: var(--accent);
    animation: pulse 1.2s ease-in-out infinite;
  }
  @keyframes pulse {
    0%, 100% { opacity: 0.3; }
    50% { opacity: 1; }
  }
  @media (prefers-reduced-motion: reduce) {
    .pulse { animation: none; }
  }

  /* Sideways as well: a pane narrower than two readable columns scrolls rather than
     switching the layout under the reader */
  .scroller {
    flex: 1;
    min-height: 0;
    overflow: auto;
    padding: 0 24px;
  }
  .rows {
    font-size: var(--fs-note);
    padding: 0 0 48px;
    margin: 0 auto;
  }
  .rows.mode-side {
    max-width: var(--content-width-off, var(--side-max));
    min-width: calc(40em + 44px);
  }
  /* One column: capped by its own text's writing system (data-script on .rows); the 36px on
     the left is where a row's tools sit */
  .rows.mode-stack,
  .rows.mode-target {
    max-width: calc(var(--content-width) + 36px);
    padding-left: 36px;
  }
  .cols,
  .row {
    display: grid;
    grid-template-columns: var(--cols);
  }
  .mode-stack .row,
  .mode-target .row {
    grid-template-columns: minmax(0, 1fr);
  }
  .cols {
    position: sticky;
    top: 0;
    z-index: 1;
    font-size: var(--fs-xs);
    color: var(--fg-muted);
    background: var(--bg);
    border-bottom: 1px solid var(--border);
    margin-bottom: 0.45em;
  }
  .cols > div:not(.gut) {
    padding: 6px 0.75em;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  /* The one line between the two columns: drawn per row down the gutter's middle, so the
     rows (which abut; their spacing is inside them) join it into one rule */
  .gut {
    background: linear-gradient(var(--border), var(--border)) no-repeat center / 1px 100%;
  }
  /* Cells space the rows with their own padding (0.45em + 0.45em = the shared 0.9em
     paragraph gap), not with a line under every row */
  .row > .src,
  .row > .tr {
    padding: 0.45em 0.75em;
    min-width: 0;
    position: relative;
    /* selectable and hyphenated by the cell's own language through .md-body (app.css [lang]
       sets --hyphens for German, French, Spanish): two columns are narrow, and a long
       compound would otherwise leave a gap */
  }
  /* The source is the second voice beside the translation: secondary ink, which holds
     ≥ 4.5:1 on the lit row's ground too */
  .row .src {
    color: var(--fg-muted);
  }
  .mode-stack .row .src {
    padding-bottom: 0.15em;
  }
  .mode-stack .row .tr {
    padding-top: 0.15em;
  }
  .mode-target .row:not(.kind-aside) .src,
  .mode-target .row.kind-aside .tr,
  .mode-stack .row.kind-aside .tr {
    display: none;
  }
  /* A caption, a reference, a byline: left as it is, one size down, in the same secondary
     ink — shown in every layout, translation-only included */
  .kind-aside .src {
    font-size: 0.875em;
  }
  .aside-mark {
    color: var(--fg-faint);
  }
  /* The row the reader picked: lit on both sides at once */
  .row.lit > .src,
  .row.lit > .tr,
  .cell.lit {
    background: var(--bg-hover);
    box-shadow: inset 3px 0 0 var(--accent);
  }
  /* A PDF row: the page drawn on the left, its paragraphs as cells on the right, each at its
     source's height (the page label row is the same height as the page cell's top padding) */
  .row.page .pdfpage {
    padding: 0 0 14px;
    align-self: start;
  }
  .mode-side .row.page .pdfpage {
    padding-top: 26px;
  }
  .mode-stack .row.page .pdfpage,
  .mode-target .row.page .pdfpage {
    padding-top: 14px;
  }
  .row.page .pagecells {
    padding: 0 0 12px;
  }
  .mode-side .pagelabel {
    height: 26px;
    box-sizing: border-box;
    padding-top: 6px;
  }
  .pagelabel {
    font-size: var(--fs-xs);
    color: var(--fg-muted);
    padding: 2px 0.75em 6px;
  }
  .cell {
    position: relative;
    padding: 0.45em 0.75em;
    border-radius: 4px;
  }
  /* A row's tools stand in the gutter to the left of the translation, clear of its text */
  .tools {
    position: absolute;
    top: 0.45em;
    right: calc(100% + 11px);
    display: flex;
    flex-direction: column;
    gap: 2px;
    opacity: 0;
    transition: opacity 120ms;
    -webkit-user-select: none;
    user-select: none;
  }
  .row .tr:hover .tools,
  .row.lit .tools,
  .cell:hover .tools,
  .cell.lit .tools,
  .tools:focus-within {
    opacity: 1;
  }
  .tools button {
    font-size: var(--fs-xs);
    width: 22px;
    height: 22px;
    display: grid;
    place-items: center;
    border: 1px solid var(--border);
    border-radius: 5px;
    background: var(--bg-panel);
    color: var(--fg-muted);
  }
  .tools button:hover {
    color: var(--fg);
    background: var(--bg-hover);
  }
  .priv {
    font-size: var(--fs-sm);
    color: var(--fg-muted);
  }
  .failed {
    margin: 0;
    font-size: var(--fs-sm);
    color: var(--co-warn);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  /* The translation's room: --lines lines of the body's line height. Asked for, the lines
     are drawn as still bars; not asked for, the room stays blank */
  .skel {
    height: calc(var(--lines) * 1.75em);
  }
  .skel.asked {
    background: repeating-linear-gradient(to bottom, transparent 0 0.5em, var(--bg-hover) 0.5em 1.25em, transparent 1.25em 1.75em);
    border-radius: 4px;
  }
  .paused {
    margin: 0;
    display: flex;
    align-items: center;
    gap: 8px;
    font-size: var(--fs-sm);
    color: var(--fg-muted);
  }
  .paused button {
    font-size: var(--fs-xs);
    padding: 2px 8px;
    border: 1px solid var(--accent);
    border-radius: 6px;
    color: var(--accent);
  }
  .paused button:hover {
    background: var(--bg-hover);
  }
  /* The note's own images: the width stays inside the column; the height follows the width
     and, once an image has been seen, the width/height attributes (views/assets.ts) */
  .row :global(img[data-asset]) {
    max-width: 100%;
    height: auto;
  }
</style>
