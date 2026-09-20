<!-- The candidate list under a note-path box (the action card's target note): the notes
  matching what the box holds, each as its stem with the folder dimmed after it, so a long
  path is told apart by the part that differs rather than cut off at the part that repeats.
  A native datalist shows the raw path in a popup as narrow as the box and clips its end —
  every note of one folder came out as the same line.
  With nothing typed, the homes already chosen on the card lead (a tidy-up files most of
  its captures into two or three notes), then the notes by last edit. Private notes are
  left out, as the @ picker leaves them out: the agent cannot write there. So are the notes
  found under an attachments/ folder: those are a note's files, not homes — the rule
  propose_actions enforces on the model's side.
  Fixed positioning: the card sits in the timeline, which scrolls, and an absolute layer
  would be clipped by it; when the timeline scrolls the list closes rather than follows.
  The focus stays in the box: a press on a row does not take it (mousedown is prevented),
  and the host forwards the box's keys through handleKey. -->
<script lang="ts">
  import type { NoteSummary } from "$lib/api";
  import { fuzzyFilter } from "$lib/commands/registry";
  import { t } from "$lib/i18n/i18n.svelte";
  import { isImeKeydown } from "$lib/ime";
  import { noteKey } from "$lib/pickNote";
  import { inAttachments, parentOf, stemOf, vault } from "$lib/stores/vault.svelte";

  let {
    anchor,
    within,
    query,
    pinned = [],
    onpick,
    onclose,
  }: {
    /** The box the list hangs under; its right edge is the list's. */
    anchor: HTMLElement;
    /** The element the list spans: as wide as it (capped), so the rows have room for a folder. */
    within: HTMLElement;
    /** What the box holds: the filter. */
    query: string;
    /** Paths to lead with when nothing is typed (the card's other targets); unknown ones are skipped. */
    pinned?: string[];
    onpick: (path: string) => void;
    onclose: () => void;
  } = $props();

  const MAX_SHOW = 8;

  const pool = $derived(vault.notes.filter((n) => !vault.privatePaths.has(n.id) && !inAttachments(n.id)));
  const cands = $derived.by((): NoteSummary[] => {
    if (query.trim()) return fuzzyFilter(pool, query, noteKey).slice(0, MAX_SHOW);
    const byId = new Map(pool.map((n) => [n.id, n]));
    const lead: NoteSummary[] = [];
    for (const p of pinned) {
      const n = byId.get(p);
      if (n && !lead.includes(n)) lead.push(n);
    }
    const rest = pool.filter((n) => !lead.includes(n)).sort((a, b) => b.mtime_ms - a.mtime_ms);
    return [...lead, ...rest].slice(0, MAX_SHOW);
  });

  let sel = $state(0);
  $effect(() => {
    void query;
    sel = 0; // after typing more of the filter, the selection starts from the top again
  });
  const selIdx = $derived(Math.max(0, Math.min(sel, cands.length - 1)));

  let el = $state<HTMLElement | null>(null);
  $effect(() => {
    void selIdx;
    if (!cands.length) return;
    const row = el?.children[selIdx];
    if (row instanceof HTMLElement) row.scrollIntoView({ block: "nearest" });
  });

  /** Where the list lands: under the box, or above it when the room below is short. */
  let pos = $state<{ left: number; width: number; top: number | null; bottom: number | null; maxH: number } | null>(null);
  function measure() {
    const a = anchor.getBoundingClientRect();
    const w = within.getBoundingClientRect();
    const width = Math.min(420, Math.max(a.width, w.width));
    const left = Math.max(4, a.right - width);
    const below = window.innerHeight - a.bottom - 8;
    const above = a.top - 8;
    const down = below >= 160 || below >= above;
    pos = {
      left,
      width,
      top: down ? a.bottom + 3 : null,
      bottom: down ? null : window.innerHeight - a.top + 3,
      maxH: Math.max(80, Math.min(240, down ? below : above)),
    };
  }
  $effect(() => {
    void anchor;
    void within;
    measure();
  });

  /** A scroll anywhere but inside the list itself: the box has moved from under it. */
  function onScroll(e: Event) {
    if (el && e.target instanceof Node && el.contains(e.target)) return;
    onclose();
  }

  /** Forwarded from the box's onkeydown; true = consumed. Enter and Escape pass the IME
   *  guard: confirming or cancelling an input-method candidate is neither picking nor closing. */
  export function handleKey(e: KeyboardEvent): boolean {
    if (isImeKeydown(e)) return false;
    if (e.key === "ArrowDown") {
      e.preventDefault();
      sel = Math.min(selIdx + 1, cands.length - 1);
      return true;
    }
    if (e.key === "ArrowUp") {
      e.preventDefault();
      sel = Math.max(selIdx - 1, 0);
      return true;
    }
    if (e.key === "Escape") {
      // stopPropagation: this Esc only closes the list; the text stays as typed
      e.preventDefault();
      e.stopPropagation();
      onclose();
      return true;
    }
    if (e.key === "Enter" && cands.length) {
      e.preventDefault();
      onpick(cands[selIdx].id);
      return true;
    }
    return false;
  }
</script>

<svelte:window onscrollcapture={onScroll} onresize={measure} />

<!-- Typed text that matches no note says so: a list that silently shows nothing reads as a
     broken list (a file without .md is not a note, and the box looked dead) -->
{#if cands.length || query.trim()}
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div
    class="ns"
    bind:this={el}
    style={pos
      ? `left:${pos.left}px; width:${pos.width}px; max-height:${pos.maxH}px; ${pos.top !== null ? `top:${pos.top}px` : `bottom:${pos.bottom}px`}`
      : "visibility:hidden"}
    onmousedown={(e) => e.preventDefault()}
  >
    {#each cands as n, i (n.id)}
      {@const dir = parentOf(n.id)}
      <button class="ns-row" class:sel={i === selIdx} title={n.id} onmouseenter={() => (sel = i)} onclick={() => onpick(n.id)}>
        <span class="ns-stem">{stemOf(n.id)}</span>
        {#if dir}<span class="ns-dir">{dir}</span>{/if}
      </button>
    {/each}
    {#if !cands.length}<div class="ns-empty">{t("chat.actionsNoMatch")}</div>{/if}
  </div>
{/if}

<style>
  .ns {
    position: fixed;
    z-index: 95; /* the @ picker's: above the overlay, below the command palette and the dialogs */
    display: flex;
    flex-direction: column;
    gap: 1px;
    padding: 4px;
    overflow-y: auto;
    background: var(--bg-panel);
    border: 1px solid var(--border);
    border-radius: 8px;
    box-shadow: var(--shadow);
  }
  .ns-row {
    display: flex;
    align-items: baseline;
    gap: 8px;
    width: 100%;
    min-width: 0;
    padding: 4px 8px;
    border-radius: 6px;
    text-align: left;
    color: var(--fg);
  }
  .ns-row.sel {
    background: var(--accent-soft);
  }
  .ns-empty {
    padding: 5px 8px;
    font-size: var(--fs-xs);
    color: var(--fg-faint);
  }
  /* The stem is what tells the notes apart, so the folder gives way first: it only
     takes what the stem leaves, and is ellipsized there */
  .ns-stem {
    flex: 0 1 auto;
    min-width: 0;
    font-size: var(--fs-xs);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .ns-dir {
    flex: 1 1 0;
    min-width: 0;
    font-size: 11px;
    color: var(--fg-faint);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
</style>
