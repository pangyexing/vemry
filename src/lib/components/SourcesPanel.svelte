<script lang="ts">
  // The session's sources workbench: every web source the session cited (the union Rust
  // settled turn by turn), numbered in table order. Only the exception is marked —
  // unverified (prose only, no search behind it); a verified or clipped source is the
  // normal case and carries no badge. The three actions a source takes — open, clip or
  // open the clip, copy the link — show on the row's hover or focus. The same data both windows
  // rebuild from the jsonl, so the list is identical wherever the conversation is shown.
  import { agent } from "$lib/stores/agent.svelte";
  import { clips } from "$lib/stores/clips.svelte";
  import { toast } from "$lib/stores/toast.svelte";
  import { t } from "$lib/i18n/i18n.svelte";
  import { domainOf } from "$lib/markdown";
  import { copyText } from "$lib/copytext";
  import { clipSource, openCitation, openClipped } from "$lib/sources";

  let { onClose }: { onClose: () => void } = $props();

  async function copyLink(url: string) {
    try {
      await copyText(url);
      toast.push(t("agentPanel.linkCopied"));
    } catch (e) {
      toast.push(String(e), "error");
    }
  }
</script>

<div class="src-panel">
  <div class="src-head">
    <span class="src-title">{t("agentPanel.sources", { n: agent.sourceTable.length })}</span>
    <button class="src-close" aria-label={t("agentPanel.closeSources")} onclick={onClose}>✕</button>
  </div>
  {#if agent.sourceTable.length === 0}
    <div class="src-empty">{t("agentPanel.noSources")}</div>
  {:else}
    {#each agent.sourceTable as s, n (s.url)}
      {@const clipped = clips.pathFor(s.url)}
      <div class="src-row">
        <button class="src-main" title={s.url} onclick={(e) => openCitation(s.url, e.metaKey || e.ctrlKey)}>
          <span class="src-n">{n + 1}</span>
          <span class="src-t">{s.title || domainOf(s.url)}</span>
          <span class="src-domain">{domainOf(s.url)}</span>
          {#if !clipped && !s.verified}
            <span class="src-badge" title={t("chat.unverifiedHint")}>{t("chat.unverified")}</span>
          {/if}
        </button>
        {#if clipped}
          <button class="src-act" title={t("reader.alreadyClippedHint")} onclick={() => openClipped(s.url)}>{t("reader.alreadyClipped")}</button>
        {:else}
          <button class="src-act" title={t("chat.clipSourceHint")} onclick={() => void clipSource(s.url, s.title)}>{t("reader.clip")}</button>
        {/if}
        <button class="src-act" title={t("agentPanel.copyLink")} onclick={() => void copyLink(s.url)}>⧉</button>
      </div>
    {/each}
  {/if}
</div>

<style>
  .src-panel {
    display: flex;
    flex-direction: column;
    gap: 1px;
  }
  .src-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 4px 8px 6px;
    font-size: var(--fs-xs);
    font-weight: 600;
    color: var(--fg-muted);
  }
  .src-close {
    color: var(--fg-faint);
    font-size: var(--fs-xs);
    padding: 0 4px;
  }
  .src-close:hover {
    color: var(--fg);
  }
  .src-empty {
    font-size: var(--fs-xs);
    color: var(--fg-muted);
    padding: 8px;
  }
  .src-row {
    display: flex;
    align-items: center;
    gap: 2px;
    border-radius: 6px;
  }
  .src-row:hover {
    background: var(--bg-hover);
  }
  .src-main {
    flex: 1;
    min-width: 0;
    display: flex;
    align-items: baseline;
    gap: 6px;
    text-align: left;
    padding: 5px 8px;
    font-size: var(--fs-xs);
  }
  .src-n {
    flex-shrink: 0;
    color: var(--fg-muted);
    font-variant-numeric: tabular-nums;
    min-width: 1.4em;
  }
  .src-t {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    color: var(--fg);
  }
  .src-domain {
    flex-shrink: 0;
    color: var(--fg-muted);
  }
  .src-badge {
    flex-shrink: 0;
    font-size: 11px;
    line-height: 1;
    padding: 2px 5px;
    border-radius: 3px;
    border: 1px solid currentColor;
    color: var(--warning);
  }
  .src-act {
    flex-shrink: 0;
    font-size: var(--fs-xs);
    color: var(--fg-muted);
    padding: 3px 6px;
    border-radius: 4px;
    visibility: hidden;
  }
  .src-row:hover .src-act,
  .src-row:focus-within .src-act {
    visibility: visible;
  }
  .src-act:hover {
    color: var(--fg);
    background: var(--bg);
  }
</style>
