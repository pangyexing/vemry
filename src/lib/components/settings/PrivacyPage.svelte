<script lang="ts">
  import { getContext, untrack } from "svelte";
  import { api } from "$lib/api";
  import { compareNames, t } from "$lib/i18n/i18n.svelte";
  import { isImeKeydown } from "$lib/ime";
  import { settingsUi } from "$lib/stores/settingsUi.svelte";
  import { vault } from "$lib/stores/vault.svelte";
  import PrivacyIcon from "../PrivacyIcon.svelte";
  import type { SearchContext } from "./context";
  import SettingRow from "./SettingRow.svelte";
  import SettingSection from "./SettingSection.svelte";

  const search = getContext<SearchContext | undefined>("settings-search");

  /** The raw lines of .vemry/privacy; adding and removing act on the file at once. */
  let rules = $state<string[]>([]);
  let newRule = $state("");
  let ruleError = $state<string | null>(null);
  /** Notes marked one by one: the frontmatter flag is what NoteSummary.private carries, so
   *  the list follows refreshNotes on its own — an unmark here needs no bookkeeping */
  const markedNotes = $derived(vault.notes.filter((n) => n.private).sort((a, b) => compareNames(a.id, b.id)));

  // The rules are read when the page opens, and again whenever the private set changes
  // (a folder marked from the tree, an external edit of .vemry/privacy): that Set is the signal
  $effect(() => {
    void vault.privatePaths;
    if (!untrack(() => vault.root) || search?.query) return;
    api.privacyRules().then((r) => (rules = r)).catch(() => (rules = []));
  });

  async function addRule() {
    const line = newRule.trim();
    if (!line) return;
    ruleError = null;
    try {
      await api.privacyRuleAdd(line);
      rules = await api.privacyRules();
      newRule = "";
    } catch (e) {
      ruleError = String(e);
    }
  }
  async function removeRule(line: string) {
    ruleError = null;
    try {
      await api.privacyRuleRemove(line);
      rules = await api.privacyRules();
    } catch (e) {
      ruleError = String(e);
    }
  }
  /** Open a note from the marked list: the dialog is modal, so it closes first, then the note opens and the tree reveals it. */
  async function openMarked(id: string) {
    settingsUi.open = false;
    await vault.openNote(id);
    vault.revealInTree();
  }
</script>

<SettingSection note={search?.query ? undefined : t("settings.privacyZonesHint")}>
  <SettingRow id="privacy.rules" block error={ruleError}>
    <small class="examples"><code>secret/</code>{t("app.listSep")}<code>diary/**</code>{t("app.listSep")}<code>!diary/Public.md</code></small>
    {#if rules.length}
      <ul class="list">
        <!-- A hand-written .vemry/privacy may hold duplicate lines or duplicate comments, so the text cannot be the key -->
        {#each rules as rule, i (i)}
          <li class:comment={rule.trim().startsWith("#")}>
            <span class="text mono">{rule}</span>
            <button class="del" title={t("settings.deleteRule")} aria-label={t("settings.deleteRule")} onclick={() => removeRule(rule)}>✕</button>
          </li>
        {/each}
      </ul>
    {:else}
      <small>{t("settings.noRules")}</small>
    {/if}
    <div class="add">
      <input
        id="setting-privacy-rules"
        class="mono"
        bind:value={newRule}
        placeholder={t("settings.rulePlaceholder")}
        spellcheck="false"
        onkeydown={(e) => e.key === "Enter" && !isImeKeydown(e) && addRule()}
      />
      <button class="btn" onclick={addRule}>{t("settings.addRule")}</button>
    </div>
  </SettingRow>
  <SettingRow id="privacy.marked" block>
    {#if markedNotes.length}
      <ul class="list">
        {#each markedNotes as n (n.id)}
          <li>
            <span class="lock"><PrivacyIcon size={12} /></span>
            <button class="text note-open" title={t("settings.openNote")} onclick={() => void openMarked(n.id)}>{n.id}</button>
            <button class="del word" title={t("cmd.unmark-private.desc")} onclick={() => void vault.unmarkPrivate(n.id, "note")}>{t("settings.unmarkNote")}</button>
          </li>
        {/each}
      </ul>
    {:else}
      <small>{t("settings.noMarkedNotes")}</small>
    {/if}
  </SettingRow>
</SettingSection>

<style>
  small {
    font-size: var(--fs-xs);
    color: var(--fg-muted);
  }
  .examples {
    display: flex;
    gap: 2px;
    flex-wrap: wrap;
  }
  code,
  .mono {
    font-family: var(--font-mono);
    font-size: var(--fs-xs);
  }
  .list {
    list-style: none;
    margin: 4px 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .list li {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 3px 6px;
    border-radius: 4px;
  }
  .list li:hover {
    background: var(--bg-hover);
  }
  .list li.comment .text {
    color: var(--fg-muted);
  }
  .text {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    text-align: left;
  }
  /* Always in view: removing a rule or a mark is what these lists exist for */
  .del {
    flex: none;
    color: var(--fg-muted);
    font-size: var(--fs-xs);
    padding: 2px 5px;
    border-radius: 4px;
  }
  .del:hover {
    color: var(--danger);
    background: var(--bg-hover);
  }
  .lock {
    display: inline-flex;
    flex-shrink: 0;
    color: var(--accent);
  }
  .note-open {
    font-family: var(--font-mono);
    font-size: var(--fs-xs);
  }
  .note-open:hover {
    color: var(--accent);
  }
  .add {
    display: flex;
    gap: 6px;
    margin-top: 4px;
  }
  .add input {
    flex: 1;
  }
</style>
