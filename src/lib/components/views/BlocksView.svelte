<script lang="ts">
  // The Blocks projection: any block of the vault as a row, with a kind filter, a
  // field condition in the flat grammar (`author = X`, `due < 2026-09-10`, `rating`),
  // grouping and sorting. The query goes to Rust (the authority on the field
  // grammar; a bad condition shows its message inline and keeps the last rows);
  // clicking a row lands on the block. The view state persists per vault.
  import { api, type BlockHit } from "$lib/api";
  import { vault } from "$lib/stores/vault.svelte";
  import { t, tn } from "$lib/i18n/i18n.svelte";
  import { fuzzyKeep } from "$lib/commands/registry";
  import { loadBucketMap, saveBucket } from "$lib/persist";
  import {
    KIND_CHOICES,
    MAX_ROWS,
    fieldKeysOf,
    groupRows,
    kindGlyph,
    kindLabel,
    rowFilterKey,
    rowKey,
    rowOf,
    sortRows,
    type GroupBy,
    type KindChoice,
    type SortBy,
  } from "$lib/views/blocks";
  import ViewHeader, { countLabel } from "./ViewHeader.svelte";
  import ViewEmpty from "./ViewEmpty.svelte";
  import Chevron from "../Chevron.svelte";
  import PrivacyIcon from "../PrivacyIcon.svelte";

  const UI_KEY = "vemry-blocks-ui";
  type Ui = { kind: KindChoice; field: string; groupBy: GroupBy; sortBy: SortBy; desc: boolean };
  const saved = vault.root ? loadBucketMap<Ui>(UI_KEY)[vault.root] : undefined;
  let kind = $state<KindChoice>(saved?.kind && KIND_CHOICES.includes(saved.kind) ? saved.kind : "any");
  let fieldText = $state(saved?.field ?? "");
  let groupBy = $state<GroupBy>(saved?.groupBy ?? "note");
  let sortBy = $state<SortBy>(saved?.sortBy ?? "note");
  let desc = $state(saved?.desc ?? false);
  let query = $state("");

  function persistUi() {
    if (!vault.root) return;
    saveBucket(UI_KEY, vault.root, { kind, field: fieldText, groupBy, sortBy, desc }, vault.recentLoaded ? vault.recent : null);
  }

  let hits = $state<BlockHit[]>([]);
  let error = $state<string | null>(null);
  let loadSeq = 0;
  let fieldTimer: ReturnType<typeof setTimeout> | null = null;

  async function load(kindNow: KindChoice, field: string) {
    const seq = ++loadSeq;
    try {
      const res = await api.queryBlocks({ kind: kindNow === "any" ? undefined : kindNow, field: field.trim() || undefined, limit: 5000 });
      if (seq !== loadSeq) return;
      hits = res;
      error = null;
    } catch (e) {
      if (seq !== loadSeq) return;
      // a bad field condition: the message shows inline and the last rows stay
      error = String(e);
    }
  }

  // The kind switch and an index change requery at once; the field text waits for a pause in typing
  $effect(() => {
    void vault.indexVersion;
    const k = kind;
    const f = fieldText;
    if (fieldTimer) clearTimeout(fieldTimer);
    fieldTimer = setTimeout(() => void load(k, f), 300);
    return () => {
      if (fieldTimer) clearTimeout(fieldTimer);
    };
  });

  const allRows = $derived(hits.map(rowOf));
  const shown = $derived(fuzzyKeep(allRows, query, rowFilterKey));
  const sorted = $derived(sortRows(shown, sortBy, desc));
  const capped = $derived(sorted.slice(0, MAX_ROWS));
  const groups = $derived(groupRows(capped, groupBy));
  const overflow = $derived(sorted.length - capped.length);
  const fieldKeys = $derived(fieldKeysOf(allRows));

  const openRow = (hit: BlockHit) => vault.openBlockAt(hit.note, hit.block.span.start, hit.block.content);
</script>

<div class="view">
  <ViewHeader
    title={t("view.blocks")}
    total={countLabel(shown.length, allRows.length, "blocks.count")}
    filterPlaceholder={t("blocks.filterPlaceholder")}
    bind:filter={query}
  >
    {#snippet left()}
      <select class="ctl" title={t("blocks.kind")} bind:value={kind} onchange={persistUi}>
        {#each KIND_CHOICES as k (k)}
          <option value={k}>{k === "any" ? t("blocks.anyKind") : kindLabel(k)}</option>
        {/each}
      </select>
      <input
        class="ctl field"
        class:bad={error !== null}
        placeholder={t("blocks.fieldPlaceholder")}
        title={t("blocks.fieldHint")}
        bind:value={fieldText}
        onchange={persistUi}
        spellcheck="false"
      />
    {/snippet}
    {#snippet right()}
      <label class="ctl-label"
        ><span>{t("blocks.groupBy")}</span>
        <select class="ctl" bind:value={groupBy} onchange={persistUi}>
          <option value="none">{t("blocks.groupNone")}</option>
          <option value="note">{t("blocks.byNote")}</option>
          <option value="kind">{t("blocks.byKind")}</option>
          {#each fieldKeys as key (key)}
            <option value={`field:${key}`}>{key}</option>
          {/each}
        </select></label
      >
      <label class="ctl-label"
        ><span>{t("blocks.sortBy")}</span>
        <select class="ctl" bind:value={sortBy} onchange={persistUi}>
          <option value="note">{t("blocks.byNote")}</option>
          <option value="kind">{t("blocks.byKind")}</option>
          <option value="text">{t("blocks.byText")}</option>
          {#each fieldKeys as key (key)}
            <option value={`field:${key}`}>{key}</option>
          {/each}
        </select></label
      >
      <button
        class="ctl dir"
        title={desc ? t("blocks.descending") : t("blocks.ascending")}
        onclick={() => {
          desc = !desc;
          persistUi();
        }}>{desc ? "↓" : "↑"}</button
      >
    {/snippet}
  </ViewHeader>
  <div class="body">
    {#if error}
      <div class="error">{error}</div>
    {/if}
    {#if allRows.length === 0}
      {#if !error}
        <ViewEmpty>{t("blocks.empty")}</ViewEmpty>
      {/if}
    {:else if shown.length === 0}
      <ViewEmpty onclear={() => (query = "")}>{t("blocks.noMatch", { query })}</ViewEmpty>
    {:else}
      {#each groups as g (g.key)}
        {#if groupBy === "none"}
          <ul>
            {#each g.rows as r (rowKey(r))}
              {@render row(r)}
            {/each}
          </ul>
        {:else}
          <details open>
            <summary>
              <span class="chevron"><Chevron /></span>
              <span>{g.title}</span><span class="count">{g.rows.length}</span>
            </summary>
            <ul>
              {#each g.rows as r (rowKey(r))}
                {@render row(r)}
              {/each}
            </ul>
          </details>
        {/if}
      {/each}
      {#if overflow > 0}
        <div class="more">{tn("blocks.more", overflow)}</div>
      {/if}
    {/if}
  </div>
</div>

{#snippet row(r: import("$lib/views/blocks").BlockRow)}
  <li>
    <button class="row" title={`${r.hit.note} · ${kindLabel(r.kind)}`} onclick={() => void openRow(r.hit)}>
      <span class="glyph" aria-hidden="true">{kindGlyph(r.kind)}</span>
      <span class="text">{r.text || kindLabel(r.kind)}</span>
      <span class="meta">
        {#each r.chips as [key, value] (key)}
          <span class="chip"><span class="fk">{key}</span>{value}</span>
        {/each}
        {#if groupBy !== "note"}<span class="chip note-chip">{r.hit.note_title}</span>{/if}
        {#if vault.privatePaths.has(r.hit.note)}<span class="lock" title={t("statusBar.privacyZoneHint")}><PrivacyIcon size={14} /></span>{/if}
      </span>
    </button>
  </li>
{/snippet}

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
  .ctl {
    font-size: var(--fs-xs);
    padding: 2px 6px;
    border: 1px solid var(--border);
    border-radius: 5px;
    background: var(--bg);
    color: var(--fg);
  }
  .ctl.field {
    width: 14em;
    font-family: var(--font-mono);
  }
  .ctl.field.bad {
    border-color: var(--danger);
  }
  .ctl.dir {
    cursor: pointer;
    min-width: 26px;
  }
  .ctl-label {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    font-size: var(--fs-xs);
    color: var(--fg-faint);
  }
  .error {
    margin: 0 0 8px;
    padding: 6px 10px;
    border-radius: 6px;
    background: color-mix(in srgb, var(--danger) 12%, transparent);
    color: var(--danger);
    font-size: var(--fs-xs);
    font-family: var(--font-mono);
    white-space: pre-wrap;
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
    user-select: none;
  }
  .chevron {
    display: inline-flex;
    align-self: center;
    color: var(--fg-faint);
    transition: transform 0.12s;
  }
  details[open] > summary .chevron {
    transform: rotate(90deg);
  }
  .count {
    font-size: var(--fs-xs);
    font-weight: 400;
    color: var(--fg-faint);
  }
  ul {
    list-style: none;
  }
  .row {
    display: grid;
    grid-template-columns: 1.4em minmax(0, 1fr) auto;
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
  .glyph {
    color: var(--fg-faint);
    font-family: var(--font-mono);
    font-size: var(--fs-xs);
    text-align: center;
  }
  .text {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .meta {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    flex-shrink: 0;
  }
  .chip {
    font-size: var(--fs-xs);
    padding: 1px 7px;
    border-radius: 999px;
    background: var(--bg-hover);
    color: var(--fg-muted);
    white-space: nowrap;
  }
  .chip .fk {
    color: var(--fg-faint);
    margin-right: 3px;
  }
  .chip .fk::after {
    content: ":";
  }
  .note-chip {
    color: var(--fg-faint);
  }
  .lock {
    display: inline-flex;
    color: var(--accent);
  }
  .more {
    padding: 8px;
    font-size: var(--fs-xs);
    color: var(--fg-faint);
  }
</style>
