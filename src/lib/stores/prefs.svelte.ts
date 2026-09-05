// The in-process mirror of behavioural preferences. Rust's settings.json is the single
// source of truth; this only caches the fields the editor's click path has to read
// synchronously (a click gesture cannot await an IPC round trip). App refreshes it once
// at startup, and SettingsModal writes the mirrored fields directly after a successful
// save — outside those two there is no writer.
import { api, type SettingsView } from "$lib/api";

class PrefsStore {
  /** An ordinary click on a rendered link in the editor opens it; with it off, only
   *  ⌘/Ctrl+click does. The default matches Rust's default_true, so the behaviour is
   *  already right before the IPC returns. */
  clickOpensLinks = $state(true);
  /** The current model's name (visible in the research panel's header); updated by refresh after the settings are saved. */
  model = $state("");
  /** The value of the "AI web search" switch; together with webSearchActive it drives the panel's degradation hint. */
  webSearchEnabled = $state(false);
  /** Whether search will really take effect (the switch is on and the endpoint is DeepSeek) — derived on the Rust side, and the check is not reimplemented in the frontend. */
  webSearchActive = $state(false);
  /** The note body's font size (px), applied through the --fs-note variable; Editor's effect uses it to trigger requestMeasure. */
  noteFontSize = $state(16);
  /** The readable-line-width switch; Editor's effect uses it to trigger requestMeasure. */
  readableLineWidth = $state(true);
  /** Whether AI blocks awaiting review show their amber bar in the editor. */
  provenanceMarks = $state(true);
  /** Rich-text pastes into the editor are converted to Markdown (the same switch as clip formatting). */
  formatClips = $state(true);
  /** The global default thinking level (none/low/high/max); the fallback value of the composer's quick-switch button. */
  reasoningEffort = $state("high");
  /** Whether the level really takes effect (the endpoint is DeepSeek) — derived on the Rust side, and the check is not reimplemented in the frontend. */
  reasoningEffortActive = $state(false);
  /** The endpoint is DeepSeek (which is known to have no vision model): the composer
   *  warns once when an image is attached. It comes from the same source as
   *  reasoningEffortActive but is its own field — "the level takes effect" is not used
   *  as a proxy for "this is DeepSeek", since the two may diverge later. */
  providerDeepseek = $state(false);

  /** Returns the full settings it fetched, which the startup path consumes along the way (saving a second IPC). */
  async refresh(): Promise<SettingsView | undefined> {
    try {
      const s = await api.settingsGet();
      this.clickOpensLinks = s.click_opens_links;
      this.model = s.model;
      this.webSearchEnabled = s.web_search_enabled;
      this.webSearchActive = s.web_search_active;
      this.applyNoteFontSize(s.note_font_size);
      this.applyReadableLineWidth(s.readable_line_width);
      this.provenanceMarks = s.provenance_marks;
      this.formatClips = s.format_clips;
      this.reasoningEffort = s.reasoning_effort;
      this.reasoningEffortActive = s.reasoning_effort_active;
      this.providerDeepseek = s.provider_deepseek;
      return s;
    } catch {
      // Failing to fetch keeps the current values
      return undefined;
    }
  }

  /** Apply the font size to :root's --fs-note, which the CSS of the editor and the
   *  reading projection picks up immediately.
   *  The clamp guards against a value edited out of range in settings.json (the same
   *  15-18 as settings_set). */
  applyNoteFontSize(px: number) {
    const v = Math.min(18, Math.max(15, Math.round(px) || 16));
    this.noteFontSize = v;
    document.documentElement.style.setProperty("--fs-note", `${v}px`);
  }

  /** Readable line width: on = remove the override and return to app.css's default
   *  column-width curve (that clamp is defined only there and is not copied out here);
   *  off = none, so max-width is unbounded and the editor and the reading projection
   *  fill the pane. */
  applyReadableLineWidth(on: boolean) {
    this.readableLineWidth = on;
    const st = document.documentElement.style;
    if (on) st.removeProperty("--content-width");
    else st.setProperty("--content-width", "none");
  }
}

export const prefs = new PrefsStore();
