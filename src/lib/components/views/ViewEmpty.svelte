<script lang="ts">
  /** The one shape of a view's empty state. "Empty" is two different things, and each view
   *  used to write its own `.empty` rule (whose padding had already drifted into three
   *  variants: 20px 0 / 20px / 16px 8px):
   *   - the vault holds no such content yet → give the syntax that creates it, which is a
   *     lesson
   *   - the filter matched nothing → give a one-click clear, which is a way back
   *  Passing onclear means the latter. Merging the two into one "no data" line would make a
   *  new vault look broken, and leave a filter that matched nothing with no way back.
   *
   *  The graph and the session replay do not use it: the former's empty state is a centred
   *  hint over the canvas and the latter's a placeholder in a narrow list, and neither is a
   *  paragraph in this body flow. */
  import type { Snippet } from "svelte";
  import { t } from "$lib/i18n/i18n.svelte";

  let { onclear, children }: { onclear?: () => void; children: Snippet } = $props();
</script>

<p class="empty">
  {@render children()}
  {#if onclear}<button class="clear" onclick={onclear}>{t("common.clearFilter")}</button>{/if}
</p>

<style>
  .empty {
    color: var(--fg-muted);
    font-size: var(--fs-sm);
    padding: 20px 0;
  }
  /* children is defined by the caller and carries the caller's scope marker, so only :global can match it */
  .empty :global(code) {
    font-family: var(--font-mono);
    font-size: var(--fs-xs);
  }
  .clear {
    color: var(--accent);
    font-size: var(--fs-sm);
    padding: 0 2px;
  }
  .clear:hover {
    text-decoration: underline;
  }
</style>
