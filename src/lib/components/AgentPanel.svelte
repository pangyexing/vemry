<script lang="ts">
  // The research panel: the AI conversation is the main axis, and the browsing state is
  // pinned at the top as a "current page card" — the browser controls are the
  // conversation's context rather than a parallel tab. The conversation body (the timeline
  // plus the composer) lives in AgentChat: the same implementation the ⌘J window uses, with
  // all the session state in the agent store.
  import { t } from "$lib/i18n/i18n.svelte";
  import { api } from "$lib/api";
  import { agent } from "$lib/stores/agent.svelte";
  import { vault } from "$lib/stores/vault.svelte";
  import { browser } from "$lib/stores/browser.svelte";
  import { clips } from "$lib/stores/clips.svelte";
  import { clipTarget } from "$lib/stores/clipTarget.svelte";
  import { toast } from "$lib/stores/toast.svelte";
  import { prefs } from "$lib/stores/prefs.svelte";
  import { hostDispatch } from "$lib/host";
  import { isAgentWindow } from "$lib/window";
  import { contextMenu } from "$lib/stores/contextMenu.svelte";
  import { displayTitle, editableTitle, renameKeydown } from "$lib/views/session";
  import StarIcon from "$lib/components/StarIcon.svelte";
  import { domainOf } from "$lib/markdown";
  import { k } from "$lib/shortcut";
  import AgentChat from "./AgentChat.svelte";

  let sessionsOpen = $state(false);
  /** A two-step delete: the first ✕ turns into "confirm delete", and only a second click really deletes (no system dialog is raised). */
  let confirmDelete = $state<string | null>(null);
  /** The per-turn fragments from before the upgrade (whose context cannot be resumed) are collapsed into a drawer by default, so they do not flood the resumable sessions. */
  let showLegacy = $state(false);

  const recentSessions = $derived(agent.sessions.filter((s) => s.resumable));
  const legacySessions = $derived(agent.sessions.filter((s) => !s.resumable));

  const currentTitle = $derived.by(() => {
    const meta = agent.sessions.find((s) => s.id === agent.sessionId);
    if (meta) return displayTitle(meta);
    // The first turn was just sent and the list has not refreshed: the first user message stands in
    const firstUser = agent.timeline.find((t) => t.kind === "user");
    return firstUser?.kind === "user" ? firstUser.text.split("\n")[0].slice(0, 40) : t("agentPanel.newSession");
  });

  function relTime(ts: string): string {
    const d = new Date(ts);
    if (isNaN(d.getTime())) return "";
    const diff = Date.now() - d.getTime();
    if (diff < 60_000) return t("agentPanel.justNow");
    if (diff < 3_600_000) return t("agentPanel.minutesAgo", { n: Math.floor(diff / 60_000) });
    if (diff < 86_400_000) return t("agentPanel.hoursAgo", { n: Math.floor(diff / 3_600_000) });
    const pad = (n: number) => String(n).padStart(2, "0");
    return `${pad(d.getMonth() + 1)}-${pad(d.getDate())}`;
  }

  function closeSessions() {
    sessionsOpen = false;
    confirmDelete = null;
    renamingId = null;
  }

  // The timeline's presentation state is reset after a session change in AgentChat (driven by the store's viewEpoch)
  function pickSession(s: { id: string; resumable: boolean }) {
    closeSessions();
    if (!s.resumable) toast.push(t("agentPanel.legacyResume"));
    run(agent.selectSession(s.id));
  }

  function newSession() {
    closeSessions();
    run(agent.newSession());
  }

  // ---- the session row's context menu (rename / delete) ----
  function sessionMenu(e: MouseEvent, s: (typeof agent.sessions)[number]) {
    contextMenu.open(e, [
      { title: displayTitle(s) },
      { label: t("sessionsView.rename"), action: () => startRename(s.id) },
      { separator: true },
      { label: t("sessionsView.delete"), danger: true, confirm: t("sessionsView.deleteConfirm"), action: () => deleteSession(s.id) },
    ]);
  }

  // ---- inline renaming (entered from the context menu; a single click on a row switches sessions, and there is no double-click rename) ----
  let renamingId = $state<string | null>(null);
  let renameEl = $state<HTMLInputElement | null>(null);
  function startRename(id: string) {
    confirmDelete = null;
    renamingId = id;
    queueMicrotask(() => renameEl?.select());
  }
  const cancelRename = () => (renamingId = null);
  function commitRename(id: string, value: string) {
    if (renamingId !== id) return; // Esc already cancelled it (the id is cleared before the blur fires)
    renamingId = null;
    const t = value.trim();
    const cur = agent.sessions.find((x) => x.id === id);
    if (!t || t === cur?.title) return;
    run(agent.renameSession(id, t));
  }

  function deleteSession(id: string) {
    confirmDelete = null;
    run(agent.deleteSession(id));
  }

  const run = (p: Promise<unknown>) => void p.catch((e) => toast.push(String(e), "error"));

  // Whether the current page has been clipped (the citation-landed badge)
  $effect(() => {
    const url = browser.current?.url;
    if (url) void clips.lookup([url]);
  });

  function openClipped(url: string) {
    const p = clips.pathFor(url);
    if (p) void hostDispatch({ kind: "open-note", path: p });
  }
  /** The clip commands on the page card belong to the main window (the editor and the clip destination are there), and in the chat window they run through the relay. */
  const cmd = (id: string) => hostDispatch({ kind: "command", id });

  // The context draft lives in the agent store: the reading projection and the @ picker hand theirs over in the same place
  async function attachContext() {
    const page = await agent.grabPage();
    if (page) agent.attachPage(page);
  }
</script>

<div class="agent">
{#if agent.windowOpen && !isAgentWindow}
  <!-- The giving-way invariant: while the conversation is in the detached window, the
       session bar, the page card and the conversation body are not rendered here — one
       session has one visible composer, and switching sessions happens over there too -->
  <div class="detached">
    <p>{t("agentPanel.inSeparateWindow")}</p>
    <div class="detached-actions">
      <button onclick={() => void agent.openWindow(vault.currentPath, vault.editorSelection).catch((e) => toast.push(String(e), "error"))}>{t("agentPanel.goThere")}</button>
      <button onclick={() => run(api.agentWindowClose(null, false))}>{t("agentPanel.backToMainWindow")}</button>
    </div>
  </div>
{:else}
  <div class="sesswrap">
    <div class="sessbar">
      <button
        class="sess-cur"
        disabled={agent.running}
        title={t(agent.running ? "agentPanel.cannotSwitchWhileRunning" : "agentPanel.switchSession")}
        onclick={() => (sessionsOpen ? closeSessions() : (sessionsOpen = true))}
      >
        <span class="sess-title">{currentTitle}</span>
        <span class="sess-caret" class:open={sessionsOpen} aria-hidden="true">
          <svg width="14" height="14" viewBox="0 0 16 16" fill="none"><path d="M4 6l4 4 4-4" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"/></svg>
        </span>
      </button>
      {#if prefs.model}
        <span class="sess-model" title={t("agentPanel.currentModel", { model: prefs.model })}>{prefs.model}</span>
      {/if}
      {#if prefs.webSearchEnabled && !prefs.webSearchActive}
        <span
          class="sess-nosearch"
          title={t("agentPanel.searchInactiveHint")}
        >{t("agentPanel.searchInactive")}</span>
      {/if}
      <button
        class="sess-new"
        disabled={agent.running || (agent.timeline.length === 0 && !agent.sessionId)}
        title={t("agentPanel.newSessionHint")}
        onclick={newSession}
      >{t("agentPanel.newSessionButton")}</button>
    </div>
    {#if sessionsOpen}
      <button class="sess-backdrop" aria-label={t("agentPanel.closeSessionList")} onclick={closeSessions}></button>
      <div class="sess-pop">
        {#if agent.sessions.length === 0}
          <div class="sess-empty">{t("agentPanel.noSessions")}</div>
        {:else if recentSessions.length === 0 && !showLegacy}
          <div class="sess-empty">{t("agentPanel.noResumable")}</div>
        {/if}
        {#each recentSessions as s (s.id)}
          {@render sessionRow(s, false)}
        {/each}
        {#if legacySessions.length}
          <button
            class="sess-legacy-toggle"
            title={t("agentPanel.legacyHint")}
            onclick={() => (showLegacy = !showLegacy)}
          >
            {showLegacy ? "▾" : "▸"} {t("agentPanel.legacyDrawer", { n: legacySessions.length })}
          </button>
          {#if showLegacy}
            {#each legacySessions as s (s.id)}
              {@render sessionRow(s, true)}
            {/each}
          {/if}
        {/if}
      </div>
    {/if}
  </div>
  {#snippet sessionRow(s: (typeof agent.sessions)[number], legacy: boolean)}
    <div class="sess-row" class:active={s.id === agent.sessionId} class:legacy>
      {#if renamingId === s.id}
        <input
          class="sess-rename"
          bind:this={renameEl}
          value={editableTitle(s)}
          placeholder={t("sessionsView.titlePlaceholder")}
          spellcheck="false"
          onkeydown={(e) => renameKeydown(e, cancelRename)}
          onblur={(e) => commitRename(s.id, e.currentTarget.value)}
        />
      {:else}
        <button class="sess-main" onclick={() => pickSession(s)} oncontextmenu={(e) => sessionMenu(e, s)}>
          {#if s.id === agent.sessionId}<span class="sess-check">✓</span>{/if}
          {#if s.forked}<span class="sess-fork" title={t("sessionsView.forked")}>⑂</span>{/if}
          <span class="sess-t">{displayTitle(s)}</span>
          <span class="sess-m">{t("agentPanel.turns", { n: s.turns })}{relTime(s.last_ts) ? ` · ${relTime(s.last_ts)}` : ""}</span>
        </button>
        {#if confirmDelete === s.id}
          <button class="sess-del confirm" onclick={() => deleteSession(s.id)}>{t("agentPanel.confirmDelete")}</button>
        {:else}
          <button class="sess-del" aria-label={t("sessionsView.delete")} onclick={() => (confirmDelete = s.id)}>✕</button>
        {/if}
      {/if}
    </div>
  {/snippet}

  {#if browser.current}
    {@const cur = browser.current}
    {@const starred = browser.isBookmarked(cur.url)}
    <div class="pagecard">
      <div class="pc-title" title={cur.title || cur.url}>{cur.title || cur.url}</div>
      <div class="pc-meta">
        <span class="pc-domain">{domainOf(cur.url)}</span>
        {#if clips.pathFor(cur.url)}
          <button class="pc-clipped" title={t("agentPanel.openClippedNote")} onclick={() => openClipped(cur.url)}>{t("reader.alreadyClipped")}</button>
        {/if}
        {#if browser.adblock?.state === "ready"}
          <span class="pc-shield" title={t("agentPanel.adblockOn", { n: browser.adblock.rules.toLocaleString() })}>🛡</span>
        {:else if browser.adblock?.state === "failed"}
          <span class="pc-shield err" title={t("agentPanel.adblockFailed", { error: browser.adblock.error })}>🛡</span>
        {/if}
        <span class="pc-nav">
          <button title={t("common.back")} aria-label={t("common.back")} onclick={() => run(api.browserBack())}>←</button>
          <button title={t("common.forward")} aria-label={t("common.forward")} onclick={() => run(api.browserForward())}>→</button>
          <button title={t("agentPanel.reload")} aria-label={t("agentPanel.reload")} onclick={() => run(api.browserReload())}>⟳</button>
          <button
            title={t(cur.docked ? "clipBar.undockHint" : "agentPanel.redock")}
            onclick={() => run(api.browserSetDocked(!cur.docked, cur.label))}
          >{t(cur.docked ? "clipBar.undock" : "clipBar.dock")}</button>
          <button
            class:starred
            title={t(starred ? "clipBar.unstar" : "clipBar.star")}
            aria-label={t(starred ? "clipBar.unstar" : "clipBar.star")}
            onclick={() => run(api.bookmarkToggle(cur.url, cur.title))}
          >
            <StarIcon filled={starred} size={13} />
          </button>
          <button title={t("reader.openInSystemBrowser")} onclick={() => run(api.urlOpenSystem(cur.url))}>↗</button>
          <button class="danger" title={t("agentPanel.closeBrowser")} aria-label={t("agentPanel.closeBrowser")} onclick={() => run(api.browserClose())}>✕</button>
        </span>
      </div>
      <div class="pc-actions">
        <button
          class="act target"
          title={clipTarget.path
            ? t("agentPanel.selectionToTargetHint", { path: `${clipTarget.path}${clipTarget.heading ? ` › ${clipTarget.heading}` : ""}`, key: k("Alt") })
            : t("agentPanel.selectionDefaultHint", { key: k("Alt") })}
          onclick={(e) => void cmd(e.altKey ? "clip-selection-to-note" : "clip-selection")}
        >{t("reader.selectionToTarget", { name: clipTarget.name ?? t("reader.note") })}</button>
        <button class="act mini" title={t("reader.pickTargetHint")} onclick={() => void cmd("clip-selection-to-note")}>▾</button>
        {#if clipTarget.path}
          <button class="act mini" title={t("reader.clearTargetHint")} onclick={() => clipTarget.clear()}>✕</button>
        {/if}
        <button class="act" title={t("agentPanel.clipPageHint", { key: k("Mod+Shift+S") })} onclick={() => void cmd("clip-page")}>{t("clipBar.clipPage")}</button>
        <button class="act" title={t("agentPanel.clipLinkHint")} onclick={() => void cmd("clip-link")}>{t("clipBar.clipLink")}</button>
        <button class="act primary" disabled={agent.grabbingPage} title={t("agentPanel.attachContextHint")} onclick={attachContext}>
          {agent.grabbingPage ? t("atPicker.extracting") : t("reader.addToContext")}
        </button>
      </div>
      {#if browser.windows.length > 1}
        <div class="pc-windows">
          {#each browser.windows as w (w.label)}
            <button class="win" class:active={w.label === cur.label} title={w.url} onclick={() => run(api.browserFocus(w.label))}>
              {w.title || w.url}
            </button>
          {/each}
        </div>
      {/if}
    </div>
  {/if}

  <AgentChat />
{/if}
</div>

<style>
  .agent {
    display: flex;
    flex-direction: column;
    flex: 1;
    min-height: 0;
  }
  .detached {
    flex: 1;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 10px;
    color: var(--fg-muted);
    font-size: var(--fs-sm);
  }
  .detached-actions {
    display: flex;
    gap: 8px;
  }
  .detached-actions button {
    font-size: var(--fs-xs);
    padding: 4px 10px;
    border: 1px solid var(--border);
    border-radius: 6px;
    color: var(--fg);
  }
  .detached-actions button:hover {
    background: var(--bg-hover);
  }
  /* ---- the session bar: the conversation's gear lever ---- */
  .sesswrap {
    position: relative;
    flex-shrink: 0;
  }
  .sessbar {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 7px 12px;
    border-bottom: 1px solid var(--border);
  }
  .sess-cur {
    flex: 1;
    min-width: 0;
    display: flex;
    align-items: center;
    gap: 6px;
    text-align: left;
    font-size: var(--fs-sm);
    font-weight: 600;
    padding: 3px 8px;
    border-radius: 6px;
  }
  .sess-cur:hover:not(:disabled) {
    background: var(--bg-hover);
  }
  .sess-cur:disabled {
    opacity: 0.6;
    cursor: default;
  }
  .sess-title {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .sess-caret {
    color: var(--fg-muted);
    flex-shrink: 0;
    display: flex;
    align-items: center;
    transition: transform 0.15s ease;
  }
  .sess-caret.open {
    transform: rotate(180deg);
  }
  .sess-cur:hover:not(:disabled) .sess-caret {
    color: var(--fg);
  }
  .sess-new {
    flex-shrink: 0;
    font-size: var(--fs-xs);
    color: var(--accent);
    border: 1px solid var(--accent);
    border-radius: 6px;
    padding: 2px 9px;
  }
  .sess-new:hover:not(:disabled) {
    background: var(--bg-hover);
  }
  .sess-new:disabled {
    color: var(--fg-faint);
    border-color: var(--border);
    cursor: default;
  }
  .sess-backdrop {
    position: fixed;
    inset: 0;
    z-index: 29;
    cursor: default;
  }
  .sess-pop {
    position: absolute;
    top: 100%;
    left: 8px;
    right: 8px;
    z-index: 30;
    background: var(--bg);
    border: 1px solid var(--border);
    border-radius: 8px;
    box-shadow: 0 8px 24px rgba(0, 0, 0, 0.12);
    max-height: 40vh;
    overflow-y: auto;
    padding: 4px;
    display: flex;
    flex-direction: column;
    gap: 1px;
  }
  .sess-empty {
    font-size: var(--fs-xs);
    color: var(--fg-faint);
    padding: 10px 8px;
  }
  .sess-row {
    display: flex;
    align-items: center;
    gap: 4px;
    min-width: 0;
    border-radius: 6px;
  }
  .sess-rename {
    flex: 1;
    min-width: 0;
    font-size: var(--fs-sm);
    padding: 5px 8px;
  }
  .sess-row.active {
    background: var(--bg-hover);
  }
  .sess-row:hover {
    background: var(--bg-hover);
  }
  .sess-main {
    flex: 1;
    min-width: 0;
    display: flex;
    align-items: baseline;
    gap: 8px;
    text-align: left;
    padding: 5px 8px;
    border-radius: 6px;
  }
  .sess-check {
    flex-shrink: 0;
    color: var(--accent);
    font-size: var(--fs-xs);
  }
  .sess-t {
    flex: 1;
    min-width: 0;
    font-size: var(--fs-sm);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  /* The ⑂ fork mark — a forked row shares its title with the source session and is told
     apart by it. At the same font size the glyph is naturally small and floats above the
     baseline, so it is enlarged a step and pushed back down; as its own flex item it takes
     the row's gap and does not sit against the title (the negative margin cancels part of
     the 8px gap, leaving about 4px visually) */
  .sess-fork {
    flex-shrink: 0;
    align-self: center;
    font-size: 15px;
    line-height: 1;
    color: var(--fg-faint);
    margin-right: -4px;
  }
  .sess-m {
    flex-shrink: 0;
    font-size: 10px;
    font-family: var(--font-mono);
    color: var(--fg-faint);
  }
  .sess-del {
    flex-shrink: 0;
    color: var(--fg-faint);
    font-size: var(--fs-xs);
    padding: 3px 7px;
    border-radius: 5px;
    visibility: hidden;
  }
  .sess-row:hover .sess-del,
  .sess-del.confirm {
    visibility: visible;
  }
  .sess-del:hover {
    color: var(--danger);
  }
  .sess-del.confirm {
    color: var(--danger);
    border: 1px solid var(--danger);
    white-space: nowrap;
  }
  .sess-legacy-toggle {
    text-align: left;
    font-size: var(--fs-xs);
    color: var(--fg-faint);
    padding: 6px 8px;
    border-top: 1px solid var(--border);
    margin-top: 3px;
    border-radius: 0;
  }
  .sess-legacy-toggle:hover {
    color: var(--fg-muted);
  }
  .sess-row.legacy {
    opacity: 0.65;
  }
  .sess-model {
    flex-shrink: 0;
    font-size: 10px;
    font-family: var(--font-mono);
    color: var(--fg-faint);
    max-width: 110px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  /* The search switch is on but the endpoint does not support it: the one runtime sign of the silent degradation (the same colour as "unverified") */
  .sess-nosearch {
    flex-shrink: 0;
    font-size: 10px;
    color: var(--warning, #b58900);
    border: 1px solid currentColor;
    border-radius: 4px;
    padding: 0 4px;
    white-space: nowrap;
    cursor: help;
  }
  /* ---- the current page card: the browsing state is the conversation's context ---- */
  .pagecard {
    border-bottom: 1px solid var(--border);
    padding: 10px 12px;
    display: flex;
    flex-direction: column;
    gap: 8px;
    background: var(--bg);
    flex-shrink: 0;
  }
  .pc-title {
    font-size: var(--fs-sm);
    font-weight: 600;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .pc-meta {
    display: flex;
    align-items: center;
    gap: 6px;
    font-size: var(--fs-xs);
    color: var(--fg-faint);
    min-width: 0;
  }
  .pc-domain {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .pc-clipped {
    color: var(--accent);
    border: 1px solid var(--accent);
    border-radius: 4px;
    padding: 0 5px;
    font-size: 10px;
    flex-shrink: 0;
  }
  .pc-shield {
    flex-shrink: 0;
  }
  .pc-shield.err {
    color: var(--danger);
  }
  .pc-nav {
    margin-left: auto;
    display: flex;
    gap: 2px;
    flex-shrink: 0;
  }
  .pc-nav button {
    min-width: 22px;
    height: 22px;
    padding: 0 4px;
    display: grid;
    place-items: center;
    border-radius: 5px;
    color: var(--fg-muted);
    font-size: var(--fs-xs);
  }
  .pc-nav button:hover {
    background: var(--bg-hover);
    color: var(--fg);
  }
  .pc-nav button.danger:hover {
    color: var(--danger);
  }
  .pc-nav button.starred {
    color: var(--accent);
  }
  .pc-actions {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
  }
  .act {
    font-size: var(--fs-xs);
    padding: 3px 9px;
    border: 1px solid var(--border);
    border-radius: 6px;
    color: var(--fg-muted);
  }
  .act:hover {
    background: var(--bg-hover);
    color: var(--fg);
  }
  .act.primary {
    color: var(--accent);
    border-color: var(--accent);
  }
  .act.target {
    max-width: 150px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .act.mini {
    padding: 3px 5px;
  }
  .act.primary:disabled {
    opacity: 0.6;
  }
  .pc-windows {
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .win {
    text-align: left;
    font-size: var(--fs-xs);
    color: var(--fg-muted);
    padding: 3px 8px;
    border-radius: 5px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .win:hover,
  .win.active {
    background: var(--bg-hover);
    color: var(--fg);
  }
</style>
