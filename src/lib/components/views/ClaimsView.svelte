<script lang="ts">
  import { t } from "$lib/i18n/i18n.svelte";
  import { vault } from "$lib/stores/vault.svelte";
  import { groupByNote, loadClaimGraph, type ClaimWithEvidence } from "$lib/views/claims";
  import { fuzzyKeep } from "$lib/commands/registry";
  import ViewHeader, { countLabel } from "./ViewHeader.svelte";
  import ViewEmpty from "./ViewEmpty.svelte";
  import PrivacyIcon from "../PrivacyIcon.svelte";
  import { cliptip } from "$lib/cliptip";
  import { selectingIn } from "$lib/views/select";

  /** A row opens its note on a click, unless the click ended a drag that selected its text. */
  function open(e: MouseEvent, path: string) {
    if (!selectingIn(e.currentTarget as Element)) void vault.openNote(path);
  }

  let claims = $state<ClaimWithEvidence[]>([]);
  let query = $state("");
  // With events firing in quick succession an older response may arrive later: only the newest one lands
  let loadSeq = 0;

  $effect(() => {
    void vault.indexVersion;
    const seq = ++loadSeq;
    loadClaimGraph()
      .then((g) => {
        if (seq !== loadSeq) return;
        claims = g.claims;
      })
      .catch((e) => (vault.error = String(e)));
  });

  /** The filter key = the claim's text plus its note's title and path: "which arguments does
   *  this note make" and "which claim mentioned X" are two ways of searching in one box
   *  (the same as Agenda's and the timeline's). */
  const shown = $derived(fuzzyKeep(claims, query, (c) => `${c.text} ${c.claim.note_title} ${c.claim.note}`));
  const unsupported = $derived(shown.filter((c) => c.evidence.length === 0).length);
  const byNote = $derived(groupByNote(shown, (c) => c.claim));
</script>

<div class="view">
  <ViewHeader
    title={t("view.claims")}
    total="{countLabel(shown.length, claims.length, 'claims.count')} / {t('claimsView.unsupported', { n: unsupported })}"
    filterPlaceholder={t("claimsView.filterPlaceholder")}
    bind:filter={query}
  />
  <div class="body">
    <div class="list">
    {#if claims.length === 0}
      <ViewEmpty>
        {t("claimsView.empty1")}<code>&gt; [!claim] a claim ^id</code
        >{t("claimsView.empty2")}<code>&gt; [!evidence] data supports:[[note#^id]]</code
        >{t("claimsView.empty3")}
      </ViewEmpty>
    {:else if shown.length === 0}
      <ViewEmpty onclear={() => (query = "")}>{t("claimsView.noMatch", { query })}</ViewEmpty>
    {/if}
    {#each byNote as group (group.note)}
      <section>
        <h3>
          <button class="note-link" onclick={() => vault.openNote(group.note)}>{group.title}</button>
          <span class="path">{group.note}</span>
          {#if vault.privatePaths.has(group.note)}<span class="lock" title={t("statusBar.privacyZoneHint")}><PrivacyIcon size={14} /></span>{/if}
        </h3>
        <!-- A user may write the same explicit ^id twice, so it cannot be a keyed-each key (production throws too) -->
        {#each group.items as c, i (i)}
          <article class="card" class:unsupported={c.evidence.length === 0}>
            <div
              class="claim"
              role="button"
              tabindex="0"
              onclick={(e) => open(e, c.claim.note)}
              onkeydown={(e) => (e.key === "Enter" || e.key === " ") && vault.openNote(c.claim.note)}
            >
              {#if /^\s*(?:>\s*)?\[!decision\]/i.test(c.claim.block.content)}<span class="kind">{t("claims.decision")}</span>{/if}
              <span class="text">{c.text}</span>
              <!-- The chip is only attached when there is a ^id: a ^id is this claim's handle
                and is content, while "no ^id" is an absence, explained once by the line below
                rather than repeated on every card at the end of the claim's text (most claims
                in a vault have no ^id yet) -->
              {#if c.claim.block.explicit_id}
                <span class="chip id">^{c.claim.block.local_id}</span>
              {/if}
            </div>
            {#if c.evidence.length}
              <ul class="evidence">
                {#each c.evidence as ev, j (j)}
                  <li>
                    <div
                      class="ev-row"
                      role="button"
                      tabindex="0"
                      onclick={(e) => open(e, ev.evidence.note)}
                      onkeydown={(e) => (e.key === "Enter" || e.key === " ") && vault.openNote(ev.evidence.note)}
                    >
                      <span class="ev-mark">⊢</span>
                      <span class="ev-text" use:cliptip={ev.text}>{ev.text}</span>
                      <span class="chip note" use:cliptip={ev.evidence.note_title}>{ev.evidence.note_title}</span>
                    </div>
                  </li>
                {/each}
              </ul>
            {:else}
              <!-- "There is no ^id" and "there is a ^id but no evidence yet" are two absences,
                explained together in one line: the former is the latter's cause (with no
                handle, evidence cannot cite it at all), and writing both side by side would
                say the same thing twice -->
              <p class="no-evidence">
                {t("claimsView.noEvidence")}{#if !c.claim.block.explicit_id}<span
                    class="why"
                    title={t("claimsView.noIdHint")}>{t("claimsView.noId")}</span
                  >{/if}
              </p>
            {/if}
          </article>
        {/each}
      </section>
    {/each}
    </div>
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
  /* A list view's width is capped like any reading surface: on a wide screen a claim ran to
     a line of 170 letters */
  .list {
    max-width: 64em;
    margin: 0 auto;
  }
  section {
    margin-bottom: 16px;
  }
  h3 {
    display: flex;
    align-items: baseline;
    gap: 8px;
    padding: 4px;
    font-size: var(--fs-sm);
  }
  .note-link {
    font-weight: 600;
    font-size: var(--fs-sm);
  }
  .note-link:hover {
    color: var(--accent);
  }
  .path {
    font-family: var(--font-mono);
    font-size: var(--fs-xs);
    color: var(--fg-muted);
  }
  .card {
    border: 1px solid var(--border);
    border-left: 2px solid var(--accent);
    border-radius: 6px;
    margin: 6px 0;
    padding: 8px 10px;
  }
  .card.unsupported {
    border-left-color: var(--fg-faint);
  }
  .claim {
    display: flex;
    align-items: baseline;
    gap: 8px;
    cursor: pointer;
    border-radius: 4px;
    padding: 2px 4px;
  }
  .claim:hover {
    background: var(--bg-hover);
  }
  .kind {
    flex-shrink: 0;
    font-size: 11px;
    line-height: 1;
    padding: 2px 5px;
    border-radius: 3px;
    border: 1px solid var(--border);
    color: var(--fg-muted);
    margin-right: 6px;
  }
  /* The claim is the card's sentence to read: body ink at the reading size, regular weight
     (a whole paragraph in semibold reads as shouting, and the note's title above is the
     semibold that groups), capped at the readable column. Selectable: a phrase of it can be
     copied or looked up */
  .text {
    flex: 1;
    min-width: 0;
    max-width: var(--content-width);
    font-size: var(--fs-md);
    line-height: 1.6;
    -webkit-user-select: text;
    user-select: text;
  }
  .chip {
    flex-shrink: 0;
    font-size: var(--fs-xs);
    color: var(--fg-muted);
    background: var(--bg-hover);
    border-radius: 4px;
    padding: 1px 6px;
  }
  .why {
    /* The same colour and size as the main sentence, separated only by a · — it is the second half of that sentence, not another piece of information */
    border-bottom: 1px dotted var(--fg-faint);
  }
  .chip.id {
    font-family: var(--font-mono);
    color: var(--accent);
  }
  .evidence {
    list-style: none;
    margin-top: 4px;
    padding-left: 14px;
  }
  .ev-row {
    display: flex;
    align-items: baseline;
    gap: 8px;
    padding: 3px 6px;
    border-radius: 4px;
    font-size: var(--fs-sm);
    cursor: pointer;
  }
  .ev-row:hover {
    background: var(--bg-hover);
  }
  .ev-mark {
    color: var(--accent);
    flex-shrink: 0;
  }
  /* Two lines of the evidence, the rest in the clip tip */
  .ev-text {
    flex: 1;
    min-width: 0;
    max-width: var(--content-width);
    color: var(--fg-muted);
    display: -webkit-box;
    -webkit-box-orient: vertical;
    -webkit-line-clamp: 2;
    line-clamp: 2;
    overflow: hidden;
    -webkit-user-select: text;
    user-select: text;
  }
  /* The note's name gives way before the evidence does: a long title shrinks to an ellipsis
     rather than squeezing the sentence beside it into one */
  .chip.note {
    flex-shrink: 1;
    min-width: 3em;
    max-width: 40%;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .no-evidence {
    margin-top: 4px;
    padding-left: 18px;
    font-size: var(--fs-xs);
    color: var(--fg-muted);
  }
  .lock {
    display: inline-flex;
    flex-shrink: 0;
    color: var(--accent);
  }
</style>
