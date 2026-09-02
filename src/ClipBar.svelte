<script lang="ts">
  // The root component of the clip toolbar window (label `clipbar-N`): a 40px toolbar laid
  // inside the bottom of the `browser-N` browser window's frame (batch D). A browser window
  // is a remote page with zero IPC and no clipping UI can get in — so this bar is a window of
  // our own attached to it (the same approach as the agent window).
  //
  // It only displays and forwards: clipping, changing the destination and opening a note are
  // relayed back through clipbar_to_main to run in the main window (delivery has to go through
  // the main window's editor buffer, and the record of where things land is collected there
  // too), and the receipt comes back through clipbar://result to flash in place. **The focus
  // return contract**: after any interaction the focus goes back to the paired browser window
  // (so space paging and keyboard scrolling are unbroken) — the one exception is "already
  // clipped → open the note", where the user explicitly wants to go to the main window.
  //
  // The pinned destination (clipTarget) is a read-only mirror in this window: a change is
  // relayed to the main window to write and broadcast back (the store already listens at
  // module level); the optimistic local assignment only makes the button's label instant.
  import { t } from "$lib/i18n/i18n.svelte";
  import { onMount, tick } from "svelte";
  import { api, onBrowserEvent, onClipbarResult, type ClipbarAction } from "$lib/api";
  import { browserLabelOf, windowLabel } from "$lib/window";
  import { fuzzyFilter } from "$lib/commands/registry";
  import { domainOf } from "$lib/markdown";
  import { fetchHeadings, noteKey, resolveOrCreateNote, type PickHeading } from "$lib/pickNote";
  import { vault } from "$lib/stores/vault.svelte";
  import { browser } from "$lib/stores/browser.svelte";
  import { clips } from "$lib/stores/clips.svelte";
  import { clipTarget } from "$lib/stores/clipTarget.svelte";
  import StarIcon from "$lib/components/StarIcon.svelte";

  const myBrowserLabel = browserLabelOf(windowLabel);
  const me = $derived(browser.windows.find((w) => w.label === myBrowserLabel) ?? null);
  const url = $derived(me?.url ?? "");
  const docked = $derived(me?.docked ?? false);
  /** The note the current page has been clipped into (the badge's visibility, its title and "open" all read it). */
  const clippedPath = $derived(clips.pathFor(url));

  // Whether this window holds key: while it does not, the WKWebView receives no mouse events,
  // and after a click the focus return would freeze :hover on the last button the pointer
  // rested on (mouseleave never arrives) — so losing focus turns the hover styling off and the
  // frozen highlight goes out at once.
  let winFocused = $state(document.hasFocus());

  // ---- the receipt flash ----
  let flash = $state<{ msg: string; ok: boolean } | null>(null);
  let flashTimer: ReturnType<typeof setTimeout> | undefined;
  function showFlash(msg: string, ok = true) {
    flash = { msg, ok };
    clearTimeout(flashTimer);
    flashTimer = setTimeout(() => (flash = null), ok ? 2600 : 4200);
  }

  /** The focus return: when an interaction ends the focus goes back to the paired browser window, so reading is not interrupted. */
  const refocusBrowser = () => void api.browserFocus(myBrowserLabel).catch(() => {});

  /** The return leg of key-follows-mouse (the Rust side's mouseEntered makes this window key,
   *  so hovering makes it live): when the mouse leaves and nothing is being typed, key goes
   *  back to the browser window; while an address or a destination is being edited it does not
   *  (typing must not be interrupted), and when this window never held key nothing is
   *  returned. */
  function onBarLeave() {
    if (addrEditing || pickerOpen) return;
    if (!document.hasFocus()) return;
    refocusBrowser();
  }

  function relay(a: ClipbarAction, focus = false) {
    return api.clipbarToMain(a, focus).catch((e) => showFlash(String(e), false));
  }

  function clip(kind: "clip-selection" | "clip-page" | "clip-link") {
    void relay({ kind, label: myBrowserLabel });
    refocusBrowser();
  }

  function openClipped() {
    // Explicitly going to the main window: the relay carries focus
    if (clippedPath) void relay({ kind: "open-note", path: clippedPath }, true);
  }

  function toggleDocked() {
    void api.browserSetDocked(!docked, myBrowserLabel).catch((e) => showFlash(String(e), false));
    refocusBrowser();
  }

  /** Back and forward: history.back/forward is sent blind (canGoBack is unknowable, a v0.4
   *  compromise, and the end of the history is a no-op) — the same as the main window's page
   *  card; after a click the focus returns to the page so space paging is unbroken. */
  function nav(dir: "back" | "forward") {
    void (dir === "back" ? api.browserBack(myBrowserLabel) : api.browserForward(myBrowserLabel))
      .catch((e) => showFlash(String(e), false));
    refocusBrowser();
  }

  const starred = $derived(browser.isBookmarked(url));
  function toggleBookmark() {
    // Bookmarking needs no state from the main window, so it invokes directly (the same as
    // browser_navigate); the star's state is broadcast through bookmarks_changed and both
    // windows' mirrors stay in sync
    if (!url) return;
    api
      .bookmarkToggle(url, me?.title ?? "")
      .then((on) => showFlash(t(on ? "clipBar.bookmarked" : "clipBar.unbookmarked")))
      .catch((e) => showFlash(String(e), false));
    refocusBrowser();
  }

  function summonAgent() {
    // The chat window takes the focus itself, so the main window is not raised beforehand; the
    // current page is extracted by the main window and attached automatically as a context chip
    // (summon-agent is its own action and does not take focus-agent's panel branch)
    void relay({ kind: "summon-agent", label: myBrowserLabel });
  }

  function clearTarget() {
    clipTarget.clear(); // optimistic: the label recovers at once; the real change is written in the main window and broadcast back
    void relay({ kind: "clear-clip-target" });
    refocusBrowser();
  }

  // ---- The address area: it shows the domain and turns into an input on a click (URL-only, the same meaning as ⌘L) ----
  let addrEditing = $state(false);
  let addrValue = $state("");
  let addrEl = $state<HTMLInputElement | null>(null);
  async function openAddr() {
    addrValue = url;
    addrEditing = true;
    await tick();
    addrEl?.focus();
    addrEl?.select();
  }
  function closeAddr(refocus = true) {
    addrEditing = false;
    if (refocus) refocusBrowser();
  }
  function addrKeydown(e: KeyboardEvent) {
    if (e.key === "Enter") {
      e.preventDefault();
      const v = addrValue.trim();
      if (!v) return closeAddr();
      api
        .browserNavigate(v, myBrowserLabel)
        .then(() => closeAddr())
        .catch((err) => showFlash(String(err), false));
    } else if (e.key === "Escape") {
      e.preventDefault();
      closeAddr();
    }
  }

  // ---- The destination picker (▾): the window grows to expand it, a simplified two-stage
  // pick-note ----
  // The CommandPalette component is not reused: a full-screen overlay and this 320px embedded
  // panel look nothing alike.
  // What the two do share — the data contract (the match key, the section projection, creating
  // a destination) — is collected in $lib/pickNote.
  let pickerOpen = $state(false);
  let pickQuery = $state("");
  let pickSel = $state(0);
  let pickStage = $state<{ path: string; title: string } | null>(null);
  let pickHeadings = $state<PickHeading[]>([]);
  let pickEl = $state<HTMLInputElement | null>(null);
  let pickListEl = $state<HTMLDivElement | null>(null);

  // The selected row scrolls into view during keyboard navigation (the same as
  // CommandPalette's and AtPicker's; the effect runs after the DOM update, and nearest is a
  // no-op for a selection set by hover, so nothing jitters)
  $effect(() => {
    void pickSel;
    void rows;
    pickListEl?.querySelector(".pick-row.sel")?.scrollIntoView({ block: "nearest" });
  });

  async function openPicker() {
    // This window does not subscribe to the vault events (see attachCurrent(false)), so the lists are fetched at the moment they are needed
    void vault.refreshNotes().catch(() => {});
    pickQuery = "";
    pickSel = 0;
    pickStage = null;
    pickerOpen = true;
    try {
      await api.clipbarSetExpanded(true);
    } catch {
      /* it still works with the window's height unchanged: the list scrolls inside the bar */
    }
    await tick();
    pickEl?.focus();
  }
  function closePicker(refocus = true) {
    pickerOpen = false;
    void api.clipbarSetExpanded(false).catch(() => {});
    if (refocus) refocusBrowser();
  }

  $effect(() => {
    if (pickerOpen && pickStage) {
      const path = pickStage.path;
      void fetchHeadings(path).then((hs) => {
        if (pickStage?.path === path) pickHeadings = hs;
      });
    } else {
      pickHeadings = [];
    }
  });

  type Row = { id: string; title: string; hint?: string; pick?: { path: string; title: string }; run: () => void };
  const rows = $derived.by((): Row[] => {
    if (pickStage) {
      const stage = pickStage;
      return [
        { id: "__whole__", title: t("palette.wholeNote"), hint: stage.path, run: () => choose(stage.path, null) },
        ...fuzzyFilter(pickHeadings, pickQuery, (h) => h.text).map((h, i) => ({
          id: `${i}:${h.text}`,
          title: `${"#".repeat(h.level)} ${h.text}`,
          hint: t("palette.appendToSection"),
          run: () => choose(stage.path, h.text),
        })),
      ];
    }
    const notes = fuzzyFilter(vault.notes, pickQuery, noteKey);
    const list: Row[] = notes.slice(0, 50).map((n) => ({
      id: n.id,
      title: n.title,
      hint: n.id,
      pick: { path: n.id, title: n.title },
      run: () => choose(n.id, null),
    }));
    const q = pickQuery.trim();
    if (q && !notes.some((n) => n.title.toLowerCase() === q.toLowerCase())) {
      list.push({
        id: "__create__",
        title: t("palette.createNamed", { title: q }),
        hint: t("clipBar.createAndClip"),
        run: async () => {
          try {
            choose(await resolveOrCreateNote(q), null);
          } catch (e) {
            showFlash(String(e), false);
          }
        },
      });
    }
    return list;
  });

  function choose(path: string, heading: string | null) {
    // The optimistic assignment is instant; the source of truth is the main window (written through the relay), and what is broadcast back is the same value
    clipTarget.path = path;
    clipTarget.heading = heading;
    closePicker();
    // The same meaning as the main window's page card ▾: choosing pins it **and immediately
    // clips the current selection in** — pinning without delivering would leave "create a note"
    // as nothing but an empty file (the confusion of the first hand test). Both relays go
    // through the same channel and the await keeps their order: the pin lands in the main
    // window first, so the clip flow reads the new destination; with no selection the bar
    // flashes "nothing selected" while the pin itself still takes effect.
    void (async () => {
      await relay({ kind: "set-clip-target", path, heading });
      await relay({ kind: "clip-selection", label: myBrowserLabel });
    })();
  }

  function enterStage(p: { path: string; title: string }) {
    pickStage = p;
    pickQuery = "";
    pickSel = 0;
    queueMicrotask(() => pickEl?.focus());
  }

  function pickKeydown(e: KeyboardEvent) {
    if (e.key === "Escape") {
      e.preventDefault();
      if (pickStage) {
        pickStage = null;
        pickQuery = "";
        pickSel = 0;
      } else closePicker();
    } else if (e.key === "Tab" && !pickStage) {
      const p = rows[pickSel]?.pick;
      if (p) {
        e.preventDefault();
        enterStage(p);
      }
    } else if (e.key === "ArrowDown") {
      e.preventDefault();
      pickSel = Math.min(pickSel + 1, rows.length - 1);
    } else if (e.key === "ArrowUp") {
      e.preventDefault();
      pickSel = Math.max(pickSel - 1, 0);
    } else if (e.key === "Enter") {
      e.preventDefault();
      rows[pickSel]?.run();
    } else if (e.key === "Backspace" && pickStage && pickQuery === "") {
      e.preventDefault();
      pickStage = null;
      pickSel = 0;
    }
  }
  // The current page's clipped badge
  $effect(() => {
    if (url) void clips.lookup([url]);
  });

  onMount(() => {
    const subs = [
      onBrowserEvent((e) => browser.handle(e)),
      onClipbarResult((r) => {
        showFlash(r.message, r.ok);
        if (r.ok && url) void clips.lookup([url]);
      }),
    ];
    // Take over the vault that is open (vault.root lets clipTarget restore its pin). It
    // subscribes to no vault events and prefetches no lists (false, false): only the
    // destination picker wants the note list, and openPicker fetches it then; a permanent
    // subscription would make every autosave in the main window cause four vault-wide IPCs on
    // every toolbar, and a copy prefetched at mount would only be overwritten by that fetch
    void Promise.all([vault.attachCurrent(false, false), browser.refresh()]).catch(() => {});
    return () => {
      for (const s of subs) void s.then((un) => un());
    };
  });
</script>

<svelte:window onfocus={() => (winFocused = true)} onblur={() => (winFocused = false)} />

<div class="clipbar" class:expanded={pickerOpen} class:blurred={!winFocused} role="toolbar" aria-label={t("clipBar.label")} tabindex="-1" onmouseleave={onBarLeave}>
  {#if pickerOpen}
    <div class="picker">
      <input
        class="pick-input"
        bind:this={pickEl}
        bind:value={pickQuery}
        oninput={() => (pickSel = 0)}
        placeholder={pickStage ? t("clipBar.pickSectionOf", { title: pickStage.title }) : t("clipBar.pickPlaceholder")}
        spellcheck="false"
        onkeydown={pickKeydown}
      />
      <div class="pick-list" bind:this={pickListEl}>
        {#each rows as row, i (row.id)}
          <div class="pick-row" class:sel={i === pickSel}>
            <button class="pick-main" onclick={row.run} onpointerenter={() => (pickSel = i)}>
              <span class="pick-t">{row.title}</span>
              {#if row.hint}<span class="pick-h">{row.hint}</span>{/if}
            </button>
            {#if row.pick}
              <button class="pick-sub" title={t("clipBar.pickSubHint")} onclick={() => enterStage(row.pick!)}>{t("palette.pickSection")}</button>
            {/if}
          </div>
        {:else}
          <div class="pick-empty">{t(pickStage ? "clipBar.noMatchingSection" : "clipBar.noMatchingNote")}</div>
        {/each}
      </div>
    </div>
  {/if}
  <div class="bar">
    <button class="act mini" title={t("common.back")} aria-label={t("common.back")} onclick={() => nav("back")}>←</button>
    <button class="act mini" title={t("common.forward")} aria-label={t("common.forward")} onclick={() => nav("forward")}>→</button>
    {#if addrEditing}
      <input
        class="addr-input"
        bind:this={addrEl}
        bind:value={addrValue}
        placeholder={t("clipBar.urlPlaceholder")}
        spellcheck="false"
        onkeydown={addrKeydown}
        onblur={() => closeAddr(false)}
      />
    {:else}
      <button class="addr" title={url ? t("clipBar.addressHint", { url }) : t("clipBar.addressEmptyHint")} onclick={openAddr}>
        {domainOf(url) || t("clipBar.addressEmpty")}
      </button>
    {/if}
    {#if url}
      <!-- The browser convention: the bookmark star belongs with the address (a page-level action) rather than scattered into the status area on the right -->
      <button class="act star" class:starred title={t(starred ? "clipBar.unstar" : "clipBar.star")} onclick={toggleBookmark}>
        <StarIcon filled={starred} />
      </button>
    {/if}
    <button class="act" title={t("clipBar.summonAgentHint")} onclick={summonAgent}>AI</button>
    <span class="sep"></span>
    <button
      class="act target"
      title={clipTarget.path ? t("clipBar.selectionToTargetHint", { path: `${clipTarget.path}${clipTarget.heading ? ` › ${clipTarget.heading}` : ""}` }) : t("clipBar.selectionDefaultHint")}
      onclick={() => clip("clip-selection")}
    >{t("reader.selectionToTarget", { name: clipTarget.name ?? t("reader.note") })}</button>
    <button class="act mini" title={t("clipBar.pickTargetHint")} onclick={() => (pickerOpen ? closePicker() : void openPicker())}>▾</button>
    {#if clipTarget.path}
      <button class="act mini" title={t("clipBar.unpinHint")} onclick={clearTarget}>✕</button>
    {/if}
    <button class="act" title={t("clipBar.clipPageHint")} onclick={() => clip("clip-page")}>{t("clipBar.clipPage")}</button>
    <button class="act" title={t("clipBar.clipLinkHint")} onclick={() => clip("clip-link")}>{t("clipBar.clipLink")}</button>
    <span class="flex"></span>
    {#if flash}
      <span class="flash" class:err={!flash.ok} title={flash.msg}>{flash.msg}</span>
    {/if}
    {#if clippedPath}
      <button class="act clipped" title={t("clipBar.openClippedHint", { path: clippedPath })} onclick={openClipped}>{t("reader.alreadyClipped")}</button>
    {/if}
    <button class="act" title={t(docked ? "clipBar.undockHint" : "clipBar.dockHint")} onclick={toggleDocked}>{t(docked ? "clipBar.undock" : "clipBar.dock")}</button>
  </div>
</div>

<style>
  /* A transparent window: the background is painted here, and the bottom corners match the
     macOS window's rounding (a square-cornered bar over a rounded window's bottom edge would
     poke out). Expanded it is a panel floating over the page, rounded on all four corners. */
  :global(html),
  :global(body) {
    background: transparent !important;
  }
  .clipbar {
    height: 100vh;
    display: flex;
    flex-direction: column;
    overflow: hidden;
    background: var(--bg-panel);
    border-top: 1px solid var(--border);
    border-radius: 0 0 10px 10px;
    color: var(--fg);
    -webkit-user-select: none;
    user-select: none;
  }
  .clipbar.expanded {
    border-radius: 10px;
    border: 1px solid var(--border);
  }
  .bar {
    flex: none;
    height: 39px; /* 40 − border-top */
    display: flex;
    align-items: center;
    gap: 4px;
    padding: 0 10px;
    min-width: 0;
  }
  .flex {
    flex: 1;
  }
  .sep {
    width: 1px;
    height: 18px;
    background: var(--border);
    margin: 0 4px;
    flex: none;
  }
  .addr {
    flex: 0 1 auto;
    max-width: 220px;
    min-width: 60px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    border: none;
    background: none;
    color: var(--fg-muted);
    font-size: 12px;
    padding: 4px 6px;
    border-radius: 6px;
    cursor: text;
    text-align: left;
  }
  .clipbar:not(.blurred) .addr:hover {
    background: var(--bg-hover);
  }
  .addr-input {
    flex: 1 1 260px;
    max-width: 380px;
    font-size: 12px;
    padding: 4px 6px;
    border: 1px solid var(--accent);
    border-radius: 6px;
    background: var(--bg);
    color: var(--fg);
    outline: none;
  }
  .act {
    flex: none;
    border: none;
    background: none;
    color: var(--fg);
    font-size: 12px;
    padding: 4px 8px;
    border-radius: 6px;
    cursor: pointer;
    white-space: nowrap;
    max-width: 200px;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .clipbar:not(.blurred) .act:hover {
    background: var(--bg-hover);
  }
  .act.mini {
    padding: 4px 5px;
    color: var(--fg-muted);
  }
  .act.target {
    color: var(--accent);
  }
  .act.clipped {
    color: var(--accent);
    background: var(--accent-soft);
  }
  .act.star {
    display: grid;
    place-items: center;
    padding: 4px 5px;
    color: var(--fg-muted);
  }
  /* Hover does not colour it (only the common background): after a click the focus return
     would freeze :hover on the button, and a frozen colour would be misread as "bookmarked" */
  .act.star.starred {
    color: var(--accent);
  }
  .flash {
    flex: 0 1 auto;
    font-size: 12px;
    color: var(--fg-muted);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    max-width: 260px;
  }
  .flash.err {
    color: #d05252;
  }

  /* ---- the expanded destination picker ---- */
  .picker {
    flex: 1;
    min-height: 0;
    display: flex;
    flex-direction: column;
    padding: 8px 8px 0;
    gap: 6px;
  }
  .pick-input {
    flex: none;
    font-size: 13px;
    padding: 6px 8px;
    border: 1px solid var(--border);
    border-radius: 6px;
    background: var(--bg);
    color: var(--fg);
    outline: none;
  }
  .pick-input:focus {
    border-color: var(--accent);
  }
  .pick-list {
    flex: 1;
    overflow-y: auto;
    min-height: 0;
  }
  .pick-row {
    display: flex;
    align-items: center;
    border-radius: 6px;
  }
  .pick-row.sel {
    background: var(--bg-hover);
  }
  .pick-main {
    flex: 1;
    min-width: 0;
    display: flex;
    align-items: baseline;
    gap: 8px;
    border: none;
    background: none;
    color: var(--fg);
    font-size: 12.5px;
    text-align: left;
    padding: 5px 8px;
    cursor: pointer;
  }
  .pick-t {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .pick-h {
    flex: none;
    color: var(--fg-faint);
    font-size: 11px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    max-width: 40%;
  }
  .pick-sub {
    flex: none;
    border: none;
    background: none;
    color: var(--fg-muted);
    font-size: 11px;
    padding: 4px 8px;
    cursor: pointer;
    visibility: hidden;
    border-radius: 6px;
  }
  .pick-row.sel .pick-sub,
  .pick-row:hover .pick-sub {
    visibility: visible;
  }
  .pick-sub:hover {
    background: var(--bg-active);
  }
  .pick-empty {
    color: var(--fg-faint);
    font-size: 12px;
    padding: 8px;
  }
</style>
