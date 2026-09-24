<script lang="ts">
  import type { LinkOpenMode } from "$lib/api";
  import { t } from "$lib/i18n/i18n.svelte";
  import { k } from "$lib/shortcut";
  import { getContext } from "svelte";
  import type { SearchContext } from "./context";
  import { form } from "./form.svelte";
  import SettingRow from "./SettingRow.svelte";
  import SettingSection from "./SettingSection.svelte";
  import Switch from "./Switch.svelte";

  const s = $derived(form.s);
  const search = getContext<SearchContext | undefined>("settings-search");
</script>

{#if s}
  <SettingSection title={t("settings.sec.text")}>
    <!-- A line of body text in the note's own face at the chosen size: the note behind the
         dialog changes too, but it sits under the scrim -->
    <p class="sample" hidden={!!search?.query} style:font-size="{s.note_font_size}px">{t("settings.fontSample")}</p>
    <SettingRow id="editing.fontSize">
      <select id="setting-editing-fontSize" value={s.note_font_size} onchange={(e) => void form.set({ note_font_size: Number(e.currentTarget.value) })}>
        <option value={15}>{t("settings.fontSizeCompact")}</option>
        <option value={16}>{t("settings.fontSizeDefault")}</option>
        <option value={17}>17 px</option>
        <option value={18}>{t("settings.fontSizeRoomy")}</option>
      </select>
    </SettingRow>
    <SettingRow id="editing.lineWidth">
      <Switch checked={s.readable_line_width} onchange={(v) => void form.set({ readable_line_width: v })} />
    </SettingRow>
  </SettingSection>
  <SettingSection title={t("settings.sec.links")}>
    <SettingRow id="editing.clickOpensLinks" hint={t("settings.clickOpensLinksHint", { modClick: k("Mod+click") })}>
      <Switch checked={s.click_opens_links} onchange={(v) => void form.set({ click_opens_links: v })} />
    </SettingRow>
    <SettingRow id="editing.openLinksIn" hint={t("settings.openLinksHint", { key: k("Mod+click") })}>
      <select id="setting-editing-openLinksIn" value={s.open_links_in} onchange={(e) => void form.set({ open_links_in: e.currentTarget.value as LinkOpenMode })}>
        <option value="reading">{t("settings.openLinksReading")}</option>
        <option value="internal">{t("settings.openLinksInternal")}</option>
        <option value="system">{t("settings.openLinksSystem")}</option>
      </select>
    </SettingRow>
    <SettingRow id="editing.readingImages">
      <Switch checked={s.reading_images_enabled} onchange={(v) => void form.set({ reading_images_enabled: v })} />
    </SettingRow>
  </SettingSection>
  <SettingSection title={t("settings.formatSection")}>
    <SettingRow id="editing.formatClips">
      <Switch checked={s.format_clips} onchange={(v) => void form.set({ format_clips: v })} />
    </SettingRow>
    <SettingRow id="editing.cjkSpacing">
      <Switch checked={s.format_cjk_spacing} onchange={(v) => void form.set({ format_cjk_spacing: v })} />
    </SettingRow>
    <SettingRow id="editing.punctuation">
      <Switch checked={s.format_punctuation} onchange={(v) => void form.set({ format_punctuation: v })} />
    </SettingRow>
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
