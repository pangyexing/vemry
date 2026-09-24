// What the settings dialog hands down to its rows.
import { getContext } from "svelte";

/** Set by SettingsModal: while the search field holds a query the pages render together
 *  and each row shows itself only when the query found it. */
export interface SearchContext {
  readonly query: string;
  matches(id: string): boolean;
}

/** The search context, when the page renders inside the dialog. */
export const settingsSearch = () => getContext<SearchContext | undefined>("settings-search");

/** The id a setting row's label points at: its control carries it. */
export const ctlIdOf = (id: string) => `setting-${id.replace(/\./g, "-")}`;

/** Attach to the one control of a row that has no id of its own (a select, a button):
 *  it takes the row's control id, so the row's label reaches it. */
export function rowControl(el: HTMLElement) {
  const id = el.closest<HTMLElement>("[data-setting]")?.dataset.setting;
  if (id) el.id = ctlIdOf(id);
}
