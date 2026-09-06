<script lang="ts" module>
  /** The category page ids; App's settings commands pick the page to open on through this */
  export type SettingsTab = "general" | "ai" | "browser" | "vault" | "updates";
</script>

<script lang="ts">
  import { open as openDialog } from "@tauri-apps/plugin-dialog";
  import { api, EFFORT_OPTIONS, type LangSetting, type LinkOpenMode, type SkillEntry, type TypeExample } from "$lib/api";
  import { scaffold } from "$lib/scaffold";
  import { skillDesc } from "$lib/skills";
  import { setLanguage, t } from "$lib/i18n/i18n.svelte";
  import { vault } from "$lib/stores/vault.svelte";
  import { scripts } from "$lib/stores/scripts.svelte";
  import { browser } from "$lib/stores/browser.svelte";
  import { toast } from "$lib/stores/toast.svelte";
  import { prefs } from "$lib/stores/prefs.svelte";
  import { updater } from "$lib/stores/updater.svelte";
  import { isImeKeydown } from "$lib/ime";
  import { k } from "$lib/shortcut";
  import { untrack } from "svelte";
  import PrivacyIcon from "./PrivacyIcon.svelte";

  let { open = $bindable(false), tab = $bindable<SettingsTab>("general") }: { open: boolean; tab?: SettingsTab } =
    $props();

  /** The category navigation on the left; everything on the "Vault" page acts on the vault immediately, so with no vault it is disabled rather than hidden */
  const TABS: { id: SettingsTab; needsVault?: boolean }[] = [
    { id: "general" },
    { id: "ai" },
    { id: "browser" },
    { id: "vault", needsVault: true },
    { id: "updates" },
  ];

  let language = $state<LangSetting>("auto");
  let baseUrl = $state("");
  let model = $state("");
  let apiKey = $state("");
  let envKeyPresent = $state(false);
  let adblockEnabled = $state(true);
  let webSearchEnabled = $state(false);
  let planConfirm = $state(true);
  let provenanceMarks = $state(true);
  let reasoningEffort = $state("high");
  let reasoningActive = $state(false);
  let openLinksIn = $state<LinkOpenMode>("reading");
  let clickOpensLinks = $state(true);
  let readingImagesEnabled = $state(true);
  let clipLocalizeImages = $state(true);
  let formatAgentWrites = $state(true);
  let formatClips = $state(true);
  let formatCjkSpacing = $state(false);
  let formatPunctuation = $state(false);
  let updaterAutoCheck = $state(false);
  let noteFontSize = $state(16);
  let readableLineWidth = $state(true);
  let ruleUrls = $state("");
  let updatingRules = $state(false);
  let saved = $state(false);
  /** The raw rule lines of .vemry/privacy; the operations take effect at once and do not follow the dialog's save semantics */
  let rules = $state<string[]>([]);
  /** This vault's persistent write policy: "allow" = AI writes are pre-authorised (allow_always was granted). */
  let writePolicy = $state<"allow" | "ask">("ask");
  /** The skills list: the built-ins and the vault's own files (disabled ones included; a switch applies from the next turn). */
  let skills = $state<SkillEntry[]>([]);
  /** The name typed for a new skill. */
  let newSkill = $state("");
  /** The example block types (product-document blocks) Settings can install. */
  let typeExamples = $state<TypeExample[]>([]);
  let newRule = $state("");
  let ruleError = $state<string | null>(null);
  /** Notes marked one by one: the frontmatter flag is what NoteSummary.private carries, and the
   *  list follows refreshNotes on its own — an unmark here needs no local bookkeeping */
  const markedNotes = $derived(vault.notes.filter((n) => n.private).sort((a, b) => a.id.localeCompare(b.id, "zh")));
  /** There are unsaved changes: a stray click on the overlay should not evaporate a half-pasted key */
  let dirty = $state(false);
  /** Whether the mouse was pressed on the overlay. A click's target is the common ancestor
   *  of the mousedown and the mouseup, so dragging to select text in an input and releasing
   *  outside the dialog lands the click on the overlay */
  let pressedOverlay = false;
  let nudge = $state(false);
  let firstInput = $state<HTMLInputElement | null>(null);

  $effect(() => {
    if (!open) return;
    // The fallback for having stopped on a needsVault page last time and opening with no
    // vault this time. untrack: this loading effect should only rerun with open — were tab
    // or vault.root a dependency, switching pages or changing vaults with the dialog open
    // would rerun it, resetting dirty and letting settingsGet overwrite the draft (a
    // half-pasted key evaporating).
    const root = untrack(() => vault.root);
    if (untrack(() => TABS.find((tb) => tb.id === tab)?.needsVault) && !root) tab = "general";
    saved = false;
    dirty = false;
    api.settingsGet().then((s) => {
      if (dirty) return; // the user started typing before the request returned, so it is not overwritten
      baseUrl = s.base_url;
      model = s.model;
      apiKey = s.api_key;
      envKeyPresent = s.env_key_present;
      language = s.language;
      adblockEnabled = s.adblock_enabled;
      webSearchEnabled = s.web_search_enabled;
      planConfirm = s.plan_confirm;
      provenanceMarks = s.provenance_marks;
      reasoningEffort = s.reasoning_effort;
      reasoningActive = s.reasoning_effort_active;
      openLinksIn = s.open_links_in;
      clickOpensLinks = s.click_opens_links;
      readingImagesEnabled = s.reading_images_enabled;
      clipLocalizeImages = s.clip_localize_images;
      formatAgentWrites = s.format_agent_writes;
      formatClips = s.format_clips;
      formatCjkSpacing = s.format_cjk_spacing;
      formatPunctuation = s.format_punctuation;
      updaterAutoCheck = s.updater_auto_check;
      noteFontSize = s.note_font_size;
      readableLineWidth = s.readable_line_width;
      ruleUrls = s.adblock_rule_urls.join("\n");
    });
    // Opening it allows pasting straight away, and only then does Escape have a focus to land on
    queueMicrotask(() => firstInput?.focus());
    ruleError = null;
    newRule = "";
    if (root) {
      api.privacyRules().then((r) => (rules = r)).catch(() => (rules = []));
      void scripts.refresh();
      api.vaultWritePolicyGet().then((p) => (writePolicy = p)).catch(() => (writePolicy = "ask"));
      api.skillsList().then((s) => (skills = s)).catch(() => (skills = []));
      api.typeExamplesList().then((s) => (typeExamples = s)).catch(() => (typeExamples = []));
    }
  });

  /** The rule lines are local state fetched on open; a change made elsewhere while the dialog
   *  is up (a folder marked from the tree, an external edit of .vemry/privacy) arrives as a new
   *  privatePaths Set, which is the refetch signal. untrack keeps this independent of the open
   *  effect's dirty/draft semantics. */
  $effect(() => {
    void vault.privatePaths;
    if (!untrack(() => open && vault.root)) return;
    api.privacyRules().then((r) => (rules = r)).catch(() => {});
  });

  /** Revoke "always allow for this vault": back to approving each write (a running session's grant is withdrawn with it). */
  async function revokeWritePolicy() {
    try {
      await api.vaultWritePolicySet("ask");
      writePolicy = "ask";
      toast.push(t("settings.writePolicyRevoked"));
    } catch (e) {
      toast.push(String(e), "error");
    }
  }

  /** The skill rows' badge: which layer the row is, in words. */
  function sourceLabel(s: SkillEntry): string {
    return t(s.source === "builtin" ? "settings.skillBuiltin" : s.source === "override" ? "settings.skillOverride" : "settings.skillCustom");
  }
  /** Opening a skill file in the editor closes the dialog: the editor is behind it. */
  async function openSkill(file: string) {
    open = false;
    await vault.openFile(`.vemry/skills/${file}`);
  }
  async function customizeSkill(name: string) {
    open = false;
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
    open = false;
    await scaffold(() => api.skillCreate(name));
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
  async function toggleSkill(name: string, enabled: boolean) {
    try {
      await api.skillSetEnabled(name, enabled);
      skills = await api.skillsList();
    } catch (e) {
      toast.push(String(e), "error");
    }
  }

  /** Every script operation has the same shape: call the api → refetch the list, with a failure going to a toast. */
  async function withRefresh(action: () => Promise<void>) {
    try {
      await action();
      await scripts.refresh();
    } catch (e) {
      toast.push(String(e), "error");
    }
  }
  const toggleScript = (name: string, enabled: boolean) => withRefresh(() => api.scriptSetEnabled(name, enabled));
  const revokeGrant = (name: string) => withRefresh(() => api.scriptGrantRevoke(name));
  const reloadScripts = () => withRefresh(() => api.scriptsReload());

  /** The triggers at a glance: how many commands / event subscriptions / scheduled tasks */
  function scriptSummary(s: (typeof scripts.infos)[number]): string {
    const parts: string[] = [];
    if (s.commands.length) parts.push(t("settings.scriptCommands", { n: s.commands.length }));
    if (s.event_triggers.length) parts.push(t("settings.scriptEvents", { list: [...new Set(s.event_triggers)].join(", ") }));
    if (s.schedules.length) parts.push(t("settings.scriptSchedules", { list: s.schedules.join(" | ") }));
    return parts.length ? parts.join(" · ") : t("settings.scriptNothing");
  }

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

  /** Open a note from the marked list: the dialog is modal, so it closes first (with the same
   *  unsaved-changes nudge as a click outside), then the note opens and the tree reveals it */
  async function openMarked(id: string) {
    if (dirty) {
      nudge = true;
      setTimeout(() => (nudge = false), 300);
      return;
    }
    open = false;
    await vault.openNote(id);
    vault.revealInTree();
  }

  /** Only "pressed and released on the overlay" really counts as clicking outside; with
   *  unsaved changes even that does not close it, so switching to the browser and back and
   *  clicking once does not lose the content — the nudge says the dialog is deliberately
   *  staying. Cancel and Escape are unambiguous intent and close as usual. */
  function overlayClick() {
    if (!pressedOverlay) return;
    if (dirty) {
      nudge = true;
      setTimeout(() => (nudge = false), 300);
      return;
    }
    open = false;
  }

  async function save() {
    await api.settingsSet({
      language,
      base_url: baseUrl.trim(),
      model: model.trim(),
      api_key: apiKey.trim(),
      adblock_enabled: adblockEnabled,
      web_search_enabled: webSearchEnabled,
      plan_confirm: planConfirm,
      provenance_marks: provenanceMarks,
      reasoning_effort: reasoningEffort,
      open_links_in: openLinksIn,
      click_opens_links: clickOpensLinks,
      reading_images_enabled: readingImagesEnabled,
      clip_localize_images: clipLocalizeImages,
      format_agent_writes: formatAgentWrites,
      format_clips: formatClips,
      format_cjk_spacing: formatCjkSpacing,
      format_punctuation: formatPunctuation,
      updater_auto_check: updaterAutoCheck,
      note_font_size: noteFontSize,
      readable_line_width: readableLineWidth,
      adblock_rule_urls: ruleUrls.split("\n").map((u) => u.trim()).filter(Boolean),
    });
    // The language takes effect at once (each window's frontend has its own $state; the Rust side already rebuilt the menu inside settings_set)
    setLanguage(language);
    updater.autoCheck = updaterAutoCheck; // the update dialog's checkbox mirrors the same setting
    void prefs.refresh(); // the in-process mirror of the editor's click gestures: after a successful write it is read back from the single source of truth rather than copied field by field
    saved = true;
    dirty = false;
    setTimeout(() => (open = false), 400);
  }

  /** Immediate semantics (as with the privacy rules): the rule sources are downloaded and recompiled without going through the save button. */
  async function updateRules() {
    updatingRules = true;
    try {
      // Unsaved changes to the rule sources reach disk first, so the update actually uses them
      await api.settingsSet({ adblock_rule_urls: ruleUrls.split("\n").map((u) => u.trim()).filter(Boolean) });
      const failures = await api.adblockUpdateRules();
      // A partial download is a success (the sources are independent lists), but the
      // sources that did not come through have to be named — silence would read as
      // "everything is up to date"
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

  /** The escape hatch for a network that cannot reach the subscription hosts: pick rule files downloaded by hand. */
  async function importRules() {
    let picked: string | string[] | null;
    try {
      picked = await openDialog({
        multiple: true,
        title: t("settings.importRulesTitle"),
        filters: [{ name: t("settings.ruleFileFilter"), extensions: ["txt"] }],
      });
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

<!-- Window level: after a click on the overlay the focus lands on body, and a keydown inside the dialog would no longer receive Escape -->
<svelte:window onkeydown={(e) => open && e.key === "Escape" && (open = false)} />

{#if open}
  <div
    class="overlay"
    onmousedown={(e) => (pressedOverlay = e.target === e.currentTarget)}
    onclick={overlayClick}
    role="presentation"
  >
    <!-- A click inside the dialog needs no stopPropagation: pressedOverlay already ensures
      only a click whose press landed on the overlay counts as "clicking outside" -->
    <div class="dialog" class:nudge role="dialog" aria-label={t("settings.title")} tabindex="-1">
      <header>
        <h2>{t("settings.title")}</h2>
      </header>
      <div class="body">
        <nav class="side" aria-label={t("settings.categories")}>
          {#each TABS as tb (tb.id)}
            <button
              class="side-item"
              class:active={tab === tb.id}
              disabled={tb.needsVault && !vault.root}
              title={tb.needsVault && !vault.root ? t("settings.needsVault") : undefined}
              aria-current={tab === tb.id ? "true" : undefined}
              onclick={() => (tab = tb.id)}
            >{t(`settings.tab.${tb.id}`)}</button>
          {/each}
        </nav>
        <div class="panel">
          {#if tab === "general"}
            <label>
              <span>{t("settings.language.label")}</span>
              <select bind:value={language} onchange={() => (dirty = true)}>
                <option value="auto">{t("settings.language.auto")}</option>
                <option value="zh-CN">{t("settings.language.zh")}</option>
                <option value="en">{t("settings.language.en")}</option>
              </select>
              <small>{t("settings.language.hint")}</small>
            </label>
            <label>
              <span>{t("settings.endpoint")}</span>
              <input
                bind:this={firstInput}
                bind:value={baseUrl}
                oninput={() => (dirty = true)}
                placeholder="https://open.bigmodel.cn/api/paas/v4"
                spellcheck="false"
                list="base-url-presets"
              />
              <datalist id="base-url-presets">
                <option value="https://open.bigmodel.cn/api/paas/v4">{t("settings.endpointZhipu")}</option>
                <option value="https://api.deepseek.com">{t("settings.endpointDeepseek")}</option>
                <option value="http://localhost:11434/v1">{t("settings.endpointOllama")}</option>
              </datalist>
              <small>{t("settings.endpointHint")}</small>
            </label>
            <label>
              <span>{t("settings.model")}</span>
              <input
                bind:value={model}
                oninput={() => (dirty = true)}
                placeholder="glm-4.7-flash"
                spellcheck="false"
                list="model-presets"
              />
              <datalist id="model-presets">
                <option value="glm-4.7-flash">{t("settings.modelZhipuFlash")}</option>
                <option value="glm-4.7">{t("settings.modelZhipu")}</option>
                <option value="deepseek-v4-flash">{t("settings.modelDeepseekFlash")}</option>
                <option value="deepseek-chat">{t("settings.modelDeepseekChat")}</option>
                <option value="deepseek-reasoner">{t("settings.modelDeepseekReasoner")}</option>
              </datalist>
              <small>{t("settings.modelHint")}</small>
            </label>
            {#if vault.root && writePolicy === "allow"}
              <div class="grant-row">
                <button class="grant-badge" title={t("settings.revokeWritePolicyHint")} onclick={revokeWritePolicy}>
                  {t("settings.revokeWritePolicy")}
                </button>
                <small>{t("settings.revokeWritePolicyNote")}</small>
              </div>
            {/if}
            <label>
              <span>{t("settings.apiKey")}</span>
              <input
                bind:value={apiKey}
                oninput={() => (dirty = true)}
                type="password"
                placeholder={t("settings.apiKeyPlaceholder")}
                spellcheck="false"
              />
              {#if envKeyPresent}
                <small class="ok">{t("settings.apiKeyEnv")}</small>
              {:else}
                <small>{t("settings.apiKeyNote")}</small>
              {/if}
            </label>
            <label>
              <span>{t("settings.fontSize")}</span>
              <select bind:value={noteFontSize} onchange={() => (dirty = true)}>
                <option value={15}>{t("settings.fontSizeCompact")}</option>
                <option value={16}>{t("settings.fontSizeDefault")}</option>
                <option value={17}>17 px</option>
                <option value={18}>{t("settings.fontSizeRoomy")}</option>
              </select>
              <small>{t("settings.fontSizeHint")}</small>
            </label>
            <div class="field">
              <label class="inline-toggle">
                <input type="checkbox" bind:checked={readableLineWidth} onchange={() => (dirty = true)} />
                <span>{t("settings.readableLineWidth")}</span>
              </label>
              <small>
                {t("settings.readableLineWidthHint")}
              </small>
            </div>
          {:else if tab === "ai"}
            <div class="field">
              <label class="inline-toggle">
                <input type="checkbox" bind:checked={webSearchEnabled} onchange={() => (dirty = true)} />
                <span>{t("settings.webSearch")}</span>
              </label>
              <small>
                {t("settings.webSearchHint")}
              </small>
            </div>
            <div class="field">
              <label class="inline-toggle">
                <input type="checkbox" bind:checked={planConfirm} onchange={() => (dirty = true)} />
                <span>{t("settings.planConfirm")}</span>
              </label>
              <small>
                {t("settings.planConfirmHint")}
              </small>
            </div>
            <div class="field">
              <label class="inline-toggle">
                <input type="checkbox" bind:checked={provenanceMarks} onchange={() => (dirty = true)} />
                <span>{t("settings.provenanceMarks")}</span>
              </label>
              <small>
                {t("settings.provenanceMarksHint")}
              </small>
            </div>
            <div class="field">
              <label for="reasoning-effort">{t("settings.reasoningEffort")}</label>
              <select id="reasoning-effort" bind:value={reasoningEffort} onchange={() => (dirty = true)}>
                {#each EFFORT_OPTIONS as o (o.value)}
                  {@const hint = t(`settings.effort.${o.value}Hint`)}
                  <option value={o.value}
                    >{t(`settings.effort.${o.value}`)}{hint
                      ? `(${hint})`
                      : o.isDefault
                        ? t("settings.reasoningDefaultSuffix")
                        : ""}</option
                  >
                {/each}
              </select>
              <small>
                {t("settings.reasoningHint")}{#if !reasoningActive}{t("settings.reasoningInactive")}{/if}
              </small>
            </div>
          {:else if tab === "updates"}
            <section class="privacy">
              <h3>{t("settings.appUpdates")}</h3>
              <div class="update-row">
                <small>{t("settings.currentVersion", { version: updater.current || "…" })}</small>
                {#if updater.phase === "downloading"}
                  <button class="btn small" disabled>
                    {t("settings.downloading")}{updater.total ? ` ${Math.round((updater.progress / updater.total) * 100)}%` : ""}
                  </button>
                {:else if updater.phase === "available"}
                  <!-- The dialog carries the install (its progress and the restart): this only brings it back -->
                  <button class="btn small" onclick={() => updater.show()}>{t("settings.downloadInstall", { version: updater.version })}</button>
                {:else if updater.phase === "ready"}
                  <button class="btn small" onclick={() => updater.restart()}>{t("settings.restartToFinish")}</button>
                {:else}
                  <button class="btn small" disabled={updater.phase === "checking"} onclick={() => updater.check(true)}>
                    {updater.phase === "checking" ? t("settings.checking") : t("cmd.check-updates.title")}
                  </button>
                {/if}
              </div>
              {#if updater.phase === "uptodate"}
                <small class="ok">{t("settings.upToDate")}</small>
              {:else if updater.phase === "available"}
                <small class="ok">{t("settings.updateAvailable", { version: updater.version })}{updater.notes ? ` — ${updater.notes}` : ""}</small>
              {:else if updater.phase === "ready"}
                <small class="ok">{t("settings.installedRestart")}</small>
              {:else if updater.phase === "error"}
                <small class="err">{updater.error}</small>
              {/if}
              <label class="inline-toggle">
                <input type="checkbox" bind:checked={updaterAutoCheck} onchange={() => (dirty = true)} />
                <span>{t("settings.autoCheck")}</span>
              </label>
              <small>
                {t("settings.autoCheckHint")}
              </small>
            </section>
          {:else if tab === "browser"}
            <section class="privacy">
              <h3>{t("settings.tab.browser")}</h3>
              <label class="inline-toggle">
                <input type="checkbox" bind:checked={adblockEnabled} onchange={() => (dirty = true)} />
                <span>{t("settings.adblock")}</span>
              </label>
              {#if browser.adblock?.state === "ready"}
                <small class="ok">{t("settings.adblockActive", { n: browser.adblock.rules.toLocaleString() })}</small>
              {:else if browser.adblock?.state === "downloading"}
                <small>{t("settings.adblockDownloading")}</small>
              {:else if browser.adblock?.state === "compiling"}
                <small>{t("settings.adblockCompiling")}</small>
              {:else if browser.adblock?.state === "no_rules"}
                <small>{t("settings.adblockNoRules")}</small>
              {:else if browser.adblock?.state === "failed"}
                <small class="err">{browser.adblock.error}</small>
              {/if}
              <label>
                <span>{t("settings.openLinksIn")}</span>
                <select bind:value={openLinksIn} onchange={() => (dirty = true)}>
                  <option value="reading">{t("settings.openLinksReading")}</option>
                  <option value="internal">{t("settings.openLinksInternal")}</option>
                  <option value="system">{t("settings.openLinksSystem")}</option>
                </select>
                <small>{t("settings.openLinksHint", { key: k("Mod+click") })}</small>
              </label>
              <div class="field">
                <label class="inline-toggle">
                  <input type="checkbox" bind:checked={clickOpensLinks} onchange={() => (dirty = true)} />
                  <span>{t("settings.clickOpensLinks")}</span>
                </label>
                <small>
                  {t("settings.clickOpensLinksHint", { modClick: k("Mod+click"), modEnter: k("Mod+Enter") })}
                </small>
              </div>
              <div class="field">
                <label class="inline-toggle">
                  <input type="checkbox" bind:checked={readingImagesEnabled} onchange={() => (dirty = true)} />
                  <span>{t("settings.readingImages")}</span>
                </label>
                <small>
                  {t("settings.readingImagesHint")}
                </small>
              </div>
              <div class="field">
                <label class="inline-toggle">
                  <input type="checkbox" bind:checked={clipLocalizeImages} onchange={() => (dirty = true)} />
                  <span>{t("settings.clipLocalizeImages")}</span>
                </label>
                <small>
                  {t("settings.clipLocalizeImagesHint")}
                </small>
              </div>
              <h4>{t("settings.formatSection")}</h4>
              <div class="field">
                <label class="inline-toggle">
                  <input type="checkbox" bind:checked={formatAgentWrites} onchange={() => (dirty = true)} />
                  <span>{t("settings.formatAgentWrites")}</span>
                </label>
                <small>{t("settings.formatAgentWritesHint")}</small>
              </div>
              <div class="field">
                <label class="inline-toggle">
                  <input type="checkbox" bind:checked={formatClips} onchange={() => (dirty = true)} />
                  <span>{t("settings.formatClips")}</span>
                </label>
                <small>{t("settings.formatClipsHint")}</small>
              </div>
              <div class="field">
                <label class="inline-toggle">
                  <input type="checkbox" bind:checked={formatCjkSpacing} onchange={() => (dirty = true)} />
                  <span>{t("settings.formatCjkSpacing")}</span>
                </label>
                <label class="inline-toggle">
                  <input type="checkbox" bind:checked={formatPunctuation} onchange={() => (dirty = true)} />
                  <span>{t("settings.formatPunctuation")}</span>
                </label>
                <small>{t("settings.formatOptInHint")}</small>
              </div>
              <label>
                <span>{t("settings.ruleUrls")}</span>
                <textarea rows="2" bind:value={ruleUrls} oninput={() => (dirty = true)} spellcheck="false"></textarea>
                <small>
                  {t("settings.ruleUrlsHint")}
                  <button class="btn small" disabled={updatingRules} onclick={updateRules}>
                    {updatingRules ? t("settings.updatingRules") : t("settings.updateRules")}
                  </button>
                  <button class="btn small" disabled={updatingRules} onclick={importRules}>
                    {t("settings.importRules")}
                  </button>
                </small>
              </label>
            </section>
          {:else if tab === "vault"}
            <section class="privacy">
              <h3>{t("settings.privacyZones")}</h3>
              <small>{t("settings.privacyZonesHint")}</small>
              <h4>{t("settings.privacyRules")}</h4>
              <small>
                {t("settings.privacyRulesHint")} <code>secret/</code>{t("app.listSep")}<code>diary/**</code>{t("app.listSep")}<code>!diary/Public.md</code>
              </small>
              {#if rules.length}
                <ul class="rules">
                  <!-- A hand-written .vemry/privacy may hold duplicate lines or duplicate comments, so the text cannot be the key -->
                  {#each rules as rule, i (i)}
                    <li class:comment={rule.trim().startsWith("#")}>
                      <span class="rule-text">{rule}</span>
                      <button class="rule-del" title={t("settings.deleteRule")} onclick={() => removeRule(rule)}>✕</button>
                    </li>
                  {/each}
                </ul>
              {:else}
                <small class="empty">{t("settings.noRules")}</small>
              {/if}
              <div class="rule-add">
                <input
                  bind:value={newRule}
                  placeholder={t("settings.rulePlaceholder")}
                  spellcheck="false"
                  onkeydown={(e) => e.key === "Enter" && !isImeKeydown(e) && addRule()}
                />
                <button class="btn" onclick={addRule}>{t("settings.addRule")}</button>
              </div>
              {#if ruleError}<small class="err">{ruleError}</small>{/if}
              <h4>{t("settings.privacyMarkedNotes")}</h4>
              {#if markedNotes.length}
                <ul class="rules marked">
                  {#each markedNotes as n (n.id)}
                    <li>
                      <span class="lock"><PrivacyIcon size={12} /></span>
                      <button class="rule-text note-open" title={t("settings.openNote")} onclick={() => void openMarked(n.id)}>{n.id}</button>
                      <button class="rule-del" title={t("cmd.unmark-private.desc")} onclick={() => void vault.unmarkPrivate(n.id, "note")}>{t("settings.unmarkNote")}</button>
                    </li>
                  {/each}
                </ul>
              {:else}
                <small class="empty">{t("settings.noMarkedNotes")}</small>
              {/if}
            </section>
            <section class="scripts">
              <h3>{t("settings.skills")}</h3>
              <small>{t("settings.skillsHint")}</small>
              <ul class="script-list">
                {#each skills as s (s.name)}
                  <li>
                    <div class="script-row">
                      <label class="script-toggle">
                        <input type="checkbox" checked={!s.disabled} onchange={() => toggleSkill(s.name, s.disabled)} />
                        <span class="script-name">{s.name}</span>
                      </label>
                      <span class="source-badge" class:custom={s.source !== "builtin"}>{sourceLabel(s)}</span>
                      {#if s.outdated}
                        <span class="source-badge outdated" title={t("settings.skillOutdatedHint")}>{t("settings.skillOutdated")}</span>
                      {/if}
                      {#if s.source === "builtin"}
                        <button class="btn small" onclick={() => void customizeSkill(s.name)}>{t("settings.skillCustomize")}</button>
                      {:else}
                        <button class="btn small" onclick={() => void openSkill(s.file)}>{t("settings.edit")}</button>
                        {#if s.source === "override"}
                          <button class="btn small" onclick={() => void restoreSkill(s.name)}>{t("settings.skillRestore")}</button>
                        {/if}
                      {/if}
                    </div>
                    {#if skillDesc(s)}
                      <small class="script-summary">{skillDesc(s)}</small>
                    {/if}
                  </li>
                {/each}
              </ul>
              <div class="rule-add">
                <input
                  bind:value={newSkill}
                  placeholder={t("settings.skillNewPlaceholder")}
                  spellcheck="false"
                  onkeydown={(e) => e.key === "Enter" && !isImeKeydown(e) && void createSkill()}
                />
                <button class="btn" onclick={() => void createSkill()}>{t("settings.skillNew")}</button>
              </div>
              {#if typeExamples.some((e) => !e.installed)}
                <h4 class="examples-title">{t("settings.typeExamples")}</h4>
                <small>{t("settings.typeExamplesHint")}</small>
                <ul class="script-list">
                  {#each typeExamples.filter((e) => !e.installed) as e (e.name)}
                    <li>
                      <div class="script-row">
                        <span class="script-name">{e.name}</span>
                        <button class="btn small" onclick={() => void installTypeExample(e.name)}>{t("settings.installSkill")}</button>
                      </div>
                      <small class="script-summary">{e.description}</small>
                    </li>
                  {/each}
                </ul>
              {/if}
            </section>
            <section class="scripts">
              <h3>
                {t("settings.userScripts")}
                <button class="btn small" onclick={reloadScripts}>{t("settings.reloadScripts")}</button>
              </h3>
              <small>
                {t("settings.userScriptsHint1")}<code>.vemry/scripts/*.js</code
                >{t("settings.userScriptsHint2")}<code>vemry.*</code>{t("settings.userScriptsHint3")}
              </small>
              {#if scripts.infos.length}
                <ul class="script-list">
                  {#each scripts.infos as s (s.name)}
                    <li>
                      <div class="script-row">
                        <label class="script-toggle">
                          <input type="checkbox" checked={s.enabled} onchange={() => toggleScript(s.name, !s.enabled)} />
                          <span class="script-name">{s.name}</span>
                        </label>
                        {#if s.write_grant}
                          <button class="grant-badge" title={t("settings.revokeGrant")} onclick={() => revokeGrant(s.name)}>
                            {t("settings.writable")}
                          </button>
                        {/if}
                      </div>
                      {#if s.enabled}
                        <small class="script-summary">{scriptSummary(s)}</small>
                      {/if}
                      {#if s.last_error}
                        <small class="err">{s.last_error}</small>
                      {/if}
                    </li>
                  {/each}
                </ul>
              {:else}
                <small class="empty">{t("settings.noScripts")}</small>
              {/if}
            </section>
          {/if}
        </div>
      </div>
      <footer>
        <button class="btn" onclick={() => (open = false)}>{t("common.cancel")}</button>
        <button class="btn primary" onclick={save}>{saved ? t("settings.saved") : t("settings.save")}</button>
      </footer>
    </div>
  </div>
{/if}

<style>
  .overlay {
    position: fixed;
    inset: 0;
    background: rgba(0, 0, 0, 0.3);
    display: grid;
    place-items: center;
    z-index: 150;
  }
  /* The same skeleton as PermissionDialog: a fixed-height container with overflow:hidden,
     so scrolling happens on the .panel layer alone (the header, the navigation and the
     footer stay put). A fixed height rather than a max-height: the dialog's size does not
     jump when the category page changes. */
  .dialog {
    width: min(640px, 92vw);
    height: min(600px, 80vh);
    background: var(--bg-panel);
    border: 1px solid var(--border);
    border-radius: 10px;
    box-shadow: var(--shadow);
    display: flex;
    flex-direction: column;
    overflow: hidden;
  }
  header {
    flex-shrink: 0;
    padding: 14px 20px 10px;
    border-bottom: 1px solid var(--border);
  }
  .body {
    flex: 1;
    display: flex;
    min-height: 0;
  }
  .side {
    flex: 0 0 128px;
    display: flex;
    flex-direction: column;
    gap: 2px;
    padding: 10px 8px;
    border-right: 1px solid var(--border);
    overflow-y: auto;
  }
  .side-item {
    text-align: left;
    font-size: var(--fs-sm);
    color: var(--fg-muted);
    padding: 6px 10px;
    border-radius: var(--radius);
  }
  .side-item:hover:not(:disabled) {
    background: var(--bg-hover);
    color: var(--fg);
  }
  .side-item.active {
    background: var(--accent-soft);
    color: var(--accent);
    font-weight: 600;
  }
  .side-item:disabled {
    opacity: 0.45;
    cursor: default;
  }
  .panel {
    flex: 1;
    min-width: 0;
    overflow-y: auto;
    padding: 16px 20px;
    display: flex;
    flex-direction: column;
    gap: 14px;
  }
  /* The first section of a page sits flush (the divider only means something between sections on one page) */
  .panel > section:first-child {
    border-top: none;
    padding-top: 0;
  }
  /* The container grouping a switch with its explanation: the hint belongs to its own control and does not take .panel's large gap */
  .field {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  /* Clicking the overlay with unsaved changes: a nudge, saying the dialog is deliberately staying */
  .dialog.nudge {
    animation: nudge 0.28s ease-in-out;
  }
  @keyframes nudge {
    0%, 100% { transform: translateX(0); }
    25% { transform: translateX(-5px); }
    75% { transform: translateX(5px); }
  }
  @media (prefers-reduced-motion: reduce) {
    .dialog.nudge {
      animation: none;
    }
  }
  h2 {
    font-size: var(--fs-lg);
    font-weight: 600;
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
  small {
    font-size: var(--fs-xs);
    color: var(--fg-faint);
  }
  small.ok {
    color: var(--accent);
  }
  .inline-toggle {
    flex-direction: row !important;
    align-items: center;
    gap: 8px !important;
    cursor: pointer;
  }
  .inline-toggle input {
    padding: 0;
  }
  .inline-toggle span {
    font-weight: 500;
  }
  select,
  textarea {
    padding: 7px 10px;
    font-size: var(--fs-sm);
    border: 1px solid var(--border);
    border-radius: 6px;
    background: var(--bg);
    color: var(--fg);
  }
  textarea {
    font-family: var(--font-mono);
    font-size: var(--fs-xs);
    resize: vertical;
  }
  .privacy {
    display: flex;
    flex-direction: column;
    gap: 6px;
    border-top: 1px solid var(--border);
    padding-top: 14px;
  }
  .privacy h3 {
    font-size: var(--fs-sm);
    font-weight: 600;
  }
  .privacy h4 {
    margin-top: 8px;
    font-size: var(--fs-xs);
    font-weight: 600;
    color: var(--fg-muted);
  }
  .privacy code {
    font-family: var(--font-mono);
    font-size: var(--fs-xs);
  }
  /* No inner scrolling of its own any more: .panel is the one scrolling surface, and a nested scrollbar only makes the wheel's target uncertain */
  .rules {
    list-style: none;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .rules li {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 3px 6px;
    border-radius: 4px;
  }
  .rules li:hover {
    background: var(--bg-hover);
  }
  .rules li.comment .rule-text {
    color: var(--fg-faint);
  }
  .rule-text {
    flex: 1;
    font-family: var(--font-mono);
    font-size: var(--fs-xs);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .rule-del {
    color: var(--fg-faint);
    font-size: var(--fs-xs);
    padding: 2px 5px;
    border-radius: 4px;
    visibility: hidden;
  }
  .rules li:hover .rule-del {
    visibility: visible;
  }
  .rule-del:hover {
    color: var(--danger);
    background: var(--bg-hover);
  }
  .rules .lock {
    display: inline-flex;
    flex-shrink: 0;
    color: var(--accent);
  }
  .note-open {
    text-align: left;
  }
  .note-open:hover {
    color: var(--accent);
  }
  /* Unlike a rule's ✕, the unmark is always visible: it is the action this list exists for */
  .rules.marked .rule-del {
    visibility: visible;
  }
  .update-row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
  }
  .rule-add {
    display: flex;
    gap: 6px;
  }
  .rule-add input {
    flex: 1;
    font-family: var(--font-mono);
  }
  .privacy .empty {
    color: var(--fg-faint);
  }
  .privacy .err {
    color: var(--danger);
  }
  .scripts {
    display: flex;
    flex-direction: column;
    gap: 6px;
    border-top: 1px solid var(--border);
    padding-top: 14px;
  }
  .scripts h3 {
    font-size: var(--fs-sm);
    font-weight: 600;
    display: flex;
    align-items: center;
    justify-content: space-between;
  }
  .scripts code {
    font-family: var(--font-mono);
    font-size: var(--fs-xs);
  }
  .btn.small {
    font-size: var(--fs-xs);
    padding: 3px 10px;
  }
  .script-list {
    list-style: none;
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .script-list li {
    display: flex;
    flex-direction: column;
    gap: 2px;
    padding: 4px 6px;
    border-radius: 4px;
  }
  .script-list li:hover {
    background: var(--bg-hover);
  }
  .script-row {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .script-toggle {
    flex-direction: row !important;
    align-items: center;
    gap: 6px !important;
    flex: 1;
    cursor: pointer;
  }
  .script-toggle input {
    padding: 0;
  }
  .script-name {
    font-family: var(--font-mono);
    font-size: var(--fs-sm) !important;
    font-weight: 500 !important;
  }
  .grant-badge {
    font-size: var(--fs-xs);
    color: var(--accent);
    border: 1px solid var(--accent);
    border-radius: 10px;
    padding: 1px 8px;
  }
  .grant-badge:hover {
    color: var(--danger);
    border-color: var(--danger);
  }
  /* which layer a skill row is: quiet for a built-in, the accent for the user's own file */
  .source-badge {
    font-size: var(--fs-xs);
    color: var(--fg-faint);
    border: 1px solid var(--border);
    border-radius: 10px;
    padding: 1px 8px;
    white-space: nowrap;
  }
  .source-badge.custom {
    color: var(--accent);
    border-color: var(--accent);
  }
  .source-badge.outdated {
    color: var(--fg-muted);
    border-style: dashed;
  }
  .grant-row {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .script-summary {
    color: var(--fg-faint);
  }
  .scripts .empty {
    color: var(--fg-faint);
  }
  .scripts .err {
    color: var(--danger);
    word-break: break-all;
  }
  footer {
    flex-shrink: 0;
    display: flex;
    justify-content: flex-end;
    gap: 8px;
    padding: 12px 20px;
    border-top: 1px solid var(--border);
  }
  .btn {
    font-size: var(--fs-sm);
    padding: 6px 14px;
    border-radius: 6px;
    border: 1px solid var(--border);
  }
  .btn.primary {
    background: var(--accent);
    border-color: var(--accent);
    color: #fff;
  }
</style>
