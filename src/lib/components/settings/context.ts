// The two contexts the settings dialog hands down to its rows.

/** Set by SettingsModal: while the search field holds a query the pages render together
 *  and each row shows itself only when the query found it. */
export interface SearchContext {
  readonly query: string;
  matches(id: string): boolean;
  /** Leave the search for the setting's page, scrolled to it. */
  go(id: string): void;
}

/** Set by SettingRow for its control: the id the row's label points at. */
export interface RowContext {
  readonly id: string;
  readonly ctlId: string;
}
