<script lang="ts">
  import { open as openDialog } from "@tauri-apps/plugin-dialog";
  import { api } from "$lib/api";
  import { i18n, t } from "$lib/i18n/i18n.svelte";
  import { browser } from "$lib/stores/browser.svelte";
  import { toast } from "$lib/stores/toast.svelte";
  import { form } from "./form.svelte";
  import SettingRow from "./SettingRow.svelte";
  import SettingSection from "./SettingSection.svelte";
  import SettingText from "./SettingText.svelte";
  import Switch from "./Switch.svelte";

  const s = $derived(form.s);
  let updatingRules = $state(false);

  /** The ad blocker's state stands in for the hint while there is something to say. */
  const adblockState = $derived.by(() => {
    const a = browser.adblock;
    if (!a) return undefined;
    if (a.state === "ready") return t("settings.adblockActive", { n: a.rules.toLocaleString(i18n.locale) });
    if (a.state === "downloading") return t("settings.adblockDownloading");
    if (a.state === "compiling") return t("settings.adblockCompiling");
    if (a.state === "no_rules") return t("settings.adblockNoRules");
    return undefined;
  });
  const splitUrls = (text: string) => text.split("\n").map((u) => u.trim()).filter(Boolean);

  /** Download and recompile the sources now (the sources field has already landed on blur). */
  async function updateRules() {
    updatingRules = true;
    try {
      // A click does not take the focus on macOS WebKit: a source typed a moment ago is
      // written first, so the update fetches what the field shows
      (document.activeElement as HTMLElement | null)?.blur();
      await form.idle();
      const failures = await api.adblockUpdateRules();
      // A partial download is a success (the sources are independent lists), but the ones
      // that did not come through are named — silence would read as "all up to date"
      toast.push(
        failures.length ? t("settings.rulesPartial", { errors: failures.join("; ") }) : t("settings.rulesUpdated"),
        failures.length ? "error" : undefined,
      );
    } catch (e) {
      toast.push(String(e), "error");
    } finally {
      updatingRules = false;
    }
  }

  /** The way out for a network that cannot reach the subscription hosts: rule files downloaded by hand. */
  async function importRules() {
    let picked: string | string[] | null;
    try {
      picked = await openDialog({ multiple: true, title: t("settings.importRulesTitle"), filters: [{ name: t("settings.ruleFileFilter"), extensions: ["txt"] }] });
    } catch (e) {
      toast.push(String(e), "error");
      return;
    }
    const paths = picked == null ? [] : Array.isArray(picked) ? picked : [picked];
    if (!paths.length) return;
    updatingRules = true;
    try {
      const n = await api.adblockImportRules(paths);
      toast.push(t("settings.rulesImported", { n: String(n) }));
    } catch (e) {
      toast.push(String(e), "error");
    } finally {
      updatingRules = false;
    }
  }
</script>

{#if s}
  <SettingSection title={t("settings.sec.adblock")}>
    <SettingRow
      id="browser.adblock"
      hint={s.adblock_enabled ? adblockState : undefined}
      error={s.adblock_enabled && browser.adblock?.state === "failed" ? browser.adblock.error : null}
    >
      <Switch checked={s.adblock_enabled} onchange={(v) => void form.set({ adblock_enabled: v })} />
    </SettingRow>
    <SettingRow id="browser.ruleUrls" block>
      <SettingText value={s.adblock_rule_urls.join("\n")} rows={2} mono commit={(v) => void form.set({ adblock_rule_urls: splitUrls(v) }, "browser.ruleUrls")} />
      <div class="actions">
        <button class="btn small" disabled={updatingRules} onclick={updateRules}>{updatingRules ? t("settings.updatingRules") : t("settings.updateRules")}</button>
        <button class="btn small" disabled={updatingRules} onclick={importRules}>{t("settings.importRules")}</button>
      </div>
    </SettingRow>
  </SettingSection>
  <SettingSection title={t("settings.sec.clip")}>
    <SettingRow id="browser.clipImages">
      <Switch checked={s.clip_localize_images} onchange={(v) => void form.set({ clip_localize_images: v })} />
    </SettingRow>
  </SettingSection>
{/if}

<style>
  .actions {
    display: flex;
    gap: 6px;
    margin-top: 6px;
  }
</style>
