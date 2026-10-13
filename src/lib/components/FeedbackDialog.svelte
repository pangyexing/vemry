<script lang="ts">
  // The feedback dialog (stores/feedback, feedback.rs): a kind, the text, where a reply
  // goes, and the diagnostics listed line by line before anything is sent. The command,
  // the help menu and the settings page all open this one surface. A failed send keeps
  // the draft and offers the same feedback as a mail in the system's mail program.
  import { api, type FeedbackDiagnostics, type FeedbackKind } from "$lib/api";
  import { i18n, t, type MessageKey } from "$lib/i18n/i18n.svelte";
  import { k } from "$lib/shortcut";
  import { account } from "$lib/stores/account.svelte";
  import { feedback, FEEDBACK_MAX } from "$lib/stores/feedback.svelte";
  import { toast } from "$lib/stores/toast.svelte";
  import { tick } from "svelte";

  const KINDS: FeedbackKind[] = ["bug", "idea", "question"];
  const DIAG_ROWS: [MessageKey, keyof FeedbackDiagnostics][] = [
    ["feedback.diag.version", "app_version"],
    ["feedback.diag.os", "os"],
    ["feedback.diag.lang", "lang"],
    ["feedback.diag.model", "model"],
  ];
  /** The counter appears this close to the limit, not before. */
  const COUNT_FROM = FEEDBACK_MAX - 500;

  let root = $state<HTMLElement | null>(null);
  let textarea = $state<HTMLTextAreaElement | null>(null);
  let diag = $state<FeedbackDiagnostics | null>(null);
  let sending = $state(false);
  let error = $state<string | null>(null);

  const num = $derived(new Intl.NumberFormat(i18n.locale));
  // `maxlength` counts UTF-16 units, at least as many as the characters the server counts
  const canSend = $derived(!sending && feedback.text.trim().length > 0 && feedback.text.length <= FEEDBACK_MAX);

  /** Each opening reads the diagnostics afresh — the model or the language may have
   *  changed since — and puts the caret in the text. */
  $effect(() => {
    if (!feedback.open) return;
    error = null;
    api.feedbackDiagnostics().then((d) => (diag = d), () => (diag = null));
    void tick().then(() => textarea?.focus());
  });

  async function send() {
    if (!canSend) return;
    sending = true;
    error = null;
    try {
      const email = account.loggedIn ? null : feedback.email.trim() || null;
      await api.feedbackSend(feedback.kind, feedback.text, email, feedback.diagnostics);
      feedback.sent();
      toast.push(t("feedback.sent"));
    } catch (e) {
      // Closed while it was on its way: the dialog is not there to show the reason
      if (feedback.open) error = String(e);
      else toast.push(String(e), "error");
    } finally {
      sending = false;
    }
  }

  function mailInstead() {
    api.feedbackMailOpen(feedback.kind, feedback.text, feedback.diagnostics).catch((e) => (error = String(e)));
  }

  /** Capture phase, so the open dialog answers before the settings modal (which can open
   *  it) or the editor sees the key. Escape closes and keeps the draft; ⌘/Ctrl+Enter sends
   *  while the focus is inside the dialog. */
  function onkeydown(e: KeyboardEvent) {
    if (!feedback.open) return;
    if (e.key === "Escape") {
      e.stopPropagation();
      feedback.close();
    } else if (e.key === "Enter" && (e.metaKey || e.ctrlKey) && root?.contains(e.target as Node)) {
      e.preventDefault();
      e.stopPropagation();
      void send();
    }
  }
</script>

<svelte:window onkeydowncapture={onkeydown} />

{#if feedback.open}
  <div class="overlay" role="presentation">
    <div class="dialog" role="dialog" aria-modal="true" aria-labelledby="feedback-title" bind:this={root}>
      <h2 id="feedback-title">{t("feedback.title")}</h2>
      <p class="lede">{t("feedback.lede")}</p>

      <div class="kinds" role="radiogroup" aria-label={t("feedback.kind")}>
        {#each KINDS as kind (kind)}
          <button
            class="kind"
            role="radio"
            aria-checked={feedback.kind === kind}
            class:on={feedback.kind === kind}
            onclick={() => (feedback.kind = kind)}
          >
            {t(`feedback.kind.${kind}` as MessageKey)}
          </button>
        {/each}
      </div>

      <div class="text">
        <textarea
          bind:this={textarea}
          bind:value={feedback.text}
          aria-label={t("feedback.text")}
          placeholder={t(`feedback.placeholder.${feedback.kind}` as MessageKey)}
          maxlength={FEEDBACK_MAX}
          rows="7"
          oninput={() => (error = null)}
        ></textarea>
        {#if feedback.text.length > COUNT_FROM}
          <small class="count">{num.format(feedback.text.length)} / {num.format(FEEDBACK_MAX)}</small>
        {/if}
      </div>

      {#if account.loggedIn}
        <p class="reply">{t("feedback.replySignedIn", { email: account.me?.email ?? "" })}</p>
      {:else}
        <label class="field">
          <span>{t("feedback.replyTo")}</span>
          <input
            type="email"
            bind:value={feedback.email}
            placeholder={t("account.emailPlaceholder")}
            autocomplete="email"
            spellcheck="false"
            oninput={() => (error = null)}
          />
          <small>{t("feedback.replyHint")}</small>
        </label>
      {/if}

      <div class="diag" class:off={!feedback.diagnostics}>
        <label class="check">
          <input type="checkbox" bind:checked={feedback.diagnostics} />
          <span>{t("feedback.diagnostics")}</span>
          <small>{t("feedback.diagnosticsHint")}</small>
        </label>
        <dl>
          {#each DIAG_ROWS as [label, field] (field)}
            <dt>{t(label)}</dt>
            <dd>{diag?.[field] || "…"}</dd>
          {/each}
        </dl>
      </div>

      {#if error}
        <p class="err" role="alert">
          {error}
          <button class="link" onclick={mailInstead}>{t("feedback.mailInstead")}</button>
        </p>
      {/if}

      <footer>
        <button class="btn" onclick={() => feedback.close()}>{t("common.cancel")}</button>
        <button class="btn primary" disabled={!canSend} title={k("Mod+Enter")} onclick={() => void send()}>
          {sending ? t("feedback.sending") : t("feedback.send")}
        </button>
      </footer>
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
    /* Above the settings modal (150): its "send feedback" button opens this */
    z-index: 200;
  }
  .dialog {
    width: min(500px, 92vw);
    max-height: 90vh;
    overflow: auto;
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
  .lede,
  .reply {
    font-size: var(--fs-sm);
    line-height: 1.6;
    color: var(--fg-muted);
    margin-top: -4px;
    word-break: break-word;
  }
  .reply {
    margin-top: 0;
  }
  .kinds {
    display: flex;
    gap: 6px;
  }
  .kind {
    font-size: var(--fs-sm);
    padding: 5px 12px;
    border-radius: 6px;
    border: 1px solid var(--border);
    color: var(--fg-muted);
  }
  .kind:hover {
    background: var(--bg-hover);
  }
  .kind.on {
    background: var(--bg-active);
    border-color: var(--border-strong);
    color: var(--fg);
  }
  .text {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  textarea {
    width: 100%;
    resize: vertical;
    min-height: 120px;
    padding: 8px 10px;
    font-size: var(--fs-sm);
    line-height: 1.6;
  }
  .count {
    align-self: flex-end;
    font-size: var(--fs-xs);
    color: var(--fg-muted);
    font-variant-numeric: tabular-nums;
  }
  .field {
    display: flex;
    flex-direction: column;
    gap: 6px;
    font-size: var(--fs-sm);
  }
  .field input {
    padding: 7px 10px;
    font-size: var(--fs-sm);
  }
  small {
    font-size: var(--fs-xs);
    color: var(--fg-muted);
    line-height: 1.6;
  }
  .diag {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .check {
    display: flex;
    align-items: baseline;
    flex-wrap: wrap;
    gap: 4px 8px;
    font-size: var(--fs-sm);
    cursor: pointer;
  }
  .check input {
    margin: 0;
    align-self: center;
  }
  dl {
    display: grid;
    grid-template-columns: max-content 1fr;
    gap: 2px 12px;
    margin: 0 0 0 22px;
    padding: 8px 10px;
    border: 1px solid var(--border);
    border-radius: 6px;
    font-size: var(--fs-xs);
    line-height: 1.6;
    -webkit-user-select: text;
    user-select: text;
  }
  dt {
    color: var(--fg-muted);
  }
  dd {
    margin: 0;
    font-family: var(--font-mono);
    word-break: break-word;
  }
  /* Unticked, the lines stay visible (what would have gone) but read as not included */
  .diag.off dl {
    opacity: 0.5;
  }
  .err {
    font-size: var(--fs-sm);
    line-height: 1.6;
    color: var(--danger);
    word-break: break-word;
  }
  .link {
    color: var(--accent);
    font-size: var(--fs-sm);
    padding: 0;
    margin-left: 4px;
    white-space: nowrap;
  }
  .link:hover {
    text-decoration: underline;
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
    white-space: nowrap;
  }
  .btn.primary {
    background: var(--accent);
    border-color: var(--accent);
    color: var(--bg);
  }
  .btn:disabled {
    opacity: 0.5;
    cursor: default;
  }
</style>
