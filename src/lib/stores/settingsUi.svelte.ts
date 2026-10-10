// Whether the settings dialog is open, on which page, and which setting to point at.
// Every way into the settings goes through openSettings with the id of what it is about
// (a page, or one setting from the index) — "change model…" lands on the model row, not
// on a page that happens to be near it.
import { PAGES, settingById, type SettingsPage, type SettingsState } from "$lib/settings";
import { account } from "$lib/stores/account.svelte";
import { prefs } from "$lib/stores/prefs.svelte";
import { vault } from "$lib/stores/vault.svelte";

class SettingsUi {
  open = $state(false);
  page = $state<SettingsPage>("general");
  /** A setting to scroll to and flash once the page has rendered; the dialog clears it. */
  focus = $state<string | null>(null);
}

export const settingsUi = new SettingsUi();

/** Open the settings on a page ("ai") or at one setting ("ai.model"); no target = General. */
export function openSettings(target: string = "general") {
  const entry = settingById(target);
  const page = entry?.page ?? (PAGES.some((p) => p.id === target) ? (target as SettingsPage) : "general");
  settingsUi.page = page;
  settingsUi.focus = entry ? entry.id : null;
  settingsUi.open = true;
}

/** The state that decides which settings rows exist right now, from the in-process
 *  mirror (every write in the dialog refreshes it): what the dialog's search and the
 *  palette's Settings rows filter by. */
export function currentSettingsState(): SettingsState {
  return { hasVault: !!vault.root, signedIn: account.loggedIn, captureTarget: prefs.captureTarget };
}
