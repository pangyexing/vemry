<!-- The one implementation of the inline-field value menu, raised by a click on a
  field chip that is not a date (dates go to TaskDateMenu): a free input, the candidate
  values (what the block types declare for the key first, then what the vault already
  uses) and "remove field". Enter in the input commits it; a click on a candidate picks
  it; a click outside or Esc cancels without writing. Viewport clamping and the closing
  discipline come from popmenu.ts, shared with TaskDateMenu.
  The input takes focus on mount, so a typed value is one keystroke away from the chip. -->
<script lang="ts">
  import { t } from "$lib/i18n/i18n.svelte";
  import { clampToViewport, dismissOn } from "$lib/popmenu";

  interface Props {
    /** The fixed positioning anchor: where the chip was clicked */
    x: number;
    y: number;
    /** The field's key: the title, and what the input is for */
    key: string;
    /** The field's current value ("" for an empty placeholder): the input's default, and the marked candidate */
    current: string;
    /** The candidates, in the order to list them (already deduplicated and capped by the caller) */
    values: string[];
    /** A numeric input (the type says `number`, or the current value is one) */
    number?: boolean;
    /** null = remove the field; the parent clears the menu's state before writing */
    onpick: (value: string | null) => void;
    /** Cancelled by a click outside or Escape: nothing is written */
    onclose: () => void;
  }
  let { x, y, key, current, values, number = false, onpick, onclose }: Props = $props();

  let el = $state<HTMLElement | null>(null);
  let input = $state<HTMLInputElement | null>(null);
  /** The landing point: rendered at the raw coordinates first, then pulled inside the
   *  viewport once it has a rect of its own. */
  let clamped = $state({ x: 0, y: 0 });
  $effect(() => {
    clamped = { x, y };
    queueMicrotask(() => {
      if (el) clamped = clampToViewport(el, x, y);
    });
  });
  const dismiss = dismissOn(() => el, () => onclose());
  $effect(() => {
    input?.focus();
    input?.select();
  });

  function commit() {
    const v = input?.value.trim() ?? "";
    if (v === "") onclose();
    else onpick(v);
  }
</script>

<svelte:window onpointerdowncapture={dismiss.onpointerdowncapture} onkeydowncapture={dismiss.onkeydowncapture} />

<div class="menu" bind:this={el} style:left="{clamped.x}px" style:top="{clamped.y}px" role="menu" tabindex="-1">
  <div class="menu-title">{key}</div>
  <input
    bind:this={input}
    type={number ? "number" : "text"}
    step="any"
    placeholder={t("field.enterValue")}
    value={current}
    onkeydown={(e) => {
      if (e.key !== "Enter" || e.isComposing) return;
      e.preventDefault();
      commit();
    }}
  />
  {#each values as v (v)}
    <button role="menuitem" class:current={v === current} onclick={() => onpick(v)}>{v}</button>
  {/each}
  <div class="menu-sep"></div>
  <button role="menuitem" class="danger" onclick={() => onpick(null)}>{t("field.remove")}</button>
</div>

<style>
  .menu {
    position: fixed;
    z-index: 250;
    min-width: 150px;
    max-width: 300px;
    background: var(--bg-panel);
    border: 1px solid var(--border);
    border-radius: 8px;
    box-shadow: var(--shadow);
    padding: 4px;
    display: flex;
    flex-direction: column;
  }
  .menu-title {
    font-size: var(--fs-xs);
    color: var(--fg-faint);
    padding: 4px 10px 2px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .menu-title::after {
    content: "::";
    margin-left: 1px;
  }
  .menu-sep {
    height: 1px;
    background: var(--border);
    margin: 4px 6px;
  }
  .menu button {
    text-align: left;
    padding: 6px 10px;
    border-radius: 5px;
    font-size: var(--fs-sm);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .menu button:hover {
    background: var(--bg-hover);
  }
  .menu button.current {
    color: var(--accent);
  }
  .menu button.danger {
    color: var(--danger);
  }
  .menu input {
    margin: 2px 6px 4px;
    padding: 3px 6px;
    font-size: var(--fs-sm);
    border: 1px solid var(--border);
    border-radius: 5px;
    background: var(--bg);
    color: inherit;
  }
</style>
