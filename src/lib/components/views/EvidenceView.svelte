<script lang="ts">
  import { t } from "$lib/i18n/i18n.svelte";
  import { vault } from "$lib/stores/vault.svelte";
  import { calloutText, groupByNote, loadClaimGraph, type EvidenceWithClaims } from "$lib/views/claims";
  import { fuzzyKeep } from "$lib/commands/registry";
  import ViewHeader, { countLabel } from "./ViewHeader.svelte";
  import ViewEmpty from "./ViewEmpty.svelte";

  let evidence = $state<EvidenceWithClaims[]>([]);
  let onlyBroken = $state(false);
  let query = $state("");
  // With events firing in quick succession an older response may arrive later: only the newest one lands
  let loadSeq = 0;

  $effect(() => {
    void vault.indexVersion;
    const seq = ++loadSeq;
    loadClaimGraph()
      .then((g) => {
        if (seq === loadSeq) evidence = g.evidence;
      })
      .catch((e) => (vault.error = String(e)));
  });

  /** A broken link: the note did not resolve, or it resolved but the target holds no claim with that ^id */
  const isBroken = (ev: EvidenceWithClaims) => ev.links.some((l) => !l.claim);
  const brokenCount = $derived(evidence.filter(isBroken).length);
  /** The switch filters first and the filter box second: the two are an AND, where "broken
   *  links only" narrows the scope and the filter box searches within it. The filter key
   *  includes supports' raw target, so "which evidence is attached to a given claim" goes
   *  through the same box. */
  const scoped = $derived(onlyBroken ? evidence.filter(isBroken) : evidence);
  const shown = $derived(
    fuzzyKeep(
      scoped,
      query,
      (ev) => `${ev.text} ${ev.evidence.note_title} ${ev.evidence.note} ${ev.links.map((l) => l.target).join(" ")}`,
    ),
  );
  const byNote = $derived(groupByNote(shown, (ev) => ev.evidence));
</script>

<div class="view">
  <ViewHeader
    title={t("view.evidence")}
    total="{countLabel(shown.length, evidence.length, 'evidence.count')} / {t('evidenceView.broken', { n: brokenCount })}"
    filterPlaceholder={t("evidenceView.filterPlaceholder")}
    bind:filter={query}
  >
    {#snippet right()}
      <label class="toggle">
        <input type="checkbox" bind:checked={onlyBroken} />
        {t("evidenceView.brokenOnly")}
      </label>
    {/snippet}
  </ViewHeader>
  <div class="body">
    {#if evidence.length === 0}
      <ViewEmpty>
        {t("evidenceView.empty1")}<code>&gt; [!evidence] data supports:[[note#^id]]</code>{t("evidenceView.empty2")}
      </ViewEmpty>
      <!-- What emptied this screen may be the filter box or the "broken links only" switch.
        The way back belongs to whichever is in the way — so the switch branch offers no
        "clear filter" button, which would do nothing if pressed -->
    {:else if shown.length === 0 && query.trim()}
      <ViewEmpty onclear={() => (query = "")}>{t("evidenceView.noMatch", { query })}</ViewEmpty>
    {:else if shown.length === 0}
      <ViewEmpty>{t("evidenceView.noneBroken", { n: evidence.length })}</ViewEmpty>
    {/if}
    {#each byNote as group (group.note)}
      <section>
        <h3>
          <button class="note-link" onclick={() => vault.openNote(group.note)}>{group.title}</button>
          <span class="path">{group.note}</span>
          {#if vault.privatePaths.has(group.note)}<span class="lock" title={t("statusBar.privacyZoneHint")}>🔒</span>{/if}
        </h3>
        <!-- A user may write the same explicit ^id twice, so it cannot be a keyed-each key (production throws too) -->
        {#each group.items as ev, i (i)}
          <article class="card">
            <div
              class="ev"
              role="button"
              tabindex="0"
              onclick={() => vault.openNote(ev.evidence.note)}
              onkeydown={(e) => (e.key === "Enter" || e.key === " ") && vault.openNote(ev.evidence.note)}
            >
              <span class="text">{ev.text}</span>
            </div>
            <div class="links">
              {#if ev.links.length === 0}
                <!-- Only the words "unattached" remain: this hangs on every unwired evidence
                  card, being unwired is the normal state of a vault that is still growing,
                  and the `supports` keyword retreats into the title to be read when needed -->
                <span class="chip faint" title={t("evidenceView.unattachedHint")}>{t("evidenceView.unattached")}</span>
              {/if}
              {#each ev.links as link, j (j)}
                {#if link.claim}
                  {@const claim = link.claim}
                  <button class="chip ok" title={calloutText(claim.block.content)} onclick={() => vault.openNote(claim.note)}>
                    ⊢ {claim.note_title} ^{claim.block.local_id}
                  </button>
                {:else if link.resolvedNote}
                  <button class="chip warn" title={t("evidenceView.noMatchingClaimHint", { id: link.target.split("#^")[1] ?? "" })} onclick={() => link.resolvedNote && vault.openNote(link.resolvedNote)}>
                    {t("evidenceView.noMatchingClaim", { target: link.target })}
                  </button>
                {:else}
                  <span class="chip broken" title={t("evidenceView.unresolvedHint")}>{t("evidenceView.unresolved", { target: link.target })}</span>
                {/if}
              {/each}
            </div>
          </article>
        {/each}
      </section>
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
  .toggle {
    display: flex;
    align-items: center;
    gap: 5px;
    font-size: var(--fs-sm);
    color: var(--fg-muted);
    cursor: pointer;
  }
  .body {
    flex: 1;
    overflow-y: auto;
    padding: 10px 20px 30px;
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
    color: var(--fg-faint);
  }
  .card {
    border: 1px solid var(--border);
    border-radius: 6px;
    margin: 6px 0;
    padding: 8px 10px;
  }
  .ev {
    cursor: pointer;
    border-radius: 4px;
    padding: 2px 4px;
  }
  .ev:hover {
    background: var(--bg-hover);
  }
  .text {
    font-size: var(--fs-sm);
  }
  .links {
    display: flex;
    flex-wrap: wrap;
    gap: 5px;
    margin-top: 6px;
    padding-left: 4px;
  }
  .chip {
    font-size: var(--fs-xs);
    border-radius: 4px;
    padding: 2px 7px;
    background: var(--bg-hover);
    color: var(--fg-muted);
  }
  .chip.ok {
    color: var(--accent);
  }
  .chip.ok:hover,
  .chip.warn:hover {
    filter: brightness(1.1);
    text-decoration: underline;
  }
  .chip.warn {
    color: var(--danger);
  }
  .chip.broken {
    color: var(--fg-faint);
    text-decoration: line-through;
  }
  .chip.faint {
    color: var(--fg-faint);
  }
  .lock {
    font-size: var(--fs-xs);
  }
</style>
