<script lang="ts">
  import { api, type SkillEntry, type TypeExample } from "$lib/api";
  import { t } from "$lib/i18n/i18n.svelte";
  import { isImeKeydown } from "$lib/ime";
  import { scaffold } from "$lib/scaffold";
  import { skillDesc, skillSourceLabel } from "$lib/skills";
  import { scripts } from "$lib/stores/scripts.svelte";
  import { settingsUi } from "$lib/stores/settingsUi.svelte";
  import { toast } from "$lib/stores/toast.svelte";
  import { vault } from "$lib/stores/vault.svelte";
  import { rowControl, settingsSearch } from "./context";
  import SettingRow from "./SettingRow.svelte";
  import SettingSection from "./SettingSection.svelte";
  import Switch from "./Switch.svelte";

  const search = settingsSearch();

  /** The built-ins and the vault's own files (disabled ones included; a switch applies from the next turn). */
  let skills = $state<SkillEntry[]>([]);
  let newSkill = $state("");
  /** The example block types (product-document blocks) this page can install. */
  let typeExamples = $state<TypeExample[]>([]);

  $effect(() => {
    if (!vault.root || search?.query) return;
    void scripts.refresh();
    api.skillsList().then((s) => (skills = s)).catch(() => (skills = []));
    api.typeExamplesList().then((s) => (typeExamples = s)).catch(() => (typeExamples = []));
  });

  /** Opening a file in the editor closes the dialog: the editor is behind it. */
  async function openSkill(file: string) {
    settingsUi.open = false;
    await vault.openFile(`.vemry/skills/${file}`);
  }
  async function customizeSkill(name: string) {
    settingsUi.open = false;
    await scaffold(() => api.skillCustomize(name));
  }
  async function restoreSkill(name: string) {
    try {
      await api.skillRestore(name);
      toast.push(t("settings.skillRestored", { name }));
      skills = await api.skillsList();
    } catch (e) {
      toast.push(String(e), "error");
    }
  }
  async function createSkill() {
    const name = newSkill.trim();
    if (!name) return;
    newSkill = "";
    settingsUi.open = false;
    await scaffold(() => api.skillCreate(name));
  }
  async function toggleSkill(name: string, enabled: boolean) {
    try {
      await api.skillSetEnabled(name, enabled);
      skills = await api.skillsList();
    } catch (e) {
      toast.push(String(e), "error");
    }
  }
  async function installTypeExample(name: string) {
    try {
      const r = await api.typeInstallExample(name);
      toast.push(t(r.created ? "settings.skillInstalled" : "settings.skillAlready", { path: r.path }));
      typeExamples = await api.typeExamplesList();
    } catch (e) {
      toast.push(String(e), "error");
    }
  }

  /** Every script operation has the same shape: call the api, then read the list again; a failure is a toast. */
  async function withRefresh(action: () => Promise<void>) {
    try {
      await action();
      await scripts.refresh();
    } catch (e) {
      toast.push(String(e), "error");
    }
  }
  /** The triggers at a glance: how many commands / event subscriptions / scheduled tasks */
  function scriptSummary(s: (typeof scripts.infos)[number]): string {
    const parts: string[] = [];
    if (s.commands.length) parts.push(t("settings.scriptCommands", { n: s.commands.length }));
    if (s.event_triggers.length) parts.push(t("settings.scriptEvents", { list: [...new Set(s.event_triggers)].join(", ") }));
    if (s.schedules.length) parts.push(t("settings.scriptSchedules", { list: s.schedules.join(" | ") }));
    return parts.length ? parts.join(" · ") : t("settings.scriptNothing");
  }
  const examplesLeft = $derived(typeExamples.filter((e) => !e.installed));
</script>

<SettingSection>
  <SettingRow id="skills.skills" block>
    <ul class="list">
      {#each skills as s (s.name)}
        <li>
          <div class="item">
            <span class="name">{s.name}</span>
            <span class="badge" class:custom={s.source !== "builtin"}>{skillSourceLabel(s)}</span>
            {#if s.outdated}
              <span class="badge outdated" title={t("settings.skillOutdatedHint")}>{t("settings.skillOutdated")}</span>
            {/if}
            <span class="spring"></span>
            {#if s.source === "builtin"}
              <button class="btn small" onclick={() => void customizeSkill(s.name)}>{t("settings.skillCustomize")}</button>
            {:else}
              <button class="btn small" onclick={() => void openSkill(s.file)}>{t("settings.edit")}</button>
              {#if s.source === "override"}
                <button class="btn small" onclick={() => void restoreSkill(s.name)}>{t("settings.skillRestore")}</button>
              {/if}
            {/if}
            <Switch checked={!s.disabled} label={s.name} onchange={(v) => void toggleSkill(s.name, v)} />
          </div>
          {#if skillDesc(s)}<small>{skillDesc(s)}</small>{/if}
        </li>
      {/each}
    </ul>
    <div class="add">
      <input
        {@attach rowControl}
        bind:value={newSkill}
        placeholder={t("settings.skillNewPlaceholder")}
        spellcheck="false"
        onkeydown={(e) => e.key === "Enter" && !isImeKeydown(e) && void createSkill()}
      />
      <button class="btn" onclick={() => void createSkill()}>{t("settings.skillNew")}</button>
    </div>
  </SettingRow>
  {#if examplesLeft.length || search?.query}
    <SettingRow id="skills.typeExamples" block>
      <ul class="list">
        {#each examplesLeft as e (e.name)}
          <li>
            <div class="item">
              <span class="name">{e.name}</span>
              <span class="spring"></span>
              <button class="btn small" onclick={() => void installTypeExample(e.name)}>{t("settings.installSkill")}</button>
            </div>
            <small>{e.description}</small>
          </li>
        {/each}
      </ul>
    </SettingRow>
  {/if}
  <SettingRow id="skills.scripts" block>
    <div class="head-actions">
      <button class="btn small" onclick={() => withRefresh(() => api.scriptsReload())}>{t("settings.reloadScripts")}</button>
    </div>
    {#if scripts.infos.length}
      <ul class="list">
        {#each scripts.infos as s (s.name)}
          <li>
            <div class="item">
              <span class="name">{s.name}</span>
              {#if s.write_grant}
                <span class="badge custom">{t("settings.writable")}</span>
                <button class="link" onclick={() => withRefresh(() => api.scriptGrantRevoke(s.name))}>{t("settings.revokeGrant")}</button>
              {/if}
              <span class="spring"></span>
              <Switch checked={s.enabled} label={s.name} onchange={(v) => void withRefresh(() => api.scriptSetEnabled(s.name, v))} />
            </div>
            {#if s.enabled}<small>{scriptSummary(s)}</small>{/if}
            {#if s.last_error}<small class="err">{s.last_error}</small>{/if}
          </li>
        {/each}
      </ul>
    {:else}
      <small>{t("settings.noScripts")}</small>
    {/if}
  </SettingRow>
</SettingSection>

<style>
  small.err {
    color: var(--danger);
    word-break: break-all;
  }
  .list {
    list-style: none;
    margin: 2px 0;
    padding: 0;
    display: flex;
    flex-direction: column;
  }
  .list li {
    display: flex;
    flex-direction: column;
    gap: 2px;
    padding: 7px 0;
    border-bottom: 1px solid var(--border);
  }
  .list li:last-child {
    border-bottom: none;
  }
  .item {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .spring {
    flex: 1;
  }
  .name {
    font-family: var(--font-mono);
    font-size: var(--fs-sm);
    font-weight: 500;
  }
  /* which layer a row is: quiet for a built-in, the accent for the user's own file */
  .badge {
    font-size: var(--fs-xs);
    color: var(--fg-muted);
    border: 1px solid var(--border);
    border-radius: 10px;
    padding: 0 8px;
    white-space: nowrap;
  }
  .badge.custom {
    color: var(--accent);
    border-color: var(--accent);
  }
  .badge.outdated {
    border-style: dashed;
  }
  .head-actions {
    display: flex;
    justify-content: flex-end;
  }
</style>
