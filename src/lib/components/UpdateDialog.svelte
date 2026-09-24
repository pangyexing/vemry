<script lang="ts">
  // The one surface of in-app updates: checking, the new version with its three
  // answers (skip / later / install), the download's progress, and the restart. The
  // store (updater.svelte.ts) owns every transition; this only renders the phase.
  import { t } from "$lib/i18n/i18n.svelte";
  import { updater } from "$lib/stores/updater.svelte";
  import icon from "../../../src-tauri/icons/128x128.png";

  const mb = (bytes: number) => `${(bytes / 1048576).toFixed(1)} MB`;
  const pct = $derived(updater.total ? Math.min(100, Math.round((updater.progress / updater.total) * 100)) : null);

  let root = $state<HTMLElement | null>(null);
  /** Opening takes the focus (the overlay blocks the pointer, so the keys must not keep
   *  going to whatever had it — the editor, the settings modal); the dialog root is the
   *  focus holder, so Enter means the primary answer until a button is tabbed to. */
  $effect(() => {
    if (updater.open) root?.focus();
  });

  /** Capture phase, so an open dialog answers Escape/Enter before the settings modal's
   *  own Escape (a window listener registered earlier) or the editor sees the key.
   *  Enter is the primary answer only while the focus is inside the dialog and not on
   *  a button: a focused Skip / Later keeps its own Enter (the native click), and a key
   *  typed anywhere else never installs or restarts. */
  function onkeydown(e: KeyboardEvent) {
    if (!updater.open) return;
    if (e.key === "Escape") {
      e.stopPropagation();
      // During a download this is "continue in background"; the outcome arrives as a toast
      updater.close();
    } else if (e.key === "Enter") {
      const target = e.target as HTMLElement | null;
      if (!root || !target || !root.contains(target) || target.tagName === "BUTTON") return;
      const p = updater.phase;
      if (p === "checking" || p === "downloading") return;
      e.preventDefault();
      e.stopPropagation();
      if (p === "available") void updater.downloadAndInstall();
      else if (p === "ready") void updater.restart();
      else updater.close();
    }
  }
</script>

<svelte:window onkeydowncapture={onkeydown} />

{#if updater.open}
  <div class="overlay" role="presentation">
    <div class="dialog" role="dialog" aria-labelledby="update-title" tabindex="-1" bind:this={root}>
      <img class="icon" src={icon} alt="" width="56" height="56" draggable="false" />
      <div class="body">
        {#if updater.phase === "checking"}
          <h2 id="update-title">{t("updater.checkingTitle")}</h2>
          <div class="bar indeterminate" role="progressbar" aria-label={t("updater.checkingTitle")}><div class="fill"></div></div>
          <footer>
            <button class="btn" onclick={() => updater.close()}>{t("common.cancel")}</button>
          </footer>
        {:else if updater.phase === "available"}
          <h2 id="update-title">{t("updater.availableTitle")}</h2>
          <p>{t("updater.availableBody", { version: updater.version, current: updater.current })}</p>
          {#if updater.notes}
            <div class="notes" aria-label={t("updater.notes")}>{updater.notes}</div>
          {/if}
          <label class="auto">
            <input type="checkbox" checked={updater.autoCheck} onchange={(e) => updater.setAutoCheck(e.currentTarget.checked)} />
            <span>{t("updater.autoCheck")}</span>
          </label>
          <footer>
            <button class="btn" onclick={() => updater.skip()}>{t("updater.skip")}</button>
            <span class="spacer"></span>
            <button class="btn" onclick={() => updater.close()}>{t("updater.later")}</button>
            <button class="btn primary" onclick={() => updater.downloadAndInstall()}>{t("updater.install")}</button>
          </footer>
        {:else if updater.phase === "downloading"}
          <h2 id="update-title">{t("updater.downloadingTitle", { version: updater.version })}</h2>
          <p>{t("updater.downloadingBody")}</p>
          <div
            class="bar"
            class:indeterminate={pct === null}
            role="progressbar"
            aria-valuemin="0"
            aria-valuemax="100"
            aria-valuenow={pct ?? undefined}
          >
            <div class="fill" style:width={pct === null ? undefined : `${pct}%`}></div>
          </div>
          <footer>
            <small class="meter">{mb(updater.progress)}{updater.total ? ` / ${mb(updater.total)}` : ""}</small>
            <span class="spacer"></span>
            <button class="btn" onclick={() => updater.close()}>{t("updater.background")}</button>
          </footer>
        {:else if updater.phase === "ready"}
          <h2 id="update-title">{t("updater.readyTitle")}</h2>
          <p>{t("updater.readyBody", { version: updater.version })}</p>
          <footer>
            <button class="btn" onclick={() => updater.close()}>{t("updater.restartLater")}</button>
            <button class="btn primary" onclick={() => updater.restart()}>{t("updater.restartNow")}</button>
          </footer>
        {:else if updater.phase === "uptodate"}
          <h2 id="update-title">{t("updater.upToDateTitle")}</h2>
          <p>{t("updater.upToDateBody", { current: updater.current })}</p>
          <footer>
            <button class="btn primary" onclick={() => updater.close()}>{t("common.ok")}</button>
          </footer>
        {:else if updater.phase === "error"}
          <h2 id="update-title">{t(updater.errorStage === "download" ? "updater.installFailedTitle" : "updater.checkFailedTitle")}</h2>
          <p class="err">{updater.error}</p>
          <footer>
            <button class="btn primary" onclick={() => updater.close()}>{t("common.ok")}</button>
          </footer>
        {/if}
      </div>
    </div>
  </div>
{/if}

<style>
  .overlay {
    position: fixed;
    inset: 0;
    background: var(--scrim);
    display: grid;
    place-items: center;
    /* Above the settings modal (150): its "check for updates" button opens this */
    z-index: 200;
  }
  .dialog {
    outline: none;
    width: min(460px, 90vw);
    background: var(--bg-panel);
    border: 1px solid var(--border);
    border-radius: 10px;
    box-shadow: var(--shadow);
    padding: 18px 18px 16px;
    display: flex;
    gap: 14px;
    align-items: flex-start;
  }
  .icon {
    flex-shrink: 0;
    width: 56px;
    height: 56px;
    -webkit-user-select: none;
    user-select: none;
  }
  .body {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 10px;
  }
  h2 {
    font-size: var(--fs-md);
    font-weight: 600;
    line-height: 1.4;
    margin-top: 2px;
  }
  p {
    font-size: var(--fs-sm);
    line-height: 1.6;
    color: var(--fg-muted);
    word-break: break-word;
  }
  p.err {
    color: var(--danger);
    word-break: break-all;
  }
  .notes {
    max-height: 160px;
    overflow: auto;
    padding: 8px 10px;
    border: 1px solid var(--border);
    border-radius: 6px;
    font-size: var(--fs-xs);
    line-height: 1.6;
    white-space: pre-wrap;
    word-break: break-word;
    -webkit-user-select: text;
    user-select: text;
  }
  .auto {
    display: flex;
    align-items: center;
    gap: 8px;
    font-size: var(--fs-sm);
    cursor: pointer;
  }
  .auto input {
    margin: 0;
  }
  .bar {
    height: 6px;
    border-radius: 3px;
    background: var(--bg-hover);
    overflow: hidden;
  }
  .bar .fill {
    height: 100%;
    width: 0;
    border-radius: 3px;
    background: var(--accent);
    transition: width 0.15s linear;
  }
  /* No Content-Length (or the check itself): a sweeping band instead of a percentage */
  .bar.indeterminate .fill {
    width: 35%;
    animation: sweep 1.2s ease-in-out infinite;
  }
  @keyframes sweep {
    from {
      transform: translateX(-100%);
    }
    to {
      transform: translateX(300%);
    }
  }
  footer {
    display: flex;
    align-items: center;
    justify-content: flex-end;
    gap: 8px;
    margin-top: 2px;
  }
  .spacer {
    flex: 1;
  }
  .meter {
    font-size: var(--fs-xs);
    color: var(--fg-muted);
    font-variant-numeric: tabular-nums;
  }
  .btn {
    font-size: var(--fs-sm);
    padding: 6px 14px;
    border-radius: 6px;
    border: 1px solid var(--border);
    white-space: nowrap;
  }
  .btn.primary {
    background: var(--accent);
    border-color: var(--accent);
    color: var(--bg);
  }
</style>
