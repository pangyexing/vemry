<script lang="ts">
  import { t, tn } from "$lib/i18n/i18n.svelte";
  import { vault } from "$lib/stores/vault.svelte";
  import { viewContext } from "$lib/stores/viewContext.svelte";
  import { toast } from "$lib/stores/toast.svelte";
  import { askAboutView } from "$lib/askview";
  import { api, type BlockQuery } from "$lib/api";
  import { fuzzyKeep } from "$lib/commands/registry";
  import { cardKey, GRADES, gradeCard, isDue, parseCard, rekey, schedule, type Card, type Grade } from "$lib/views/cards";
  import { chipDate, localDate } from "$lib/views/tasks";
  import { byNote } from "$lib/views/queryrows";
  import ViewHeader, { countLabel } from "./ViewHeader.svelte";
  import ViewEmpty from "./ViewEmpty.svelte";

  // ---- The Review view: the flashcards due today, one at a time — question, answer on
  // demand, four grades. Cards are `[!card]` callouts anywhere in the vault (the query is
  // `tag: card`, the same projection API every view uses); a grade writes the schedule
  // fields back onto the card's line, so the note is the deck and there is no study database.
  const QUERY: BlockQuery = { tag: "card", limit: 5000 };

  let cards = $state<Card[]>([]);
  let query = $state("");
  // Fixed at load time (a session straddling midnight simply keeps the day it started)
  let today = $state(localDate());
  // With events firing in quick succession an older response may arrive later: only the newest one lands
  let loadSeq = 0;
  /** Cards graded this round, by key: the count for the progress line, and what holds a
   *  card graded Again (still due) at the end of the queue instead of letting it come
   *  straight back. Re-keyed on every load (a grade shifts the offsets below it). */
  let graded = $state<Record<string, Grade>>({});
  /** The key of the card whose answer is showing (flipping is per card, so a new card starts face down). */
  let flippedFor = $state<string | null>(null);
  /** Study ahead: with nothing due, the user may go through the cards scheduled for later. */
  let ahead = $state(false);
  let root = $state<HTMLDivElement | null>(null);

  async function load() {
    const seq = ++loadSeq;
    const hits = await api.queryBlocks(QUERY);
    if (seq !== loadSeq) return;
    const next = hits.map(parseCard);
    graded = rekey(graded, cards, next);
    cards = next;
    today = localDate();
  }

  $effect(() => {
    void vault.indexVersion;
    load().catch((e) => (vault.error = String(e)));
  });

  /** The filter key = question, answer, the note's title and path: "the cards about X" and "this deck" in one box. */
  const shown = $derived(fuzzyKeep(cards, query, (c) => `${c.front} ${c.back} ${c.hit.note_title} ${c.hit.note}`));
  const dueCards = $derived(shown.filter((c) => isDue(c, today)));
  /** Overdue first (the oldest due date first), then the never-graded, then by note. */
  const byDue = (a: Card, b: Card) => (a.due ?? "9999").localeCompare(b.due ?? "9999") || byNote(a.hit, b.hit);
  const queue = $derived.by(() => {
    const pool = ahead ? shown : dueCards;
    const fresh = pool.filter((c) => !(cardKey(c) in graded)).sort(byDue);
    const again = pool.filter((c) => graded[cardKey(c)] === "again");
    return [...fresh, ...again];
  });
  // Typed by hand: an index into an array is never null to TS, and the null is the point here
  const current = $derived<Card | null>(queue[0] ?? null);
  const currentKey = $derived(current ? cardKey(current) : null);
  const flipped = $derived(currentKey !== null && flippedFor === currentKey);
  const doneCount = $derived(Object.values(graded).filter((g) => g !== "again").length);
  /** The next date any card falls due after today (for the all-done line). */
  const nextDue = $derived.by(() => {
    let best: string | null = null;
    for (const c of shown) if (c.due && c.due > today && (best === null || c.due < best)) best = c.due;
    return best;
  });

  // The header's "Ask AI" and the @ picker's current-view candidate carry this view's query
  $effect(() => {
    viewContext.set({
      view: "review",
      label: query.trim() ? `${t("view.review")} · ${query.trim()}` : t("view.review"),
      query: QUERY,
      rows: shown.length,
    });
    return () => viewContext.set(null);
  });

  // Keys work as soon as the view opens (Space to flip, 1–4 to grade) without a click first
  $effect(() => {
    root?.focus();
  });

  function flip() {
    if (current) flippedFor = cardKey(current);
  }

  /** What each grade would schedule, for the button labels (the interval preview is the
   *  one thing that makes the four grades mean something on a card seen for the first time). */
  function preview(c: Card, g: Grade): string {
    const s = schedule(c, g, today);
    return s.interval === 0 ? t("reviewView.today") : t("reviewView.days", { n: s.interval });
  }

  let grading = false;
  async function grade(g: Grade) {
    const c = current;
    if (!c || grading) return;
    grading = true;
    try {
      const next = await gradeCard(c, g, today);
      if (!next) {
        toast.push(t("reviewView.stale"), "error");
        await load();
        return;
      }
      graded = { ...graded, [cardKey(c)]: g };
      // The write reindexed the note before returning: reloading now gives every card its
      // fresh span (the graded one moves on or, after Again, to the tail of the queue)
      await load();
    } catch (e) {
      toast.push(String(e), "error");
    } finally {
      grading = false;
    }
  }

  function onKey(e: KeyboardEvent) {
    const el = e.target as HTMLElement | null;
    if (el && (el.tagName === "INPUT" || el.tagName === "TEXTAREA")) return;
    if (!current) return;
    if (e.key === " " || e.key === "Enter") {
      e.preventDefault();
      if (flipped) void grade("good");
      else flip();
    } else if (flipped && e.key >= "1" && e.key <= "4") {
      e.preventDefault();
      void grade(GRADES[Number(e.key) - 1]);
    }
  }

  function openCurrent() {
    if (current) void vault.openBlockAt(current.hit.note, current.hit.block.span.start, current.hit.block.content);
  }
</script>

<!-- svelte-ignore a11y_no_noninteractive_tabindex, a11y_no_noninteractive_element_interactions -->
<div class="view" bind:this={root} role="region" aria-label={t("view.review")} tabindex="0" onkeydown={onKey}>
  <ViewHeader
    title={t("view.review")}
    total="{countLabel(shown.length, cards.length, 'review.count')} / {t('reviewView.due', { n: dueCards.length })}"
    filterPlaceholder={t("reviewView.filterPlaceholder")}
    bind:filter={query}
    askAi={askAboutView}
  />
  <div class="body">
    {#if cards.length === 0}
      <ViewEmpty>
        {t("reviewView.empty1")}<code>&gt; [!card] question</code>{t("reviewView.empty2")}
      </ViewEmpty>
    {:else if shown.length === 0}
      <ViewEmpty onclear={() => (query = "")}>{t("reviewView.noMatch", { query })}</ViewEmpty>
    {:else if current}
      {@const c = current}
      <div class="stage">
        <div class="meta">
          <span class="progress">{doneCount} / {doneCount + queue.length}</span>
          {#if ahead}<span class="ahead-tag">{t("reviewView.aheadHint")}</span>{/if}
          <span class="spring"></span>
          <button class="note-link" onclick={openCurrent}>{c.hit.note_title}</button>
          {#if c.due === null}<span class="tag">{t("reviewView.newCard")}</span>{/if}
        </div>
        <article class="card">
          <p class="front">{c.front}</p>
          {#if flipped}
            <hr />
            <p class="back" class:none={!c.back}>{c.back || t("reviewView.noAnswer")}</p>
          {/if}
        </article>
        <div class="actions">
          {#if !flipped}
            <button class="primary" onclick={flip}>{t("reviewView.showAnswer")}<kbd>␣</kbd></button>
          {:else}
            {#each GRADES as g, i (g)}
              <button class="grade {g}" onclick={() => void grade(g)}>
                <span class="g-label">{t(`reviewView.grade.${g}` as never)}</span>
                <span class="g-when">{preview(c, g)}</span>
                <kbd>{i + 1}</kbd>
              </button>
            {/each}
          {/if}
        </div>
      </div>
    {:else}
      <div class="done">
        <div class="done-mark">✓</div>
        <p class="done-title">{t("reviewView.allDone")}</p>
        <p class="done-sub">
          {tn("review.count", shown.length)}{#if nextDue} · {t("reviewView.nextDue", { date: chipDate(nextDue) })}{/if}
        </p>
        {#if !ahead && shown.some((c) => !(cardKey(c) in graded))}
          <button class="ahead" onclick={() => (ahead = true)}>{t("reviewView.studyAhead")}</button>
        {/if}
      </div>
    {/if}
  </div>
</div>

<style>
  .view {
    height: 100%;
    display: flex;
    flex-direction: column;
    min-height: 0;
    outline: none;
  }
  .body {
    flex: 1;
    overflow-y: auto;
    padding: 10px 20px 30px;
  }
  .stage {
    max-width: 640px;
    margin: 24px auto 0;
    display: flex;
    flex-direction: column;
    gap: 14px;
  }
  .meta {
    display: flex;
    align-items: baseline;
    gap: 10px;
    font-size: var(--fs-xs);
    color: var(--fg-muted);
  }
  .progress {
    font-family: var(--font-mono);
    font-variant-numeric: tabular-nums;
  }
  .spring {
    flex: 1;
  }
  .note-link {
    font-size: var(--fs-xs);
    color: var(--fg-muted);
  }
  .note-link:hover {
    color: var(--accent);
  }
  .tag,
  .ahead-tag {
    font-size: 10px;
    line-height: 1;
    padding: 2px 5px;
    border-radius: 3px;
    border: 1px solid var(--border);
  }
  .card {
    border: 1px solid var(--border);
    border-left: 2px solid var(--accent);
    border-radius: 8px;
    padding: 22px 26px;
    background: var(--bg-panel);
    min-height: 140px;
  }
  .front {
    font-size: var(--fs-lg);
    font-weight: 600;
    line-height: 1.5;
    white-space: pre-wrap;
    text-wrap: balance;
  }
  hr {
    border: none;
    border-top: 1px solid var(--border);
    margin: 16px 0;
  }
  .back {
    font-size: var(--fs-sm);
    line-height: 1.6;
    white-space: pre-wrap;
  }
  .back.none {
    color: var(--fg-faint);
    font-style: italic;
  }
  .actions {
    display: flex;
    gap: 8px;
    justify-content: center;
  }
  .actions button {
    display: inline-flex;
    align-items: baseline;
    gap: 8px;
    padding: 7px 14px;
    border-radius: 6px;
    border: 1px solid var(--border);
    font-size: var(--fs-sm);
  }
  .actions button:hover {
    background: var(--bg-hover);
  }
  .primary {
    color: var(--accent);
    border-color: var(--accent) !important;
  }
  .grade {
    flex-direction: column;
    align-items: center !important;
    gap: 2px !important;
    min-width: 88px;
    padding: 8px 10px !important;
  }
  .g-label {
    font-weight: 600;
  }
  .g-when {
    font-size: var(--fs-xs);
    color: var(--fg-muted);
    font-variant-numeric: tabular-nums;
  }
  /* The four grades read by position and label; only Again takes a warning tint (it is the one that undoes progress) */
  .grade.again .g-label {
    color: var(--danger);
  }
  kbd {
    font-family: var(--font-mono);
    font-size: 10px;
    color: var(--fg-faint);
    border: 1px solid var(--border);
    border-radius: 3px;
    padding: 0 4px;
    line-height: 1.5;
  }
  .done {
    max-width: 640px;
    margin: 60px auto 0;
    text-align: center;
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 6px;
  }
  .done-mark {
    font-size: 28px;
    color: var(--accent);
  }
  .done-title {
    font-size: var(--fs-md);
    font-weight: 600;
  }
  .done-sub {
    font-size: var(--fs-xs);
    color: var(--fg-muted);
  }
  .ahead {
    margin-top: 10px;
    padding: 6px 14px;
    border-radius: 6px;
    border: 1px solid var(--border);
    font-size: var(--fs-sm);
  }
  .ahead:hover {
    background: var(--bg-hover);
  }
</style>
