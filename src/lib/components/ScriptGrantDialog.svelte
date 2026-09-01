<script lang="ts">
  // The blocking half of the script write-permission flow: the script thread
  // is suspended inside vemry.write / vemry.prompt until the user answers
  // (deadline paused). Grant = once per script, revocable in settings.
  import { t } from "$lib/i18n/i18n.svelte";
  import { scripts } from "$lib/stores/scripts.svelte";
  import { isImeKeydown } from "$lib/ime";

  let promptValue = $state("");

  $effect(() => {
    if (scripts.pendingPrompt) promptValue = "";
  });

  function onkeydown(e: KeyboardEvent) {
    if (scripts.pendingGrant && e.key === "Escape") void scripts.respondGrant(false);
    else if (scripts.pendingPrompt && e.key === "Escape") void scripts.respondPrompt(null);
  }
</script>

<svelte:window {onkeydown} />

{#if scripts.pendingGrant}
  {@const req = scripts.pendingGrant}
  <div class="overlay" role="presentation">
    <div class="dialog" role="dialog" aria-label={t("scriptGrant.title")}>
      <h2>{t("scriptGrant.heading")}</h2>
      <p>
        {t("scriptGrant.bodyBefore")}<code>{req.script}</code>{t("scriptGrant.bodyBetween")}<code>{req.path}</code>{t("scriptGrant.bodyAfter")}
      </p>
      <small>
        {t("scriptGrant.noteBefore")}<code>.vemry/script-log.jsonl</code>{t("scriptGrant.noteAfter")}
      </small>
      <footer>
        <button class="btn" onclick={() => scripts.respondGrant(false)}>{t("permission.deny")}</button>
        <button class="btn primary" onclick={() => scripts.respondGrant(true)}>{t("scriptGrant.allowWrite")}</button>
      </footer>
    </div>
  </div>
{:else if scripts.pendingPrompt}
  {@const req = scripts.pendingPrompt}
  <div class="overlay" role="presentation">
    <div class="dialog" role="dialog" aria-label={t("scriptGrant.promptTitle")}>
      <h2>{req.script}</h2>
      <p>{req.message}</p>
      <!-- svelte-ignore a11y_autofocus -->
      <input
        autofocus
        bind:value={promptValue}
        placeholder={req.placeholder ?? ""}
        spellcheck="false"
        onkeydown={(e) => e.key === "Enter" && !isImeKeydown(e) && scripts.respondPrompt(promptValue)}
      />
      <footer>
        <button class="btn" onclick={() => scripts.respondPrompt(null)}>{t("common.cancel")}</button>
        <button class="btn primary" onclick={() => scripts.respondPrompt(promptValue)}>{t("common.ok")}</button>
      </footer>
    </div>
  </div>
{/if}

<style>
  .overlay {
    position: fixed;
    inset: 0;
    background: rgba(0, 0, 0, 0.3);
    display: grid;
    place-items: center;
    z-index: 200;
  }
  .dialog {
    width: min(440px, 90vw);
    background: var(--bg-panel);
    border: 1px solid var(--border);
    border-radius: 10px;
    box-shadow: var(--shadow);
    padding: 20px;
    display: flex;
    flex-direction: column;
    gap: 12px;
  }
  h2 {
    font-size: var(--fs-lg);
    font-weight: 600;
  }
  p {
    font-size: var(--fs-sm);
    line-height: 1.6;
    word-break: break-all;
  }
  code {
    font-family: var(--font-mono);
    font-size: var(--fs-xs);
    background: var(--bg-hover);
    padding: 1px 5px;
    border-radius: 4px;
  }
  small {
    font-size: var(--fs-xs);
    color: var(--fg-faint);
    line-height: 1.6;
  }
  input {
    padding: 7px 10px;
    font-size: var(--fs-sm);
  }
  footer {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
  }
  .btn {
    font-size: var(--fs-sm);
    padding: 6px 14px;
    border-radius: 6px;
    border: 1px solid var(--border);
  }
  .btn.primary {
    background: var(--accent);
    border-color: var(--accent);
    color: #fff;
  }
</style>
