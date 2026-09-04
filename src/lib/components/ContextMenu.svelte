<!-- The one rendering point of the application's context menu (mounted once in App, with
  the state in the contextMenu store). Viewport clamping, closing on a click outside or
  Esc, danger, disabled and the second confirmation (confirm) are built in; the styling
  follows the FileTree .menu family. -->
<script lang="ts">
  import { usableRight } from "$lib/viewport";
  import { contextMenu, type MenuItem } from "$lib/stores/contextMenu.svelte";

  const pos = $derived(contextMenu.pos);
  const entries = $derived(contextMenu.entries);
  /** A menu with a checkable item reserves the left gutter on every row, so the ticked and unticked labels stay in one column. */
  const hasChecks = $derived(entries.some((e) => "checked" in e && e.checked != null));

  let el = $state<HTMLElement | null>(null);
  let confirming = $state<number | null>(null);
  /** The open second level: which parent item, and where its flyout sits (fixed coordinates, flipped left when the right edge overflows). */
  let sub = $state<{ index: number; x: number; y: number } | null>(null);
  let subEl = $state<HTMLElement | null>(null);
  let subTimer: ReturnType<typeof setTimeout> | null = null;
  /** The clamped landing point: rendered at the raw coordinates first, then measured against its own rect after mounting and pulled inside the viewport. */
  let clamped = $state<{ x: number; y: number } | null>(null);

  $effect(() => {
    confirming = null;
    sub = null;
    const p = pos;
    clamped = p;
    if (!p) return;
    queueMicrotask(() => {
      if (!el || pos !== p) return;
      const r = el.getBoundingClientRect();
      clamped = {
        x: Math.min(p.x, Math.max(4, usableRight() - r.width - 4)),
        y: Math.min(p.y, Math.max(4, window.innerHeight - r.height - 4)),
      };
    });
  });

  const close = () => contextMenu.close();

  function onItemClick(item: MenuItem, i: number, btn: HTMLElement) {
    if (item.disabled) return;
    if (item.submenu) {
      openSub(i, btn);
      return;
    }
    if (item.confirm && confirming !== i) {
      confirming = i;
      return;
    }
    close();
    item.action?.();
  }

  /** The flyout is anchored against the row that opened it, so the caller hands over
   *  the element it already has: `entries` counts separators and titles that are not
   *  menuitems, so an index into the button list would anchor against the wrong row. */
  function openSub(i: number, btn: HTMLElement) {
    if (subTimer) clearTimeout(subTimer);
    const r = btn.getBoundingClientRect();
    sub = { index: i, x: r.right + 2, y: r.top - 4 };
    queueMicrotask(() => {
      if (!subEl || !sub || sub.index !== i) return;
      const sr = subEl.getBoundingClientRect();
      const x = r.right + 2 + sr.width > window.innerWidth - 4 ? Math.max(4, r.left - sr.width - 2) : r.right + 2;
      const y = Math.min(r.top - 4, Math.max(4, window.innerHeight - sr.height - 4));
      sub = { index: i, x, y };
    });
  }

  function onItemEnter(item: MenuItem, i: number, btn: HTMLElement) {
    if (subTimer) clearTimeout(subTimer);
    if (item.submenu && !item.disabled) subTimer = setTimeout(() => openSub(i, btn), 120);
    else if (sub) subTimer = setTimeout(() => (sub = null), 200);
  }

  function onSubClick(item: MenuItem) {
    if (item.disabled) return;
    close();
    item.action?.();
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
  <div
    class="menu"
    bind:this={el}
    style:left={`${clamped.x}px`}
    style:top={`${clamped.y}px`}
    role="menu"
    tabindex="-1"
    onmouseleave={() => {
      if (subTimer) clearTimeout(subTimer);
      subTimer = setTimeout(() => (sub = null), 200);
    }}
    onmouseenter={() => {
      if (subTimer) clearTimeout(subTimer);
    }}
  >
    {#each entries as entry, i (i)}
      {#if "separator" in entry}
        <div class="menu-sep"></div>
      {:else if "title" in entry}
        <div class="menu-title">{entry.title}</div>
      {:else}
        <button
          class:danger={entry.danger || (entry.confirm != null && confirming === i)}
          class:open={sub?.index === i}
          class:checkable={hasChecks}
          disabled={entry.disabled}
          onclick={(e) => onItemClick(entry, i, e.currentTarget)}
          onmouseenter={(e) => onItemEnter(entry, i, e.currentTarget)}
          role={entry.checked != null ? "menuitemcheckbox" : "menuitem"}
          aria-checked={entry.checked != null ? entry.checked : undefined}
        >
          {#if entry.checked}
            <svg class="check" width="12" height="12" viewBox="0 0 20 20" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
              <path d="M4 10.5 L8.5 15 L16 6" />
            </svg>
          {/if}
          {confirming === i && entry.confirm ? entry.confirm : entry.label}
          {#if entry.submenu}<span class="key-hint">›</span>{:else if entry.hint}<span class="key-hint">{entry.hint}</span>{/if}
        </button>
      {/if}
    {/each}
    {#if sub}
      {@const parent = entries[sub.index]}
      {#if parent && !("separator" in parent) && !("title" in parent) && parent.submenu}
        <div class="menu menu-sub" bind:this={subEl} style:left={`${sub.x}px`} style:top={`${sub.y}px`} role="menu">
          {#each parent.submenu as item, j (j)}
            {#if "separator" in item}
              <div class="menu-sep"></div>
            {:else if "title" in item}
              <div class="menu-title">{item.title}</div>
            {:else}
              <button class:danger={item.danger} disabled={item.disabled} onclick={() => onSubClick(item)} role="menuitem">
                {item.label}
                {#if item.hint}<span class="key-hint">{item.hint}</span>{/if}
              </button>
            {/if}
          {/each}
        </div>
      {/if}
    {/if}
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
    /* A long list (every open tab) scrolls inside the panel rather than running off the
       viewport; the flyout is position: fixed and is not clipped by this */
    max-height: calc(100vh - 16px);
    overflow-y: auto;
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
  .menu button:hover:not(:disabled),
  .menu button.open {
    background: var(--bg-hover);
  }
  .menu button.checkable {
    position: relative;
    padding-left: 26px;
  }
  .check {
    position: absolute;
    left: 8px;
    top: 50%;
    transform: translateY(-50%);
    color: var(--accent);
  }
  /* the flyout is a child of the root menu so the outside-click and Esc handling cover it unchanged */
  .menu-sub {
    z-index: 251;
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
