<script module lang="ts">
  import { t, tn } from "$lib/i18n/i18n.svelte";

  /** The one way to write the count line. While a filter is in effect it gives "hits of
   *  total", so how much was filtered out is visible as a number — showing only the hits
   *  would make a typo look as though the vault held that little.
   *  `baseKey` is a `_one`/`_other` catalog pair: English needs the plural,
   *  and the two numbers read differently per language ("12 of 37 tasks"). */
  export function countLabel(shown: number, all: number, baseKey: string): string {
    const total = tn(baseKey, all);
    return shown === all ? total : t("viewChrome.countFiltered", { shown, total });
  }
</script>

<script lang="ts">
  /** The one shape of a projection view's header. All seven views used to write the same
   *  `header/h2/.total` rules, which had already drifted apart in two places (align-items
   *  was baseline in some and center in others, and the filter box was 200px in one and
   *  220px in another) — while they are sibling views at the same level, and the same kind
   *  of information should land on the same x.
   *
   *  The order in the row is fixed as: title · count · left slot — spring — right slot ·
   *  hint · filter box · ✕.
   *  The spring is a real element rather than a margin-left:auto on some member — that
   *  would require "the first member on the right" to change identity depending on whether
   *  a view has a filter box or a hint, and the `ViewClose push` switch grew out of exactly
   *  that. With a spring, no member on the right has to know its own position.
   *
   *  Whether there is a filter box is decided by filterPlaceholder: filtering requires a
   *  sentence saying what is filtered, so the two are tied together and one boolean switch
   *  is saved. */
  import type { Snippet } from "svelte";
  import { vault } from "$lib/stores/vault.svelte";
  import { cliptip } from "$lib/cliptip";

  let {
    title,
    total,
    hint,
    filterPlaceholder,
    filter = $bindable(""),
    left,
    right,
    askAi,
    lead,
    clipTitle = false,
    onclose,
    closeHint,
  }: {
    title: string;
    /** The count line, "37 tasks" and so on — each view decides what it counts (the
     *  reading surfaces put their secondary line here: the page's domain, the progress) */
    total?: string;
    /** A grey sentence on the right (an interaction hint or the view's promise) */
    hint?: string;
    /** Given, it renders the filter box and doubles as its placeholder */
    filterPlaceholder?: string;
    /** Given, an "Ask AI" button hands what the view shows to the composer as context */
    askAi?: () => void;
    filter?: string;
    /** The left slot: on the same side as the count, for a legend and the like — things read
     *  along with the title (a legend may also be a filter, as Agenda's is: it then sits by
     *  the count it narrows) */
    left?: Snippet;
    /** The right slot: for tools such as switches */
    right?: Snippet;
    /** Before the title: the reading projection's back / forward pair */
    lead?: Snippet;
    /** A title that is content rather than a view's name (a page's or a document's title,
     *  any length): it grows up to its own width, gives way down to a few characters before
     *  the tools wrap, and shows the rest in a clip tip */
    clipTitle?: boolean;
    /** A surface that is left rather than closed (the reading projection and the
     *  translation sit over a tab rather than being one): what ✕ does, and its hover line */
    onclose?: () => void;
    closeHint?: string;
  } = $props();
</script>

<header>
  {@render lead?.()}
  {#if clipTitle}
    <h2 class="clip" use:cliptip={title}>{title}</h2>
  {:else}
    <h2>{title}</h2>
  {/if}
  {#if total !== undefined}<span class="total">{total}</span>{/if}
  {@render left?.()}
  <span class="spring"></span>
  <!-- The right side is one flex item: in a pane too narrow for the whole row it drops
    under the title as a whole and keeps its right alignment (margin-left auto), rather
    than its members being squeezed to a character's width one by one -->
  <span class="tools">
    {@render right?.()}
    {#if askAi}
      <button class="ask" title={t("viewChrome.askAiHint")} onclick={askAi}>{t("viewChrome.askAi")}</button>
    {/if}
    {#if hint}<span class="hint">{hint}</span>{/if}
    {#if filterPlaceholder !== undefined}
      <input class="filter" bind:value={filter} placeholder={filterPlaceholder} spellcheck="false" />
    {/if}
    <!-- ✕ is Esc's visible entry point: it closes that view's tab, and closeTab activates the neighbour -->
    <button
      class="close"
      title={closeHint ?? t("viewChrome.closeView")}
      aria-label={t("viewChrome.closeViewLabel")}
      onclick={() => (onclose ? onclose() : void vault.closeActiveView())}>✕</button
    >
  </span>
</header>

<style>
  header {
    display: flex;
    /* Always center: baseline would hang the filter box high by its text's baseline, and
       whether there is a filter box differs per view, so baseline alignment cannot give
       "one shape" */
    align-items: center;
    /* A pane narrower than the row: the tool group wraps under the title (a measured
       screenshot at ~370px had the title on two lines, the count and the legend's
       buttons broken into one character per line). Nothing here shrinks below its
       words, so the only thing that gives is the line */
    flex-wrap: wrap;
    gap: 6px 10px;
    padding: 14px 20px 10px;
    border-bottom: 1px solid var(--border);
  }
  h2 {
    font-size: var(--fs-lg);
    font-weight: 600;
    white-space: nowrap;
  }
  /* The basis is what the row wraps by, so the tools stay on the title's line down to a
     12em title; above that it grows to its own width (max-content) and no further, leaving
     the spring the rest; below it, it gives way to a few characters */
  h2.clip {
    flex: 1 1 12em;
    max-width: max-content;
    min-width: 6em;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .total {
    font-size: var(--fs-xs);
    color: var(--fg-faint);
    white-space: nowrap;
  }
  .spring {
    flex: 1;
    /* the gap already provides the spacing, so the spring itself takes no width */
    min-width: 0;
  }
  .tools {
    display: flex;
    align-items: center;
    gap: 10px;
    margin-left: auto;
    min-width: 0;
  }
  .hint {
    font-size: var(--fs-xs);
    color: var(--fg-faint);
    /* In a narrow pane the hint is truncated first, so the filter box is not squeezed out —
       the hint is a sentence one can do without, and the filter box is an entry point */
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  /* The filter box gives way before the group wraps, down to a width that still shows a
     few characters of what was typed */
  .filter {
    width: 210px;
    min-width: 110px;
    flex-shrink: 1;
    padding: 5px 10px;
    font-size: var(--fs-sm);
  }
  .ask {
    flex-shrink: 0;
    white-space: nowrap;
    font-size: var(--fs-xs);
    color: var(--accent);
    border: 1px solid var(--accent);
    border-radius: 6px;
    padding: 2px 9px;
  }
  .ask:hover {
    background: var(--bg-hover);
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
</style>
