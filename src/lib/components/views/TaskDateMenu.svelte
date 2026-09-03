<!-- The one implementation of the task reschedule menu (shared by the Agenda chip, the
  kanban chip and a kanban drop): today / tomorrow / next week / a chosen date, plus
  "remove the date" when current is set. Viewport clamping and closing on a click outside
  or Esc come from popmenu.ts, shared with FieldValueMenu — the ⚠ about never listening on
  click lives there, and this menu is the one that reproduces it (a kanban drop). -->
<script lang="ts">
  import { t } from "$lib/i18n/i18n.svelte";
  import { clampToViewport, dismissOn } from "$lib/popmenu";
  import { dateFieldLabel, type DateField, dateIconSvg, localDate } from "$lib/views/tasks";

  interface Props {
    /** The fixed positioning anchor: where the chip was clicked, or where the drop landed */
    x: number;
    y: number;
    /** Decides the title's icon and its default title (DATE_FIELDS[field].label) */
    field: DateField;
    /** Overrides the title: the kanban drop usage passes "schedule it for which day" */
    title?: string;
    /** The field's current value: only with one is "remove the date" shown, and it is the date input's default */
    current?: string;
    /** null = remove the date; the parent clears the menu's state before writing to disk */
    onpick: (date: string | null) => void;
    /** Cancelled by a click outside or Escape — in the kanban drop usage, cancelling cancels the whole drop and nothing is written */
    onclose: () => void;
  }
  let { x, y, field, title, current, onpick, onclose }: Props = $props();

  let el = $state<HTMLElement | null>(null);
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
</script>

<svelte:window onpointerdowncapture={dismiss.onpointerdowncapture} onkeydowncapture={dismiss.onkeydowncapture} />

<div class="menu" bind:this={el} style:left="{clamped.x}px" style:top="{clamped.y}px" role="menu" tabindex="-1">
  <div class="menu-title">{@html dateIconSvg(field)}{title ?? dateFieldLabel(field)}</div>
  <button role="menuitem" onclick={() => onpick(localDate())}>{t("dateMenu.today")}</button>
  <button role="menuitem" onclick={() => onpick(localDate(1))}>{t("dateMenu.tomorrow")}</button>
  <button role="menuitem" onclick={() => onpick(localDate(7))}>{t("dateMenu.nextWeek")}</button>
  <input
    type="date"
    title={t("dateMenu.pickDate")}
    value={current ?? localDate()}
    onchange={(e) => {
      const v = e.currentTarget.value;
      if (v) onpick(v);
    }}
  />
  {#if current}
    <div class="menu-sep"></div>
    <button role="menuitem" class="danger" onclick={() => onpick(null)}>{t("dateMenu.removeDate")}</button>
  {/if}
</div>

<style>
  .menu {
    position: fixed;
    z-index: 250;
    min-width: 150px;
    background: var(--bg-panel);
    border: 1px solid var(--border);
    border-radius: 8px;
    box-shadow: var(--shadow);
    padding: 4px;
    display: flex;
    flex-direction: column;
  }
  .menu-title {
    display: flex;
    align-items: center;
    gap: 5px;
    font-size: var(--fs-xs);
    color: var(--fg-faint);
    padding: 4px 10px 2px;
  }
  /* dateIconSvg is injected with {@html}, and the svg carries no scope marker, so its size is matched with :global */
  .menu-title :global(svg) {
    width: 0.9em;
    height: 0.9em;
    flex-shrink: 0;
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
  }
  .menu button:hover {
    background: var(--bg-hover);
  }
  .menu button.danger {
    color: var(--danger);
  }
  .menu input[type="date"] {
    margin: 2px 6px 4px;
    padding: 3px 6px;
    font-size: var(--fs-sm);
    border: 1px solid var(--border);
    border-radius: 5px;
    background: var(--bg);
    color: inherit;
  }
</style>
