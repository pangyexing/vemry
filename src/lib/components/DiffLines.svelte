<script lang="ts">
  // The lines of a unified diff, one block per line, inside the caller's <pre> (which owns
  // the frame: its border, its height, its scrolling, and user-select on the scroller;
  // the lines own the type). One drawing for the three places a diff is read before a
  // decision or after one — the permission dialog, the note history, the session replay —
  // so a line reads the same in all three.
  //
  // A deleted line keeps the body ink: it is text the reader has to read before allowing
  // the change (the red that used to carry it fell to 4.24:1 on the dark ground). Its
  // tinted ground and the strike say "deleted"; the strike is drawn in the danger colour
  // so the letters under it stay whole. Hunk headers and file headers are secondary text.
  import type { DiffLine } from "$lib/views/diff";

  let { lines }: { lines: DiffLine[] } = $props();
</script>

<!-- The newline is part of each line's text, so a copied selection comes out as the diff it is -->
{#each lines as line, i (i)}<span class="dl {line.cls}">{line.text + "\n"}</span>{/each}

<style>
  .dl {
    display: block;
    font-family: var(--font-mono);
    font-size: var(--fs-xs);
    line-height: 1.6;
    padding: 0 var(--diff-pad, 10px);
    white-space: pre-wrap;
    /* A long word breaks only where nothing else can: `break-all` split English words at any letter */
    overflow-wrap: anywhere;
  }
  .add {
    background: var(--diff-add-bg);
    color: var(--accent);
  }
  .del {
    background: var(--diff-del-bg);
    color: var(--fg);
    text-decoration: line-through;
    text-decoration-color: color-mix(in srgb, var(--danger) 70%, transparent);
  }
  .hunk,
  .meta,
  .ctx {
    color: var(--fg-muted);
  }
</style>
