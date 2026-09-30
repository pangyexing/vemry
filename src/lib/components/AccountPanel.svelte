<script lang="ts">
  // The account page of the settings dialog (Settings › Account): signed out, an
  // e-mail address and the 6-digit code it receives; signed in, the entitlements as
  // label/value rows (plan and expiry, hosted models, the month's quota with a bar of
  // what is left, the credit pack, the subscription) and the actions on them. Purchase and subscription management open vemry.app in the
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
  let refreshing = $state(false);

  const me = $derived(account.me);
  const status = $derived(account.status);
  /** Days since the snapshot, for the offline notice while it is stale. */
  const staleDays = $derived.by(() => {
    const at = status?.fetched_at ? Date.parse(status.fetched_at) : NaN;
    if (Number.isNaN(at)) return 0;
    return Math.floor((Date.now() - at) / 86_400_000);
  });

  /** Credits in the interface's locale ("3,000"). A credit is $0.001 of charge; it is never
   *  shown as money, since "$6.00" beside a $10 pack would read as its price. The interface
   *  shows credits, never tokens. */
  const count = $derived(new Intl.NumberFormat(i18n.locale, { maximumFractionDigits: 0 }));
  function credits(fen: number): string {
    return count.format(Math.abs(fen));
  }
  /** A date the server sent (YYYY-MM-DD or RFC 3339) in the interface's locale; unparseable stays as it came. */
  function day(s: string | null | undefined): string {
    if (!s) return "";
    const d = new Date(s.length === 10 ? `${s}T00:00:00` : s);
    return Number.isNaN(d.getTime()) ? s : d.toLocaleDateString(i18n.locale, { year: "numeric", month: "short", day: "numeric" });
  }
  function planLabel(plan: string, trial: boolean): string {
    if (trial) return t("account.plan.trial");
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

  /** An account page on vemry.app (`section` is its path under /account), in the system browser: through a
   *  single-use sign-in link so the browser does not ask for the e-mail code again, else the plain page. */
  async function openPortal(section = "") {
    const page = section ? `/account/${section}` : "/account";
    let url: string;
    try {
      url = await api.accountWebLink(page);
    } catch {
      const base = me?.portal_url || "https://vemry.app/account";
      url = section ? `${base}/${section}` : base;
    }
    void api.urlOpenSystem(url).catch((e) => toast.push(String(e), "error"));
  }
</script>

<section class="account">
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
    {@const sub = me.subscription}
    {@const subscribed = !!sub && sub.status !== "canceled"}
    <dl class="kv">
      <dt>{t("account.row.email")}</dt>
      <dd>{me.email}</dd>
      <dt>{t("account.row.plan")}</dt>
      <dd>
        <span class="badge" class:pro={me.plan === "pro" || me.plan === "founder"}>{planLabel(me.plan, me.trial)}</span>
        {#if me.pro_until}<span class="muted">{t("account.until", { date: day(me.pro_until) })}</span>{/if}
      </dd>
      <dt>{t("account.row.hosted")}</dt>
      <dd class:muted={!me.hosted}>{me.hosted ? t("account.hostedOn") : t("account.hostedOff")}</dd>
      {#if me.quota}
        {@const left = Math.max(0, me.quota.total_fen - me.quota.used_fen)}
        {@const share = me.quota.total_fen > 0 ? Math.min(100, Math.round((left * 100) / me.quota.total_fen)) : 0}
        <dt>{t(me.trial ? "account.row.quotaTrial" : "account.row.quota")}</dt>
        <dd class="quota">
          <strong>{t("account.quotaLeft", { n: credits(left) })}</strong>
          <span class="meter" class:low={share < 10} aria-hidden="true"><span style:width="{share}%"></span></span>
          <small>{t(me.trial ? "account.quotaOfTrial" : "account.quotaOf", { total: credits(me.quota.total_fen), date: day(me.quota.resets_at) })}</small>
        </dd>
      {/if}
      <dt>{t("account.row.pack")}</dt>
      <dd class:warn={me.pack_fen < 0}>
        {me.pack_fen < 0 ? t("account.packNegative", { amount: credits(me.pack_fen) }) : t("account.pack", { amount: credits(me.pack_fen) })}
      </dd>
      {#if sub}
        <dt>{t("account.row.subscription")}</dt>
        <dd class="sub">
          {#if sub.status === "past_due"}
            <span class="warn">{t("account.subscriptionPastDue")}</span>
          {:else if sub.cancel_at_period_end || sub.status === "canceling" || sub.status === "canceled"}
            <span>{t("account.subscriptionCancelled", { date: day(me.pro_until) })}</span>
          {:else}
            <span>{t("account.subscriptionOn", { date: day(sub.next_charge_at), price: sub.price ? ` · ${sub.price}` : "" })}</span>
          {/if}
          <button class="btn small" onclick={() => void openPortal("subscription")}>{t("account.manageSubscription")}</button>
        </dd>
      {/if}
    </dl>
    {#if me.peak && me.hosted}
      <small>{t("account.peak")}</small>
    {/if}
    {#if status?.freshness === "expired"}
      <small class="err">{t("account.expired")}</small>
    {:else if status?.freshness === "stale" && staleDays >= 1}
      <small class="warn">{t("account.offlineStale", { days: staleDays })}</small>
    {/if}
    <div class="row actions">
      <!-- With a subscription running the account page offers only the pack -->
      <button class="btn primary" onclick={() => void openPortal()}>{subscribed ? t("account.buyPack") : t("account.buy")}</button>
      <button class="btn" disabled={refreshing} onclick={() => void refresh()}>{refreshing ? t("account.refreshing") : t("account.refresh")}</button>
      <button class="btn signout" disabled={busy} onclick={() => void signOut()}>{t("account.signOut")}</button>
    </div>
    <small>{t("account.buyHint")}</small>
  {/if}
  {#if error}<small class="err">{error}</small>{/if}
</section>

<style>
  .account {
    display: flex;
    flex-direction: column;
    gap: 8px;
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
  /* the entitlements: a label column and a value column, as on the account page */
  .kv {
    display: grid;
    grid-template-columns: max-content minmax(0, 1fr);
    gap: 8px 20px;
    margin: 4px 0;
    font-size: var(--fs-sm);
  }
  .kv dt {
    color: var(--fg-muted);
  }
  .kv dd {
    margin: 0;
    display: flex;
    align-items: center;
    gap: 8px;
    flex-wrap: wrap;
    min-width: 0;
  }
  /* the month's quota: what is left, a bar of it, then the total and the reset */
  .kv dd.quota {
    flex-direction: column;
    align-items: flex-start;
    gap: 5px;
  }
  .quota strong {
    font-weight: 600;
    font-variant-numeric: tabular-nums;
  }
  .meter {
    display: block;
    width: 100%;
    max-width: 240px;
    height: 5px;
    border-radius: 999px;
    background: var(--bg-hover);
    overflow: hidden;
  }
  .meter > span {
    display: block;
    height: 100%;
    border-radius: inherit;
    background: var(--accent);
  }
  .meter.low > span {
    background: var(--warning);
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
  .actions {
    margin-top: 4px;
  }
  /* signing out is not one of the account's actions: it stands apart, at the far end */
  .btn.signout {
    margin-left: auto;
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
