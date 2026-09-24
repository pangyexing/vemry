<script lang="ts">
  import { getContext, setContext, type Snippet } from "svelte";
  import { t } from "$lib/i18n/i18n.svelte";
  import { settingById, settingHint, settingLabel } from "$lib/settings";
  import type { RowContext, SearchContext } from "./context";
  import { form } from "./form.svelte";

  // One setting: its label and one-line hint on the left, the control on the right (or,
  // for a text field or a list, below). The words come from the settings index by id, so
  // the row, the dialog's search and the command palette name it the same way.
  let {
    id,
    block = false,
    hint,
    error = null,
    children,
  }: {
    id: string;
    /** The control goes under the label, full width (text fields, lists, cards). */
    block?: boolean;
    /** A hint that says something only this moment knows (a status, a failure); replaces the index's. */
    hint?: string;
    error?: string | null;
    children?: Snippet;
  } = $props();

  const entry = $derived(settingById(id));
  const ctlId = $derived(`setting-${id.replace(/\./g, "-")}`);
  setContext<RowContext>("setting-row", {
    get id() {
      return id;
    },
    get ctlId() {
      return ctlId;
    },
  });
  const search = getContext<SearchContext | undefined>("settings-search");
  const searching = $derived(!!search?.query);
  const shown = $derived(!searching || !!search?.matches(id));
  const text = $derived(hint ?? (entry ? settingHint(entry) : undefined));
</script>

<div class="row" class:block data-setting={id} hidden={!shown}>
  <label class="label" id="{ctlId}-label" for={ctlId}>{entry ? settingLabel(entry) : id}</label>
  {#if error}
    <small class="hint err">{error}</small>
  {:else if text || form.justSaved === id}
    <small class="hint">{text ?? ""}{#if form.justSaved === id}<span class="saved">{t("settings.saved")}</span>{/if}</small>
  {/if}
  <div class="control">
    {#if searching && entry?.jump}
      <button class="go" onclick={() => search?.go(id)}>{t("settings.goTo")} ›</button>
    {:else}
      {@render children?.()}
    {/if}
  </div>
</div>

<style>
  .row {
    display: grid;
    grid-template-columns: minmax(0, 1fr) auto;
    gap: 2px 18px;
    align-items: center;
    padding: 10px 0;
    border-bottom: 1px solid var(--border);
    border-radius: 6px;
    transition: background 0.6s;
  }
  .row[hidden] {
    display: none;
  }
  /* Every cell is placed by name: in a grid where one item is placed, the free ones are
     auto-placed around it and would change places */
  .label {
    grid-column: 1;
    grid-row: 1;
    font-size: var(--fs-sm);
    font-weight: 500;
    cursor: default;
  }
  .hint {
    grid-column: 1;
    grid-row: 2;
    font-size: var(--fs-xs);
    color: var(--fg-muted);
  }
  .hint.err {
    color: var(--danger);
  }
  .saved {
    margin-left: 8px;
    color: var(--accent);
  }
  .control {
    grid-column: 2;
    grid-row: 1 / span 2;
    justify-self: end;
    display: flex;
    align-items: center;
    gap: 8px;
    min-width: 0;
  }
  .row.block {
    grid-template-columns: minmax(0, 1fr);
  }
  .row.block .control {
    grid-column: 1;
    grid-row: 3;
    justify-self: stretch;
    flex-direction: column;
    align-items: stretch;
    margin-top: 6px;
  }
  /* The row an entry point asked for: a moment of the soft accent, then back */
  .row:global(.flash) {
    background: var(--accent-soft);
  }
  .go {
    font-size: var(--fs-xs);
    color: var(--accent);
    padding: 2px 0;
    align-self: flex-start;
  }
  /* The native controls a page puts in a row take the dialog's field look */
  .control :global(select),
  .control :global(input:not([type="checkbox"]):not([type="radio"])),
  .control :global(textarea) {
    padding: 5px 10px;
    font-size: var(--fs-sm);
    border: 1px solid var(--border);
    border-radius: 6px;
    background: var(--bg);
    color: var(--fg);
    min-width: 0;
  }
  .control :global(select) {
    max-width: 240px;
  }
  @media (prefers-reduced-motion: reduce) {
    .row {
      transition: none;
    }
  }
</style>
