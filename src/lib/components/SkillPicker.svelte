<script lang="ts">
  // The composer's `/` candidate overlay: the enabled skills, chosen by name. A choice
  // completes the head token to `/name ` in place; at send that token attaches the skill's
  // text to the message as the method to follow (the `@` chips are the material, this is the
  // how). Positioned like AtPicker (overlaypos.ts), styled by
  // the same `at-*` rules in app.css; the focus stays in the textarea — AgentChat forwards
  // the keys through handleKey and a mouse press keeps the focus with preventDefault.
  import { t } from "$lib/i18n/i18n.svelte";
  import type { SkillEntry } from "$lib/api";
  import { fuzzyFilter } from "$lib/commands/registry";
  import { isImeKeydown } from "$lib/ime";
  import RefIcon from "./RefIcon.svelte";
  import { composerOverlayRect, type OverlayRect } from "./overlaypos";
  import { skillDesc, skillSourceLabel, skillTitle } from "$lib/skills";

  let {
    skills,
    query,
    anchor,
    onPick,
    onClose,
  }: {
    /** The enabled skills — AgentChat's one list, refreshed as this layer opens. */
    skills: SkillEntry[];
    query: string;
    anchor: HTMLElement | null;
    onPick: (name: string) => void;
    onClose: () => void;
  } = $props();

  let pos = $state<OverlayRect | null>(null);
  function measure() {
    pos = composerOverlayRect(anchor);
  }
  $effect(() => {
    void query; // remeasured whenever the filter text changes: auto-growing moves the composer's top edge
    measure();
  });

  const MAX_SHOW = 12;
  /** Matched on the name, the title and description the UI shows, the file's own description
   *  and the triggers (the words a user would type, in any language). */
  const cands = $derived(
    fuzzyFilter(skills, query, (s) => `${s.name} ${skillTitle(s)} ${skillDesc(s)} ${s.description} ${s.triggers.join(" ")}`).slice(0, MAX_SHOW),
  );

  let sel = $state(0);
  $effect(() => {
    void query;
    sel = 0;
  });
  const selIdx = $derived(Math.max(0, Math.min(sel, cands.length - 1)));
  let listEl = $state<HTMLDivElement | null>(null);
  $effect(() => {
    void selIdx;
    const row = listEl?.children[selIdx];
    if (row instanceof HTMLElement) row.scrollIntoView({ block: "nearest" });
  });

  /** Forwarded in from AgentChat's composer onkeydown; true = consumed. Enter and Esc pass
   *  the IME guard: confirming or cancelling an input-method candidate is neither. */
  export function handleKey(e: KeyboardEvent): boolean {
    if (isImeKeydown(e)) return false;
    if (e.key === "ArrowDown") {
      e.preventDefault();
      sel = Math.min(selIdx + 1, cands.length - 1);
      return true;
    }
    if (e.key === "ArrowUp") {
      e.preventDefault();
      sel = Math.max(selIdx - 1, 0);
      return true;
    }
    if (e.key === "Escape") {
      e.preventDefault();
      e.stopPropagation();
      onClose();
      return true;
    }
    if ((e.key === "Enter" || e.key === "Tab") && cands.length) {
      e.preventDefault();
      onPick(cands[selIdx].name);
      return true;
    }
    if (e.key === "Enter") {
      onClose(); // with no candidate, the /token goes out with the message as written
      return false;
    }
    return false;
  }
</script>

<svelte:window onresize={measure} />

<!-- svelte-ignore a11y_no_static_element_interactions -->
<div
  class="at-picker"
  style={pos ? `left:${pos.left}px; width:${pos.width}px; bottom:${pos.bottom}px; max-height:${pos.maxH}px` : "visibility:hidden"}
  onmousedown={(e) => e.preventDefault()}
>
  <div class="at-head">
    <span>{query ? t("skillPicker.headingWithQuery", { query }) : t("skillPicker.heading")}</span>
    <span class="at-keys">{t("atPicker.keys")}</span>
  </div>
  <div class="at-list" bind:this={listEl}>
    {#if cands.length === 0}
      <div class="at-empty">{t("skillPicker.empty")}</div>
    {/if}
    {#each cands as s, i (s.name)}
      <button class="at-row" class:sel={i === selIdx} title={skillDesc(s)} onmouseenter={() => (sel = i)} onclick={() => onPick(s.name)}>
        <span class="at-icon"><RefIcon kind="skill" size={14} /></span>
        <span class="at-title">/{s.name}</span>
        <span class="at-sub">{skillTitle(s) !== s.name ? `${skillTitle(s)} · ` : ""}{skillDesc(s)}</span>
        {#if s.source !== "builtin"}<span class="at-sub tag">{skillSourceLabel(s)}</span>{/if}
      </button>
    {/each}
  </div>
</div>
