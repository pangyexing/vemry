<script lang="ts">
  import { api, type NoteLinks } from "$lib/api";
  import { t } from "$lib/i18n/i18n.svelte";
  import { plainText } from "$lib/markdown";
  import { vault } from "$lib/stores/vault.svelte";
  import { contextMenu } from "$lib/stores/contextMenu.svelte";
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
</script>

{#snippet chevron(open: boolean)}
  <span class="chevron" class:open><Chevron /></span>
{/snippet}

<div class="panel">
  {#if !vault.currentIsNote}
    <p class="hint">{t("outline.emptyHint")}</p>
  {:else}
    <section>
      {#each vault.outline ?? [] as item, i (i)}
        <button
          class="outline-entry"
          class:active={i === activeIdx}
          style:padding-left="{(item.level - minLevel) * 12 + 8}px"
          onclick={() => void jump(item.from)}
        >
          <!-- Only the display is folded: App's landing point after closing a window matches against the outline's original text -->
          {plainText(item.text) || t("outline.untitled")}
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
    color: var(--fg-faint);
    margin-bottom: 8px;
    padding: 2px 0;
  }
  .sec-head:hover {
    color: var(--fg-muted);
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
  .outline-entry {
    display: block;
    width: 100%;
    text-align: left;
    padding: 4px 8px;
    border-radius: 4px;
    margin-bottom: 1px;
    font-size: var(--fs-sm);
    color: var(--fg-muted);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .outline-entry:hover {
    background: var(--bg-hover);
    color: var(--fg);
  }
  .outline-entry.active {
    color: var(--accent);
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
    color: var(--fg-faint);
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
    font-size: 10px;
    color: var(--fg-faint);
    border: 1px solid var(--border);
    border-radius: 3px;
    padding: 0 4px;
  }
  .hint {
    font-size: var(--fs-xs);
    color: var(--fg-faint);
    padding: 4px 8px;
  }
</style>
