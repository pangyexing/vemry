//! App-level settings, stored in the OS app-config directory — never inside a
//! vault (vaults get synced and committed; keys must not).

use serde::{Deserialize, Serialize};
use tauri::AppHandle;
use vemry_agent::provider::{ModelCap, ProviderKind, Vendor, DEFAULT_MODEL, ZHIPU_BASE_URL};
use vemry_agent::ProviderConfig;

use crate::config_store;

const FILE: &str = "settings.json";

/// Emitted (with no payload) after `settings_set` saves, so every window refreshes
/// its mirror; `onSettingsChanged` in api.ts listens.
pub const CHANGED_EVENT: &str = "settings://changed";

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Settings {
    /// The providers the user brings their own key for, in the order the settings
    /// page lists them. vemry hosted is built in and never stored here.
    #[serde(default)]
    pub providers: Vec<CustomProvider>,
    /// The current model: what the next chat message is sent to, switched from the
    /// composer's model menu, the palette or the settings page, and the same for
    /// every window. None only in a file from before the provider list existed —
    /// `load` migrates that, so every reader after it sees Some (and reads it
    /// through `chat_ref`, which repairs a choice whose provider has gone).
    #[serde(default)]
    pub chat_model: Option<ModelRef>,
    #[serde(default)]
    pub recent_vaults: Vec<String>,
    /// The vaults whose agent writes the user pre-authorised ("always allow in this
    /// vault"), by canonical root. Kept with the app rather than in the vault, so a
    /// vault copied or cloned from elsewhere cannot arrive with its writes allowed.
    #[serde(default)]
    pub write_allow_vaults: Vec<String>,
    /// The interface language. Auto follows the system locale — the frontend
    /// resolves it from navigator.language and the Rust side from sys_locale (the
    /// native menu has to be built before the frontend loads, so it cannot borrow
    /// the former).
    #[serde(default)]
    pub language: LangSetting,
    /// Vault reopened on launch — the one open when the app was last used.
    #[serde(default)]
    pub last_vault: Option<String>,
    /// Ad blocking in the built-in browser (macOS WKContentRuleList; ignored on other platforms).
    #[serde(default = "default_true")]
    pub adblock_enabled: bool,
    /// The rule subscription sources. Nothing is bundled (third-party filter lists
    /// are copyleft), so these are downloaded: once per app run when the first
    /// browser or reader page opens with no rules cached yet, and whenever the user
    /// asks for an update. Sources are fetched independently — an unreachable host
    /// costs only its own list — and rules can also be imported from a file.
    #[serde(default = "default_rule_urls")]
    pub adblock_rule_urls: Vec<String>,
    /// Where an http(s) link clicked in the editor goes.
    #[serde(default)]
    pub open_links_in: LinkOpenMode,
    /// A rendered link in the editor opens on an ordinary single click (⌘/Ctrl+click
    /// then opens it a different way); with this off, only ⌘/Ctrl+click opens it. In
    /// the raw state (the line the cursor is on) a click always means editing.
    #[serde(default = "default_true")]
    pub click_opens_links: bool,
    /// The agent's web search (the provider's server-side web_search: DeepSeek's
    /// Anthropic-format endpoint, Anthropic's, OpenAI's, xAI's, Meta's and Zhipu's). Search is billed
    /// separately from the conversation, so it is off by default.
    #[serde(default)]
    pub web_search_enabled: bool,
    /// The agent's plan confirmation (the propose_plan tool plus a blocking gate):
    /// a multi-step task raises a plan card first and waits for the user to run,
    /// edit or reject it, and continues within the same turn once confirmed. On by
    /// default; like web_search it is an addition to the tool table frozen per
    /// session — a change takes effect from the next session.
    #[serde(default = "default_true")]
    pub plan_confirm: bool,
    /// AI blocks awaiting review show an amber bar in the editor (the origin overlay
    /// is a command, not a setting).
    #[serde(default = "default_true")]
    pub provenance_marks: bool,
    /// Load images in the reading projection through the vemry-asset proxy (no
    /// cookies, with the page's Referer). It affects the projection alone; a note's
    /// own remote images follow `note_remote_images`.
    #[serde(default = "default_true")]
    pub reading_images_enabled: bool,
    /// Load a note's remote images (`![](https://…)`) without asking, through the same
    /// proxy. Off, the default, one waits as a placeholder until the user loads it, so
    /// opening a note makes no network request (a tracking pixel cannot leak reading
    /// behaviour); an image already in the proxy's cache shows either way, offline.
    #[serde(default)]
    pub note_remote_images: bool,
    /// After a whole-page clip, download the body's images into the vault
    /// (clips/assets/, deduplicated by content hash) and rewrite the note to
    /// reference them locally. The download happens only at the moment of clipping
    /// and goes through the same SSRF protection pipeline as vemry-asset; from then
    /// on reading makes no network request, and hotlink protection or an image host
    /// going away no longer loses the pictures.
    #[serde(default = "default_true")]
    pub clip_localize_images: bool,
    /// Run the Markdown formatter (vemry_core::format) over what the agent writes —
    /// a new note's content, an appended section — before it lands, so the diff the
    /// gate shows is what lands. The user's own lines are never touched by an
    /// agent write.
    #[serde(default = "default_true")]
    pub format_agent_writes: bool,
    /// The same formatter over a clip note before it lands, and over a rich-text
    /// paste turned into Markdown in the editor.
    #[serde(default = "default_true")]
    pub format_clips: bool,
    /// Formatter opt-ins that change the prose's bytes (block hashes and change times
    /// move with them): a space between CJK and Latin runs; full-width punctuation
    /// after CJK characters. Off unless the user wants them.
    #[serde(default)]
    pub format_cjk_spacing: bool,
    #[serde(default)]
    pub format_punctuation: bool,
    /// Check for updates automatically at startup (one silent check; a new version
    /// opens the update dialog, and is never downloaded on its own).
    /// Off by default — offline first: with it off, an update check happens only
    /// when the user presses "check for updates"; downloading and installing always
    /// need the user's explicit confirmation.
    #[serde(default)]
    pub updater_auto_check: bool,
    /// The version the user answered "skip this version" to in the update dialog:
    /// the silent startup check stays quiet about it (a later version shows again),
    /// while a check the user asks for still shows it — skipping mutes a nag, it
    /// does not hide a release from someone looking for one.
    #[serde(default)]
    pub updater_skipped_version: Option<String>,
    /// The note body's font size (px), applied to the editor and the reading
    /// projection; the UI font size does not follow it.
    /// settings_set clamps it to 15..=18, so editing the JSON by hand cannot put it
    /// out of range.
    #[serde(default = "default_note_font_size")]
    pub note_font_size: u8,
    /// Readable line width (on by default): the body column follows --content-width
    /// — around 42em in a narrow pane, growing with the pane and capping around
    /// 54em; with it off, the editor and the reading projection fill the pane.
    /// It affects the body's typography alone; the UI layout does not move.
    #[serde(default = "default_true")]
    pub readable_line_width: bool,
    /// The thinking depth level (none/low/high/max; DeepSeek's official default is
    /// high, and none turns thinking off). Sent only to a model that takes a level,
    /// in that vendor's words (the enablement check is reasoning_active).
    /// settings_set clamps an invalid value, so editing the JSON by hand cannot put
    /// it out of range.
    #[serde(default = "default_reasoning_effort")]
    pub reasoning_effort: String,
    /// The geometry (in logical pixels) the detached AI chat window had when it was
    /// last closed, so it reopens where it was.
    #[serde(default)]
    pub agent_window: Option<crate::agent_window::WindowGeom>,
    /// The main window's geometry, one entry per display it has been on, the most
    /// recently used first (window_geom.rs): it reopens where it last was on a display
    /// that is connected now, and at 85% of the display the first time.
    #[serde(default)]
    pub main_window: Vec<crate::window_geom::DisplayGeom>,
    /// The browser's shape: true = the panel docked in the main window (the
    /// default), false = the detached standalone window. browser_set_docked persists
    /// every toggle — "remember the last choice", with no entry in the settings page.
    #[serde(default = "default_true")]
    pub browser_docked: bool,
    /// The geometry (in logical pixels) a detached browser window had when it was
    /// last closed, so it reopens where it was (the same as agent_window; with
    /// several windows, only the last one closed is recorded).
    #[serde(default)]
    pub browser_window: Option<crate::agent_window::WindowGeom>,
    /// Side-by-side translation's target language (a BCP 47 tag the prompt names:
    /// zh-CN, en, ja…); empty = the interface language.
    #[serde(default)]
    pub translate_target: String,
    /// The model translations are sent to, from the same list as the chat's; None =
    /// the current chat model.
    #[serde(default)]
    pub translation_model: Option<ModelRef>,
    /// One line of the user's own instruction appended to the translation prompt's
    /// rules ("keep technical terms in English", say).
    #[serde(default)]
    pub translate_addendum: String,
    /// The quick capture window's global shortcut, in the shortcut plugin's grammar
    /// (`Alt+Shift+Space`; the frontend renders it as ⌥⇧Space or Alt+Shift+Space).
    /// One combination for every application, vemry itself included — a registered
    /// hot key fires in the foreground too. The default is a combination no
    /// common application binds; settings_set refuses one that does not parse.
    #[serde(default = "default_capture_hotkey")]
    pub capture_hotkey: String,
    /// Where Enter in the capture window lands a thought (⌘Enter still picks a note
    /// for one capture): inbox.md, today's journal note, or a pinned note and
    /// section. The pin itself is not here: a note path is relative to one vault,
    /// so it lives in that vault's `.vemry/config.json` (`capture_pin`, read and
    /// written through commands.rs) and every vault has its own, the way the clip
    /// target does. The mode alone is an application preference.
    #[serde(default)]
    pub capture_target: CaptureTarget,
    /// The vault a capture from a phone lands in (the root, spelled like
    /// `last_vault`): set to the open vault when the first pairing code is shown,
    /// changed in Settings › Quick capture › Phone. A phone cannot see which vault
    /// is open on the desk, and privacy zones are a vault's, so the landing vault
    /// is fixed here rather than taken from whatever happens to be open. None =
    /// the open vault, whichever it is (a pairing from before the field existed).
    #[serde(default)]
    pub inbox_vault: Option<String>,
    /// The last time captures from a phone landed (served in `SettingsView`, the settings
    /// page's "last one filed n minutes ago in inbox.md"). Kept on disk rather than
    /// in memory because the question it answers — is the phone connected? — is
    /// asked most right after a restart.
    #[serde(default)]
    pub inbox_last_landed: Option<InboxLanding>,
    /// An application behaviour (Settings › General): keep running in the menu
    /// bar (system tray) when the main window is closed — the close button hides
    /// the window instead of quitting, a tray icon stands for the app (a click on
    /// it is the capture shortcut's twin), and only an explicit quit ends the
    /// process. On by default: an application outliving its window is the desktop
    /// norm, and quick capture's promise — one key from any application — would
    /// otherwise end silently the first time the window is closed. Nothing is
    /// prompted about it; the settings hint says what closing means.
    #[serde(default = "default_true")]
    pub tray_resident: bool,
    /// The single-provider fields of a file from before the provider list: one
    /// endpoint, key and model, a hosted/custom switch with its own hosted model,
    /// and the translation model as a bare name. Read once by `migrate_models`,
    /// never written again (new keys carry the new shapes, so an old file never
    /// meets a type it cannot parse — a parse failure would reset every setting).
    #[serde(default, rename = "base_url", skip_serializing)]
    legacy_base_url: Option<String>,
    #[serde(default, rename = "model", skip_serializing)]
    legacy_model: Option<String>,
    #[serde(default, rename = "api_key", skip_serializing)]
    legacy_api_key: Option<String>,
    #[serde(default, rename = "provider", skip_serializing)]
    legacy_provider: Option<ProviderKind>,
    #[serde(default, rename = "hosted_model", skip_serializing)]
    legacy_hosted_model: Option<String>,
    #[serde(default, rename = "translate_model", skip_serializing)]
    legacy_translate_model: Option<String>,
}

/// One landing of captures from a phone, for the settings page (mirrored as
/// `InboxLanding` in api.ts).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct InboxLanding {
    /// RFC 3339, this machine's clock.
    pub at: String,
    /// The spot's label (`note.md › Section`, as capture.ts spells it).
    pub target: String,
    /// The vault's folder name when the captures were written straight into a
    /// vault other than the open one (inbox.rs, "The landing vault"); None when
    /// they went through the main window.
    #[serde(default)]
    pub vault: Option<String>,
}

/// The capture target's wire values, mirrored as `CaptureTarget` in api.ts.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CaptureTarget {
    #[default]
    Inbox,
    Journal,
    Pinned,
}


/// The provider id of vemry hosted in a `ModelRef`. A custom provider's id is
/// always `p<N>` (`set_providers` assigns it), so the two never meet.
pub const HOSTED: &str = "hosted";

/// One provider the user brings their own key for: an endpoint, its key, and the
/// model ids it serves. Mirrored as `CustomProvider` in api.ts.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CustomProvider {
    /// Stable within this file (`p1`, `p2`, …): a model choice names its provider
    /// by it, so renaming a provider or changing its endpoint keeps the choice.
    #[serde(default)]
    pub id: String,
    /// The user's name for it; empty = named after the endpoint (`display_name`).
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub base_url: String,
    /// Plaintext, like the rest of this owner-only file. A Zhipu endpoint with this
    /// empty takes ZHIPU_API_KEY from the environment (`takes_env_key`).
    #[serde(default)]
    pub api_key: String,
    #[serde(default)]
    pub models: Vec<String>,
}

impl CustomProvider {
    /// What the interface calls it: the user's name, else the vendor for the
    /// endpoints the settings page offers, else the endpoint's host, else the endpoint
    /// as typed (empty for a row with nothing in it yet).
    pub fn display_name(&self) -> String {
        let name = self.name.trim();
        if !name.is_empty() {
            return name.to_string();
        }
        vendor_name(&self.probe()).or_else(|| self.probe().host()).unwrap_or_else(|| self.base_url.trim().to_string())
    }

    /// Whether ZHIPU_API_KEY stands in for this provider's key: a Zhipu endpoint
    /// with no key of its own. The variable is never sent to any other host, and a
    /// key the user typed — which they can see — wins over it.
    pub fn takes_env_key(&self) -> bool {
        self.api_key.trim().is_empty() && self.probe().is_zhipu()
    }

    /// The endpoint alone, for the host predicates `ProviderConfig` owns.
    fn probe(&self) -> ProviderConfig {
        ProviderConfig::custom(self.base_url.clone(), "", "")
    }
}

/// A model choice: a provider (`HOSTED` or a custom provider's id) and one of its
/// model ids. For vemry hosted an empty model means the catalog's default.
/// Mirrored as `ModelRef` in api.ts.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelRef {
    #[serde(default)]
    pub provider: String,
    #[serde(default)]
    pub model: String,
}

impl ModelRef {
    pub fn new(provider: impl Into<String>, model: impl Into<String>) -> Self {
        ModelRef { provider: provider.into(), model: model.into() }
    }

    pub fn is_hosted(&self) -> bool {
        self.provider == HOSTED
    }
}

/// A model choice resolved into what a request is built from.
#[derive(Clone, Debug)]
pub struct Resolved {
    pub config: ProviderConfig,
    /// The choice as it resolved: for vemry hosted, the catalog's default spelled
    /// out when the choice left the model empty (or named one the catalog dropped).
    pub model_ref: ModelRef,
    /// Why no request can be built from it, when none can.
    pub unready: Option<Unready>,
}

/// Why a model choice cannot be sent to yet; `message` is the user's wording.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Unready {
    /// vemry hosted while signed out.
    SignIn,
    /// No provider with a model, and vemry hosted not signed in.
    NoModel,
    /// The provider (named) has no endpoint.
    NoEndpoint(String),
    /// The provider (named) has no key, nor an environment key standing in.
    NoKey(String),
}

impl Unready {
    pub fn message(&self) -> String {
        let lang = crate::i18n::current();
        match self {
            Unready::SignIn => crate::i18n::t(lang, "ipc.hosted.signIn"),
            Unready::NoModel => crate::i18n::t(lang, "ipc.noModel"),
            Unready::NoEndpoint(name) => crate::i18n::tf(lang, "ipc.noEndpointFor", &[("provider", name)]),
            Unready::NoKey(name) => crate::i18n::tf(lang, "ipc.noApiKeyFor", &[("provider", name)]),
        }
    }
}

/// One provider's models as the pickers list them (the composer's model menu, the
/// palette, the settings page), computed here so a window without the account
/// store — the detached chat window — gets the hosted catalog too. Mirrored as
/// `ModelGroup` in api.ts.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ModelGroup {
    /// `HOSTED` or the custom provider's id.
    pub provider: String,
    /// The custom provider's display name; empty for vemry hosted, which the
    /// frontend names from the catalog.
    pub name: String,
    pub models: Vec<ModelChoice>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ModelChoice {
    pub id: String,
    /// The catalog's default (vemry hosted only).
    pub default: bool,
}

impl Settings {
    /// The formatter's opt-ins as configured. The editor's "format note" command
    /// uses these directly (it runs at the user's explicit request, with no switch);
    /// the two automatic producers ask `clip_format` / `agent_format`, which fold
    /// their on/off switch in.
    pub fn format_options(&self) -> vemry_core::FormatOptions {
        vemry_core::FormatOptions { cjk_spacing: self.format_cjk_spacing, punctuation: self.format_punctuation }
    }

    /// The formatter for machine-composed clip text (a clipped page, a rich-text
    /// paste): the options when `format_clips` is on, `None` when it is off.
    pub fn clip_format(&self) -> Option<vemry_core::FormatOptions> {
        self.format_clips.then(|| self.format_options())
    }

    /// The formatter for the agent's writes: the options when `format_agent_writes`
    /// is on, `None` when it is off.
    pub fn agent_format(&self) -> Option<vemry_core::FormatOptions> {
        self.format_agent_writes.then(|| self.format_options())
    }
}

/// `raw` run through the formatter when there is one, `raw` itself when the
/// switch behind it is off — the one spelling of "format if the setting says so".
pub fn format_or(raw: &str, options: Option<vemry_core::FormatOptions>) -> std::borrow::Cow<'_, str> {
    match options {
        Some(o) => std::borrow::Cow::Owned(vemry_core::format_markdown(raw, &o)),
        None => std::borrow::Cow::Borrowed(raw),
    }
}

/// The interface language setting. The wire values match the frontend's
/// `LangSetting` exactly (mirrored in `api.ts`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum LangSetting {
    #[default]
    #[serde(rename = "auto")]
    Auto,
    #[serde(rename = "en")]
    En,
    #[serde(rename = "zh-CN")]
    ZhCn,
    #[serde(rename = "ja")]
    Ja,
    #[serde(rename = "ko")]
    Ko,
    #[serde(rename = "fr")]
    Fr,
    #[serde(rename = "de")]
    De,
    #[serde(rename = "es")]
    Es,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LinkOpenMode {
    /// The reading projection in the main window's centre pane (the default): the
    /// body is extracted and rendered, with a browser tab as the escape hatch.
    #[default]
    Reading,
    /// A tab of the built-in browser (docked beside the editor unless detached).
    Internal,
    /// The system's default browser.
    System,
}

fn default_true() -> bool {
    true
}

fn default_note_font_size() -> u8 {
    16
}

fn default_reasoning_effort() -> String {
    "high".into()
}

fn default_capture_hotkey() -> String {
    "Alt+Shift+Space".into()
}

fn default_rule_urls() -> Vec<String> {
    vec![
        "https://easylist.to/easylist/easylist.txt".into(),
        "https://easylist-downloads.adblockplus.org/easylistchina.txt".into(),
    ]
}

impl Default for Settings {
    fn default() -> Self {
        // A first start offers Zhipu's flagship, waiting for a key
        let zhipu = CustomProvider {
            id: "p1".into(),
            name: String::new(),
            base_url: ZHIPU_BASE_URL.into(),
            api_key: String::new(),
            models: vec![DEFAULT_MODEL.into()],
        };
        Settings {
            chat_model: Some(ModelRef::new("p1", DEFAULT_MODEL)),
            providers: vec![zhipu],
            recent_vaults: vec![],
            write_allow_vaults: vec![],
            language: LangSetting::default(),
            last_vault: None,
            adblock_enabled: true,
            adblock_rule_urls: default_rule_urls(),
            open_links_in: LinkOpenMode::default(),
            click_opens_links: true,
            web_search_enabled: false,
            plan_confirm: true,
            provenance_marks: true,
            reading_images_enabled: true,
            note_remote_images: false,
            clip_localize_images: true,
            format_agent_writes: true,
            format_clips: true,
            format_cjk_spacing: false,
            format_punctuation: false,
            updater_auto_check: false,
            updater_skipped_version: None,
            note_font_size: default_note_font_size(),
            readable_line_width: true,
            reasoning_effort: default_reasoning_effort(),
            agent_window: None,
            main_window: Vec::new(),
            browser_docked: true,
            browser_window: None,
            translate_target: String::new(),
            translation_model: None,
            translate_addendum: String::new(),
            capture_hotkey: default_capture_hotkey(),
            capture_target: CaptureTarget::default(),
            inbox_vault: None,
            inbox_last_landed: None,
            tray_resident: true,
            legacy_base_url: None,
            legacy_model: None,
            legacy_api_key: None,
            legacy_provider: None,
            legacy_hosted_model: None,
            legacy_translate_model: None,
        }
    }
}

impl Settings {
    /// The provider every chat request is built from: the current model, resolved.
    /// `run_agent_turn` and every predicate below read it, so what the settings page
    /// and the composer show is what is sent.
    pub fn provider_config(&self) -> ProviderConfig {
        self.chat().config
    }

    /// Whether the current model is a vemry hosted one (the account gate before a
    /// turn, the relay's error wording).
    pub fn hosted(&self) -> bool {
        self.chat_ref().is_hosted()
    }

    /// Whether web search is actually in effect: the switch is on and the provider
    /// runs the search (a DeepSeek, Anthropic, OpenAI, xAI or Meta endpoint, Zhipu's GLM-5.3, or the hosted relay
    /// where its catalog row allows it; see `ProviderConfig::web_search_available`). The settings page badge
    /// (web_search_active) and the agent side's enablement check share this one
    /// implementation — writing the boolean expression twice would drift sooner or
    /// later.
    pub fn web_search_active(&self) -> bool {
        self.web_search_enabled && self.provider_config().web_search_available()
    }

    /// Whether the thinking level is actually in effect: the current model takes one
    /// (`ProviderConfig::reasoning_supported`). The settings page hint, whether the composer's
    /// quick switch is shown, and the agent side's send check share this one
    /// implementation (it is not reimplemented in TS).
    pub fn reasoning_active(&self) -> bool {
        self.provider_config().reasoning_supported()
    }

    /// The current model, resolved against the providers and the account as they
    /// are now.
    pub fn chat(&self) -> Resolved {
        let creds = crate::account::hosted_credentials();
        let r = self.chat_ref_with(creds.is_some());
        self.resolve_with(&r, creds, env_zhipu_key().as_deref())
    }

    /// The translation model, resolved; the chat's when none is chosen (or the
    /// chosen one's provider has gone).
    pub fn translation(&self) -> Resolved {
        match self.translation_model.as_ref().and_then(|r| self.normalize_ref(r)) {
            Some(r) => self.resolve_with(&r, crate::account::hosted_credentials(), env_zhipu_key().as_deref()),
            None => self.chat(),
        }
    }

    /// The current model choice, repaired: see `normalize_ref` and `fallback_ref`.
    pub fn chat_ref(&self) -> ModelRef {
        self.chat_ref_with(crate::account::hosted_credentials().is_some())
    }

    fn chat_ref_with(&self, signed_in: bool) -> ModelRef {
        self.chat_model.as_ref().and_then(|r| self.normalize_ref(r)).unwrap_or_else(|| self.fallback_ref(signed_in))
    }

    /// A choice made valid against the provider list: a vemry hosted one stands as
    /// it is (the catalog is checked when it resolves, and signed out it resolves to
    /// "sign in" — the user chose hosted, so that is the right answer); a custom one
    /// keeps its model when the provider still lists it, or takes the provider's
    /// first. None when the provider is gone or lists no model.
    pub fn normalize_ref(&self, r: &ModelRef) -> Option<ModelRef> {
        if r.is_hosted() {
            return Some(ModelRef::new(HOSTED, r.model.trim()));
        }
        let p = self.providers.iter().find(|p| p.id == r.provider)?;
        let model = r.model.trim();
        let model = if p.models.iter().any(|m| m == model) { model } else { p.models.first()? };
        Some(ModelRef::new(&p.id, model))
    }

    /// The choice that stands in when there is none: the first provider with a
    /// model, else vemry hosted when signed in, else nothing (which resolves to
    /// "no model").
    fn fallback_ref(&self, signed_in: bool) -> ModelRef {
        if let Some(p) = self.providers.iter().find(|p| !p.models.is_empty()) {
            return ModelRef::new(&p.id, &p.models[0]);
        }
        if signed_in {
            return ModelRef::new(HOSTED, "");
        }
        ModelRef::default()
    }

    /// The pure half of resolving: the account's token and catalog, and the
    /// environment's key, are passed in.
    fn resolve_with(&self, r: &ModelRef, creds: Option<(String, Vec<ModelCap>)>, env_key: Option<&str>) -> Resolved {
        if r.is_hosted() {
            let (token, catalog) = creds.unwrap_or_default();
            // A model the catalog no longer offers is not sent (the relay would answer
            // model_unknown): the catalog's default stands in
            let listed = catalog.is_empty() || catalog.iter().any(|m| m.id == r.model);
            let config = ProviderConfig::hosted(token, if listed { &r.model } else { "" }, catalog);
            let unready = config.api_key.is_empty().then_some(Unready::SignIn);
            return Resolved { model_ref: ModelRef::new(HOSTED, &config.model), config, unready };
        }
        let Some(p) = self.providers.iter().find(|p| p.id == r.provider) else {
            return Resolved { config: ProviderConfig::custom("", "", ""), model_ref: r.clone(), unready: Some(Unready::NoModel) };
        };
        let key = if p.takes_env_key() { env_key.unwrap_or_default() } else { p.api_key.trim() };
        let config = ProviderConfig::custom(p.base_url.trim(), r.model.trim(), key);
        let unready = if config.model.is_empty() {
            Some(Unready::NoModel)
        } else if config.base_url.is_empty() {
            Some(Unready::NoEndpoint(p.display_name()))
        } else if config.api_key.is_empty() {
            Some(Unready::NoKey(p.display_name()))
        } else {
            None
        };
        Resolved { config, model_ref: r.clone(), unready }
    }

    /// Whether the environment's key is what this provider would send (the settings
    /// page says so under its key field).
    pub fn env_key_applies(p: &CustomProvider) -> bool {
        p.takes_env_key() && env_zhipu_key().is_some()
    }

    /// Replace the provider list with what the settings page sends: fields trimmed,
    /// model ids trimmed with blanks and repeats dropped, and ids kept only for rows
    /// that already had them — a new row, an id this file never issued and a second
    /// row claiming the same id each get a fresh one. A blank row stays (adding a
    /// provider saves at once, before anything is typed). The two model choices are
    /// repaired in the same write, so neither can point at a provider just removed
    /// (nor, later, at a new provider that reuses its id).
    pub fn set_providers(&mut self, rows: Vec<CustomProvider>, signed_in: bool) {
        let old: Vec<String> = self.providers.iter().map(|p| p.id.clone()).collect();
        let mut next = old.iter().filter_map(|id| id.strip_prefix('p')?.parse::<u32>().ok()).max().unwrap_or(0);
        let mut taken: Vec<String> = Vec::new();
        let mut out = Vec::with_capacity(rows.len());
        for row in rows {
            let mut models: Vec<String> = Vec::new();
            for m in &row.models {
                let m = m.trim();
                if !m.is_empty() && !models.iter().any(|x| x == m) {
                    models.push(m.to_string());
                }
            }
            let id = if old.contains(&row.id) && !taken.contains(&row.id) {
                row.id
            } else {
                next += 1;
                format!("p{next}")
            };
            taken.push(id.clone());
            out.push(CustomProvider {
                id,
                name: row.name.trim().to_string(),
                base_url: row.base_url.trim().to_string(),
                api_key: row.api_key.trim().to_string(),
                models,
            });
        }
        self.providers = out;
        self.chat_model = Some(self.chat_ref_with(signed_in));
        self.translation_model = self.translation_model.take().and_then(|r| self.normalize_ref(&r));
    }

    /// Every model the pickers offer, grouped by provider: vemry hosted first when
    /// signed in with a catalog, then each provider that lists a model.
    pub fn model_groups(&self) -> Vec<ModelGroup> {
        self.model_groups_with(crate::account::hosted_credentials().map(|(_, c)| c))
    }

    fn model_groups_with(&self, catalog: Option<Vec<ModelCap>>) -> Vec<ModelGroup> {
        let mut groups = Vec::new();
        if let Some(catalog) = catalog.filter(|c| !c.is_empty()) {
            // The default is the flagged row, or the first when none is (ProviderConfig::hosted's rule)
            let default = catalog.iter().find(|m| m.default).unwrap_or(&catalog[0]).id.clone();
            let models = catalog.iter().map(|m| ModelChoice { id: m.id.clone(), default: m.id == default }).collect();
            groups.push(ModelGroup { provider: HOSTED.into(), name: String::new(), models });
        }
        for p in self.providers.iter().filter(|p| !p.models.is_empty()) {
            let models = p.models.iter().map(|m| ModelChoice { id: m.clone(), default: false }).collect();
            groups.push(ModelGroup { provider: p.id.clone(), name: p.display_name(), models });
        }
        groups
    }

    /// A file from before the provider list becomes one: its endpoint, key and model
    /// the first provider, its hosted/custom switch the current model, its bare
    /// translation model a choice on the same side. Runs on load while `chat_model`
    /// is unset — a file written since always has it — and is deterministic, so
    /// loading again before anything is saved gives the same result; the next save
    /// writes the new shape and none of the old keys.
    fn migrate_models(&mut self) {
        if self.chat_model.is_some() {
            return;
        }
        let trimmed = |v: Option<String>| v.map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
        let base_url = trimmed(self.legacy_base_url.take());
        let model = trimmed(self.legacy_model.take());
        let api_key = trimmed(self.legacy_api_key.take());
        let hosted = self.legacy_provider.take() == Some(ProviderKind::Hosted);
        let hosted_model = trimmed(self.legacy_hosted_model.take()).unwrap_or_default();
        let translate_model = trimmed(self.legacy_translate_model.take());
        if !self.providers.is_empty() {
            // A list with no choice (a hand edit): the first model
            self.chat_model = Some(self.fallback_ref(false));
            return;
        }
        // A hosted user who never typed a key had only the untouched Zhipu default
        // behind the switch: nothing of theirs to carry over
        if !hosted || api_key.is_some() {
            let base_url = base_url.unwrap_or_else(|| ZHIPU_BASE_URL.into());
            self.providers.push(CustomProvider {
                id: "p1".into(),
                name: String::new(),
                base_url,
                api_key: api_key.unwrap_or_default(),
                models: vec![model.unwrap_or_else(|| DEFAULT_MODEL.into())],
            });
        }
        let chat = if hosted {
            ModelRef::new(HOSTED, hosted_model)
        } else {
            ModelRef::new("p1", &self.providers[0].models[0])
        };
        if let Some(tm) = translate_model.filter(|tm| *tm != chat.model) {
            self.translation_model = Some(if hosted {
                ModelRef::new(HOSTED, tm)
            } else {
                self.providers[0].models.push(tm.clone());
                ModelRef::new("p1", tm)
            });
        }
        self.chat_model = Some(chat);
    }
}

/// The ZHIPU_API_KEY environment variable, when set and not blank.
fn env_zhipu_key() -> Option<String> {
    std::env::var("ZHIPU_API_KEY").ok().filter(|k| !k.is_empty())
}

/// The vendor behind an endpoint the settings page offers (an interoperability fact,
/// decided on the host), which names a provider the user left unnamed. Not copy: a
/// vendor's name reads the same in every language.
fn vendor_name(endpoint: &ProviderConfig) -> Option<String> {
    let name = match endpoint.vendor() {
        Vendor::DeepSeek => "DeepSeek",
        Vendor::Zhipu if endpoint.is_zhipu() => "Zhipu",
        Vendor::Zhipu => "Z.ai",
        Vendor::OpenAi => "OpenAI",
        Vendor::Anthropic => "Anthropic",
        Vendor::XAi => "xAI",
        Vendor::Moonshot => "Moonshot",
        Vendor::Meta => "Meta",
        Vendor::Ollama => "Ollama",
        Vendor::Other => return None,
    };
    Some(name.into())
}

/// The valid thinking levels. Both entry points (settings_set persisting, and
/// run_agent_turn overriding per turn) share this allowlist — **what they do when
/// it fails differs** (persisting clamps back to the default, a per-turn override
/// falls back to the stored setting), so only the check is shared and never the
/// fallback.
pub fn is_reasoning_effort(v: &str) -> bool {
    matches!(v, "none" | "low" | "high" | "max")
}

/// An in-process cache: load is called per request on several hot paths (the image
/// proxy, clipping, navigation) and should not read and parse the disk each time.
/// save is the only writer in the process and refreshes the cache after a
/// successful write; the price is that editing settings.json externally needs a
/// restart to take effect (which was never promised before either).
static CACHE: std::sync::RwLock<Option<Settings>> = std::sync::RwLock::new(None);

pub fn load(app: &AppHandle) -> Settings {
    if let Some(s) = CACHE.read().unwrap().as_ref() {
        return s.clone();
    }
    let mut s: Settings = config_store::read(app, FILE);
    clean_vault_paths(&mut s);
    s.migrate_models();
    *CACHE.write().unwrap() = Some(s.clone());
    s
}

/// Up to and including rc1, std canonicalize on Windows stored the `\\?\D:\…`
/// verbatim prefix into recent/last. This washes that existing dirt out on load;
/// washing can produce a "dirty + clean" duplicate, so it deduplicates while
/// keeping the order.
/// Not cfg(windows): a Unix path never starts with `\\?\` (so this is a no-op
/// there), and compiling it unconditionally makes the test actually run on the
/// Linux/macOS CI.
fn clean_vault_paths(s: &mut Settings) {
    let mut cleaned: Vec<String> = Vec::new();
    for p in &s.recent_vaults {
        let c = strip_verbatim(p);
        if !cleaned.contains(&c) {
            cleaned.push(c);
        }
    }
    s.recent_vaults = cleaned;
    s.last_vault = s.last_vault.take().map(|p| strip_verbatim(&p));
    s.inbox_vault = s.inbox_vault.take().map(|p| strip_verbatim(&p));
}

fn strip_verbatim(p: &str) -> String {
    // The UNC form has to be handled before the bare prefix, or `\\?\UNC\srv\share` would be stripped to `UNC\srv\share`
    if let Some(rest) = p.strip_prefix(r"\\?\UNC\") {
        format!(r"\\{rest}")
    } else if let Some(rest) = p.strip_prefix(r"\\?\") {
        rest.to_string()
    } else {
        p.to_string()
    }
}

pub fn save(app: &AppHandle, settings: &Settings) -> Result<(), String> {
    // Owner-only, like account.json: the file holds the user's own API key
    config_store::save_private(app, FILE, settings)?;
    *CACHE.write().unwrap() = Some(settings.clone());
    Ok(())
}

/// The atomic entry point for read-modify-write. Should a scattered `load → modify
/// → save` run concurrently on two threads (browser_set_docked on the command
/// thread and the geometry write of the main thread's window hook, say), the later
/// writer would overwrite the field the earlier one just stored with the snapshot
/// it loaded; this holds the lock across the whole read-modify-write, so a lost
/// field cannot happen. A closure returning false means nothing changed, and the
/// write is skipped.
pub fn update(app: &AppHandle, f: impl FnOnce(&mut Settings) -> bool) -> Result<(), String> {
    static UPDATE: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _g = UPDATE.lock().unwrap();
    let mut s = load(app);
    if !f(&mut s) {
        return Ok(());
    }
    save(app, &s)
}

pub fn remember_vault(app: &AppHandle, path: &str) {
    let mut s = load(app);
    s.recent_vaults.retain(|p| p != path);
    s.recent_vaults.insert(0, path.to_string());
    s.recent_vaults.truncate(8);
    s.last_vault = Some(path.to_string());
    let _ = save(app, &s);
}

/// Remove one entry from the recent list (the right-click "remove from recent
/// list", a way out for a directory opened by mistake; the directory on disk is
/// untouched). Should it also be last_vault, that falls to the first of the
/// remaining recents (or is cleared when there are none) — otherwise the next start
/// would open it again, while simply clearing it would throw the user back to the
/// welcome page. A failed write is propagated: the cache is refreshed only after a
/// successful save, so swallowing the error would report a false success and leave
/// the interface as it was.
pub fn forget_vault(app: &AppHandle, path: &str) -> Result<Vec<String>, String> {
    let mut s = load(app);
    s.recent_vaults.retain(|p| p != path);
    if s.last_vault.as_deref() == Some(path) {
        s.last_vault = s.recent_vaults.first().cloned();
    }
    save(app, &s)?;
    Ok(s.recent_vaults)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Existing rc1 data: the verbatim prefix is stripped, duplicates are removed in
    /// order, and last_vault is washed along with it.
    #[test]
    fn clean_vault_paths_strips_verbatim_and_dedups() {
        let mut s = Settings {
            recent_vaults: vec![
                r"\\?\D:\worknotes2".into(),
                r"D:\worknotes2".into(), // a duplicate of the clean path — should merge once stripped
                r"\\?\UNC\srv\share\notes".into(),
                r"\\?\D:\".into(),
                "/Users/me/notes".into(), // a Unix path is kept as is
            ],
            last_vault: Some(r"\\?\D:\worknotes2".into()),
            ..Default::default()
        };
        clean_vault_paths(&mut s);
        assert_eq!(
            s.recent_vaults,
            vec![
                r"D:\worknotes2".to_string(),
                r"\\srv\share\notes".to_string(),
                r"D:\".to_string(),
                "/Users/me/notes".to_string(),
            ]
        );
        assert_eq!(s.last_vault.as_deref(), Some(r"D:\worknotes2"));
    }

    /// P2 changed the default to Reading: a value an existing user stored must be
    /// kept verbatim, and only a missing field takes the new default — an upgrade
    /// does not change existing behaviour.
    #[test]
    fn link_open_mode_upgrade_keeps_stored_value() {
        let s: Settings = serde_json::from_str(r#"{"open_links_in":"internal"}"#).unwrap();
        assert_eq!(s.open_links_in, LinkOpenMode::Internal);
        let s: Settings = serde_json::from_str(r#"{"open_links_in":"system"}"#).unwrap();
        assert_eq!(s.open_links_in, LinkOpenMode::System);
        let s: Settings = serde_json::from_str("{}").unwrap();
        assert_eq!(s.open_links_in, LinkOpenMode::Reading);
    }

    /// The automatic update check is a switch added later: an old config missing the
    /// field takes the default false (offline first), and a stored true must be kept.
    #[test]
    fn updater_auto_check_defaults_false_keeps_stored() {
        let s: Settings = serde_json::from_str("{}").unwrap();
        assert!(!s.updater_auto_check);
        let s: Settings = serde_json::from_str(r#"{"updater_auto_check":true}"#).unwrap();
        assert!(s.updater_auto_check);
    }

    /// The skipped version is a field added later: an old config missing it means
    /// nothing was skipped, and a stored value must be kept.
    #[test]
    fn updater_skipped_version_defaults_none_keeps_stored() {
        let s: Settings = serde_json::from_str("{}").unwrap();
        assert_eq!(s.updater_skipped_version, None);
        let s: Settings = serde_json::from_str(r#"{"updater_skipped_version":"0.7.0"}"#).unwrap();
        assert_eq!(s.updater_skipped_version.as_deref(), Some("0.7.0"));
    }

    /// The body font size is a field added later: an old config missing it takes the
    /// default 16, and a stored value must be kept.
    #[test]
    fn note_font_size_defaults_16_keeps_stored() {
        let s: Settings = serde_json::from_str("{}").unwrap();
        assert_eq!(s.note_font_size, 16);
        let s: Settings = serde_json::from_str(r#"{"note_font_size":18}"#).unwrap();
        assert_eq!(s.note_font_size, 18);
    }

    /// Readable line width is a switch added later: an old config missing it takes
    /// the default true, and a stored false must be kept.
    #[test]
    fn readable_line_width_defaults_true_keeps_stored() {
        let s: Settings = serde_json::from_str("{}").unwrap();
        assert!(s.readable_line_width);
        let s: Settings = serde_json::from_str(r#"{"readable_line_width":false}"#).unwrap();
        assert!(!s.readable_line_width);
    }

    /// The thinking level is a field added later: an old config missing it takes the
    /// default "high" (DeepSeek's official default), and a stored value must be kept.
    #[test]
    fn reasoning_effort_defaults_high_keeps_stored() {
        let s: Settings = serde_json::from_str("{}").unwrap();
        assert_eq!(s.reasoning_effort, "high");
        let s: Settings = serde_json::from_str(r#"{"reasoning_effort":"none"}"#).unwrap();
        assert_eq!(s.reasoning_effort, "none");
    }

    /// The browser's shape is a remembered switch: an old config missing it takes
    /// the default true (the panel docked in the main window, the same-window
    /// panel's default), and a stored false (the detached window) must be kept.
    #[test]
    fn browser_docked_defaults_true_keeps_stored() {
        let s: Settings = serde_json::from_str("{}").unwrap();
        assert!(s.browser_docked);
        assert!(s.browser_window.is_none());
        assert!(s.main_window.is_empty());
        let s: Settings = serde_json::from_str(r#"{"browser_docked":false}"#).unwrap();
        assert!(!s.browser_docked);
    }

    /// A file as `load` sees it: parsed, then migrated.
    fn loaded(json: &str) -> Settings {
        let mut s: Settings = serde_json::from_str(json).unwrap();
        s.migrate_models();
        s
    }

    fn catalog() -> Vec<ModelCap> {
        vec![
            ModelCap { id: "deepseek-flash".into(), default: true, search: true, images: true, max_tokens: None },
            ModelCap { id: "deepseek-v4-pro".into(), default: false, search: true, images: false, max_tokens: None },
        ]
    }

    fn provider(id: &str, base_url: &str, key: &str, models: &[&str]) -> CustomProvider {
        CustomProvider {
            id: id.into(),
            name: String::new(),
            base_url: base_url.into(),
            api_key: key.into(),
            models: models.iter().map(|m| m.to_string()).collect(),
        }
    }

    /// A first start and an empty file land on the same thing: Zhipu's flagship,
    /// no key yet.
    #[test]
    fn a_fresh_file_is_the_default_zhipu_provider() {
        let fresh = loaded("{}");
        let default = Settings::default();
        assert_eq!(fresh.providers, default.providers);
        assert_eq!(fresh.chat_model, default.chat_model);
        assert_eq!(fresh.providers[0].display_name(), "Zhipu");
        assert_eq!(fresh.chat_model, Some(ModelRef::new("p1", DEFAULT_MODEL)));
        assert_eq!(fresh.translation_model, None);
    }

    /// A bring-your-own-key file keeps its endpoint, key and model as the first
    /// provider, named after the vendor, and the choice points at it.
    #[test]
    fn a_custom_file_migrates_to_one_provider() {
        let s = loaded(r#"{"base_url":" https://api.deepseek.com ","model":"deepseek-v4-pro","api_key":"sk-1"}"#);
        assert_eq!(s.providers, vec![provider("p1", "https://api.deepseek.com", "sk-1", &["deepseek-v4-pro"])]);
        assert_eq!(s.providers[0].display_name(), "DeepSeek");
        assert_eq!(s.chat_model, Some(ModelRef::new("p1", "deepseek-v4-pro")));
        let p = s.resolve_with(&s.chat_ref_with(false), None, None);
        assert_eq!((p.config.base_url.as_str(), p.config.model.as_str(), p.config.api_key.as_str()), ("https://api.deepseek.com", "deepseek-v4-pro", "sk-1"));
        assert_eq!(p.unready, None);
        // Any other host names the provider after itself
        let s = loaded(r#"{"base_url":"http://192.168.1.5:8000/v1","model":"qwen"}"#);
        assert_eq!(s.providers[0].display_name(), "192.168.1.5:8000");
        let s = loaded(r#"{"base_url":"http://localhost:11434/v1","model":"qwen3:14b"}"#);
        assert_eq!(s.providers[0].display_name(), "Ollama");
        // Every vendor the settings page offers names an unnamed provider
        for (base, name) in [
            ("https://api.openai.com/v1", "OpenAI"),
            ("https://api.anthropic.com", "Anthropic"),
            ("https://api.x.ai/v1", "xAI"),
            ("https://api.moonshot.cn/v1", "Moonshot"),
            ("https://api.z.ai/api/paas/v4", "Z.ai"),
            ("https://api.meta.ai/v1", "Meta"),
        ] {
            let p = CustomProvider { base_url: base.into(), ..Default::default() };
            assert_eq!(p.display_name(), name, "{base}");
        }
    }

    /// A hosted file keeps hosted as the current model, and the key typed behind the
    /// switch as a provider — or no provider when nothing was typed.
    #[test]
    fn a_hosted_file_migrates_to_the_hosted_choice() {
        let s = loaded(r#"{"provider":"hosted","hosted_model":"deepseek-v4-pro","model":"glm-4.7","api_key":"k"}"#);
        assert_eq!(s.chat_model, Some(ModelRef::new(HOSTED, "deepseek-v4-pro")));
        assert_eq!(s.providers, vec![provider("p1", ZHIPU_BASE_URL, "k", &["glm-4.7"])]);
        let s = loaded(r#"{"provider":"hosted"}"#);
        assert_eq!(s.chat_model, Some(ModelRef::new(HOSTED, "")));
        assert!(s.providers.is_empty());
    }

    /// The old bare translation model becomes a choice on the same side.
    #[test]
    fn the_translation_model_migrates_beside_the_chat() {
        let s = loaded(r#"{"base_url":"https://api.deepseek.com","model":"deepseek-v4-pro","api_key":"k","translate_model":"deepseek-flash"}"#);
        assert_eq!(s.providers[0].models, vec!["deepseek-v4-pro", "deepseek-flash"]);
        assert_eq!(s.translation_model, Some(ModelRef::new("p1", "deepseek-flash")));
        let s = loaded(r#"{"provider":"hosted","translate_model":"deepseek-flash"}"#);
        assert_eq!(s.translation_model, Some(ModelRef::new(HOSTED, "deepseek-flash")));
        // The same as the chat's: nothing to choose
        let s = loaded(r#"{"model":"glm-4.7","translate_model":"glm-4.7"}"#);
        assert_eq!(s.translation_model, None);
        assert_eq!(s.providers[0].models, vec!["glm-4.7"]);
    }

    /// The migrated file is written in the new shape alone and is not migrated
    /// again; an emptied list stays empty.
    #[test]
    fn a_migrated_file_carries_no_old_keys() {
        let s = loaded(r#"{"provider":"hosted","base_url":"x","model":"m","api_key":"k","hosted_model":"h","translate_model":"t"}"#);
        let json = serde_json::to_value(&s).unwrap();
        for key in ["base_url", "model", "api_key", "provider", "hosted_model", "translate_model"] {
            assert!(json.get(key).is_none(), "{key} written back");
        }
        let again = loaded(&json.to_string());
        assert_eq!((again.providers, again.chat_model, again.translation_model), (s.providers, s.chat_model, s.translation_model));
        let s = loaded(r#"{"providers":[],"chat_model":{"provider":"","model":""}}"#);
        assert!(s.providers.is_empty());
        assert_eq!(s.resolve_with(&s.chat_ref_with(false), None, None).unready, Some(Unready::NoModel));
    }

    /// The list the settings page sends is cleaned, and ids are kept only for rows
    /// that already had them.
    #[test]
    fn set_providers_cleans_rows_and_assigns_ids() {
        let mut s = Settings::default();
        s.set_providers(
            vec![
                CustomProvider { name: " Mine ".into(), ..provider("p1", " https://open.bigmodel.cn/api/paas/v4 ", " k ", &[" glm-4.7 ", "", "glm-4.7", "glm-4.7-flash"]) },
                provider("", "", "", &[]),
                provider("p9", "https://api.deepseek.com", "k2", &["deepseek-flash"]),
                provider("p1", "http://localhost:11434/v1", "", &["qwen3:14b"]),
            ],
            false,
        );
        let ids: Vec<&str> = s.providers.iter().map(|p| p.id.as_str()).collect();
        assert_eq!(ids, ["p1", "p2", "p3", "p4"]);
        assert_eq!(s.providers[0], CustomProvider { name: "Mine".into(), ..provider("p1", ZHIPU_BASE_URL, "k", &["glm-4.7", "glm-4.7-flash"]) });
        // The blank row stays, named after nothing; a typed name wins over the vendor's
        assert_eq!(s.providers[1].display_name(), "");
        assert_eq!(s.providers[0].display_name(), "Mine");
        assert_eq!(s.providers[3].display_name(), "Ollama");
    }

    /// Removing the chosen model or its provider moves the choice; the translation
    /// choice goes back to "same as chat" when its provider goes.
    #[test]
    fn removing_a_provider_repairs_the_choices() {
        let mut s = Settings::default();
        s.set_providers(
            vec![provider("p1", ZHIPU_BASE_URL, "k", &["glm-4.7-flash"]), provider("", "https://api.deepseek.com", "k2", &["deepseek-flash", "deepseek-v4-pro"])],
            false,
        );
        s.chat_model = Some(ModelRef::new("p2", "deepseek-v4-pro"));
        s.translation_model = Some(ModelRef::new("p2", "deepseek-flash"));
        // The model goes: the provider's first takes its place
        s.set_providers(vec![provider("p1", ZHIPU_BASE_URL, "k", &["glm-4.7-flash"]), provider("p2", "https://api.deepseek.com", "k2", &["deepseek-flash"])], false);
        assert_eq!(s.chat_model, Some(ModelRef::new("p2", "deepseek-flash")));
        // The provider goes: the first provider with a model, and translation follows the chat
        s.set_providers(vec![provider("p1", ZHIPU_BASE_URL, "k", &["glm-4.7-flash"])], false);
        assert_eq!(s.chat_model, Some(ModelRef::new("p1", "glm-4.7-flash")));
        assert_eq!(s.translation_model, None);
        // Nothing left: vemry hosted when signed in, nothing otherwise
        s.set_providers(vec![], true);
        assert_eq!(s.chat_model, Some(ModelRef::new(HOSTED, "")));
        s.set_providers(vec![], false);
        assert_eq!(s.chat_model, Some(ModelRef::new(HOSTED, "")), "a hosted choice stands; resolving it asks to sign in");
        s.chat_model = None;
        s.set_providers(vec![], false);
        assert_eq!(s.chat_model, Some(ModelRef::default()));
    }

    /// A dangling choice (a hand edit) reads as the fallback without being written.
    #[test]
    fn a_dangling_choice_reads_as_the_fallback() {
        let s = Settings { chat_model: Some(ModelRef::new("p7", "x")), ..Default::default() };
        assert_eq!(s.chat_ref_with(false), ModelRef::new("p1", DEFAULT_MODEL));
        let s = Settings { chat_model: Some(ModelRef::new("p1", "gone")), ..Default::default() };
        assert_eq!(s.chat_ref_with(false), ModelRef::new("p1", DEFAULT_MODEL));
    }

    /// vemry hosted: the relay with the device token; signed out, an empty key and
    /// "sign in" — never sent bare. A model the catalog dropped gives way to its
    /// default.
    #[test]
    fn a_hosted_choice_resolves_against_the_account() {
        let s = Settings { chat_model: Some(ModelRef::new(HOSTED, "deepseek-v4-pro")), ..Default::default() };
        let r = s.resolve_with(&s.chat_ref_with(true), Some(("tok".into(), catalog())), None);
        assert!(r.config.is_hosted() && r.unready.is_none());
        assert_eq!((r.config.api_key.as_str(), r.config.model.as_str()), ("tok", "deepseek-v4-pro"));
        assert!(r.config.chat_completions_url().ends_with("/v1/chat/completions"));
        let r = s.resolve_with(&ModelRef::new(HOSTED, "retired"), Some(("tok".into(), catalog())), None);
        assert_eq!(r.config.model, "deepseek-flash");
        assert_eq!(r.model_ref, ModelRef::new(HOSTED, "deepseek-flash"));
        let r = s.resolve_with(&ModelRef::new(HOSTED, ""), Some(("tok".into(), catalog())), None);
        assert_eq!(r.model_ref, ModelRef::new(HOSTED, "deepseek-flash"));
        let r = s.resolve_with(&s.chat_ref_with(false), None, None);
        assert!(r.config.is_hosted() && r.config.api_key.is_empty());
        assert_eq!(r.unready, Some(Unready::SignIn));
        // No account in the process (a test): the predicates read the hosted choice
        assert!(s.hosted() && s.reasoning_active());
    }

    /// The environment's key fills a blank Zhipu key and goes nowhere else.
    #[test]
    fn the_env_key_fills_only_a_blank_zhipu_key() {
        let s = Settings {
            providers: vec![provider("p1", ZHIPU_BASE_URL, "", &["glm-4.7-flash"]), provider("p2", "https://api.deepseek.com", "", &["deepseek-flash"]), provider("p3", ZHIPU_BASE_URL, "mine", &["glm-4.7"])],
            ..Default::default()
        };
        let key = |r: &str, m: &str| s.resolve_with(&ModelRef::new(r, m), None, Some("env")).config.api_key;
        assert_eq!(key("p1", "glm-4.7-flash"), "env");
        assert_eq!(key("p2", "deepseek-flash"), "");
        assert_eq!(key("p3", "glm-4.7"), "mine");
        assert_eq!(s.resolve_with(&ModelRef::new("p2", "deepseek-flash"), None, Some("env")).unready, Some(Unready::NoKey("DeepSeek".into())));
    }

    /// Why a choice cannot be sent, in order: no model, no endpoint, no key.
    #[test]
    fn unready_names_what_is_missing() {
        let s = Settings {
            providers: vec![CustomProvider { name: "Home".into(), ..provider("p1", "", "k", &["m"]) }, provider("p2", "https://api.deepseek.com", "", &["deepseek-flash"])],
            ..Default::default()
        };
        let unready = |r: ModelRef| s.resolve_with(&r, None, None).unready;
        assert_eq!(unready(ModelRef::new("p1", "m")), Some(Unready::NoEndpoint("Home".into())));
        assert_eq!(unready(ModelRef::new("p2", "deepseek-flash")), Some(Unready::NoKey("DeepSeek".into())));
        assert_eq!(unready(ModelRef::new("p9", "m")), Some(Unready::NoModel));
        assert_eq!(unready(ModelRef::new("p1", "")), Some(Unready::NoModel));
    }

    /// The pickers list vemry hosted first (signed in, with a catalog), then every
    /// provider that has a model, in the settings page's order.
    #[test]
    fn model_groups_put_hosted_first() {
        let s = Settings {
            providers: vec![provider("p1", ZHIPU_BASE_URL, "", &["glm-4.7-flash"]), provider("p2", "", "", &[]), provider("p3", "http://192.168.1.5:8000/v1", "", &["qwen3:14b"])],
            ..Default::default()
        };
        let groups = s.model_groups_with(Some(catalog()));
        let shape: Vec<(&str, &str, usize)> = groups.iter().map(|g| (g.provider.as_str(), g.name.as_str(), g.models.len())).collect();
        assert_eq!(shape, [(HOSTED, "", 2), ("p1", "Zhipu", 1), ("p3", "192.168.1.5:8000", 1)]);
        assert!(groups[0].models[0].default && !groups[0].models[1].default);
        assert_eq!(s.model_groups_with(None).len(), 2);
        assert_eq!(s.model_groups_with(Some(vec![])).len(), 2);
    }

    /// The capture fields are added later: an old config missing them takes the
    /// default shortcut, inbox.md and no fixed vault for the phone, and stored
    /// values must be kept. The pin is the vault's, not a field here: a config
    /// that still carries one is read without complaint.
    #[test]
    fn capture_fields_default_and_keep_stored() {
        let s: Settings = serde_json::from_str("{}").unwrap();
        assert_eq!(s.capture_hotkey, "Alt+Shift+Space");
        assert_eq!(s.capture_target, CaptureTarget::Inbox);
        assert_eq!(s.inbox_vault, None);
        let s: Settings = serde_json::from_str(
            r#"{"capture_hotkey":"Ctrl+Shift+Space","capture_target":"pinned","capture_pinned_path":"notes/ideas.md","inbox_vault":"/home/me/notes"}"#,
        )
        .unwrap();
        assert_eq!(s.capture_hotkey, "Ctrl+Shift+Space");
        assert_eq!(s.capture_target, CaptureTarget::Pinned);
        assert_eq!(s.inbox_vault.as_deref(), Some("/home/me/notes"));
        let s: Settings = serde_json::from_str(r#"{"capture_target":"journal"}"#).unwrap();
        assert_eq!(s.capture_target, CaptureTarget::Journal);
    }

    /// The phone's vault is washed like the other two stored roots.
    #[test]
    fn inbox_vault_is_washed_of_the_verbatim_prefix() {
        let mut s: Settings = serde_json::from_str("{}").unwrap();
        s.inbox_vault = Some(r"\\?\D:\notes".to_string());
        clean_vault_paths(&mut s);
        assert_eq!(s.inbox_vault.as_deref(), Some(r"D:\notes"));
    }

    /// The resident switch is on for an old config (the capture promise holds
    /// from the first close) and a stored false is kept.
    #[test]
    fn tray_resident_defaults_on_and_keeps_stored() {
        let s: Settings = serde_json::from_str("{}").unwrap();
        assert!(s.tray_resident);
        let s: Settings = serde_json::from_str(r#"{"tray_resident":false}"#).unwrap();
        assert!(!s.tray_resident);
    }

    /// Click-opens-links is a switch added later: an old config missing it takes the
    /// default true, and a stored false must be kept.
    #[test]
    fn click_opens_links_defaults_true_keeps_stored() {
        let s: Settings = serde_json::from_str("{}").unwrap();
        assert!(s.click_opens_links);
        let s: Settings = serde_json::from_str(r#"{"click_opens_links":false}"#).unwrap();
        assert!(!s.click_opens_links);
    }
}
