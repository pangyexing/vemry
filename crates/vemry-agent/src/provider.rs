//! Provider configuration. Any OpenAI-compatible endpoint works — Zhipu is
//! the free default; Ollama/DeepSeek/OpenAI are a base_url + model away.

use serde::{Deserialize, Serialize};

pub const ZHIPU_BASE_URL: &str = "https://open.bigmodel.cn/api/paas/v4";
pub const DEFAULT_MODEL: &str = "glm-4.7-flash";

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

    /// The Responses API endpoint. [to verify · live] The DeepSeek documentation
    /// only gives a base URL, so the path follows the OpenAI convention of
    /// `/responses`.
    pub fn responses_url(&self) -> String {
        format!("{}/responses", self.base_url.trim_end_matches('/'))
    }

    /// Whether base_url points at the official DeepSeek endpoint (the only place
    /// the Responses API / web_search is available). The setting is free text, so
    /// when a missing scheme (`api.deepseek.com`) leaves no host to parse, this
    /// prepends `https://` and tries again — a missing scheme must not silently
    /// flip the classification back to Chat Completions.
    pub fn is_deepseek(&self) -> bool {
        let host_of =
            |s: &str| reqwest::Url::parse(s).ok().and_then(|u| u.host_str().map(str::to_owned));
        let url = self.base_url.trim();
        host_of(url)
            .or_else(|| host_of(&format!("https://{url}")))
            .is_some_and(|host| host == "api.deepseek.com" || host.ends_with(".deepseek.com"))
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
}
