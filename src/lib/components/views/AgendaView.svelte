<script lang="ts">
  import { api, type BlockHit } from "$lib/api";
  import { t } from "$lib/i18n/i18n.svelte";
  import { fuzzyKeep } from "$lib/commands/registry";
  import { vault } from "$lib/stores/vault.svelte";
  import { byWhen, DATE_FIELDS, dateFieldLabel, type DateField, dateIconSvg, dueLabel, localDate, rescheduleTask, rowOf, taskFilterKey, type TaskRow, taskStateIconSvg, toggleTask, whenOf } from "$lib/views/tasks";
  import { k as kbd } from "$lib/shortcut";
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

  /** The rows are built before they are filtered: the grouping reads the state and dates
   *  rowOf gave, the filtering reads taskFilterKey, and both share one Row — so rowOf runs
   *  once per block. The key holds the note's title and path, since "what else is unfinished
   *  in this note" is the most common way of searching. */
  const allRows = $derived(hits.map(rowOf));
  const shown = $derived(fuzzyKeep(allRows, query, taskFilterKey));

  /** The groups: overdue / today / the next 7 days / later / doing / no date / done */
  const groups = $derived.by(() => {
    const today = localDate();
    const weekEnd = localDate(7);
    const g = { overdue: [] as Row[], today: [] as Row[], week: [] as Row[], later: [] as Row[], doing: [] as Row[], nodate: [] as Row[], done: [] as Row[] };
    for (const r of shown) {
      const when = whenOf(r);
      if (r.state === "done") g.done.push(r);
      else if (when && when < today) g.overdue.push(r);
      // A task starting today with ⏳ belongs under "today" even when its due date is later
      else if (when === today || r.scheduled === today) g.today.push(r);
      else if (when && when <= weekEnd) g.week.push(r);
      else if (when) g.later.push(r);
      else if (r.state === "doing") g.doing.push(r);
      else g.nodate.push(r);
    }
    for (const rows of Object.values(g)) rows.sort(byWhen);
    return g;
  });

  const total = $derived(shown.length);

  /** The guard every write in Agenda shares: the note being edited is saved first, so unsaved changes are not overwritten */
  function dirtyGuard(hit: BlockHit, act: string): boolean {
    if (hit.note === vault.currentPath && vault.dirty) {
      vault.error = t("task.dirtyGuard", { key: kbd("Mod+S"), action: act });
      return false;
    }
    return true;
  }

  async function onToggle(e: Event, hit: BlockHit) {
    e.stopPropagation();
    if (!dirtyGuard(hit, t("task.act.toggle"))) return;
    try {
      // A stale span (the note changed after the query): nothing is written, and after a requery the user retries on the new data
      if (!(await toggleTask(hit))) await load();
    } catch (err) {
      vault.error = String(err);
    }
  }

  /** The reschedule menu a date chip opens; an empty current means the task does not have that date yet */
  let dateMenu = $state<{ x: number; y: number; hit: BlockHit; field: DateField; current?: string } | null>(null);

  function openDateMenu(e: MouseEvent, r: Row, field: DateField) {
    e.stopPropagation();
    dateMenu = { x: e.clientX, y: e.clientY, hit: r.hit, field, current: field === "due" ? r.due : r.scheduled };
  }

  async function applyDate(date: string | null) {
    const m = dateMenu;
    dateMenu = null;
    if (!m || !dirtyGuard(m.hit, t("task.act.editDate"))) return;
    try {
      if (!(await rescheduleTask(m.hit, m.field, date))) await load();
    } catch (err) {
      vault.error = String(err);
    }
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

{#snippet taskRow(r: Row, overdue: boolean)}
  <li>
    <div
      class="row"
      role="button"
      tabindex="0"
      onclick={() => void openTask(r.hit)}
      onkeydown={(e) =>
        e.target === e.currentTarget && (e.key === "Enter" || e.key === " ") && void openTask(r.hit)}
    >
      <button class="check" class:done={r.state === "done"} aria-label={t("task.toggleDone")} onclick={(e) => onToggle(e, r.hit)}>
        {@html taskStateIconSvg(r.state)}
      </button>
      <span class="text" class:done={r.state === "done"}>{r.text}</span>
      <!-- The date and the source each take a fixed-width slot: a row with no date holds its
        place too, which is what lets the source chips line up into a real column (before
        this, the chips of the with-date and without-date groups differed by nearly 70px on
        the left and looked like a saw edge) -->
      <span class="dates">
        {#if r.due}
          {@render dateChip(r, "due", t("task.editDateWith", { field: dateFieldLabel("due"), date: r.due }), dueLabel(r.due, overdue), overdue ? "due overdue" : "due")}
        {/if}
        {#if r.scheduled}
          {@render dateChip(r, "scheduled", t("task.editDateWith", { field: dateFieldLabel("scheduled"), date: r.scheduled }), r.scheduled)}
        {/if}
        {#if !r.due && r.state !== "done"}
          {@render dateChip(r, "due", t("task.setDate", { field: dateFieldLabel("due") }), "+", "add-date")}
        {/if}
      </span>
      <span class="meta">
        <span class="chip note-chip">{r.hit.note_title}</span>
        {#if vault.privatePaths.has(r.hit.note)}<span class="lock" title={t("statusBar.privacyZoneHint")}><PrivacyIcon size={14} /></span>{/if}
      </span>
    </div>
  </li>
{/snippet}

{#snippet section(title: string, rows: Row[], overdue: boolean, collapsed: boolean)}
  {#if rows.length}
    <details open={!collapsed}>
      <summary>
        <!-- summary is display:flex, and the UA's disclosure triangle only renders on
          display:list-item, so it disappeared with it — the collapsed and expanded states used
          to look identical ("Later 3" had a count but showed no rows and looked broken). This
          chevron is the only collapse indicator -->
        <span class="chevron"><Chevron /></span>
        <span class:overdue-title={overdue}>{title}</span><span class="count">{rows.length}</span>
      </summary>
      <ul>
        <!-- The key is the block's byte span start: physically unique within a note (an
          explicit ^id may be copied into duplicates and cannot be a key); a stable key keeps
          the DOM of unchanged rows from being rebuilt after a requery -->
        {#each rows as r (r.hit.note + ":" + r.hit.block.span.start)}
          {@render taskRow(r, overdue)}
        {/each}
      </ul>
    </details>
  {/if}
{/snippet}

<div class="view">
  <ViewHeader title={t("view.agenda")} total={countLabel(total, allRows.length, "task.count")} filterPlaceholder={t("task.filterPlaceholder")} bind:filter={query}>
    {#snippet left()}
      <!-- The legend: the calendar and hourglass chip icons used to be recognisable only on hover -->
      <span class="legend" aria-hidden="true">
        <span>{@html dateIconSvg("due")}{dateFieldLabel("due")}</span>
        <span>{@html dateIconSvg("scheduled")}{dateFieldLabel("scheduled")}</span>
      </span>
    {/snippet}
  </ViewHeader>
  <div class="body">
    {#if hits.length === 0}
      <ViewEmpty>
        {t("task.empty1")}<code>- [ ] a task 📅 2026-01-01</code>{t("task.empty2")}<code>- [/]</code
        >{t("task.empty3")}<code>⏳ 2026-01-01</code>{t("task.empty4")}
      </ViewEmpty>
    {:else if total === 0}
      <ViewEmpty onclear={() => (query = "")}>{t("task.noMatch", { query })}</ViewEmpty>
    {:else}
      {@render section(t("agenda.overdue"), groups.overdue, true, false)}
      {@render section(t("agenda.today"), groups.today, false, false)}
      {@render section(t("agenda.week"), groups.week, false, false)}
      {@render section(t("agenda.later"), groups.later, false, true)}
      {@render section(t("agenda.doing"), groups.doing, false, false)}
      {@render section(t("agenda.nodate"), groups.nodate, false, false)}
      {@render section(t("agenda.done"), groups.done, false, true)}
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
    gap: 10px;
    font-size: var(--fs-xs);
    color: var(--fg-faint);
  }
  .legend span {
    display: inline-flex;
    align-items: center;
    gap: 3px;
  }
  /* dateIconSvg is injected with {@html}, and the svg carries no scope marker, so its size is matched with :global */
  .legend :global(svg) {
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
  .overdue-title {
    color: var(--danger);
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
     squeezing the source column; 13em is the size of the longest chip, "a date plus N days
     late" (a bare date is about 7.2em).
     The source track's 10em floor is the overflow guard: as the pane narrows the fixed
     tracks fill up first, and without a floor the source chip would be pushed out of the
     container and the whole view would grow a horizontal scrollbar. */
  .row {
    --ag-text: 26em;
    --ag-date: 13em;
    --ag-note: 10em;
    display: grid;
    grid-template-columns: auto minmax(0, var(--ag-text)) minmax(var(--ag-date), auto) minmax(var(--ag-note), 1fr);
    align-items: center;
    width: 100%;
    text-align: left;
    padding: 5px 8px;
    border-radius: 6px;
    gap: 8px;
    font-size: var(--fs-sm);
    cursor: pointer;
  }
  .row:hover {
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
</style>
