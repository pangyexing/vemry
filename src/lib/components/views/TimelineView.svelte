<script lang="ts">
  import { untrack } from "svelte";
  import type { NoteSummary } from "$lib/api";
  import { fuzzyKeep } from "$lib/commands/registry";
  import { cliptip } from "$lib/cliptip";
  import { selectingIn } from "$lib/views/select";
  import { i18n, t, tn } from "$lib/i18n/i18n.svelte";
  import { parentOf, stemOf, vault } from "$lib/stores/vault.svelte";
  import { ymd } from "$lib/views/tasks";
  import Chevron from "../Chevron.svelte";
  import ViewHeader, { countLabel } from "./ViewHeader.svelte";
  import ViewEmpty from "./ViewEmpty.svelte";
  import PrivacyIcon from "../PrivacyIcon.svelte";

  let query = $state("");
  let body = $state<HTMLDivElement | null>(null);

  // ---- background changes wait while the reader is down the list ----
  // Every vault event re-sorts the whole column: a note edited, written by the AI or synced
  // in jumps to "today" and every row under it moves down. At the top of the list that is
  // the news and it shows at once; further down, the rows the reader is on stay where they
  // are, and "N new changes" at the top offers the update.
  const stamp = (n: NoteSummary) => `${n.mtime_ms}|${n.created ?? ""}|${n.title}|${n.tags.join(" ")}`;
  /** How many notes differ between two lists: changed or new, plus the ones gone. */
  function changes(before: NoteSummary[], after: NoteSummary[]): number {
    if (before === after) return 0;
    const old = new Map(before.map((n) => [n.id, stamp(n)]));
    let n = 0;
    for (const x of after) {
      if (old.get(x.id) !== stamp(x)) n++;
      old.delete(x.id);
    }
    return n + old.size;
  }
  /** The notes the column is drawn from: the live list while the reader is at the top. */
  let shown = $state.raw<NoteSummary[]>(vault.notes);
  let atTop = $state(true);
  $effect(() => {
    const next = vault.notes;
    untrack(() => {
      if (atTop || changes(shown, next) === 0) shown = next;
    });
  });
  const pending = $derived(changes(shown, vault.notes));
  function onscroll() {
    if (!body) return;
    atTop = body.scrollTop <= 0;
    if (atTop && pending > 0) shown = vault.notes;
  }
  /** Taking the update is the reader's own act, and what it is for is seeing the changes:
   *  the list takes them and goes to its top, where they are. */
  function showChanges() {
    shown = vault.notes;
    if (body) body.scrollTop = 0;
  }

  /** A row opens its note on a click, unless the click ended a drag that selected its text. */
  function open(e: MouseEvent, id: string) {
    if (!selectingIn(e.currentTarget as Element)) void vault.openNote(id);
  }

  /** The frontmatter `created` is free text; it is used when it parses, and mtime otherwise */
  function parseCreated(s: string): Date | null {
    const m = s.trim().match(/^(\d{4})-(\d{2})-(\d{2})$/);
    // A bare date is parsed in the local time zone (Date("YYYY-MM-DD") reads it as UTC, which is a day out across time zones)
    const d = m ? new Date(+m[1], +m[2] - 1, +m[3]) : new Date(s);
    return isNaN(d.getTime()) ? null : d;
  }

  type Row = { note: NoteSummary; time: Date; source: "created" | "modified" };

  /** Only the directory part of a path is new information: when the file name matches the
   *  title (as most notes do) only the directory is kept, and a note of that name at the root
   *  leaves the whole line empty; when the title and the file name differ, the full path is
   *  given. When it returns an empty string the span still has to render — it carries flex:1
   *  and is the strut pushing the chips on the right to the end of the row */
  function pathHint(note: NoteSummary): string {
    if (stemOf(note.id) !== note.title) return note.id;
    const dir = parentOf(note.id);
    return dir ? `${dir}/` : "";
  }

  const rows = $derived.by(() => {
    const all: Row[] = shown.map((note) => {
      const created = note.created ? parseCreated(note.created) : null;
      return created
        ? { note, time: created, source: "created" as const }
        : { note, time: new Date(note.mtime_ms), source: "modified" as const };
    });
    const filtered = fuzzyKeep(all, query, (r) => `${r.note.title} ${r.note.id} ${r.note.tags.join(" ")}`);
    return filtered.sort((a, b) => b.time.getTime() - a.time.getTime());
  });

  // A day is written the way the interface language writes a date (month, day and
  // weekday in its own order and words: "Tue, Sep 22", "9月22日 周二"), with the year only
  // across years. Built once per language, not once per row.
  const dayFmt = $derived(new Intl.DateTimeFormat(i18n.locale, { month: "short", day: "numeric", weekday: "short" }));
  const yearDayFmt = $derived(new Intl.DateTimeFormat(i18n.locale, { year: "numeric", month: "short", day: "numeric", weekday: "short" }));

  function dayLabel(key: string, d: Date): string {
    const now = new Date();
    const today = ymd(now);
    const yesterday = ymd(new Date(Date.now() - 86400_000));
    if (key === today) return t("timeline.today");
    if (key === yesterday) return t("timeline.yesterday");
    return (d.getFullYear() === now.getFullYear() ? dayFmt : yearDayFmt).format(d);
  }

  // The clock in the interface language: 24-hour where the locale uses one, 12-hour with
  // its own am/pm marker where it does not. Built once per language like `weekday` above,
  // not once per row — a busy day renders a few hundred.
  const clock = $derived(new Intl.DateTimeFormat(i18n.locale, { hour: "2-digit", minute: "2-digit" }));
  const hhmm = (d: Date) => clock.format(d);
  /** The minute bucket used for the same-minute check. Every real time-zone offset is a whole
   *  number of minutes, so dividing is enough — comparing hhmm strings would build two
   *  strings per row, and this compares once per row. */
  const minuteOf = (d: Date) => Math.floor(d.getTime() / 60_000);

  /** Consecutive rows of the same minute and the same source are merged into a cluster. One
   *  bulk import takes 6 rows at 08-18 20:13 and pushes the rest of that day off screen — but
   *  each of them is still a real event and must not be dropped, so this folds rather than
   *  truncates. The threshold is 3: folding two saves no row and only adds a click. */
  const FOLD_MIN = 3;

  type Cluster = { key: string; time: Date; source: "created" | "modified"; rows: Row[] };

  const days = $derived.by(() => {
    const out: { key: string; label: string; clusters: Cluster[] }[] = [];
    for (const r of rows) {
      const key = ymd(r.time);
      let day = out[out.length - 1];
      if (!day || day.key !== key) {
        day = { key, label: dayLabel(key, r.time), clusters: [] };
        out.push(day);
      }
      const last = day.clusters[day.clusters.length - 1];
      // The rows are already in reverse time order, so those of the same minute are
      // necessarily adjacent; the source has to match as well, or "created 3" would swallow a
      // modification in the middle
      if (last && last.source === r.source && minuteOf(last.time) === minuteOf(r.time)) last.rows.push(r);
      // A cluster's key is its first member's note id, which is unique vault-wide. When two
      // sources alternate within one minute two clusters would share a key, and "date + time +
      // source" as a key would collide
      else day.clusters.push({ key: r.note.id, time: r.time, source: r.source, rows: [r] });
    }
    return out;
  });
</script>

{#snippet noteRow(r: Row)}
  <li>
    <div
      class="row"
      class:created={r.source === "created"}
      role="button"
      tabindex="0"
      onclick={(e) => open(e, r.note.id)}
      onkeydown={(e) => (e.key === "Enter" || e.key === " ") && vault.openNote(r.note.id)}
    >
      <!-- The end of a row no longer prints "modified" on every line (T1); which kind this
        row's time is hangs on the time's own hover tip, so nothing is lost and it does not
        take the end of the line -->
      <span class="time" title={t(r.source === "created" ? "timeline.createdTime" : "timeline.modifiedTime")}>{hhmm(r.time)}</span>
      <span class="title" use:cliptip={r.note.title}>{r.note.title}</span>
      <span class="path">{pathHint(r.note)}</span>
      <!-- frontmatter tags are not deduplicated, and a repeated tag would collide as a key -->
      {#each [...new Set(r.note.tags)].slice(0, 3) as tag (tag)}
        <span class="chip">#{tag}</span>
      {/each}
      <!-- "Modified" is the default source (the vast majority of a vault), and printing it
        would only repeat the same word at the end of every row; only the exception,
        "created", is marked -->
      {#if r.source === "created"}<span class="source">{t("timeline.created")}</span>{/if}
      {#if vault.privatePaths.has(r.note.id)}<span class="lock" title={t("statusBar.privacyZoneHint")}><PrivacyIcon size={14} /></span>{/if}
    </div>
  </li>
{/snippet}

<div class="view">
  <ViewHeader title={t("view.timeline")} total={countLabel(rows.length, shown.length, "timeline.noteCount")} filterPlaceholder={t("timeline.filterPlaceholder")} bind:filter={query} />
  <div class="body" bind:this={body} {onscroll}>
    <!-- No height of its own: the offer floats over the list rather than pushing it down -->
    {#if pending > 0 && !atTop}
      <div class="news-wrap">
        <button class="news" onclick={showChanges}>{tn("timeline.newChanges", pending)}</button>
      </div>
    {/if}
    {#each days as day (day.key)}
      <section>
        <h3>{day.label}</h3>
        <ul>
          {#each day.clusters as c (c.key)}
            {#if c.rows.length < FOLD_MIN}
              {#each c.rows as r (r.note.id)}
                {@render noteRow(r)}
              {/each}
            {:else}
              <li>
                <!-- The collapsed state is <details>'s own to remember: the cluster's key is
                  stable, so the DOM node is reused after a requery and an expanded cluster is
                  not folded back -->
                <details>
                  <summary class="row" class:created={c.source === "created"}>
                    <!-- The chevron comes after the time: putting it first would push this
                      row's time out of the time column and make a cluster look indented -->
                    <span class="time" title={t(c.source === "created" ? "timeline.createdTime" : "timeline.modifiedTime")}>{hhmm(c.time)}</span>
                    <span class="chevron"><Chevron /></span>
                    <span class="title">{tn("timeline.noteCount", c.rows.length)}</span>
                    <span class="path">{t(c.source === "created" ? "timeline.sameMinuteCreated" : "timeline.sameMinuteModified")}</span>
                  </summary>
                  <ul class="sub">
                    {#each c.rows as r (r.note.id)}
                      {@render noteRow(r)}
                    {/each}
                  </ul>
                </details>
              </li>
            {/if}
          {/each}
        </ul>
      </section>
    {:else}
      {#if query.trim()}
        <ViewEmpty onclear={() => (query = "")}>{t("timeline.noMatch", { query })}</ViewEmpty>
      {:else}
        <ViewEmpty>{t("timeline.empty")}</ViewEmpty>
      {/if}
    {/each}
  </div>
</div>

<style>
  .view {
    height: 100%;
    display: flex;
    flex-direction: column;
    min-height: 0;
  }
  .body {
    flex: 1;
    overflow-y: auto;
    padding: 10px 20px 30px;
  }
  section {
    margin-bottom: 10px;
  }
  .news-wrap {
    position: sticky;
    top: 0;
    height: 0;
    z-index: 2;
    display: flex;
    justify-content: center;
  }
  .news {
    margin-top: 6px;
    font-size: var(--fs-xs);
    color: var(--accent);
    background: var(--bg);
    border: 1px solid var(--accent);
    border-radius: 999px;
    padding: 2px 12px;
    box-shadow: var(--shadow);
    white-space: nowrap;
  }
  .news:hover {
    background: var(--bg-hover);
  }
  h3 {
    font-size: var(--fs-sm);
    font-weight: 600;
    color: var(--fg-muted);
    padding: 8px 4px 4px;
    position: sticky;
    top: 0;
    background: var(--bg);
    /* .body has a 10px padding-top: while stuck, a shadow of the same colour backs the gap
       above as well, so the row's text no longer peeks out from under the heading */
    box-shadow: 0 -10px 0 0 var(--bg);
  }
  /* The timeline: the name promises an axis, where before there was only a grouped list of
     section > h3 + ul. The axis is drawn on each day group's ul rather than down the whole
     column — the h3 is sticky, and a continuous vertical line would pass behind a stuck
     heading */
  ul {
    list-style: none;
    position: relative;
    padding-left: 18px;
  }
  ul::before {
    content: "";
    position: absolute;
    left: 8px;
    top: 8px;
    bottom: 8px;
    width: 1px;
    background: var(--border-strong);
  }
  .row {
    position: relative;
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 5px 8px;
    border-radius: 6px;
    font-size: var(--fs-sm);
    cursor: pointer;
  }
  /* The node on the axis. It hangs on .row rather than the li: a folded cluster's li also
     holds the expanded sublist, and centring by li would put the dot in the middle of the
     whole cluster */
  .row::before {
    content: "";
    position: absolute;
    left: -13px;
    top: 50%;
    transform: translateY(-50%);
    width: 7px;
    height: 7px;
    border-radius: 50%;
    background: var(--bg);
    border: 1.5px solid var(--border-strong);
  }
  /* A solid dot means created. The same reasoning as T1's: "modified" is the default, and only the exception is marked */
  .row.created::before {
    background: var(--accent);
    border-color: var(--accent);
  }
  .row:hover {
    background: var(--bg-hover);
  }
  /* An expanded member row is indented one step, and neither the axis nor the node is redrawn — a cluster is one event on the axis */
  ul.sub {
    padding-left: 14px;
  }
  ul.sub::before,
  ul.sub .row::before {
    display: none;
  }
  /* summary is display:flex (see .row), and the UA's disclosure triangle only renders on
     display:list-item, so it disappeared with it; the chevron is the same collapse language
     as the file tree's, the outline's and Agenda's */
  .chevron {
    flex-shrink: 0;
    display: inline-flex;
    color: var(--fg-faint);
    transition: transform 0.12s;
  }
  details[open] > summary .chevron {
    transform: rotate(90deg);
  }
  .time {
    flex-shrink: 0;
    font-family: var(--font-mono);
    font-size: var(--fs-xs);
    color: var(--fg-muted);
  }
  .title {
    font-weight: 600;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    /* It shares the width with .path: a long title can shrink rather than squeezing the path out */
    min-width: 0;
    flex-shrink: 1;
    max-width: 55%;
    -webkit-user-select: text;
    user-select: text;
  }
  .path {
    flex: 1;
    min-width: 0;
    font-family: var(--font-mono);
    font-size: var(--fs-xs);
    color: var(--fg-muted);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .chip {
    flex-shrink: 0;
    font-size: var(--fs-xs);
    color: var(--fg-muted);
    background: var(--bg-hover);
    border-radius: 4px;
    padding: 1px 6px;
  }
  .source {
    flex-shrink: 0;
    font-size: var(--fs-xs);
    color: var(--fg-muted);
  }
  .lock {
    display: inline-flex;
    flex-shrink: 0;
    color: var(--accent);
  }
</style>
