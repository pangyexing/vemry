<script lang="ts">
  import { onDestroy, untrack } from "svelte";
  import { api, type CaptureTarget, type InboxDoor, type InboxLanding, type PairStart } from "$lib/api";
  import { comboFromEvent, hotkeyLabel, isModifierKey } from "$lib/capture";
  import { copyText } from "$lib/copytext";
  import { i18n, t } from "$lib/i18n/i18n.svelte";
  import { k } from "$lib/shortcut";
  import { account } from "$lib/stores/account.svelte";
  import { CAPTURE_HOTKEY_DEFAULT } from "$lib/stores/prefs.svelte";
  import { openSettings } from "$lib/stores/settingsUi.svelte";
  import { toast } from "$lib/stores/toast.svelte";
  import { vault } from "$lib/stores/vault.svelte";
  import { rowControl, settingsSearch } from "./context";
  import { form } from "./form.svelte";
  import SettingRow from "./SettingRow.svelte";
  import SettingSection from "./SettingSection.svelte";
  import SettingText from "./SettingText.svelte";

  const s = $derived(form.s);
  const search = settingsSearch();
  const searching = $derived(!!search?.query);
  let alive = true;

  // ---- the shortcut recorder: the field takes the next combination pressed (a modifier
  // plus a key; Esc keeps the current one). The grammar and the "usable shortcut" rule
  // live in capture.ts; Rust registers it when it is written and refuses one another app
  // holds, and the refusal shows under the row.
  //
  // The keys are taken at the window, in the capture phase, for as long as the field is
  // armed — not by a handler on the field: a mouse click does not focus a button on macOS
  // WebKit, so a keydown handler there would never see the press. Capture phase plus
  // stopPropagation also keeps the combination from reaching the editor and the dialog's
  // own Escape.
  let recording = $state(false);
  let recordingError = $state<string | null>(null);
  $effect(() => {
    if (!recording) return;
    window.addEventListener("keydown", recordHotkey, true);
    return () => window.removeEventListener("keydown", recordHotkey, true);
  });
  function recordHotkey(e: KeyboardEvent) {
    e.preventDefault();
    e.stopPropagation();
    if (e.key === "Escape") {
      recording = false;
      return;
    }
    if (isModifierKey(e)) return; // the key is still coming
    const combo = comboFromEvent(e);
    if (combo === null) {
      recordingError = t("settings.capture.hotkeyNeedsModifier");
      return;
    }
    recording = false;
    recordingError = null;
    void setHotkey(combo);
  }
  const setHotkey = (combo: string) => form.set({ capture_hotkey: combo }, "capture.hotkey");
  const hotkeyError = $derived.by(() => {
    if (recordingError) return recordingError;
    const e = form.errors["capture.hotkey"] ?? s?.capture_hotkey_error;
    return e ? t("settings.capture.hotkeyFailed", { error: e }) : null;
  });

  // ---- the phone inbox: the pairing QR code on show (null = none), the paired phones, the last failure
  let phonePair = $state<PairStart | null>(null);
  let phonePairing = $state(false);
  let phoneDoors = $state<InboxDoor[]>([]);
  let phoneError = $state<string | null>(null);
  /** What "retry" after the failure does: show a code again, or read the list again. */
  let phoneRetry: (() => Promise<void>) | null = null;
  /** The phone that just claimed the code: the card is a tick for three seconds, then folds. */
  let phoneJustPaired = $state<InboxDoor | null>(null);
  /** The code's remaining time, m:ss, ticking every second while a code is on show. */
  let phoneCodeLeft = $state("");
  let phoneCopied = $state(false);
  const inboxLastLanded = $derived(s?.inbox_last_landed ?? null);
  let phoneWatch: ReturnType<typeof setInterval> | null = null;
  let phoneTick: ReturnType<typeof setInterval> | null = null;
  let phoneOkTimer: ReturnType<typeof setTimeout> | null = null;
  const inboxVaultChoices = $derived.by(() => {
    const recent = s?.recent_vaults ?? [];
    const cur = s?.inbox_vault ?? "";
    return cur && !recent.includes(cur) ? [cur, ...recent] : recent;
  });

  async function showPairCode() {
    phonePairing = true;
    phoneError = null;
    phoneCopied = false;
    try {
      // The phones paired before this code, read fresh: a list still loading (or failed) would make
      // every phone already paired look like the one that just claimed the code
      await loadDoors();
      if (phoneError) {
        phoneRetry = showPairCode;
        return;
      }
      const before = new Set(phoneDoors.map((d) => d.id));
      const pair = await api.inboxPairStart();
      // The page was left while the code was on its way: no card to show it on, and no watch to start
      // (a watch nothing stops would poll, and renew the code at every expiry, for the life of the app)
      if (!alive) return;
      phonePair = pair;
      // Showing the first code fixes the landing vault to the open one (inbox.rs): read it back
      void form.load();
      // Until the code is claimed or expires, the paired phones are polled: a new one turns the
      // card into a tick, and an expiry while the page is still open asks for a fresh code
      const until = new Date(pair.expires_at).getTime();
      stopPhoneWatch();
      phoneWatch = setInterval(async () => {
        await loadDoors();
        const fresh = phoneDoors.find((d) => !before.has(d.id));
        if (fresh) {
          stopPhoneWatch();
          phonePair = null;
          phoneJustPaired = fresh;
          phoneOkTimer = setTimeout(() => (phoneJustPaired = null), 3000);
        } else if (Number.isNaN(until)) {
          stopPhoneWatch();
          phonePair = null;
        } else if (Date.now() > until) {
          stopPhoneWatch();
          void showPairCode();
        }
      }, 4000);
      phoneCodeLeft = countdown(until);
      phoneTick = setInterval(() => (phoneCodeLeft = countdown(until)), 1000);
    } catch (e) {
      phonePair = null;
      phoneError = e instanceof Error ? e.message : String(e);
      phoneRetry = showPairCode;
    } finally {
      phonePairing = false;
    }
  }
  function countdown(until: number): string {
    const sec = Math.max(0, Math.round((until - Date.now()) / 1000));
    return `${Math.floor(sec / 60)}:${String(sec % 60).padStart(2, "0")}`;
  }
  function stopPhoneWatch() {
    if (phoneWatch) clearInterval(phoneWatch);
    if (phoneTick) clearInterval(phoneTick);
    phoneWatch = null;
    phoneTick = null;
  }
  /** The code as a link, for a phone that cannot scan: sent to it any way at all, it pairs on opening. */
  async function copyPairLink() {
    if (!phonePair) return;
    phoneCopied = await copyText(phonePair.url);
    if (phoneCopied) setTimeout(() => (phoneCopied = false), 2000);
  }
  async function loadDoors() {
    if (!account.loggedIn) {
      phoneDoors = [];
      return;
    }
    try {
      phoneDoors = await api.inboxDoors();
    } catch (e) {
      phoneError = e instanceof Error ? e.message : String(e);
      phoneRetry = loadDoors;
    }
  }
  async function revokeDoor(id: number) {
    try {
      await api.inboxDoorRevoke(id);
      phoneDoors = phoneDoors.filter((d) => d.id !== id);
      toast.push(t("settings.capture.phoneRevoked"));
    } catch (e) {
      phoneError = e instanceof Error ? e.message : String(e);
      phoneRetry = loadDoors;
    }
  }
  function retryPhone() {
    phoneError = null;
    void (phoneRetry ?? loadDoors)();
  }
  function kindLabel(d: InboxDoor): string {
    return d.kind === "pwa" ? t("settings.capture.kindPwa") : d.kind === "shortcut" ? t("settings.capture.kindShortcut") : t("settings.capture.kindOther");
  }
  /** The phone's own name ("Android · 微信"); its kind when it gave none. */
  const doorName = (d: InboxDoor) => d.name || kindLabel(d);
  const monthDay = $derived(new Intl.DateTimeFormat(i18n.locale, { month: "long", day: "numeric" }));
  const yearMonthDay = $derived(new Intl.DateTimeFormat(i18n.locale, { year: "numeric", month: "long", day: "numeric" }));
  const relative = $derived(new Intl.RelativeTimeFormat(i18n.locale, { numeric: "auto" }));
  /** "today", or the day (with its year when it is not this year's). */
  function dateLabel(iso: string): string {
    const d = new Date(iso);
    if (Number.isNaN(d.getTime())) return iso;
    const now = new Date();
    if (d.toDateString() === now.toDateString()) return t("settings.capture.today");
    return (d.getFullYear() === now.getFullYear() ? monthDay : yearMonthDay).format(d);
  }
  /** "just now", "2 minutes ago", "yesterday". */
  function ago(iso: string): string {
    const ms = Date.now() - new Date(iso).getTime();
    if (Number.isNaN(ms)) return "";
    const min = Math.round(ms / 60000);
    if (min < 1) return t("settings.capture.justNow");
    if (min < 60) return relative.format(-min, "minute");
    const h = Math.round(min / 60);
    if (h < 24) return relative.format(-h, "hour");
    return relative.format(-Math.round(h / 24), "day");
  }
  /** "paired Sep 21 · sent 7 minutes ago": when the door was made, and its last drop (the service's last_seen_at). */
  function doorWhen(d: InboxDoor): string {
    const since = t("settings.capture.phoneDoorSince", { date: dateLabel(d.created_at) });
    const seen = d.last_seen_at ? t("settings.capture.phoneDoorSeen", { ago: ago(d.last_seen_at) }) : t("settings.capture.phoneDoorNever");
    return `${since} · ${seen}`;
  }
  /** "Last one filed 7 minutes ago in inbox.md", naming the vault when it was not the open one. */
  function lastLandedLine(l: InboxLanding): string {
    return l.vault
      ? t("settings.capture.phoneLastLandedIn", { ago: ago(l.at), vault: l.vault, target: l.target })
      : t("settings.capture.phoneLastLanded", { ago: ago(l.at), target: l.target });
  }
  // The page opened (not a search's glance at it), or the account signed in: the paired
  // phones are read — once, not again for every setting written on the page
  $effect(() => {
    if (searching || !account.loggedIn) return;
    untrack(() => void loadDoors());
  });
  onDestroy(() => {
    alive = false;
    recording = false;
    stopPhoneWatch();
    if (phoneOkTimer) clearTimeout(phoneOkTimer);
  });
</script>

{#if s}
  <SettingSection title={t("settings.sec.captureWindow")}>
    <SettingRow id="capture.hotkey" error={hotkeyError}>
      {#if s.capture_hotkey !== CAPTURE_HOTKEY_DEFAULT}
        <button class="link" onclick={() => void setHotkey(CAPTURE_HOTKEY_DEFAULT)}>{t("settings.capture.hotkeyReset")}</button>
      {/if}
      <!-- A button, not an input: the keys pressed are the value, and nothing may be typed into it.
           Focused by hand on click (a mouse click does not focus a button on macOS WebKit), so that
           clicking anywhere else blurs it and disarms the recorder. Already armed: a no-op, so
           Space's keyup activation cannot wipe the "needs a modifier" error. -->
      <button
        {@attach rowControl}
        class="hotkey-field"
        class:recording
        class:bad={!!hotkeyError}
        onclick={(e) => {
          if (recording) return;
          e.currentTarget.focus();
          recording = true;
          recordingError = null;
        }}
        onblur={() => (recording = false)}
      >
        {recording ? t("settings.capture.hotkeyRecording") : hotkeyLabel(s.capture_hotkey)}
      </button>
    </SettingRow>
    <SettingRow id="capture.target">
      <select {@attach rowControl} value={s.capture_target} onchange={(e) => void form.set({ capture_target: e.currentTarget.value as CaptureTarget })}>
        <option value="inbox">{t("settings.capture.targetInbox")}</option>
        <option value="journal">{t("settings.capture.targetJournal")}</option>
        <option value="pinned">{t("settings.capture.targetPinned")}</option>
      </select>
    </SettingRow>
    {#if s.capture_target === "pinned"}
      {#if !vault.root}
        <SettingRow id="capture.pinnedNote" hint={t("settings.capture.pinnedNeedsVault")}></SettingRow>
      {:else}
        <SettingRow id="capture.pinnedNote">
          <SettingText value={s.capture_pinned_path} list="capture-pinned-notes" placeholder="notes/ideas.md" width="220px" patch={(v) => ({ capture_pinned_path: v.trim() })} />
          <datalist id="capture-pinned-notes">
            {#each vault.notes as n (n.id)}
              <option value={n.id}>{n.title}</option>
            {/each}
          </datalist>
        </SettingRow>
        <SettingRow id="capture.pinnedHeading">
          <SettingText value={s.capture_pinned_heading} width="220px" patch={(v) => ({ capture_pinned_heading: v.trim() })} />
        </SettingRow>
      {/if}
    {/if}
  </SettingSection>

  <SettingSection title={t("settings.capture.phone")}>
    <SettingRow id="capture.phone" block hint={account.loggedIn && inboxLastLanded ? `${t("settings.capture.phoneHint")} ${lastLandedLine(inboxLastLanded)}` : undefined}>
      {#if !account.loggedIn}
        <small class="empty">{t("settings.capture.phoneSignIn")} <button class="link" onclick={() => openSettings("account.panel")}>{t("settings.capture.phoneSignInGo")} →</button></small>
      {:else}
        <!-- The pairing card's four states: folded (a button and a hint), a code on show with its countdown,
             just paired (a tick for three seconds), failed (the error and a retry) -->
        {#if phoneJustPaired}
          <div class="pair-ok">
            <svg width="16" height="16" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M3 8.5l3 3 7-7" /></svg>
            <span>{t("settings.capture.phonePairedAs", { name: doorName(phoneJustPaired) })}</span>
          </div>
        {:else if phonePair}
          <div class="pair-card">
            <!-- The QR code is our own SVG (inbox.rs renders it in currentColor); nothing user-written is in it -->
            <div class="qr" title={phonePair.url}>{@html phonePair.svg}</div>
            <div class="pair-steps">
              <b>{t("settings.capture.phoneScanTitle")}</b>
              <ol>
                <li>{t("settings.capture.phoneStep1")}</li>
                <li>{t("settings.capture.phoneStep2")}</li>
                <li>{t("settings.capture.phoneStep3")}</li>
              </ol>
              <small>{t("settings.capture.phoneCodeLeft", { time: phoneCodeLeft })}</small>
              <div class="line">
                <button class="btn small" onclick={copyPairLink}>{phoneCopied ? t("settings.capture.phoneCopied") : t("settings.capture.phoneCopyLink")}</button>
                <button class="btn small" disabled={phonePairing} onclick={showPairCode}>{t("settings.capture.phoneNewCode")}</button>
              </div>
            </div>
          </div>
        {:else}
          <div class="line">
            <button {@attach rowControl} class="btn small" disabled={phonePairing} onclick={showPairCode}>{t("settings.capture.phoneShowCode")}</button>
            <small>{t("settings.capture.phoneCodeHint")}</small>
          </div>
        {/if}
        {#if phoneError}
          <small class="err-text">{phoneError} <button class="link" onclick={retryPhone}>{t("settings.capture.phoneRetry")}</button></small>
        {/if}
        {#if phoneDoors.length}
          <ul class="doors" aria-label={t("settings.capture.phoneDoors")}>
            {#each phoneDoors as d (d.id)}
              <li>
                <span class="door-name">{doorName(d)}{#if d.name}<span class="door-kind">{kindLabel(d)}</span>{/if}</span>
                <button class="link door-del" onclick={() => revokeDoor(d.id)}>{t("settings.capture.phoneRevoke")}</button>
                <small class="door-when">{doorWhen(d)}</small>
              </li>
            {/each}
          </ul>
        {:else}
          <small class="empty">{t("settings.capture.phoneNoDoors")}</small>
        {/if}
      {/if}
    </SettingRow>
    {#if account.loggedIn}
      <SettingRow id="capture.phoneVault">
        <select {@attach rowControl} value={s.inbox_vault ?? ""} onchange={(e) => void form.set({ inbox_vault: e.currentTarget.value })}>
          <option value="">{t("settings.capture.phoneVaultOpen")}</option>
          {#each inboxVaultChoices as root (root)}
            <option value={root} title={root}>{vault.nameOf(root)}</option>
          {/each}
        </select>
      </SettingRow>
    {/if}
  </SettingSection>
{/if}

<style>
  /* The shortcut recorder: a field that reads like an input and takes a key combination instead of text */
  .hotkey-field {
    min-width: 160px;
    padding: 5px 10px;
    border: 1px solid var(--border-strong);
    border-radius: 6px;
    background: var(--bg);
    color: var(--fg);
    font-family: var(--font-mono);
    font-size: var(--fs-sm);
    text-align: left;
  }
  .hotkey-field.recording {
    border-color: var(--accent);
    color: var(--fg-muted);
    font-family: inherit;
  }
  .hotkey-field.bad {
    border-color: var(--danger);
  }
  .line {
    display: flex;
    align-items: center;
    gap: 10px;
    flex-wrap: wrap;
  }
  .err-text {
    color: var(--danger);
  }
  /* The pairing card: the code at the left, the three steps and the countdown at the right */
  .pair-card {
    display: grid;
    grid-template-columns: 148px 1fr;
    gap: 16px;
    align-items: start;
    padding: 14px;
    border: 1px solid var(--border);
    border-radius: var(--radius);
  }
  .qr {
    width: 148px;
    height: 148px;
    color: var(--fg);
  }
  .qr :global(svg) {
    width: 100%;
    height: 100%;
    display: block;
  }
  .pair-steps {
    display: flex;
    flex-direction: column;
    gap: 6px;
    font-size: var(--fs-sm);
  }
  .pair-steps ol {
    margin: 0;
    padding-left: 1.3em;
  }
  .pair-steps li {
    margin: 2px 0;
  }
  .pair-ok {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 8px 12px;
    border-radius: var(--radius);
    background: var(--accent-soft);
    color: var(--accent);
    font-size: var(--fs-sm);
    font-weight: 600;
  }
  /* The paired phones: the phone's own name with its kind faint after it, when it paired and last
     sent below, and unpair always in view — it is the action the list exists for */
  .doors {
    list-style: none;
    margin: 6px 0 0;
    padding: 0;
    display: flex;
    flex-direction: column;
  }
  .doors li {
    display: grid;
    grid-template-columns: 1fr auto;
    gap: 0 12px;
    align-items: center;
    padding: 6px 0;
    border-bottom: 1px solid var(--border);
  }
  .doors li:last-child {
    border-bottom: 0;
  }
  /* Every cell is named: an item locked to a row alone is placed before the free ones and would take the first column */
  .door-name {
    grid-column: 1;
    grid-row: 1;
    font-size: var(--fs-sm);
  }
  .door-kind {
    color: var(--fg-muted);
    font-size: var(--fs-xs);
    margin-left: 6px;
  }
  .door-when {
    grid-column: 1;
    grid-row: 2;
  }
  .door-del {
    grid-column: 2;
    grid-row: 1 / span 2;
    justify-self: end;
    color: var(--fg-muted);
  }
  .door-del:hover {
    color: var(--danger);
  }
</style>
