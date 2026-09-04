<script lang="ts">
  // The editor's tab bar (replacing the old note-header): back and forward at the left end,
  // the tab list on the right.
  // A preview tab is italic (a single click reuses that slot) and a double click on the
  // title promotes it; drag-reordering goes through pointer events plus the suppressClick
  // contract (the same as FileTree's, with no ghost drawn — the id-keyed each preserves DOM
  // identity, the pointer capture is unbroken, and moveTab runs live while dragging).
  import { vault, stemOf, viewOfTab, type Tab } from "$lib/stores/vault.svelte";
  import { VIEW_ICONS } from "./ActivityBar.svelte";
  import { contextMenu } from "$lib/stores/contextMenu.svelte";
  import { t } from "$lib/i18n/i18n.svelte";
  import { k } from "$lib/shortcut";
  import { tick } from "svelte";
  import Chevron from "./Chevron.svelte";

  let stripEl = $state<HTMLElement | null>(null);

  // ---- overflow ----
  // The strip hides its scrollbar, so with many tabs open nothing would say that more sit
  // beyond an edge. Two signals stand in: the overflowing edge fades out (a mask on the
  // strip), and a list button appears at the right end that names every open tab. Both
  // follow the measured scroll state, refreshed on scroll, on a resize of the strip and
  // after the tab list itself changes (the widths settle only once the DOM has).
  let overflowLeft = $state(false);
  let overflowRight = $state(false);
  const overflowing = $derived(overflowLeft || overflowRight);

  function measureOverflow() {
    const el = stripEl;
    if (!el) return;
    overflowLeft = el.scrollLeft > 1;
    overflowRight = el.scrollLeft + el.clientWidth < el.scrollWidth - 1;
  }

  $effect(() => {
    const el = stripEl;
    if (!el) return;
    const ro = new ResizeObserver(measureOverflow);
    ro.observe(el);
    return () => ro.disconnect();
  });

  $effect(() => {
    // read what changes a tab's width: the set of tabs, the italic of the preview slot, and
    // which tab is active (the active tab keeps its natural width while the rest give way, so
    // an activation alone can push the strip into overflow, or out of it, with no scroll to
    // report it; the observer above sees only the strip's own box, not its content)
    void vault.tabs.map((t) => t.pinned);
    void vault.activeTabId;
    void tick().then(measureOverflow);
  });

  /** The list of every open tab, hung under the button at the right end: the active one is
   *  ticked, a view tab reads as "View: …" (the same wording as its tooltip), and "Close
   *  all" sits below — the one place to empty a crowded bar in one go. */
  function listTabs(btn: HTMLElement) {
    const r = btn.getBoundingClientRect();
    const active = vault.activeTabId;
    contextMenu.openAt(r.left, r.bottom + 4, [
      ...vault.tabs.map((tab) => {
        const view = viewOfTab(tab.path);
        return {
          label: view ? t("tabBar.viewTab", { name: t(`view.${view}`) }) : tabName(tab.path),
          checked: tab.id === active,
          action: () => activate(tab, tab.id === active),
        };
      }),
      { separator: true },
      {
        label: t("tabBar.closeAll"),
        action: async () => {
          // two existing funnels instead of a third: the others go first, then the survivor
          // closes to empty (flush, the mainView fallback) exactly as a single close does
          const id = vault.activeTabId;
          if (id == null) return;
          await vault.closeOtherTabs(id);
          if (vault.activeTabId === id) await vault.closeTab(id);
        },
      },
    ]);
  }

  /** A tab's display name: a view tab shows the view's name (the sentinel path never leaks
   *  out); a note drops the .md (the same rule as the file tree's) and everything else keeps
   *  its extension. */
  const tabName = (p: string) => {
    const view = viewOfTab(p);
    return view ? t(`view.${view}`) : stemOf(p);
  };

  /** A single click on a tab: already active = pull the cursor and the scroll back into view,
   *  otherwise switch to it. click and keydown share the semantics, and writing them twice
   *  would certainly drift. */
  const activate = (tab: Tab, isActive: boolean) =>
    isActive ? vault.revealActiveTab() : void vault.activateTab(tab.id);

  // A vertical wheel becomes horizontal scrolling. Svelte 5's onwheel is passive by default
  // and preventDefault would be ignored (the lesson GraphView learned) — so it is attached
  // non-passive by hand, or the vertical scroll's default is not stopped and the outer
  // container scrolls and bounces while it moves sideways
  $effect(() => {
    const el = stripEl;
    if (!el) return;
    const onWheel = (e: WheelEvent) => {
      if (!e.deltaX && e.deltaY) {
        e.preventDefault();
        el.scrollLeft += e.deltaY;
      }
    };
    el.addEventListener("wheel", onWheel, { passive: false });
    return () => el.removeEventListener("wheel", onWheel);
  });

  // When the active tab changes it is scrolled into view (a tab at the end of an overflowing bar)
  $effect(() => {
    const id = vault.activeTabId;
    if (id == null || !stripEl) return;
    stripEl.querySelector(`[data-tab-id="${id}"]`)?.scrollIntoView({ block: "nearest", inline: "nearest" });
  });

  // ---- drag-reordering ----
  let pressed: { id: number; x: number; y: number } | null = null;
  let dragging = $state<number | null>(null);
  let suppressClick = false;

  function pressTab(e: PointerEvent, id: number) {
    if (e.button !== 0) return;
    pressed = { id, x: e.clientX, y: e.clientY };
    (e.currentTarget as HTMLElement | null)?.setPointerCapture?.(e.pointerId);
  }

  function dragMove(e: PointerEvent) {
    if (pressed && dragging == null) {
      if (Math.hypot(e.clientX - pressed.x, e.clientY - pressed.y) < 5) return;
      dragging = pressed.id;
    }
    if (dragging == null || !stripEl) return;
    e.preventDefault();
    // The target index = the number of "other tabs" whose midpoint is left of the pointer
    // (exactly moveTab's remove-then-insert rule); an unchanged value does nothing, avoiding
    // an empty splice at pointermove frequency
    const els = [...stripEl.querySelectorAll<HTMLElement>("[data-tab-id]")];
    const fromIdx = els.findIndex((el) => Number(el.dataset.tabId) === dragging);
    if (fromIdx < 0) return;
    let to = 0;
    for (let i = 0; i < els.length; i++) {
      if (i === fromIdx) continue;
      const r = els[i].getBoundingClientRect();
      if (r.left + r.width / 2 < e.clientX) to++;
    }
    if (to !== fromIdx) vault.moveTab(dragging, to);
  }

  function dragEnd() {
    pressed = null;
    if (dragging == null) return;
    dragging = null;
    suppressClick = true;
  }

  function dragCancel() {
    pressed = null;
    dragging = null;
    // After a pointercancel no click comes to consume the one-shot flag, and a leftover would swallow the next click
    suppressClick = false;
  }

  function tabMenu(e: MouseEvent, tab: Tab) {
    e.preventDefault();
    e.stopPropagation();
    const close = [
      { label: t("tabBar.close"), hint: k("Mod+W"), action: () => void vault.closeTab(tab.id) },
      { label: t("tabBar.closeOthers"), action: () => void vault.closeOtherTabs(tab.id) },
      { label: t("tabBar.closeToRight"), action: () => void vault.closeTabsToRight(tab.id) },
    ];
    // A view tab is not a file: revealing it or copying its path means nothing, so only the closing items are kept
    if (viewOfTab(tab.path)) {
      contextMenu.open(e, close);
      return;
    }
    contextMenu.open(e, [
      ...close,
      { separator: true },
      {
        label: t("tabBar.revealInTree"),
        action: async () => {
          await vault.activateTab(tab.id);
          vault.revealInTree();
        },
      },
      { label: t("tabBar.copyRelativePath"), action: () => void navigator.clipboard.writeText(tab.path) },
    ]);
  }
</script>

<svelte:window
  onpointermove={dragMove}
  onpointerup={dragEnd}
  onpointercancel={dragCancel}
  onclickcapture={(e) => {
    if (suppressClick) {
      suppressClick = false;
      e.stopPropagation();
      e.preventDefault();
    }
  }}
  onkeydowncapture={(e) => {
    if (e.key === "Escape" && dragging != null) {
      // The capture phase: App's window bubble listener is registered earlier and the bubble
      // phase runs first, so an Esc cancelling a drag that is not caught during capture would
      // also trigger its "Escape closes the projection view" layer and close the kanban or
      // agenda tab being looked at (the same discipline as Kanban's)
      e.stopPropagation();
      e.preventDefault();
      dragCancel();
      // The click on release lands back on the starting tab (pointer capture semantics) and must not switch to it
      suppressClick = true;
    }
  }}
/>

<div
  class="tabbar"
  class:tree-closed={!vault.leftOpen}
  class:panel-closed={!vault.rightOpen}
  class:projected={vault.mainView === "reading"}
>
  <div class="nav-btns">
    <button title={t("tabBar.backWithKey", { key: k("Mod+Alt+←") })} aria-label={t("common.back")} disabled={!vault.canGoBack} onclick={() => void vault.goBack()}>
      <svg width="16" height="16" viewBox="0 0 20 20" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
        <path d="M12 5 L7 10 L12 15" />
      </svg>
    </button>
    <button title={t("tabBar.forwardWithKey", { key: k("Mod+Alt+→") })} aria-label={t("common.forward")} disabled={!vault.canGoForward} onclick={() => void vault.goForward()}>
      <svg width="16" height="16" viewBox="0 0 20 20" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
        <path d="M8 5 L13 10 L8 15" />
      </svg>
    </button>
  </div>
  <!-- A vertical wheel becomes horizontal scrolling: the tab bar only scrolls sideways (the listener is attached non-passive by hand in the script) -->
  <div class="tabstrip" class:fade-left={overflowLeft} class:fade-right={overflowRight} bind:this={stripEl} onscroll={measureOverflow}>
    {#each vault.tabs as tab (tab.id)}
      {@const isActive = tab.id === vault.activeTabId}
      {@const view = viewOfTab(tab.path)}
      <div
        class="tab"
        class:active={isActive}
        class:preview={!tab.pinned}
        class:dirty={isActive && vault.dirty}
        class:dragging={dragging === tab.id}
        data-tab-id={tab.id}
        role="tab"
        tabindex="-1"
        aria-selected={isActive}
        title={view ? t("tabBar.viewTab", { name: t(`view.${view}`) }) : tab.path}
        onpointerdown={(e) => pressTab(e, tab.id)}
        onclick={() => activate(tab, isActive)}
        onkeydown={(e) => (e.key === "Enter" || e.key === " ") && activate(tab, isActive)}
        ondblclick={() => vault.pinActiveTab()}
        onauxclick={(e) => {
          if (e.button === 1) {
            e.preventDefault();
            void vault.closeTab(tab.id);
          }
        }}
        oncontextmenu={(e) => tabMenu(e, tab)}
      >
        {#if view}
          <!-- A view tab carries a small icon (the same glyphs as the rail's): it tells the
            "Kanban" view apart from a note that happens to be called "Kanban" -->
          <svg class="vicon" width="14" height="14" viewBox="0 0 20 20" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
            {#each VIEW_ICONS[view] as d (d)}<path {d} />{/each}
          </svg>
        {/if}
        <span class="tab-name">{tabName(tab.path)}</span>
        <span class="slot">
          <span class="dot" title={t("tabBar.unsaved", { key: k("Mod+S") })}>●</span>
          <button
            class="x"
            title={t("tabBar.close")}
            aria-label={t("tabBar.closeNamed", { name: tabName(tab.path) })}
            tabindex="-1"
            onpointerdown={(e) => e.stopPropagation()}
            onclick={(e) => {
              e.stopPropagation();
              void vault.closeTab(tab.id);
            }}
          >
            <svg width="12" height="12" viewBox="0 0 20 20" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" aria-hidden="true">
              <path d="M5 5 L15 15 M15 5 L5 15" />
            </svg>
          </button>
        </span>
      </div>
    {/each}
  </div>
  {#if overflowing}
    <button
      class="list-btn"
      title={t("tabBar.listTabs", { n: vault.tabs.length })}
      aria-label={t("tabBar.listTabs", { n: vault.tabs.length })}
      aria-haspopup="menu"
      onclick={(e) => listTabs(e.currentTarget)}
    >
      <span class="count">{vault.tabs.length}</span>
      <span class="chevron"><Chevron /></span>
    </button>
  {/if}
</div>

<style>
  .tabbar {
    display: flex;
    align-items: center;
    gap: 6px;
    /* The 8/5 padding is not arbitrary: this bottom edge has to land on the same horizontal
       line as the right column's .tabs (both columns' tabs are 13px/1.5 text plus a 2px active
       bottom border), and a bar height slightly off would show a step where the columns meet */
    padding: 8px 12px 0;
    width: 100%;
    min-width: 0;
    /* The same bottom edge as the right column's .tabs: the body scrolling under the bar is a hard cut, and without a divider the hierarchy cannot be read */
    border-bottom: 1px solid var(--border);
    /* Attached directly to .center's flex column, and never squashed by a projection view */
    flex-shrink: 0;
  }
  /* Under the reading projection (the one overlay state that does not enter the tabs) the
     whole bar quietens down — what the main area shows is not any tab's content, so the
     active highlight (the accent bottom border) is withdrawn; clicking any tab lands back on
     its content.
     Every other view is a tab itself, keeps its active highlight as usual and does not take
     this branch */
  .tabbar.projected .tab {
    color: var(--fg-faint);
    border-bottom-color: transparent;
  }
  .tabbar.projected .tab:hover {
    color: var(--fg-muted);
  }
  .vicon {
    flex-shrink: 0;
  }
  /* With a sidebar collapsed it clears the floating reopen button in that corner (the old note-header contract) */
  .tabbar.tree-closed {
    padding-left: 46px;
  }
  .tabbar.panel-closed {
    padding-right: 46px;
  }
  .nav-btns {
    display: flex;
    gap: 2px;
    flex-shrink: 0;
  }
  .nav-btns button {
    display: grid;
    place-items: center;
    width: 22px;
    height: 22px;
    border-radius: 5px;
    color: var(--fg-muted);
  }
  .nav-btns button:hover {
    background: var(--bg-hover);
    color: var(--fg);
  }
  .nav-btns button:disabled {
    opacity: 0.35;
    pointer-events: none;
  }
  .tabstrip {
    display: flex;
    align-items: stretch;
    min-width: 0;
    overflow-x: auto;
    /* The thin scrollbar is hidden: the wheel or a drag moves it sideways, and an always-visible bar would lower the bar's height */
    scrollbar-width: none;
    /* The width of the faded edge; the scroll padding is the same length, so scrollIntoView
       lands the active tab clear of the fade, not under it */
    --fade: 28px;
    scroll-padding-inline: var(--fade);
  }
  .tabstrip::-webkit-scrollbar {
    display: none;
  }
  /* The overflowing edge fades out: the one cue, with the scrollbar hidden, that tabs continue past it */
  .tabstrip.fade-left {
    -webkit-mask-image: linear-gradient(to right, transparent 0, #000 var(--fade));
    mask-image: linear-gradient(to right, transparent 0, #000 var(--fade));
  }
  .tabstrip.fade-right {
    -webkit-mask-image: linear-gradient(to left, transparent 0, #000 var(--fade));
    mask-image: linear-gradient(to left, transparent 0, #000 var(--fade));
  }
  .tabstrip.fade-left.fade-right {
    -webkit-mask-image: linear-gradient(to right, transparent 0, #000 var(--fade), #000 calc(100% - var(--fade)), transparent 100%);
    mask-image: linear-gradient(to right, transparent 0, #000 var(--fade), #000 calc(100% - var(--fade)), transparent 100%);
  }
  .list-btn {
    display: flex;
    align-items: center;
    gap: 2px;
    flex-shrink: 0;
    height: 22px;
    padding: 0 5px 0 7px;
    border-radius: 5px;
    color: var(--fg-muted);
    font-size: var(--fs-xs);
    font-variant-numeric: tabular-nums;
  }
  .list-btn:hover {
    background: var(--bg-hover);
    color: var(--fg);
  }
  /* The shared chevron points right; the list hangs below the button, so it points down */
  .list-btn .chevron {
    display: inline-flex;
    transform: rotate(90deg);
  }
  .tab {
    display: flex;
    align-items: center;
    gap: 4px;
    /* Content-sized while the bar has room; once it is short of room every tab gives way in
       proportion down to the minimum, and only then does the strip scroll — a dozen tabs
       stay in sight as narrow ones rather than half of them slipping off the edge */
    flex: 0 1 auto;
    min-width: 96px;
    max-width: 180px;
    padding: 5px 6px 5px 10px;
    border-radius: 5px 5px 0 0;
    border-bottom: 2px solid transparent;
    font-size: var(--fs-sm);
    color: var(--fg-muted);
    cursor: pointer;
    -webkit-user-select: none;
    user-select: none;
  }
  .tab:hover {
    background: var(--bg-hover);
  }
  /* The same visual language as the right column's .tabs: a 2px accent bottom border marks the active one */
  .tab.active {
    color: var(--fg);
    border-bottom-color: var(--accent);
    /* The one tab whose name must stay readable in a crowded bar keeps its natural width */
    flex-shrink: 0;
  }
  .tab.dragging {
    opacity: 0.6;
  }
  .tab.preview .tab-name {
    font-style: italic;
  }
  .tab-name {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .slot {
    position: relative;
    width: 16px;
    height: 16px;
    flex-shrink: 0;
  }
  .dot {
    display: none;
    position: absolute;
    inset: 0;
    text-align: center;
    line-height: 16px;
    color: var(--accent);
    font-size: 9px;
  }
  .tab.dirty .dot {
    display: block;
  }
  .tab.dirty:hover .dot {
    display: none;
  }
  .x {
    position: absolute;
    inset: 0;
    display: grid;
    place-items: center;
    border-radius: 4px;
    color: var(--fg-faint);
    visibility: hidden;
  }
  .tab:hover .x,
  .tab.active .x {
    visibility: visible;
  }
  .tab.dirty:not(:hover) .x {
    visibility: hidden;
  }
  .x:hover {
    background: var(--bg-active);
    color: var(--fg);
  }
</style>
