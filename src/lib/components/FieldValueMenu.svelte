<!-- The one implementation of the inline-field value menu, raised by a click on a
  field chip that is not a date (dates go to TaskDateMenu): a free input, the candidate
  values (what the block types declare for the key first, then what the vault already
  uses) and "remove field". Enter in the input commits it; a click on a candidate picks
  it; a click outside or Esc cancels without writing. Viewport clamping and the closing
  discipline come from popmenu.ts, shared with TaskDateMenu.
  For a state field (`state` is its vocabulary) the candidates are the steps in their
  declared order, each with its glyph, and the first entry advances to the next step —
  the same gesture whether the chip sits in the editor, an embed or a query row.
  The input takes focus on mount, so a typed value is one keystroke away from the chip. -->
<script lang="ts">
  import { t } from "$lib/i18n/i18n.svelte";
  import { isImeKeydown } from "$lib/ime";
  import { clampToViewport, dismissOn } from "$lib/popmenu";
  import { stateGlyphSvg, stateTone } from "$lib/chips";
  import { stateIndex } from "$lib/fields";

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
    /** The key is a state field: its vocabulary in declared order (fields.ts's stateField) */
    steps?: string[] | null;
    /** null = remove the field; the parent clears the menu's state before writing */
    onpick: (value: string | null) => void;
    /** Cancelled by a click outside or Escape: nothing is written */
    onclose: () => void;
  }
  let { x, y, key, current, values, number = false, steps = null, onpick, onclose }: Props = $props();

  /** Where the current value sits in the vocabulary (-1: not a step), and the step after it. */
  const at = $derived(steps ? stateIndex(steps, current) : -1);
  const next = $derived(steps && at >= 0 && at < steps.length - 1 ? steps[at + 1] : null);

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
      if (e.key !== "Enter" || isImeKeydown(e)) return;
      e.preventDefault();
      commit();
    }}
  />
  {#if steps}
    {#if next}
      <button role="menuitem" class="advance" onclick={() => onpick(next)}>
        <span class="glyph {`tone-${stateTone(at + 1, steps.length)}`}">{@html stateGlyphSvg(at + 1, steps.length)}</span>{t("field.advance", { value: next })}
      </button>
      <div class="menu-sep"></div>
    {/if}
    {#each steps as v, i (v)}
      <button role="menuitem" class:current={i === at} onclick={() => onpick(v)}>
        <span class="glyph {`tone-${stateTone(i, steps.length)}`}">{@html stateGlyphSvg(i, steps.length)}</span>{v}
      </button>
    {/each}
    {#each values.filter((v) => stateIndex(steps!, v) < 0) as v (v)}
      <button role="menuitem" class:current={v === current} onclick={() => onpick(v)}><span class="glyph"></span>{v}</button>
    {/each}
  {:else}
    {#each values as v (v)}
      <button role="menuitem" class:current={v === current} onclick={() => onpick(v)}>{v}</button>
    {/each}
  {/if}
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
  .menu button.advance {
    font-weight: 600;
  }
  /* A step's glyph before its value (chips.ts), in the step's tone */
  .glyph {
    display: inline-block;
    width: 14px;
    height: 14px;
    vertical-align: -2px;
    margin-right: 6px;
  }
  .glyph :global(svg) {
    width: 100%;
    height: 100%;
    display: block;
  }
  .tone-idle {
    color: var(--state-idle);
  }
  .tone-active {
    color: var(--state-active);
  }
  .tone-done {
    color: var(--state-done);
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
