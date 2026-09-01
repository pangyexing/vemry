<script lang="ts">
  // The root component of the detached AI chat window (label `agent`): the same AgentPanel
  // (session bar + page card + conversation body) mounted in a native window that can be
  // dragged out of the main one.
  //
  // This is the second webview: its stores are not shared with the main window, the source
  // of truth for the conversation state is Rust (the session jsonl and the audit log), and
  // the two windows each project it. Startup runs four things concurrently: the settings /
  // taking over the open vault (the lists plus the vault event subscription, without
  // touching editor state) / the browser windows' snapshot / taking the handoff package
  // (draft, chips, level plus the main window's current note), after which the timeline is
  // rebuilt from the current session.
  // The close handshake: the native ✕ or ⌘W reaches Rust first, Rust asks back
  // (close-requested), and here the running turn is cancelled, the draft handed back and
  // the window really closed; the main window then takes over from the jsonl.
  // "Open the note / insert / pin / a command" goes back through host.ts to run in the main
  // window (the editor lives only there).
  import { t } from "$lib/i18n/i18n.svelte";
  import { onMount, tick } from "svelte";
  import {
    api,
    onAgentWindowCloseRequested,
    onAgentWindowCommand,
    onAgentWindowNote,
    onAgentWindowSelection,
    onBrowserEvent,
    type AgentWindowCommand,
  } from "$lib/api";
  import { agent } from "$lib/stores/agent.svelte";
  import { vault } from "$lib/stores/vault.svelte";
  import { browser } from "$lib/stores/browser.svelte";
  import { prefs } from "$lib/stores/prefs.svelte";
  import { toast } from "$lib/stores/toast.svelte";
  import AgentPanel from "$lib/components/AgentPanel.svelte";
  import PermissionDialog from "$lib/components/PermissionDialog.svelte";
  import ContextMenu from "$lib/components/ContextMenu.svelte";
  import ToastStack from "$lib/components/ToastStack.svelte";

  let ready = $state(false);
  const err = (e: unknown) => toast.push(String(e), "error");
  /** A live push already arrived during boot: the handoff package's snapshot is older, so it must not overwrite it. */
  let notePushed = false;
  let selPushed = false;

  onMount(() => {
    const subs = [
      onAgentWindowNote((p) => {
        notePushed = true;
        agent.hostNote = p;
      }),
      onAgentWindowSelection((s) => {
        selPushed = true;
        agent.hostSelection = s;
      }),
      onAgentWindowCloseRequested(() => void closeWindow()),
      onAgentWindowCommand(handleCommand),
      onBrowserEvent((e) => browser.handle(e)),
    ];
    void boot();
    return () => {
      for (const s of subs) void s.then((un) => un());
    };
  });

  /** Commands from the main window to this one. Live events and the queued commands
   *  take_handoff brings back (an emit before the webview mounts is dropped, so Rust queues
   *  them) go through the same handling. */
  function handleCommand(c: AgentWindowCommand) {
    if (c.kind === "new-session") void agent.newSession().catch(err);
    else if (c.kind === "send")
      void agent
        .send(c.text, c.currentNote)
        .then(() => vault.refreshNotes())
        .catch(err);
    else if (c.kind === "attach-page") {
      // The toolbar's "summon the AI with this page": with the window open the handoff
      // package cannot reach it, so the extracted page is attached through this channel;
      // a failure is reported in this window (where the user is looking)
      if (c.page) agent.attachPage(c.page);
      else toast.push(c.error ?? t("app.pageExtractFailed"), "error");
    } else if (c.kind === "memo-result") {
      // The receipt of a `#` quick memory: when the main window's write fails the input is given back to the user (a success is reported in this window too)
      if (c.ok) toast.push(t("app.memoRecorded"));
      else {
        toast.push(c.error ?? t("app.memoFailed"), "error");
        if (agent.pendingMemoDraft != null) agent.draft = agent.pendingMemoDraft;
      }
      agent.pendingMemoDraft = null;
    }
  }

  async function boot() {
    try {
      // The four are independent of each other (settings / the vault's lists / the browser
      // snapshot / the handoff package), and running them in series would be a blank window
      // seven IPC round trips long; concurrently it only waits for the slowest. The session
      // timeline still comes after the handoff package — the draft handed back has to land
      // in the store first.
      const [, , , h] = await Promise.all([
        prefs.refresh(),
        vault.attachCurrent(),
        browser.refresh(),
        api.agentWindowTakeHandoff(),
      ]);
      if (!notePushed) agent.hostNote = h.current_note;
      if (!selPushed) agent.hostSelection = h.selection;
      if (h.handoff) agent.applyHandoff(h.handoff);
      // The commands queued before mounting (the toolbar's attach-page from summoning the AI, say) are handled in order
      for (const c of h.queued) handleCommand(c);
      await agent.loadCurrent();
    } catch (e) {
      err(e);
    }
    ready = true;
    await tick();
    agent.composer?.focus();
  }

  let closing = false;
  /** Answer Rust's close handshake (the native ✕, ⌘W and the main window's "back to the
   *  main window" share this entry point). A running turn is cancelled first — no
   *  background turn is left with nobody to receive its events. */
  async function closeWindow() {
    if (closing) return;
    closing = true;
    try {
      if (agent.running) await agent.cancel();
      await api.agentWindowClose(agent.handoff(), false);
    } catch (e) {
      closing = false;
      err(e);
    }
  }

  function onkeydown(e: KeyboardEvent) {
    if (!(e.metaKey || e.ctrlKey) || e.shiftKey || e.altKey) return;
    const key = e.key.toLowerCase();
    if (key === "j") {
      // ⌘J inside the window only flashes (the hint is suppressed by isAgentWindow): this is already the end of "pop it out"
      e.preventDefault();
      agent.composer?.summon();
    } else if (key === "w") {
      // Off mac, ⌘W/Ctrl+W is taken here; on mac the menu's "close window" takes the key
      // first and goes through Rust's close() → the CloseRequested handshake, which ends up
      // in the same place (menu.rs, batch D)
      e.preventDefault();
      void closeWindow();
    }
  }
</script>

<svelte:window {onkeydown} />

<div class="win">
  {#if ready}
    <AgentPanel />
  {/if}
</div>

<ContextMenu />
<PermissionDialog />
<ToastStack />

<style>
  .win {
    height: 100vh;
    display: flex;
    flex-direction: column;
    background: var(--bg);
    color: var(--fg);
    overflow: hidden;
  }
</style>
