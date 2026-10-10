// The "models" probe: the provider list, the current model and its pickers, driven the
// way a user would — the settings page, the composer's model menu, the palette, the
// detached chat window — plus, when the first provider can be sent to, two real turns
// that move from one model (thinking off) to another (thinking on) and leave their
// models in the session log. It works in a scratch vault, and puts the settings back as
// it found them before it ends.
import { api, emitProbe, onProbe, type CustomProvider, type ModelRef, type SettingsView } from "$lib/api";
import { executeCommand } from "$lib/commands/registry";
import { t } from "$lib/i18n/i18n.svelte";
import { sameRef, selectModel } from "$lib/models";
import { agent } from "$lib/stores/agent.svelte";
import { contextMenu } from "$lib/stores/contextMenu.svelte";
import { prefs } from "$lib/stores/prefs.svelte";
import { openSettings, settingsUi } from "$lib/stores/settingsUi.svelte";
import { vault } from "$lib/stores/vault.svelte";
import { Probe, sleep } from "./index";

const LEGACY_KEYS = ["base_url", "model", "api_key", "provider", "hosted_model", "translate_model"];
const NEW_KEYS = ["providers", "chat_model", "translation_model"];
const refKey = (r: ModelRef) => JSON.stringify([r.provider, r.model]);

/** The stored fields of a provider row (the view's derived ones left out). */
const stored = ({ id, name, base_url, api_key, models }: CustomProvider): CustomProvider => ({ id, name, base_url, api_key, models });

/** The visible text of the elements a selector finds. */
const texts = (selector: string, root: ParentNode = document) =>
  [...root.querySelectorAll<HTMLElement>(selector)].map((el) => (el.textContent ?? "").replace(/\s+/g, " ").trim());

/** Click the model chip the way a pointer does: the menu opens where the click was, so a
 *  bare element.click() (at 0,0) would put it in the window's corner. */
function clickChip(): boolean {
  const chip = document.querySelector<HTMLButtonElement>(".tool-btn.model");
  if (!chip) return false;
  const r = chip.getBoundingClientRect();
  chip.dispatchEvent(new MouseEvent("click", { bubbles: true, cancelable: true, clientX: r.left + r.width / 2, clientY: r.top }));
  return true;
}

/** The model items of an open menu: everything before "Manage models…" (the thinking level
 *  and the web search switch, also checkable, come after it). */
function modelItems(menu: HTMLElement): HTMLButtonElement[] {
  const items: HTMLButtonElement[] = [];
  for (const el of menu.querySelectorAll<HTMLButtonElement>("button")) {
    if ((el.textContent ?? "").trim() === t("chat.manageModels")) break;
    items.push(el);
  }
  return items;
}

/** Open the composer's model menu in this window and click the item for `model` under the group named `group`. */
async function pickFromMenu(p: Probe, group: string, model: string): Promise<boolean> {
  if (!p.check("the model chip is on screen", clickChip())) return false;
  if (!p.check("the model menu opens", await p.until(() => !!document.querySelector(".menu[role=menu]")))) return false;
  const menu = document.querySelector<HTMLElement>(".menu[role=menu]")!;
  // The item sits after its group's title row
  let inGroup = false;
  for (const el of menu.children) {
    if (el.classList.contains("menu-title")) inGroup = (el.textContent ?? "").trim() === group;
    else if (inGroup && el instanceof HTMLButtonElement && (el.textContent ?? "").trim() === model) {
      el.click();
      return true;
    }
  }
  p.check(`the menu offers ${model} under ${group}`, false, texts(".menu-title, .menu button", menu));
  return false;
}

export async function runMain() {
  const p = new Probe();
  p.log("models probe: main window");
  await p.until(() => !!vault.root, 15000);
  const original: SettingsView = await api.settingsGet();
  const originalVault = vault.root;
  let s = original;
  const first = original.providers[0];

  await p.step("migration", async () => {
    const keys = await api.probeSettingsKeys();
    p.log(`settings.json on disk before any write holds: ${keys.filter((k) => LEGACY_KEYS.includes(k) || NEW_KEYS.includes(k)).join(", ") || "(neither shape)"}`);
    for (const pv of original.providers) {
      p.log(`provider ${pv.id} "${pv.display_name}" ${pv.base_url} key:${pv.api_key ? "set" : "none"}${pv.env_key ? " (env)" : ""} models:[${pv.models.join(", ")}]`);
    }
    p.check("at least one provider after load", original.providers.length > 0);
    p.check("a current model is chosen", !!original.chat_model.provider, original.chat_model);
    p.check("the current model is one the pickers list", original.model_choices.some((g) => g.models.some((m) => sameRef({ provider: g.provider, model: m.id }, original.chat_model))));
    p.log(`current model ${refKey(original.chat_model)}; unready: ${original.chat_unready ?? "no"}`);
  });

  await p.step("scratch vault", async () => {
    const path = await api.probeScratchPath();
    await vault.createVault(path, "probe");
    const name = path.split("/").pop()!;
    p.check("the scratch vault is open", !!vault.root?.endsWith(name), vault.root);
    await api.noteWrite("Apples.md", "# Apples\n\nApples keep best in a cool, dark place.\n");
    await api.noteWrite("Pears.md", "# Pears\n\nPears ripen at room temperature, then go in the fridge.\n");
    await vault.refreshNotes();
  });

  // Two providers beside the user's: a local one with no key, and a twin serving the same
  // model id as the first (with a blank and a repeated id the write should drop)
  const twinModel = first?.models[0] ?? "deepseek-flash";
  const proModel = "deepseek-v4-pro";
  const firstIsDeepseek = !!first && /(^|\.)deepseek\.com/.test(new URL(first.base_url.includes("://") ? first.base_url : `https://${first.base_url}`).host);
  let ollamaId = "";
  let twinId = "";

  await p.step("providers", async () => {
    const rows: CustomProvider[] = original.providers.map(stored);
    if (firstIsDeepseek && !rows[0].models.includes(proModel)) rows[0] = { ...rows[0], models: [...rows[0].models, proModel] };
    rows.push({ id: "", name: "", base_url: "http://localhost:11434/v1", api_key: "", models: ["qwen3:14b"] });
    rows.push({ id: "", name: "Probe twin", base_url: "http://127.0.0.1:9/v1", api_key: "probe-key", models: [twinModel, ` ${twinModel} `, "  "] });
    await api.settingsSet({ providers: rows });
    s = await api.settingsGet();
    const n = original.providers.length;
    p.check("two providers added", s.providers.length === n + 2, s.providers.map((x) => x.id));
    const ids = s.providers.map((x) => x.id);
    p.check("every provider has its own id", ids.every((id) => /^p\d+$/.test(id)) && new Set(ids).size === ids.length, ids);
    p.check("existing ids are kept", original.providers.every((x, i) => ids[i] === x.id));
    const ollama = s.providers[n];
    const twin = s.providers[n + 1];
    ollamaId = ollama.id;
    twinId = twin.id;
    p.check("an unnamed local provider is named after its vendor", ollama.display_name === "Ollama", ollama.display_name);
    p.check("a typed name wins", twin.display_name === "Probe twin", twin.display_name);
    p.check("blank and repeated model ids are dropped", JSON.stringify(twin.models) === JSON.stringify([twinModel]), twin.models);
    const keys = await api.probeSettingsKeys();
    p.check("after a write the file holds the new shape", NEW_KEYS.slice(0, 2).every((k) => keys.includes(k)), keys.filter((k) => NEW_KEYS.includes(k)));
    p.check("after a write the file holds none of the old keys", !keys.some((k) => LEGACY_KEYS.includes(k)), keys.filter((k) => LEGACY_KEYS.includes(k)));
    p.check("the pickers list the new providers", s.model_choices.some((g) => g.provider === ollamaId) && s.model_choices.some((g) => g.provider === twinId));
  });

  await p.step("settings page", async () => {
    openSettings("ai.providers");
    const cards = await p.until(() => document.querySelectorAll(".providers li").length === s.providers.length, 5000);
    p.check("one card per provider", cards, document.querySelectorAll(".providers li").length);
    const names = texts(".providers li .name");
    p.check("the cards carry the providers' names", names.includes("Ollama") && names.includes("Probe twin"), names);
    const select = document.querySelector<HTMLSelectElement>("#setting-ai-model");
    p.check("the current-model select shows the current model", select?.value === refKey(s.chat_model), select?.value);
    const groups = [...(select?.querySelectorAll("optgroup") ?? [])].map((g) => g.label);
    p.check("the select groups models by provider", groups.includes("Ollama") && groups.includes("Probe twin"), groups);
    await sleep(600);
    await p.shot("1-settings-providers");
    document.querySelector('[data-setting="ai.model"]')?.scrollIntoView({ block: "start" });
    await sleep(300);
    await p.shot("2-settings-current-model");
    settingsUi.open = false;
  });

  await p.step("composer menu", async () => {
    void executeCommand("show-agent");
    await p.until(() => !!document.querySelector(".tool-btn.model"), 5000);
    const chip = document.querySelector<HTMLElement>(".tool-btn.model .model-name");
    // The first provider's model id is served by the twin too, so the chip names its provider
    if (sameRef(s.chat_model, { provider: first.id, model: twinModel })) {
      p.check("a model id two providers serve is shown with its provider", (chip?.textContent ?? "").includes(` · ${first.display_name}`), chip?.textContent);
    }
    clickChip();
    await p.until(() => !!document.querySelector(".menu[role=menu]"));
    const menu = document.querySelector<HTMLElement>(".menu[role=menu]");
    const titles = menu ? texts(".menu-title", menu) : [];
    p.check("the menu has a group per provider", titles.includes("Ollama") && titles.includes("Probe twin") && titles.includes(first.display_name), titles);
    const checked = menu ? modelItems(menu).filter((b) => b.getAttribute("aria-checked") === "true").map((b) => (b.textContent ?? "").trim()) : [];
    p.check("the current model alone is ticked", checked.length === 1 && checked[0] === s.chat_model.model, checked);
    p.check("the menu leads to Settings", !!menu && texts("button", menu).includes(t("chat.manageModels")));
    await p.shot("3-composer-menu");
    contextMenu.close();
    await sleep(200);
    if (await pickFromMenu(p, "Ollama", "qwen3:14b")) {
      p.check("picking from the menu makes it the current model", await p.until(() => prefs.model === "qwen3:14b"), prefs.model);
      p.check("the choice is stored", sameRef((await api.settingsGet()).chat_model, { provider: ollamaId, model: "qwen3:14b" }));
      p.check("a provider with no key says so", !!prefs.chatUnready?.includes("Ollama"), prefs.chatUnready);
      p.check("the chip warns", await p.until(() => !!document.querySelector(".tool-btn.model.unready")));
      await p.shot("4-chip-unready");
    }
  });

  await p.step("palette", async () => {
    void executeCommand("switch-model");
    const open = await p.until(() => document.querySelectorAll(".palette li[role=option]").length > 0);
    p.check("Switch model… lists the models", open);
    const titles = texts(".palette li[role=option] .title");
    const expected = s.model_choices.reduce((n, g) => n + g.models.length, 0);
    p.check("every model is a row, the twin's too", titles.length === expected + 1 && titles.includes("qwen3:14b"), titles);
    const twinRow = [...document.querySelectorAll<HTMLElement>(".palette li[role=option]")].some((li) => li.querySelector(".title")?.textContent?.trim() === twinModel && li.querySelector(".hint")?.textContent?.trim() === "Probe twin");
    p.check("the same model id under another provider is its own row", twinRow);
    const current = texts(".palette li[role=option] .hint").find((h) => h === t("palette.currentModel", { provider: "Ollama" }));
    p.check("the current model's row says so", !!current);
    await p.shot("5-palette-models");
    const target = firstIsDeepseek ? proModel : twinModel;
    const rows = [...document.querySelectorAll<HTMLElement>(".palette li[role=option]")];
    const row = rows.find((li) => li.querySelector(".title")?.textContent?.trim() === target && li.querySelector(".hint")?.textContent?.trim() === first.display_name);
    row?.querySelector<HTMLButtonElement>("button.main")?.click();
    p.check("a palette row switches the model", await p.until(() => prefs.model === target && prefs.chatModel.provider === first.id), prefs.chatModel);
  });

  await p.step("two real turns across models", async () => {
    await selectModel({ provider: first.id, model: twinModel });
    if (!firstIsDeepseek || prefs.chatUnready) {
      p.log(`skipped: the first provider is not DeepSeek with a key (${prefs.chatUnready ?? first.base_url})`);
      return;
    }
    await agent.newSession();
    const turn = async (effort: string, text: string) => {
      agent.effortOverride = effort;
      const run = agent.send(text, null);
      const finished = await Promise.race([run.then(() => true), sleep(240000).then(() => false)]);
      if (!finished) await agent.cancel();
      return finished;
    };
    const errors = () => agent.timeline.filter((i) => i.kind === "error").map((i) => (i as { text: string }).text);
    p.check("turn 1 (deepseek-flash, thinking off) finishes", await turn("none", "Read Apples.md with the read tool, then tell me in one sentence how to store apples."));
    p.check("turn 1 has no error", errors().length === 0, errors());
    await selectModel({ provider: first.id, model: proModel });
    p.check("turn 2 (deepseek-v4-pro, thinking high) finishes", await turn("high", "Now read Pears.md with the read tool and say in one sentence how pears differ."));
    p.check("turn 2 has no error — earlier tool turns without thinking text were accepted", errors().length === 0, errors());
    agent.effortOverride = null;
    const id = await api.agentSessionCurrent();
    const lines = id ? (await api.sessionRead(id)).split("\n").filter(Boolean).map((l) => JSON.parse(l) as { kind: string; model?: string; tool?: string }) : [];
    const models = lines.filter((l) => l.kind === "assistant").map((l) => l.model);
    p.log(`assistant rows' models: ${JSON.stringify(models)}`);
    p.check("turn 1 made a tool call (the case under test)", lines.some((l) => l.kind === "tool_call"));
    p.check("the log names each answer's model", models.length > 0 && models.every(Boolean), models);
    p.check("both models appear in the log", models.includes(twinModel) && models.includes(proModel), models);
    await p.shot("6-chat-after-turns");
  });

  await p.step("detached window", async () => {
    const seen: string[] = [];
    let ready = false;
    const offs = await Promise.all([onProbe<string>("seen", (m) => seen.push(m)), onProbe<string>("agent-ready", () => (ready = true))]);
    try {
      await agent.openWindow(null, null);
      p.check("the detached window starts its probe", await p.until(() => ready, 20000));
      // A model picked in the main window reaches the detached window's chip
      await selectModel({ provider: twinId, model: twinModel });
      p.check("a pick in the main window reaches the detached window", await p.until(() => seen.includes(twinModel), 5000), seen);
      // And one picked from the detached window's own menu reaches the main window
      await emitProbe("pick", { group: "Ollama", model: "qwen3:14b" });
      p.check("a pick in the detached window reaches the main window", await p.until(() => prefs.model === "qwen3:14b", 8000), prefs.model);
      await emitProbe("shot", "7-detached-window");
      await sleep(2500);
      await emitProbe("close", null);
      p.check("the detached window closes", await p.until(() => !agent.windowOpen, 8000));
    } finally {
      offs.forEach((off) => off());
    }
  });

  await p.step("translation model", async () => {
    await api.settingsSet({ translation_model: { provider: ollamaId, model: "qwen3:14b" } });
    s = await api.settingsGet();
    p.check("a translation model is stored apart from the chat's", sameRef(s.translation_model, { provider: ollamaId, model: "qwen3:14b" }), s.translation_model);
    openSettings("ai.translateModel");
    const shown = await p.until(() => document.querySelector<HTMLSelectElement>("#setting-ai-translateModel")?.value === refKey({ provider: ollamaId, model: "qwen3:14b" }));
    p.check("the settings page shows it", shown, document.querySelector<HTMLSelectElement>("#setting-ai-translateModel")?.value);
    await sleep(400);
    await p.shot("8-settings-translation");
    settingsUi.open = false;
    await api.settingsSet({ translation_model: { provider: "", model: "" } });
    p.check("back to the same as chat", (await api.settingsGet()).translation_model === null);
  });

  await p.step("removing a provider repairs the choice", async () => {
    await selectModel({ provider: twinId, model: twinModel });
    await api.settingsSet({ translation_model: { provider: twinId, model: twinModel } });
    const rows = (await api.settingsGet()).providers.map(stored).filter((x) => x.id !== twinId);
    await api.settingsSet({ providers: rows });
    s = await api.settingsGet();
    p.check("the chat falls back to the first provider's first model", s.chat_model.provider === rows[0].id && s.chat_model.model === rows[0].models[0], s.chat_model);
    p.check("the translation choice goes back to the chat's", s.translation_model === null, s.translation_model);
    p.check("every window's mirror follows", await p.until(() => prefs.chatModel.provider === rows[0].id), prefs.chatModel);
  });

  await p.step("restore", async () => {
    await api.settingsSet({
      providers: original.providers.map(stored),
      chat_model: original.chat_model,
      translation_model: original.translation_model ?? { provider: "", model: "" },
    });
    s = await api.settingsGet();
    p.check("the providers are back", JSON.stringify(s.providers.map(stored)) === JSON.stringify(original.providers.map(stored)));
    p.check("the current model is back", sameRef(s.chat_model, original.chat_model), s.chat_model);
    if (originalVault) await vault.openVault(originalVault);
  });

  await p.done();
}

/** The detached window's part: it reports what its chip shows after every change made
 *  elsewhere, and acts on the main window's asks (pick from its own menu, a picture, close). */
export async function runAgent() {
  const p = new Probe();
  p.log("models probe: detached window");
  prefs.onExternal(() => void emitProbe("seen", prefs.model));
  await onProbe<{ group: string; model: string }>("pick", (r) => void pickFromMenu(p, r.group, r.model));
  await onProbe<string>("shot", (name) => void p.shot(name));
  await onProbe("close", () => void api.agentWindowClose(agent.handoff(), false));
  p.check("the detached window shows the model chip", await p.until(() => !!document.querySelector(".tool-btn.model"), 15000), prefs.model);
  await emitProbe("agent-ready", prefs.model);
}
