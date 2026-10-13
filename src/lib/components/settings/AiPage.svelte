<script lang="ts">
  import { api, EFFORT_OPTIONS, HOSTED, type CustomProvider, type ModelGroup, type ModelRef, type ProviderView } from "$lib/api";
  import { i18n, t, type MessageKey } from "$lib/i18n/i18n.svelte";
  import { isImeKeydown } from "$lib/ime";
  import { groupName, modelLabel, sameRef } from "$lib/models";
  import { account } from "$lib/stores/account.svelte";
  import { openSettings } from "$lib/stores/settingsUi.svelte";
  import { toast } from "$lib/stores/toast.svelte";
  import { targetLabel, TARGET_TAGS } from "$lib/stores/translate.svelte";
  import { vault } from "$lib/stores/vault.svelte";
  import { rowControl, settingsSearch } from "./context";
  import { form } from "./form.svelte";
  import SettingRow from "./SettingRow.svelte";
  import SettingSwitch from "./SettingSwitch.svelte";
  import SettingSection from "./SettingSection.svelte";
  import SettingText from "./SettingText.svelte";

  const s = $derived(form.s);
  const search = settingsSearch();

  /** This vault's standing write grant ("allow" = the permission dialog's "always allow in this vault"). */
  let writePolicy = $state<"allow" | "ask">("ask");
  // Read in search results too: the grant's row exists only while there is one, so a search
  // that finds it has to know whether it does. Once per vault, not per keystroke
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

  /** A model choice as a <select> value, and back. */
  const refKey = (r: ModelRef) => JSON.stringify([r.provider, r.model]);
  function refOf(key: string): ModelRef {
    const [provider, model] = JSON.parse(key) as [string, string];
    return { provider, model };
  }
  const listed = (groups: ModelGroup[], r: ModelRef) => groups.some((g) => g.models.some((m) => sameRef({ provider: g.provider, model: m.id }, r)));
  /** An option's text: a closed <select> shows the option alone, not its group, so a model id two
   *  providers serve names its provider (modelLabel), and vemry hosted's default says so. */
  const choiceLabel = (g: ModelGroup, m: { id: string; default: boolean }) => {
    const label = modelLabel({ provider: g.provider, model: m.id }, s?.model_choices ?? []);
    return m.default ? t("settings.hostedModelDefault", { model: label }) : label;
  };

  /** The providers as shown: every edit sends the whole list back with one row changed, and
   *  Rust cleans it and keeps the ids. The view's derived fields ride along — Rust ignores
   *  them, and the dialog's immediate update keeps each card's name until the read-back. */
  function rows(): CustomProvider[] {
    return form.s?.providers ?? [];
  }
  function withRow(i: number, change: Partial<CustomProvider>): CustomProvider[] {
    return rows().map((p, j) => (j === i ? { ...p, ...change } : p));
  }
  const saveRows = (list: CustomProvider[]) => void form.set({ providers: list }, "ai.providers");

  /** What is typed in each provider's "add a model id" field, by provider id. */
  let modelDrafts = $state<Record<string, string>>({});
  function addModel(i: number, p: ProviderView) {
    const id = (modelDrafts[p.id] ?? "").trim();
    if (!id) return;
    if (!p.models.includes(id)) saveRows(withRow(i, { models: [...p.models, id] }));
    modelDrafts[p.id] = "";
  }

  /** The provider whose Remove was clicked once: a second click within a few seconds removes
   *  it with its key. (A timer, not the button's blur: a click does not focus a button on macOS.) */
  let removing = $state<string | null>(null);
  let removingTimer: ReturnType<typeof setTimeout> | undefined;
  function removeProvider(i: number, p: ProviderView) {
    clearTimeout(removingTimer);
    if (removing !== p.id) {
      removing = p.id;
      removingTimer = setTimeout(() => (removing = null), 3000);
      return;
    }
    removing = null;
    saveRows(rows().filter((_, j) => j !== i));
  }

  /** The endpoints "Add a provider…" offers, with the model ids each is known to serve. The
   *  name is left empty: Rust names an unnamed provider after its vendor or host. Moonshot
   *  runs a mainland and an international site with separate keys; the preset picks the one
   *  the interface language suggests, and the endpoint list offers both. */
  const MOONSHOT_CN = "https://api.moonshot.cn/v1";
  const MOONSHOT_INTL = "https://api.moonshot.ai/v1";
  const PRESETS: Record<string, { base_url: string; models: string[] }> = {
    zhipu: { base_url: "https://open.bigmodel.cn/api/paas/v4", models: ["glm-5.3-flash", "glm-5.3"] },
    deepseek: { base_url: "https://api.deepseek.com", models: ["deepseek-flash", "deepseek-v4-pro"] },
    openai: { base_url: "https://api.openai.com/v1", models: ["gpt-6-astra", "gpt-6.1-sol", "gpt-6-luna"] },
    anthropic: { base_url: "https://api.anthropic.com", models: ["claude-opus-5-5", "claude-sonnet-5-5", "claude-haiku-5-5"] },
    xai: { base_url: "https://api.x.ai/v1", models: ["grok-4.7", "grok-4.3"] },
    moonshot: { base_url: MOONSHOT_INTL, models: ["kimi-k3"] },
    meta: { base_url: "https://api.meta.ai/v1", models: ["muse-spark-1.3"] },
    zai: { base_url: "https://api.z.ai/api/paas/v4", models: ["glm-5.3-flash", "glm-5.3"] },
    ollama: { base_url: "http://localhost:11434/v1", models: [] },
    other: { base_url: "", models: [] },
  };
  function addProvider(kind: string) {
    const preset = PRESETS[kind];
    if (!preset) return;
    const base_url = kind === "moonshot" && i18n.locale.startsWith("zh") ? MOONSHOT_CN : preset.base_url;
    saveRows([...rows(), { id: "", name: "", api_key: "", base_url, models: [...preset.models] }]);
  }
  /** The model ids the presets name, for the "add a model id" suggestions, each labelled with
   *  its vendor (the two flagship lines whose ids say nothing keep a label of their own). */
  const MODEL_SUGGESTIONS: [string, MessageKey][] = [
    ["glm-5.3-flash", "settings.endpointZhipu"],
    ["glm-5.3", "settings.modelZhipu"],
    ["deepseek-flash", "settings.modelDeepseekFlash"],
    ["deepseek-v4-pro", "settings.modelDeepseekPro"],
    ...PRESETS.openai.models.map((m): [string, MessageKey] => [m, "settings.endpointOpenai"]),
    ...PRESETS.anthropic.models.map((m): [string, MessageKey] => [m, "settings.endpointAnthropic"]),
    ...PRESETS.xai.models.map((m): [string, MessageKey] => [m, "settings.endpointXai"]),
    ...PRESETS.moonshot.models.map((m): [string, MessageKey] => [m, "settings.endpointMoonshot"]),
    ...PRESETS.meta.models.map((m): [string, MessageKey] => [m, "settings.endpointMeta"]),
  ];

  const effortLabel = (value: string, isDefault: boolean) => {
    const name = t(`settings.effort.${value as "none"}`);
    const hint = t(`settings.effort.${value as "none"}Hint`);
    return hint ? `${name} · ${hint}` : isDefault ? `${name}${t("settings.reasoningDefaultSuffix")}` : name;
  };
</script>

{#if s}
  <SettingSection title={t("settings.sec.model")}>
    <!-- The current model: every model the providers below list, vemry hosted's first when signed in -->
    {@const chatListed = listed(s.model_choices, s.chat_model)}
    <SettingRow id="ai.model" hint={s.chat_unready ?? undefined}>
      <select {@attach rowControl} value={refKey(s.chat_model)} onchange={(e) => void form.set({ chat_model: refOf(e.currentTarget.value) })}>
        {#if !chatListed}
          <!-- vemry hosted while signed out, or nothing at all: said as its own row, which cannot be picked -->
          <option value={refKey(s.chat_model)} disabled>{s.chat_model.provider === HOSTED ? t("settings.hostedSignedOutOption") : t("settings.noModels")}</option>
        {/if}
        {#each s.model_choices as g (g.provider)}
          <optgroup label={groupName(g)}>
            {#each g.models as m (m.id)}
              <option value={refKey({ provider: g.provider, model: m.id })}>{choiceLabel(g, m)}</option>
            {/each}
          </optgroup>
        {/each}
      </select>
    </SettingRow>
    <SettingRow id="ai.hosted" hint={account.loggedIn ? t("settings.hostedReadyHint") : undefined}>
      {#if !account.loggedIn}
        <button {@attach rowControl} class="link" onclick={() => openSettings("account.panel")}>{t("settings.signIn")} →</button>
      {/if}
    </SettingRow>
    <SettingRow id="ai.providers" block>
      {#if s.providers.length}
        <ul class="providers">
          <!-- A row added a moment ago has no id until Rust reads it back -->
          {#each s.providers as p, i (p.id || `new-${i}`)}
            <li>
              <div class="head">
                <span class="name">{p.display_name || t("settings.providerUnnamed")}</span>
                <span class="spring"></span>
                <button class="del" class:armed={removing === p.id} onclick={() => removeProvider(i, p)}
                  >{removing === p.id ? t("settings.providerRemoveConfirm") : t("settings.providerRemove")}</button
                >
              </div>
              <div class="fields">
                <span class="field-label">{t("settings.providerName")}</span>
                <SettingText label={t("settings.providerName")} value={p.name} placeholder={p.display_name} patch={(v) => ({ providers: withRow(i, { name: v.trim() }) })} />
                <span class="field-label">{t("settings.endpoint")}</span>
                <SettingText label={t("settings.endpoint")} value={p.base_url} list="base-url-presets" mono placeholder="https://…" patch={(v) => ({ providers: withRow(i, { base_url: v.trim() }) })} />
                <span class="field-label">{t("settings.apiKey")}</span>
                <SettingText label={t("settings.apiKey")} value={p.api_key} type="password" placeholder={t("settings.apiKeyPlaceholder")} patch={(v) => ({ providers: withRow(i, { api_key: v.trim() }) })} />
                {#if p.env_key}<small class="field-note">{t("settings.apiKeyEnvFallback")}</small>{/if}
                <span class="field-label">{t("settings.providerModels")}</span>
                <div class="models">
                  {#each p.models as m (m)}
                    <span class="model">
                      {m}
                      <button title={t("settings.providerModelRemove", { model: m })} aria-label={t("settings.providerModelRemove", { model: m })} onclick={() => saveRows(withRow(i, { models: p.models.filter((x) => x !== m) }))}>✕</button>
                    </span>
                  {:else}
                    <small>{t("settings.providerNoModels")}</small>
                  {/each}
                </div>
                <span></span>
                <div class="add">
                  <input
                    class="mono"
                    list="model-presets"
                    aria-label={t("settings.providerModelPlaceholder")}
                    placeholder={t("settings.providerModelPlaceholder")}
                    spellcheck="false"
                    bind:value={() => modelDrafts[p.id] ?? "", (v) => (modelDrafts[p.id] = v)}
                    onkeydown={(e) => e.key === "Enter" && !isImeKeydown(e) && addModel(i, p)}
                  />
                  <button class="btn small" onclick={() => addModel(i, p)}>{t("settings.providerModelAdd")}</button>
                </div>
              </div>
            </li>
          {/each}
        </ul>
      {/if}
      <div class="add-provider">
        <select
          {@attach rowControl}
          value=""
          onchange={(e) => {
            addProvider(e.currentTarget.value);
            e.currentTarget.value = "";
          }}
        >
          <option value="" disabled>{t("settings.providerAdd")}</option>
          <option value="zhipu">{t("settings.endpointZhipu")}</option>
          <option value="deepseek">{t("settings.endpointDeepseek")}</option>
          <option value="openai">{t("settings.endpointOpenai")}</option>
          <option value="anthropic">{t("settings.endpointAnthropic")}</option>
          <option value="xai">{t("settings.endpointXai")}</option>
          <option value="moonshot">{t("settings.endpointMoonshot")}</option>
          <option value="meta">{t("settings.endpointMeta")}</option>
          <option value="zai">{t("settings.endpointZai")}</option>
          <option value="ollama">{t("settings.endpointOllama")}</option>
          <option value="other">{t("settings.providerPresetOther")}</option>
        </select>
        <small>{t("settings.apiKeyNote")}</small>
      </div>
    </SettingRow>
    <SettingRow id="ai.effort" hint={s.reasoning_effort_active ? undefined : t("settings.reasoningInactive")}>
      <select {@attach rowControl} value={s.reasoning_effort} onchange={(e) => void form.set({ reasoning_effort: e.currentTarget.value })}>
        {#each EFFORT_OPTIONS as o (o.value)}
          <option value={o.value}>{effortLabel(o.value, !!o.isDefault)}</option>
        {/each}
      </select>
    </SettingRow>
  </SettingSection>
  <SettingSection title={t("settings.sec.behaviour")}>
    <SettingSwitch id="ai.webSearch" field="web_search_enabled" />
    <SettingSwitch id="ai.planConfirm" field="plan_confirm" />
    <SettingSwitch id="ai.provenanceMarks" field="provenance_marks" />
    <SettingSwitch id="ai.formatAgentWrites" field="format_agent_writes" />
    <!-- The vault's standing grant belongs to neither provider; the row exists only while there is one to revoke -->
    {#if vault.root && writePolicy === "allow"}
      <SettingRow id="ai.writeGrant">
        <button {@attach rowControl} class="btn small" onclick={revokeWritePolicy}>{t("settings.revoke")}</button>
      </SettingRow>
    {/if}
  </SettingSection>
  <SettingSection title={t("settings.translate.section")} note={search?.query ? undefined : t("settings.translate.disclosure")}>
    <SettingRow id="ai.translateTarget">
      {#if targetChoice === "custom"}
        <SettingText value={customTarget ? s.translate_target : ""} placeholder={t("settings.translate.targetCustomPlaceholder")} width="120px" patch={(v) => ({ translate_target: v.trim() })} />
      {/if}
      <select
        {@attach rowControl}
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
      {@const tr = s.translation_model?.provider ? s.translation_model : null}
      <select
        {@attach rowControl}
        value={tr ? refKey(tr) : ""}
        onchange={(e) => {
          const v = e.currentTarget.value;
          void form.set({ translation_model: v ? refOf(v) : { provider: "", model: "" } });
        }}
      >
        <option value="">{t("settings.translate.modelSameAsChat", { model: s.active_model || t("settings.noModels") })}</option>
        {#if tr && !listed(s.model_choices, tr)}
          <option value={refKey(tr)} disabled>{t("settings.hostedSignedOutOption")}</option>
        {/if}
        {#each s.model_choices as g (g.provider)}
          <optgroup label={groupName(g)}>
            {#each g.models as m (m.id)}
              <option value={refKey({ provider: g.provider, model: m.id })}>{choiceLabel(g, m)}</option>
            {/each}
          </optgroup>
        {/each}
      </select>
    </SettingRow>
    <SettingRow id="ai.translateAddendum" block>
      <SettingText value={s.translate_addendum} patch={(v) => ({ translate_addendum: v.trim() })} />
    </SettingRow>
  </SettingSection>

  <datalist id="base-url-presets">
    <option value={PRESETS.zhipu.base_url}>{t("settings.endpointZhipu")}</option>
    <option value={PRESETS.deepseek.base_url}>{t("settings.endpointDeepseek")}</option>
    <option value={PRESETS.openai.base_url}>{t("settings.endpointOpenai")}</option>
    <option value={PRESETS.anthropic.base_url}>{t("settings.endpointAnthropic")}</option>
    <option value={PRESETS.xai.base_url}>{t("settings.endpointXai")}</option>
    <option value={MOONSHOT_CN}>{t("settings.endpointMoonshot")}</option>
    <option value={MOONSHOT_INTL}>{t("settings.endpointMoonshot")}</option>
    <option value={PRESETS.meta.base_url}>{t("settings.endpointMeta")}</option>
    <option value={PRESETS.zai.base_url}>{t("settings.endpointZai")}</option>
    <option value={PRESETS.ollama.base_url}>{t("settings.endpointOllama")}</option>
  </datalist>
  <datalist id="model-presets">
    {#each MODEL_SUGGESTIONS as [id, label] (id)}
      <option value={id}>{t(label)}</option>
    {/each}
  </datalist>
{/if}

<style>
  /* One card per provider: its name and Remove on top, the fields in a two-column grid */
  .providers {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .providers li {
    border: 1px solid var(--border);
    border-radius: 8px;
    padding: 8px 10px 10px;
  }
  .head {
    display: flex;
    align-items: center;
    gap: 8px;
    margin-bottom: 6px;
  }
  .name {
    font-size: var(--fs-sm);
    font-weight: 600;
  }
  .spring {
    flex: 1;
  }
  .del {
    font-size: var(--fs-xs);
    color: var(--fg-muted);
    padding: 2px 6px;
    border-radius: 4px;
  }
  .del:hover,
  .del.armed {
    color: var(--danger);
    background: var(--bg-hover);
  }
  .fields {
    display: grid;
    grid-template-columns: auto minmax(0, 1fr);
    align-items: center;
    gap: 6px 10px;
  }
  .field-label {
    font-size: var(--fs-xs);
    color: var(--fg-muted);
  }
  .field-note {
    grid-column: 2;
    margin-top: -2px;
  }
  .models {
    display: flex;
    flex-wrap: wrap;
    gap: 4px;
  }
  .model {
    display: inline-flex;
    align-items: center;
    gap: 2px;
    font-family: var(--font-mono);
    font-size: var(--fs-xs);
    border: 1px solid var(--border);
    border-radius: 10px;
    padding: 1px 4px 1px 8px;
  }
  .model button {
    color: var(--fg-muted);
    font-size: var(--fs-xs);
    padding: 0 3px;
    border-radius: 6px;
  }
  .model button:hover {
    color: var(--danger);
    background: var(--bg-hover);
  }
  .add {
    display: flex;
    gap: 6px;
  }
  .add input {
    flex: 1;
  }
  .mono {
    font-family: var(--font-mono);
  }
  .add-provider {
    display: flex;
    align-items: center;
    gap: 10px;
    margin-top: 8px;
  }
  .add-provider small {
    color: var(--fg-muted);
    font-size: var(--fs-xs);
  }
</style>
