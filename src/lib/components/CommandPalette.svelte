<script lang="ts">
  import { api, type HistoryEntryView, type SearchHit } from "$lib/api";
  import { allCommands, commandDescription, commandTitle, executeCommand, fuzzyFilter, groupLabel, groupOf, type Command } from "$lib/commands/registry";
  import { t } from "$lib/i18n/i18n.svelte";
  import { browser, revealPanelIfDocked } from "$lib/stores/browser.svelte";
  import { fetchHeadings, noteKey, resolveOrCreateNote, type PickHeading } from "$lib/pickNote";
  import { vault } from "$lib/stores/vault.svelte";
  import { scripts } from "$lib/stores/scripts.svelte";
  import { toast } from "$lib/stores/toast.svelte";
  import { isImeKeydown } from "$lib/ime";
  import { k } from "$lib/shortcut";

  export type PaletteMode = "commands" | "notes" | "pick-note" | "search" | "capture" | "vaults" | "url";

  /** A URL's display and filter form: the scheme is dropped and the percent encoding decoded
   *  (so a non-ASCII path is readable and can be matched by a query in that script); a failed
   *  decode (a malformed sequence) is kept as written. Opening still uses the raw url field. */
  function prettyUrl(u: string): string {
    const s = u.replace(/^https?:\/\//, "");
    try {
      return decodeURI(s);
    } catch {
      return s;
    }
  }

  interface Item {
    id: string;
    title: string;
    hint?: string;
    /** The shortcut, right-aligned on the same line as the title. It is its own field rather
     *  than sharing one with hint: a command with both a shortcut and a description would
     *  necessarily lose one of them in a single slot (the old `hotkey ?? description` lost the
     *  description), and they answer different questions — "what does this do" and "how do I
     *  press it next time without opening the palette". */
    key?: string;
    /** Command mode with an empty query: this item is the first of its group, and the group header is rendered before it. */
    groupLabel?: string;
    /** The note row of pick-note's first stage: Tab enters the section selection from it. */
    pick?: { path: string; title: string };
    /** A bookmark or history row in url mode: return opens that address (a special case in submit, not going through run). */
    url?: string;
    /** A secondary action at the end of the row (a drill-style button that does not close the palette): unbookmark or delete from the history. */
    action?: { label: string; run: () => void };
    /** Absent = this row has no "return runs it here" action (in url mode both opening and
     *  selecting live in submit's url special case, which has to check docked and the
     *  new-window modifier). */
    run?: () => void | Promise<void>;
  }

  let {
    mode = $bindable<PaletteMode | null>(null),
    onPickNote,
    onCaptureToNote,
  }: {
    mode: PaletteMode | null;
    /** pick-note mode: the chosen (or newly created) note's path comes back through the
     *  callback rather than being opened; heading is the destination section chosen in the
     *  second stage (absent or null = the end of the note). */
    onPickNote?: (path: string, heading?: string | null) => void;
    /** capture mode's ⌘Enter: the text is handed to the caller to take into pick-note and choose a destination. */
    onCaptureToNote?: (text: string) => void;
  } = $props();

  let query = $state("");
  let selected = $state(0);
  let searchHits = $state<SearchHit[]>([]);
  /** The browsing history snapshot fetched once when url mode opens (bookmarks come from browser.bookmarks' permanent mirror). */
  let urlHistory = $state<HistoryEntryView[]>([]);
  let inputEl = $state<HTMLInputElement | null>(null);
  let listEl = $state<HTMLUListElement | null>(null);
  /** pick-note's second stage: a note is chosen and a destination section is being picked (the query becomes the section filter). */
  let pickStage = $state<{ path: string; title: string } | null>(null);
  let pickHeadings = $state<PickHeading[]>([]);

  const placeholders: Record<PaletteMode, string> = {
    commands: t("palette.commands"),
    notes: t("palette.notes"),
    "pick-note": t("palette.pickNote"),
    search: t("palette.search"),
    capture: t("palette.capture", { key: k("Mod+Enter") }),
    vaults: t("palette.vaults"),
    url: t("palette.url", { key: k("Mod+Enter") }),
  };
  const placeholder = $derived(
    mode === "pick-note" && pickStage
      ? t("palette.pickHeading", { title: pickStage.title })
      : mode
        ? placeholders[mode]
        : "",
  );

  $effect(() => {
    if (mode) {
      query = "";
      selected = 0;
      searchHits = [];
      pickStage = null;
      queueMicrotask(() => inputEl?.focus());
    }
    if (mode === "url") {
      // Fetching once when it opens is enough: the history only changes on a navigation, and there is no need to keep up while the palette is open
      api
        .browserHistoryList()
        .then((l) => (urlHistory = l))
        .catch(() => (urlHistory = []));
    }
  });

  // url mode's DNS prefetch: 150ms after the input pauses or the highlight moves, the target
  // host's resolution is pushed into the system cache early, so WebKit skips the DNS step on
  // return (the same thing an address bar does).
  // A half-typed input (with no dot) is stopped on the Rust side and never leaks; a failure is
  // silent — a prefetch has no visible consequence.
  $effect(() => {
    if (mode !== "url") return;
    const target = items[selected]?.url ?? query.trim();
    if (!target) return;
    const t = setTimeout(() => void api.browserPrefetch(target).catch(() => {}), 150);
    return () => clearTimeout(t);
  });

  /** url mode's candidate pool: bookmarks first plus the history, deduplicated by url (first
   *  seen wins = bookmarks take precedence), with the filter key built and lowercased ahead of
   *  time. It depends on the two mirrors alone and not on query — putting it inside items
   *  would decodeURI and rebuild up to 500 strings on every keystroke. */
  const urlCandidates = $derived.by(() => {
    const seen = new Set<string>();
    return [
      ...browser.bookmarks.map((b) => ({ url: b.url, title: b.title, starred: true })),
      ...urlHistory.map((h) => ({ url: h.url, title: h.title, starred: false })),
    ]
      .filter((c) => (seen.has(c.url) ? false : (seen.add(c.url), true)))
      .map((c) => {
        const pretty = prettyUrl(c.url); // decoded once per entry (the reason this derived exists)
        return { ...c, pretty, key: `${c.title} ${pretty}`.toLowerCase() };
      });
  });

  // The second stage's heading list is another projection of query(); it opens no new read path
  $effect(() => {
    if (mode === "pick-note" && pickStage) {
      const path = pickStage.path;
      void fetchHeadings(path).then((hs) => {
        if (pickStage?.path === path) pickHeadings = hs;
      });
    } else {
      pickHeadings = [];
    }
  });

  // Typing `#` in the query: the same mental model as [[Note#Heading]] — before the # it
  // filters notes and takes the highlighted row into the second stage (the same meaning as Tab:
  // a choice moved to with ↓ has to be honoured rather than always biting the first hit), and
  // the text after the # comes along as the section filter; with no hit it is left as written
  // (showing no results). items cannot be read here (the $derived evaluates lazily against the
  // new query containing the #), so the same filtering is replayed with the # prefix, where
  // selected is exactly the index the user highlighted in the old list.
  $effect(() => {
    if (mode !== "pick-note" || pickStage || !query.includes("#")) return;
    const hash = query.indexOf("#");
    const rest = query.slice(hash + 1);
    const cands = fuzzyFilter(vault.notes, query.slice(0, hash), noteKey);
    const cand = cands[selected] ?? cands[0];
    if (cand) {
      pickStage = { path: cand.id, title: cand.title };
      query = rest;
      selected = 0;
    }
  });

  function exitPickStage() {
    pickStage = null;
    query = "";
    selected = 0;
  }

  // The one way into the second stage: shared by Tab and the "choose a section" button at the
  // end of a row. A click takes the focus onto the button, and it has to be pulled back to the
  // input to keep filtering sections from the keyboard
  function enterPickStage(p: { path: string; title: string }) {
    pickStage = p;
    query = "";
    selected = 0;
    queueMicrotask(() => inputEl?.focus());
  }

  $effect(() => {
    if (mode === "search" && query.trim()) {
      const q = query;
      api.search(q, 30).then((hits) => {
        if (q === query) searchHits = hits;
      });
    } else {
      searchHits = [];
    }
  });

  let items = $derived.by((): Item[] => {
    if (mode === "commands") {
      // The registry itself is a plain Map and is not reactive; what a script hot reload changes
      // is scripts.infos, so depending on it explicitly lets an open palette see added and
      // removed script commands immediately
      void scripts.infos;
      const cmds = fuzzyFilter(allCommands(), query, (c: Command) => `${commandTitle(c)} ${commandDescription(c) ?? ""} ${c.id}`);
      // An empty query keeps the registry's group order and marks the group headers; with a query it flattens by fuzzy score and does not group
      const grouped = !query.trim();
      return cmds.map((c, i) => {
        const g = groupOf(c);
        // Already sorted by group order, so differing from the previous one makes it a group header
        const group = grouped && (i === 0 || g !== groupOf(cmds[i - 1])) ? groupLabel(g) : undefined;
        return {
          id: c.id,
          title: commandTitle(c),
          hint: commandDescription(c),
          key: c.hotkey,
          groupLabel: group,
          run: () => c.run(),
        };
      });
    }
    if (mode === "pick-note" && pickStage) {
      // The second stage: that note's section list, with "the end of the note" fixed as the first item (Enter reaches it with no friction)
      const stage = pickStage;
      return [
        { id: "__whole__", title: t("palette.wholeNote"), hint: stage.path, run: () => onPickNote?.(stage.path, null) },
        ...fuzzyFilter(pickHeadings, query, (h) => h.text).map((h, i) => ({
          id: `${i}:${h.text}`,
          title: `${"#".repeat(h.level)} ${h.text}`,
          hint: t("palette.appendToSection"),
          run: () => onPickNote?.(stage.path, h.text),
        })),
      ];
    }
    if (mode === "notes" || mode === "pick-note") {
      // pick-note and notes share the filtering and the create row; only "what happens after choosing" differs
      const pick = mode === "pick-note" ? onPickNote : undefined;
      const notes = fuzzyFilter(vault.notes, query, noteKey);
      const list: Item[] = notes.slice(0, 50).map((n) => ({
        id: n.id,
        title: n.title,
        hint: n.id,
        pick: pick ? { path: n.id, title: n.title } : undefined,
        run: () => (pick ? pick(n.id) : vault.openNote(n.id)),
      }));
      if (query.trim() && !notes.some((n) => n.title.toLowerCase() === query.trim().toLowerCase())) {
        const title = query.trim();
        list.push({
          id: "__create__",
          title: t("palette.createNamed", { title }),
          hint: pick ? t("palette.createAndPick") : t("palette.createAndOpen"),
          // Both branches create the file through Rust's create_note: the file-name sanitising
          // and same-name deduplication live in the vault engine alone, and TS does not mirror
          // the sanitising rules. The pick branch recognises an existing file of that name first
          // — a note whose title has come apart from its file name cannot be caught by the
          // suppression above, and noteCreate's deduplication would conjure up a "title 1.md";
          // clipping into an existing note keeps the append semantics
          run: async () => {
            if (!pick) return void vault.createAndOpen(title);
            try {
              pick(await resolveOrCreateNote(title));
            } catch (e) {
              toast.push(String(e), "error");
            }
          },
        });
      }
      return list;
    }
    if (mode === "vaults") {
      const vaults = fuzzyFilter(vault.recent, query, (p) => p);
      const list: Item[] = vaults.map((p) => ({
        id: p,
        title: p.split("/").pop() ?? p,
        hint: p === vault.root ? t("palette.currentVault", { path: p }) : p,
        run: () => {
          if (p !== vault.root) void vault.openVault(p);
        },
      }));
      list.push(
        {
          id: "__pick__",
          title: t("palette.openOtherVault"),
          hint: t("palette.pickFolder"),
          run: () => executeCommand("open-vault"),
        },
        {
          id: "__create__",
          title: t("cmd.create-vault.title"),
          hint: t("palette.pickLocation"),
          run: () => executeCommand("create-vault"),
        },
      );
      return list;
    }
    if (mode === "search") {
      return searchHits.map((h, i) => ({
        id: `${h.note}:${i}`,
        title: h.title,
        hint: h.snippet,
        run: () => vault.openNote(h.note),
      }));
    }
    if (mode === "url") {
      // Both opening and selecting live in submit's url special case (the docked check and the
      // new-window modifier), so no row in this section carries a run.
      const unstar = (url: string) => ({
        label: t("palette.unstar"),
        run: () => void api.bookmarkRemove(url).catch((e) => toast.push(String(e), "error")),
      });
      const removeHistory = (url: string) => ({
        label: t("common.delete"),
        run: () => {
          // The local snapshot drops it optimistically (the history has no event mirror), and a failure only means it reappears the next time it is opened
          urlHistory = urlHistory.filter((h) => h.url !== url);
          void api.browserHistoryRemove(url).catch((e) => toast.push(String(e), "error"));
        },
      });
      const q = query.trim();
      if (!q) {
        const list: Item[] = [];
        // Row 0: summon it directly — with a window it returns to it, without one it reopens the last page (the single open-browser implementation)
        if (browser.windows.length > 0) {
          const cur = browser.current;
          list.push({
            id: "__last__",
            title: t("palette.backToBrowser"),
            hint: cur ? cur.title || prettyUrl(cur.url) : undefined,
          });
        } else if (urlHistory.length > 0) {
          const last = urlHistory[0];
          list.push({
            id: "__last__",
            title: t("palette.reopenLast"),
            hint: last.title || prettyUrl(last.url),
          });
        }
        browser.bookmarks.slice(0, 8).forEach((b, i) => {
          list.push({
            id: `bm:${b.url}`,
            title: b.title || prettyUrl(b.url),
            hint: prettyUrl(b.url),
            groupLabel: i === 0 ? t("palette.bookmarks") : undefined,
            url: b.url,
            action: unstar(b.url),
          });
        });
        // When row 0 is already "reopen the last page" (= history[0]), the recent section skips it rather than repeating it
        const skipFirst = browser.windows.length === 0 && urlHistory.length > 0 ? 1 : 0;
        urlHistory.slice(skipFirst, skipFirst + 12).forEach((h, i) => {
          list.push({
            id: `h:${h.url}`,
            title: h.title || prettyUrl(h.url),
            hint: prettyUrl(h.url),
            groupLabel: i === 0 ? t("palette.recent") : undefined,
            url: h.url,
            action: removeHistory(h.url),
          });
        });
        return list;
      }
      // With a query: the direct row keeps the "what you type is a URL" mental model, and the
      // candidate pool is fuzzy-filtered after it (the filter key holds the decoded URL, so a
      // non-ASCII path can be matched by a query in that script)
      const list: Item[] = [
        { id: "__open__", title: t("palette.openUrl", { url: q }), hint: t("palette.builtInBrowser"), url: q },
      ];
      for (const c of fuzzyFilter(urlCandidates, q, (c) => c.key).slice(0, 30)) {
        list.push({
          id: `${c.starred ? "bm" : "h"}:${c.url}`,
          title: c.starred ? `★ ${c.title || c.pretty}` : c.title || c.pretty,
          hint: c.pretty,
          url: c.url,
          action: c.starred ? unstar(c.url) : removeHistory(c.url),
        });
      }
      return list;
    }
    return [];
  });

  $effect(() => {
    if (selected >= items.length) selected = Math.max(0, items.length - 1);
  });

  // Keyboard navigation in a long list (the history, the notes): the selected row scrolls into
  // view. The effect runs after the DOM update and reads the class directly; nearest does not
  // jump a page, and a selection set by hover is already in view, so scrollIntoView is a no-op
  // and nothing jitters
  $effect(() => {
    void selected;
    void items;
    listEl?.querySelector("li.selected")?.scrollIntoView({ block: "nearest" });
  });

  function close() {
    mode = null;
  }

  /** url mode's fixed footer "clear the browsing history": once the history is long, the last
   *  rows take a lot of scrolling to reach — a fixed footer stays out of the ↑↓ navigation and
   *  is always in sight. The palette stays open to show the result (the bookmarks remain and
   *  the recents are empty), and it is the same implementation as the ⌘P command (whose toast
   *  receipt lives in the command). */
  function clearHistory() {
    urlHistory = [];
    void executeCommand("clear-browser-history");
    inputEl?.focus();
  }

  async function submit(modifier = false) {
    if (mode === "capture") {
      const text = query.trim();
      if (!text) return close();
      // ⌘Enter: take the text first and then raise pick-note to choose a destination (the same "take, then choose" as clipping)
      if (modifier && onCaptureToNote) {
        close();
        onCaptureToNote(text);
        return;
      }
      // It lands before the palette closes: on failure the palette and the text stay for a retry, so a memo is never lost silently
      try {
        await vault.capture(text);
        close();
      } catch (e) {
        toast.push(String(e), "error");
      }
      return;
    }
    if (mode === "url") {
      const it = items[selected];
      close();
      // Row 0 is the special row: summoning directly goes through the single open-browser implementation (focus / reopen the last page / a message)
      if (it?.id === "__last__") {
        void executeCommand("open-browser");
        return;
      }
      const url = it?.url ?? query.trim();
      if (url) {
        // Send the open first: the slow path (building or claiming a window plus navigating)
        // must not queue behind the settingsGet round trip
        const opening = api.browserOpen(url, modifier).catch((e) => toast.push(String(e), "error"));
        // A docked window's navigation and clip panel are in the main window, so they are brought out along with it (the same check as ⌘P's "open the browser")
        await revealPanelIfDocked(modifier ? null : browser.current?.label);
        await opening;
      }
      return;
    }
    const item = items[selected];
    if (item?.run) {
      close();
      await item.run();
    }
  }

  function onkeydown(e: KeyboardEvent) {
    if (e.key === "Escape") {
      e.preventDefault();
      // Esc in the second stage means "back to choosing a note" rather than closing the palette
      if (pickStage) exitPickStage();
      else close();
    } else if (e.key === "Tab" && mode === "pick-note" && !pickStage) {
      const p = items[selected]?.pick;
      if (p) {
        e.preventDefault();
        enterPickStage(p);
      }
    } else if (e.key === "Backspace" && pickStage && query === "") {
      e.preventDefault();
      exitPickStage();
    } else if (e.key === "ArrowDown") {
      e.preventDefault();
      selected = Math.min(selected + 1, items.length - 1);
    } else if (e.key === "ArrowUp") {
      e.preventDefault();
      selected = Math.max(selected - 1, 0);
    } else if (e.key === "Enter") {
      if (isImeKeydown(e)) return; // searching commands or writing a memo through an input method: confirming a candidate is not running it
      e.preventDefault();
      void submit(e.metaKey || e.ctrlKey);
    }
  }
</script>

{#if mode}
  <div class="overlay" onclick={close} role="presentation">
    <div
      class="palette"
      onclick={(e) => e.stopPropagation()}
      onkeydown={(e) => e.stopPropagation()}
      role="dialog"
      aria-label={t("cmd.command-palette.title")}
      tabindex="-1"
    >
      <input
        bind:this={inputEl}
        bind:value={query}
        {placeholder}
        {onkeydown}
        spellcheck="false"
      />
      {#if mode !== "capture"}
        <ul bind:this={listEl}>
          {#each items as item, i (item.id)}
            {#if item.groupLabel}
              <li class="group-head" aria-hidden="true">{item.groupLabel}</li>
            {/if}
            <li class:selected={i === selected}>
              <button
                class="main"
                onclick={() => void submit()}
                onmouseenter={() => (selected = i)}
              >
                <span class="line">
                  <span class="title">{item.title}</span>
                  {#if item.key}<kbd class="key">{item.key}</kbd>{/if}
                </span>
                {#if item.hint}<span class="hint">{item.hint}</span>{/if}
              </button>
              {#if item.pick}
                {@const p = item.pick}
                <!-- The mouse equivalent of the keyboard's Tab: clicking the row clips the whole note, and clicking here enters the second stage to pick a section -->
                <button
                  class="drill"
                  onclick={() => enterPickStage(p)}
                  onmouseenter={() => (selected = i)}
                  aria-label={t("palette.pickHeadingLabel", { title: p.title })}
                >
                  {t("palette.pickSection")}
                </button>
              {/if}
              {#if item.action}
                {@const a = item.action}
                <!-- The row's secondary action (url mode's "unbookmark"): it does not close the palette, and the list refreshes with the mirror -->
                <button class="drill" onclick={a.run} onmouseenter={() => (selected = i)}>
                  {a.label}
                </button>
              {/if}
            </li>
          {:else}
            <li class="empty">
              {query ? t("palette.noMatch") : mode === "url" ? t("palette.noBookmarksOrHistory") : t("palette.typeToFilter")}
            </li>
          {/each}
        </ul>
      {/if}
      {#if mode === "url" && urlHistory.length > 0}
        <div class="foot">
          <button class="foot-act" title={t("palette.clearHistoryHint")} onclick={clearHistory}>
            {t("cmd.clear-browser-history.title")}
          </button>
        </div>
      {/if}
    </div>
  </div>
{/if}

<style>
  .overlay {
    position: fixed;
    inset: 0;
    background: rgba(0, 0, 0, 0.25);
    display: flex;
    justify-content: center;
    padding-top: 12vh;
    z-index: 100;
  }
  .palette {
    width: min(560px, 90vw);
    max-height: 60vh;
    background: var(--bg-panel);
    border: 1px solid var(--border);
    border-radius: 10px;
    box-shadow: var(--shadow);
    display: flex;
    flex-direction: column;
    overflow: hidden;
    height: fit-content;
  }
  input {
    border: none;
    border-bottom: 1px solid var(--border);
    border-radius: 0;
    background: transparent;
    padding: 14px 16px;
    font-size: var(--fs-lg);
  }
  input:focus-visible {
    outline: none;
  }
  ul {
    list-style: none;
    overflow-y: auto;
    padding: 6px;
  }
  li {
    display: flex;
    align-items: stretch;
    border-radius: 6px;
  }
  li.selected {
    background: var(--accent-soft);
  }
  li button.main {
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    gap: 1px;
    flex: 1;
    min-width: 0;
    text-align: left;
    padding: 7px 10px;
    border-radius: 6px;
  }
  li button.drill {
    flex: none;
    align-self: center;
    margin-right: 4px;
    padding: 4px 8px;
    border-radius: 6px;
    font-size: var(--fs-xs);
    color: var(--fg-muted);
    white-space: nowrap;
    visibility: hidden;
  }
  li.selected button.drill {
    visibility: visible;
  }
  li button.drill:hover {
    color: var(--accent);
    background: var(--bg-hover);
  }
  /* The title and the shortcut on one line: the shortcut sits at the end of the row and the
     title fills the rest. The shortcut used to take a second line of its own, which made every
     command two rows tall and fitted only seven on a screen — while a shortcut is something one
     cares about after reading the title, so the end of the row saves more vertical space than
     the next line */
  .line {
    display: flex;
    align-items: baseline;
    gap: 10px;
    width: 100%;
    min-width: 0;
  }
  .title {
    font-size: var(--fs-md);
    /* Truncated to one line: a long title (from the history or the notes) wrapping would make the row heights uneven and fit far fewer on a screen */
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .key {
    margin-left: auto;
    flex: none;
    font-family: inherit;
    font-size: var(--fs-xs);
    color: var(--fg-faint);
    white-space: nowrap;
  }
  .foot {
    flex: none;
    display: flex;
    justify-content: flex-end;
    border-top: 1px solid var(--border);
    padding: 4px 8px;
  }
  .foot-act {
    padding: 3px 8px;
    border-radius: 5px;
    font-size: var(--fs-xs);
    color: var(--fg-muted);
  }
  .foot-act:hover {
    color: var(--danger);
    background: var(--bg-hover);
  }
  .hint {
    font-size: var(--fs-xs);
    color: var(--fg-muted);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    max-width: 100%;
  }
  .empty {
    display: block;
    padding: 16px;
    text-align: center;
    color: var(--fg-faint);
    font-size: var(--fs-sm);
  }
  li.group-head {
    display: block;
    padding: 8px 10px 3px;
    font-size: var(--fs-xs);
    color: var(--fg-faint);
    letter-spacing: 0.08em;
    -webkit-user-select: none;
    user-select: none;
  }
</style>
