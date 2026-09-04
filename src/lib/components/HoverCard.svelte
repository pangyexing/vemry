<!-- The preview card of a block citation in the chat: fixed, above everything but the
  menus, pointer-transparent, clamped inside the viewport after it has measured itself
  (the same landing rule as the context menu). Text only — never {@html}. -->
<script module lang="ts">
  export interface HoverCardData {
    x: number;
    y: number;
    title: string;
    text: string;
    missing?: boolean;
  }
</script>

<script lang="ts">
  import { usableRight } from "$lib/viewport";
  let { card }: { card: HoverCardData | null } = $props();
  let el = $state<HTMLElement | null>(null);
  let pos = $state<{ x: number; y: number } | null>(null);

  $effect(() => {
    const c = card;
    pos = c ? { x: c.x, y: c.y } : null;
    if (!c) return;
    queueMicrotask(() => {
      if (!el || card !== c) return;
      const r = el.getBoundingClientRect();
      pos = {
        x: Math.min(c.x, Math.max(4, usableRight() - r.width - 4)),
        y: c.y + r.height > window.innerHeight - 4 ? Math.max(4, c.y - r.height - 28) : c.y,
      };
    });
  });
</script>

{#if card && pos}
  <div class="hover-card" class:missing={card.missing} bind:this={el} style:left={`${pos.x}px`} style:top={`${pos.y}px`}>
    <div class="hc-title">{card.title}</div>
    <div class="hc-text">{card.text}</div>
  </div>
{/if}

<style>
  .hover-card {
    position: fixed;
    z-index: 95;
    max-width: 360px;
    padding: 8px 10px;
    background: var(--bg-panel);
    border: 1px solid var(--border);
    border-radius: 8px;
    box-shadow: var(--shadow);
    font-size: var(--fs-sm);
    color: var(--fg);
    pointer-events: none;
  }
  .hc-title {
    font-size: var(--fs-xs);
    color: var(--fg-faint);
    margin-bottom: 4px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .hc-text {
    white-space: pre-wrap;
    word-break: break-word;
    max-height: 160px;
    overflow: hidden;
  }
  .missing .hc-text {
    color: var(--fg-faint);
  }
</style>
