<script lang="ts">
  // The root component of the detached browser window's chrome bar (webview label
  // `browserbar`): the same BrowserPanel chrome the main window's panel column uses,
  // mounted in a 72px child webview above the tabs of `browserwin`. A tab is a remote
  // page with zero IPC and no chrome can get in — so the bar is a webview of our own
  // beside it (the same approach as the agent window).
  //
  // Its stores are not shared with the main window: the tab list mirrors Rust through
  // browser://event, the vault is taken over for the destination picker's note list
  // alone (no event subscription — every autosave in the main window would otherwise
  // cost this bar four vault-wide IPCs), and the pinned destination (clipTarget) is a
  // read-only mirror the main window broadcasts to.
  import { onMount } from "svelte";
  import { onBrowserEvent } from "$lib/api";
  import { vault } from "$lib/stores/vault.svelte";
  import { browser } from "$lib/stores/browser.svelte";
  import BrowserPanel from "$lib/components/BrowserPanel.svelte";

  onMount(() => {
    const sub = onBrowserEvent((e) => browser.handle(e));
    void Promise.all([vault.attachCurrent(false, false), browser.refresh()]).catch(() => {});
    return () => void sub.then((un) => un());
  });
</script>

<BrowserPanel mode="bar" />
