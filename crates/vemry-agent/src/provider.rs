//! Provider configuration. Any OpenAI-compatible endpoint works — Zhipu is
//! the free default; Ollama/DeepSeek/OpenAI are a base_url + model away.

use serde::{Deserialize, Serialize};

pub const ZHIPU_BASE_URL: &str = "https://open.bigmodel.cn/api/paas/v4";
pub const DEFAULT_MODEL: &str = "glm-4.7-flash";
/// The one DeepSeek model whose image understanding the documentation marks as
/// unsupported (2026-09-14; `deepseek-flash` takes JPEG/PNG/GIF/WebP natively).
pub const DEEPSEEK_PRO_MODEL: &str = "deepseek-v4-pro";

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ProviderConfig {
    pub base_url: String,
    pub model: String,
    #[serde(skip_serializing_if = "String::is_empty", default)]
    pub api_key: String,
}

impl ProviderConfig {
    pub fn zhipu(api_key: String) -> Self {
        ProviderConfig { base_url: ZHIPU_BASE_URL.into(), model: DEFAULT_MODEL.into(), api_key }
    }

    pub fn chat_completions_url(&self) -> String {
        format!("{}/chat/completions", self.base_url.trim_end_matches('/'))
    }

    /// The Anthropic-format Messages endpoint DeepSeek documents as
    /// `https://api.deepseek.com/anthropic`: the origin of base_url plus that
    /// path, so a base_url spelled with `/v1` (which the Chat Completions line
    /// accepts) reaches the same place.
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

    /// Whether base_url points at the official DeepSeek endpoint (the only place
    /// the Anthropic-format line, and with it web search, is available). Decided
    /// on the host — a missing scheme must not silently flip the classification
    /// back to a plain endpoint.
    pub fn is_deepseek(&self) -> bool {
        self.parsed_base()
            .and_then(|u| u.host_str().map(str::to_owned))
            .is_some_and(|host| host == "api.deepseek.com" || host.ends_with(".deepseek.com"))
    }

    /// The official DeepSeek endpoint with the pro model selected (see
    /// [`DEEPSEEK_PRO_MODEL`] for why this one model is singled out).
    pub fn is_deepseek_pro(&self) -> bool {
        self.is_deepseek() && self.model.trim() == DEEPSEEK_PRO_MODEL
    }

    /// Whether the server-side web_search can take effect: the DeepSeek endpoint,
    /// any of its models (the Anthropic-format line runs the search for flash and
    /// pro alike — measured 2026-09-14). Everything that shows "search is on" or
    /// sends the request with the tool reads this one predicate, so the composer
    /// chip and the request cannot disagree.
    pub fn web_search_available(&self) -> bool {
        self.is_deepseek()
    }

    /// Whether an attached image would be refused by the provider: the pro model
    /// has no image understanding, the flash model and non-DeepSeek endpoints are
    /// left to answer for themselves.
    pub fn images_unsupported(&self) -> bool {
        self.is_deepseek_pro()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_deepseek_classifies_by_host_with_or_without_scheme() {
        let cfg = |base: &str| ProviderConfig {
            base_url: base.into(),
            model: String::new(),
            api_key: String::new(),
        };
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
    fn every_deepseek_model_searches_and_only_pro_refuses_images() {
        let cfg = |base: &str, model: &str| ProviderConfig {
            base_url: base.into(),
            model: model.into(),
            api_key: String::new(),
        };
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
        let cfg = |base: &str| ProviderConfig { base_url: base.into(), model: String::new(), api_key: String::new() };
        for base in ["https://api.deepseek.com", "https://api.deepseek.com/", "https://api.deepseek.com/v1", "api.deepseek.com/v1 "] {
            assert_eq!(cfg(base).anthropic_messages_url(), "https://api.deepseek.com/anthropic/v1/messages", "{base}");
        }
        assert_eq!(cfg("http://localhost:11434/v1").anthropic_messages_url(), "http://localhost:11434/anthropic/v1/messages");
    }
}
