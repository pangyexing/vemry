<!-- The one rendering point of the application's context menu (mounted once in App, with
  the state in the contextMenu store). Viewport clamping, closing on a click outside or
  Esc, danger, disabled and the second confirmation (confirm) are built in; the styling
  follows the FileTree .menu family. -->
<script lang="ts">
  import { contextMenu, type MenuItem } from "$lib/stores/contextMenu.svelte";

  const pos = $derived(contextMenu.pos);
  const entries = $derived(contextMenu.entries);

  let el = $state<HTMLElement | null>(null);
  let confirming = $state<number | null>(null);
  /** The clamped landing point: rendered at the raw coordinates first, then measured against its own rect after mounting and pulled inside the viewport. */
  let clamped = $state<{ x: number; y: number } | null>(null);

  $effect(() => {
    confirming = null;
    const p = pos;
    clamped = p;
    if (!p) return;
    queueMicrotask(() => {
      if (!el || pos !== p) return;
      const r = el.getBoundingClientRect();
      clamped = {
        x: Math.min(p.x, Math.max(4, window.innerWidth - r.width - 4)),
        y: Math.min(p.y, Math.max(4, window.innerHeight - r.height - 4)),
      };
    });
  });

  const close = () => contextMenu.close();

  function onItemClick(item: MenuItem, i: number) {
    if (item.disabled) return;
    if (item.confirm && confirming !== i) {
      confirming = i;
      return;
    }
    close();
    item.action();
  }

  // Close on a click outside (pointerdown comes before click, so a click inside the menu is
  // unaffected); the right-click that opened the menu arrives before pos is assigned, pos is
  // still null, and it naturally does not close by mistake
  function onWindowPointerdown(e: PointerEvent) {
    if (!pos) return;
    if (el && e.target instanceof Node && el.contains(e.target)) return;
    close();
  }
  function onWindowKeydown(e: KeyboardEvent) {
    if (!pos || e.key !== "Escape") return;
    // Only the menu closes: App's layered Escape logic must not switch a projection view away or close a panel along with it
    e.stopPropagation();
    e.preventDefault();
    close();
  }
</script>

<svelte:window onpointerdowncapture={onWindowPointerdown} onkeydowncapture={onWindowKeydown} />

{#if clamped}
  <div class="menu" bind:this={el} style:left={`${clamped.x}px`} style:top={`${clamped.y}px`} role="menu">
    {#each entries as entry, i (i)}
      {#if "separator" in entry}
        <div class="menu-sep"></div>
      {:else if "title" in entry}
        <div class="menu-title">{entry.title}</div>
      {:else}
        <button
          class:danger={entry.danger || (entry.confirm != null && confirming === i)}
          disabled={entry.disabled}
          onclick={() => onItemClick(entry, i)}
          role="menuitem"
        >
          {confirming === i && entry.confirm ? entry.confirm : entry.label}
          {#if entry.hint}<span class="key-hint">{entry.hint}</span>{/if}
        </button>
      {/if}
    {/each}
  </div>
{/if}

<style>
  .menu {
    position: fixed;
    z-index: 250;
    min-width: 160px;
    background: var(--bg-panel);
    border: 1px solid var(--border);
    border-radius: 8px;
    box-shadow: var(--shadow);
    padding: 4px;
    display: flex;
    flex-direction: column;
  }
  .menu-title {
    font-size: var(--fs-xs);
    font-family: var(--font-mono);
    color: var(--fg-faint);
    padding: 4px 10px 2px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    max-width: 220px;
  }
  .menu-sep {
    height: 1px;
    background: var(--border);
    margin: 4px 6px;
  }
  .menu button {
    text-align: left;
    padding: 6px 10px;
    border-radius: 5px;
    font-size: var(--fs-sm);
    /* A long name on an item carrying a path or a title is truncated rather than widening the menu */
    max-width: 300px;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .menu button:hover:not(:disabled) {
    background: var(--bg-hover);
  }
  .menu button.danger {
    color: var(--danger);
  }
  .menu button:disabled {
    color: var(--fg-faint);
    cursor: default;
  }
  .key-hint {
    float: right;
    margin-left: 14px;
    color: var(--fg-faint);
    font-size: var(--fs-xs);
  }
</style>
