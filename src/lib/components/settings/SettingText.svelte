<script lang="ts">
  import { getContext } from "svelte";
  import { isImeKeydown } from "$lib/ime";
  import type { RowContext } from "./context";

  // A text setting: typed freely, written when the field loses the focus or on Enter
  // (a textarea only on the focus leaving). While the field is not being edited it
  // follows the stored value, so a change made elsewhere shows up here.
  let {
    value,
    commit,
    placeholder = "",
    type = "text",
    list,
    mono = false,
    rows = 0,
    width,
  }: {
    value: string;
    /** Called with the typed text when it differs from the stored value. */
    commit: (v: string) => void;
    placeholder?: string;
    type?: "text" | "password";
    list?: string;
    mono?: boolean;
    /** > 0 makes it a textarea. */
    rows?: number;
    /** A fixed width for a short field that sits at the row's end. */
    width?: string;
  } = $props();

  const row = getContext<RowContext | undefined>("setting-row");
  let draft = $state("");
  let editing = $state(false);
  $effect(() => {
    if (!editing) draft = value;
  });
  function done() {
    editing = false;
    if (draft !== value) commit(draft);
  }
</script>

{#if rows > 0}
  <textarea
    id={row?.ctlId}
    class:mono
    {rows}
    {placeholder}
    spellcheck="false"
    bind:value={draft}
    onfocus={() => (editing = true)}
    onblur={done}
  ></textarea>
{:else}
  <input
    id={row?.ctlId}
    class:mono
    {type}
    {list}
    {placeholder}
    spellcheck="false"
    style:width
    bind:value={draft}
    onfocus={() => (editing = true)}
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
