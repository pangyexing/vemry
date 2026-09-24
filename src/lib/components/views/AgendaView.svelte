<script lang="ts">
  import { tick } from "svelte";
  import { api, type BlockQuery, type BlockHit } from "$lib/api";
  import { viewContext } from "$lib/stores/viewContext.svelte";
  import { askAboutView } from "$lib/askview";
  import { t } from "$lib/i18n/i18n.svelte";
  import { fuzzyKeep } from "$lib/commands/registry";
  import { vault } from "$lib/stores/vault.svelte";
  import { byDate, byModified, byPriority, dateFieldLabel, type DateField, dateIconSvg, deleteTask, dueLabel, EDIT_FIELDS, type EditField, estimateSum, formatEstimate, isBlocked, isClosed, isUrgent, localDate, nearDate, openIds, placeTask, priorityLabel, rescheduleTask, rowOf, TaskKeys, taskFilterKey, taskMenuEntries, type TaskPhase, type TaskRow, taskStateIconSvg, toggleTask, updateTask } from "$lib/views/tasks";
  import { fieldChipsOf, type FieldChip } from "$lib/views/blockrow";
  import { stateGlyphSvg } from "$lib/chips";
  import { types } from "$lib/stores/types.svelte";
  import { commitTaskWrite, copyTaskId } from "$lib/views/taskwrite";
  import { contextMenu } from "$lib/stores/contextMenu.svelte";
  import { cliptip } from "$lib/cliptip";
  import { holdAnchor } from "$lib/scrollanchor";
  import Chevron from "../Chevron.svelte";
  import TaskDateMenu from "./TaskDateMenu.svelte";
  import ViewHeader, { countLabel } from "./ViewHeader.svelte";
  import ViewEmpty from "./ViewEmpty.svelte";
  import PrivacyIcon from "../PrivacyIcon.svelte";

  /** The task hits and their identities (TaskKeys), in one assignment: a row is built
   *  from both, so they must never be read from two different queries. */
  let data = $state.raw<{ hits: BlockHit[]; keys: string[] }>({ hits: [], keys: [] });
  const ids = new TaskKeys();
  /** The scrolling list: a requery keeps its first visible row where it was. */
  let bodyEl = $state<HTMLElement>();
  // With events firing in quick succession several loads run at once and an older response may arrive later: only the newest one lands
  let loadSeq = 0;

  /** A requery comes from anywhere — an autosave, an AI write, a phone capture landing, a
   *  tick here — and none of it is the reader scrolling: the row at the top of the list
   *  keeps its place on screen while rows above it come and go (scrollanchor.ts). */
  async function load() {
    const seq = ++loadSeq;
    try {
      const res = await api.queryBlocks({ kind: "task", limit: 5000 });
      if (seq !== loadSeq) return;
      const keys = ids.assign(res);
      await holdAnchor(bodyEl, () => void (data = { hits: res, keys }), tick);
    } catch (e) {
      vault.error = String(e);
    }
  }

  $effect(() => {
    void vault.indexVersion;
    void load();
  });

  // Building a row, placing it and the ordering all live in views/tasks.ts: the kanban
  // reads the same rules, so the two views cannot disagree about a task (Row is the local
  // alias of TaskRow over there)
  type Row = TaskRow;

  let query = $state("");
  /** The second filter, set from the legend: only the tasks carrying this date are shown,
   *  and they are shown by it — the groups and the order inside them read that date instead
   *  of the due-first default, so "start date" is a real view of when things start rather
   *  than the due-date view minus some rows. One kind at a time: a task with both dates is
   *  found by either, so a "both" state would only ever be a stricter version of one of
   *  them. */
  let dateKind = $state<EditField>();

  // What this view shows is attachable to the AI (the @ picker's "current view", the
  // header's "Ask AI"): the live query and the row count are registered while mounted.
  // The text filter is applied by Rust's query as a substring match, the closest the
  // query API has to the fuzzy filter here.
  $effect(() => {
    const q: BlockQuery = { kind: "task", limit: 5000 };
    if (query.trim()) q.text = query.trim();
    if (dateKind) q.field = dateKind;
    viewContext.set({ view: "agenda", label: query.trim() ? `${t("view.agenda")} · ${query.trim()}` : t("view.agenda"), query: q, rows: shown.length });
    return () => viewContext.set(null);
  });

  /** The rows are built before they are filtered: the grouping reads the state and dates
   *  rowOf gave, the filtering reads taskFilterKey, and both share one Row — so rowOf runs
   *  once per block. The key holds the note's title and path, since "what else is unfinished
   *  in this note" is the most common way of searching. */
  const allRows = $derived(data.hits.map((h, i) => rowOf(h, data.keys[i])));
  const shown = $derived.by(() => {
    const kept = fuzzyKeep(allRows, query, taskFilterKey);
    const kind = dateKind;
    return kind ? kept.filter((r) => r[kind]) : kept;
  });
  /** The one way back from an empty list: either filter may be the one in the way, and a
   *  button that cleared only the other would do nothing when pressed */
  const clearFilters = () => {
    query = "";
    setDateKind(undefined);
  };

  /** The day the rows are read against, taken once per requery: the grouping and the dates
   *  of one render agree on it. */
  const today = $derived.by(() => {
    void data;
    return localDate();
  });

  /** The groups, in their order on screen: overdue / today / the next 7 days / later /
   *  not started yet / in progress / no date / done (cancelled included) — the phases
   *  tasks.ts's placeTask gives, which the kanban's columns are cut from too. */
  const PHASES: TaskPhase[] = ["overdue", "today", "week", "later", "future", "doing", "nodate", "done"];

  /** Rows the reader changed from this view — ticked, set to a state, given a date — and
   *  where they stood: each stays in its group, at its place (sorted as it was, `as`),
   *  showing what it is now, until the view closes. The reader sees the result of the
   *  click where they clicked, and no other row moves under the pointer; the next visit
   *  files it where it belongs. Plain rather than reactive: it is written just before the
   *  write whose requery reads it. */
  const stay = new Map<string, { group: TaskPhase; as: Row }>();
  function hold(r: Row) {
    if (!stay.has(r.key)) stay.set(r.key, { group: placeTask(r, today, dateKind).phase, as: r });
  }
  /** Pressing a legend entry regroups every row by another date: the rows held in the
   *  previous grouping are let go with it. */
  function setDateKind(kind: EditField | undefined) {
    dateKind = kind;
    stay.clear();
  }

  const groups = $derived.by(() => {
    const kind = dateKind;
    const g = Object.fromEntries(PHASES.map((p) => [p, [] as { row: Row; as: Row }[]])) as Record<TaskPhase, { row: Row; as: Row }[]>;
    for (const r of shown) {
      const s = stay.get(r.key);
      g[s?.group ?? placeTask(r, today, kind).phase].push({ row: r, as: s?.as ?? r });
    }
    // Each group is sorted once: what is marked urgent comes first, then the two with no
    // date to read follow the task's own change time, most recently changed first, and the
    // rest go by date. With a date kind pressed every group goes by that date — it is the
    // order that was asked for, the done group included
    const byWhen = byPriority(byDate(kind));
    const out = {} as Record<TaskPhase, Row[]>;
    for (const p of PHASES) {
      const cmp = !kind && (p === "nodate" || p === "done") ? byPriority(byModified) : byWhen;
      out[p] = g[p].sort((a, b) => cmp(a.as, b.as)).map((e) => e.row);
    }
    return out;
  });

  /** The groups that had rows at some point of this visit, under the current filters: a
   *  group whose last row went elsewhere (done in another note, moved by an AI write)
   *  keeps its heading, at 0, rather than taking the rows below it up by a heading's
   *  height. A filter change is the reader's own regrouping and starts afresh. */
  let kept = { sig: "", names: new Set<TaskPhase>() };
  const keptGroups = $derived.by(() => {
    const sig = `${query}\u0000${dateKind ?? ""}`;
    if (kept.sig !== sig) kept = { sig, names: new Set() };
    for (const p of PHASES) if (groups[p].length) kept.names.add(p);
    return new Set(kept.names);
  });

  /** The ids the open tasks carry: what a ⛔ can still wait on. */
  const open = $derived(openIds(allRows));
  /** A row's field chips, derived once per render pass rather than inside the row's
   *  markup: fieldChipsOf walks every enabled type for every field of the block, and a
   *  hover or a menu opening would otherwise rerun that for every row on screen. */
  const chipsOf = $derived.by(() => {
    const types_ = types.enabled;
    const m = new Map<string, FieldChip[]>();
    for (const r of shown) m.set(r.key, fieldChipsOf(r.hit, types_));
    return m;
  });
  /** Each note's language (the index's reading of it): a row's words are set in their
   *  note's language, so a Japanese note's task takes Japanese glyphs and line rules
   *  whatever the interface is. */
  /** The estimates of the day added up, shown beside the group's count when any row has one. */
  const todayEstimate = $derived.by(() => {
    const sum = estimateSum(groups.today);
    return sum > 0 ? t("agenda.estimate", { v: formatEstimate(sum) }) : "";
  });

  const total = $derived(shown.length);

  /** Whether a task's due date reads as late is the task's own: its due date has passed
   *  and it is not done. In the default view that is the same as sitting in the overdue
   *  group (a task's date is its due date whenever it has one); with the start date
   *  pressed the group follows the start date, and a task that started last month but is
   *  due next week must not be painted late. */
  const isLate = (r: Row) => !isClosed(r.state) && !!r.due && r.due < today;

  /** The write funnel every path shares (taskwrite.ts, the kanban's too): the dirty guard, a requery on a stale span and error reporting. */
  const commit = (hit: BlockHit, act: string, write: () => Promise<boolean>) => commitTaskWrite(hit, act, write, load);

  function onToggle(e: Event, r: Row) {
    e.stopPropagation();
    hold(r);
    void commit(r.hit, t("task.act.toggle"), () => toggleTask(r.hit));
  }

  /** The reschedule menu a date opens; an empty current means the task does not have that date yet */
  let dateMenu = $state.raw<{ x: number; y: number; row: Row; field: DateField; current?: string } | null>(null);

  function openDateMenu(e: MouseEvent, r: Row, field: DateField) {
    e.stopPropagation();
    dateMenu = { x: e.clientX, y: e.clientY, row: r, field, current: r[field] };
  }

  /** The row a menu is about keeps its hover colour while the menu is open. A date's
   *  menu hangs off the date and the row is plain to see; the context menu, and the date
   *  menu it opens, float at the right-click with the pointer gone from the row — and a
   *  "start date: today / tomorrow" with no row lit says nothing about which task it is
   *  setting. The context menu remembers what it was opened for (owner) until it closes. */
  const heldKey = $derived(dateMenu ? dateMenu.row.key : contextMenu.owner);

  function applyDate(date: string | null) {
    const m = dateMenu;
    dateMenu = null;
    if (!m) return;
    hold(m.row);
    void commit(m.row.hit, t("task.act.editDate"), () => rescheduleTask(m.row.hit, m.field, date));
  }

  /** A row's context menu: the entries are the kanban card's (taskMenuEntries), and the
   *  date entries open the same TaskDateMenu a date does, at the right-click. */
  function rowMenu(e: MouseEvent, r: Row) {
    e.stopPropagation();
    const at = { x: e.clientX, y: e.clientY };
    contextMenu.open(
      e,
      taskMenuEntries(r, {
        open: () => void openTask(r.hit),
        editDate: (field, current) => (dateMenu = { ...at, row: r, field, current }),
        setState: (state, act) => {
          hold(r);
          void commit(r.hit, act, () => updateTask(r.hit, { state }));
        },
        copyId: () => void copyTaskId(r, commit),
        remove: () => void commit(r.hit, t("task.act.remove"), () => deleteTask(r.hit)),
      }),
      r.key,
    );
  }

  /** Clicking a task jumps to its line. The kanban uses the same vault.openBlockAt — the
   *  byte → UTF-16 conversion and rechecking currentPath after the await are contracts kept
   *  in one place. */
  const openTask = (hit: BlockHit) => vault.openBlockAt(hit.note, hit.block.span.start);

  /** How a date reads in a row: an overdue due date as its lateness, every other as a
   *  named day or a short date — the kanban's words (D8). Today's date is not written in
   *  the today group: the heading says it. */
  function dateWord(r: Row, field: DateField, date: string): string {
    return field === "due" && isLate(r) ? dueLabel(date, true, "short") : nearDate(date);
  }
  /** A date's tone, which is what tells the three apart on a row (the legend in the
   *  header names them): the due date muted, or late; the scheduled day in the accent;
   *  the start day in the go colour. A finished row's dates recede with its words. */
  function dateTone(r: Row, field: DateField): string {
    if (isClosed(r.state)) return "";
    if (field === "due") return isLate(r) ? "late" : "";
    return field === "scheduled" ? "scheduled" : field === "start" ? "start" : "";
  }
</script>

{#snippet dateButton(r: Row, field: DateField, date: string, cls: string)}
  <!-- The date as a word; the full date is the tip (and what a click edits) -->
  {@const tip = t("task.editDateWith", { field: dateFieldLabel(field), date })}
  <button class="date {cls} {dateTone(r, field)}" aria-label={tip} use:cliptip={{ text: tip, always: true }} onclick={(e) => openDateMenu(e, r, field)}
    >{dateWord(r, field, date)}</button
  >
{/snippet}

{#snippet taskRow(r: Row, group: TaskPhase)}
  {@const closed = isClosed(r.state)}
  {@const blocked = isBlocked(r, open)}
  {@const main = placeTask(r, today, dateKind).field}
  {@const mainDate = main ? r[main] : undefined}
  <li data-anchor={r.key}>
    <div
      class="row"
      class:held={r.key === heldKey}
      class:blocked
      class:closed
      role="button"
      tabindex="0"
      onclick={() => void openTask(r.hit)}
      oncontextmenu={(e) => rowMenu(e, r)}
      onkeydown={(e) =>
        e.target === e.currentTarget && (e.key === "Enter" || e.key === " ") && void openTask(r.hit)}
    >
      <button class="check" aria-label={t("task.toggleDone")} onclick={(e) => onToggle(e, r)}>
        {@html taskStateIconSvg(r.state)}
      </button>
      <!-- The words take the row's width, clipped at their logical end (dir=auto: a
        right-to-left task keeps its opening words); the whole text is cliptip's, laid
        over the clipped line a quarter second into the hover -->
      <span class="text" lang={vault.noteById.get(r.hit.note)?.lang ?? undefined} dir="auto" use:cliptip={r.text}>{r.text}</span>
      <!-- The date column: one date, the one that placed the row in its group, at a fixed
        width so the column stands still whatever the filter leaves -->
      <span class="when">
        {#if main && mainDate}
          {@render dateButton(r, main, mainDate, group === "today" && mainDate === today ? "quiet" : "")}
        {:else if !closed}
          <button class="date add-date" aria-label={t("task.setDate", { field: dateFieldLabel(dateKind ?? "due") })} use:cliptip={{ text: t("task.setDate", { field: dateFieldLabel(dateKind ?? "due") }), always: true }} onclick={(e) => openDateMenu(e, r, dateKind ?? "due")}>+</button>
        {/if}
      </span>
      <span class="meta-cell">
        <!-- The rest of the task as one line of secondary text ending in its source note:
          the other dates, the attributes, the fields and the tags, each once. One
          ellipsis runs across it, and the tip reads the whole line -->
        <span class="meta" use:cliptip={{}}>
          {#if r.priority !== "normal"}<span class="mi" class:late={isUrgent(r.priority) && !closed}>{priorityLabel(r.priority)}</span><span class="sep">{" · "}</span>{/if}
          {#if blocked}<span class="mi late">{t("task.chip.blocked")}</span><span class="sep">{" · "}</span>{/if}
          {#each EDIT_FIELDS as f (f)}
            {#if f !== main && r[f]}{@render dateButton(r, f, r[f]!, "mi")}<span class="sep">{" · "}</span>{/if}
          {/each}
          {#if r.repeat}<span class="mi">{r.repeat}</span><span class="sep">{" · "}</span>{/if}
          {#if r.estimate}<span class="mi">{t("task.chip.estimate", { v: formatEstimate(r.estimate) })}</span><span class="sep">{" · "}</span>{/if}
          {#if r.deleteOnDone}<span class="mi">{t("task.chip.deleteOnDone")}</span><span class="sep">{" · "}</span>{/if}
          <!-- The inline fields (views/blockrow.ts's one derivation): a state field with its
            step's glyph, the rest as key: value — the board shows the same -->
          {#each chipsOf.get(r.key) ?? [] as f (f.key)}
            {#if f.kind === "state"}
              <span class="mi"><i class="glyph">{@html stateGlyphSvg(f.step, f.of)}</i>{f.value}</span><span class="sep">{" · "}</span>
            {:else}
              <span class="mi">{f.key}: {f.value}</span><span class="sep">{" · "}</span>
            {/if}
          {/each}
          {#each r.tags as tag (tag)}<span class="mi">#{tag}</span><span class="sep">{" · "}</span>{/each}
          <span class="mi">{r.hit.note_title}</span>
        </span>
        {#if vault.privatePaths.has(r.hit.note)}<span class="lock" aria-label={t("statusBar.privacyZoneHint")} use:cliptip={{ text: t("statusBar.privacyZoneHint"), always: true }}><PrivacyIcon size={14} /></span>{/if}
      </span>
    </div>
  </li>
{/snippet}

{#snippet dateKindToggle(field: EditField)}
  <!-- A legend entry names its date by word and by the tone the rows write it in; pressed,
    it is also the filter. It says what it is in words, so it carries no tip -->
  <button class="kind {field}" aria-pressed={dateKind === field} onclick={() => setDateKind(dateKind === field ? undefined : field)}>
    {@html dateIconSvg(field)}{dateFieldLabel(field)}
  </button>
{/snippet}

{#snippet section(group: TaskPhase, title: string, collapsed: boolean, tone: "" | "late" | "today" = "", extra: string = "")}
  {@const rows = groups[group]}
  {#if rows.length || keptGroups.has(group)}
    <details open={!collapsed}>
      <summary>
        <!-- summary is display:flex, and the UA's disclosure triangle only renders on
          display:list-item, so it disappeared with it — the collapsed and expanded states used
          to look identical ("Later 3" had a count but showed no rows and looked broken). This
          chevron is the only collapse indicator -->
        <span class="chevron"><Chevron /></span>
        <span class="title {tone}">{title}</span><span class="count">{rows.length}</span>
        {#if extra}<span class="extra">{extra}</span>{/if}
      </summary>
      <ul>
        {#each rows as r (r.key)}
          {@render taskRow(r, group)}
        {/each}
      </ul>
    </details>
  {/if}
{/snippet}

<div class="view">
  <ViewHeader title={t("view.agenda")} total={countLabel(total, allRows.length, "task.count")} filterPlaceholder={t("task.filterPlaceholder")} bind:filter={query} askAi={askAboutView}>
    {#snippet left()}
      <!-- The legend doubles as a filter. As a legend it names the tones the rows write
        their dates in (a row carries no icons: they are here, once); pressed, an entry
        keeps only the tasks carrying that date, placed and ordered by it, and the count
        beside it shows how many that left. It sits on the left, by the count it narrows,
        rather than among the tools on the right -->
      <span class="legend">
        {@render dateKindToggle("due")}
        {@render dateKindToggle("scheduled")}
        {@render dateKindToggle("start")}
      </span>
    {/snippet}
  </ViewHeader>
  <div class="body" bind:this={bodyEl}>
    {#if data.hits.length === 0}
      <ViewEmpty>
        {t("task.empty1")}<code>- [ ] a task 📅 2026-01-01</code>{t("task.empty2")}<code>- [/]</code
        >{t("task.empty3")}<code>⏳ 2026-01-01</code>{t("task.empty4")}
      </ViewEmpty>
    {:else if total === 0 && !query.trim() && dateKind}
      <ViewEmpty onclear={clearFilters}>{t("task.noneWithDate", { field: dateFieldLabel(dateKind) })}</ViewEmpty>
    {:else if total === 0}
      <ViewEmpty onclear={clearFilters}>{t("task.noMatch", { query })}</ViewEmpty>
    {:else}
      {@render section("overdue", t("agenda.overdue"), false, "late")}
      {@render section("today", t("agenda.today"), false, "today", todayEstimate)}
      {@render section("week", t("agenda.week"), false)}
      {@render section("later", t("agenda.later"), true)}
      {@render section("future", t("agenda.future"), true)}
      {@render section("doing", t("agenda.doing"), false)}
      {@render section("nodate", t("agenda.nodate"), false)}
      {@render section("done", t("agenda.done"), true)}
    {/if}
  </div>
</div>

{#if dateMenu}
  <TaskDateMenu
    x={dateMenu.x}
    y={dateMenu.y}
    field={dateMenu.field}
    current={dateMenu.current}
    onpick={(date) => void applyDate(date)}
    onclose={() => (dateMenu = null)}
  />
{/if}

<style>
  .view {
    height: 100%;
    display: flex;
    flex-direction: column;
    min-height: 0;
    /* The task name's size: the reader's note size, a step down — a list to scan is not
       a page to read, but it follows the setting (18 → 15.75 → 16px) rather than staying
       at 13px whatever the reader chose. Whole pixels, like every size here. The row's
       tracks are in this em */
    --ag-fs: round(nearest, calc(var(--fs-note) * 0.875), 1px);
    /* The date tones: late in the danger colour, the start day in the go colour, each
       drawn a fifth of the way to the text colour — as they are, the two fall under
       4.5:1 on a hovered row (4.24 in dark, 4.23 in light); mixed, 4.98 and 5.50 */
    --ag-late: color-mix(in srgb, var(--danger) 80%, var(--fg));
    --ag-start: color-mix(in srgb, var(--ok) 80%, var(--fg));
  }
  .legend {
    display: flex;
    gap: 6px;
  }
  /* An entry: the date's icon in its row tone, then its name; pressed, it takes the
     accent — a hover only lifts the background, so the pressed one stays
     distinguishable from a hovered one */
  .kind {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    white-space: nowrap;
    font-size: var(--fs-xs);
    color: var(--fg-muted);
    border-radius: 4px;
    padding: 1px 6px;
  }
  .kind:hover {
    background: var(--bg-hover);
  }
  .kind[aria-pressed="true"] {
    color: var(--accent);
    background: var(--accent-soft);
  }
  /* dateIconSvg is injected with {@html}, and the svg carries no scope marker, so its size is matched with :global */
  .kind :global(svg) {
    width: 0.95em;
    height: 0.95em;
  }
  .kind.scheduled :global(svg) {
    color: var(--accent);
  }
  .kind.start :global(svg) {
    color: var(--ag-start);
  }
  /* The list scrolls sideways only when the pane is narrower than the table's floor (the
     sum of the tracks' minimums below, about 330px at the default size), the way the
     board does when its columns no longer fit */
  .body {
    flex: 1;
    overflow: auto;
    overscroll-behavior-x: contain;
    padding: 10px 20px 30px;
  }
  /* Each group is at least as wide as its rows' minimum, so it is the groups that overflow
     the pane and scroll, not the tracks that collapse inside a row. At most 60em of the
     task name's size (840px at the default): on a wide screen the date and the source stay
     within a glance of the task, the list left-aligned under the header, the extra width
     to its right */
  details {
    margin-bottom: 6px;
    min-width: min-content;
    max-width: calc(var(--ag-fs) * 60);
  }
  /* A group's heading stays at the top of the pane while its rows scroll under it (the
     timeline's day headings do the same): with 24 undated and 77 done tasks the eye
     needs to know which group it is in. sticky is bounded by the details element, so a
     heading yields to the next group's rather than stacking; the shadow backs the pane's
     10px top padding so a row's text does not peek out above the heading */
  summary {
    display: flex;
    align-items: baseline;
    gap: 8px;
    cursor: pointer;
    padding: 6px 4px;
    font-size: var(--fs-sm);
    font-weight: 600;
    -webkit-user-select: none;
    user-select: none;
    position: sticky;
    top: 0;
    z-index: 1;
    background: var(--bg);
    box-shadow: 0 -10px 0 0 var(--bg);
  }
  /* The same collapse language as the file tree's and the outline panel's: collapsed points right, expanded turns 90° */
  .chevron {
    display: inline-flex;
    align-self: center;
    color: var(--fg-faint);
    transition: transform 0.12s;
  }
  details[open] > summary .chevron {
    transform: rotate(90deg);
  }
  /* The two headings that ask for attention: overdue in the danger colour, today in the accent — the rest read as plain labels */
  .title.late {
    color: var(--danger);
  }
  .title.today {
    color: var(--accent);
  }
  .count,
  .extra {
    font-size: var(--fs-xs);
    font-weight: 400;
    color: var(--fg-muted);
    font-variant-numeric: tabular-nums;
  }
  ul {
    list-style: none;
  }
  /* A four-column grid: the checkbox, the words, the date, the rest. Each row is its own
     grid container, so the tracks can only line up across rows (and across groups) by
     being the same in every row: nothing here sizes to a row's content. The words take
     what the others leave (1fr) down to a floor of 14em; the date column is 6.2em, which
     holds the longest short word ("12 days late", "2027年9月30日") in 12px; the rest
     takes a fixed share of the row, so its source note ends every row at one edge. Below
     the floors' sum the whole table scrolls sideways (.body) rather than squeezing the
     words — a measured screenshot at ~370px once had them down to four characters. */
  .row {
    display: grid;
    grid-template-columns: auto minmax(14em, 1fr) 6.2em minmax(0, min(22%, 12em));
    align-items: center;
    width: 100%;
    text-align: left;
    padding: 5px 8px;
    border-radius: 6px;
    gap: 8px;
    font-size: var(--ag-fs);
    line-height: 1.5;
    cursor: pointer;
  }
  .row:hover,
  .row.held {
    background: var(--bg-hover);
  }
  .check {
    padding: 2px;
    border-radius: 4px;
    display: inline-flex;
  }
  /* The state glyph (chips.ts) at the editor checkbox's 15px; it brings its own colour,
     and the hover turns the box accent as a preview of the click */
  .check :global(svg) {
    width: 15px;
    height: 15px;
    display: block;
  }
  .check:hover {
    background: var(--bg-active);
  }
  .check:hover :global(.vstate) {
    color: var(--accent);
  }
  .text {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    text-align: start;
  }
  /* A finished task, and one still waiting on another, step back to the secondary ink
     (a done one struck through as well) — readable, not faded; what tells done from
     cancelled is the checkbox's own shape */
  .row.closed .text,
  .row.blocked .text {
    color: var(--fg-muted);
  }
  .row.closed .text {
    text-decoration: line-through;
  }
  .when,
  .meta-cell {
    display: flex;
    align-items: center;
    min-width: 0;
    font-size: var(--fs-xs);
  }
  /* A date: plain secondary text with tabular digits, no ground — a button because a
     click reschedules. Its tone says which date it is (the legend) */
  .date {
    min-width: 0;
    /* the side padding (the hover's ground) reaches into the gaps, so the words have the
       whole column: "2027年9月30日" is 86px of the 87 */
    max-width: calc(100% + 6px);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    padding: 0 3px;
    margin: 0 -3px;
    border-radius: 4px;
    color: var(--fg-muted);
    font-variant-numeric: tabular-nums;
  }
  .date:hover {
    color: var(--accent);
    background: var(--bg-active);
  }
  .date.late {
    color: var(--ag-late);
  }
  .date.scheduled {
    color: var(--accent);
  }
  .date.start {
    color: var(--ag-start);
  }
  /* Today's date in the today group, and the "+" of a task with no date, show while the
     row is hovered: the heading already says the day, and a column of plus signs would be
     noise. Hidden whole (not faded), and still a button to reach by Tab */
  .date.quiet,
  .add-date {
    opacity: 0;
  }
  .row:hover .date.quiet,
  .row:hover .add-date,
  .date:focus-visible {
    opacity: 1;
  }
  /* The rest of the row as one line of secondary text, a dot between its parts, the
     source note last; one ellipsis runs across all of it (the tip has the whole line) */
  .meta {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    color: var(--fg-muted);
  }
  .mi {
    display: inline;
  }
  /* A date on the meta line is a word of it: inline, so the line's one ellipsis runs
     across it too */
  .mi.date {
    display: inline;
    overflow: visible;
    max-width: none;
    margin: 0;
    padding: 0;
  }
  .mi.late {
    color: var(--ag-late);
  }
  /* The dot between parts is a mark, not a word: the faint ink */
  .sep {
    color: var(--fg-faint);
  }
  .glyph {
    display: inline-block;
    width: 14px;
    height: 14px;
    vertical-align: -3px;
    margin-right: 3px;
  }
  .glyph :global(svg) {
    width: 100%;
    height: 100%;
  }
  .lock {
    display: inline-flex;
    flex-shrink: 0;
    margin-left: 4px;
    color: var(--accent);
  }
</style>
