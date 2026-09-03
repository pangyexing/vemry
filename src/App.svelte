<script lang="ts">
  import { t } from "$lib/i18n/i18n.svelte";
  import { open as openDialog, save as saveDialog, confirm as confirmDialog } from "@tauri-apps/plugin-dialog";
  import { extractOutline } from "$lib/outline";
  import { api, emitClipbarResult, onAgentWindowAction, onAgentWindowClosed, onBrowserEvent, onClipbarAction, onMenuEvent, onReadingEvent, onScriptEvent, type ClipbarResult, type ScriptInfo } from "$lib/api";
  import { clipbarLabelOf } from "$lib/window";
  import { runHostAction } from "$lib/host";
  import { executeCommand, registerCommand, syncScriptCommands } from "$lib/commands/registry";
  import { MEMORY_DRAFT_PROMPT, MEMORY_REVIEW_PROMPT } from "$lib/commands/memoryPrompts";
  import { vault } from "$lib/stores/vault.svelte";
  import { scaffold } from "$lib/scaffold";
  import { agent } from "$lib/stores/agent.svelte";
  import { scripts } from "$lib/stores/scripts.svelte";
  import { types } from "$lib/stores/types.svelte";
  import { browser, revealPanelIfDocked } from "$lib/stores/browser.svelte";
  import { reading } from "$lib/stores/reading.svelte";
  import { toast } from "$lib/stores/toast.svelte";
  import { clipTarget } from "$lib/stores/clipTarget.svelte";
  import { prefs } from "$lib/stores/prefs.svelte";
  import { updater } from "$lib/stores/updater.svelte";
  import { isMac, k } from "$lib/shortcut";
  import Editor from "$lib/components/Editor.svelte";
  import FileTree from "$lib/components/FileTree.svelte";
  import ActivityBar, { ABAR_W } from "$lib/components/ActivityBar.svelte";
  import StatusBar from "$lib/components/StatusBar.svelte";
  import TabBar from "$lib/components/TabBar.svelte";
  import OutlinePanel from "$lib/components/OutlinePanel.svelte";
  import AgentPanel from "$lib/components/AgentPanel.svelte";
  import ToastStack from "$lib/components/ToastStack.svelte";
  import PermissionDialog from "$lib/components/PermissionDialog.svelte";
  import ContextMenu from "$lib/components/ContextMenu.svelte";
  import { contextMenu } from "$lib/stores/contextMenu.svelte";
  import ScriptGrantDialog from "$lib/components/ScriptGrantDialog.svelte";
  import SettingsModal, { type SettingsTab } from "$lib/components/SettingsModal.svelte";
  import CommandPalette, { type PaletteMode } from "$lib/components/CommandPalette.svelte";
  import AgendaView from "$lib/components/views/AgendaView.svelte";
  import KanbanView from "$lib/components/views/KanbanView.svelte";
  import TimelineView from "$lib/components/views/TimelineView.svelte";
  import ClaimsView from "$lib/components/views/ClaimsView.svelte";
  import EvidenceView from "$lib/components/views/EvidenceView.svelte";
  import GraphView from "$lib/components/views/GraphView.svelte";
  import SessionsView from "$lib/components/views/SessionsView.svelte";
  import ReadingView from "$lib/components/views/ReadingView.svelte";

  let paletteMode = $state<PaletteMode | null>(null);

  /** The quote block waiting to land for "clip into a chosen note" (url is there so the images
   *  can be localised once it lands; a projection selection has none). The text is taken before
   *  the pick-note panel is raised — a projection selection collapses once the panel takes the
   *  focus, so it has to be taken first and chosen second. It is not cleared when the panel is
   *  cancelled: pick-note can only be entered from the entry points that assign it, so a stale
   *  value cannot be delivered. */
  let pendingQuote: { text: string; url?: string } | null = null;
  /** The text waiting to land for "a quick memo into a chosen note" (entered with ⌘Enter in the
   *  capture panel). It is mutually exclusive with pendingQuote — both entry points clear the
   *  other before assigning, or a stale value left after one flow's panel was cancelled would be
   *  delivered by the other. */
  let pendingCapture: string | null = null;

  function pickNoteForQuote(text: string, url?: string) {
    pendingCapture = null;
    pendingQuote = { text, url };
    paletteMode = "pick-note";
  }
  function pickNoteForCapture(text: string) {
    pendingQuote = null;
    pendingCapture = text;
    paletteMode = "pick-note";
  }

  /** The pick-note callback: it branches by the entry point the panel was opened from, and
   *  heading is the destination section chosen in the panel's second stage (null or absent = the
   *  end of the note). A quick memo only delivers and pins nothing; a clip lands and pins the
   *  target (the section included) — after which "selection → note" goes straight there with one
   *  click, until ✕ releases it.
   *  The landing rules live in clipTarget.deliver / vault.capture. */
  function deliverPicked(path: string, heading: string | null = null) {
    if (pendingCapture !== null) {
      const text = pendingCapture;
      pendingCapture = null;
      vault.capture(text, path, heading).then(
        // When a missing section falls back to the end of the file, insertBlock has already said so — do not claim "› section" again (as in clipTarget.deliver)
        (r) => { if (!(heading && r.landed === "tail")) toast.push(t("app.recordedIn", { path: `${path}${heading ? ` › ${heading}` : ""}` })); },
        (e) => toast.push(String(e), "error"),
      );
      return;
    }
    const quote = pendingQuote;
    pendingQuote = null;
    if (!quote) return;
    clipTarget.path = path;
    clipTarget.heading = heading;
    void clipTarget.deliver(path, quote.text, quote.url, heading).then((ok) => {
      // With a url it came from a browser-window selection (a projection selection has none and does not count as a browsing session's landing)
      if (ok && quote.url) recordClipLanding(path, heading);
    });
  }
  // The clip destination's vault-switch cleanup and restore live inside the clipTarget store (persisted per vault)
  let settingsOpen = $state(false);
  /** Which category page the settings open on: the ordinary entry lands on "General", and check-updates lands straight on "Updates" */
  let settingsTab = $state<SettingsTab>("general");

  // ---- The sidebars: dragging a sash, double-clicking to reset, swapping sides (swapSides) ----
  // The sum of the three columns' minimum widths plus the activity bar (140+300+280+44=764) goes
  // with tauri.conf.json's minWidth=764 and Rust's MIN_MAIN_VISIBLE=804, and a change means
  // looking at all three
  // (the coupling the old CSS grid comment described, now collected here)
  const TREE_MIN = 140;
  const CENTER_MIN = 300;
  const PANEL_MIN = 280;
  // The sash being dragged (the highlighted handle line plus the full-screen col-resize cursor
  // overlay) has its state in the vault store (vault.resizing): the UI-state persistence effect
  // uses it to pause the synchronous localStorage writes while dragging
  /** The column template: a column the user has dragged takes a fixed upper bound (when space
   *  runs short the grid protects the 300 content area first and squeezes the sidebars: until
   *  the editor reaches its minimum, a sidebar's width does not move at all); one never dragged
   *  keeps the default elastic formula. An fr column only takes what is left once the minmax
   *  columns are fed, so the content area's floor has to be written into its own min. Swapping
   *  sides = reversing the column order plus the children's order (see .layout.swapped). */
  const gridCols = $derived.by(() => {
    const tree = `minmax(${TREE_MIN}px, ${vault.leftWidth ?? 220}px)`;
    const panel = vault.rightWidth != null
      ? `minmax(${PANEL_MIN}px, ${vault.rightWidth}px)`
      : `minmax(${PANEL_MIN}px, min(30%, 560px))`;
    const cols: string[] = [];
    if (vault.leftOpen) cols.push(tree);
    cols.push(`minmax(${CENTER_MIN}px, 1fr)`);
    if (vault.rightOpen) cols.push(panel);
    if (vault.swapSides) cols.reverse();
    return cols.join(" ");
  });
  function startResize(e: PointerEvent, which: "tree" | "panel") {
    if (e.button !== 0) return;
    e.preventDefault(); // suppress the selection and focus side effects of the compatibility mousedown
    const sash = e.currentTarget as HTMLElement;
    const startX = e.clientX;
    const startW = sash.parentElement!.getBoundingClientRect().width;
    // The drag direction depends on which side that column is docked on right now: on the left, dragging right widens it
    const onLeft = which === "tree" ? !vault.swapSides : vault.swapSides;
    const min = which === "tree" ? TREE_MIN : PANEL_MIN;
    // The upper bound leaves the minimum widths for the content area, the opposite sidebar and
    // the activity bar; the room a docked browser window takes (padding-right) is deducted from
    // the available width too
    const otherMin = which === "tree" ? (vault.rightOpen ? PANEL_MIN : 0) : (vault.leftOpen ? TREE_MIN : 0);
    const max = Math.max(min, window.innerWidth - Math.max(browser.overlayInset, 0) - ABAR_W - CENTER_MIN - otherMin);
    vault.resizing = which;
    sash.setPointerCapture(e.pointerId);
    const move = (ev: PointerEvent) => {
      const w = Math.round(Math.min(max, Math.max(min, startW + (onLeft ? 1 : -1) * (ev.clientX - startX))));
      if (which === "tree") vault.leftWidth = w;
      else vault.rightWidth = w;
    };
    const up = () => {
      vault.resizing = null;
      sash.removeEventListener("pointermove", move);
      sash.removeEventListener("pointerup", up);
      sash.removeEventListener("pointercancel", up);
    };
    sash.addEventListener("pointermove", move);
    sash.addEventListener("pointerup", up);
    sash.addEventListener("pointercancel", up);
  }

  vault.loadRecent();
  // The settings are fetched once at startup (the prefs mirror brings updater_auto_check back
  // with it): one silent update check — only when the user turned it on in the settings (off by
  // default, offline first)
  void prefs.refresh().then((s) => {
    if (s?.updater_auto_check) void updater.check(false);
  });

  // ---- user scripts: palette sync + toasts + blocking dialogs ----
  function applyScripts(infos: ScriptInfo[]) {
    scripts.infos = infos;
    syncScriptCommands(infos, (script, command) => {
      api.scriptCommandRun(script, command, vault.currentPath).catch((e) => toast.push(String(e), "error"));
    });
  }
  /** commands_changed pushes since startup — the fallback pull below must
   *  never overwrite a fresher push with a pre-reload snapshot. */
  let pushSeq = 0;
  const scriptListener = onScriptEvent((e) => {
    if (e.type === "commands_changed") {
      pushSeq += 1;
      applyScripts(e.scripts);
    } else if (e.type === "toast") toast.push(e.message, e.level);
    else if (e.type === "error") toast.push(t("app.scriptError", { script: e.script, message: e.message }), "error");
    else if (e.type === "grant_request") scripts.pendingGrant = e;
    else if (e.type === "prompt_request") scripts.pendingPrompt = e;
  });
  // Restoring the vault has to wait for the listen registration: the script host loads the
  // scripts inside vault_open, and a grant_request or prompt_request arriving before the
  // listener would be lost forever (the dialog never appears and the script thread waits until
  // it times out)
  void scriptListener.catch(() => {}).finally(() => void vault.restoreLast());
  // The browser's state pushes: a window is only ever created by this frontend's commands, so
  // none exists at startup and there is no early-event race; besides listen, a snapshot is
  // fetched once as a backstop (a window may already be open after a hot reload).
  // Batch D ③: opening the first window clears the landing record (a new browsing session), and
  // closing the last returns to the landing point (Rust has already refocused the main window,
  // and this adds the navigation).
  void onBrowserEvent((e) => {
    if (e.type === "opened" && browser.windows.length === 0) lastClipLanding = null;
    browser.handle(e);
    if (e.type === "closed" && browser.windows.length === 0 && lastClipLanding) {
      const { path, heading } = lastClipLanding;
      lastClipLanding = null;
      void openLanding(path, heading).catch((err) => toast.push(String(err), "error"));
    }
  }).then(() => void browser.refresh());
  // The action relay of the clip toolbar (at the bottom of a browser window, batch D): the clip
  // flow, the pinned destination and the editor live in the main window alone, so they are run
  // here on its behalf; once a clip action finishes, the receipt is sent back to the toolbar to
  // flash there (it cannot see the main window's toast). Raising the main window for open-note
  // has already been done by Rust's relay (the focus parameter).
  void onClipbarAction(async (a) => {
    if (a.kind === "set-clip-target") {
      clipTarget.path = a.path;
      clipTarget.heading = a.heading;
      return; // the store's persistence effect writes it to disk and broadcasts it back to the toolbar automatically
    }
    if (a.kind === "clear-clip-target") {
      clipTarget.clear();
      return;
    }
    if (a.kind === "open-note") {
      await runHostAction(a).catch((e) => toast.push(String(e), "error"));
      return;
    }
    if (a.kind === "summon-agent") {
      // Summoning the AI from the browser: the detached chat window is opened (or focused) first
      // for immediate feedback, and the current page is then extracted and attached as a page
      // chip through the to_agent channel (with the window open the handoff package cannot reach
      // it; attachPage is a single-page replacement, so summoning again only refreshes it). A
      // failed extraction's error goes into the chat window too.
      if (!vault.root) {
        void emitClipbarResult(clipbarLabelOf(a.label), { ok: false, message: t("app.openVaultFirst") }).catch(() => {});
        return;
      }
      try {
        // Opening the window and extracting the body do not depend on each other (only the
        // attachment at the end needs both): sending them concurrently cuts a stretch off the
        // wait between "press AI" and the page chip appearing
        const [, page] = await Promise.all([
          agent.openWindow(vault.currentPath, vault.editorSelection),
          api.browserPageContext(a.label),
        ]);
        await api.agentWindowToAgent({ kind: "attach-page", page });
      } catch (e) {
        toast.push(String(e), "error");
        // The error goes all three ways: to the chat window (which may be open, where the user is
        // looking), and to the toolbar's flash (when opening the window itself failed there is no
        // chat window, and the main window's toast is hidden behind the browser window — without
        // this receipt what the user sees is "the AI button did nothing")
        void api.agentWindowToAgent({ kind: "attach-page", page: null, error: String(e) }).catch(() => {});
        void emitClipbarResult(clipbarLabelOf(a.label), { ok: false, message: String(e) }).catch(() => {});
      }
      return;
    }
    const r =
      a.kind === "clip-selection"
        ? await clipSelectionFlow(a.label)
        : a.kind === "clip-page"
          ? await clipPageFlow(a.label)
          : await clipLinkFlow(a.label);
    void emitClipbarResult(clipbarLabelOf(a.label), r).catch(() => {});
  });
  // The native menu bar (macOS) → a command: the payload is a registry command id, and everything is a command
  void onMenuEvent((id) => void executeCommand(id));
  // The detached AI chat window: once closed it is taken over (the draft is taken back and the
  // timeline rebuilt from Rust's current session); the actions it wants the main window to run
  // (open a note, insert, pin, a command) are executed here
  void onAgentWindowClosed(() =>
    void agent.reattach().then(() => {
      // The conversation is back in the panel: a flash tells the user where it is; the pop-out hint is not shown again — they have just come from there
      if (vault.rightOpen && vault.rightTab === "agent") agent.composer?.summon({ hint: false });
    }),
  );
  // A `#` quick memory needs a receipt: the chat window cleared its input and is waiting for this ok, and on failure the text has to go back to the user
  void onAgentWindowAction((a) =>
    void runHostAction(a)
      .then(() => {
        if (a.kind === "memo") void api.agentWindowToAgent({ kind: "memo-result", ok: true }).catch(() => {});
      })
      .catch((e) => {
        toast.push(String(e), "error");
        if (a.kind === "memo") void api.agentWindowToAgent({ kind: "memo-result", ok: false, error: String(e) }).catch(() => {});
      }),
  );
  // After a hot reload or an unusual startup the window may already be open: mirror the real state once
  void api.agentWindowIsOpen().then((open) => (agent.windowOpen = open)).catch(() => {});
  // The main window switched notes → push it to the chat window (only while that window is open; the value at the moment it opens travels with the handoff package)
  $effect(() => {
    const path = vault.currentPath;
    if (agent.windowOpen) void api.agentWindowSetNote(path).catch(() => {});
  });
  // The editor's selection → pushed to the chat window (where it is the `@ current selection`
  // candidate). During a drag the snapshot changes on every mousemove, so it is debounced until
  // the hand stops; the value at the moment the window opens travels with the handoff package
  $effect(() => {
    const sel = vault.editorSelection;
    if (!agent.windowOpen) return;
    const t = setTimeout(() => void api.agentWindowSetSelection(sel).catch(() => {}), 120);
    return () => clearTimeout(t);
  });
  // The reading projection (P2): the Reading arm of the url_open funnel sends a signal, and the
  // frontend switches the view and fetches.
  // mainView is switched here rather than in the store — the reading store does not touch vault, which avoids a module cycle
  void onReadingEvent((e) => {
    if (e.type === "open") {
      vault.enterReading();
      void reading.open(e.url);
    }
  });
  // Fetched once as a backstop (the host's commands_changed may have been emitted before listen);
  // should a push have arrived in the meantime, this possibly stale snapshot is discarded
  $effect(() => {
    if (!vault.root) return;
    const seqAtRequest = pushSeq;
    void api.scriptsList().then((infos) => {
      if (pushSeq === seqAtRequest) applyScripts(infos);
    });
  });
  // The block types of the vault just opened (types_changed keeps them fresh afterwards)
  $effect(() => {
    if (!vault.root) return;
    void types.refresh();
  });

  async function pickVault() {
    const dir = await openDialog({ directory: true, title: t("app.pickVaultFolder") });
    if (typeof dir === "string") await vault.openVault(dir);
  }

  /** Create a vault: the save dialog is used as "choose a location and name a new directory" —
   *  the path it returns usually does not exist, the Rust side only accepts one that does not
   *  exist or is empty, and picking a directory with data by mistake is an error rather than
   *  silently becoming "open". */
  async function createVaultFlow() {
    const target = await saveDialog({ title: t("app.newVaultTitle"), defaultPath: t("app.newVaultDefaultName") });
    if (typeof target === "string") await vault.createVault(target);
  }

  // ---- everything is a command ----
  registerCommand({ id: "open-vault", group: "vault", run: pickVault });
  registerCommand({
    id: "create-vault", group: "vault",
    run: createVaultFlow,
  });
  registerCommand({
    id: "switch-vault", group: "vault",
    run: () => { paletteMode = "vaults"; },
  });
  registerCommand({
    id: "new-note", hotkey: k("Mod+N"), group: "note",
    run: () => { paletteMode = "notes"; },
  });
  registerCommand({
    id: "open-note", hotkey: k("Mod+O"), group: "nav",
    run: () => { paletteMode = "notes"; },
  });
  registerCommand({
    id: "search", hotkey: k("Mod+Shift+F"), group: "nav",
    run: () => { paletteMode = "search"; },
  });
  registerCommand({
    id: "command-palette", hotkey: k("Mod+P"), group: "nav",
    run: () => { paletteMode = "commands"; },
  });
  registerCommand({
    id: "quick-capture", descriptionParams: { key: k("Mod+Enter") }, hotkey: k("Mod+Shift+C"), group: "note",
    run: () => { paletteMode = "capture"; },
  });
  /** Layered back: inside the reading projection the projection's page stack is consumed first,
   *  and once it is empty the document history takes over (the projection occupies one stop in
   *  navStack, so it returns to the note it was entered from); with neither, it falls back to the
   *  view it came from — so ⌘⌥← always responds inside the projection and never skips past the
   *  note in front of you to the one before it. */
  function navBack() {
    if (vault.mainView === "reading") {
      if (reading.canGoBack) reading.back();
      else if (vault.canGoBack) void vault.goBack();
      else vault.exitReading();
    } else void vault.goBack();
  }
  function navForward() {
    if (vault.mainView === "reading") {
      if (reading.canGoForward) reading.forward();
      else void vault.goForward();
    } else void vault.goForward();
  }
  registerCommand({
    id: "nav-back", hotkey: k("Mod+Alt+←"), group: "nav",
    run: navBack,
  });
  registerCommand({
    id: "nav-forward", hotkey: k("Mod+Alt+→"), group: "nav",
    run: navForward,
  });
  registerCommand({
    id: "undo-file-op", descriptionParams: { key: k("Mod+Z") }, group: "note",
        run: () => vault.undoFileOp(),
  });
  registerCommand({
    id: "toggle-left", hotkey: k("Mod+B"), group: "view",
    run: () => { vault.leftOpen = !vault.leftOpen; },
  });
  registerCommand({
    id: "toggle-right", hotkey: k("Mod+."), group: "view",
    run: () => { vault.rightOpen = !vault.rightOpen; },
  });
  registerCommand({
    id: "toggle-sidebar-side", group: "view",
        run: () => { vault.swapSides = !vault.swapSides; },
  });
  registerCommand({
    id: "show-agent", group: "research",
    run: () => { vault.rightOpen = true; vault.rightTab = "agent"; },
  });
  registerCommand({
    // One shortcut, one session, shown where you are: with the panel's agent tab visible it sends
    // the cursor into the panel (the input flashes and a hint says "press ⌘J again for a detached
    // window"); only when the cursor is already in the panel's input does another press escalate
    // to opening (or focusing) the detached native chat window — the same session projected into
    // another window, not a second conversation.
    // The giving-way invariant: while that window is open the panel shows only a placeholder
    // (AgentPanel), so exactly one composer is ever visible.
    id: "focus-agent", descriptionParams: { key: k("Mod+J") }, hotkey: k("Mod+J"), group: "research",
        run: () => {
      if (!vault.root) return;
      const panelVisible = !agent.windowOpen && vault.rightOpen && vault.rightTab === "agent";
      const c = agent.composer;
      if (panelVisible && c && !c.isFocused()) { c.summon(); return; }
      void agent.openWindow(vault.currentPath, vault.editorSelection).catch((e) => toast.push(String(e), "error"));
    },
  });
  registerCommand({
    // The command entry point of the composer's "+" button (principle 5: everything is a
    // command): the native file dialog → import into the current note's attachment bucket →
    // chips. When no conversation surface is mounted (the panel is collapsed and no window is
    // open) it is summoned first.
    id: "agent-attach-files", group: "research",
    run: () => {
      if (!vault.root) return;
      const c = agent.composer;
      if (c) { c.attachFiles(); return; }
      void executeCommand("focus-agent");
    },
  });
  registerCommand({
    // Named after the sidebar's "Outline" tab (the panel holds the outline plus the collapsed backlinks and outgoing-links sections)
    id: "show-outline", group: "view",
    run: () => { vault.rightOpen = true; vault.rightTab = "outline"; },
  });
  registerCommand({
    id: "open-url", hotkey: k("Mod+L"), group: "nav",
    run: () => { paletteMode = "url"; },
  });
  registerCommand({
    // The single implementation of "summon it directly": the menu bar's "Research → open the
    // browser" and an empty ⌘L return both come here (Rust's browser_open_last: focus an existing
    // window or reopen the most recent page from the history).
    id: "open-browser", group: "nav",
    run: async () => {
      try {
        const label = await api.browserOpenLast();
        if (!label) {
          toast.push(t("app.noHistory", { key: k("Mod+L") }));
          return;
        }
        await revealPanelIfDocked(label);
      } catch (e) {
        toast.push(String(e), "error");
      }
    },
  });
  registerCommand({
    id: "clear-browser-history", group: "system",
    run: async () => {
      try {
        await api.browserHistoryClear();
        toast.push(t("app.historyCleared"));
      } catch (e) {
        toast.push(String(e), "error");
      }
    },
  });
  registerCommand({
    id: "show-browser", group: "research",
    run: () => { vault.rightOpen = true; vault.rightTab = "agent"; },
  });
  registerCommand({
    // "Everything is a command": the keyboard path beside the ☆ buttons (on the page card and the
    // toolbar), on the same bookmark_toggle semantics as both of them (the state is mirrored
    // through the bookmarks_changed broadcast)
    id: "bookmark-toggle", descriptionParams: { key: k("Mod+L") }, group: "research",
    run: async () => {
      const cur = browser.current;
      if (!cur) {
        toast.push(t("app.noBrowserWindow"));
        return;
      }
      try {
        const on = await api.bookmarkToggle(cur.url, cur.title);
        toast.push(t(on ? "clipBar.bookmarked" : "clipBar.unbookmarked"));
      } catch (e) {
        toast.push(String(e), "error");
      }
    },
  });
  // ---- The three granularities of clipping: a selection → a quote block, a whole page →
  // clips/, a link → a reference entry ----
  // The command bodies are lifted into flows: the main window's commands (with label absent =
  // the window focused last) and the browser toolbar's relay (label = the paired browser window)
  // share one copy; the return value gives the toolbar its flash receipt, while the main window
  // still toasts as before (so there is a trace when the user switches back).
  //
  // Batch D ③, "return to the landing point on closing": during a browsing session (from the
  // first browser window opening to the last one closing), the note the last clip landed in
  // takes the user there once the final window closes — "clip it, ⌘W, and you are back at the
  // landing point".
  // The recording points are collected in the flows below plus deliverPicked (a clip started from
  // the toolbar is relayed here to run too, so there is no second path); the reading projection,
  // an agent pin and a quick memo are not part of a browsing session, do not come through these
  // paths and naturally record nothing. An ordinary variable is enough, since nothing renders
  // from it.
  let lastClipLanding: { path: string; heading: string | null } | null = null;
  function recordClipLanding(path: string | null | undefined, heading: string | null = null) {
    if (path && browser.windows.length > 0) lastClipLanding = { path, heading };
  }
  /** After the last browser window closes, return to the clip's landing point: open the note and,
   *  with a section, scroll to it (the same UTF-16 units as the outline's, rather than
   *  query_blocks' byte offsets). */
  async function openLanding(path: string, heading: string | null) {
    // Refresh the lists before checking existence: the landing point is often a clips/ note
    // created a few hundred milliseconds ago (clipping a whole page and pressing ⌘W at once is
    // this feature's signature flow), the mirror driven by the watcher broadcast has usually not
    // caught up, and checking against a stale list would silently skip the navigation; what is
    // really being checked is "deleted after it landed"
    if (!vault.notes.some((n) => n.id === path)) await vault.refreshNotes();
    if (!vault.notes.some((n) => n.id === path)) return; // deleted after it landed: give up silently
    await vault.openNote(path);
    if (heading && vault.currentPath === path) {
      const hit = extractOutline(vault.currentContent).find((o) => o.text === heading);
      // revealInEditor: return to the editor and tick before scrolling (the same contract as the outline's jumps)
      if (hit) await vault.revealInEditor(hit.from);
    }
  }
  /** The three things a successful clip landing does: the main window's toast, recording the
   *  landing point and the toolbar's receipt — the two surfaces of one event (the toast and the
   *  bar's flash) share one wording, so changing the copy changes one place and nothing drifts. */
  function landedOk(path: string | null | undefined, message: string): ClipbarResult {
    toast.push(message);
    recordClipLanding(path);
    return { ok: true, message };
  }
  async function clipSelectionFlow(label?: string): Promise<ClipbarResult> {
    try {
      // The pinned destination wins: choose once and deliver many times (including the pinned
      // section); when the target has been deleted the pin is released (ensureLive has already
      // said so) and it falls back to the default destination. The fresh version: a note the
      // toolbar just created with "create and clip into" has not reached this window's notes
      // mirror yet, and a stale list must not kill a fresh pin
      if ((await clipTarget.ensureLiveFresh()) && clipTarget.path) {
        const target = clipTarget.path;
        const heading = clipTarget.heading;
        const spot = clipTarget.name ?? target;
        const res = await api.clipSelection(false, label);
        if (!res.text) return { ok: false, message: t("app.nothingSelected") };
        const ok = await clipTarget.deliver(target, res.text, res.url, heading);
        if (ok) recordClipLanding(target, heading);
        return ok ? { ok: true, message: t("app.clippedInto", { spot }) } : { ok: false, message: t("app.insertFailed") };
      }
      const res = await api.clipSelection(!vault.canInsertIntoVisibleNote, label);
      if (res.saved_path) return landedOk(res.saved_path, t("app.clippedTo", { path: res.saved_path }));
      // The order contract of inserting or appending, flushing and localising the images is maintained in deliver alone
      const cur = vault.currentPath;
      if (cur && (await clipTarget.deliver(cur, res.text, res.url))) {
        recordClipLanding(cur);
        return { ok: true, message: t("app.insertedInto", { path: cur }) };
      }
      // The race backstop: the editor has gone or cannot be inserted into after the check (deliver does not land), so it goes to clips instead
      const saved = await api.clipSelection(true, label);
      if (saved.saved_path) return landedOk(saved.saved_path, t("app.clippedTo", { path: saved.saved_path }));
      toast.push(t("app.clipFailed"), "error");
      return { ok: false, message: t("app.clipFailed") };
    } catch (e) {
      toast.push(String(e), "error");
      return { ok: false, message: String(e) };
    }
  }
  async function clipPageFlow(label?: string): Promise<ClipbarResult> {
    try {
      const rel = await api.clipPage(label);
      return landedOk(rel, t("app.clippedTo", { path: rel }));
    } catch (e) {
      toast.push(String(e), "error");
      return { ok: false, message: String(e) };
    }
  }
  async function clipLinkFlow(label?: string): Promise<ClipbarResult> {
    try {
      const res = await api.clipLink(label);
      if (vault.canInsertIntoVisibleNote && vault.insertAtCursor?.(res.text)) {
        return landedOk(vault.currentPath, t("app.insertedInto", { path: vault.currentPath ?? "" }));
      }
      await vault.capture(res.text);
      // The parenthetical only means something to a reader in the main window (explaining why it did not go into the current note); the bar's flash takes the short sentence
      toast.push(t("app.savedToInboxNoEditor"));
      recordClipLanding("inbox.md");
      return { ok: true, message: t("app.savedToInbox") };
    } catch (e) {
      toast.push(String(e), "error");
      return { ok: false, message: String(e) };
    }
  }
  registerCommand({
    id: "clip-selection", group: "clip",
    run: async () => {
      await clipSelectionFlow();
    },
  });
  registerCommand({
    id: "clip-selection-to-clips", group: "clip",
    run: async () => {
      try {
        const res = await api.clipSelection(true);
        if (res.saved_path) {
          toast.push(t("app.clippedTo", { path: res.saved_path }));
          recordClipLanding(res.saved_path);
        }
      } catch (e) { toast.push(String(e), "error"); }
    },
  });
  registerCommand({
    id: "clip-selection-to-note", group: "clip",
    run: async () => {
      try {
        const res = await api.clipSelection(false);
        if (!res.text) return;
        pickNoteForQuote(res.text, res.url);
      } catch (e) { toast.push(String(e), "error"); }
    },
  });
  registerCommand({
    id: "clip-page", hotkey: k("Mod+Shift+S"), group: "clip",
    run: async () => {
      await clipPageFlow();
    },
  });
  registerCommand({
    id: "clip-link", group: "clip",
    run: async () => {
      await clipLinkFlow();
    },
  });
  registerCommand({
    id: "reload-scripts", group: "system",
    run: async () => { try { await api.scriptsReload(); } catch (e) { toast.push(String(e), "error"); } },
  });
  registerCommand({
    id: "init-memory", group: "research",
    run: async () => {
      if (agent.running) { toast.push(t("app.agentBusy")); return; }
      const exists = vault.notes.some((n) => n.id === "VEMRY.md");
      // With no API key configured the agent's survey necessarily fails (send only surfaces the
      // error inside the panel), and the static template path must not become unreachable —
      // offline or without a key it falls back to v0.5's template behaviour
      const s = await api.settingsGet().catch(() => undefined);
      const noKey = s ? !s.api_key && !s.env_key_present : false;
      // A nearly empty vault has nothing to survey, so it falls back to the static template for the user to fill in
      if (!exists && (vault.notes.length < 3 || noKey)) {
        if (noKey) toast.push(t("app.noKeyStaticTemplate"));
        try { await api.initMemoryFile(); await vault.openNote("VEMRY.md"); } catch (e) { vault.error = String(e); }
        return;
      }
      if (exists && noKey) {
        toast.push(t("app.noKeyOpenedForEdit"));
        await vault.openNote("VEMRY.md");
        return;
      }
      const prompt = exists ? MEMORY_REVIEW_PROMPT() : MEMORY_DRAFT_PROMPT();
      if (agent.windowOpen) {
        // While the conversation has given way to the detached window, it is sent there (the same
        // route as clear-agent) rather than streamed into the Channel of the main window's hidden
        // panel — there is only a placeholder over there, and a whole turn's answer would go unseen
        await api.agentWindowToAgent({ kind: "send", text: prompt, currentNote: vault.currentPath });
        return;
      }
      vault.rightOpen = true;
      vault.rightTab = "agent";
      await agent.send(prompt, vault.currentPath);
      await vault.refreshNotes();
    },
  });
  registerCommand({
    id: "open-memory", group: "research",
    run: () => vault.openNote("VEMRY.md"),
  });
  registerCommand({ id: "block-type", group: "note", run: () => { paletteMode = "types"; } });
  registerCommand({ id: "init-research-skill", group: "research", run: () => scaffold(api.initResearchSkill) });
  registerCommand({
    id: "reveal-in-tree", group: "nav",
    run: () => vault.revealInTree(),
  });
  for (const [id, embed] of [["copy-block-link", false], ["copy-block-embed", true]] as const) {
    registerCommand({
      id, group: "note",
      run: () => {
        if (!vault.copyBlockRef?.(embed)) toast.push(t("block.noBlockHere"), "error");
      },
    });
  }
  // Block commands (the keys themselves are CodeMirror bindings in blocksel.ts; the
  // hints here are display only)
  for (const [id, op, hotkey] of [
    ["block-move-up", "move-up", k("Mod+Shift+↑")],
    ["block-move-down", "move-down", k("Mod+Shift+↓")],
    ["block-duplicate", "duplicate", k("Mod+Shift+D")],
    ["block-delete", "delete", undefined],
  ] as const) {
    registerCommand({
      id, group: "note", hotkey,
      run: () => {
        if (!vault.blockCommand?.(op)) toast.push(t("block.noBlockHere"), "error");
      },
    });
  }
  // Provenance review (the ledger is post-write review; the permission gate is pre-write consent)
  registerCommand({
    id: "toggle-provenance", group: "note",
    run: () => {
      vault.provenanceOverlay = !vault.provenanceOverlay;
      toast.push(t(vault.provenanceOverlay ? "prov.overlayOn" : "prov.overlayOff"));
    },
  });
  registerCommand({
    id: "provenance-accept", group: "note",
    run: () => {
      const id = vault.pendingAtCursor?.();
      if (!id) {
        toast.push(t("prov.noPendingHere"), "error");
        return;
      }
      vault.acceptProvenance(id).then(() => toast.push(t("prov.accepted"))).catch((e) => toast.push(String(e), "error"));
    },
  });
  registerCommand({
    id: "provenance-accept-all", group: "note",
    run: () => {
      if (!vault.pendingCount) {
        toast.push(t("prov.noPending"), "error");
        return;
      }
      vault.acceptProvenance(null).then((n) => toast.push(t("prov.acceptedAll", { n }))).catch((e) => toast.push(String(e), "error"));
    },
  });
  registerCommand({
    id: "provenance-reject", group: "note",
    run: () => {
      const id = vault.pendingAtCursor?.();
      if (!id) {
        toast.push(t("prov.noPendingHere"), "error");
        return;
      }
      void vault.rejectProvenance?.(id);
    },
  });
  registerCommand({
    id: "provenance-reject-all", group: "note",
    run: () => {
      if (!vault.pendingCount) {
        toast.push(t("prov.noPending"), "error");
        return;
      }
      void vault.rejectProvenance?.(null);
    },
  });
  registerCommand({
    id: "provenance-next-pending", group: "note",
    run: () => {
      if (!vault.nextPending?.()) toast.push(t("prov.noPending"), "error");
    },
  });
  registerCommand({
    id: "delete-current-note", group: "note",
    run: async () => {
      if (vault.currentPath) {
        try { await vault.deletePath(vault.currentPath); } catch (e) { vault.error = String(e); }
      }
    },
  });
  registerCommand({
    id: "mark-private", group: "note",
    run: async () => {
      const path = vault.currentPath;
      if (path) await vault.markPrivate(path, path.endsWith(".md") ? "note" : "file");
    },
  });
  registerCommand({
    id: "unmark-private", group: "note",
    run: async () => {
      const path = vault.currentPath;
      if (!path) return;
      // The registry has no enable predicate: an inherited path explains itself here instead
      // of a silent no-op (Rust removes only the exact rule line)
      const st = vault.privacyStatus(path);
      if (st.kind === "inherited") {
        toast.push(t("fileTree.privacyCovered", { zone: `${st.zone}/` }));
        return;
      }
      await vault.unmarkPrivate(path, path.endsWith(".md") ? "note" : "file");
    },
  });
  // The projection views: different presentations of the same index data (a view is a query),
  // opened into the tab system through openView (the preview-slot semantics, promoted by a double
  // click or an edit, as a document is). Escape closes a view's tab.
  registerCommand({
    id: "view-agenda", group: "view",
    run: () => void vault.openView("agenda"),
  });
  registerCommand({
    id: "view-kanban", group: "view",
    run: () => void vault.openView("kanban"),
  });
  registerCommand({
    id: "view-timeline", group: "view",
    run: () => void vault.openView("timeline"),
  });
  registerCommand({
    id: "view-claims", group: "view",
    run: () => void vault.openView("claims"),
  });
  registerCommand({
    id: "view-evidence", group: "view",
    run: () => void vault.openView("evidence"),
  });
  registerCommand({
    id: "view-graph", group: "view",
    run: () => void vault.openView("graph"),
  });
  registerCommand({
    // The same data no longer has three names: the sidebar tab "Research" is the panel, and this is "Session replay"
    id: "view-sessions", group: "view",
    run: () => void vault.openView("sessions"),
  });
  registerCommand({
    id: "view-reading", group: "view",
    run: () => {
      if (reading.current || reading.loading || reading.error) vault.enterReading();
      else toast.push(t("app.noReaderPageYet"));
    },
  });
  // The editor's tabs. On mac ⌘W is dispatched back to close-tab by the native menu's accelerator
  // (menu.rs's close_focused_window), and off mac it goes through App's keydown; with no tab it is a no-op
  registerCommand({
    id: "close-tab", descriptionParams: { key: k("Mod+W") }, group: "view",
    run: async () => { if (vault.activeTabId != null) await vault.closeTab(vault.activeTabId); },
  });
  registerCommand({
    id: "next-tab", descriptionParams: { key: `${k("Mod+Shift+]")} / ${k("Ctrl+Tab")}` }, group: "view",
    run: () => void vault.cycleTab(1),
  });
  registerCommand({
    id: "prev-tab", descriptionParams: { key: `${k("Mod+Shift+[")} / ${k("Ctrl+Shift+Tab")}` }, group: "view",
    run: () => void vault.cycleTab(-1),
  });
  registerCommand({
    // A one-off explicit migration (⚠ never run automatically): an old bucket is shared by stem
    // and its ownership cannot be decided, while a new one is unique to a note's full path. A
    // dry run's report → a confirmation → the real run; mixed buckets and orphans stay put
    id: "migrate-attachments",     group: "system",
    run: async () => {
      await vault.flushEditor?.();
      try {
        const dry = await api.attachmentsMigrate(true);
        if (!dry.moved.length && !dry.mixed.length && !dry.orphaned.length && !dry.conflicts.length) {
          toast.push(t("app.attachmentsAlreadyMigrated"));
          return;
        }
        const lines = [
          t("app.migratePlan", { buckets: dry.moved.length, notes: dry.rewritten_notes }),
          dry.mixed.length ? t("app.migrateMixed", { n: dry.mixed.length, list: dry.mixed.join(t("app.listSep")) }) : "",
          dry.orphaned.length ? t("app.migrateOrphaned", { n: dry.orphaned.length }) : "",
          dry.conflicts.length ? t("app.migrateConflicts", { n: dry.conflicts.length, list: dry.conflicts.join(t("app.listSep")) }) : "",
        ].filter(Boolean);
        if (!dry.moved.length) {
          toast.push(lines.join(" "));
          return;
        }
        const ok = await confirmDialog(lines.join("\n"), { title: t("cmd.migrate-attachments.title"), kind: "warning" });
        if (!ok) return;
        const r = await api.attachmentsMigrate(false);
        // The rewrite goes through write_note (the self-write suppression swallows NoteModified
        // and only IndexReady is broadcast), so the buffer of an open note is not reloaded by an
        // event — without aligning it explicitly, the next edit or autosave would write the
        // pre-migration links back (the attachments have moved = dead links)
        await vault.reloadCurrent();
        toast.push(t("app.migrateDone", { buckets: r.moved.length, notes: r.rewritten_notes }));
        await vault.refreshNotes();
      } catch (e) {
        vault.error = String(e);
      }
    },
  });
  registerCommand({ id: "settings", group: "system", run: () => { settingsTab = "general"; settingsOpen = true; } });
  registerCommand({
    id: "check-updates", group: "system",
    run: () => { settingsTab = "updates"; settingsOpen = true; void updater.check(true); },
  });
  registerCommand({
    // The old "clear the AI conversation" only cleared the timeline and not the model context —
    // the meaning is now a new session: the UI and Rust's history change gear together, and the
    // old conversation stays in the session list
    id: "clear-agent", group: "research",
        // While the conversation is in the detached window it runs there (its timeline has to be cleared in step)
    run: () => void (agent.windowOpen ? api.agentWindowToAgent({ kind: "new-session" }) : agent.newSession()),
  });

  function tabsMenu(e: MouseEvent) {
    contextMenu.open(e, [
      // The ✓ prefix is this menu's presentation; the action goes through command dispatch and does not copy a command's body
      { label: `${vault.rightTab === "agent" ? "✓ " : ""}${t("cmd.show-agent.title")}`, action: () => void executeCommand("show-agent") },
      { label: `${vault.rightTab === "outline" ? "✓ " : ""}${t("cmd.show-outline.title")}`, action: () => void executeCommand("show-outline") },
      { separator: true },
      { label: t("cmd.toggle-sidebar-side.title"), action: () => void executeCommand("toggle-sidebar-side") },
      { label: t("app.collapseSidebar"), hint: k("Mod+."), action: () => { vault.rightOpen = false; } },
    ]);
  }

  function onkeydown(e: KeyboardEvent) {
    const mod = e.metaKey || e.ctrlKey;
    if (!mod) {
      if (e.key === "Escape") {
        // An Escape with the focus in an input belongs to that input (cancelling an edit, clearing
        // it) and does not count as "leave the projection view" — otherwise cancelling a create
        // input in the file tree would switch back to the editor along the way.
        // But swallowing it and doing nothing would trap the user inside the projection (Esc would
        // stop working with the focus in the research panel's input), so this press returns the
        // focus first and one more Esc leaves
        const field = e.target instanceof HTMLElement
          ? e.target.closest<HTMLElement>("input, textarea, [contenteditable='true']")
          : null;
        if (paletteMode) paletteMode = null;
        else if (!settingsOpen && !agent.pendingPermission && vault.mainView !== "editor") {
          if (field) field.blur();
          else if (vault.mainView === "reading") vault.exitReading();
          else void vault.closeActiveView();
        }
      }
      return;
    }
    const key = e.key.toLowerCase();
    // The isMac gate: on macOS ⌘, ⌘N ⌘O ⌘⇧F ⌘⇧C ⌘⇧S belong to the native menu's accelerators
    // (class 1 of menu.rs's three), so this gives way to avoid firing twice (a double ⌘⇧S would
    // clip twice); ⌘P ⌘L ⌘B ⌘. ⌘⌥arrows stay frontend-only (the menu sets no accelerator)
    if (key === "p" && !e.shiftKey) { e.preventDefault(); paletteMode = "commands"; }
    else if (key === "l" && !e.shiftKey) { e.preventDefault(); paletteMode = "url"; }
    else if (!isMac && key === "o") { e.preventDefault(); paletteMode = "notes"; }
    else if (!isMac && key === "f" && e.shiftKey) { e.preventDefault(); paletteMode = "search"; }
    else if (!isMac && key === "c" && e.shiftKey) { e.preventDefault(); paletteMode = "capture"; }
    else if (!isMac && key === "s" && e.shiftKey) { e.preventDefault(); void executeCommand("clip-page"); }
    else if (!isMac && key === "n") { e.preventDefault(); paletteMode = "notes"; }
    else if (key === "b") { e.preventDefault(); vault.leftOpen = !vault.leftOpen; }
    else if (key === ".") { e.preventDefault(); vault.rightOpen = !vault.rightOpen; }
    else if (key === "j" && !e.shiftKey && !e.altKey) { e.preventDefault(); void executeCommand("focus-agent"); }
    else if (!isMac && key === ",") { e.preventDefault(); void executeCommand("settings"); }

    // ⌘⌥←/→ for forward and back; ⌘[/] is taken by CodeMirror's defaultKeymap for indenting
    else if (e.altKey && key === "arrowleft") { e.preventDefault(); navBack(); }
    else if (e.altKey && key === "arrowright") { e.preventDefault(); navForward(); }

    // ---- The editor's tabs. ⌘⇧[] is decided by e.code (with shift, e.key is {/});
    // ⌘⇧[ is not taken by CM (which only takes ⌘[/] for indenting). ctrl+Tab cycles in order (no
    // MRU).
    // On mac ⌘W belongs to the native menu's accelerator (it always beats the webview, the lesson
    // of batch D), so only the non-mac binding is here ----
    else if (e.shiftKey && e.code === "BracketRight") { e.preventDefault(); void vault.cycleTab(1); }
    else if (e.shiftKey && e.code === "BracketLeft") { e.preventDefault(); void vault.cycleTab(-1); }
    else if (e.ctrlKey && !e.metaKey && key === "tab") { e.preventDefault(); void vault.cycleTab(e.shiftKey ? -1 : 1); }
    else if (!isMac && key === "w" && !e.shiftKey) {
      // Ctrl+W is the delete-word muscle memory of terminals and editors: with the focus in an
      // input, or the settings or a panel open, it must not close the tab underneath (the same
      // field check as the Escape branch's)
      const field = e.target instanceof HTMLElement
        && e.target.closest("input, textarea, [contenteditable='true']");
      if (!settingsOpen && !paletteMode && !field) { e.preventDefault(); void executeCommand("close-tab"); }
    }
  }
</script>

<svelte:window {onkeydown} />

{#if !vault.root}
  <main class="welcome">
    <div class="welcome-card">
      <h1>vemry</h1>
      <p class="tagline">{t("app.tagline")}</p>
      <div class="welcome-actions">
        <button class="open-btn" onclick={pickVault}>{t("app.openVaultFolder")}</button>
        <button class="open-btn ghost" onclick={createVaultFlow}>{t("cmd.create-vault.title")}</button>
      </div>
      {#if vault.recent.length}
        <div class="recent">
          <span class="recent-label">{t("app.recent")}</span>
          {#each vault.recent as path (path)}
            <!-- lrm bookends: under the rtl truncation trick, the neutral characters at a path's ends would be reordered by bidi -->
            <button
              class="recent-item"
              onclick={() => vault.openVault(path)}
              oncontextmenu={(e) =>
                contextMenu.open(e, [{ label: t("fileTree.removeFromRecent"), action: () => void vault.removeRecent(path) }])}
            >&lrm;{path}&lrm;</button>
          {/each}
        </div>
      {/if}
      {#if vault.error}<p class="error">{vault.error}</p>{/if}
    </div>
  </main>
{:else}
  <!-- With the main window full-screen or against the screen's right edge, a docked browser
    window lies over its right edge (a native child window is always above the DOM), so the whole
    layout gives way by the inset's width → visually a split view with notes on the left and the
    web on the right.
    The giving-way hangs on frame (the vertical wrapper of the shell plus the bottom status bar):
    once the sides are swapped the activity bar is at the far right, the status bar's right
    section is the cursor gauge, and a docked window is the main window's full height — miss any
    of them and it would be covered by the native browser window -->
  <div class="frame" style:padding-right={browser.overlayInset > 0 ? `${browser.overlayInset}px` : null}>
    <div class="shell" class:swapped={vault.swapSides}>
    <!-- The activity bar sits outside the layout (the grid): fixed-width chrome takes no part in
      the column template's minmax negotiation; swapping sides relies on the shell's row-reverse
      alone, so it always hugs the window's outermost edge. It is outside #key vault.root as well:
      a vault switch does not remount it and nothing flickers -->
    <ActivityBar />
    <div class="layout" class:swapped={vault.swapSides} style:grid-template-columns={gridCols}>
      {#if vault.leftOpen}
        <aside class="left">
          <!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
          <div
            class="sash"
            class:at-left={vault.swapSides}
            class:at-right={!vault.swapSides}
            class:active={vault.resizing === "tree"}
            role="separator"
            aria-orientation="vertical"
            aria-label={t("app.resizeFileTree")}
            title={t("app.dragToResize")}
            onpointerdown={(e) => startResize(e, "tree")}
            ondblclick={() => (vault.leftWidth = null)}
          ></div>
          <!-- key: remounted on a vault switch, which resets the transient state (the paste
            destination, an inline edit and so on)
            (the collapsed state moved up into the vault store, is persisted per vault and is
            unaffected by the remount) -->
          {#key vault.root}
            <FileTree />
          {/key}
        </aside>
      {/if}
      <main class="center">
        <!-- The tab bar is permanent: documents and projection views live in one bar (a view tab
          is the VIEW_TAB_PREFIX in vault, with no buffer); the reading projection is the one
          overlay state that does not enter the tabs, and during it the bar only greys out rather
          than disappearing. tabs empty ⟺ nothing is open -->
        {#if vault.tabs.length > 0}
          <TabBar />
        {/if}
        <!-- The main-area host shared by the editor and the projections: .center is a flex column,
          and this layer uses flex:1 to make a block container of a definite height, which is what
          keeps the height:100% inside (editor-wrap and each view's root) meaning what it did -->
        <div class="main-host">
          <!-- Editor stays mounted and is hidden with CSS: CodeMirror holds the unsaved buffer and
            the undo stack, and unmounting and remounting would lose the edits (currentContent is
            stale while dirty) -->
          <div class="editor-host" style:display={vault.mainView === "editor" ? "contents" : "none"}>
            <Editor />
          </div>
          {#key vault.root}
            {#if vault.mainView === "agenda"}
              <AgendaView />
            {:else if vault.mainView === "kanban"}
              <KanbanView />
            {:else if vault.mainView === "timeline"}
              <TimelineView />
            {:else if vault.mainView === "claims"}
              <ClaimsView />
            {:else if vault.mainView === "evidence"}
              <EvidenceView />
            {:else if vault.mainView === "graph"}
              <GraphView />
            {:else if vault.mainView === "sessions"}
              <SessionsView />
            {:else if vault.mainView === "reading"}
              <ReadingView onPickQuoteTarget={pickNoteForQuote} />
            {/if}
          {/key}
        </div>
      </main>
      {#if vault.rightOpen}
        <aside class="right">
          <!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
          <div
            class="sash"
            class:at-left={!vault.swapSides}
            class:at-right={vault.swapSides}
            class:active={vault.resizing === "panel"}
            role="separator"
            aria-orientation="vertical"
            aria-label={t("app.resizeSidebar")}
            title={t("app.dragToResize")}
            onpointerdown={(e) => startResize(e, "panel")}
            ondblclick={() => (vault.rightWidth = null)}
          ></div>
          <!-- svelte-ignore a11y_no_static_element_interactions -->
          <div class="tabs" oncontextmenu={tabsMenu}>
            <button class:active={vault.rightTab === "outline"} onclick={() => (vault.rightTab = "outline")}>{t("app.tabOutline")}</button>
            <button class:active={vault.rightTab === "agent"} onclick={() => (vault.rightTab = "agent")}>{t("app.tabResearch")}</button>
            <button class="panel-close" title={t("app.collapseSidebarKey", { key: k("Mod+.") })} aria-label={t("app.collapseSidebar")} onclick={() => (vault.rightOpen = false)}>
              <svg width="18" height="18" viewBox="0 0 20 20" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true" style:transform={vault.swapSides ? "scaleX(-1)" : null}>
                <rect x="2.5" y="3.5" width="15" height="13" rx="1.75" />
                <path d="M12.5 3.5 V16.5 M6.5 8 L8.5 10 L6.5 12" />
              </svg>
            </button>
          </div>
          <!-- Both panels stay mounted and are switched with CSS: changing tabs loses neither the
            scroll position, the input draft nor the research panel's web-context attachment -->
          <div class="panel-host" style:display={vault.rightTab === "outline" ? "flex" : "none"}>
            <OutlinePanel />
          </div>
          <div class="panel-host" style:display={vault.rightTab === "outline" ? "none" : "flex"}>
            <AgentPanel />
          </div>
        </aside>
      {:else}
        <button class="reopen" class:at-right={!vault.swapSides} class:at-left={vault.swapSides} title={t("app.openSidebarKey", { key: k("Mod+.") })} aria-label={t("app.openSidebar")} onclick={() => (vault.rightOpen = true)}>
          <svg width="18" height="18" viewBox="0 0 20 20" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true" style:transform={vault.swapSides ? "scaleX(-1)" : null}>
            <rect x="2.5" y="3.5" width="15" height="13" rx="1.75" />
            <path d="M12.5 3.5 V16.5" />
          </svg>
        </button>
      {/if}
    </div>
    </div>
    <StatusBar />
  </div>
  {#if vault.resizing}
    <!-- The full-screen cursor overlay while dragging: the sash has already called
      setPointerCapture, so the overlay intercepts no events and only unifies the col-resize cursor
      across the window and suppresses the hover feedback of whatever it passes over -->
    <div class="resize-veil" aria-hidden="true"></div>
  {/if}
  {#if vault.error}
    <div class="toast" role="alert">
      {vault.error}
      <button onclick={() => (vault.error = null)}>✕</button>
    </div>
  {/if}
{/if}

<!-- With the main layout's bottom status bar the toast is raised; the welcome page has none and keeps the default -->
<ToastStack lift={vault.root ? "var(--statusbar-h)" : "0px"} />
<CommandPalette bind:mode={paletteMode} onPickNote={deliverPicked} onCaptureToNote={pickNoteForCapture} />
<ContextMenu />
<PermissionDialog />
<ScriptGrantDialog />
<SettingsModal bind:open={settingsOpen} bind:tab={settingsTab} />

<style>
  .welcome {
    height: 100%;
    display: grid;
    place-items: center;
  }
  .welcome-card {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 12px;
    max-width: 420px;
    text-align: center;
    padding: 24px;
  }
  h1 {
    font-size: 28px;
    font-weight: 600;
    letter-spacing: -0.02em;
  }
  .tagline {
    color: var(--fg-muted);
    font-size: var(--fs-sm);
    line-height: 1.6;
  }
  .welcome-actions {
    margin-top: 12px;
    display: flex;
    gap: 10px;
  }
  .open-btn {
    background: var(--accent);
    color: #fff;
    padding: 9px 20px;
    border-radius: 8px;
    font-size: var(--fs-md);
  }
  .open-btn:hover {
    opacity: 0.92;
  }
  .open-btn.ghost {
    background: transparent;
    color: var(--fg-muted);
    border: 1px solid var(--border);
  }
  .open-btn.ghost:hover {
    opacity: 1;
    color: var(--fg);
    background: var(--bg-hover);
  }
  .recent {
    margin-top: 20px;
    display: flex;
    flex-direction: column;
    gap: 2px;
    width: 100%;
  }
  .recent-label {
    font-size: var(--fs-xs);
    color: var(--fg-faint);
    text-transform: uppercase;
    letter-spacing: 0.06em;
    margin-bottom: 4px;
  }
  .recent-item {
    font-size: var(--fs-sm);
    color: var(--fg-muted);
    font-family: var(--font-mono);
    padding: 5px 8px;
    border-radius: 5px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    direction: rtl;
    text-align: left;
  }
  .recent-item:hover {
    background: var(--bg-hover);
    color: var(--fg);
  }
  .error {
    color: var(--danger);
    font-size: var(--fs-xs);
  }

  .frame {
    height: 100%;
    display: flex;
    flex-direction: column;
  }
  .shell {
    display: flex;
    flex: 1;
    min-height: 0;
  }
  /* Swapping sides mirrors the activity bar together with all three columns: the activity bar always hugs the window's outermost edge */
  .shell.swapped {
    flex-direction: row-reverse;
  }
  .layout {
    flex: 1 1 auto;
    min-width: 0;
    display: grid;
    /* The column template is derived by the script (gridCols) and written as an inline style: the
      open/closed state, the swap and the user's drag are all combined in that one place. The
      elastic policy and the coupling (the content area's floor of 300, and the three columns' min
      plus the activity bar = 764 going with minWidth=764 / Rust's MIN_MAIN_VISIBLE=804) are
      explained in the script's comments; only one default fallback is kept here */
    grid-template-columns: minmax(140px, 220px) minmax(300px, 1fr) minmax(280px, min(30%, 560px));
    /* single row capped at viewport height — panels scroll inside, never grow the row */
    grid-template-rows: minmax(0, 1fr);
  }
  /* Swapping the sidebars: the grid takes them in the document order after order, and the column
    template has already been reversed by the script; the divider border moves to whichever side
    faces the content area */
  .layout.swapped .right {
    order: 1;
    border-left: none;
    border-right: 1px solid var(--border-strong);
  }
  .layout.swapped .center {
    order: 2;
  }
  .layout.swapped .left {
    order: 3;
    border-right: none;
    border-left: 1px solid var(--border-strong);
  }
  .left {
    position: relative;
    border-right: 1px solid var(--border-strong);
    background: var(--bg-panel);
    display: flex;
    flex-direction: column;
    min-width: 0;
    min-height: 0;
  }
  .center {
    display: flex;
    flex-direction: column;
    min-width: 0;
    min-height: 0;
    overflow: hidden;
    background: var(--bg);
  }
  /* A block container of a definite height: the height:100% of editor-wrap and each projection
     view's root measures against it (a height:100% on a flex item would measure .center and push
     through the tab bar) */
  .main-host {
    flex: 1;
    min-height: 0;
    min-width: 0;
  }
  .right {
    position: relative;
    border-left: 1px solid var(--border-strong);
    background: var(--bg-panel);
    display: flex;
    flex-direction: column;
    min-width: 0;
    min-height: 0;
    overflow: hidden;
  }
  /* The divider: it rides the sidebar's edge facing the content area (a sidebar has
    overflow:hidden, so the whole strip has to sit inside it and cannot cross), and the handle line
    appears after a moment's hover or while dragging */
  .sash {
    position: absolute;
    top: 0;
    bottom: 0;
    width: 6px;
    cursor: col-resize;
    z-index: 30;
    touch-action: none;
  }
  .sash.at-left {
    left: 0;
  }
  .sash.at-right {
    right: 0;
  }
  .sash::after {
    content: "";
    position: absolute;
    top: 0;
    bottom: 0;
    width: 2px;
    background: transparent;
    /* It lights up after a hover delay (:hover overrides the delay) and goes out at once when the pointer leaves (through this 0s) */
    transition: background 0.1s ease 0s;
  }
  .sash.at-left::after {
    left: 0;
  }
  .sash.at-right::after {
    right: 0;
  }
  .sash:hover::after {
    background: var(--accent);
    transition-delay: 0.3s;
  }
  .sash.active::after {
    background: var(--accent);
    transition-delay: 0s;
  }
  .resize-veil {
    position: fixed;
    inset: 0;
    z-index: 250;
    cursor: col-resize;
  }
  .panel-host {
    flex: 1;
    flex-direction: column;
    min-height: 0;
    overflow: hidden;
  }
  .tabs {
    display: flex;
    gap: 2px;
    padding: 8px 8px 0;
    border-bottom: 1px solid var(--border);
  }
  .tabs button {
    font-size: var(--fs-sm);
    color: var(--fg-muted);
    padding: 5px 12px;
    border-radius: 5px 5px 0 0;
    border-bottom: 2px solid transparent;
  }
  .tabs button.active {
    color: var(--fg);
    border-bottom-color: var(--accent);
  }
  .tabs .panel-close {
    margin-left: auto;
    display: grid;
    place-items: center;
    width: 26px;
    height: 26px;
    padding: 0;
    border-radius: 6px;
    border-bottom: none;
    color: var(--fg-muted);
    align-self: center;
  }
  .tabs .panel-close:hover {
    background: var(--bg-hover);
    color: var(--fg);
  }
  /* Once collapsed the grid column disappears entirely, and the only way to reopen it floats in the corner on that column's docking side */
  .reopen {
    position: fixed;
    top: 8px;
    z-index: 40;
    display: grid;
    place-items: center;
    width: 30px;
    height: 30px;
    border-radius: 6px;
    color: var(--fg-faint);
  }
  .reopen.at-right {
    right: 8px;
  }
  .reopen.at-left {
    left: 8px;
  }
  .reopen:hover {
    background: var(--bg-hover);
    color: var(--fg-muted);
  }
  .toast {
    position: fixed;
    /* It clears the bottom status bar, on the same baseline as ToastStack */
    bottom: calc(16px + var(--statusbar-h));
    left: 50%;
    transform: translateX(-50%);
    background: var(--bg-panel);
    border: 1px solid var(--border);
    border-left: 2px solid var(--danger);
    border-radius: 6px;
    padding: 8px 12px;
    font-size: var(--fs-sm);
    display: flex;
    gap: 10px;
    align-items: center;
    box-shadow: var(--shadow);
    z-index: 300;
    max-width: 80vw;
  }
</style>
