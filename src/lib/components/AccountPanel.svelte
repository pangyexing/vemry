<script lang="ts">
  // The account page of the settings dialog (Settings › Account): signed out, an
  // e-mail address and the 6-digit code it receives; signed in, the entitlements
  // (plan, expiry, "about N papers left", the credit pack, the subscription) and the
  // actions on them. Purchase and subscription management open vemry.app in the
  // system browser — payment never passes through the app. Everything shown is the
  // account store's snapshot; the store follows Rust's account event.
  import { api } from "$lib/api";
  import { i18n, t } from "$lib/i18n/i18n.svelte";
  import { account } from "$lib/stores/account.svelte";
  import { toast } from "$lib/stores/toast.svelte";
  import { isImeKeydown } from "$lib/ime";

  /** Sign-in: the address first, then the code it received. */
  let step = $state<"email" | "code">("email");
  let email = $state("");
  let code = $state("");
  let busy = $state(false);
  let error = $state<string | null>(null);
  let redeemCode = $state("");
  let redeemBusy = $state(false);
  let refreshing = $state(false);

  const me = $derived(account.me);
  const status = $derived(account.status);
  /** Days since the snapshot, for the offline notice while it is stale. */
  const staleDays = $derived.by(() => {
    const at = status?.fetched_at ? Date.parse(status.fetched_at) : NaN;
    if (Number.isNaN(at)) return 0;
    return Math.floor((Date.now() - at) / 86_400_000);
  });

  /** Fen → an amount in the interface's locale with the currency spelled by its code
   *  ("CN¥12.34" in English, "¥12.34" in Chinese, "元12.34" in Japanese): a bare ¥ reads as
   *  yen to a Japanese user. The interface shows money and papers, never tokens. */
  const money = $derived(new Intl.NumberFormat(i18n.locale, { style: "currency", currency: "CNY" }));
  function yuan(fen: number): string {
    return money.format(Math.abs(fen) / 100);
  }
  /** A date the server sent (YYYY-MM-DD or RFC 3339) in the interface's locale; unparseable stays as it came. */
  function day(s: string | null | undefined): string {
    if (!s) return "";
    const d = new Date(s.length === 10 ? `${s}T00:00:00` : s);
    return Number.isNaN(d.getTime()) ? s : d.toLocaleDateString(i18n.locale, { year: "numeric", month: "short", day: "numeric" });
  }
  function planLabel(plan: string): string {
    return plan === "pro" ? t("account.plan.pro") : plan === "founder" ? t("account.plan.founder") : t("account.plan.free");
  }

  async function sendCode() {
    const addr = email.trim();
    if (!addr || busy) return; // Enter while a request is out must not send a second one
    busy = true;
    error = null;
    try {
      await api.accountLoginStart(addr);
      step = "code";
      code = "";
      toast.push(t("account.codeSent", { email: addr }));
    } catch (e) {
      error = String(e);
    } finally {
      busy = false;
    }
  }

  async function verify() {
    const c = code.trim();
    if (c.length < 6 || busy) return;
    busy = true;
    error = null;
    try {
      await api.accountLoginVerify(email.trim(), c);
      step = "email";
      code = "";
    } catch (e) {
      error = String(e);
    } finally {
      busy = false;
    }
  }

  async function signOut() {
    busy = true;
    try {
      await api.accountLogout();
    } finally {
      busy = false;
    }
  }

  /** The wording of a failed action. A 401 has already cleared the account by the time
   *  the error arrives (Rust acts on it before answering), so the panel is showing the
   *  sign-in form again: the relay's "sign in again under Settings › Account" would point
   *  at the page the user is on, and the panel's own line says it. */
  function failed(e: unknown): string {
    return account.loggedIn ? String(e) : t("account.sessionRevoked");
  }

  async function refresh() {
    refreshing = true;
    error = null;
    try {
      await account.refresh();
    } catch (e) {
      error = failed(e);
    } finally {
      refreshing = false;
    }
  }

  async function redeem() {
    const c = redeemCode.trim();
    if (!c || redeemBusy) return;
    redeemBusy = true;
    error = null;
    try {
      await api.accountRedeem(c);
      redeemCode = "";
      toast.push(t("account.redeemed"));
    } catch (e) {
      error = failed(e);
    } finally {
      redeemBusy = false;
    }
  }

  /** The account page on vemry.app (`portal_url` from the snapshot; `#hash` picks a section), in the system browser. */
  function openPortal(hash = "") {
    const base = me?.portal_url || "https://vemry.app/account";
    void api.urlOpenSystem(hash ? `${base}#${hash}` : base).catch((e) => toast.push(String(e), "error"));
  }
</script>

<section class="account">
  <h3>{t("account.title")}</h3>
  {#if !account.loggedIn}
    <p class="note">{t("account.intro")}</p>
    {#if step === "email"}
      <label>
        <span>{t("account.email")}</span>
        <input
          type="email"
          bind:value={email}
          placeholder={t("account.emailPlaceholder")}
          spellcheck="false"
          autocomplete="email"
          onkeydown={(e) => e.key === "Enter" && !isImeKeydown(e) && void sendCode()}
        />
      </label>
      <div class="row">
        <button class="btn primary" disabled={busy || !email.trim()} onclick={() => void sendCode()}>
          {busy ? t("account.sending") : t("account.sendCode")}
        </button>
      </div>
    {:else}
      <small>{t("account.codeSent", { email: email.trim() })}</small>
      <label>
        <span>{t("account.code")}</span>
        <input
          class="code"
          bind:value={code}
          placeholder={t("account.codePlaceholder")}
          inputmode="numeric"
          autocomplete="one-time-code"
          maxlength="6"
          spellcheck="false"
          onkeydown={(e) => e.key === "Enter" && !isImeKeydown(e) && void verify()}
        />
      </label>
      <div class="row">
        <button class="btn primary" disabled={busy || code.trim().length < 6} onclick={() => void verify()}>
          {busy ? t("account.signingIn") : t("account.signIn")}
        </button>
        <button class="btn" disabled={busy} onclick={() => void sendCode()}>{t("account.resend")}</button>
        <button class="btn" disabled={busy} onclick={() => { step = "email"; code = ""; error = null; }}>{t("account.changeEmail")}</button>
      </div>
    {/if}
  {:else if me}
    <div class="who">
      <span>{t("account.signedInAs", { email: me.email })}</span>
      <span class="badge" class:pro={me.plan === "pro" || me.plan === "founder"}>{planLabel(me.plan)}</span>
    </div>
    <ul class="facts">
      {#if me.pro_until}
        <li>{t("account.proUntil", { date: day(me.pro_until) })}</li>
      {/if}
      <li>{me.hosted ? t("account.hostedOn") : t("account.hostedOff")}</li>
      {#if me.quota}
        {@const left = Math.max(0, me.quota.total_fen - me.quota.used_fen)}
        {#if account.studyLeft != null}
          <li class="lead">{t("account.studyLeft", { n: account.studyLeft })}</li>
        {/if}
        <li>
          {t("account.quota", { left: yuan(left), total: yuan(me.quota.total_fen) })}
          {#if me.quota.resets_at}<span class="muted"> · {t("account.quotaResets", { date: day(me.quota.resets_at) })}</span>{/if}
        </li>
      {/if}
      <li class:warn={me.pack_fen < 0}>
        {me.pack_fen < 0 ? t("account.packNegative", { amount: yuan(me.pack_fen) }) : t("account.pack", { amount: yuan(me.pack_fen) })}
      </li>
      {#if me.subscription}
        {@const sub = me.subscription}
        <li class="sub">
          {#if sub.status === "past_due"}
            <span class="warn">{t("account.subscriptionPastDue")}</span>
          {:else if sub.cancel_at_period_end || sub.status === "canceling" || sub.status === "canceled"}
            {t("account.subscriptionCancelled", { date: day(me.pro_until) })}
          {:else}
            {t("account.subscriptionOn", { date: day(sub.next_charge_at), price: sub.price ? ` · ${sub.price}` : "" })}
          {/if}
          <button class="btn small" onclick={() => openPortal("subscription")}>{t("account.manageSubscription")}</button>
        </li>
      {/if}
      {#if me.peak && me.hosted}
        <li class="muted">{t("account.peak")}</li>
      {/if}
    </ul>
    {#if status?.freshness === "expired"}
      <small class="err">{t("account.expired")}</small>
    {:else if status?.freshness === "stale" && staleDays >= 1}
      <small class="warn">{t("account.offlineStale", { days: staleDays })}</small>
    {/if}
    <div class="row">
      <button class="btn primary" onclick={() => openPortal()}>{t("account.buy")}</button>
      <button class="btn" disabled={refreshing} onclick={() => void refresh()}>{refreshing ? t("account.refreshing") : t("account.refresh")}</button>
      <button class="btn" disabled={busy} onclick={() => void signOut()}>{t("account.signOut")}</button>
    </div>
    <small>{t("account.buyHint")}</small>
    <h4>{t("account.redeem")}</h4>
    <div class="row">
      <input
        class="redeem"
        bind:value={redeemCode}
        placeholder={t("account.redeemPlaceholder")}
        spellcheck="false"
        onkeydown={(e) => e.key === "Enter" && !isImeKeydown(e) && void redeem()}
      />
      <button class="btn" disabled={redeemBusy || !redeemCode.trim()} onclick={() => void redeem()}>{t("account.redeemBtn")}</button>
    </div>
  {/if}
  {#if error}<small class="err">{error}</small>{/if}
</section>

<style>
  .account {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  h3 {
    font-size: var(--fs-sm);
    font-weight: 600;
  }
  h4 {
    margin-top: 8px;
    font-size: var(--fs-xs);
    font-weight: 600;
    color: var(--fg-muted);
  }
  .note {
    font-size: var(--fs-xs);
    color: var(--fg-muted);
    line-height: 1.5;
  }
  label {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  label span {
    font-size: var(--fs-sm);
    font-weight: 600;
  }
  input {
    padding: 7px 10px;
    font-size: var(--fs-sm);
  }
  input.code {
    font-family: var(--font-mono);
    letter-spacing: 0.2em;
    max-width: 12ch;
  }
  input.redeem {
    flex: 1;
    font-family: var(--font-mono);
  }
  .row {
    display: flex;
    align-items: center;
    gap: 6px;
    flex-wrap: wrap;
  }
  small {
    font-size: var(--fs-xs);
    color: var(--fg-muted);
  }
  small.err,
  .warn {
    color: var(--danger);
  }
  .muted {
    color: var(--fg-muted);
  }
  .who {
    display: flex;
    align-items: center;
    gap: 8px;
    font-size: var(--fs-sm);
  }
  /* the plan, quiet for free and the accent for a paid one */
  .badge {
    font-size: var(--fs-xs);
    color: var(--fg-muted);
    border: 1px solid var(--border);
    border-radius: 10px;
    padding: 1px 8px;
    white-space: nowrap;
  }
  .badge.pro {
    color: var(--accent);
    border-color: var(--accent);
  }
  .facts {
    list-style: none;
    display: flex;
    flex-direction: column;
    gap: 4px;
    font-size: var(--fs-sm);
  }
  .facts .lead {
    font-weight: 600;
  }
  .facts .sub {
    display: flex;
    align-items: center;
    gap: 8px;
    flex-wrap: wrap;
  }
  .btn {
    font-size: var(--fs-sm);
    padding: 6px 14px;
    border-radius: 6px;
    border: 1px solid var(--border);
  }
  .btn.small {
    font-size: var(--fs-xs);
    padding: 3px 10px;
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
