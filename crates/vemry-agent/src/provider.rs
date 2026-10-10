//! Provider configuration. Any OpenAI-compatible endpoint works — Zhipu is
//! the free default; Ollama/DeepSeek/OpenAI are a base_url + model away.
//!
//! Two kinds of provider (`ProviderKind`): the user's own endpoint and key
//! (`Custom`, everything above), and vemry's hosted access (`Hosted`), where the
//! endpoint is the relay at `api.vemry.app`, the key is the account's device token
//! and what each model can do (search, images, the output ceiling) comes from the
//! model catalog the account snapshot carries rather than from the host name.

use serde::{Deserialize, Serialize};

pub const ZHIPU_BASE_URL: &str = "https://open.bigmodel.cn/api/paas/v4";
pub const DEFAULT_MODEL: &str = "glm-4.7-flash";
/// The one DeepSeek model whose image understanding the documentation marks as
/// unsupported (2026-09-14; `deepseek-flash` takes JPEG/PNG/GIF/WebP natively).
pub const DEEPSEEK_PRO_MODEL: &str = "deepseek-v4-pro";

/// The hosted relay. Compiled into the client like the download host: fixed on
/// day one, and a move later is a DNS change rather than a release.
pub const HOSTED_API_BASE: &str = "https://api.vemry.app";

/// The relay's origin: `HOSTED_API_BASE`, unless — in a debug build only — the
/// `VEMRY_API_BASE` environment variable names another one (a local server during
/// development). A release build always talks to the real host: the network-exit
/// list is closed, and no environment variable may send exit 6, with the device
/// token it carries, anywhere else. Never persisted — the override is read every
/// time, so a shell without it goes back to the real host.
pub fn hosted_base_url() -> String {
    if cfg!(debug_assertions) {
        hosted_base_from(std::env::var("VEMRY_API_BASE").ok().as_deref())
    } else {
        HOSTED_API_BASE.to_string()
    }
}

/// The origin an override resolves to: the override with its trailing slash
/// trimmed, or the real host when it is unset or blank. The pure half of
/// `hosted_base_url`, so it can be tested without touching the process
/// environment (which the other tests read concurrently).
fn hosted_base_from(override_: Option<&str>) -> String {
    override_
        .map(|s| s.trim().trim_end_matches('/').to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| HOSTED_API_BASE.to_string())
}

/// Which kind of provider a `ProviderConfig` describes. The wire spelling is the
/// settings file's (`"custom"` / `"hosted"`); a config from before the field
/// existed is `Custom`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ProviderKind {
    #[default]
    Custom,
    Hosted,
}

/// One row of the hosted model catalog (the `models` list of the account
/// snapshot): what the relay offers and what each model can do. Every flag
/// defaults to off, so a row the server adds a field to still parses.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelCap {
    pub id: String,
    #[serde(default)]
    pub default: bool,
    /// Web search is available (the Anthropic-format line runs it).
    #[serde(default)]
    pub search: bool,
    /// Image input is accepted.
    #[serde(default)]
    pub images: bool,
    /// The output ceiling the relay clamps `max_tokens` to.
    #[serde(default)]
    pub max_tokens: Option<u64>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ProviderConfig {
    pub base_url: String,
    pub model: String,
    #[serde(skip_serializing_if = "String::is_empty", default)]
    pub api_key: String,
    #[serde(default)]
    pub kind: ProviderKind,
    /// The hosted model catalog (None on a custom provider, and on a hosted one
    /// whose snapshot has not arrived yet).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub catalog: Option<Vec<ModelCap>>,
}

impl ProviderConfig {
    /// A custom (bring-your-own-key) provider: the three fields every endpoint needs.
    pub fn custom(base_url: impl Into<String>, model: impl Into<String>, api_key: impl Into<String>) -> Self {
        ProviderConfig {
            base_url: base_url.into(),
            model: model.into(),
            api_key: api_key.into(),
            kind: ProviderKind::Custom,
            catalog: None,
        }
    }

    pub fn zhipu(api_key: String) -> Self {
        ProviderConfig::custom(ZHIPU_BASE_URL, DEFAULT_MODEL, api_key)
    }

    /// The hosted provider: the relay's `/v1` as base_url (so the two URL rules
    /// below reach `/v1/chat/completions` and `/anthropic/v1/messages` unchanged),
    /// the device token as the key, and the catalog the snapshot carries. An empty
    /// `model` picks the catalog's default.
    pub fn hosted(token: String, model: &str, catalog: Vec<ModelCap>) -> Self {
        let model = if model.trim().is_empty() {
            catalog.iter().find(|m| m.default).or(catalog.first()).map(|m| m.id.clone()).unwrap_or_default()
        } else {
            model.trim().to_string()
        };
        ProviderConfig {
            base_url: format!("{}/v1", hosted_base_url()),
            model,
            api_key: token,
            kind: ProviderKind::Hosted,
            catalog: Some(catalog),
        }
    }

    pub fn is_hosted(&self) -> bool {
        self.kind == ProviderKind::Hosted
    }

    /// The catalog row of the selected model (hosted only).
    fn cap(&self) -> Option<&ModelCap> {
        let model = self.model.trim();
        self.catalog.as_ref()?.iter().find(|m| m.id == model)
    }

    pub fn chat_completions_url(&self) -> String {
        format!("{}/chat/completions", self.base_url.trim_end_matches('/'))
    }

    /// The Anthropic-format Messages endpoint DeepSeek documents as
    /// `https://api.deepseek.com/anthropic`: the origin of base_url plus that
    /// path, so a base_url spelled with `/v1` (which the Chat Completions line
    /// accepts) reaches the same place. The relay exposes the same shape at the
    /// same path, so the hosted base (`…/v1`) needs no special case.
    pub fn anthropic_messages_url(&self) -> String {
        let origin = self
            .parsed_base()
            .and_then(|u| u.host_str().map(|h| format!("{}://{}{}", u.scheme(), h, u.port().map(|p| format!(":{p}")).unwrap_or_default())))
            .unwrap_or_else(|| self.base_url.trim().trim_end_matches('/').to_string());
        format!("{origin}/anthropic/v1/messages")
    }

    /// base_url parsed; the setting is free text, so when a missing scheme
    /// (`api.deepseek.com`) leaves nothing to parse, `https://` is prepended and
    /// parsing tried again.
    fn parsed_base(&self) -> Option<reqwest::Url> {
        let url = self.base_url.trim();
        reqwest::Url::parse(url)
            .ok()
            .filter(|u| u.host_str().is_some())
            .or_else(|| reqwest::Url::parse(&format!("https://{url}")).ok())
    }

    /// base_url's host, with its port when one is spelled (`localhost:11434`); None
    /// when the free text holds no host at all. The settings page names a provider
    /// the user left unnamed after it.
    pub fn host(&self) -> Option<String> {
        let u = self.parsed_base()?;
        let host = u.host_str()?;
        Some(match u.port() {
            Some(p) => format!("{host}:{p}"),
            None => host.to_string(),
        })
    }

    /// base_url's host name alone, without the port: what the vendor checks below
    /// compare against.
    fn host_name(&self) -> Option<String> {
        self.parsed_base().and_then(|u| u.host_str().map(str::to_owned))
    }

    /// Whether base_url points at the official DeepSeek endpoint (the only place
    /// the Anthropic-format line, and with it web search, is available on a custom
    /// provider). Decided on the host — a missing scheme must not silently flip the
    /// classification back to a plain endpoint.
    pub fn is_deepseek(&self) -> bool {
        self.host_name().is_some_and(|host| host == "api.deepseek.com" || host.ends_with(".deepseek.com"))
    }

    /// Whether base_url points at Zhipu's endpoint — the one host the
    /// `ZHIPU_API_KEY` environment variable may be sent to. Decided on the host, like
    /// `is_deepseek`.
    pub fn is_zhipu(&self) -> bool {
        self.host_name().is_some_and(|host| host == "open.bigmodel.cn" || host.ends_with(".bigmodel.cn"))
    }

    /// The official DeepSeek endpoint with the pro model selected (see
    /// [`DEEPSEEK_PRO_MODEL`] for why this one model is singled out).
    pub fn is_deepseek_pro(&self) -> bool {
        self.is_deepseek() && self.model.trim() == DEEPSEEK_PRO_MODEL
    }

    /// Whether the server-side web_search can take effect. Custom: the DeepSeek
    /// endpoint, any of its models (the Anthropic-format line runs the search for
    /// flash and pro alike — measured 2026-09-14). Hosted: the catalog says so for
    /// the selected model. Everything that shows "search is on" or sends the request
    /// with the tool reads this one predicate, so the composer chip and the request
    /// cannot disagree.
    pub fn web_search_available(&self) -> bool {
        match self.kind {
            ProviderKind::Custom => self.is_deepseek(),
            ProviderKind::Hosted => self.cap().is_some_and(|m| m.search),
        }
    }

    /// Whether an attached image would be refused by the provider. Custom: the pro
    /// model has no image understanding, the flash model and non-DeepSeek endpoints
    /// are left to answer for themselves. Hosted: the catalog's `images` flag.
    pub fn images_unsupported(&self) -> bool {
        match self.kind {
            ProviderKind::Custom => self.is_deepseek_pro(),
            ProviderKind::Hosted => self.cap().is_some_and(|m| !m.images),
        }
    }

    /// Whether `stream_options.include_usage` is sent: DeepSeek needs asking for
    /// the usage tail (a Zhipu stream carries it by default, and an unknown
    /// parameter risks a 400 there), and the relay's upstream is DeepSeek.
    pub fn include_usage(&self) -> bool {
        self.is_deepseek() || self.is_hosted()
    }

    /// Whether the thinking-level fields are understood: a DeepSeek endpoint, or the
    /// relay in front of one.
    pub fn reasoning_supported(&self) -> bool {
        self.is_deepseek() || self.is_hosted()
    }

    /// The stream-idle limit: with no SSE event for this long, the stream is
    /// considered dead. 600 s through the relay — the vendor's peak-time queue
    /// sends only `: keep-alive` comment lines for up to ten minutes, and the read
    /// loop counts events, not comments — and 120 s otherwise.
    pub fn stream_idle_timeout(&self) -> std::time::Duration {
        std::time::Duration::from_secs(if self.is_hosted() { 600 } else { 120 })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_deepseek_classifies_by_host_with_or_without_scheme() {
        let cfg = |base: &str| ProviderConfig::custom(base, "", "");
        assert!(cfg("https://api.deepseek.com").is_deepseek());
        assert!(cfg("api.deepseek.com").is_deepseek());
        assert!(cfg(" api.deepseek.com/v1 ").is_deepseek());
        assert!(!cfg(ZHIPU_BASE_URL).is_deepseek());
        assert!(!cfg("open.bigmodel.cn/api/paas/v4").is_deepseek());
        // Decided on the host; the string "deepseek" appearing in the path does not count
        assert!(!cfg("https://evil.com/api.deepseek.com").is_deepseek());
        assert!(!cfg("localhost:11434").is_deepseek());
        assert!(!cfg("").is_deepseek());
    }

    #[test]
    fn host_keeps_the_port_and_is_zhipu_reads_the_host() {
        let cfg = |base: &str| ProviderConfig::custom(base, "", "");
        assert_eq!(cfg("https://api.deepseek.com/v1").host().as_deref(), Some("api.deepseek.com"));
        assert_eq!(cfg("api.deepseek.com").host().as_deref(), Some("api.deepseek.com"));
        assert_eq!(cfg("http://localhost:11434/v1").host().as_deref(), Some("localhost:11434"));
        assert_eq!(cfg("").host(), None);
        assert!(cfg(ZHIPU_BASE_URL).is_zhipu());
        assert!(cfg("open.bigmodel.cn/api/paas/v4").is_zhipu());
        // Decided on the host; the string appearing in the path does not count
        assert!(!cfg("https://evil.com/open.bigmodel.cn").is_zhipu());
        assert!(!cfg("https://api.deepseek.com").is_zhipu());
        assert!(!cfg("").is_zhipu());
    }

    #[test]
    fn every_deepseek_model_searches_and_only_pro_refuses_images() {
        let cfg = |base: &str, model: &str| ProviderConfig::custom(base, model, "");
        let pro = cfg("https://api.deepseek.com", "deepseek-v4-pro");
        assert!(pro.is_deepseek_pro() && pro.web_search_available() && pro.images_unsupported());
        // The model string is free text like base_url: surrounding whitespace is forgiven
        assert!(cfg("api.deepseek.com", " deepseek-v4-pro ").is_deepseek_pro());
        // The current flash name, its retired alias and the retired chat/reasoner names: search yes, images fine
        for m in ["deepseek-flash", "deepseek-v4-flash", "deepseek-chat", "deepseek-reasoner", ""] {
            let c = cfg("https://api.deepseek.com", m);
            assert!(c.web_search_available() && !c.images_unsupported(), "{m}");
        }
        // Another host searches nothing, whatever the model is called
        assert!(!cfg(ZHIPU_BASE_URL, "deepseek-v4-pro").is_deepseek_pro());
        assert!(!cfg(ZHIPU_BASE_URL, "deepseek-flash").web_search_available());
    }

    #[test]
    fn anthropic_url_is_the_origin_plus_the_documented_path() {
        let cfg = |base: &str| ProviderConfig::custom(base, "", "");
        for base in ["https://api.deepseek.com", "https://api.deepseek.com/", "https://api.deepseek.com/v1", "api.deepseek.com/v1 "] {
            assert_eq!(cfg(base).anthropic_messages_url(), "https://api.deepseek.com/anthropic/v1/messages", "{base}");
        }
        assert_eq!(cfg("http://localhost:11434/v1").anthropic_messages_url(), "http://localhost:11434/anthropic/v1/messages");
    }

    fn catalog() -> Vec<ModelCap> {
        vec![
            ModelCap { id: "deepseek-flash".into(), default: true, search: true, images: true, max_tokens: Some(16384) },
            ModelCap { id: "deepseek-v4-pro".into(), default: false, search: true, images: false, max_tokens: Some(16384) },
            ModelCap { id: "glm-4.7".into(), default: false, search: false, images: false, max_tokens: None },
        ]
    }

    /// The hosted provider reaches the relay's two endpoints through the same two
    /// URL rules the custom provider uses — no special case, only a base of `…/v1`.
    #[test]
    fn hosted_urls_come_from_the_relay_base() {
        let cfg = ProviderConfig::hosted("tok".into(), "", catalog());
        assert!(cfg.is_hosted());
        assert_eq!(cfg.chat_completions_url(), format!("{HOSTED_API_BASE}/v1/chat/completions"));
        assert_eq!(cfg.anthropic_messages_url(), format!("{HOSTED_API_BASE}/anthropic/v1/messages"));
        assert_eq!(cfg.api_key, "tok");
        // The relay is not api.deepseek.com, but its upstream is: usage and the thinking level are sent
        assert!(!cfg.is_deepseek() && cfg.include_usage() && cfg.reasoning_supported());
        assert_eq!(cfg.stream_idle_timeout().as_secs(), 600);
        assert_eq!(ProviderConfig::custom("https://api.deepseek.com", "", "").stream_idle_timeout().as_secs(), 120);
    }

    /// An empty model picks the catalog's default; a named one is kept (trimmed).
    #[test]
    fn hosted_model_defaults_from_the_catalog() {
        assert_eq!(ProviderConfig::hosted("t".into(), "", catalog()).model, "deepseek-flash");
        assert_eq!(ProviderConfig::hosted("t".into(), " deepseek-v4-pro ", catalog()).model, "deepseek-v4-pro");
        // No default flag anywhere: the first row; an empty catalog: no model at all
        let mut no_default = catalog();
        no_default[0].default = false;
        assert_eq!(ProviderConfig::hosted("t".into(), "", no_default).model, "deepseek-flash");
        assert_eq!(ProviderConfig::hosted("t".into(), "", vec![]).model, "");
    }

    /// Hosted capabilities come from the catalog row, not from the host name.
    #[test]
    fn hosted_capabilities_read_the_catalog() {
        let flash = ProviderConfig::hosted("t".into(), "deepseek-flash", catalog());
        assert!(flash.web_search_available() && !flash.images_unsupported());
        let pro = ProviderConfig::hosted("t".into(), "deepseek-v4-pro", catalog());
        assert!(pro.web_search_available() && pro.images_unsupported());
        let glm = ProviderConfig::hosted("t".into(), "glm-4.7", catalog());
        assert!(!glm.web_search_available() && glm.images_unsupported());
        // A model the catalog does not list: nothing is promised
        let unknown = ProviderConfig::hosted("t".into(), "other", catalog());
        assert!(!unknown.web_search_available() && !unknown.images_unsupported());
        // No catalog yet (the snapshot has not arrived): the same
        let bare = ProviderConfig { catalog: None, ..unknown };
        assert!(!bare.web_search_available() && !bare.images_unsupported());
    }

    /// A stored config from before the kind existed is a custom one, and a hosted
    /// one round-trips with its catalog.
    #[test]
    fn kind_defaults_to_custom_and_round_trips() {
        let old: ProviderConfig = serde_json::from_str(r#"{"base_url":"https://api.deepseek.com","model":"m"}"#).unwrap();
        assert_eq!(old.kind, ProviderKind::Custom);
        assert!(old.catalog.is_none());
        let hosted = ProviderConfig::hosted("t".into(), "", catalog());
        let back: ProviderConfig = serde_json::from_str(&serde_json::to_string(&hosted).unwrap()).unwrap();
        assert_eq!(back.kind, ProviderKind::Hosted);
        assert_eq!(back.catalog, hosted.catalog);
    }

    /// The development override replaces the origin and nothing else; unset or
    /// blank means the real host.
    #[test]
    fn hosted_base_honours_the_env_override() {
        assert_eq!(hosted_base_from(Some("http://127.0.0.1:8080/")), "http://127.0.0.1:8080");
        assert_eq!(hosted_base_from(Some("  ")), HOSTED_API_BASE);
        assert_eq!(hosted_base_from(None), HOSTED_API_BASE);
        let cfg = ProviderConfig { base_url: format!("{}/v1", hosted_base_from(Some("http://127.0.0.1:8080"))), ..ProviderConfig::hosted("t".into(), "", catalog()) };
        assert_eq!(cfg.chat_completions_url(), "http://127.0.0.1:8080/v1/chat/completions");
        assert_eq!(cfg.anthropic_messages_url(), "http://127.0.0.1:8080/anthropic/v1/messages");
    }
}
