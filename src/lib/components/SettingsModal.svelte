<script lang="ts">
  import { setContext, tick, untrack } from "svelte";
  import { t, tn } from "$lib/i18n/i18n.svelte";
  import { PAGES, pageAvailable, pageLabel, searchSettings, settingAvailable, type SettingsPage } from "$lib/settings";
  import { currentSettingsState, settingsUi } from "$lib/stores/settingsUi.svelte";
  import { vault } from "$lib/stores/vault.svelte";
  import type { SearchContext } from "./settings/context";
  import { form } from "./settings/form.svelte";
  import AccountPage from "./settings/AccountPage.svelte";
  import AiPage from "./settings/AiPage.svelte";
  import BrowserPage from "./settings/BrowserPage.svelte";
  import CapturePage from "./settings/CapturePage.svelte";
  import EditingPage from "./settings/EditingPage.svelte";
  import GeneralPage from "./settings/GeneralPage.svelte";
  import PrivacyPage from "./settings/PrivacyPage.svelte";
  import SkillsPage from "./settings/SkillsPage.svelte";

  // The settings dialog: the pages on the left under a search field, one page on the right.
  // Every control writes the moment it changes (settings/form.svelte.ts), so there is no
  // Save and no Cancel — Escape, a click outside and the close button all just close.
  const PAGE_COMPONENTS = {
    general: GeneralPage,
    editing: EditingPage,
    ai: AiPage,
    account: AccountPage,
    browser: BrowserPage,
    capture: CapturePage,
    privacy: PrivacyPage,
    skills: SkillsPage,
  } as const;

  let query = $state("");
  const q = $derived(query.trim());
  let searchEl = $state<HTMLInputElement | null>(null);
  let panelEl = $state<HTMLDivElement | null>(null);
  const hits = $derived(q ? searchSettings(q).filter((s) => settingAvailable(s, currentSettingsState())) : []);
  const hitIds = $derived(new Set(hits.map((h) => h.id)));
  const hitPages = $derived(PAGES.filter((p) => hits.some((h) => h.page === p.id)));

  setContext<SearchContext>("settings-search", {
    get query() {
      return q;
    },
    matches: (id) => hitIds.has(id),
  });

  // Each opening reads the settings afresh and puts the cursor in the search field. Only
  // the opening: were the page or the vault a dependency, every page switch would read the
  // settings again and pull the focus back to the search field
  $effect(() => {
    if (!settingsUi.open) return;
    untrack(() => {
      query = "";
      void form.load();
      if (!pageAvailable(settingsUi.page, !!vault.root)) settingsUi.page = "general";
    });
    queueMicrotask(() => searchEl?.focus());
  });

  // The setting an entry point (or a search result's "go there") asked for: the search
  // gives way to its page, and the row is scrolled to the middle and flashed once that
  // page (and the settings it needs) has rendered
  $effect(() => {
    const id = settingsUi.focus;
    if (!id || !settingsUi.open || !form.s) return;
    query = "";
    void settingsUi.page;
    void tick().then(() => {
      const row = panelEl?.querySelector<HTMLElement>(`[data-setting="${id}"]`);
      settingsUi.focus = null;
      if (!row) return;
      row.scrollIntoView({ block: "center" });
      row.classList.add("flash");
      setTimeout(() => row.classList.remove("flash"), 1400);
    });
  });

  function go(page: SettingsPage) {
    query = "";
    settingsUi.page = page;
    panelEl?.scrollTo({ top: 0 });
  }

  /** Close in any way: a text field still holding the focus is blurred first, so what was
   *  typed into it is written like any other edit. */
  function close() {
    (document.activeElement as HTMLElement | null)?.blur?.();
    settingsUi.open = false;
  }

  /** Only a press and a release on the overlay count as a click outside: a drag that selects
   *  text in a field and ends outside the dialog lands its click on the overlay too. */
  let pressedOverlay = false;

  function onWindowKey(e: KeyboardEvent) {
    if (!settingsUi.open) return;
    if (e.key === "Escape") {
      // Escape in the search field with a query clears the query first
      if (query && document.activeElement === searchEl) {
        query = "";
        return;
      }
      close();
    } else if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === "f") {
      e.preventDefault();
      searchEl?.focus();
      searchEl?.select();
    }
  }
</script>

<!-- Window level: after a click on the overlay the focus lands on body, and a keydown inside the dialog would no longer receive Escape -->
<svelte:window onkeydown={onWindowKey} />

{#if settingsUi.open}
  <div
    class="overlay"
    onmousedown={(e) => (pressedOverlay = e.target === e.currentTarget)}
    onclick={() => pressedOverlay && close()}
    role="presentation"
  >
    <div class="dialog" role="dialog" aria-label={t("settings.title")} tabindex="-1">
      <header>
        <h2>{t("settings.title")}</h2>
        <button class="close" aria-label={t("common.close")} title={t("common.close")} onclick={close}>✕</button>
      </header>
      <div class="body">
        <nav class="side" aria-label={t("settings.categories")}>
          <label class="find" class:typed={!!query}>
            <svg width="13" height="13" viewBox="0 0 20 20" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" aria-hidden="true"
              ><path d="M8.75 4a4.75 4.75 0 1 0 0 9.5a4.75 4.75 0 1 0 0-9.5M12.1 12.1l4.4 4.4" /></svg
            >
            <input bind:this={searchEl} bind:value={query} placeholder={t("settings.search")} aria-label={t("settings.search")} spellcheck="false" />
          </label>
          {#each PAGES as p (p.id)}
            <button
              class="side-item"
              class:active={!query && settingsUi.page === p.id}
              disabled={!pageAvailable(p.id, !!vault.root)}
              title={pageAvailable(p.id, !!vault.root) ? undefined : t("settings.needsVault")}
              aria-current={!query && settingsUi.page === p.id ? "true" : undefined}
              onclick={() => go(p.id)}
            >{pageLabel(p.id)}</button>
          {/each}
        </nav>
        <div class="panel" bind:this={panelEl}>
          {#if !form.s}
            <!-- the first read is a local file: this shows for a frame at most -->
          {:else if q}
            {#if hits.length}
              <p class="count">{tn("settings.searchCount", hits.length)}</p>
              {#each hitPages as p (p.id)}
                {@const Page = PAGE_COMPONENTS[p.id]}
                <div class="result-page">
                  <button class="where" onclick={() => go(p.id)}>{pageLabel(p.id)} ›</button>
                  <Page />
                </div>
              {/each}
            {:else}
              <p class="count">{t("settings.searchEmpty", { q })}</p>
            {/if}
          {:else}
            {@const Page = PAGE_COMPONENTS[settingsUi.page]}
            <h3 class="page-title">{pageLabel(settingsUi.page)}</h3>
            {#key settingsUi.page}
              <Page />
            {/key}
          {/if}
        </div>
      </div>
    </div>
  </div>
{/if}

<style>
  .overlay {
    position: fixed;
    inset: 0;
    background: var(--scrim);
    display: grid;
    place-items: center;
    z-index: 150;
  }
  /* A fixed height rather than a max-height: the dialog does not jump when the page
     changes. overflow:hidden so the panel is the one scrolling surface. */
  .dialog {
    width: min(720px, 92vw);
    height: min(600px, 80vh);
    background: var(--bg-panel);
    border: 1px solid var(--border);
    border-radius: 10px;
    box-shadow: var(--shadow);
    display: flex;
    flex-direction: column;
    overflow: hidden;
  }
  header {
    flex-shrink: 0;
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 12px 14px 10px 20px;
    border-bottom: 1px solid var(--border);
  }
  h2 {
    font-size: var(--fs-lg);
    font-weight: 600;
  }
  .close {
    width: 26px;
    height: 26px;
    display: grid;
    place-items: center;
    border-radius: 6px;
    color: var(--fg-muted);
  }
  .close:hover {
    background: var(--bg-hover);
    color: var(--fg);
  }
  .body {
    flex: 1;
    display: flex;
    min-height: 0;
  }
  .side {
    flex: 0 0 152px;
    display: flex;
    flex-direction: column;
    gap: 2px;
    padding: 10px 8px;
    border-right: 1px solid var(--border);
    overflow-y: auto;
  }
  .find {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 0 8px;
    margin-bottom: 8px;
    border: 1px solid var(--border);
    border-radius: 6px;
    background: var(--bg);
    color: var(--fg-muted);
  }
  .find:focus-within {
    border-color: var(--accent);
  }
  .find svg {
    flex: none;
  }
  .find input {
    flex: 1;
    min-width: 0;
    border: none;
    background: transparent;
    padding: 5px 0;
    font-size: var(--fs-sm);
    outline: none;
  }
  .side-item {
    text-align: left;
    font-size: var(--fs-sm);
    color: var(--fg-muted);
    padding: 5px 10px;
    border-radius: var(--radius);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .side-item:hover:not(:disabled) {
    background: var(--bg-hover);
    color: var(--fg);
  }
  /* The current page, drawn the way "current" is drawn across the app: the active ground
     plus a 2px accent bar at the start of the row, the label in the body ink */
  .side-item.active {
    background: var(--bg-active);
    color: var(--fg);
    font-weight: 500;
    border-top-left-radius: 0;
    border-bottom-left-radius: 0;
    box-shadow: inset 2px 0 0 var(--accent);
  }
  .side-item:disabled {
    color: var(--fg-faint);
    cursor: default;
  }
  .panel {
    flex: 1;
    min-width: 0;
    overflow-y: auto;
    padding: 14px 22px 24px;
  }
  .page-title {
    font-size: 15px;
    font-weight: 600;
    margin-bottom: 8px;
  }
  .count {
    font-size: var(--fs-xs);
    color: var(--fg-muted);
    margin-bottom: 6px;
  }
  .result-page + .result-page {
    margin-top: 18px;
  }
  .where {
    font-size: var(--fs-sm);
    font-weight: 600;
    color: var(--fg);
    padding: 0;
    margin-bottom: 2px;
  }
  .where:hover {
    color: var(--accent);
  }
  /* The buttons the pages share */
  .panel :global(.btn) {
    font-size: var(--fs-sm);
    padding: 5px 14px;
    border-radius: 6px;
    border: 1px solid var(--border);
    white-space: nowrap;
  }
  .panel :global(.btn.small) {
    font-size: var(--fs-xs);
    padding: 3px 10px;
  }
  .panel :global(.btn:disabled) {
    opacity: 0.5;
    cursor: default;
  }
  .panel :global(.link) {
    color: var(--accent);
    font-size: var(--fs-xs);
    padding: 0;
    white-space: nowrap;
  }
  /* The small print and the "type a name, then add" line the list pages share */
  .panel :global(small) {
    font-size: var(--fs-xs);
    color: var(--fg-muted);
  }
  .panel :global(.add) {
    display: flex;
    gap: 6px;
    margin-top: 6px;
  }
  .panel :global(.add input) {
    flex: 1;
  }
</style>
