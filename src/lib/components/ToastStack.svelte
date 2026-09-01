<script lang="ts">
  // The toast stack in the bottom-right corner: the main window and the detached chat
  // window each mount one (the toast store belongs to each window).
  // lift: a window that has to clear the bottom status bar passes
  // var(--statusbar-h); a window without one (the detached chat window, the welcome
  // page) uses the default 0. The 16px baseline is collected here, so the caller does
  // not do the addition.
  import { toast } from "$lib/stores/toast.svelte";

  let { lift = "0px" }: { lift?: string } = $props();
</script>

{#if toast.items.length}
  <div class="toast-stack" style:bottom="calc(16px + {lift})">
    {#each toast.items as t (t.id)}
      <div class="toast-item" class:error={t.level === "error"} role="status">
        {t.count > 1 ? `${t.message} ×${t.count}` : t.message}
        <button onclick={() => toast.dismiss(t.id)}>✕</button>
      </div>
    {/each}
  </div>
{/if}

<style>
  .toast-stack {
    position: fixed;
    right: 16px;
    display: flex;
    flex-direction: column;
    gap: 6px;
    z-index: 300;
    max-width: min(360px, 70vw);
  }
  .toast-item {
    background: var(--bg-panel);
    border: 1px solid var(--border);
    border-left: 2px solid var(--accent);
    border-radius: 6px;
    padding: 8px 12px;
    font-size: var(--fs-sm);
    display: flex;
    gap: 10px;
    align-items: baseline;
    box-shadow: var(--shadow);
    word-break: break-all;
  }
  .toast-item.error {
    border-left-color: var(--danger);
  }
  .toast-item button {
    color: var(--fg-faint);
    flex-shrink: 0;
  }
  .toast-item button:hover {
    color: var(--fg);
  }
</style>
