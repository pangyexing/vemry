<script lang="ts">
  import { getContext } from "svelte";
  import { api, EFFORT_OPTIONS } from "$lib/api";
  import { t } from "$lib/i18n/i18n.svelte";
  import { account } from "$lib/stores/account.svelte";
  import { settingsUi } from "$lib/stores/settingsUi.svelte";
  import { toast } from "$lib/stores/toast.svelte";
  import { targetLabel, TARGET_TAGS } from "$lib/stores/translate.svelte";
  import { vault } from "$lib/stores/vault.svelte";
  import type { SearchContext } from "./context";
  import { form } from "./form.svelte";
  import SettingRow from "./SettingRow.svelte";
  import SettingSection from "./SettingSection.svelte";
  import SettingText from "./SettingText.svelte";
  import Switch from "./Switch.svelte";

  const s = $derived(form.s);
  const search = getContext<SearchContext | undefined>("settings-search");

  /** This vault's standing write grant ("allow" = the permission dialog's "always allow in this vault"). */
  let writePolicy = $state<"allow" | "ask">("ask");
  $effect(() => {
    if (!vault.root) return;
    api.vaultWritePolicyGet().then((p) => (writePolicy = p)).catch(() => (writePolicy = "ask"));
  });
  async function revokeWritePolicy() {
    try {
      await api.vaultWritePolicySet("ask");
      writePolicy = "ask";
      toast.push(t("settings.writePolicyRevoked"));
    } catch (e) {
      toast.push(String(e), "error");
    }
  }

  /** The translation target: "" follows the interface, a listed tag, or "custom" with a tag typed below. */
  const customTarget = $derived(!!s && s.translate_target !== "" && !(TARGET_TAGS as readonly string[]).includes(s.translate_target));
  let pickingCustom = $state(false);
  const targetChoice = $derived(customTarget || pickingCustom ? "custom" : (s?.translate_target ?? ""));

  const effortLabel = (value: string, isDefault: boolean) => {
    const name = t(`settings.effort.${value as "none"}`);
    const hint = t(`settings.effort.${value as "none"}Hint`);
    return hint ? `${name} · ${hint}` : isDefault ? `${name}${t("settings.reasoningDefaultSuffix")}` : name;
  };
</script>

{#if s}
  <SettingSection title={t("settings.sec.model")}>
    <SettingRow id="ai.provider">
      <div class="segmented" role="radiogroup" aria-label={t("settings.provider")}>
        <button id="setting-ai-provider" role="radio" aria-checked={s.provider === "hosted"} class:active={s.provider === "hosted"} onclick={() => void form.set({ provider: "hosted" })}
          >{t("settings.providerHosted")}</button
        >
        <button role="radio" aria-checked={s.provider === "custom"} class:active={s.provider === "custom"} onclick={() => void form.set({ provider: "custom" })}
          >{t("settings.providerCustom")}</button
        >
      </div>
    </SettingRow>
    {#if s.provider === "hosted"}
      {@const catalogDefault = account.models.find((m) => m.default)?.id}
      <SettingRow id="ai.model" hint={account.loggedIn ? t("settings.hostedModelHint") : t("settings.hostedSignedOut")}>
        {#if !account.loggedIn}
          <button class="link" onclick={() => (settingsUi.page = "account")}>{t("settings.signIn")} →</button>
        {/if}
        <select id="setting-ai-model" value={s.hosted_model} disabled={!account.loggedIn} onchange={(e) => void form.set({ hosted_model: e.currentTarget.value })}>
          <option value="">{catalogDefault ? t("settings.hostedModelDefault", { model: catalogDefault }) : t("settings.hostedModelDefaultSignedOut")}</option>
          {#each account.models as m (m.id)}
            <option value={m.id}>{m.id}</option>
          {/each}
        </select>
      </SettingRow>
    {:else}
      <SettingRow id="ai.model" hint={t("settings.modelHint")}>
        <SettingText value={s.model} list="model-presets" placeholder="glm-4.7-flash" width="220px" commit={(v) => void form.set({ model: v.trim() }, "ai.model")} />
      </SettingRow>
      <SettingRow id="ai.endpoint" block>
        <SettingText value={s.base_url} list="base-url-presets" placeholder="https://open.bigmodel.cn/api/paas/v4" commit={(v) => void form.set({ base_url: v.trim() }, "ai.endpoint")} />
      </SettingRow>
      <SettingRow id="ai.apiKey" block hint={s.env_key_present ? t("settings.apiKeyEnv") : undefined}>
        <SettingText value={s.api_key} type="password" placeholder={t("settings.apiKeyPlaceholder")} commit={(v) => void form.set({ api_key: v.trim() }, "ai.apiKey")} />
      </SettingRow>
    {/if}
    <SettingRow id="ai.effort" hint={s.reasoning_effort_active ? undefined : t("settings.reasoningInactive")}>
      <select id="setting-ai-effort" value={s.reasoning_effort} onchange={(e) => void form.set({ reasoning_effort: e.currentTarget.value })}>
        {#each EFFORT_OPTIONS as o (o.value)}
          <option value={o.value}>{effortLabel(o.value, !!o.isDefault)}</option>
        {/each}
      </select>
    </SettingRow>
  </SettingSection>
  <SettingSection title={t("settings.sec.behaviour")}>
    <SettingRow id="ai.webSearch">
      <Switch checked={s.web_search_enabled} onchange={(v) => void form.set({ web_search_enabled: v })} />
    </SettingRow>
    <SettingRow id="ai.planConfirm">
      <Switch checked={s.plan_confirm} onchange={(v) => void form.set({ plan_confirm: v })} />
    </SettingRow>
    <SettingRow id="ai.provenanceMarks">
      <Switch checked={s.provenance_marks} onchange={(v) => void form.set({ provenance_marks: v })} />
    </SettingRow>
    <SettingRow id="ai.formatAgentWrites">
      <Switch checked={s.format_agent_writes} onchange={(v) => void form.set({ format_agent_writes: v })} />
    </SettingRow>
    <!-- The vault's standing grant belongs to neither provider; the row exists only while there is one to revoke -->
    {#if vault.root && writePolicy === "allow"}
      <SettingRow id="ai.writeGrant">
        <button id="setting-ai-writeGrant" class="btn small" onclick={revokeWritePolicy}>{t("settings.revoke")}</button>
      </SettingRow>
    {/if}
  </SettingSection>
  <SettingSection title={t("settings.translate.section")} note={search?.query ? undefined : t("settings.translate.disclosure")}>
    <SettingRow id="ai.translateTarget">
      {#if targetChoice === "custom"}
        <SettingText value={customTarget ? s.translate_target : ""} placeholder={t("settings.translate.targetCustomPlaceholder")} width="120px" commit={(v) => void form.set({ translate_target: v.trim() }, "ai.translateTarget")} />
      {/if}
      <select
        id="setting-ai-translateTarget"
        value={targetChoice}
        onchange={(e) => {
          const v = e.currentTarget.value;
          pickingCustom = v === "custom";
          if (v !== "custom") void form.set({ translate_target: v });
        }}
      >
        <option value="">{t("settings.translate.targetAuto")}</option>
        {#each TARGET_TAGS as tag (tag)}
          <option value={tag}>{targetLabel(tag)}</option>
        {/each}
        <option value="custom">{t("settings.translate.targetCustom")}</option>
      </select>
    </SettingRow>
    <SettingRow id="ai.translateModel">
      <SettingText value={s.translate_model} list="model-presets" placeholder={s.active_model || "glm-4.7-flash"} width="220px" commit={(v) => void form.set({ translate_model: v.trim() }, "ai.translateModel")} />
    </SettingRow>
    <SettingRow id="ai.translateAddendum" block>
      <SettingText value={s.translate_addendum} commit={(v) => void form.set({ translate_addendum: v.trim() }, "ai.translateAddendum")} />
    </SettingRow>
  </SettingSection>

  <datalist id="base-url-presets">
    <option value="https://open.bigmodel.cn/api/paas/v4">{t("settings.endpointZhipu")}</option>
    <option value="https://api.deepseek.com">{t("settings.endpointDeepseek")}</option>
    <option value="http://localhost:11434/v1">{t("settings.endpointOllama")}</option>
  </datalist>
  <datalist id="model-presets">
    <option value="glm-4.7-flash">{t("settings.modelZhipuFlash")}</option>
    <option value="glm-4.7">{t("settings.modelZhipu")}</option>
    <option value="deepseek-flash">{t("settings.modelDeepseekFlash")}</option>
    <option value="deepseek-v4-pro">{t("settings.modelDeepseekPro")}</option>
  </datalist>
{/if}

<style>
  /* two states side by side, the chosen one filled */
  .segmented {
    display: inline-flex;
    border: 1px solid var(--border);
    border-radius: 6px;
    overflow: hidden;
  }
  .segmented button {
    font-size: var(--fs-sm);
    padding: 4px 12px;
    color: var(--fg-muted);
    white-space: nowrap;
  }
  .segmented button + button {
    border-left: 1px solid var(--border);
  }
  .segmented button.active {
    background: var(--accent-soft);
    color: var(--accent);
    font-weight: 600;
  }
</style>
