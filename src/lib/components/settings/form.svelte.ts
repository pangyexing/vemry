// The settings as the dialog shows them, written back one field at a time the moment a
// control changes: there is no draft, no Save and no Cancel, so closing the dialog in any
// way loses nothing and nothing waits for a button. A text field writes when it loses the
// focus or on Enter (the dialog blurs the focused field before it closes).
import { api, type SettingsUpdate, type SettingsView } from "$lib/api";
import { setLanguage } from "$lib/i18n/i18n.svelte";
import { prefs } from "$lib/stores/prefs.svelte";
import { toast } from "$lib/stores/toast.svelte";
import { updater } from "$lib/stores/updater.svelte";

class SettingsForm {
  /** The settings as last read back; null until the dialog's first read. */
  s = $state<SettingsView | null>(null);
  /** A write the backend refused, under the setting row it was made from. */
  errors = $state<Record<string, string>>({});
  /** The row whose text field just landed: its hint line says "Saved" for a moment. */
  justSaved = $state<string | null>(null);
  #savedTimer: ReturnType<typeof setTimeout> | null = null;
  /** Writes in flight: a read-back only applies when no newer write started after it, so a
   *  fast second toggle is not flipped back by the first one's read. */
  #seq = 0;
  #last: Promise<unknown> = Promise.resolve();

  /** Settles once every write started so far has: an action that reads a setting back
   *  from disk (updating the rule sources) waits on it after blurring the field. */
  idle(): Promise<unknown> {
    return this.#last;
  }

  async load() {
    const s = await prefs.refresh();
    if (s) this.s = s;
  }

  /** Write one change made on the row `row`. The view updates at once; the read-back
   *  afterwards brings the derived fields (the active model, web search availability,
   *  the tray and shortcut errors) and feeds the in-process mirror the editor reads
   *  synchronously. A refusal is shown under the row; `saved` marks a text field's row
   *  as saved when the write lands. */
  set(patch: SettingsUpdate, row?: string, saved = false): Promise<boolean> {
    const p = this.#write(patch, row, saved);
    this.#last = this.#last.then(() => p);
    return p;
  }

  async #write(patch: SettingsUpdate, row: string | undefined, saved: boolean): Promise<boolean> {
    const seq = ++this.#seq;
    if (this.s) this.s = { ...this.s, ...(patch as Partial<SettingsView>) };
    if (row && this.errors[row]) {
      const { [row]: _, ...rest } = this.errors;
      this.errors = rest;
    }
    let failed: string | null = null;
    try {
      await api.settingsSet(patch);
    } catch (e) {
      failed = String(e);
    }
    const s = await prefs.refresh();
    if (s && seq === this.#seq) this.s = s;
    if (s) {
      if ("language" in patch) setLanguage(s.language);
      if ("updater_auto_check" in patch) updater.autoCheck = s.updater_auto_check;
    }
    if (failed) {
      if (row) this.errors = { ...this.errors, [row]: failed };
      else toast.push(failed, "error");
      return false;
    }
    if (row && saved) {
      this.justSaved = row;
      if (this.#savedTimer) clearTimeout(this.#savedTimer);
      this.#savedTimer = setTimeout(() => (this.justSaved = null), 1500);
    }
    return true;
  }
}

export const form = new SettingsForm();
