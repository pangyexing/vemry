<script lang="ts">
  import { api, type BlockQuery, type BlockHit } from "$lib/api";
  import { viewContext } from "$lib/stores/viewContext.svelte";
  import { askAboutView } from "$lib/askview";
  import { t } from "$lib/i18n/i18n.svelte";
  import { fuzzyKeep } from "$lib/commands/registry";
  import { vault } from "$lib/stores/vault.svelte";
  import { assignTaskId, attrIconSvg, byDate, byModified, byPriority, dateFieldLabel, type DateField, dateIconSvg, deleteTask, dueLabel, estimateSum, formatEstimate, isBlocked, isClosed, localDate, newTaskId, openIds, priorityLabel, rescheduleTask, rowOf, taskFilterKey, taskKey, taskMenuEntries, type TaskRow, taskStateIconSvg, toggleTask, updateTask, whenOf } from "$lib/views/tasks";
  import { copyText } from "$lib/copytext";
  import { toast } from "$lib/stores/toast.svelte";
  import { fieldChipsOf } from "$lib/views/blockrow";
  import { stateGlyphSvg, stateTone } from "$lib/chips";
  import { types } from "$lib/stores/types.svelte";
  import { commitTaskWrite } from "$lib/views/taskwrite";
  import { contextMenu } from "$lib/stores/contextMenu.svelte";
  import { cliptip } from "$lib/cliptip";
  import Chevron from "../Chevron.svelte";
  import TaskDateMenu from "./TaskDateMenu.svelte";
  import ViewHeader, { countLabel } from "./ViewHeader.svelte";
  import ViewEmpty from "./ViewEmpty.svelte";
  import PrivacyIcon from "../PrivacyIcon.svelte";

  let hits = $state<BlockHit[]>([]);
  // With events firing in quick succession several loads run at once and an older response may arrive later: only the newest one lands
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

  // Building a row, the grouping date and the ordering all live in views/tasks.ts: the
  // kanban reads the same rules, so the two views cannot disagree about a task (Row is the
  // local alias of TaskRow over there)
  type Row = TaskRow;

  let query = $state("");
  /** The second filter, set from the legend: only the tasks carrying this date are shown,
   *  and they are shown by it — the groups and the order inside them read that date instead
   *  of the due-first default, so "start date" is a real view of when things start rather
   *  than the due-date view minus some rows. One kind at a time: a task with both dates is
   *  found by either, so a "both" state would only ever be a stricter version of one of
   *  them. */
  let dateKind = $state<DateField>();

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
  const allRows = $derived(hits.map(rowOf));
  const shown = $derived.by(() => {
    const kept = fuzzyKeep(allRows, query, taskFilterKey);
    const kind = dateKind;
    return kind ? kept.filter((r) => r[kind]) : kept;
  });
  /** The one way back from an empty list: either filter may be the one in the way, and a
   *  button that cleared only the other would do nothing when pressed */
  const clearFilters = () => {
    query = "";
    dateKind = undefined;
  };

  /** The day the rows are read against, taken once per requery: the grouping and the chips
   *  of one render agree on it. */
  const today = $derived.by(() => {
    void hits;
    return localDate();
  });
  /** The groups: overdue / today / the next 7 days / later / not started yet / doing / no date / done (cancelled included).
   *  In the default view the three dates cut a task's life into four spans: before the
   *  start day it is not to be done yet; from the scheduled day to the due day it is
   *  today's work; past the due day it is overdue; and a task with neither a scheduled
   *  nor a due day sits by its state. The start day alone places nothing under a date —
   *  a started task with no other date is simply available. With a date kind pressed
   *  every task sits by that one date instead. */
  const groups = $derived.by(() => {
    const weekEnd = localDate(7);
    const kind = dateKind;
    const g = { overdue: [] as Row[], today: [] as Row[], week: [] as Row[], later: [] as Row[], future: [] as Row[], doing: [] as Row[], nodate: [] as Row[], done: [] as Row[] };
    for (const r of shown) {
      const when = kind ? r[kind] : (r.due ?? r.scheduled);
      if (isClosed(r.state)) g.done.push(r);
      else if (!kind && r.start && r.start > today) g.future.push(r);
      // only a due day passes: a task scheduled last week with no due day is still today's work
      else if (kind ? when && when < today : r.due && r.due < today) g.overdue.push(r);
      else if (kind ? when === today : r.due === today || (r.scheduled && r.scheduled <= today)) g.today.push(r);
      else if (when && when <= weekEnd) g.week.push(r);
      else if (when) g.later.push(r);
      else if (r.state === "doing") g.doing.push(r);
      else g.nodate.push(r);
    }
    // Each group is sorted once: what is marked urgent comes first, then the two with no
    // date to read follow the task's own change time, most recently changed first, and the
    // rest go by date. With a date kind pressed every group goes by that date — it is the
    // order that was asked for, the done group included
    const byWhen = byPriority(byDate(kind));
    for (const [name, rows] of Object.entries(g)) rows.sort(!kind && (name === "nodate" || name === "done") ? byPriority(byModified) : byWhen);
    return g;
  });

  /** The ids the open tasks carry: what a ⛔ can still wait on. */
  const open = $derived(openIds(allRows));
  /** The estimates of the day added up, shown beside the group's count when any row has one. */
  const todayEstimate = $derived.by(() => {
    const sum = estimateSum(groups.today);
    return sum > 0 ? t("agenda.estimate", { v: formatEstimate(sum) }) : "";
  });

  const total = $derived(shown.length);

  /** Whether a task's due chip reads as late is the task's own: its due date has passed
   *  and it is not done. In the default view that is the same as sitting in the overdue
   *  group (a task's date is its due date whenever it has one); with the start date
   *  pressed the group follows the start date, and a task that started last month but is
   *  due next week must not be painted late. */
  const isLate = (r: Row) => !isClosed(r.state) && !!r.due && r.due < today;
  /** The date track's width follows what the shown rows carry. Every row shares it, so the
   *  source chips still line up into one column across the groups (the reason the track is
   *  fixed at all); it is only as wide as the widest chip the current rows can show — the
   *  full "date · N days late" (13em), a bare date (the chip is about 7.2em), or nothing but
   *  the hover "+" — rather than reserving 13em for a lateness that no row in a filtered
   *  list has. The task text keeps its own cap; the space goes to the source column. */
  const dateTrack = $derived(shown.some(isLate) ? "13em" : shown.some((r) => r.due || r.scheduled || r.start) ? "7.5em" : "2.6em");

  /** The write funnel every path shares (taskwrite.ts, the kanban's too): the dirty guard, a requery on a stale span and error reporting. */
  const commit = (hit: BlockHit, act: string, write: () => Promise<boolean>) => commitTaskWrite(hit, act, write, load);

  function onToggle(e: Event, hit: BlockHit) {
    e.stopPropagation();
    void commit(hit, t("task.act.toggle"), () => toggleTask(hit));
  }

  /** The reschedule menu a date chip opens; an empty current means the task does not have that date yet */
  let dateMenu = $state<{ x: number; y: number; key: string; hit: BlockHit; field: DateField; current?: string } | null>(null);

  function openDateMenu(e: MouseEvent, r: Row, field: DateField) {
    e.stopPropagation();
    dateMenu = { x: e.clientX, y: e.clientY, key: taskKey(r), hit: r.hit, field, current: r[field] };
  }

  /** The row a menu is about keeps its hover colour while the menu is open. A chip's date
   *  menu hangs off the chip and the row is plain to see; the context menu, and the date
   *  menu it opens, float at the right-click with the pointer gone from the row — and a
   *  "start date: today / tomorrow" with no row lit says nothing about which task it is
   *  setting. The context menu remembers what it was opened for (owner) until it closes. */
  const heldKey = $derived(dateMenu?.key ?? contextMenu.owner);

  function applyDate(date: string | null) {
    const m = dateMenu;
    dateMenu = null;
    if (m) void commit(m.hit, t("task.act.editDate"), () => rescheduleTask(m.hit, m.field, date));
  }

  /** A row's context menu: the entries are the kanban card's (taskMenuEntries), and the
   *  date entries open the same TaskDateMenu a chip does, at the right-click. */
  function rowMenu(e: MouseEvent, r: Row) {
    e.stopPropagation();
    const at = { x: e.clientX, y: e.clientY };
    const key = taskKey(r);
    contextMenu.open(
      e,
      taskMenuEntries(r, {
        open: () => void openTask(r.hit),
        editDate: (field, current) => (dateMenu = { ...at, key, hit: r.hit, field, current }),
        setState: (state, act) => void commit(r.hit, act, () => updateTask(r.hit, { state })),
        copyId: () => void copyTaskId(r),
        remove: () => void commit(r.hit, t("task.act.remove"), () => deleteTask(r.hit)),
      }),
      key,
    );
  }

  /** The task's id to the clipboard — minted and written onto its line first when it has
   *  none (what a `⛔` on another task names; an id exists only once something needs it). */
  async function copyTaskId(r: Row) {
    const id = r.taskId ?? newTaskId();
    const ok = r.taskId ? true : await commit(r.hit, t("task.act.id"), () => assignTaskId(r.hit, id));
    if (ok && (await copyText(id))) toast.push(t("task.copiedId", { id }));
  }

  /** Clicking a task jumps to its line. The kanban uses the same vault.openBlockAt — the
   *  byte → UTF-16 conversion and rechecking currentPath after the await are contracts kept
   *  in one place. */
  const openTask = (hit: BlockHit) => vault.openBlockAt(hit.note, hit.block.span.start);

</script>

{#snippet dateChip(r: Row, field: DateField, label: string, text: string, cls: string = "")}
  <!-- The explanation uses aria-label rather than title: a native tooltip pops below the
    cursor, exactly covering the next task row (the most obstructive place while hovering),
    and the meaning of the calendar and hourglass icons is already explained by the legend
    in the header, so a chip no longer has to be hovered to be recognised -->
  <button class="chip {cls}" aria-label={label} onclick={(e) => openDateMenu(e, r, field)}>
    {@html dateIconSvg(field)}{text}
  </button>
{/snippet}

{#snippet taskRow(r: Row)}
  {@const dueOverdue = isLate(r)}
  {@const closed = isClosed(r.state)}
  {@const blocked = isBlocked(r, open)}
  <li>
    <div
      class="row"
      class:held={taskKey(r) === heldKey}
      class:blocked
      role="button"
      tabindex="0"
      onclick={() => void openTask(r.hit)}
      oncontextmenu={(e) => rowMenu(e, r)}
      onkeydown={(e) =>
        e.target === e.currentTarget && (e.key === "Enter" || e.key === " ") && void openTask(r.hit)}
    >
      <button class="check" class:done={closed} aria-label={t("task.toggleDone")} onclick={(e) => onToggle(e, r.hit)}>
        {@html taskStateIconSvg(r.state)}
      </button>
      <!-- The text is clipped to its track: a long task is read whole from cliptip's box, a quarter second into the hover (the kanban card does the same) -->
      <span class="text" class:done={closed} use:cliptip={r.text}>{r.text}</span>
      <!-- The date and the source each take a fixed-width slot: a row with no date holds its
        place too, which is what lets the source chips line up into a real column (before
        this, the chips of the with-date and without-date groups differed by nearly 70px on
        the left and looked like a saw edge) -->
      <span class="dates">
        {#if r.due}
          {@render dateChip(r, "due", t("task.editDateWith", { field: dateFieldLabel("due"), date: r.due }), dueLabel(r.due, dueOverdue), dueOverdue ? "due overdue" : "due")}
        {/if}
        {#if r.scheduled}
          {@render dateChip(r, "scheduled", t("task.editDateWith", { field: dateFieldLabel("scheduled"), date: r.scheduled }), r.scheduled, "scheduled")}
        {/if}
        {#if r.start}
          {@render dateChip(r, "start", t("task.editDateWith", { field: dateFieldLabel("start"), date: r.start }), r.start, "start")}
        {/if}
        {#if !r.due && !closed}
          {@render dateChip(r, "due", t("task.setDate", { field: dateFieldLabel("due") }), "+", "add-date")}
        {/if}
      </span>
      <span class="meta">
        <!-- The task's other attributes, each its icon and value in its tone: what presses
          (an urgent priority, a lock still shut) in danger, the rest muted -->
        {#if r.priority !== "normal"}
          <span class="chip attr-chip" class:urgent={r.priority === "highest" || r.priority === "high"} title={priorityLabel(r.priority)}>{@html attrIconSvg("priority", r.priority)}{priorityLabel(r.priority)}</span>
        {/if}
        {#if blocked}
          <span class="chip attr-chip urgent" title={t("task.chip.blockedBy", { ids: r.dependsOn.join(", ") })}>{@html attrIconSvg("depends")}{t("task.chip.blocked")}</span>
        {/if}
        {#if r.repeat}
          <span class="chip attr-chip" title={t("task.chip.repeat", { rule: r.repeat })}>{@html attrIconSvg("repeat")}{r.repeat}</span>
        {/if}
        {#if r.estimate}
          <span class="chip attr-chip" title={t("task.chip.estimate", { v: formatEstimate(r.estimate) })}>{@html attrIconSvg("estimate")}{formatEstimate(r.estimate)}</span>
        {/if}
        {#if r.deleteOnDone}
          <span class="chip attr-chip" title={t("task.chip.deleteOnDone")}>{@html attrIconSvg("delete")}</span>
        {/if}
        <!-- The task's inline fields as chips (views/blockrow.ts's one derivation): a state
          field with its step's glyph, the rest as key: value — the board shows the same -->
        {#each fieldChipsOf(r.hit, types.enabled) as f (f.kind === "tag" ? f.tag : f.key)}
          {#if f.kind === "state"}
            <span class="chip state-chip {`tone-${stateTone(f.step, f.of)}`}" title={`${f.key}:: ${f.value}`}><i class="glyph">{@html stateGlyphSvg(f.step, f.of)}</i>{f.value}</span>
          {:else if f.kind === "field"}
            <span class="chip field-chip" title={`${f.key}:: ${f.value}`}><span class="fk">{f.key}</span>{f.value}</span>
          {/if}
        {/each}
        <span class="chip note-chip">{r.hit.note_title}</span>
        {#if vault.privatePaths.has(r.hit.note)}<span class="lock" title={t("statusBar.privacyZoneHint")}><PrivacyIcon size={14} /></span>{/if}
      </span>
    </div>
  </li>
{/snippet}

{#snippet dateKindToggle(field: DateField)}
  <button
    class="kind"
    aria-pressed={dateKind === field}
    title={t(dateKind === field ? "task.onlyWithOff" : "task.onlyWith", { field: dateFieldLabel(field) })}
    onclick={() => (dateKind = dateKind === field ? undefined : field)}
  >
    {@html dateIconSvg(field)}{dateFieldLabel(field)}
  </button>
{/snippet}

{#snippet section(title: string, rows: Row[], collapsed: boolean, tone: "" | "late" | "today" = "", extra: string = "")}
  {#if rows.length}
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
        {#each rows as r (taskKey(r))}
          {@render taskRow(r)}
        {/each}
      </ul>
    </details>
  {/if}
{/snippet}

<div class="view">
  <ViewHeader title={t("view.agenda")} total={countLabel(total, allRows.length, "task.count")} filterPlaceholder={t("task.filterPlaceholder")} bind:filter={query} askAi={askAboutView}>
    {#snippet left()}
      <!-- The legend doubles as a filter. As a legend it names the calendar and hourglass
        chip icons (they used to be recognisable only on hover); pressed, an entry keeps only
        the tasks carrying that date, placed and ordered by it, and the count beside it shows
        how many that left. It sits on the left, by the count it narrows, rather than among
        the tools on the right -->
      <span class="legend">
        {@render dateKindToggle("due")}
        {@render dateKindToggle("scheduled")}
        {@render dateKindToggle("start")}
      </span>
    {/snippet}
  </ViewHeader>
  <div class="body" style:--ag-date={dateTrack}>
    {#if hits.length === 0}
      <ViewEmpty>
        {t("task.empty1")}<code>- [ ] a task 📅 2026-01-01</code>{t("task.empty2")}<code>- [/]</code
        >{t("task.empty3")}<code>⏳ 2026-01-01</code>{t("task.empty4")}
      </ViewEmpty>
    {:else if total === 0 && !query.trim() && dateKind}
      <ViewEmpty onclear={clearFilters}>{t("task.noneWithDate", { field: dateFieldLabel(dateKind) })}</ViewEmpty>
    {:else if total === 0}
      <ViewEmpty onclear={clearFilters}>{t("task.noMatch", { query })}</ViewEmpty>
    {:else}
      {@render section(t("agenda.overdue"), groups.overdue, false, "late")}
      {@render section(t("agenda.today"), groups.today, false, "today", todayEstimate)}
      {@render section(t("agenda.week"), groups.week, false)}
      {@render section(t("agenda.later"), groups.later, true)}
      {@render section(t("agenda.future"), groups.future, true)}
      {@render section(t("agenda.doing"), groups.doing, false)}
      {@render section(t("agenda.nodate"), groups.nodate, false)}
      {@render section(t("agenda.done"), groups.done, true)}
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
  }
  .legend {
    display: flex;
    gap: 6px;
  }
  /* Each entry wears the shape of the date chips it stands for, so the legend reads as
     "these two chips". Faint at rest; pressed, it takes the accent — a hover only lifts the
     background, so the pressed one stays distinguishable from a hovered one */
  .kind {
    display: inline-flex;
    align-items: center;
    gap: 3px;
    font-size: var(--fs-xs);
    color: var(--fg-faint);
    background: var(--bg-hover);
    border-radius: 4px;
    padding: 1px 6px;
  }
  .kind:hover {
    color: var(--fg-muted);
    background: var(--bg-active);
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
  .body {
    flex: 1;
    overflow-y: auto;
    padding: 10px 20px 30px;
  }
  details {
    margin-bottom: 6px;
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
  .count {
    font-size: var(--fs-xs);
    font-weight: 400;
    color: var(--fg-faint);
  }
  ul {
    list-style: none;
  }
  /* A four-column grid. The column widths can only be fixed: each row is its own grid
     container, an auto track that fits its content only aligns within one container, and
     per-row autos each grow their own way — across groups they line up even less. The price
     is the blank space to the right of a short task, and what it buys is the date and source
     columns sharing one x across the whole table (groups included).
     --ag-text is also the task name's cap: .text used to take flex:1 and fill the row, which
     pushed a short task's date out past 500px and made the eye sweep a whole row to pair
     them up.
     The date track uses minmax: the few rows carrying both 📅 and ⏳ let it grow rather than
     squeezing the source column. Its floor, --ag-date, is set on .body from the rows shown
     (dateTrack): 13em when a "date plus N days late" chip is on the table, less otherwise.
     The source track's 10em floor is the overflow guard: as the pane narrows the fixed
     tracks fill up first, and without a floor the source chip would be pushed out of the
     container and the whole view would grow a horizontal scrollbar. */
  .row {
    --ag-text: 26em;
    --ag-note: 10em;
    display: grid;
    grid-template-columns: auto minmax(0, var(--ag-text)) minmax(var(--ag-date, 13em), auto) minmax(var(--ag-note), 1fr);
    align-items: center;
    width: 100%;
    text-align: left;
    padding: 5px 8px;
    border-radius: 6px;
    gap: 8px;
    font-size: var(--fs-sm);
    cursor: pointer;
  }
  .row:hover,
  .row.held {
    background: var(--bg-hover);
  }
  .check {
    font-size: var(--fs-md);
    color: var(--fg-muted);
    padding: 2px;
    border-radius: 4px;
    display: inline-flex;
  }
  /* taskStateIconSvg is injected with {@html}, and its size follows .check's font-size */
  .check :global(svg) {
    width: 1em;
    height: 1em;
    display: block;
  }
  .check:hover {
    color: var(--accent);
    background: var(--bg-hover);
  }
  .check.done {
    color: var(--accent);
  }
  .text {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .text.done {
    color: var(--fg-faint);
    text-decoration: line-through;
  }
  .dates,
  .meta {
    display: flex;
    align-items: center;
    gap: 6px;
    min-width: 0;
  }
  .chip {
    flex-shrink: 0;
    font-size: var(--fs-xs);
    color: var(--fg-muted);
    background: var(--bg-hover);
    border-radius: 4px;
    padding: 1px 6px;
  }
  .chip.due.overdue {
    color: var(--danger);
  }
  /* The date chips' tones: a start date in the go colour, the scheduled day in the accent */
  .chip.start {
    color: var(--ok);
  }
  .chip.scheduled {
    color: var(--accent);
  }
  /* The other attributes: the icon then the value; urgent in danger */
  .attr-chip {
    display: inline-flex;
    align-items: center;
    gap: 4px;
  }
  .attr-chip :global(svg) {
    width: 0.9em;
    height: 0.9em;
    flex-shrink: 0;
  }
  .attr-chip.urgent {
    color: var(--danger);
  }
  /* A task waiting on another: read as not yet actionable */
  .row.blocked .text {
    color: var(--fg-faint);
  }
  .extra {
    font-size: var(--fs-xs);
    font-weight: 400;
    color: var(--fg-faint);
  }
  /* A date chip is a button: the icon and the date in a row; a note-chip is still a span and is unaffected by flex */
  button.chip {
    display: inline-flex;
    align-items: center;
    gap: 4px;
  }
  button.chip:hover {
    color: var(--accent);
    background: var(--bg-active);
  }
  button.chip :global(svg) {
    width: 0.9em;
    height: 0.9em;
    flex-shrink: 0;
  }
  /* A task with no date shows "📅+" while its row is hovered; always showing it would fill the list with plus signs */
  .add-date {
    opacity: 0;
  }
  .row:hover .add-date,
  .add-date:focus-visible {
    opacity: 1;
  }
  /* The one chip allowed to shrink: in a narrow pane it has to give way and be truncated,
     or the grid track cannot hold it (.chip is flex-shrink: 0 everywhere else) */
  .note-chip {
    flex-shrink: 1;
    max-width: 160px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .lock {
    display: inline-flex;
    flex-shrink: 0;
    color: var(--accent);
  }
  /* A field chip: the key faint before the value; a state field's with its glyph in the
     step's tone (chips.ts) — the same shapes as the board's */
  .field-chip .fk {
    color: var(--fg-faint);
    margin-right: 3px;
  }
  .field-chip .fk::after {
    content: ":";
  }
  .state-chip {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    padding-left: 4px;
  }
  .state-chip .glyph {
    display: inline-flex;
    width: 13px;
    height: 13px;
    flex-shrink: 0;
  }
  .state-chip .glyph :global(svg) {
    width: 100%;
    height: 100%;
  }
  .tone-idle {
    color: var(--state-idle);
  }
  .tone-active {
    color: var(--fg);
  }
  .tone-active .glyph {
    color: var(--state-active);
  }
  .tone-done {
    color: var(--state-done);
  }
</style>
