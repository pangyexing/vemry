<script lang="ts">
  // The root component of the quick capture window (label `capture`): a composer on a
  // transparent, always-on-top window that one global shortcut shows from any
  // application. Rust keeps the window alive and hidden between presses
  // (capture_window.rs), so this component mounts once and lives for the session.
  //
  // This is another webview with its own stores. It reads the vault (the notes list for
  // ⌘Enter's picker) but never writes a note: what it lands is relayed to the main window
  // (capture_to_main → capture://action), whose vault.capture chooses between the editor
  // buffer and note_insert — the main window is the only place that can see an unsaved
  // buffer — and the receipt comes back through capture://result. The attachments are the
  // one thing written from here: an image goes to the target note's bucket through
  // attachment_save before the relay, so the text can embed it.
  //
  // capture://shown arrives on every press: the text is focused at once, and the settings
  // and the notes are refreshed behind it (no watcher subscription — a hidden window has no
  // business doing four IPC round trips on every autosave of the main window).
  import { onMount, tick } from "svelte";
  import { api, onCaptureShown } from "$lib/api";
  import { isMac } from "$lib/shortcut";
  import { prefs } from "$lib/stores/prefs.svelte";
  import { vault } from "$lib/stores/vault.svelte";
  import CaptureComposer from "$lib/components/CaptureComposer.svelte";

  let composer = $state<CaptureComposer | null>(null);

  onMount(() => {
    const sub = onCaptureShown(() => void shown());
    void boot();
    return () => void sub.then((un) => un());
  });

  /** The settings (the landing target, the shortcut label) and the vault's notes (the picker), both read by the composer. */
  function refresh() {
    return Promise.all([prefs.refresh(), vault.attachCurrent(false, true).catch(() => undefined)]);
  }

  async function boot() {
    await refresh();
    await tick();
    // Only now can a show be honoured: the listeners are registered and the notes are in — a show that arrived earlier is replayed by Rust
    void api.captureWindowReady().catch(() => undefined);
  }

  function shown() {
    composer?.shown();
    // The notes list and the target may have changed since the last press (a note created, the settings edited); refreshed behind the focus, not before it
    void refresh();
  }

  function onkeydown(e: KeyboardEvent) {
    // Off mac, ⌘W/Ctrl+W is taken here; on mac the menu's "close window" takes the key first and Rust hides the window (menu.rs)
    if (!isMac && (e.metaKey || e.ctrlKey) && !e.shiftKey && !e.altKey && e.key.toLowerCase() === "w") {
      e.preventDefault();
      void api.captureWindowHide().catch(() => undefined);
    }
  }
</script>

<svelte:window {onkeydown} />

<div class="win">
  <CaptureComposer bind:this={composer} />
</div>

<style>
  /* The window is transparent (capture_window.rs): the page shows nothing but the composer's own rounded box, and the system draws the shadow around it */
  :global(html),
  :global(body) {
    background: transparent !important;
    overflow: hidden;
  }
  .win {
    color: var(--fg);
  }
</style>
