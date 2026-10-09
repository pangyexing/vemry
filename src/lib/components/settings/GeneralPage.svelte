<script lang="ts">
  import { executeCommand } from "$lib/commands/registry";
  import { LANGS, langName, t, type LangSetting } from "$lib/i18n/i18n.svelte";
  import { updater } from "$lib/stores/updater.svelte";
  import { rowControl } from "./context";
  import { form } from "./form.svelte";
  import SettingRow from "./SettingRow.svelte";
  import SettingSwitch from "./SettingSwitch.svelte";
  import SettingSection from "./SettingSection.svelte";

  const s = $derived(form.s);
  const versionLine = $derived.by(() => {
    const v = t("settings.currentVersion", { version: updater.current || "…" });
    if (updater.phase === "uptodate") return `${v} · ${t("settings.upToDate")}`;
    if (updater.phase === "available") return `${v} · ${t("settings.updateAvailable", { version: updater.version })}`;
    if (updater.phase === "ready") return `${v} · ${t("settings.installedRestart")}`;
    return v;
  });
</script>

{#if s}
  <SettingSection>
    <SettingRow id="general.language">
      <select
        {@attach rowControl}
        value={s.language}
        onchange={(e) => void form.set({ language: e.currentTarget.value as LangSetting })}
      >
        <option value="auto">{t("settings.language.auto")}</option>
        {#each LANGS as tag (tag)}
          <option value={tag}>{langName(tag)}</option>
        {/each}
      </select>
    </SettingRow>
    <!-- What closing the window means is an application behaviour, so it lives here; the icon's failure (a desktop without a tray) replaces the hint -->
    <SettingSwitch id="general.resident" field="tray_resident" hint={s.tray_resident && s.tray_error ? t("settings.residentFailed", { error: s.tray_error }) : undefined} />
  </SettingSection>
  <SettingSection title={t("settings.appUpdates")}>
    <SettingRow id="general.version" hint={updater.phase === "error" ? undefined : versionLine} error={updater.phase === "error" ? updater.error : null}>
      {#if updater.phase === "downloading"}
        <button class="btn small" disabled>
          {t("settings.downloading")}{updater.total ? ` ${Math.round((updater.progress / updater.total) * 100)}%` : ""}
        </button>
      {:else if updater.phase === "available"}
        <!-- The update dialog carries the install (its progress and the restart): this only brings it back -->
        <button {@attach rowControl} class="btn small" onclick={() => updater.show()}>{t("settings.downloadInstall", { version: updater.version })}</button>
      {:else if updater.phase === "ready"}
        <button {@attach rowControl} class="btn small" onclick={() => updater.restart()}>{t("settings.restartToFinish")}</button>
      {:else}
        <button {@attach rowControl} class="btn small" disabled={updater.phase === "checking"} onclick={() => updater.check(true)}>
          {updater.phase === "checking" ? t("settings.checking") : t("cmd.check-updates.title")}
        </button>
      {/if}
    </SettingRow>
    <SettingSwitch id="general.autoCheck" field="updater_auto_check" />
  </SettingSection>
  <SettingSection title={t("settings.about")}>
    <SettingRow id="general.licenses">
      <button {@attach rowControl} class="btn small" onclick={() => void executeCommand("open-licenses")}>{t("settings.licensesOpen")}</button>
    </SettingRow>
  </SettingSection>
{/if}
