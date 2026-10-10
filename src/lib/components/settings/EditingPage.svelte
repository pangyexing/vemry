<script lang="ts">
  import type { LinkOpenMode } from "$lib/api";
  import { t } from "$lib/i18n/i18n.svelte";
  import { rowControl, settingsSearch } from "./context";
  import { form } from "./form.svelte";
  import SettingRow from "./SettingRow.svelte";
  import SettingSwitch from "./SettingSwitch.svelte";
  import SettingSection from "./SettingSection.svelte";

  const s = $derived(form.s);
  const search = settingsSearch();
</script>

{#if s}
  <SettingSection title={t("settings.sec.text")}>
    <!-- A line of body text in the note's own face at the chosen size: the note behind the
         dialog changes too, but it sits under the scrim -->
    <p class="sample" hidden={!!search?.query} style:font-size="{s.note_font_size}px">{t("settings.fontSample")}</p>
    <SettingRow id="editing.fontSize">
      <select {@attach rowControl} value={s.note_font_size} onchange={(e) => void form.set({ note_font_size: Number(e.currentTarget.value) })}>
        <option value={15}>{t("settings.fontSizeCompact")}</option>
        <option value={16}>{t("settings.fontSizeDefault")}</option>
        <option value={17}>17 px</option>
        <option value={18}>{t("settings.fontSizeRoomy")}</option>
      </select>
    </SettingRow>
    <SettingSwitch id="editing.lineWidth" field="readable_line_width" />
  </SettingSection>
  <SettingSection title={t("settings.sec.links")}>
    <SettingSwitch id="editing.clickOpensLinks" field="click_opens_links" />
    <SettingRow id="editing.openLinksIn">
      <select {@attach rowControl} value={s.open_links_in} onchange={(e) => void form.set({ open_links_in: e.currentTarget.value as LinkOpenMode })}>
        <option value="reading">{t("settings.openLinksReading")}</option>
        <option value="internal">{t("settings.openLinksInternal")}</option>
        <option value="system">{t("settings.openLinksSystem")}</option>
      </select>
    </SettingRow>
    <SettingSwitch id="editing.readingImages" field="reading_images_enabled" />
    <SettingSwitch id="editing.noteRemoteImages" field="note_remote_images" />
  </SettingSection>
  <SettingSection title={t("settings.formatSection")}>
    <SettingSwitch id="editing.formatClips" field="format_clips" />
    <SettingSwitch id="editing.cjkSpacing" field="format_cjk_spacing" />
    <SettingSwitch id="editing.punctuation" field="format_punctuation" />
  </SettingSection>
{/if}

<style>
  .sample {
    line-height: 1.6;
    padding: 10px 12px;
    margin: 6px 0 4px;
    border: 1px solid var(--border);
    border-radius: 6px;
    background: var(--bg);
  }
  .sample[hidden] {
    display: none;
  }
</style>
