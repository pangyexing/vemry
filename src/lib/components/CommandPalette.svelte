<script lang="ts">
  import { api, type HistoryEntryView, type NoteSummary, type SearchHit } from "$lib/api";
  import {
    allCommands,
    commandAvailable,
    commandDescription,
    commandEnglishTitle,
    commandHotkey,
    commandTitle,
    commandWhyNot,
    executeCommand,
    fuzzyFilter,
    fuzzyPositions,
    fuzzyScoreText,
    groupLabel,
    groupOf,
    markRuns,
    termPositions,
    type Command,
  } from "$lib/commands/registry";
  import { t } from "$lib/i18n/i18n.svelte";
  import { browser } from "$lib/stores/browser.svelte";
  import { fetchHeadings, noteKey, noteMatches, notesByRecency, resolveOrCreateNote, type PickHeading } from "$lib/pickNote";
  import { parentOf, typePath, vault } from "$lib/stores/vault.svelte";
  import { scripts } from "$lib/stores/scripts.svelte";
  import { types } from "$lib/stores/types.svelte";
  import { scaffold } from "$lib/scaffold";
  import { toast } from "$lib/stores/toast.svelte";
  import { isImeKeydown } from "$lib/ime";
  import { k } from "$lib/shortcut";

  export type PaletteMode = "commands" | "notes" | "new-note" | "pick-note" | "search" | "vaults" | "url" | "types";

  import { prettyUrl, webCandidates } from "$lib/urls";
  import { pageLabel, searchSettings, settingAvailable, settingHint, settingLabel } from "$lib/settings";
  import { openSettings } from "$lib/stores/settingsUi.svelte";

  interface Item {
    id: string;
    title: string;
    /** Inline after the title, in muted text: a description, a folder, an address. It fills
     *  the rest of the row and is truncated to it; the row's tooltip carries it in full. */
    hint?: string;
    /** A second line under the title. Only search's snippet uses it: the text is the reason
     *  the row exists, so it gets a line of its own rather than the tail of the title's. */
    detail?: string;
    /** The shortcut, right-aligned on the same line as the title. It is its own field rather
     *  than sharing one with hint: a command with both a shortcut and a description would
     *  necessarily lose one of them in a single slot (the old `hotkey ?? description` lost the
     *  description), and they answer different questions — "what does this do" and "how do I
     *  press it next time without opening the palette". */
    key?: string;
    /** This item is the first of a section, and the section header is rendered before it. */
    groupLabel?: string;
    /** A flat list's row (the recent commands, a query's matches) carries its command group
     *  as a muted word before the title; a grouped list has the header instead. */
    group?: string;
    /** The characters of the title (code-point indices) the query matched, drawn in bold. */
    marks?: number[] | null;
    /** The same for the detail line: a full-text hit's words in its snippet. */
    detailMarks?: number[] | null;
    /** The row cannot run now: its title turns muted, the hint says why, and Enter or a
     *  click does nothing. */
    off?: boolean;
    /** A command row: its id, recorded among the recent commands when it runs. */
    cmd?: string;
    /** The note row of pick-note's first stage: Tab enters the section selection from it. */
    pick?: { path: string; title: string };
    /** A bookmark or history row in url mode: return opens that address (a special case in submit, not going through run). */
    url?: string;
    /** A secondary action at the end of the row that does not close the palette (unbookmark,
     *  delete from the history, disable a block type); ⌘↵ runs it, or ⌘⌫ in url mode, whose
     *  ⌘↵ is the new-tab modifier. */
    action?: { label: string; run: () => void };
    /** Absent = this row has no "return runs it here" action (in url mode both opening and
     *  selecting live in submit's url special case, which has to check docked and the
     *  new-window modifier). */
    run?: () => void | Promise<void>;
  }

  let {
    mode = $bindable<PaletteMode | null>(null),
    onPickNote,
  }: {
    mode: PaletteMode | null;
    /** pick-note mode: the chosen (or newly created) note's path comes back through the
     *  callback rather than being opened; heading is the destination section chosen in the
     *  second stage (absent or null = the end of the note). */
    onPickNote?: (path: string, heading?: string | null) => void;
  } = $props();

  let query = $state("");
  let selected = $state(0);
  /** The hits with the query they answer: the rows are drawn from the pair, so a keystroke
   *  does not re-mark the previous hits against text they were not found for. */
  let searched = $state<{ q: string; hits: SearchHit[] }>({ q: "", hits: [] });
  /** The browsing history snapshot fetched once when url mode opens (bookmarks come from browser.bookmarks' permanent mirror). */
  let urlHistory = $state<HistoryEntryView[]>([]);
  let inputEl = $state<HTMLInputElement | null>(null);
  let listEl = $state<HTMLUListElement | null>(null);
  /** pick-note's second stage: a note is chosen and a destination section is being picked (the query becomes the section filter). */
  let pickStage = $state<{ path: string; title: string } | null>(null);
  let pickHeadings = $state<PickHeading[]>([]);

  // ---- The recent commands: the ones run from the palette, most recent first. Only the
  // palette's own runs count — a command reached by its key or from the menu already has
  // a faster way in, and the list is for what the user comes here to find. Per app, not
  // per vault: the commands are the app's (a script command of another vault simply does
  // not resolve and is skipped) ----
  const RECENT_KEY = "vemry-palette-recent";
  const RECENT_SHOWN = 5;
  function loadRecent(): string[] {
    try {
      const v = JSON.parse(localStorage.getItem(RECENT_KEY) ?? "[]");
      return Array.isArray(v) ? v.filter((x): x is string => typeof x === "string") : [];
    } catch {
      return [];
    }
  }
  let recentIds = $state<string[]>(loadRecent());
  function noteUse(id: string) {
    recentIds = [id, ...recentIds.filter((x) => x !== id)].slice(0, 12);
    try {
      localStorage.setItem(RECENT_KEY, JSON.stringify(recentIds));
    } catch {
      /* storage unavailable: the list lives for this session */
    }
  }
  /** A recently run command ranks a little higher among matches of the same kind — never
   *  above a better kind of match (the tiers are 10000 apart). */
  function recentBoost(id: string): number {
    const r = recentIds.indexOf(id);
    return r < 0 ? 0 : Math.max(0, 8 - r);
  }

  const placeholders: Record<PaletteMode, string> = $derived({
    commands: t("palette.commands"),
    notes: t("palette.notes"),
    "new-note": t("palette.newNote"),
    "pick-note": t("palette.pickNote"),
    search: t("palette.search"),
    vaults: t("palette.vaults"),
    url: t("palette.url"),
    types: t("palette.types"),
  });
  const placeholder = $derived(
    mode === "pick-note" && pickStage
      ? t("palette.pickHeading", { title: pickStage.title })
      : mode
        ? placeholders[mode]
        : "",
  );

  /** The two "find something" spaces share one entry (the activity bar's magnifier, and the
   *  segmented head at the top of the palette): notes, and the web. They never mix in one
   *  list — a typed query is either a note search or an address — but switching between them
   *  carries the text over, so a query aimed at the wrong space is one click or one shortcut
   *  from the right one. */
  const FIND_MODES: PaletteMode[] = ["search", "url"];
  const isFind = $derived(mode !== null && FIND_MODES.includes(mode));
  let prevMode: PaletteMode | null = null;
  $effect(() => {
    if (mode) {
      // The text carries over only between the two find spaces; every other mode starts clean
      const carry = prevMode !== null && prevMode !== mode && FIND_MODES.includes(prevMode) && FIND_MODES.includes(mode);
      if (!carry) query = "";
      selected = 0;
      searched = { q: "", hits: [] };
      pickStage = null;
      queueMicrotask(() => inputEl?.focus());
    }
    prevMode = mode;
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

  /** url mode's candidate pool (the same list the new-tab page draws on): it depends on the
   *  two mirrors alone and not on query, so a keystroke only filters it. */
  const urlCandidates = $derived(webCandidates(browser.bookmarks, urlHistory));

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

  /** A keystroke puts the highlight back on the first row: the list has just been
   *  re-ranked, and what was under the highlight is some other row now. The one exception
   *  is the `#` that takes pick-note into its second stage — the effect above reads the row
   *  the user had highlighted. */
  function oninput() {
    if (mode === "pick-note" && !pickStage && query.includes("#")) return;
    selected = 0;
  }

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
    if (mode !== "search" || !query.trim()) {
      searched = { q: "", hits: [] };
      return;
    }
    // A search reads the whole vault under its lock: one per pause in the typing, not per key
    const q = query.trim();
    const timer = setTimeout(() => {
      void api.search(q, 30).then((hits) => {
        if (q === query.trim()) searched = { q, hits };
      });
    }, 100);
    return () => clearTimeout(timer);
  });

  /** The commands a query matches, ranked in three tiers: the title, then the English
   *  title and the id (so an English name finds a command in any interface language), then
   *  the description alone. A title match always outranks a description that happens to
   *  contain the word. Within a tier the match quality decides, nudged by recent use. */
  function rankCommands(cmds: Command[], q: string): { c: Command; title: string; titleHit: boolean }[] {
    const scored: { c: Command; title: string; titleHit: boolean; score: number }[] = [];
    for (const c of cmds) {
      const title = commandTitle(c);
      let tier = 3;
      let s = fuzzyScoreText(title, q);
      if (s === null) {
        tier = 2;
        const en = commandEnglishTitle(c);
        s = fuzzyScoreText(`${en === title ? "" : en} ${c.id}`, q);
      }
      if (s === null) {
        tier = 1;
        s = fuzzyScoreText(commandDescription(c) ?? "", q);
      }
      if (s === null) continue;
      scored.push({ c, title, titleHit: tier === 3, score: tier * 10000 + s + recentBoost(c.id) });
    }
    return scored.sort((a, b) => b.score - a.score);
  }

  let items = $derived.by((): Item[] => {
    if (mode === "commands") {
      // The registry itself is a plain Map and is not reactive; what a script hot reload changes
      // is scripts.infos, so depending on it explicitly lets an open palette see added and
      // removed script commands immediately
      void scripts.infos;
      // The command that opens the palette is left out: in the one place it would be visible it is a no-op
      const cmds = allCommands().filter((c) => c.id !== "command-palette");
      const row = (c: Command, title = commandTitle(c)): Item => ({
        id: c.id,
        cmd: c.id,
        title,
        hint: commandDescription(c),
        key: commandHotkey(c),
        run: () => c.run(),
      });
      const q = query.trim();
      if (!q) {
        // An empty query: the recent commands, then every available one in the registry's
        // group order under group headers. What cannot run now is left out altogether — a
        // query still finds it, with the reason
        const avail = cmds.filter(commandAvailable);
        const byId = new Map(avail.map((c) => [c.id, c]));
        const recent = recentIds.flatMap((id) => byId.get(id) ?? []).slice(0, RECENT_SHOWN);
        const list: Item[] = recent.map((c, i) => ({
          ...row(c),
          id: `recent:${c.id}`,
          group: groupLabel(groupOf(c)),
          groupLabel: i === 0 ? t("palette.recentlyUsed") : undefined,
        }));
        avail.forEach((c, i) => {
          // Already sorted by group order, so differing from the previous one makes it a group header
          const g = groupOf(c);
          list.push({ ...row(c), groupLabel: i === 0 || g !== groupOf(avail[i - 1]) ? groupLabel(g) : undefined });
        });
        return list;
      }
      // A query flattens the list: the available matches, then the settings it finds (by
      // the dialog's own search, so both find a setting by the same words), then the
      // matches that cannot run now, under their own header and with the reason
      const ranked = rankCommands(cmds, q);
      const flat = (r: (typeof ranked)[number]): Item => ({
        ...row(r.c, r.title),
        group: groupLabel(groupOf(r.c)),
        marks: r.titleHit ? fuzzyPositions(r.title, q) : null,
      });
      const list: Item[] = ranked.filter((r) => commandAvailable(r.c)).map(flat);
      searchSettings(q)
        .filter((s) => settingAvailable(s, !!vault.root))
        .forEach((s, i) => {
          const title = settingLabel(s);
          list.push({
            id: `setting:${s.id}`,
            title,
            hint: settingHint(s),
            key: pageLabel(s.page),
            marks: termPositions(title, q),
            groupLabel: i === 0 ? t("palette.settingsGroup") : undefined,
            run: () => openSettings(s.id),
          });
        });
      ranked
        .filter((r) => !commandAvailable(r.c))
        .forEach((r, i) => {
          const it = flat(r);
          list.push({
            ...it,
            off: true,
            hint: commandWhyNot(r.c) ?? it.hint,
            groupLabel: i === 0 ? t("palette.unavailable") : undefined,
            run: undefined,
          });
        });
      return list;
    }
    if (mode === "pick-note" && pickStage) {
      // The second stage: that note's section list, with "the end of the note" fixed as the first item (Enter reaches it with no friction)
      const stage = pickStage;
      return [
        { id: "__whole__", title: t("palette.wholeNote"), hint: stage.path, run: () => onPickNote?.(stage.path, null) },
        ...fuzzyFilter(pickHeadings, query, (h) => h.text).map((h, i) => ({
          id: `${i}:${h.text}`,
          title: `${"#".repeat(h.level)} ${h.text}`,
          marks: fuzzyPositions(`${"#".repeat(h.level)} ${h.text}`, query),
          hint: t("palette.appendToSection"),
          run: () => onPickNote?.(stage.path, h.text),
        })),
      ];
    }
    if (mode === "new-note") {
      // ⌘N: the note to create comes first and is highlighted, so a title and Enter create
      // it; the notes it resembles follow, one ↓ away — there to stop a duplicate, never in
      // the way of the new one
      const q = query.trim();
      if (!q) return [];
      const { notes, create } = noteMatches(query);
      const list: Item[] = [];
      if (create) {
        list.push({
          id: "__create__",
          title: t("palette.createNamed", { title: create }),
          hint: t("palette.createAndOpen"),
          run: () => vault.createAndOpen(create).catch((e) => toast.push(String(e), "error")),
        });
      }
      notes.slice(0, 20).forEach((n, i) =>
        list.push({
          id: n.id,
          title: n.title,
          marks: fuzzyPositions(n.title, q),
          hint: parentOf(n.id),
          groupLabel: i === 0 && create ? t("palette.similarNotes") : undefined,
          run: () => vault.openNote(n.id),
        }),
      );
      return list;
    }
    if (mode === "notes" || mode === "pick-note") {
      // pick-note and notes share the filtering and the create row; only "what happens after choosing" differs
      const pick = mode === "pick-note" ? onPickNote : undefined;
      const q = query.trim();
      const { notes, create: title } = q ? noteMatches(query) : { notes: notesByRecency(!!pick), create: null };
      const list: Item[] = notes.slice(0, 50).map((n) => ({
        id: n.id,
        title: n.title,
        marks: fuzzyPositions(n.title, q),
        hint: parentOf(n.id),
        pick: pick ? { path: n.id, title: n.title } : undefined,
        run: () => (pick ? pick(n.id) : vault.openNote(n.id)),
      }));
      if (title) {
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
    if (mode === "types") {
      // The vault's block types, disabled ones included (the file is what opens, and a
      // disabled one is exactly the one to edit). A name with no file yet gets a create row,
      // and while there is no `book` the example is offered under that name
      const q = query.trim();
      const has = (name: string) => types.list.some((x) => x.name.toLowerCase() === name.toLowerCase());
      const list: Item[] = fuzzyFilter(types.list, query, (x) => `${x.name} ${x.description}`).map((x) => {
        const title = x.icon ? `${x.icon} ${x.name}` : x.name;
        return {
          id: x.file,
          title,
          marks: fuzzyPositions(title, q),
          hint: x.disabled ? t("palette.typeDisabled") : x.description || typePath(x.file),
          run: () => vault.openFile(typePath(x.file)),
          // The row's toggle (the button, or ⌘↩): the file's disabled line flips and types_changed
          // redraws the row, the palette staying open like url mode's unbookmark
          action: {
            label: t(x.disabled ? "palette.typeEnable" : "palette.typeDisable"),
            run: () => void api.typeSetDisabled(x.file, !x.disabled).catch((e) => toast.push(String(e), "error")),
          },
        };
      });
      if (!has("book") && fuzzyFilter(["book"], query, (s) => s).length > 0) {
        list.push({
          id: "__example__",
          title: "book",
          hint: t("palette.typeExample"),
          run: () => scaffold(() => api.typeCreate("book")),
        });
      }
      if (q && !has(q) && q.toLowerCase() !== "book") {
        list.push({
          id: "__create__",
          title: t("palette.createType", { name: q }),
          hint: t("palette.createTypeHint"),
          run: () => scaffold(() => api.typeCreate(q)),
        });
      }
      return list;
    }
    if (mode === "vaults") {
      const q = query.trim();
      const vaults = fuzzyFilter(vault.recent, query, (p) => p);
      const list: Item[] = vaults.map((p) => {
        const title = p.split("/").pop() ?? p;
        return {
          id: p,
          title,
          marks: fuzzyPositions(title, q),
          hint: p === vault.root ? t("palette.currentVault", { path: p }) : p,
          run: () => {
            if (p !== vault.root) void vault.openVault(p);
          },
        };
      });
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
      // A hit opens where the index found it (its line and column); the words are drawn
      // in bold in the title and the snippet, and the folder tells two notes of one title apart
      const { q, hits } = searched;
      return hits.map((h, i) => ({
        id: `${h.note}:${i}`,
        title: h.title,
        marks: termPositions(h.title, q),
        hint: parentOf(h.note),
        key: t("palette.lineN", { n: h.line }),
        detail: h.snippet,
        detailMarks: termPositions(h.snippet, q),
        run: () => vault.openNoteAtLine(h.note, h.line, h.col),
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
        if (browser.tabs.length > 0) {
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
        // A page with no title is shown by its address, and then the address is not repeated as the hint
        browser.bookmarks.slice(0, 8).forEach((b, i) => {
          list.push({
            id: `bm:${b.url}`,
            title: b.title || prettyUrl(b.url),
            hint: b.title ? prettyUrl(b.url) : undefined,
            groupLabel: i === 0 ? t("palette.bookmarks") : undefined,
            url: b.url,
            action: unstar(b.url),
          });
        });
        // When row 0 is already "reopen the last page" (= history[0]), the recent section skips it rather than repeating it
        const skipFirst = browser.tabs.length === 0 && urlHistory.length > 0 ? 1 : 0;
        urlHistory.slice(skipFirst, skipFirst + 12).forEach((h, i) => {
          list.push({
            id: `h:${h.url}`,
            title: h.title || prettyUrl(h.url),
            hint: h.title ? prettyUrl(h.url) : undefined,
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
        const title = c.starred ? `★ ${c.title || c.pretty}` : c.title || c.pretty;
        list.push({
          id: `${c.starred ? "bm" : "h"}:${c.url}`,
          title,
          marks: fuzzyPositions(title, q),
          hint: c.title ? c.pretty : undefined,
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

  // ---- The footer: the keys of the mode at hand, so nothing the keyboard can do is known
  // only to those who read the source. The placeholder says what to type; the footer says
  // what the keys do (some entries depend on the highlighted row) ----
  const ENTER = k("Enter");
  const ESC = k("Escape");
  const MOD_ENTER = k("Mod+Enter");
  interface FootKey {
    /** The keys; empty for a line of plain advice ("type a new name to create it"). */
    keys: string;
    label: string;
  }
  const foot = $derived.by((): FootKey[] => {
    const sel = items[selected];
    const close: FootKey = { keys: ESC, label: t("palette.foot.close") };
    switch (mode) {
      case "commands":
        return [{ keys: "↑↓", label: t("palette.foot.select") }, { keys: ENTER, label: t("palette.foot.run") }, close, { keys: "", label: t("palette.foot.typeSetting") }];
      case "new-note":
        return sel && sel.id !== "__create__"
          ? [{ keys: ENTER, label: t("palette.foot.open") }, close]
          : [{ keys: ENTER, label: t("palette.foot.create") }, { keys: "↓", label: t("palette.foot.openExisting") }, close];
      case "notes":
        return [{ keys: ENTER, label: t(sel?.id === "__create__" ? "palette.foot.create" : "palette.foot.open") }, { keys: "", label: t("palette.foot.typeNewToCreate") }, close];
      case "pick-note":
        return pickStage
          ? [{ keys: ENTER, label: t(sel?.id === "__whole__" ? "palette.foot.wholeNote" : "palette.foot.appendSection") }, { keys: ESC, label: t("palette.foot.back") }]
          : [{ keys: ENTER, label: t("palette.foot.wholeNote") }, { keys: "Tab / #", label: t("palette.foot.pickSection") }, close];
      case "search":
        return [{ keys: ENTER, label: t("palette.foot.openAtLine") }, close];
      case "url": {
        const keys: FootKey[] = [{ keys: ENTER, label: t("palette.foot.open") }, { keys: MOD_ENTER, label: t("palette.foot.newTab") }];
        if (sel?.action) keys.push({ keys: k("Mod+Backspace"), label: sel.action.label });
        return [...keys, close];
      }
      case "types": {
        const keys: FootKey[] = [{ keys: ENTER, label: t("palette.foot.openFile") }];
        if (sel?.action) keys.push({ keys: MOD_ENTER, label: sel.action.label });
        return [...keys, { keys: "", label: t("palette.foot.typeNewToCreate") }, close];
      }
      case "vaults":
        return [{ keys: ENTER, label: t("palette.foot.switch") }, close];
      default:
        return [];
    }
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
        // ⌘↩ opens a new tab; a bare ↩ navigates the active one (a blank tab included)
        await api.browserOpen(url, modifier).catch((e) => toast.push(String(e), "error"));
      }
      return;
    }
    const item = items[selected];
    // ⌘↩ on a row with a secondary action runs that instead, the palette staying open (url
    // mode's ⌘↩ is its new-window modifier and returned above)
    if (modifier && item?.action) {
      item.action.run();
      return;
    }
    if (item?.run && !item.off) {
      if (item.cmd) noteUse(item.cmd);
      close();
      await item.run();
    }
  }

  function onkeydown(e: KeyboardEvent) {
    // ⌘L / ⌘⇧F switch between the two find spaces while the palette is open (off mac; on mac
    // both are menu accelerators that reach the same commands and never arrive here)
    const mod = e.metaKey || e.ctrlKey;
    if (mod && isFind && e.key.toLowerCase() === "l" && !e.shiftKey) {
      e.preventDefault();
      mode = "url";
      return;
    }
    if (mod && isFind && e.key.toLowerCase() === "f" && e.shiftKey) {
      e.preventDefault();
      mode = "search";
      return;
    }
    // url mode's secondary action (unbookmark, delete from the history) on ⌘⌫, since its ⌘↩
    // is the new-tab modifier; on a row without one ⌘⌫ stays the input's own "delete to the
    // start of the line"
    if (mod && e.key === "Backspace" && mode === "url" && items[selected]?.action) {
      e.preventDefault();
      items[selected].action?.run();
      return;
    }
    if (e.key === "Escape") {
      e.preventDefault();
      // Esc in the second stage means "back to choosing a note" rather than closing the palette
      if (pickStage) exitPickStage();
      else close();
    } else if (e.key === "Tab") {
      // The palette is modal and its list is driven by ↑↓, so Tab never walks the rows (they
      // sit outside the tab order too) — it used to land the focus ring on the first row while
      // the highlight stayed elsewhere. In pick-note's first stage it is the way into the
      // section stage
      e.preventDefault();
      const p = mode === "pick-note" && !pickStage && !e.shiftKey ? items[selected]?.pick : undefined;
      if (p) enterPickStage(p);
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

  const LIST_ID = "palette-list";
  const optionId = (i: number) => `palette-opt-${i}`;
  const emptyText = $derived(
    query
      ? t("palette.noMatch")
      : mode === "url"
        ? t("palette.noBookmarksOrHistory")
        : mode === "new-note"
          ? t("palette.newNoteEmpty")
          : t("palette.typeToFilter"),
  );
</script>

{#snippet marked(text: string, marks: readonly number[] | null | undefined)}
  {#each markRuns(text, marks) as run, i (i)}{#if run.hit}<b>{run.text}</b>{:else}{run.text}{/if}{/each}
{/snippet}

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
      {#if isFind}
        <!-- The two find spaces as underline tabs, the same header language as the sidebar's Outline | Research -->
        <div class="spaces" role="tablist" aria-label={t("palette.findSpaces")}>
          <button class="space" class:active={mode === "search"} role="tab" aria-selected={mode === "search"} onclick={() => (mode = "search")}>
            {t("palette.modeNotes")}<kbd class="space-key">{k("Mod+Shift+F")}</kbd>
          </button>
          <button class="space" class:active={mode === "url"} role="tab" aria-selected={mode === "url"} onclick={() => (mode = "url")}>
            {t("palette.modeWeb")}<kbd class="space-key">{k("Mod+L")}</kbd>
          </button>
        </div>
      {/if}
      <input
        bind:this={inputEl}
        bind:value={query}
        {placeholder}
        {onkeydown}
        {oninput}
        spellcheck="false"
        role="combobox"
        aria-expanded="true"
        aria-controls={LIST_ID}
        aria-autocomplete="list"
        aria-activedescendant={items.length ? optionId(selected) : undefined}
      />
      <ul bind:this={listEl} id={LIST_ID} role="listbox" aria-label={placeholder}>
        {#each items as item, i (item.id)}
          {#if item.groupLabel}
            <li class="group-head" role="presentation">{item.groupLabel}</li>
          {/if}
          <li class:selected={i === selected} class:off={item.off} id={optionId(i)} role="option" aria-selected={i === selected} aria-disabled={item.off || undefined}>
            <button
              class="main"
              tabindex="-1"
              title={item.hint}
              onclick={() => {
                selected = i;
                void submit();
              }}
              onmouseenter={() => (selected = i)}
            >
              <span class="line">
                {#if item.group}<span class="grp">{item.group}</span>{/if}
                <span class="title">{@render marked(item.title, item.marks)}</span>
                {#if item.hint}<span class="hint">{item.hint}</span>{/if}
                {#if item.key}<kbd class="key">{item.key}</kbd>{/if}
              </span>
              {#if item.detail}<span class="detail">{@render marked(item.detail, item.detailMarks)}</span>{/if}
            </button>
            {#if item.pick}
              {@const p = item.pick}
              <!-- The mouse equivalent of the keyboard's Tab: clicking the row clips the whole note, and clicking here enters the second stage to pick a section -->
              <button
                class="drill"
                tabindex="-1"
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
              <button class="drill" tabindex="-1" onclick={a.run} onmouseenter={() => (selected = i)}>
                {a.label}
              </button>
            {/if}
          </li>
        {:else}
          <li class="empty" role="presentation">{emptyText}</li>
        {/each}
      </ul>
      {#if foot.length || (mode === "url" && urlHistory.length > 0)}
        <div class="foot">
          {#each foot as f, i (i)}
            <span class="foot-key">{#if f.keys}<kbd>{f.keys}</kbd>{/if}{f.label}</span>
          {/each}
          {#if mode === "url" && urlHistory.length > 0}
            <button class="foot-act" title={t("palette.clearHistoryHint")} onclick={clearHistory}>
              {t("cmd.clear-browser-history.title")}
            </button>
          {/if}
        </div>
      {/if}
    </div>
  </div>
{/if}

<style>
  .overlay {
    position: fixed;
    inset: 0;
    background: var(--scrim);
    display: flex;
    justify-content: center;
    padding-top: 12vh;
    z-index: 100;
  }
  .palette {
    width: min(640px, 90vw);
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
  /* The find spaces' head: underline tabs flush with the top edge (App's .tabs, the sidebar
     header, is the reference — same padding, radius and accent underline), the shortcut set
     faint beside each label so the head teaches the keys without shouting them */
  .spaces {
    display: flex;
    gap: 2px;
    padding: 8px 10px 0;
    border-bottom: 1px solid var(--border);
  }
  .space {
    display: inline-flex;
    align-items: baseline;
    gap: 8px;
    font-size: var(--fs-sm);
    color: var(--fg-muted);
    padding: 5px 12px;
    border-radius: 5px 5px 0 0;
    border-bottom: 2px solid transparent;
  }
  .space:hover {
    color: var(--fg);
    background: var(--bg-hover);
  }
  .space.active {
    color: var(--fg);
    border-bottom-color: var(--accent);
  }
  .space-key {
    font-family: inherit;
    font-size: var(--fs-xs);
    color: var(--fg-muted);
    white-space: nowrap;
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
  /* A row that cannot run now: its title in the secondary ink (never faded by opacity —
     the reason beside it has to stay readable), the hint saying why */
  li.off .title {
    color: var(--fg-muted);
  }
  li button.main {
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    gap: 1px;
    flex: 1;
    min-width: 0;
    text-align: left;
    padding: 6px 10px;
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
  /* One line per row: the title, then the hint in muted text filling the rest of the row,
     then the shortcut at the end. The hint used to take a second line of its own, which made
     a row with one twice as tall as a row without — the list read as two kinds of rows and
     fitted nine commands on a screen — while both the hint and the shortcut are read after
     the title, so the tail of the same line is where they belong. The title keeps its full
     width and the hint gives way (flex-basis 0): a hint that does not fit is truncated, and
     the row's tooltip carries it in full */
  .line {
    display: flex;
    align-items: baseline;
    gap: 10px;
    width: 100%;
    min-width: 0;
  }
  /* A flat list's group word before the title: the header's job, done in the row */
  .grp {
    flex: none;
    font-size: var(--fs-xs);
    color: var(--fg-muted);
    white-space: nowrap;
  }
  /* The matched characters: bold in the row's own ink, no colour — the accent is for what
     can be clicked and where you are */
  .title b,
  .detail b {
    font-weight: 600;
    color: var(--fg);
  }
  .title {
    font-size: var(--fs-md);
    /* Truncated to one line: a long title (from the history or the notes) wrapping would make the row heights uneven and fit far fewer on a screen */
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .hint {
    flex: 1;
    min-width: 0;
    font-size: var(--fs-xs);
    color: var(--fg-muted);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .key {
    margin-left: auto;
    flex: none;
    font-family: inherit;
    font-size: var(--fs-xs);
    color: var(--fg-muted);
    white-space: nowrap;
  }
  /* The keys of the mode at hand on the left, the mode's own action (url mode's "clear the
     browsing history") pushed to the right */
  .foot {
    flex: none;
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 2px 16px;
    border-top: 1px solid var(--border);
    padding: 5px 14px;
    font-size: var(--fs-xs);
    color: var(--fg-muted);
    -webkit-user-select: none;
    user-select: none;
  }
  .foot-key kbd {
    font-family: inherit;
    color: var(--fg);
    margin-right: 5px;
  }
  .foot-act {
    margin-left: auto;
    margin-right: -8px;
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
  .detail {
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
    color: var(--fg-muted);
    font-size: var(--fs-sm);
  }
  li.group-head {
    display: block;
    padding: 10px 10px 3px;
    font-size: var(--fs-xs);
    color: var(--fg-muted);
    letter-spacing: 0.08em;
    -webkit-user-select: none;
    user-select: none;
  }
  /* Tracking pulls Han, kana and hangul apart rather than setting a heading off */
  li.group-head:is(:lang(zh), :lang(ja), :lang(ko)) {
    letter-spacing: normal;
  }
  /* The first header sits right under the input's rule and needs no room above it */
  li.group-head:first-child {
    padding-top: 4px;
  }
</style>
