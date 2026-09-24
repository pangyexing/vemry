<script lang="ts">
  import { LANGS, langName, t, type LangSetting } from "$lib/i18n/i18n.svelte";
  import { isMac, k } from "$lib/shortcut";
  import { updater } from "$lib/stores/updater.svelte";
  import { form } from "./form.svelte";
  import SettingRow from "./SettingRow.svelte";
  import SettingSection from "./SettingSection.svelte";
  import Switch from "./Switch.svelte";

  const s = $derived(form.s);
  const residentHint = $derived(isMac ? t("settings.residentHint", { quit: k("Mod+Q") }) : t("settings.residentHintTray"));
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
        id="setting-general-language"
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
    <SettingRow
      id="general.resident"
      hint={s.tray_resident && s.tray_error ? t("settings.residentFailed", { error: s.tray_error }) : residentHint}
    >
      <Switch checked={s.tray_resident} onchange={(v) => void form.set({ tray_resident: v })} />
    </SettingRow>
  </SettingSection>
  <SettingSection title={t("settings.appUpdates")}>
    <SettingRow id="general.version" hint={updater.phase === "error" ? undefined : versionLine} error={updater.phase === "error" ? updater.error : null}>
      {#if updater.phase === "downloading"}
        <button class="btn small" disabled>
          {t("settings.downloading")}{updater.total ? ` ${Math.round((updater.progress / updater.total) * 100)}%` : ""}
        </button>
      {:else if updater.phase === "available"}
        <!-- The update dialog carries the install (its progress and the restart): this only brings it back -->
        <button id="setting-general-version" class="btn small" onclick={() => updater.show()}>{t("settings.downloadInstall", { version: updater.version })}</button>
      {:else if updater.phase === "ready"}
        <button id="setting-general-version" class="btn small" onclick={() => updater.restart()}>{t("settings.restartToFinish")}</button>
      {:else}
        <button id="setting-general-version" class="btn small" disabled={updater.phase === "checking"} onclick={() => updater.check(true)}>
          {updater.phase === "checking" ? t("settings.checking") : t("cmd.check-updates.title")}
        </button>
      {/if}
    </SettingRow>
    <SettingRow id="general.autoCheck">
      <Switch checked={s.updater_auto_check} onchange={(v) => void form.set({ updater_auto_check: v })} />
    </SettingRow>
  </SettingSection>
{/if}
