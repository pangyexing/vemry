//! App-level settings, stored in the OS app-config directory — never inside a
//! vault (vaults get synced and committed; keys must not).

use serde::{Deserialize, Serialize};
use tauri::AppHandle;

use crate::config_store;

const FILE: &str = "settings.json";

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Settings {
    #[serde(default = "default_base_url")]
    pub base_url: String,
    #[serde(default = "default_model")]
    pub model: String,
    /// Stored key; the ZHIPU_API_KEY env var takes precedence.
    #[serde(default)]
    pub api_key: String,
    #[serde(default)]
    pub recent_vaults: Vec<String>,
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
    /// The agent's web search (the DeepSeek Responses API's server-side
    /// web_search). Search is billed separately from the conversation, so it is off
    /// by default.
    #[serde(default)]
    pub web_search_enabled: bool,
    /// The agent's plan confirmation (the propose_plan tool plus a blocking gate):
    /// a multi-step task raises a plan card first and waits for the user to run,
    /// edit or reject it, and continues within the same turn once confirmed. On by
    /// default; like web_search it is an addition to the tool table frozen per
    /// session — a change takes effect from the next session.
    #[serde(default = "default_true")]
    pub plan_confirm: bool,
    /// Load images in the reading projection through the vemry-asset proxy (no
    /// cookies, with the page's Referer). It affects the projection alone; a note on
    /// disk always makes zero network requests (a tracking pixel cannot leak reading
    /// behaviour).
    #[serde(default = "default_true")]
    pub reading_images_enabled: bool,
    /// After a whole-page clip, download the body's images into the vault
    /// (clips/assets/, deduplicated by content hash) and rewrite the note to
    /// reference them locally. The download happens only at the moment of clipping
    /// and goes through the same SSRF protection pipeline as vemry-asset; from then
    /// on reading makes no network request, and hotlink protection or an image host
    /// going away no longer loses the pictures.
    #[serde(default = "default_true")]
    pub clip_localize_images: bool,
    /// Check for updates automatically at startup (one silent check; a new version
    /// is only announced, never downloaded).
    /// Off by default — offline first: with it off, an update check happens only
    /// when the user presses "check for updates"; downloading and installing always
    /// need the user's explicit confirmation.
    #[serde(default)]
    pub updater_auto_check: bool,
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
    /// high, and none turns thinking off). Sent to a DeepSeek endpoint only — Zhipu
    /// does not know the parameter, and the enablement check is reasoning_active.
    /// settings_set clamps an invalid value, so editing the JSON by hand cannot put
    /// it out of range.
    #[serde(default = "default_reasoning_effort")]
    pub reasoning_effort: String,
    /// The geometry (in logical pixels) the detached AI chat window had when it was
    /// last closed, so it reopens where it was.
    #[serde(default)]
    pub agent_window: Option<crate::agent_window::WindowGeom>,
    /// The shape a built-in browser window is built in: false = a detached, centred
    /// window (the default), true = docked against the main window's right edge.
    /// browser_set_docked persists every toggle — "remember the last choice", with
    /// no entry in the settings page.
    #[serde(default)]
    pub browser_docked: bool,
    /// The geometry (in logical pixels) a detached browser window had when it was
    /// last closed, so it reopens where it was (the same as agent_window; with
    /// several windows, only the last one closed is recorded).
    #[serde(default)]
    pub browser_window: Option<crate::agent_window::WindowGeom>,
}

/// The interface language setting. The wire values match the frontend's
/// `LangSetting` exactly (mirrored in `api.ts`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum LangSetting {
    #[default]
    #[serde(rename = "auto")]
    Auto,
    #[serde(rename = "zh-CN")]
    ZhCn,
    #[serde(rename = "en")]
    En,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LinkOpenMode {
    /// The reading projection in the main window's centre pane (the default since
    /// v0.5 P2): the body is extracted and rendered, with a browser window as the
    /// escape hatch.
    #[default]
    Reading,
    /// A built-in knowledge browser window.
    Internal,
    /// The system's default browser.
    System,
}

fn default_base_url() -> String {
    vemry_agent::provider::ZHIPU_BASE_URL.into()
}

fn default_model() -> String {
    vemry_agent::provider::DEFAULT_MODEL.into()
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

fn default_rule_urls() -> Vec<String> {
    vec![
        "https://easylist.to/easylist/easylist.txt".into(),
        "https://easylist-downloads.adblockplus.org/easylistchina.txt".into(),
    ]
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            base_url: default_base_url(),
            model: default_model(),
            api_key: String::new(),
            recent_vaults: vec![],
            language: LangSetting::default(),
            last_vault: None,
            adblock_enabled: true,
            adblock_rule_urls: default_rule_urls(),
            open_links_in: LinkOpenMode::default(),
            click_opens_links: true,
            web_search_enabled: false,
            plan_confirm: true,
            reading_images_enabled: true,
            clip_localize_images: true,
            updater_auto_check: false,
            note_font_size: default_note_font_size(),
            readable_line_width: true,
            reasoning_effort: default_reasoning_effort(),
            agent_window: None,
            browser_docked: false,
            browser_window: None,
        }
    }
}

impl Settings {
    pub fn effective_api_key(&self) -> String {
        std::env::var("ZHIPU_API_KEY").ok().filter(|k| !k.is_empty()).unwrap_or_else(|| self.api_key.clone())
    }

    pub fn provider_config(&self) -> vemry_agent::ProviderConfig {
        vemry_agent::ProviderConfig {
            base_url: self.base_url.clone(),
            model: self.model.clone(),
            api_key: self.effective_api_key(),
        }
    }

    /// Whether web search is actually in effect: the switch is on and the endpoint is
    /// DeepSeek. The settings page badge (web_search_active) and the agent side's
    /// enablement check share this one implementation — writing the boolean
    /// expression twice would drift sooner or later.
    pub fn web_search_active(&self) -> bool {
        self.web_search_enabled && self.provider_config().is_deepseek()
    }

    /// Whether the thinking level is actually in effect: the endpoint is DeepSeek.
    /// The settings page hint, whether the composer's quick switch is shown, and the
    /// agent side's send check share this one implementation (it is not reimplemented
    /// in TS).
    pub fn reasoning_active(&self) -> bool {
        self.provider_config().is_deepseek()
    }
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
    config_store::save(app, FILE, settings)?;
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

    /// Browser docking is a switch batch D turned into one: an old config missing it
    /// takes the default false (a detached, centred window, batch D's new default
    /// shape), and a stored true (docked) must be kept.
    #[test]
    fn browser_docked_defaults_false_keeps_stored() {
        let s: Settings = serde_json::from_str("{}").unwrap();
        assert!(!s.browser_docked);
        assert!(s.browser_window.is_none());
        let s: Settings = serde_json::from_str(r#"{"browser_docked":true}"#).unwrap();
        assert!(s.browser_docked);
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
