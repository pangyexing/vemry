<script lang="ts">
  import { api, type BlockHit } from "$lib/api";
  import { vault, stemOf } from "$lib/stores/vault.svelte";
  import {
    DATE_FIELDS,
    dateFieldLabel,
    type DateField,
    dateIconSvg,
    deleteTask,
    dueLabel,
    localDate,
    rescheduleTask,
    taskFilterKey,
    taskKey,
    taskStateIconSvg,
    updateTask,
    type TaskEdit,
  } from "$lib/views/tasks";
  import {
    activeField,
    cardCol,
    cardsOf,
    columnDefsFor,
    columnsOf,
    DEFAULT_ORDER,
    defaultDesc,
    dropEdit,
    fieldKeysOf,
    parseSortKey,
    quickTaskPayload,
    sortKeyId,
    type Card,
    type CardOrder,
    type GroupMode,
  } from "$lib/views/kanban";
  import { t } from "$lib/i18n/i18n.svelte";
  import { k as kbd } from "$lib/shortcut";
  import { contextMenu, type MenuEntry } from "$lib/stores/contextMenu.svelte";
  import { toast } from "$lib/stores/toast.svelte";
  import { loadBucketMap, saveBucket } from "$lib/persist";
  import { noteKey } from "$lib/pickNote";
  // The list sorts by its own contract (cardComparator inside columnsOf), so the order-preserving
  // fuzzyKeep is used; picking a destination note is a selector, where "the closest match
  // first" is right, so that still uses fuzzyFilter
  import { fuzzyFilter, fuzzyKeep } from "$lib/commands/registry";
  import TaskDateMenu from "./TaskDateMenu.svelte";
  import ViewHeader, { countLabel } from "./ViewHeader.svelte";
  import ViewEmpty from "./ViewEmpty.svelte";
  import PrivacyIcon from "../PrivacyIcon.svelte";

  let hits = $state<BlockHit[]>([]);
  let query = $state("");
  // With events firing in quick succession several loads run at once and an older response may arrive later: only the newest one lands (the same as Agenda's)
  let loadSeq = 0;

  async function load() {
    const seq = ++loadSeq;
    try {
      const res = await api.queryBlocks({ kind: "task", limit: 5000 });
      if (seq === loadSeq) hits = res;
    } catch (e) {
      vault.error = String(e);
    }
  }

  $effect(() => {
    void vault.indexVersion;
    void load();
  });

  /** The cards are built before they are filtered: the column assignment reads the column and
   *  dates cardOf settled, and the filtering reads taskFilterKey — the same key as Agenda's,
   *  so the two views give the same answer to the same keystroke; and building a card happens
   *  once per block rather than on every keystroke. */
  const allCards = $derived(cardsOf(hits, localDate()));
  const shown = $derived(fuzzyKeep(allCards, query, taskFilterKey));
  const total = $derived(shown.length);

  /** The guard every write shares (the same as Agenda's): the note being edited is saved
   *  first, so unsaved changes are not overwritten. The meaning still holds with several tabs
   *  — only the active tab can be dirty. */
  function dirtyGuard(hit: BlockHit, act: string): boolean {
    if (hit.note === vault.currentPath && vault.dirty) {
      vault.error = t("task.dirtyGuard", { key: kbd("Mod+S"), action: act });
      return false;
    }
    return true;
  }

  /** The shared write funnel: dirtyGuard, a requery on a stale span (false) and error
   *  reporting. Every write path (a drag's updateTask, a chip's rescheduleTask) has to come
   *  through here. true only when the write went to disk: a drop's provisional card waits
   *  for that write's reload, and every other outcome has no reload coming. */
  async function commit(hit: BlockHit, act: string, write: () => Promise<boolean>): Promise<boolean> {
    if (!dirtyGuard(hit, act)) return false;
    try {
      // A stale span (the note changed after the query): nothing is written, and after a requery the user retries on the new data
      if (await write()) return true;
      await load();
    } catch (err) {
      vault.error = String(err);
    }
    return false;
  }

  const applyEdit = (hit: BlockHit, edit: TaskEdit, act: string) => commit(hit, act, () => updateTask(hit, edit));

  async function onToggle(e: Event, card: Card) {
    e.stopPropagation();
    await applyEdit(card.hit, { state: card.state === "done" ? "todo" : "done" }, t("task.act.toggle"));
  }

  /** Clicking a card jumps to the task's line. Agenda uses the same vault.openBlockAt — the
   *  byte → UTF-16 conversion and rechecking currentPath after the await are contracts kept in
   *  one place. */
  const openCard = (card: Card) => vault.openBlockAt(card.hit.note, card.hit.block.span.start);

  // ---- Dragging: pointer events, the same contract as FileTree's (a 5px threshold, a ghost
  // that intercepts no events, elementFromPoint to resolve the drop target, suppressClick to
  // swallow the click after a drag, Escape to cancel) ----
  let pressed: { card: Card; x: number; y: number } | null = null;
  let drag = $state<{ card: Card; x: number; y: number } | null>(null);
  let dropTarget = $state<string | null>(null);
  let suppressClick = false;
  /** Swallow the click that follows this release. It comes within the same task as the
   *  pointerup, and the capture handler below consumes the flag then. It comes only when the
   *  card's element is still in the document: a drop into "scheduled" moves the card to
   *  another column in the render right after the release, and no click is sent for a
   *  detached element. The flag must not live on past that task, or it swallows the user's
   *  next click — the first one on the date menu */
  function swallowNextClick() {
    suppressClick = true;
    setTimeout(() => (suppressClick = false), 0);
  }
  /** The date menu has two origins: drop = dragged into the "scheduled" column (which needs a
   *  concrete date, since any default would be arbitrary, and cancelling cancels the whole
   *  drop — the card already sits in that column meanwhile, see pendingDrop); chip = clicking
   *  a card's date or scheduling chip to reschedule directly. */
  type MenuState =
    | { mode: "drop"; x: number; y: number; card: Card }
    | { mode: "chip"; x: number; y: number; card: Card; field: DateField; current?: string };
  let menu = $state<MenuState | null>(null);
  /** A card dropped into "scheduled" shows there at once, at the top of the column, rather
   *  than only after its date is picked: date null while the menu is open (Esc or a click
   *  outside puts the card back where it was), then the picked date until a query made after
   *  the write has landed the card in its real place. The card is looked up by key in
   *  whatever hits are current, so a reload while the menu is open does not lose it, and a
   *  reload from elsewhere while the write is in flight (still showing the old date) does not
   *  put it back early — only the write's own completion clears it. */
  let pendingDrop = $state.raw<{ card: Card; date: string | null } | null>(null);
  /** The pending card's key, read once per render by the column pass and by every card. */
  const pendingKey = $derived(pendingDrop ? taskKey(pendingDrop.card) : null);

  function pressCard(e: PointerEvent, card: Card) {
    if (e.button !== 0) return;
    pressed = { card, x: e.clientX, y: e.clientY };
    // capture: move/up still arrive when the pointer leaves the window, so a drag is never left hanging
    (e.currentTarget as HTMLElement | null)?.setPointerCapture?.(e.pointerId);
  }

  function dragMove(e: PointerEvent) {
    if (pressed && !drag) {
      if (Math.hypot(e.clientX - pressed.x, e.clientY - pressed.y) < 5) return;
      drag = { card: pressed.card, x: e.clientX, y: e.clientY };
      cancelMenu();
    }
    if (!drag) return;
    e.preventDefault();
    drag.x = e.clientX;
    drag.y = e.clientY;
    const el = document.elementFromPoint(e.clientX, e.clientY)?.closest<HTMLElement>("[data-drop-col]");
    const col = el?.dataset.dropCol ?? null;
    // Dropping back in the same column means nothing happens, so it is not highlighted misleadingly
    dropTarget = col !== null && col !== cardCol(drag.card, groupBy) ? col : null;
  }

  function dragEnd(e: PointerEvent) {
    pressed = null;
    if (!drag) {
      // A drag Escape cancelled armed the flag for this release's click; it is bounded here
      // the same way as a drop's
      if (suppressClick) swallowNextClick();
      return;
    }
    const d = drag.card;
    const target = dropTarget;
    drag = null;
    dropTarget = null;
    swallowNextClick();
    if (target === null) return;
    if (target === "scheduled" && groupBy.by === "state") {
      // The date menu pops up at the drop point and only writes once a date is chosen (Esc or
      // a click outside cancels); the card moves into the column right away, not after the choice
      menu = { mode: "drop", x: e.clientX, y: e.clientY, card: d };
      pendingDrop = { card: d, date: null };
      return;
    }
    const edit = dropOf(d, target);
    if (edit) void applyEdit(d.hit, edit, t("task.act.move"));
  }

  /** Closing the menu without a choice: a drop still waiting for its date is undone (the
   *  card returns to its column); a write already in flight is left to land. */
  function cancelMenu() {
    menu = null;
    if (pendingDrop?.date === null) pendingDrop = null;
  }

  function dragCancel() {
    pressed = null;
    drag = null;
    dropTarget = null;
    // After a pointercancel no click comes to consume the one-shot flag — a leftover would
    // swallow the user's next, unrelated click (the same discipline as FileTree's)
    suppressClick = false;
  }

  const dropOf = (card: Card, col: string, date?: string) => dropEdit(card, col, localDate(), date, groupBy);

  /** Clicking a date chip or the scheduling ghost chip: the menu pops up at the chip and reschedules directly, without dragging */
  function openChipMenu(e: MouseEvent, card: Card, field: DateField) {
    e.stopPropagation();
    menu = { mode: "chip", x: e.clientX, y: e.clientY, card, field, current: field === "due" ? card.due : card.scheduled };
  }

  /** The menu's choice: drop follows the landing semantics of a drag (dropEdit includes the
   *  reopening merged in when dragging out of done); chip goes through the single-field
   *  rescheduleTask rather than updateTask, because a null there has to clear that one field
   *  (TaskEdit.clearDates clears both, which is the wrong meaning). */
  function onMenuPick(date: string | null) {
    const m = menu;
    menu = null;
    if (!m) return;
    if (m.mode === "drop") {
      // The drop menu passes no current, so no "remove" item appears; null is the defensive branch
      const edit = date === null ? null : dropOf(m.card, "scheduled", date);
      if (!edit) {
        pendingDrop = null;
        return;
      }
      // The provisional card carries the date now, until a query made after the write has
      // come back — the note is reindexed before the write's IPC resolves, so that query
      // necessarily carries the date (a newer load superseding it post-dates the write too).
      // A refused, stale or failed write has no reload coming, so the card goes back at once
      pendingDrop = { card: m.card, date };
      void applyEdit(m.card.hit, edit, t("task.act.editDate")).then(async (ok) => {
        if (ok) await load();
        pendingDrop = null;
      });
    } else {
      const { hit } = m.card;
      void commit(hit, t("task.act.editDate"), () => rescheduleTask(hit, m.field, date));
    }
  }

  /** The card's context menu: the reschedule entry reuses TaskDateMenu in chip mode (raised at
   *  the right-click), and the state items go through the same applyEdit funnel. entries are
   *  computed for the card at the moment of open. */
  function cardMenu(e: MouseEvent, card: Card) {
    e.stopPropagation();
    const at = { x: e.clientX, y: e.clientY };
    const chipMenu = (field: DateField, current?: string) => () =>
      (menu = { mode: "chip", ...at, card, field, current });
    const entries: MenuEntry[] = [{ label: t("kanban.jumpToSource"), action: () => void openCard(card) }, { separator: true }];
    if (card.due) entries.push({ label: `${t("task.editDate", { field: dateFieldLabel("due") })}…`, action: chipMenu("due", card.due) });
    if (card.scheduled)
      entries.push({ label: `${t("task.editDate", { field: dateFieldLabel("scheduled") })}…`, action: chipMenu("scheduled", card.scheduled) });
    if (!card.due && !card.scheduled && card.state !== "done")
      entries.push({ label: `${t("task.setDate", { field: dateFieldLabel("due") })}…`, action: chipMenu("due") });
    entries.push({ separator: true });
    if (card.state !== "done")
      entries.push({
        label: card.state === "doing" ? t("kanban.markBackTodo") : t("kanban.markDoing"),
        action: () => void applyEdit(card.hit, { state: card.state === "doing" ? "todo" : "doing" }, t("task.act.state")),
      });
    entries.push({
      label: card.state === "done" ? t("kanban.reopen") : t("kanban.markDone"),
      action: () => void applyEdit(card.hit, { state: card.state === "done" ? "todo" : "done" }, t("task.act.toggle")),
    });
    entries.push(
      { separator: true },
      {
        label: t("kanban.removeTask"),
        danger: true,
        // The second confirmation (built into ContextMenu): what is deleted is the task line in the note's source, sub-content included for a multi-line task
        confirm: t("kanban.removeConfirm"),
        action: () => void commit(card.hit, t("task.act.remove"), () => deleteTask(card.hit)),
      },
    );
    contextMenu.open(e, entries);
  }

  // ---- Column collapsing plus the quick-create destination: persisted per vault (persist.ts's
  // buckets). The component is rebuilt wholesale by {#key vault.root}, so reading the bucket
  // once at initialisation is enough and there is no cross-vault timing trap ----
  const KANBAN_UI_KEY = "vemry-kanban-ui";
  type KanbanUi = { collapsed: string[]; target: string; groupBy?: GroupMode; sort?: { key: string; desc: boolean } };
  const savedUi = vault.root ? loadBucketMap<KanbanUi>(KANBAN_UI_KEY)[vault.root] : undefined;
  // A record rather than a Set: a toggle is one assignment (a Set would need a whole
  // copy-on-write), and an unfamiliar column name restored from storage is harmless and need
  // not be filtered through the column list again
  let collapsed = $state<Record<string, boolean>>(
    Object.fromEntries((savedUi?.collapsed ?? []).map((c) => [c, true])),
  );
  /** The grouping (persisted per vault with the rest of the board's UI state). */
  let groupBy = $state<GroupMode>(
    savedUi?.groupBy?.by === "field" && savedUi.groupBy.key ? { by: "field", key: savedUi.groupBy.key } : { by: "state" },
  );
  /** The order inside a column (persisted with the grouping); a stored key this build
   *  does not know falls back to the default. */
  let order = $state<CardOrder>(orderOf(savedUi?.sort));
  function orderOf(saved?: { key: string; desc: boolean }): CardOrder {
    const key = saved && parseSortKey(saved.key);
    return key ? { key, desc: !!saved?.desc } : DEFAULT_ORDER;
  }
  /** The columns follow the grouping: the five state columns, or one per value of the chosen field plus "none". */
  const defs = $derived(columnDefsFor(groupBy, shown));
  const cols = $derived.by(() => {
    const cols = columnsOf(shown, defs, groupBy, order);
    // The card of a drop still landing sits at the top of "scheduled" (it has no date yet to
    // sort by), whichever column its data still puts it in; the write's reload sorts it in
    if (pendingKey && groupBy.by === "state") {
      for (const list of Object.values(cols)) {
        const i = list.findIndex((c) => taskKey(c) === pendingKey);
        if (i < 0) continue;
        cols.scheduled.unshift(...list.splice(i, 1));
        break;
      }
    }
    return cols;
  });
  /** The field keys the tasks carry, for the group-by menu. */
  const fieldKeys = $derived(fieldKeysOf(allCards));
  /** The keys the two menus list: the ones the tasks carry, plus a restored key no task
   *  carries any more — without it the select would show nothing selected. */
  const menuKeys = $derived.by(() => {
    const keys = [...fieldKeys];
    for (const k of [groupBy.by === "field" ? groupBy.key : null, order.key.by === "field" ? order.key.key : null])
      if (k && !keys.includes(k)) keys.push(k);
    return keys;
  });
  const groupSel = $derived(groupBy.by === "state" ? "state" : sortKeyId(groupBy));
  function setGroup(v: string) {
    const key = parseSortKey(v);
    groupBy = key?.by === "field" ? key : { by: "state" };
    collapsed = {};
    persistUi();
  }
  const sortSel = $derived(sortKeyId(order.key));
  /** Picking a key starts it in its natural direction (newest first for the modification time, ascending for the rest); the arrow flips it from there. */
  function setSort(v: string) {
    const key = parseSortKey(v);
    if (!key) return;
    order = { key, desc: defaultDesc(key) };
    persistUi();
  }
  function flipSortDir() {
    order = { key: order.key, desc: !order.desc };
    persistUi();
  }
  /** The note a quick create lands in; it defaults to inbox.md, the same as a quick memo's. */
  let addTarget = $state(savedUi?.target || "inbox.md");

  function persistUi() {
    if (!vault.root) return;
    saveBucket(
      KANBAN_UI_KEY,
      vault.root,
      {
        collapsed: Object.keys(collapsed).filter((id) => collapsed[id]),
        target: addTarget,
        groupBy: $state.snapshot(groupBy),
        sort: { key: sortKeyId(order.key), desc: order.desc },
      },
      vault.recentLoaded ? vault.recent : null,
    );
  }

  function toggleCollapse(id: string) {
    collapsed[id] = !collapsed[id];
    persistUi();
  }

  // ---- Quick create inside a column: the payload goes through quickTaskPayload and lands
  // through vault.insertBlock (the same funnel as clipping and quick memos: when the target is
  // the note that is open it goes through the buffer, avoiding a dirty-write conflict) ----
  let adding = $state<string | null>(null);
  let addText = $state("");
  let addDate = $state(localDate(1));
  let addBusy = false;
  let pickingTarget = $state(false);
  let targetQuery = $state("");

  // The same matcher as the command palette's and the @ candidates': one query has to give the same set of candidates in all three places
  const targetChoices = $derived(fuzzyFilter(vault.notes, targetQuery, noteKey).slice(0, 8));

  function openAdd(col: string) {
    adding = col;
    addText = "";
    addDate = localDate(1);
    pickingTarget = false;
  }

  function closeAdd() {
    adding = null;
    pickingTarget = false;
  }

  async function submitAdd() {
    const col = adding;
    if (!col || addBusy) return;
    const payload = quickTaskPayload(col, addText, localDate(), addDate, groupBy);
    if (!payload) return;
    // A destination restored from localStorage may have been deleted or renamed: note_insert
    // means "create and write" for a missing note, and without checking it would silently
    // rebuild a phantom file (the same consideration as clipTarget.ensureLive). inbox.md is the
    // exception — it is the quick memo's default inbox and should be created when absent
    if (addTarget !== "inbox.md" && !vault.notes.some((n) => n.id === addTarget)) {
      toast.push(t("kanban.targetGone", { path: addTarget }), "error");
      pickingTarget = true;
      return;
    }
    addBusy = true;
    try {
      const r = await vault.insertBlock(addTarget, payload, null);
      if (!r.intoEditor) await vault.refreshNotes();
      addText = ""; // continuous entry: the input stays open, and the card appears by itself once the index event comes back
    } catch (err) {
      toast.push(err instanceof Error ? err.message : String(err), "error");
    } finally {
      addBusy = false;
    }
  }

  /** Focus on mount (the quick-create input, the destination search box). */
  function focusIt(el: HTMLElement) {
    el.focus();
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
    // Closing the menu on Esc or a click outside belongs to TaskDateMenu itself; this only
    // cancels a drag. capture + stop: cancelling a drag must not let App's layered Escape close
    // the kanban view along with it
    if (e.key !== "Escape" || !drag) return;
    e.stopPropagation();
    dragCancel();
    // The click on release lands back on the starting card (pointer capture semantics) and must not open it
    suppressClick = true;
  }}
/>

{#snippet dateChip(card: Card, field: DateField, title: string, text: string, cls: string = "")}
  <!-- pointerdown is stopped: otherwise holding the chip and moving 5px would trigger
    pressCard's drag plus pointer capture (the same discipline as the .check button's); click is
    stopped so the whole card's openCard is not triggered -->
  <button
    class="chip {cls}"
    {title}
    onpointerdown={(e) => e.stopPropagation()}
    onclick={(e) => openChipMenu(e, card, field)}
  >
    {@html dateIconSvg(field)}{text}
  </button>
{/snippet}

<!-- The field keys both selects offer, one spelling of the `field:<key>` option value -->
{#snippet fieldOptions()}
  {#if menuKeys.length}
    <optgroup label={t("kanban.fieldsGroup")}>
      {#each menuKeys as key (key)}
        <option value={sortKeyId({ by: "field", key })}>{key}</option>
      {/each}
    </optgroup>
  {/if}
{/snippet}

<div class="view">
  <ViewHeader
    title={t("view.kanban")}
    total={countLabel(total, hits.length, "task.count")}
    filterPlaceholder={t("task.filterPlaceholder")}
    bind:filter={query}
  >
    {#snippet right()}
      <!-- The grouping: the time axis, or any inline field the tasks carry (one column per value) -->
      <label class="tool">
        <span>{t("kanban.groupBy")}</span>
        <select value={groupSel} onchange={(e) => setGroup(e.currentTarget.value)}>
          <option value="state">{t("kanban.groupByState")}</option>
          {@render fieldOptions()}
        </select>
      </label>
      <!-- The order inside a column. "Default" is each column's own natural order (the
        tooltip says which) and has no direction; every other key applies to every column
        alike, and only then is there an arrow to flip it -->
      <span class="tool">
        <label>
          <span>{t("kanban.sortBy")}</span>
          <select
            value={sortSel}
            title={order.key.by === "auto" ? t("kanban.sortAutoHint") : undefined}
            onchange={(e) => setSort(e.currentTarget.value)}
          >
            <option value="auto">{t("kanban.sortAuto")}</option>
            <option value="date">{t("kanban.sortDate")}</option>
            <option value="modified">{t("kanban.sortModified")}</option>
            <option value="note">{t("kanban.sortNote")}</option>
            <option value="text">{t("kanban.sortText")}</option>
            {@render fieldOptions()}
          </select>
        </label>
        {#if order.key.by !== "auto"}
          <button class="sort-dir" title={order.desc ? t("kanban.sortDesc") : t("kanban.sortAsc")} aria-pressed={order.desc} onclick={flipSortDir}>
            <!-- an arrow beside bars growing (ascending) or shrinking (descending), on the 24 viewBox of the other card icons -->
            <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
              <path d={order.desc ? "m3 16 4 4 4-4M7 20V4M11 4h10M11 8h7M11 12h4" : "m3 8 4-4 4 4M7 4v16M11 12h4M11 16h7M11 20h10"} />
            </svg>
          </button>
        {/if}
      </span>
    {/snippet}
  </ViewHeader>
  {#if total === 0}
    <div class="empty-pad">
      {#if hits.length === 0}
        <ViewEmpty>
          {t("task.empty1")}<code>- [ ] a task 📅 2026-01-01</code>{t("task.empty2")}<code>- [/]</code
          >{t("task.empty3")}<code>⏳ 2026-01-01</code>{t("task.empty4")}
        </ViewEmpty>
      {:else}
        <ViewEmpty onclear={() => (query = "")}>{t("task.noMatch", { query })}</ViewEmpty>
      {/if}
    </div>
  {:else}
    <!-- dragging: while dragging, an empty column grows back to full size so a drop target does not shrink -->
    <div class="board" class:dragging={!!drag}>
      {#each defs as colDef (colDef.id)}
        {@const cards = cols[colDef.id]}
        <section
          class="col"
          class:is-empty={cards.length === 0}
          class:collapsed={collapsed[colDef.id]}
          class:drop-armed={dropTarget === colDef.id}
          data-drop-col={colDef.drop ? colDef.id : undefined}
        >
          <!-- The column header sits outside the .cards scrolling container and naturally does
            not scroll with the cards — do not move it inside the ul.
            Clicking collapses and expands it; collapsed into a strip it is still a valid drop target -->
          <button
            class="col-head"
            class:overdue-title={colDef.id === "overdue" && groupBy.by === "state"}
            title={collapsed[colDef.id] ? t("kanban.expand") : t("kanban.collapse")}
            onclick={() => toggleCollapse(colDef.id)}
          >
            <i class="dot" data-tone={colDef.tone}></i><span class="col-title">{colDef.title}</span><span class="count"
              >{cards.length}</span
            >
          </button>
          {#if collapsed[colDef.id]}
            <!-- The strip placeholder: full height, so the drop hit area does not collapse -->
            <div class="col-strip"></div>
          {:else}
            <ul class="cards">
            {#each cards as card (taskKey(card))}
              {@const pend = pendingKey === taskKey(card) ? pendingDrop : null}
              <!-- The field the pending drop writes: activeField, the same one dropEdit picks -->
              {@const pendField = pend ? activeField(card) : null}
              <li>
                <!-- A done card dragged out is reopened by the drop, and the provisional card already shows it so -->
                <div
                  class="card"
                  class:doing={card.state === "doing"}
                  class:done={card.state === "done" && !pend}
                  class:pending={!!pend}
                  role="button"
                  tabindex="0"
                  onpointerdown={(e) => pressCard(e, card)}
                  onclick={() => void openCard(card)}
                  oncontextmenu={(e) => cardMenu(e, card)}
                  onkeydown={(e) =>
                    e.target === e.currentTarget && (e.key === "Enter" || e.key === " ") && void openCard(card)}
                >
                  <div class="card-main">
                    <button
                      class="check"
                      title={t("task.toggleDone")}
                      onpointerdown={(e) => e.stopPropagation()}
                      onclick={(e) => void onToggle(e, card)}
                    >
                      {@html taskStateIconSvg(pend && card.state === "done" ? "todo" : card.state)}
                    </button>
                    <span class="text">{card.text}</span>
                  </div>
                  <div class="card-meta">
                    {#if pend}
                      <!-- The provisional date chip stands in for the chip of the field the drop
                        writes: an ellipsis while the day is being picked, the day itself until
                        the write lands. A label, not a button — the menu is already open -->
                      <span class="chip pending-chip">{@html dateIconSvg(pendField ?? "due")}{pend.date ?? "…"}</span>
                    {/if}
                    {#if card.due && pendField !== "due"}
                      {@const od = card.col === "overdue"}
                      {@render dateChip(card, "due", t("task.editDateWith", { field: dateFieldLabel("due"), date: card.due }), dueLabel(card.due, od, "short"), od ? "overdue" : "")}
                    {/if}
                    {#if card.scheduled && pendField !== "scheduled"}
                      {@render dateChip(card, "scheduled", t("task.editDate", { field: dateFieldLabel("scheduled") }), card.scheduled)}
                    {/if}
                    {#each card.tags as tag (tag)}
                      <span class="chip tag-chip">#{tag}</span>
                    {/each}
                    {#each card.fieldChips as [key, value] (key)}
                      <span class="chip field-chip" title={`${key}:: ${value}`}><span class="fk">{key}</span>{value}</span>
                    {/each}
                    <span class="chip note-chip">{card.hit.note_title}</span>
                    {#if !card.due && !card.scheduled && card.state !== "done" && !pend}
                      <!-- A card with no date shows a calendar ghost chip on hover: a new field
                        goes through activeField = due. It is absolutely positioned over the
                        checkbox's indent gap in the meta row (26px, structurally always empty)
                        and stays out of the flex flow — hidden with opacity in the flow it would
                        still take space and, with a wide note chip at a large font size, be
                        pushed onto a new line (an extra blank line on the card); floating it in
                        the bottom-right corner would land it in the middle of a wide note chip
                        (three rounds of screenshots proved both) -->
                      {@render dateChip(card, activeField(card), t("task.setDate", { field: dateFieldLabel(activeField(card)) }), "", "add-date")}
                    {/if}
                    {#if vault.privatePaths.has(card.hit.note)}<span class="lock" title={t("statusBar.privacyZoneHint")}><PrivacyIcon size={14} /></span>{/if}
                  </div>
                </div>
              </li>
            {:else}
              <!-- An empty column is a quiet dashed placeholder, and only while dragging does it show "drop here" -->
              <li class="col-empty" class:armed={!!drag && colDef.drop}>
                {drag && colDef.drop ? t("kanban.dropHere") : ""}
              </li>
            {/each}
            </ul>
            {#if colDef.add}
              {#if adding === colDef.id}
                <div class="add-form">
                  <div class="add-line">
                    <input
                      class="add-input"
                      placeholder={t("kanban.newTaskPlaceholder")}
                      bind:value={addText}
                      use:focusIt
                      onkeydown={(e) => {
                        if (e.key === "Enter") {
                          e.preventDefault();
                          void submitAdd();
                        } else if (e.key === "Escape") {
                          // Only the create row closes: it does not bubble to App's layered Escape (which would close the kanban view)
                          e.stopPropagation();
                          closeAdd();
                        }
                      }}
                    />
                    {#if colDef.id === "scheduled" && groupBy.by === "state"}
                      <input type="date" class="add-date" title={t("kanban.scheduleDate")} bind:value={addDate} />
                    {/if}
                  </div>
                  <button
                    class="add-target"
                    title={t("kanban.pickTargetNote")}
                    onclick={() => {
                      pickingTarget = !pickingTarget;
                      targetQuery = "";
                    }}
                  >
                    → {stemOf(addTarget)}
                  </button>
                  {#if pickingTarget}
                    <div class="target-pick">
                      <input
                        placeholder={t("kanban.searchNotes")}
                        bind:value={targetQuery}
                        use:focusIt
                        onkeydown={(e) => {
                          if (e.key === "Escape") {
                            e.stopPropagation();
                            pickingTarget = false;
                          }
                        }}
                      />
                      {#each targetChoices as n (n.id)}
                        <button
                          class="target-item"
                          onclick={() => {
                            addTarget = n.id;
                            pickingTarget = false;
                            persistUi();
                          }}
                        >
                          {n.title}
                        </button>
                      {:else}
                        <div class="target-none">{t("kanban.noMatchingNote")}</div>
                      {/each}
                    </div>
                  {/if}
                </div>
              {:else}
                <button class="add-open" onclick={() => openAdd(colDef.id)}>{t("kanban.newTask")}</button>
              {/if}
            {/if}
          {/if}
        </section>
      {/each}
    </div>
  {/if}
</div>

{#if drag}
  <div class="drag-ghost" style:left="{drag.x + 12}px" style:top="{drag.y + 10}px">{drag.card.text}</div>
{/if}

{#if menu}
  <TaskDateMenu
    x={menu.x}
    y={menu.y}
    field={menu.mode === "chip" ? menu.field : activeField(menu.card)}
    title={menu.mode === "drop" ? t("kanban.dropWhen") : undefined}
    current={menu.mode === "chip" ? menu.current : undefined}
    onpick={onMenuPick}
    onclose={cancelMenu}
  />
{/if}

<style>
  .view {
    height: 100%;
    display: flex;
    flex-direction: column;
    min-height: 0;
  }
  /* ViewEmpty brings its own 20px above and below; the kanban's empty state hangs straight
     under the header without .body's horizontal padding, which this layer supplies */
  .empty-pad {
    padding: 0 20px;
  }
  /* A flex-weight layout: columns with cards share the space evenly and empty ones narrow to give way; on a very narrow viewport the whole board still scrolls horizontally as a backstop */
  .board {
    flex: 1;
    min-height: 0;
    display: flex;
    gap: 10px;
    padding: 10px 20px 20px;
    overflow-x: auto;
    overscroll-behavior-x: contain;
  }
  .col {
    flex: 1 1 0;
    min-width: 176px;
    display: flex;
    flex-direction: column;
    min-height: 0;
    border-radius: 8px;
    padding: 4px;
    transition:
      flex-grow 0.15s ease,
      min-width 0.15s ease;
    /* The drop highlight uses outline: it does not squeeze the content */
  }
  .board.dragging,
  .board.dragging .card {
    cursor: grabbing;
  }
  /* An empty column narrows (its header "Scheduled 0" stays fully readable); while dragging it
    grows back to full size so a drop target does not shrink. A collapsed column is excluded —
    collapsing wins over the empty narrowing */
  .board:not(.dragging) .col.is-empty:not(.collapsed) {
    flex-grow: 0.35;
    min-width: 108px;
  }
  /* Collapsed by hand: a vertical strip that is still a valid drop target (data-drop-col is kept, and the armed highlight works as usual) */
  .col.collapsed {
    flex: 0 0 auto;
    min-width: 0;
    width: 34px;
    padding: 4px 2px;
  }
  .col.collapsed .col-head {
    writing-mode: vertical-rl;
    padding: 8px 2px;
    gap: 8px;
    align-items: center;
  }
  /* Inside the strip the count badge returns to horizontal writing: following vertical-rl would
    turn it into a horizontal capsule poking out of the 34px strip (screenshots caught "20"
    overflowing and "13" being clipped) */
  .col.collapsed .count {
    writing-mode: horizontal-tb;
  }
  .col-strip {
    flex: 1;
  }
  .col.drop-armed {
    outline: 2px solid var(--accent);
    outline-offset: -2px;
    background: var(--accent-soft);
  }
  .col-head {
    display: flex;
    align-items: center;
    gap: 6px;
    font-size: var(--fs-sm);
    font-weight: 600;
    color: var(--fg-muted);
    padding: 4px 6px 8px;
    white-space: nowrap;
    border-radius: 6px;
    text-align: left;
  }
  .col-head:hover {
    background: var(--bg-hover);
  }
  .overdue-title {
    color: var(--danger);
  }
  /* The column header's semantic colour dot: overdue danger / today accent / scheduled muted / done faint */
  .dot {
    width: 6px;
    height: 6px;
    border-radius: 50%;
    flex-shrink: 0;
  }
  .dot[data-tone="danger"] {
    background: var(--danger);
  }
  .dot[data-tone="accent"] {
    background: var(--accent);
  }
  .dot[data-tone="muted"] {
    background: var(--fg-muted);
  }
  .dot[data-tone="faint"] {
    background: var(--fg-faint);
  }
  .count {
    font-size: var(--fs-xs);
    font-weight: 400;
    color: var(--fg-muted);
    background: var(--bg-hover);
    border-radius: 999px;
    padding: 0 6px;
    line-height: 1.6;
  }
  .cards {
    list-style: none;
    /* It does not fill the column's height: it fits its content and only scrolls when too long
      — "+ new task" has to sit right under the last card (flex:1 would push it to the very
      bottom of the column, half a screen below the cards when there are few) */
    flex: 0 1 auto;
    min-height: 0;
    overflow-y: auto;
    /* Horizontal scrolling is turned off explicitly: with overflow-y alone, overflow-x is
      computed as auto, and one or two pixels of overflow would raise a column-level horizontal
      scrollbar */
    overflow-x: hidden;
    /* Scrolling a column to its end does not carry the board or the page along */
    overscroll-behavior: contain;
    display: flex;
    flex-direction: column;
    gap: 8px;
    padding: 2px;
  }
  /* A thin scrollbar inside a column (the global 8px is too loud) */
  .cards::-webkit-scrollbar {
    width: 6px;
  }
  .cards::-webkit-scrollbar-thumb {
    background: var(--border);
    border-radius: 3px;
  }
  .card {
    /* relative: the positioning anchor of the ghost "+" chip */
    position: relative;
    background: var(--bg-panel);
    border: 1px solid var(--border);
    border-radius: 6px;
    padding: 8px 10px;
    font-size: var(--fs-sm);
    cursor: pointer;
    -webkit-user-select: none;
    user-select: none;
  }
  /* Hover changes the colour and not the shape: a shadow is too heavy, and a transform would disturb the drag's elementFromPoint lookup */
  .card:hover {
    background: var(--bg-hover);
    border-color: var(--fg-faint);
  }
  /* doing is orthogonal to the dates: an accent bar on the left edge plus the half-filled check icon, already sorted to the top of its column */
  .card.doing {
    border-left: 3px solid var(--accent);
  }
  /* A done card is quietened as a whole and becomes readable again on hover */
  .card.done {
    opacity: 0.7;
  }
  .card.done:hover {
    opacity: 1;
  }
  .card.done .text {
    color: var(--fg-faint);
    text-decoration: line-through;
  }
  /* The card of a drop still landing (pendingDrop): a dashed accent frame says "provisional",
     kept through the write's few milliseconds until the reload draws the card plain */
  .card.pending,
  .card.pending:hover {
    border-color: var(--accent);
    border-style: dashed;
  }
  .card-main {
    display: flex;
    align-items: flex-start;
    gap: 6px;
  }
  .check {
    flex-shrink: 0;
    font-size: var(--fs-md);
    color: var(--fg-muted);
    padding: 2px;
    border-radius: 4px;
    display: inline-flex;
  }
  .check:hover {
    color: var(--accent);
    background: var(--bg-active);
  }
  .check :global(svg) {
    width: 1em;
    height: 1em;
    display: block;
  }
  .text {
    min-width: 0;
    /* Truncated to two lines: the card's height stays controlled and a long task does not burst the column */
    display: -webkit-box;
    -webkit-line-clamp: 2;
    line-clamp: 2;
    -webkit-box-orient: vertical;
    overflow: hidden;
  }
  .card-meta {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 4px;
    margin-top: 6px;
    padding-left: 26px;
  }
  .chip {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    /* A chip that cannot be held used to be drawn outside the card (measured: with a longer
       overdue string it was clipped at the column's edge): sealed inside the card, what
       overflows is the chip's own text rather than the layout */
    max-width: 100%;
    overflow: hidden;
    flex-shrink: 0;
    font-size: var(--fs-xs);
    color: var(--fg-muted);
    background: var(--bg-hover);
    border-radius: 4px;
    padding: 1px 6px;
  }
  .chip.overdue {
    color: var(--danger);
  }
  /* The provisional date chip: the ghost chip's dashed style, in accent */
  .chip.pending-chip {
    border: 1px dashed var(--accent);
    color: var(--accent);
    background: var(--accent-soft);
    padding: 0 5px;
  }
  .chip :global(svg) {
    width: 0.9em;
    height: 0.9em;
    flex-shrink: 0;
  }
  /* A date chip is a button (clickable to reschedule); note and tag chips are still spans */
  button.chip:hover {
    color: var(--accent);
    background: var(--bg-active);
  }
  /* A card with no date shows the calendar icon only on hover; always showing it would fill a
    column with plus signs (the same reason as Agenda's).
    It is absolutely positioned over the checkbox's indent gap at the start of the meta row
    (card-meta's padding-left of 26px, structurally always blank), stays entirely out of the
    layout flow and overlaps no chip — hidden with opacity in the flow it would still take
    space and force a wrap, and floating it in the bottom-right corner would land it on a wide
    note chip; both were tried and failed.
    The selector carries .chip to raise its specificity: button.chip:hover must not outweigh
    this state */
  .chip.add-date {
    position: absolute;
    left: 10px;
    bottom: 7px;
    opacity: 0;
    pointer-events: none;
    border: 1px dashed var(--fg-faint);
    background: var(--bg-hover);
    color: var(--fg-muted);
    padding: 0 5px;
  }
  .card:hover .add-date,
  .add-date:focus-visible {
    opacity: 1;
    pointer-events: auto;
  }
  .chip.add-date:hover {
    color: var(--accent);
    border-color: var(--accent);
    background: var(--bg-hover);
  }
  /* An inline #tag: an accent tint, set apart from the note-name chip (muted). A text chip has
    to be able to shrink (overriding .chip's flex-shrink:0, which is there for the date chips):
    otherwise, on a narrow card whose max-width exceeds the space left on the row, the chip
    neither shrinks nor is clipped and runs straight through the padding into the card's border
    (proved by a screenshot); once it may shrink, the ellipsis appears naturally where the row
    runs out of width */
  .tag-chip {
    color: var(--accent);
    background: var(--accent-soft);
    max-width: 90px;
    flex-shrink: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    display: inline-block;
  }
  .note-chip {
    max-width: 140px;
    flex-shrink: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    display: inline-block;
  }
  .lock {
    display: inline-flex;
    flex-shrink: 0;
    color: var(--accent);
  }
  /* An empty column is normally a quiet dashed placeholder (it does not say "no tasks"), and while armed for a drag it lights up with accent and a hint */
  .col-empty {
    border: 1px dashed var(--border);
    border-radius: 6px;
    min-height: 44px;
    display: flex;
    align-items: center;
    justify-content: center;
    font-size: var(--fs-xs);
    color: var(--fg-faint);
  }
  .col-empty.armed {
    border-color: var(--accent);
    color: var(--accent);
  }
  /* ---- Quick create inside a column: the button lives at the bottom of the column and does not scroll away with the cards ---- */
  .add-open {
    margin-top: 6px;
    padding: 6px 8px;
    border-radius: 6px;
    font-size: var(--fs-xs);
    color: var(--fg-faint);
    text-align: left;
  }
  .add-open:hover {
    color: var(--fg-muted);
    background: var(--bg-hover);
  }
  .add-form {
    /* relative: the positioning anchor of the destination picker overlay */
    position: relative;
    margin-top: 6px;
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .add-line {
    display: flex;
    gap: 4px;
  }
  .add-input {
    flex: 1;
    min-width: 0;
    padding: 5px 8px;
    font-size: var(--fs-sm);
  }
  .add-date {
    padding: 3px 6px;
    font-size: var(--fs-xs);
  }
  .add-target {
    align-self: flex-start;
    max-width: 100%;
    font-size: var(--fs-xs);
    color: var(--fg-faint);
    padding: 1px 6px;
    border-radius: 4px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .add-target:hover {
    color: var(--accent);
    background: var(--bg-hover);
  }
  /* The destination picker pops above the form: the space left at the bottom of a column is often not enough */
  .target-pick {
    position: absolute;
    bottom: 100%;
    left: 0;
    right: 0;
    margin-bottom: 4px;
    z-index: 100;
    background: var(--bg-panel);
    border: 1px solid var(--border);
    border-radius: 8px;
    box-shadow: var(--shadow);
    padding: 4px;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .target-pick input {
    padding: 4px 8px;
    font-size: var(--fs-xs);
  }
  .target-item {
    text-align: left;
    padding: 4px 8px;
    border-radius: 5px;
    font-size: var(--fs-sm);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .target-item:hover {
    background: var(--bg-hover);
  }
  .target-none {
    padding: 4px 8px;
    font-size: var(--fs-xs);
    color: var(--fg-faint);
  }
  .drag-ghost {
    position: fixed;
    z-index: 400;
    pointer-events: none;
    max-width: 220px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    background: var(--bg-panel);
    border: 1px solid var(--border);
    border-radius: 6px;
    box-shadow: var(--shadow);
    padding: 4px 10px;
    font-size: var(--fs-xs);
  }
  /* The header's tools (group by, sort by): a caption plus a select, and for the sort a
     direction button pressed against its select — the 6px inside a tool against the
     header's 10px between tools is what keeps the arrow read as part of "sort by" */
  .tool,
  .tool label {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    font-size: var(--fs-xs);
    color: var(--fg-faint);
  }
  .tool select {
    font-size: var(--fs-xs);
    padding: 2px 4px;
    border: 1px solid var(--border);
    border-radius: 5px;
    background: var(--bg);
    color: var(--fg);
  }
  .sort-dir {
    display: grid;
    place-items: center;
    width: 22px;
    height: 22px;
    border: 1px solid var(--border);
    border-radius: 5px;
    background: var(--bg);
    color: var(--fg-muted);
  }
  .sort-dir:hover {
    background: var(--bg-hover);
    color: var(--fg);
  }
  .sort-dir svg {
    width: 14px;
    height: 14px;
  }
  .field-chip .fk {
    color: var(--fg-faint);
    margin-right: 3px;
  }
  .field-chip .fk::after {
    content: ":";
  }
</style>
