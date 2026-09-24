<script lang="ts">
  import type { Snippet } from "svelte";

  // A group of rows under a small heading. It hides itself when a search left none of its
  // rows showing (the rows hide themselves; :has reads what is left).
  let { title, note, children }: { title?: string; note?: string; children: Snippet } = $props();
</script>

<section class="sec">
  {#if title}<h3>{title}</h3>{/if}
  {#if note}<p class="note">{note}</p>{/if}
  <div class="rows">{@render children()}</div>
</section>

<style>
  .sec {
    display: flex;
    flex-direction: column;
  }
  .sec:not(:has(.rows > :not([hidden]))) {
    display: none;
  }
  .sec + :global(.sec) {
    margin-top: 18px;
  }
  h3 {
    font-size: var(--fs-xs);
    font-weight: 600;
    color: var(--fg-muted);
    margin-bottom: 2px;
  }
  .note {
    font-size: var(--fs-xs);
    color: var(--fg-muted);
    margin: 2px 0 4px;
  }
  /* The last row of a group needs no rule under it: the next heading's space separates */
  .rows > :global(.row:last-child) {
    border-bottom: none;
  }
</style>
