<script lang="ts">
  // The notice stack, bottom centre of the window: the editor and the composer end just
  // above it, so it lands where the eye already is — a corner shared with a panel of the
  // same colour was passed over. The surface is inverted (the theme's ink as ground) so it
  // stands out from whatever it lies over. The main window and the detached chat window
  // each mount one (the toast store belongs to each window).
  // lift: a window with a bottom status bar passes var(--statusbar-h); the detached chat
  // window and the welcome page keep the default 0. The 16px baseline is added here.
  // inset: the width a docked browser window covers on the right (a native child window is
  // always above the DOM), so the stack centres in the part of the window that can be seen.
  import { fade, fly } from "svelte/transition";
  import { t } from "$lib/i18n/i18n.svelte";
  import { toast } from "$lib/stores/toast.svelte";
  import { vault } from "$lib/stores/vault.svelte";

  let { lift = "0px", inset = 0 }: { lift?: string; inset?: number } = $props();
  // The vault store's sticky error slot is announced through this stack like every other
  // notice, so there is one place at the bottom of the window to look at, not a second strip
  // in a second place — and it is announced in whichever window the failure happened in,
  // since each window has its own store and mounts its own stack. The slot is cleared once
  // shown: the same text failing twice is said twice, and no later reader mistakes an old
  // failure for the result of its own operation. The welcome page (no vault yet) keeps
  // rendering the slot inline.
  $effect(() => {
    if (vault.root && vault.error) {
      toast.push(vault.error, "error");
      vault.error = null;
    }
  });
  /** Motion is skipped, not slowed, for someone who asked the system for less of it. */
  const reduced = typeof matchMedia === "function" && matchMedia("(prefers-reduced-motion: reduce)").matches;
  const dur = (ms: number) => (reduced ? 0 : ms);
</script>

{#if toast.items.length}
  <div class="toast-stack" style:bottom="calc(16px + {lift})" style:right="{inset}px">
    {#each toast.items as it (it.id)}
      <div
        class="toast-item"
        class:error={it.level === "error"}
        role={it.level === "error" ? "alert" : "status"}
        in:fly={{ y: 10, duration: dur(180) }}
        out:fade={{ duration: dur(120) }}
        onmouseenter={() => toast.hold(it.id)}
        onmouseleave={() => toast.release(it.id)}
      >
        <span class="dot" aria-hidden="true"></span>
        <span class="text">{it.count > 1 ? `${it.message} ×${it.count}` : it.message}</span>
        <button onclick={() => toast.dismiss(it.id)} aria-label={t("common.close")}>✕</button>
      </div>
    {/each}
  </div>
{/if}

<style>
  .toast-stack {
    position: fixed;
    left: 0;
    padding: 0 16px;
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 8px;
    z-index: 300;
    /* The stack spans the width so its items can centre; only the items take the pointer */
    pointer-events: none;
  }
  .toast-item {
    pointer-events: auto;
    max-width: min(560px, 100%);
    display: flex;
    align-items: baseline;
    gap: 10px;
    padding: 10px 12px 10px 14px;
    border-radius: 8px;
    background: var(--fg);
    color: var(--bg);
    font-size: var(--fs-md);
    line-height: 1.45;
    box-shadow: var(--shadow);
    /* A vault-relative path breaks where it must, not letter by letter */
    overflow-wrap: anywhere;
  }
  .dot {
    flex: none;
    align-self: center;
    width: 7px;
    height: 7px;
    border-radius: 50%;
    background: var(--accent-inverse);
  }
  .toast-item.error .dot {
    background: var(--danger-inverse);
  }
  .text {
    flex: 1 1 auto;
    min-width: 0;
  }
  .toast-item button {
    flex: none;
    color: var(--bg);
    opacity: 0.55;
    font-size: var(--fs-sm);
    padding: 0 2px;
  }
  .toast-item button:hover {
    opacity: 1;
  }
</style>
