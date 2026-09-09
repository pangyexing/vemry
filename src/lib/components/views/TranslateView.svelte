<script lang="ts">
  // Side-by-side translation: the source on the left, its translation on the right,
  // one grid row per segment — the row takes the taller side's height, so the two
  // columns stay aligned by construction and nothing has to synchronise scrolling.
  // Rows fill in as the model streams; the rows on screen are asked for first.
  // The store holds the document and the translations; this only renders and
  // reports what is visible. Leaving goes through vault.exitTranslate (Esc, ✕),
  // which is what closes the job.
  import { t, tn } from "$lib/i18n/i18n.svelte";
  import { api, type TranslateSegment } from "$lib/api";
  import { translate, targetLabel, fmtTokens, TARGET_TAGS, type TranslateMode } from "$lib/stores/translate.svelte";
  import { fileKind, mimeOf, vault } from "$lib/stores/vault.svelte";
  import { toast } from "$lib/stores/toast.svelte";
  import { executeCommand } from "$lib/commands/registry";
  import { handleCodeCopyClick, renderNoteMarkdown } from "$lib/markdown";
  import { k } from "$lib/shortcut";
  import { loadPdf, segmentTops, type PDFDocumentProxy } from "$lib/pdf";
  import PdfPage from "$lib/components/PdfPage.svelte";
  import "katex/dist/katex.min.css";

  /** Below this width two columns are too narrow to read, and the rows interleave; a PDF's
   *  page needs more room than a paragraph of text to be legible. */
  const NARROW_PX = 880;
  const NARROW_PDF_PX = 1000;
  /** Visibility reports are batched: a scroll fires many intersections in a row. */
  const REPORT_MS = 150;

  const doc = $derived(translate.doc);
  let root = $state<HTMLDivElement | null>(null);
  let width = $state(0);
  const isPdf = $derived(doc?.source.kind === "pdf");
  const narrow = $derived(width > 0 && width < (isPdf ? NARROW_PDF_PX : NARROW_PX));
  /** The layout in effect: the chosen one, unless the pane is too narrow for two columns. */
  const mode = $derived<TranslateMode>(translate.mode === "side" && narrow ? "stack" : translate.mode);
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
  /** Where each PDF segment starts on its page (a fraction of the page height), by segment id; looked up per page as the document arrives. */
  let tops = $state<Record<number, number>>({});
  $effect(() => {
    const d = pdfDoc;
    const groups = pageGroups;
    if (!d || !groups.length) { tops = {}; return; }
    let stale = false;
    void (async () => {
      const next: Record<number, number> = {};
      for (const g of groups) {
        const segs = g.segs.filter((s) => !s.private);
        if (!segs.length) continue;
        const page = await d.getPage(g.page);
        const found = await segmentTops(page, segs.map((s) => s.text));
        if (stale) return;
        segs.forEach((s, i) => { const f = found[i]; if (f != null) next[s.id] = f; });
      }
      if (!stale) tops = next;
    })();
    return () => { stale = true; };
  });
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
      for (const s of g.segs) {
        if (s.private || s.kind.type === "aside") continue;
        const want = tops[s.id] != null ? tops[s.id] * ph : bottom;
        const gap = Math.max(0, want - bottom);
        gaps[s.id] = gap;
        bottom += gap + (cellHeights[s.id] ?? 0) + 6;
      }
    }
    return gaps;
  });
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

  // ---- rendering ----
  /** One segment's HTML from prefix + body: the note renderer (wikilinks, callouts, KaTeX, local images as data-asset). */
  function html(prefix: string, body: string): string {
    return renderNoteMarkdown(prefix + body);
  }

  // The note's own images: <img data-asset> filled from the vault, one blob URL per path for the view's life
  const assetUrls = new Map<string, Promise<string | null>>();
  function loadAsset(target: string): Promise<string | null> {
    // As written, when it names a path; a bare file name has nothing to resolve against here
    if (!target.includes("/") || fileKind(target) !== "image") return Promise.resolve(null);
    let p = assetUrls.get(target);
    if (!p) {
      p = api
        .fileReadBinary(target)
        .then((buf) => URL.createObjectURL(new Blob([buf], { type: mimeOf(target) })))
        .catch(() => {
          // one transient read failure should not nail this image as "missing" for the view's life
          assetUrls.delete(target);
          return null;
        });
      assetUrls.set(target, p);
    }
    return p;
  }
  /** Fill the images of a rendered cell after each render. */
  function assets(node: HTMLElement) {
    const fill = () => {
      for (const img of node.querySelectorAll<HTMLImageElement>("img[data-asset]:not([src])")) {
        const target = img.dataset.asset ?? "";
        void loadAsset(target).then((u) => {
          if (u && img.isConnected) img.src = u;
        });
      }
    };
    fill();
    const mo = new MutationObserver(fill);
    mo.observe(node, { childList: true, subtree: true });
    return { destroy: () => mo.disconnect() };
  }
  $effect(() => () => {
    for (const p of assetUrls.values()) void p.then((u) => u && URL.revokeObjectURL(u));
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
  /** The rows on screen right now, so a rebuilt observer picks them all up again. */
  const observed = new Set<HTMLElement>();
  function observe(node: HTMLElement, id: number) {
    observed.add(node);
    io?.observe(node);
    return { destroy: () => { observed.delete(node); io?.unobserve(node); visible.delete(id); } };
  }
  // The observer measures against the scroller, so it is built here rather than in the
  // action: reopening the document (a changed target language) mounts a new scroller,
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
    return () => { io?.disconnect(); io = null; };
  });
  $effect(() => () => { if (reportTimer) clearTimeout(reportTimer); });

  $effect(() => {
    if (!root) return;
    const ro = new ResizeObserver(([e]) => (width = e.contentRect.width));
    ro.observe(root);
    return () => ro.disconnect();
  });

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

<div class="view">
  <header>
    {#if doc}
      <span class="title" title={doc.source.kind === "page" ? doc.source.url : doc.source.path}>{doc.title}</span>
      <span class="actions">
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
        {#if failed > 0}
          <button class="chip warn" title={t("translate.retryFailed")} onclick={() => translate.retryFailed()}>
            {tn("translate.failedCount", failed)} · {t("translate.retryFailed")}
          </button>
        {:else if done >= total}
          <span class="chip ok">{t("translate.complete")}</span>
        {:else}
          <span class="chip" title={t("translate.progressHint")}>{t("translate.progress", { done, total })}</span>
        {/if}
        <button title={t("translate.copyAll")} onclick={() => void copyAll()}>{t("translate.copy")}</button>
        <button class="primary" title={t("translate.saveHint")} disabled={translate.saving} onclick={() => void save()}
          >{translate.saving && remaining > 0 ? t("translate.savingRemaining", { n: remaining }) : t("translate.save")}</button
        >
      </span>
    {:else}
      <span class="title faint">{t("view.translate")}</span>
    {/if}
    <button class="close" title={t("translate.leaveHint")} aria-label={t("translate.leave")} onclick={() => vault.exitTranslate()}>✕</button>
  </header>

  {#if translate.loading}
    <div class="state"><span class="pulse"></span>{t("translate.waiting")}</div>
  {:else if translate.error}
    <div class="state error"><p>{translate.error}</p></div>
  {:else if doc}
    <div class="scroller" bind:this={root}>
      <div class="rows mode-{mode}" class:pdf={isPdf}>
        {#if mode === "side"}
          <div class="cols">
            <div>{t("translate.original")}{#if doc.source.kind === "note"} <span class="hint">· {t("translate.sourceHint")}</span>{:else if doc.source.kind === "pdf"} <span class="hint">· {t("translate.pdfHint")}</span>{/if}</div>
            <div>{t("translate.translation")}</div>
          </div>
        {/if}
        {#snippet trCell(seg: TranslateSegment)}
          {@const row = translate.rows[seg.id]}
          {#if seg.private}
            <span class="priv">{t("translate.privateSegment")}</span>
          {:else if seg.kind.type === "aside"}
            <span class="aside-mark" title={t("translate.asideHint")}>·</span>
          {:else if !seg.translate}
            {@html html(seg.prefix, seg.text)}
          {:else if row?.done && row.text != null}
            {@html html(seg.prefix, row.text)}
            <span class="tools">
              <button title={t("translate.retranslate")} onclick={(e) => { e.stopPropagation(); translate.request([seg.id], true); }}>↻</button>
              <button title={t("translate.copy")} onclick={(e) => { e.stopPropagation(); void copyRow(seg); }}>⧉</button>
            </span>
          {:else if row?.text != null}
            <p class="partial">{seg.prefix}{row.text}<span class="cur"></span></p>
          {:else if row?.error}
            <p class="failed" title={row.error}>{t("translate.untranslated")} · {row.error}</p>
            <span class="tools">
              <button title={t("translate.retranslate")} onclick={(e) => { e.stopPropagation(); translate.request([seg.id], true); }}>↻</button>
            </span>
          {:else if translate.pausedAt === seg.id}
            <!-- the background translation stopped here at its budget: the reader decides how much further -->
            <p class="paused">
              <span>{t("translate.pausedHere")}</span>
              <button onclick={(e) => { e.stopPropagation(); translate.continueBackground(false); }}>{t("translate.continue")}</button>
              <button onclick={(e) => { e.stopPropagation(); translate.continueBackground(true); }}>{t("translate.translateAll")}</button>
            </p>
          {:else if translate.requested[seg.id] || !translate.idle}
            <div class="skel"><i></i><i></i><i class="short"></i></div>
          {:else}
            <!-- not asked for yet: translated when it comes into view, or with "translate everything" -->
            <div class="skel still" title={t("translate.notYet")}><i></i><i class="short"></i></div>
          {/if}
        {/snippet}
        {#if doc.source.kind === "pdf"}
          {#each pageGroups as g (g.page)}
            <div class="row page">
              <div class="src pdfpage">
                <div class="pagebox" bind:clientHeight={pageHeights[g.page]}>
                  {#if pdfDoc}<PdfPage doc={pdfDoc} n={g.page} {root} highlight={litOn(g)} />{/if}
                </div>
              </div>
              <div class="tr md-body pagecells">
                <div class="pagelabel">{t("translate.pageN", { n: g.page })}</div>
                {#each g.segs.filter((s) => s.kind.type !== "aside" && !s.private) as seg (seg.id)}
                  <!-- svelte-ignore a11y_no_static_element_interactions, a11y_click_events_have_key_events -->
                  <div
                    class="cell"
                    class:lit={translate.selected === seg.id}
                    style:margin-top={`${cellGaps[seg.id] ?? 0}px`}
                    bind:clientHeight={cellHeights[seg.id]}
                    use:observe={seg.id}
                    data-id={seg.id}
                    onclick={(e) => { onCellClick(e); select(seg.id); }}
                  >
                    {@render trCell(seg)}
                  </div>
                {/each}
              </div>
            </div>
          {/each}
        {:else}
          {#each doc.segments as seg (seg.id)}
            <!-- svelte-ignore a11y_no_static_element_interactions, a11y_click_events_have_key_events -->
            <div class="row kind-{seg.kind.type}" class:lit={translate.selected === seg.id} use:observe={seg.id} data-id={seg.id}>
              <div class="src md-body" use:assets ondblclick={() => openSource(seg)} onclick={onCellClick}>
                {@html html(seg.prefix, seg.text)}
              </div>
              <div class="tr md-body" use:assets onclick={(e) => { onCellClick(e); select(seg.id); }}>
                {@render trCell(seg)}
              </div>
            </div>
          {/each}
        {/if}
      </div>
    </div>
  {:else}
    <div class="state faint">{t("translate.noSource")} · {k("Mod+Shift+T")}</div>
  {/if}
</div>

<style>
  .view {
    height: 100%;
    display: flex;
    flex-direction: column;
    min-height: 0;
  }
  header {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 8px 14px;
    border-bottom: 1px solid var(--border);
    flex-shrink: 0;
    min-width: 0;
  }
  .title {
    font-size: var(--fs-sm);
    font-weight: 600;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    min-width: 0;
  }
  .title.faint {
    color: var(--fg-faint);
    font-weight: 400;
  }
  .actions {
    margin-left: auto;
    display: flex;
    align-items: center;
    gap: 4px;
    flex-shrink: 0;
  }
  .actions button,
  .actions .chip {
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
  .actions .ok {
    color: var(--co-success);
    border-color: transparent;
  }
  .actions .warn {
    color: var(--co-warn);
    border-color: var(--co-warn);
  }
  .actions .primary {
    color: var(--accent);
    border-color: var(--accent);
  }
  .close {
    flex-shrink: 0;
    min-width: 24px;
    height: 24px;
    display: grid;
    place-items: center;
    border-radius: 5px;
    color: var(--fg-muted);
  }
  .close:hover {
    background: var(--bg-hover);
    color: var(--fg);
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
  .state.faint {
    color: var(--fg-faint);
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

  .scroller {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
  }
  .rows {
    font-size: var(--fs-note);
    line-height: 1.7;
    padding: 0 0 48px;
  }
  .rows.mode-stack,
  .rows.mode-target {
    max-width: var(--content-width);
    margin: 0 auto;
  }
  .cols {
    position: sticky;
    top: 0;
    z-index: 1;
    display: grid;
    grid-template-columns: 1fr 1fr;
    font-size: var(--fs-xs);
    letter-spacing: 0.04em;
    text-transform: uppercase;
    color: var(--fg-faint);
    background: var(--bg);
    border-bottom: 1px solid var(--border);
  }
  .cols > div {
    padding: 5px 20px;
  }
  .cols > div + div {
    border-left: 1px solid var(--border);
  }
  .cols .hint {
    text-transform: none;
    letter-spacing: 0;
  }
  .row {
    display: grid;
    grid-template-columns: 1fr 1fr;
    border-bottom: 1px solid var(--border);
  }
  /* A page wants more width than its translation: the page is small type, the translation is not */
  .rows.pdf .row,
  .rows.pdf .cols {
    grid-template-columns: 3fr 2fr;
  }
  .mode-stack .row,
  .mode-target .row {
    grid-template-columns: 1fr;
  }
  .row > div {
    padding: 10px 20px;
    min-width: 0;
    position: relative;
  }
  .row .src {
    color: var(--fg-muted);
  }
  .mode-side .row .tr {
    border-left: 1px solid var(--border);
  }
  .mode-stack .row .src {
    padding-bottom: 2px;
  }
  .mode-stack .row .tr {
    padding-top: 2px;
  }
  .mode-target .row .src {
    display: none;
  }
  /* The row the reader picked: lit on both sides at once */
  .row.lit > div {
    background: var(--bg-hover);
    box-shadow: inset 3px 0 0 var(--accent);
  }
  .row:not(.page) .tr:hover .tools,
  .cell:hover .tools {
    opacity: 1;
  }
  /* A PDF row: the page drawn on the left, its paragraphs as cells on the right, each at its
     source's height (the page label row is the same height as the page cell's top padding) */
  .row.page .pdfpage {
    padding: 0 20px 14px;
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
  .kind-aside .src {
    opacity: 0.55;
  }
  .aside-mark {
    color: var(--fg-faint);
  }
  .pagelabel {
    font-size: var(--fs-xs);
    letter-spacing: 0.04em;
    color: var(--fg-faint);
    padding: 2px 20px 6px;
  }
  .cell {
    position: relative;
    padding: 6px 20px;
    border-radius: 4px;
  }
  .cell.lit {
    background: var(--bg-hover);
    box-shadow: inset 3px 0 0 var(--accent);
  }
  .tools {
    position: absolute;
    top: 6px;
    right: 8px;
    display: flex;
    gap: 2px;
    opacity: 0;
    transition: opacity 120ms;
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
  .partial {
    margin: 0;
    white-space: pre-wrap;
  }
  .cur {
    display: inline-block;
    width: 2px;
    height: 1em;
    background: var(--accent);
    vertical-align: -0.15em;
    margin-left: 2px;
    animation: pulse 1s steps(2) infinite;
  }
  .priv {
    font-size: var(--fs-sm);
    color: var(--fg-faint);
    font-style: italic;
  }
  .failed {
    margin: 0;
    font-size: var(--fs-sm);
    color: var(--co-warn);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .skel {
    display: flex;
    flex-direction: column;
    gap: 8px;
    padding: 6px 0;
  }
  .skel i {
    display: block;
    height: 0.75em;
    border-radius: 4px;
    background: linear-gradient(90deg, var(--bg-panel), var(--border), var(--bg-panel));
    background-size: 200% 100%;
    animation: shimmer 1.4s linear infinite;
  }
  .skel i.short {
    width: 55%;
  }
  .skel.still i {
    animation: none;
    background: var(--bg-panel);
    opacity: 0.7;
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
  @keyframes shimmer {
    from { background-position: 200% 0; }
    to { background-position: -200% 0; }
  }
  @media (prefers-reduced-motion: reduce) {
    .skel i, .cur, .pulse { animation: none; }
  }

  /* ---- the cell typography (the note renderer's tags; the same rhythm as the reading projection) ---- */
  .row :global(p) { margin: 0 0 0.6em; }
  .row :global(p:last-child) { margin-bottom: 0; }
  .row :global(.md-h) { font-weight: 600; line-height: 1.4; margin: 0.2em 0; }
  .row :global(.md-h1) { font-size: 1.6em; }
  .row :global(.md-h2) { font-size: 1.42em; }
  .row :global(.md-h3) { font-size: 1.27em; }
  .row :global(.md-h4) { font-size: 1.12em; }
  .row :global(.md-h5), .row :global(.md-h6) { font-size: 1em; }
  .row :global(ul), .row :global(ol) { margin: 0; padding-left: 1.6em; }
  .row :global(li) { margin: 0.1em 0; }
  .row :global(blockquote) { margin: 0; padding: 2px 0 2px 12px; border-left: 3px solid var(--border); color: var(--fg-muted); }
  .row :global(code) { font-family: var(--font-mono); font-size: 0.88em; background: var(--bg-panel); border-radius: 4px; padding: 1px 5px; }
  .row :global(.md-code) { margin: 0; padding: 10px 12px; background: var(--bg-panel); border: 1px solid var(--border); border-radius: var(--radius); overflow-x: auto; font-size: 0.88em; line-height: 1.6; }
  .row :global(.md-code code) { background: none; padding: 0; }
  .row :global(.md-hr) { border: none; border-top: 1px solid var(--border); margin: 0.4em 0; }
  .row :global(.md-link), .row :global(.md-wiki) { color: var(--accent); text-decoration: underline; text-underline-offset: 2px; cursor: pointer; }
  .row :global(img.md-img) { max-width: 100%; height: auto; border-radius: 6px; }
  .row :global(.katex-display) { overflow-x: auto; overflow-y: hidden; margin: 0.2em 0; }
  .row :global(.md-tablewrap) { margin: 0; }
  .row :global(th), .row :global(td) { padding: 4px 8px; }
</style>
