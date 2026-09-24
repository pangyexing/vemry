<script lang="ts">
  import { api, type NoteLinks } from "$lib/api";
  import { t } from "$lib/i18n/i18n.svelte";
  import { plainText } from "$lib/markdown";
  import { vault } from "$lib/stores/vault.svelte";
  import { contextMenu } from "$lib/stores/contextMenu.svelte";
  import { cliptip } from "$lib/cliptip";
  import Chevron from "./Chevron.svelte";

  let links = $state<NoteLinks | null>(null);

  function backlinkMenu(e: MouseEvent, source: string, title: string) {
    contextMenu.open(e, [
      { title },
      { label: t("outline.openNote"), action: () => void vault.openNote(source) },
      { label: t("tabBar.revealInTree"), action: async () => { await vault.openNote(source); vault.revealInTree(); } },
    ]);
  }
  function outgoingMenu(e: MouseEvent, target: string, resolved: boolean) {
    // followLink has create semantics for an unresolved target, so the menu's wording says so
    contextMenu.open(e, [
      { title: target },
      { label: resolved ? t("outline.openNote") : t("outline.createAndOpenNote"), action: () => void vault.followLink(target) },
      ...(resolved
        ? [{ label: t("tabBar.revealInTree"), action: async () => { await vault.followLink(target); vault.revealInTree(); } }]
        : []),
    ]);
  }

  $effect(() => {
    const path = vault.currentPath;
    const _ = vault.reloadCounter;
    // The echo of our own save is swallowed by handleEvent's content comparison and no
    // longer bumps reloadCounter, so a newly written [[link]] reaches the panel through
    // indexVersion (incremented on every vault event)
    void vault.indexVersion;
    if (!path || !path.endsWith(".md")) {
      links = null;
      return;
    }
    api
      .linksGet(path)
      .then((l) => (links = l))
      .catch(() => (links = null));
  });

  /** The indentation baseline is the document's minimum level: a document starting at ## does not hang a level short. */
  const minLevel = $derived(vault.outline?.length ? Math.min(...vault.outline.map((o) => o.level)) : 1);
  /** The section the cursor is in = the last heading whose from ≤ the cursor. */
  const activeIdx = $derived.by(() => {
    const o = vault.outline ?? [];
    let idx = -1;
    for (let i = 0; i < o.length && o[i].from <= vault.editorCursor; i++) idx = i;
    return idx;
  });

  /** Under a projection view (agenda, the reading projection and so on) it returns to the
   *  editor before scrolling: the contract is collected in vault.revealInEditor (shared
   *  with "land back after closing a browser window"). */
  const jump = (from: number) => void vault.revealInEditor(from);

  /** The headings are the note's own words: they are set in its language (the index reads
   *  it), so a Japanese note's outline takes Japanese glyph forms under any interface. */
  const noteLang = $derived(vault.notes.find((n) => n.id === vault.currentPath)?.lang ?? undefined);

  let panelEl = $state<HTMLElement | null>(null);
  // The current section stays in sight: when the reader's cursor moves into a section whose
  // entry lies outside the panel's viewport, the list scrolls by the nearest distance that
  // brings it back — and not at all otherwise, so the list holds still while the section
  // is visible
  $effect(() => {
    const i = activeIdx;
    const panel = panelEl;
    if (i < 0 || !panel) return;
    const el = panel.querySelector<HTMLElement>(`[data-outline="${i}"]`);
    if (!el) return;
    const p = panel.getBoundingClientRect();
    const r = el.getBoundingClientRect();
    // a hidden panel (the research tab in front) measures zero: nothing to follow
    if (p.height === 0) return;
    if (r.top < p.top) panel.scrollTop -= p.top - r.top;
    else if (r.bottom > p.bottom) panel.scrollTop += r.bottom - p.bottom;
  });
</script>

{#snippet chevron(open: boolean)}
  <span class="chevron" class:open><Chevron /></span>
{/snippet}

<div class="panel" bind:this={panelEl}>
  {#if !vault.currentIsNote}
    <p class="hint">{t("outline.emptyHint")}</p>
  {:else}
    <section lang={noteLang}>
      {#each vault.outline ?? [] as item, i (i)}
        {@const label = plainText(item.text) || t("outline.untitled")}
        <!-- Only the display is folded: App's landing point after closing a window matches against the outline's original text -->
        <button
          class="outline-entry"
          class:active={i === activeIdx}
          class:top={item.level === minLevel}
          data-outline={i}
          style:padding-left="{(item.level - minLevel) * 12 + 8}px"
          onclick={() => void jump(item.from)}
          use:cliptip={label}
        >
          {label}
        </button>
      {:else}
        <p class="hint">{t("outline.noHeadings")}</p>
      {/each}
    </section>
    {#if links}
      <section>
        <button class="sec-head" onclick={() => (vault.backlinksOpen = !vault.backlinksOpen)}>
          {@render chevron(vault.backlinksOpen)}
          {t("outline.backlinks")} <span class="count">{links.backlinks.length}</span>
        </button>
        {#if vault.backlinksOpen}
          {#each links.backlinks as bl (bl.source)}
            <button class="entry" onclick={() => vault.openNote(bl.source)} oncontextmenu={(e) => backlinkMenu(e, bl.source, bl.source_title)}>
              <span class="entry-title">{bl.source_title}</span>
              {#if bl.snippet}<span class="snippet">{bl.snippet}</span>{/if}
            </button>
          {:else}
            <p class="hint">{t("outline.noBacklinks")}</p>
          {/each}
        {/if}
      </section>
      <section>
        <button class="sec-head" onclick={() => (vault.outgoingOpen = !vault.outgoingOpen)}>
          {@render chevron(vault.outgoingOpen)}
          {t("outline.outgoing")} <span class="count">{links.outgoing.length}</span>
        </button>
        {#if vault.outgoingOpen}
          {#each links.outgoing as out, i (i)}
            <button
              class="entry"
              class:unresolved={!out.resolved}
              onclick={() => vault.followLink(out.raw_target)}
              oncontextmenu={(e) => outgoingMenu(e, out.raw_target, !!out.resolved)}
            >
              <span class="entry-title">{out.raw_target}</span>
              {#if !out.resolved}<span class="badge">{t("outline.notCreated")}</span>{/if}
            </button>
          {:else}
            <p class="hint">{t("outline.noLinks")}</p>
          {/each}
        {/if}
      </section>
    {/if}
  {/if}
</div>

<style>
  .panel {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    padding: 12px;
  }
  section {
    margin-bottom: 20px;
  }
  .sec-head {
    display: flex;
    align-items: center;
    gap: 6px;
    width: 100%;
    text-align: left;
    font-size: var(--fs-xs);
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.06em;
    color: var(--fg-muted);
    margin-bottom: 8px;
    padding: 2px 0;
  }
  /* Han, kana and hangul have no capitals, and tracking only pulls the characters apart */
  .sec-head:is(:lang(zh), :lang(ja), :lang(ko)) {
    text-transform: none;
    letter-spacing: normal;
  }
  .sec-head:hover {
    color: var(--fg);
  }
  .chevron {
    display: inline-flex;
    color: var(--fg-faint);
    transition: transform 0.12s;
  }
  .chevron.open {
    transform: rotate(90deg);
  }
  .count {
    font-weight: 400;
  }
  /* 26px entries (an 18px line box and 4px above and below), the file tree's row height.
     The headings are the note's words and take the body ink; the top level the note uses is
     one step heavier, the rest are told apart by their indent */
  .outline-entry {
    display: block;
    width: 100%;
    text-align: left;
    padding: 4px 8px;
    border-radius: 4px;
    font-size: var(--fs-sm);
    line-height: 18px;
    color: var(--fg);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .outline-entry.top {
    font-weight: 500;
  }
  .outline-entry:hover {
    background: var(--bg-hover);
  }
  /* The section the cursor is in: "current" drawn the one way it is drawn across the app —
     the active ground plus a 2px accent bar at the start of the row */
  .outline-entry.active {
    background: var(--bg-active);
    border-radius: 0 4px 4px 0;
    box-shadow: inset 2px 0 0 var(--accent);
  }
  .entry {
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    gap: 2px;
    width: 100%;
    text-align: left;
    padding: 6px 8px;
    border-radius: 4px;
    margin-bottom: 2px;
  }
  .entry:hover {
    background: var(--bg-hover);
  }
  .entry-title {
    font-size: var(--fs-sm);
    color: var(--accent);
  }
  .entry.unresolved .entry-title {
    color: var(--fg-muted);
  }
  .snippet {
    font-size: var(--fs-xs);
    color: var(--fg-muted);
    line-height: 1.5;
    display: -webkit-box;
    -webkit-line-clamp: 2;
    line-clamp: 2;
    -webkit-box-orient: vertical;
    overflow: hidden;
  }
  .badge {
    font-size: 11px;
    color: var(--fg-muted);
    border: 1px solid var(--border);
    border-radius: 3px;
    padding: 0 4px;
  }
  .hint {
    font-size: var(--fs-xs);
    color: var(--fg-muted);
    padding: 4px 8px;
  }
</style>
