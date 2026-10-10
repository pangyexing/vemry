<script lang="ts">
  import type { SettingsUpdate } from "$lib/api";
  import { isImeKeydown } from "$lib/ime";
  import { rowControl } from "./context";
  import { form } from "./form.svelte";

  // A text setting: typed freely, written when the field loses the focus or on Enter
  // (a textarea only on the focus leaving), and its row then says "Saved". While the
  // field is not being edited it follows the stored value, so a change made elsewhere
  // shows up here. A field unmounted while it still holds the focus (the page switched
  // by a click, which takes no focus on macOS WebKit, so no blur ever comes) writes then.
  let {
    value,
    patch,
    placeholder = "",
    type = "text",
    list,
    mono = false,
    rows = 0,
    width,
    label,
  }: {
    value: string;
    /** The write for the typed text, made when it differs from the stored value. */
    patch: (v: string) => SettingsUpdate;
    placeholder?: string;
    type?: "text" | "password";
    list?: string;
    mono?: boolean;
    /** > 0 makes it a textarea. */
    rows?: number;
    /** A fixed width for a short field that sits at the row's end. */
    width?: string;
    /** For one of several fields in a row (a provider's card): the field names itself
     *  rather than taking the row's control id, which only one control can hold. */
    label?: string;
  } = $props();

  let draft = $state("");
  let editing = $state(false);
  $effect(() => {
    if (!editing) draft = value;
  });
  /** The row the field belongs to, read when it takes the focus (it is still in the page then). */
  let row: string | undefined;
  function begin(el: HTMLElement) {
    editing = true;
    row = el.closest<HTMLElement>("[data-setting]")?.dataset.setting;
  }
  function done() {
    editing = false;
    if (draft === value) return;
    void form.set(patch(draft), row, true);
  }
</script>

{#if rows > 0}
  <textarea
    {@attach !label && rowControl}
    aria-label={label}
    class:mono
    {rows}
    {placeholder}
    spellcheck="false"
    bind:value={draft}
    onfocus={(e) => begin(e.currentTarget)}
    onblur={done}
  ></textarea>
{:else}
  <input
    {@attach !label && rowControl}
    aria-label={label}
    class:mono
    {type}
    {list}
    {placeholder}
    spellcheck="false"
    style:width
    bind:value={draft}
    onfocus={(e) => begin(e.currentTarget)}
    onblur={done}
    onkeydown={(e) => {
      if (e.key === "Enter" && !isImeKeydown(e)) {
        e.preventDefault();
        done();
        editing = true;
      }
    }}
  />
{/if}

<style>
  .mono {
    font-family: var(--font-mono);
  }
  textarea {
    font-size: var(--fs-xs) !important;
    resize: vertical;
  }
</style>
