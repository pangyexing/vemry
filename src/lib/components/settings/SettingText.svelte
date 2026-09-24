<script lang="ts">
  import type { SettingsUpdate } from "$lib/api";
  import { isImeKeydown } from "$lib/ime";
  import { rowControl } from "./context";
  import { form } from "./form.svelte";

  // A text setting: typed freely, written when the field loses the focus or on Enter
  // (a textarea only on the focus leaving), and its row then says "Saved". While the
  // field is not being edited it follows the stored value, so a change made elsewhere
  // shows up here.
  let {
    value,
    patch,
    placeholder = "",
    type = "text",
    list,
    mono = false,
    rows = 0,
    width,
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
  } = $props();

  let draft = $state("");
  let editing = $state(false);
  $effect(() => {
    if (!editing) draft = value;
  });
  function done(el: HTMLElement) {
    editing = false;
    if (draft === value) return;
    const row = el.closest<HTMLElement>("[data-setting]")?.dataset.setting;
    void form.set(patch(draft), row, true);
  }
</script>

{#if rows > 0}
  <textarea
    {@attach rowControl}
    class:mono
    {rows}
    {placeholder}
    spellcheck="false"
    bind:value={draft}
    onfocus={() => (editing = true)}
    onblur={(e) => done(e.currentTarget)}
  ></textarea>
{:else}
  <input
    {@attach rowControl}
    class:mono
    {type}
    {list}
    {placeholder}
    spellcheck="false"
    style:width
    bind:value={draft}
    onfocus={() => (editing = true)}
    onblur={(e) => done(e.currentTarget)}
    onkeydown={(e) => {
      if (e.key === "Enter" && !isImeKeydown(e)) {
        e.preventDefault();
        done(e.currentTarget);
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
