// The in-process mirror of behavioural preferences. Rust's settings.json is the single
// source of truth; this only caches the fields the editor's click path has to read
// synchronously (a click gesture cannot await an IPC round trip). App refreshes it once
// at startup, and SettingsModal writes the mirrored fields directly after a successful
// save — outside those two there is no writer.
import { api, type CaptureTarget, type ProviderSetting, type SettingsView } from "$lib/api";

/** settings.rs default_capture_hotkey, in the shortcut plugin's grammar; the settings page's "reset" puts it back. */
export const CAPTURE_HOTKEY_DEFAULT = "Alt+Shift+Space";

class PrefsStore {
  /** An ordinary click on a rendered link in the editor opens it; with it off, only
   *  ⌘/Ctrl+click does. The default matches Rust's default_true, so the behaviour is
   *  already right before the IPC returns. */
  clickOpensLinks = $state(true);
  /** The model requests actually name (visible in the research panel's header): the hosted
   *  choice or the catalog's default in hosted mode, the custom field otherwise — Rust's
   *  active_model; updated by refresh after the settings are saved. */
  model = $state("");
  /** Which provider the AI talks to; the status bar shows the hosted quota only in hosted mode. */
  provider = $state<ProviderSetting>("custom");
  /** The value of the "AI web search" switch — the composer's model menu and the settings page write the same field. */
  webSearchEnabled = $state(false);
  /** Whether the provider runs the search at all (the endpoint is DeepSeek) — derived on the Rust side, and the check is not reimplemented in the frontend; the menu item is disabled without it. */
  webSearchAvailable = $state(false);
  /** Whether search will really take effect (the switch is on and the provider runs it) — derived on the Rust side. */
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
  /** The selected model has no image understanding (DeepSeek's pro model): the
   *  composer warns once when an image is attached. Decided on the Rust side next to
   *  the other provider predicates; not reimplemented here. */
  imagesUnsupported = $state(false);
  /** Side-by-side translation's target language ("" = the interface language): the status bar's chip asks whether the open note is already in it. */
  translateTarget = $state("");
  /** Quick capture: the global shortcut in the plugin's grammar (the activity bar's
   *  hint and the palette row render it through capture.ts's hotkeyLabel), and where
   *  Enter lands a capture — read by the capture window at the moment of landing. */
  captureHotkey = $state(CAPTURE_HOTKEY_DEFAULT);
  captureTarget = $state<CaptureTarget>("inbox");
  capturePinnedPath = $state("");
  capturePinnedHeading = $state("");

  /** Returns the full settings it fetched, which the startup path consumes along the way (saving a second IPC). */
  async refresh(): Promise<SettingsView | undefined> {
    try {
      const s = await api.settingsGet();
      this.clickOpensLinks = s.click_opens_links;
      this.model = s.active_model;
      this.provider = s.provider;
      this.webSearchEnabled = s.web_search_enabled;
      this.webSearchAvailable = s.web_search_available;
      this.webSearchActive = s.web_search_active;
      this.applyNoteFontSize(s.note_font_size);
      this.applyReadableLineWidth(s.readable_line_width);
      this.provenanceMarks = s.provenance_marks;
      this.formatClips = s.format_clips;
      this.reasoningEffort = s.reasoning_effort;
      this.reasoningEffortActive = s.reasoning_effort_active;
      this.imagesUnsupported = s.images_unsupported;
      this.translateTarget = s.translate_target;
      this.captureHotkey = s.capture_hotkey;
      this.captureTarget = s.capture_target;
      this.capturePinnedPath = s.capture_pinned_path;
      this.capturePinnedHeading = s.capture_pinned_heading;
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
