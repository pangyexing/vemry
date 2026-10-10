// The model pickers' shared vocabulary: the composer's model menu, the palette's "Switch
// model…" and the settings page list the same groups (Rust's model_choices) and name a
// choice the same way.
import { api, HOSTED, type ModelGroup, type ModelRef } from "$lib/api";
import { t } from "$lib/i18n/i18n.svelte";
import { prefs } from "$lib/stores/prefs.svelte";
import { toast } from "$lib/stores/toast.svelte";

/** A group's heading: vemry hosted by its own name, a provider by the user's (a blank new one as "New provider"). */
export function groupName(g: Pick<ModelGroup, "provider" | "name">): string {
  if (g.provider === HOSTED) return t("settings.providerHosted");
  return g.name || t("settings.providerUnnamed");
}

export function sameRef(a: ModelRef | null | undefined, b: ModelRef | null | undefined): boolean {
  return !!a && !!b && a.provider === b.provider && a.model === b.model;
}

/** The name of a choice's provider, from the groups. */
export function providerName(r: ModelRef, groups: ModelGroup[] = prefs.modelChoices): string {
  const g = groups.find((g) => g.provider === r.provider);
  if (g) return groupName(g);
  return r.provider === HOSTED ? t("settings.providerHosted") : "";
}

/** A choice as a label: the model id, with its provider when another group lists the same id. */
export function modelLabel(r: ModelRef, groups: ModelGroup[] = prefs.modelChoices): string {
  const twice = groups.filter((g) => g.models.some((m) => m.id === r.model)).length > 1;
  const name = providerName(r, groups);
  return twice && name ? `${r.model} · ${name}` : r.model;
}

/** Make a choice the current model: written once, heard by every window, read back here at once. */
export async function selectModel(r: ModelRef) {
  try {
    await api.settingsSet({ chat_model: { provider: r.provider, model: r.model } });
    await prefs.refresh();
  } catch (e) {
    toast.push(String(e), "error");
  }
}
