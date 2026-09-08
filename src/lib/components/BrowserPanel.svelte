<script lang="ts">
  // The browser chrome: the tab strip, the address row with the clip and AI entry
  // points, and — docked — the slot the native tab view is laid over. One component
  // serves both shapes: mode="docked" is the main window's panel column, where every
  // action runs in place; mode="bar" is the root of the detached window's chrome bar
  // (a separate webview above the tabs), where the clip actions are relayed to the
  // main window (the editor, the clip destination and the landing record live there)
  // and the destination picker grows the bar over the page.
  //
  // A tab is a native view: nothing this component draws can float over it. Menus
  // are native popups (they do), a dropdown under the address row is not offered,
  // and while a full-window overlay is open the tab is hidden and the slot shows a
  // placeholder (browser.veiled).
  //
  // **The focus-return contract**: after any interaction the keyboard goes back to the
  // page, so space paging and keyboard scrolling are unbroken; "open the clipped note"
  // is the one exception, where the user explicitly wants the main window.
  import { t } from "$lib/i18n/i18n.svelte";
  import { onMount, tick } from "svelte";
  import { Menu, type MenuOptions } from "@tauri-apps/api/menu";
  import { api, onBrowserbarCommand, onBrowserbarResult, type BrowserbarAction, type HistoryEntryView } from "$lib/api";
  import { isUrlLike, prettyUrl, webCandidates, type WebCandidate } from "$lib/urls";
  import { executeCommand, fuzzyFilter } from "$lib/commands/registry";
  import { domainOf } from "$lib/markdown";
  import { isMac, k } from "$lib/shortcut";
  import { fetchHeadings, noteKey, resolveOrCreateNote, type PickHeading } from "$lib/pickNote";
  import { vault } from "$lib/stores/vault.svelte";
  import { agent } from "$lib/stores/agent.svelte";
  import { browser, MAIN_MIN_BESIDE_PANEL, PANEL_MIN } from "$lib/stores/browser.svelte";
  import { clips } from "$lib/stores/clips.svelte";
  import { clipTarget } from "$lib/stores/clipTarget.svelte";
  import { toast } from "$lib/stores/toast.svelte";
  import StarIcon from "$lib/components/StarIcon.svelte";

  let { mode = "docked" }: { mode?: "docked" | "bar" } = $props();
  const inBar = $derived(mode === "bar");

  const BLANK = "about:blank";
  const cur = $derived(browser.current);
  /** A blank tab (just opened with + or ⌘T): its page is hidden and the slot shows the new-tab page. */
  const isBlank = $derived(cur?.url === BLANK);
  const url = $derived(cur && !isBlank ? cur.url : "");
  const starred = $derived(browser.isBookmarked(url));
  /** The note the current page has been clipped into (the badge and "open it" both read it). */
  const clippedPath = $derived(clips.pathFor(url));

  // ---- the receipt flash (bar mode: the main window's toast is out of sight) ----
  let flash = $state<{ msg: string; ok: boolean } | null>(null);
  let flashTimer: ReturnType<typeof setTimeout> | undefined;
  function showFlash(msg: string, ok = true) {
    flash = { msg, ok };
    clearTimeout(flashTimer);
    flashTimer = setTimeout(() => (flash = null), ok ? 2600 : 4200);
  }
  const err = (e: unknown) => (inBar ? showFlash(String(e), false) : toast.push(String(e), "error"));

  /** The focus return: the keyboard goes back to the page once an interaction ends. A blank
   *  tab has no page to take it (its view is hidden, the address input holds the keyboard);
   *  `force` is for a navigation just sent, whose Navigated event has not flipped isBlank yet. */
  const refocus = (force = false) => {
    if (cur && (force || !isBlank)) void api.browserFocus(cur.label).catch(() => {});
  };
  /** After a navigation of the active tab: the address row folds back into the button
   *  (addrEditing may be left over from an edit of the previous tab's address), and the
   *  keyboard goes to the page as after any navigation. */
  function afterNavigate() {
    addrEditing = false;
    refocus(true);
  }
  function run(p: Promise<unknown>, back = true) {
    void p.catch(err).finally(() => {
      if (back) refocus();
    });
  }
  function relay(a: BrowserbarAction, focus = false) {
    return api.browserbarToMain(a, focus).catch((e) => showFlash(String(e), false));
  }

  // ---- navigation and tabs ----
  const back = () => run(api.browserBack(cur?.label));
  const forward = () => run(api.browserForward(cur?.label));
  const reload = () => run(api.browserReload(cur?.label));
  const activate = (label: string) => run(api.browserActivate(label));
  const closeTab = (label: string) => run(api.browserCloseTab(label), false);
  /** + and ⌘T: a blank tab in place. Docked, the address row's input opens on its own and takes
   *  the keyboard, and the slot shows the new-tab page (bookmarks, recents) under it; in the bar
   *  the address input opens for it. No popup anywhere — the user's eye stays on the panel. */
  function newTab() {
    void api
      .browserNewTab()
      .then(() => {
        if (inBar) void openAddr(false);
      })
      .catch(err);
  }
  const closeAll = () => run(api.browserCloseAll(), false);
  /** The items of a native popup menu (the tab menu, the clip menu, the more menu). */
  type NativeItems = NonNullable<MenuOptions["items"]>;
  /** The menu shown last: a native menu is a resource that only an explicit close frees,
   *  and an item's action can still arrive after popup() resolves — so each menu is freed
   *  when the next one opens, not on its own dismissal. */
  let lastMenu: Menu | null = null;
  async function popup(items: NativeItems) {
    const menu = await Menu.new({ items });
    const prev = lastMenu;
    lastMenu = menu;
    if (prev) void prev.close().catch(() => {});
    await menu.popup();
  }
  function tabMenu(e: MouseEvent, tab: { label: string; url: string }) {
    e.preventDefault();
    void popup([
      { id: "close", text: t("browserPanel.closeTab"), action: () => closeTab(tab.label) },
      { item: "Separator" },
      { id: "system", text: t("reader.openInSystemBrowser"), action: () => run(api.urlOpenSystem(tab.url), false) },
    ]);
  }

  // ---- the address row ----
  let addrEditing = $state(false);
  let addrValue = $state("");
  let addrEl = $state<HTMLInputElement | null>(null);
  /** Docked, a blank tab has nothing to show in the address but the input itself: it stays
   *  open (a blur does not fold it back into the button) and is the new-tab page's one input —
   *  a second one drawn in the slot would only duplicate it. */
  const addrOpen = $derived(addrEditing || (isBlank && !inBar));
  async function openAddr(forNewTab = false) {
    addrValue = forNewTab ? "" : url;
    addrEditing = true;
    await tick();
    addrEl?.focus();
    addrEl?.select();
  }
  function closeAddr(back = true) {
    addrEditing = false;
    if (back) refocus();
  }
  function addrKeydown(e: KeyboardEvent) {
    if (isBlank && !inBar) return ntKeydown(e);
    if (e.key === "Enter") {
      e.preventDefault();
      const v = addrValue.trim();
      if (!v) return closeAddr();
      // A bare Enter navigates the active tab (a blank one included); ⌘Enter opens a new one
      const newTab = e.metaKey || e.ctrlKey || !cur;
      const p = newTab ? api.browserOpen(v, true) : api.browserNavigate(v, cur?.label);
      p.then(afterNavigate).catch(err);
    } else if (e.key === "Escape") {
      e.preventDefault();
      closeAddr();
    }
  }
  function toggleBookmark() {
    if (!url) return;
    run(
      api.bookmarkToggle(url, cur?.title ?? "").then((on) => {
        const msg = t(on ? "clipBar.bookmarked" : "clipBar.unbookmarked");
        if (inBar) showFlash(msg);
        else toast.push(msg);
      }),
    );
  }

  // ---- the AI and clip entry points ----
  /** Docked: the page joins the research panel's conversation as context (the panel is brought
   *  out). Bar: the floating chat window is summoned with the page attached (relayed; the main
   *  window opens the window first and extracts the page after). */
  /** The translation pane inside the current tab: opened, or closed when it is open (the same IPC from the bar and the docked chrome). */
  const trState = $derived(cur ? browser.translating[cur.label] ?? null : null);
  function translatePage() {
    if (!cur) return;
    api.browserTranslateToggle(cur.label).catch(err);
  }

  async function ai() {
    if (!cur) return;
    if (inBar) {
      void relay({ kind: "summon-agent", label: cur.label });
      return;
    }
    try {
      const page = await agent.grabPage();
      if (page) {
        agent.attachPage(page);
        vault.rightOpen = true;
        vault.rightTab = "agent";
      }
    } catch (e) {
      err(e);
    }
  }
  function clip(kind: "clip-selection" | "clip-page" | "clip-link") {
    if (!cur) return;
    if (inBar) void relay({ kind, label: cur.label });
    else void executeCommand(kind);
    refocus();
  }
  function clipMenu() {
    const items: NativeItems = [
      { id: "pick", text: t("browserPanel.pickTarget"), action: () => pickTarget() },
      { item: "Separator" },
      { id: "page", text: t("clipBar.clipPage"), action: () => clip("clip-page") },
      { id: "link", text: t("clipBar.clipLink"), action: () => clip("clip-link") },
    ];
    if (clipTarget.path) items.push({ item: "Separator" }, { id: "unpin", text: t("clipBar.unpinHint"), action: clearTarget });
    if (clippedPath) items.push({ item: "Separator" }, { id: "open", text: t("clipBar.openClippedHint", { path: clippedPath }), action: openClipped });
    void popup(items);
  }
  function pickTarget() {
    if (inBar) void openPicker();
    else void executeCommand("clip-selection-to-note");
  }
  function clearTarget() {
    clipTarget.clear(); // bar: optimistic, the main window writes it and broadcasts back
    if (inBar) void relay({ kind: "clear-clip-target" });
    refocus();
  }
  function openClipped() {
    if (!clippedPath) return;
    if (inBar) void relay({ kind: "open-note", path: clippedPath }, true);
    else void vault.openNote(clippedPath);
  }
  function moreMenu() {
    const ad = browser.adblock;
    const adText =
      ad?.state === "ready"
        ? t("agentPanel.adblockOn", { n: ad.rules.toLocaleString() })
        : ad?.state === "failed"
          ? t("agentPanel.adblockFailed", { error: ad.error })
          : null;
    const items: NativeItems = [];
    if (adText) items.push({ id: "adblock", text: adText, enabled: false }, { item: "Separator" });
    if (url) items.push({ id: "system", text: t("reader.openInSystemBrowser"), action: () => run(api.urlOpenSystem(url), false) });
    items.push({ id: "history", text: t("cmd.clear-browser-history.title"), action: () => run(api.browserHistoryClear(), false) });
    void popup(items);
  }

  // ---- the shape switch ----
  const toggleDock = () => run(api.browserSetDocked(inBar), false);

  // ---- bar mode: the destination picker (a simplified two-stage pick-note; the bar grows
  // over the page to make room). The CommandPalette component is not reused (a
  // full-screen overlay and this 360px strip look nothing alike); what the two share —
  // the match key, the section projection, creating a destination — is in $lib/pickNote.
  let pickerOpen = $state(false);
  let pickQuery = $state("");
  let pickSel = $state(0);
  let pickStage = $state<{ path: string; title: string } | null>(null);
  let pickHeadings = $state<PickHeading[]>([]);
  let pickEl = $state<HTMLInputElement | null>(null);
  let pickListEl = $state<HTMLDivElement | null>(null);

  $effect(() => {
    void pickSel;
    void rows;
    pickListEl?.querySelector(".pick-row.sel")?.scrollIntoView({ block: "nearest" });
  });
  async function openPicker() {
    // The bar subscribes to no vault events (attachCurrent(false)), so the list is fetched when it is needed
    void vault.refreshNotes().catch(() => {});
    pickQuery = "";
    pickSel = 0;
    pickStage = null;
    pickerOpen = true;
    await api.browserbarSetExpanded(true).catch(() => {});
    await tick();
    pickEl?.focus();
  }
  function closePicker(back = true) {
    pickerOpen = false;
    void api.browserbarSetExpanded(false).catch(() => {});
    if (back) refocus();
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
  /** Choosing pins the destination **and clips the current selection into it** (pinning
   *  alone would leave a created note empty); the two relays are awaited in order so the
   *  pin lands in the main window before the clip flow reads it. */
  function choose(path: string, heading: string | null) {
    clipTarget.path = path;
    clipTarget.heading = heading;
    closePicker();
    if (!cur) return;
    const label = cur.label;
    void (async () => {
      await relay({ kind: "set-clip-target", path, heading });
      await relay({ kind: "clip-selection", label });
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

  // The clipped badge follows the current page
  $effect(() => {
    if (url) void clips.lookup([url]);
  });

  // ---- docked: the new-tab page in a blank tab's slot — bookmarks and recents (the same two
  // lists as the palette's web space, rendered in place so opening a tab never sends the eye
  // to a popup), filtered by what is typed in the address row, which takes the keyboard at
  // once and is the page's only input ----
  let ntSel = $state(0);
  let ntHistory = $state<HistoryEntryView[]>([]);
  $effect(() => {
    if (inBar || !isBlank) return;
    addrValue = "";
    ntSel = 0;
    api
      .browserHistoryList()
      .then((l) => (ntHistory = l))
      .catch(() => (ntHistory = []));
    // The input mounts with the blank tab (addrOpen), so it exists only after the render
    void tick().then(() => addrEl?.focus());
  });
  type NtRow = WebCandidate & { head?: string };
  // The same pool and the same fuzzy filter as the palette's web space, so a query ranks the same in both
  const ntCandidates = $derived(webCandidates(browser.bookmarks, ntHistory));
  const ntRows = $derived.by((): NtRow[] => {
    const q = addrValue.trim();
    const pick = (starred: boolean, cap: number): NtRow[] =>
      fuzzyFilter(ntCandidates.filter((c) => c.starred === starred), q, (c) => c.key).slice(0, q ? 24 : cap);
    const marks = pick(true, 8);
    const recents = pick(false, 12);
    if (!q) {
      if (marks[0]) marks[0] = { ...marks[0], head: t("palette.bookmarks") };
      if (recents[0]) recents[0] = { ...recents[0], head: t("palette.recent") };
    }
    return [...marks, ...recents].slice(0, 24);
  });
  $effect(() => {
    void ntSel;
    void ntRows;
    rootEl?.querySelector(".nt-row.sel")?.scrollIntoView({ block: "nearest" });
  });
  function ntOpen(target: string) {
    if (!cur) return;
    api.browserNavigate(target, cur.label).then(afterNavigate).catch(err);
  }
  function ntKeydown(e: KeyboardEvent) {
    if (e.key === "Enter") {
      e.preventDefault();
      const v = addrValue.trim();
      // An address opens as typed; anything else opens the selected row, and with no row to
      // select it is handed to navigate as the address row always did
      if (isUrlLike(v)) ntOpen(v);
      else if (ntRows[ntSel]) ntOpen(ntRows[ntSel].url);
      else if (v) ntOpen(v);
    } else if (e.key === "Escape") {
      e.preventDefault();
      if (cur) closeTab(cur.label); // a blank tab abandoned is a tab closed
    } else if (e.key === "ArrowDown") {
      e.preventDefault();
      ntSel = Math.min(ntSel + 1, Math.max(0, ntRows.length - 1));
    } else if (e.key === "ArrowUp") {
      e.preventDefault();
      ntSel = Math.max(ntSel - 1, 0);
    }
  }

  // ---- docked mode: telling Rust where the panel column is. Only the anchors go over
  // (the column's width and the chrome's height), and only when they change — Rust places
  // the tab from them on every window resize itself. During a sash drag nothing is sent
  // (the tab is veiled meanwhile) and the final width goes once the drag ends. ----
  let rootEl = $state<HTMLDivElement | null>(null);
  let chromeEl = $state<HTMLDivElement | null>(null);
  /** The last anchors sent: the observer fires on every height change of the window too
   *  (the column is full-height), and those change nothing Rust needs to hear about. */
  let reported = "";
  function report(visible: boolean) {
    if (inBar) return;
    // Before the elements exist a report would be a guess; the observer sends one once they do
    if (visible && !(rootEl && chromeEl)) return;
    const w = Math.round(rootEl?.getBoundingClientRect().width ?? browser.panelWidth);
    const h = Math.round(chromeEl?.getBoundingClientRect().height ?? 0);
    const key = `${visible} ${w} ${h}`;
    if (key === reported) return;
    reported = key;
    void api.browserPanelLayout(visible, w, h).catch(() => {});
  }
  $effect(() => {
    if (!inBar && vault.resizing === null) report(true);
  });
  // Bar mode: the menu bar's ⌘L and ⌘T land here while the detached window holds the focus
  function barCommand(id: string) {
    if (id === "open-url") void openAddr(false);
    else if (id === "browser-new-tab") newTab();
  }
  function barKeydown(e: KeyboardEvent) {
    // Off mac the menu bar does not exist, so the bar keeps its own bindings (on mac they would fire twice)
    if (isMac || !(e.metaKey || e.ctrlKey)) return;
    const key = e.key.toLowerCase();
    if (key === "l") {
      e.preventDefault();
      void openAddr(false);
    } else if (key === "t") {
      e.preventDefault();
      newTab();
    } else if (key === "w" && cur) {
      e.preventDefault();
      closeTab(cur.label);
    } else if (key === "r") {
      e.preventDefault();
      reload();
    }
  }

  onMount(() => {
    if (inBar) {
      const subs = [
        onBrowserbarResult((r) => {
          showFlash(r.message, r.ok);
          if (r.ok && url) void clips.lookup([url]);
        }),
        onBrowserbarCommand(barCommand),
      ];
      return () => {
        for (const s of subs) void s.then((un) => un());
      };
    }
    const ro = new ResizeObserver(() => {
      if (vault.resizing !== "browser") report(true);
    });
    if (rootEl) ro.observe(rootEl);
    if (chromeEl) ro.observe(chromeEl);
    return () => {
      ro.disconnect();
      report(false);
    };
  });
</script>

<svelte:window onkeydown={inBar ? barKeydown : undefined} />

<div
  class="bpanel"
  class:bar={inBar}
  bind:this={rootEl}
  style:width={inBar ? null : `${browser.panelWidth}px`}
  style:max-width={inBar ? null : `calc(100vw - ${MAIN_MIN_BESIDE_PANEL}px)`}
  style:min-width={inBar ? null : `${PANEL_MIN}px`}
>
  <div class="chrome" bind:this={chromeEl}>
    <!-- The tab strip. In the bar it is also the detached window's drag region (the traffic lights float over its left end) -->
    <div class="tabs" role="tablist" aria-label={t("browserPanel.tabs")} data-tauri-drag-region={inBar ? "" : null}>
      {#each browser.tabs as tab (tab.label)}
        <div
          class="tab"
          class:active={tab.label === cur?.label}
          role="tab"
          tabindex="-1"
          aria-selected={tab.label === cur?.label}
          title={tab.title ? `${tab.title}\n${tab.url}` : tab.url}
          onclick={() => activate(tab.label)}
          onkeydown={(e) => { if (e.key === "Enter") activate(tab.label); }}
          onauxclick={(e) => { if (e.button === 1) closeTab(tab.label); }}
          oncontextmenu={(e) => void tabMenu(e, tab)}
        >
          <span class="tab-title">{tab.url === BLANK ? t("browserPanel.newTab") : tab.title || domainOf(tab.url) || tab.url}</span>
          <button class="tab-close" title={t("browserPanel.closeTab")} aria-label={t("browserPanel.closeTab")} onclick={(e) => { e.stopPropagation(); closeTab(tab.label); }}>
            <svg width="10" height="10" viewBox="0 0 10 10" fill="none" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" aria-hidden="true"><path d="M2 2 L8 8 M8 2 L2 8" /></svg>
          </button>
        </div>
      {/each}
      <button class="ico" title="{t('browserPanel.newTab')} {k('Mod+T')}" aria-label={t("browserPanel.newTab")} onclick={newTab}>
        <svg width="14" height="14" viewBox="0 0 14 14" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" aria-hidden="true"><path d="M7 2.5 V11.5 M2.5 7 H11.5" /></svg>
      </button>
      <span class="spacer" data-tauri-drag-region={inBar ? "" : null}></span>
      {#if inBar}
        <button class="ico" title={t("browserPanel.dock")} aria-label={t("browserPanel.dock")} onclick={toggleDock}>
          <svg width="16" height="16" viewBox="0 0 20 20" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><rect x="2.5" y="3.5" width="15" height="13" rx="1.75" /><path d="M12.5 3.5 V16.5" /></svg>
        </button>
      {:else}
        <!-- Detach, and close: × closes the browser outright (every tab), the plain "close a window"
          gesture — there is no hide-but-keep state, so nothing else is needed here -->
        <button class="ico" title={t("browserPanel.detach")} aria-label={t("browserPanel.detach")} onclick={toggleDock}>
          <svg width="16" height="16" viewBox="0 0 20 20" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M16.5 3.5 H11.5 M16.5 3.5 V8.5 M16.5 3.5 L9.5 10.5 M8.5 4.5 H4.5 A1 1 0 0 0 3.5 5.5 V15.5 A1 1 0 0 0 4.5 16.5 H14.5 A1 1 0 0 0 15.5 15.5 V11.5" /></svg>
        </button>
        <button class="ico" title={t("browserPanel.closeBrowser")} aria-label={t("browserPanel.closeBrowser")} onclick={closeAll}>
          <svg width="14" height="14" viewBox="0 0 14 14" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" aria-hidden="true"><path d="M3 3 L11 11 M11 3 L3 11" /></svg>
        </button>
      {/if}
    </div>
    <div class="nav">
      <button class="ico" title={t("common.back")} aria-label={t("common.back")} onclick={back} disabled={!cur}>
        <svg width="14" height="14" viewBox="0 0 14 14" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M9 2.5 L4.5 7 L9 11.5" /></svg>
      </button>
      <button class="ico" title={t("common.forward")} aria-label={t("common.forward")} onclick={forward} disabled={!cur}>
        <svg width="14" height="14" viewBox="0 0 14 14" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M5 2.5 L9.5 7 L5 11.5" /></svg>
      </button>
      <button class="ico" title="{t('agentPanel.reload')} {k('Mod+R')}" aria-label={t("agentPanel.reload")} onclick={reload} disabled={!cur}>
        <svg width="14" height="14" viewBox="0 0 14 14" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M11.5 7 A4.5 4.5 0 1 1 10.2 3.8 M10.5 1.8 V4.3 H8" /></svg>
      </button>
      {#if addrOpen}
        <input
          class="addr-input"
          bind:this={addrEl}
          bind:value={addrValue}
          placeholder={t("clipBar.urlPlaceholder")}
          spellcheck="false"
          oninput={() => (ntSel = 0)}
          onkeydown={addrKeydown}
          onblur={() => closeAddr(false)}
        />
      {:else}
        <button class="addr" title={url ? t("clipBar.addressHint", { url }) : t("clipBar.addressEmptyHint")} onclick={() => void openAddr(false)}>
          {#if url}
            <span class="addr-host">{domainOf(url)}</span>
            {#if cur?.title}<span class="addr-title">{cur.title}</span>{/if}
          {:else}
            <span class="addr-title">{t("clipBar.addressEmpty")}</span>
          {/if}
        </button>
      {/if}
      {#if url}
        <button class="ico star" class:starred title={t(starred ? "clipBar.unstar" : "clipBar.star")} aria-label={t(starred ? "clipBar.unstar" : "clipBar.star")} onclick={toggleBookmark}>
          <StarIcon filled={starred} size={13} />
        </button>
      {/if}
      <button class="act" title={inBar ? t("clipBar.summonAgentHint") : t("browserPanel.aiDockedHint")} onclick={() => void ai()} disabled={!cur || isBlank || agent.grabbingPage}>AI</button>
      <button
        class="act"
        class:on={!!trState}
        title={trState ? (trState.done >= trState.total ? t("browserPanel.translateDoneHint") : t("browserPanel.translateActiveHint", trState)) : t("browserPanel.translateHint")}
        onclick={() => void translatePage()}
        disabled={!cur || isBlank}
      >{t("browserPanel.translate")}{#if trState && trState.done < trState.total}<span class="tr-count">{trState.done}/{trState.total}</span>{/if}</button>
      <span class="clipgroup">
        <button
          class="act primary"
          title={clipTarget.path ? t("clipBar.selectionToTargetHint", { path: `${clipTarget.path}${clipTarget.heading ? ` › ${clipTarget.heading}` : ""}` }) : t("clipBar.selectionDefaultHint")}
          onclick={() => clip("clip-selection")}
          disabled={!cur || isBlank}
        >{t("reader.selectionToTarget", { name: clipTarget.name ?? t("reader.note") })}</button>
        <button class="act primary caret" title={t("browserPanel.clipMenuHint")} aria-label={t("browserPanel.clipMenuHint")} onclick={() => void clipMenu()} disabled={!cur || isBlank}>▾</button>
      </span>
      {#if clippedPath}
        <button class="ico clipped" title={t("clipBar.openClippedHint", { path: clippedPath })} aria-label={t("reader.alreadyClipped")} onclick={openClipped}>✓</button>
      {/if}
      <button class="ico" title={t("browserPanel.more")} aria-label={t("browserPanel.more")} onclick={() => void moreMenu()}>
        <svg width="14" height="14" viewBox="0 0 14 14" fill="currentColor" aria-hidden="true"><circle cx="3" cy="7" r="1.2" /><circle cx="7" cy="7" r="1.2" /><circle cx="11" cy="7" r="1.2" /></svg>
      </button>
    </div>
    {#if flash}
      <div class="flash" class:err={!flash.ok} title={flash.msg}>{flash.msg}</div>
    {/if}
  </div>

  {#if inBar}
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
  {:else}
    <!-- The native tab view is laid over this slot; what is drawn here shows for a blank tab
      (its new-tab page) and while the tab is veiled under an overlay -->
    <div class="page-slot" class:newtab={isBlank}>
      {#if isBlank}
        <div class="nt">
          <div class="nt-list">
            {#each ntRows as row, i (row.url)}
              {#if row.head}<div class="nt-head">{row.head}</div>{/if}
              <button class="nt-row" class:sel={i === ntSel} onclick={() => ntOpen(row.url)} onpointerenter={() => (ntSel = i)}>
                <span class="nt-title">{#if row.starred}<span class="nt-star"><StarIcon filled size={11} /></span>{/if}{row.title || prettyUrl(row.url)}</span>
                <span class="nt-url">{prettyUrl(row.url)}</span>
              </button>
            {:else}
              <div class="nt-empty">{t(addrValue.trim() ? "browserPanel.noMatch" : "browserPanel.newTabEmpty")}</div>
            {/each}
          </div>
        </div>
      {:else if browser.veiled}
        <div class="veil">
          <div class="veil-title">{cur?.title || domainOf(url)}</div>
          <div class="veil-hint">{domainOf(url)}</div>
        </div>
      {/if}
    </div>
  {/if}
</div>

<style>
  .bpanel {
    flex: none;
    display: flex;
    flex-direction: column;
    /* Docked, the inline min-width (the store's PANEL_MIN) wins over the max-width when the
      window is too narrow for the panel and the main area's minimum together: the column
      keeps its floor — a column squeezed to nothing would leave Rust's native view (which
      has the same floor) over the window's edge with no chrome to close it */
    min-width: 0;
    height: 100%;
    background: var(--bg-panel);
    border-left: 1px solid var(--border-strong);
    color: var(--fg);
    -webkit-user-select: none;
    user-select: none;
    box-sizing: border-box;
  }
  .bpanel.bar {
    height: 100vh;
    border-left: none;
  }
  .chrome {
    flex: none;
    display: flex;
    flex-direction: column;
    position: relative;
  }
  .tabs,
  .nav {
    height: 36px;
    display: flex;
    align-items: center;
    gap: 3px;
    padding: 0 6px;
    min-width: 0;
    box-sizing: border-box;
  }
  .tabs {
    overflow-x: auto;
    overflow-y: hidden;
    scrollbar-width: none;
  }
  .tabs::-webkit-scrollbar {
    display: none;
  }
  /* The traffic lights float over the bar's left end in the detached window */
  .bar .tabs {
    padding-left: 76px;
  }
  .nav {
    border-top: 1px solid var(--border);
    border-bottom: 1px solid var(--border);
  }
  .spacer {
    flex: 1;
    align-self: stretch;
  }
  .tab {
    flex: 0 1 180px;
    min-width: 72px;
    height: 26px;
    display: flex;
    align-items: center;
    gap: 4px;
    padding: 0 4px 0 9px;
    border-radius: 6px;
    color: var(--fg-muted);
    font-size: 12px;
    cursor: default;
    box-sizing: border-box;
  }
  .tab:hover {
    background: var(--bg-hover);
  }
  .tab.active {
    background: var(--bg);
    color: var(--fg);
    box-shadow: inset 0 0 0 1px var(--border);
  }
  .tab-title {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .tab-close {
    flex: none;
    width: 18px;
    height: 18px;
    display: grid;
    place-items: center;
    border: none;
    background: none;
    color: var(--fg-faint);
    border-radius: 4px;
    cursor: pointer;
    visibility: hidden;
  }
  .tab:hover .tab-close,
  .tab.active .tab-close {
    visibility: visible;
  }
  .tab-close:hover {
    background: var(--bg-active);
    color: var(--fg);
  }
  .ico {
    flex: none;
    width: 26px;
    height: 26px;
    display: grid;
    place-items: center;
    border: none;
    background: none;
    color: var(--fg-muted);
    border-radius: 6px;
    cursor: pointer;
    padding: 0;
  }
  .ico:hover:not(:disabled) {
    background: var(--bg-hover);
    color: var(--fg);
  }
  .ico:disabled {
    opacity: 0.35;
    cursor: default;
  }
  .ico.star.starred,
  .ico.clipped {
    color: var(--accent);
  }
  .ico.clipped {
    background: var(--accent-soft);
    font-size: 12px;
  }
  .addr {
    flex: 1 1 60px;
    min-width: 40px;
    height: 26px;
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 0 8px;
    border: 1px solid var(--border);
    border-radius: 6px;
    background: var(--bg);
    color: var(--fg);
    font-size: 12px;
    cursor: text;
    text-align: left;
    overflow: hidden;
    box-sizing: border-box;
  }
  .addr:hover {
    border-color: var(--border-strong);
  }
  .addr-host {
    flex: none;
    max-width: 55%;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .addr-title {
    flex: 1;
    min-width: 0;
    color: var(--fg-muted);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .addr-input {
    flex: 1 1 60px;
    min-width: 40px;
    height: 26px;
    font-size: 12px;
    padding: 0 8px;
    border: 1px solid var(--accent);
    border-radius: 6px;
    background: var(--bg);
    color: var(--fg);
    outline: none;
    box-sizing: border-box;
  }
  .act {
    flex: none;
    height: 26px;
    border: 1px solid var(--border);
    background: var(--bg);
    color: var(--fg);
    font-size: 12px;
    padding: 0 8px;
    border-radius: 6px;
    cursor: pointer;
    white-space: nowrap;
    max-width: 180px;
    overflow: hidden;
    text-overflow: ellipsis;
    box-sizing: border-box;
  }
  .act:hover:not(:disabled) {
    background: var(--bg-hover);
  }
  .act:disabled {
    opacity: 0.45;
    cursor: default;
  }
  .clipgroup {
    flex: 0 1 auto;
    min-width: 0;
    display: inline-flex;
  }
  .act.primary {
    color: var(--accent);
    border-color: color-mix(in srgb, var(--accent) 45%, var(--border));
    border-radius: 6px 0 0 6px;
    min-width: 0;
  }
  .act.primary.caret {
    border-radius: 0 6px 6px 0;
    border-left: none;
    padding: 0 5px;
  }
  .flash {
    position: absolute;
    left: 8px;
    right: 8px;
    bottom: 2px;
    pointer-events: none;
    font-size: 11px;
    color: var(--fg-muted);
    background: var(--bg-panel);
    text-align: center;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .flash.err {
    color: var(--danger, #d05252);
  }

  /* ---- the page slot (docked) ---- */
  .page-slot {
    flex: 1;
    min-height: 0;
    position: relative;
    background: var(--bg);
    display: flex;
    align-items: center;
    justify-content: center;
  }
  .page-slot.newtab {
    align-items: stretch;
    justify-content: stretch;
  }
  /* The new-tab page: the lists under the address row (which is its input), the same row language as the palette's */
  .nt {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    padding: 6px 12px 8px;
  }
  .nt-list {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
  }
  .nt-head {
    font-size: var(--fs-xs);
    color: var(--fg-faint);
    padding: 10px 8px 4px;
  }
  .nt-row {
    width: 100%;
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    gap: 1px;
    text-align: left;
    padding: 6px 8px;
    border: none;
    background: none;
    color: var(--fg);
    border-radius: 6px;
    cursor: pointer;
    min-width: 0;
  }
  .nt-row.sel {
    background: var(--accent-soft);
  }
  .nt-title,
  .nt-url {
    max-width: 100%;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .nt-title {
    font-size: var(--fs-sm);
    display: inline-flex;
    align-items: center;
    gap: 6px;
  }
  .nt-star {
    color: var(--accent);
    display: inline-flex;
  }
  .nt-url {
    font-size: var(--fs-xs);
    color: var(--fg-muted);
  }
  .nt-empty {
    color: var(--fg-faint);
    font-size: var(--fs-sm);
    padding: 12px 8px;
  }
  .veil {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 4px;
    color: var(--fg-faint);
    padding: 24px;
    text-align: center;
    max-width: 80%;
  }
  .veil-title {
    color: var(--fg-muted);
    font-size: 13px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    max-width: 100%;
  }
  .veil-hint {
    font-size: 11.5px;
  }

  /* ---- the expanded destination picker (bar) ---- */
  .picker {
    flex: 1;
    min-height: 0;
    display: flex;
    flex-direction: column;
    padding: 8px 8px 8px;
    gap: 6px;
    border-top: 1px solid var(--border);
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
  .act.on {
    color: var(--accent);
    border-color: var(--accent);
  }
  .tr-count {
    margin-left: 4px;
    font-size: 10px;
    opacity: 0.8;
    font-variant-numeric: tabular-nums;
  }
</style>
