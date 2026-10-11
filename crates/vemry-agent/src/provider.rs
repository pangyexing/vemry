//! Provider configuration. Any OpenAI-compatible endpoint works — Zhipu is
//! the free default; Ollama/DeepSeek/OpenAI are a base_url + model away, and
//! Anthropic's own endpoint is spoken to in its native Messages shape.
//!
//! Two kinds of provider (`ProviderKind`): the user's own endpoint and key
//! (`Custom`, everything above), and vemry's hosted access (`Hosted`), where the
//! endpoint is the relay at `api.vemry.app`, the key is the account's device token
//! and what each model can do (search, images, the output ceiling) comes from the
//! model catalog the account snapshot carries rather than from the host name.
//!
//! What a custom endpoint accepts beyond the common protocol — usage on the stream,
//! a thinking level, the thinking text sent back, server-side search — differs per
//! vendor, so it is decided by `Vendor`, read off the host: an interoperability fact
//! about what each host's API takes. An endpoint none of them names gets the common
//! protocol and nothing more.

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

/// The vendor behind a custom endpoint, decided on its host (see the module
/// comment). The host names are the vendors' documented API hosts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Vendor {
    /// `api.deepseek.com`
    DeepSeek,
    /// `open.bigmodel.cn`, and its international twin `api.z.ai`
    Zhipu,
    /// `api.openai.com`
    OpenAi,
    /// `api.anthropic.com`: the native Messages API, not its compatibility layer
    Anthropic,
    /// `api.x.ai`
    XAi,
    /// `api.moonshot.cn` and `api.moonshot.ai`
    Moonshot,
    /// `api.meta.ai`: the Meta Model API (the Llama API it replaced was retired)
    Meta,
    /// A local Ollama on its default port
    Ollama,
    /// Any other OpenAI-compatible endpoint
    Other,
}

impl Vendor {
    /// The vendor a host name (without its port) and port belong to.
    fn of(host: &str, port: Option<u16>) -> Vendor {
        let under = |domain: &str| host == domain || host.ends_with(&format!(".{domain}"));
        if under("deepseek.com") {
            Vendor::DeepSeek
        } else if under("bigmodel.cn") || host == "api.z.ai" {
            Vendor::Zhipu
        } else if host == "api.openai.com" {
            Vendor::OpenAi
        } else if host == "api.anthropic.com" {
            Vendor::Anthropic
        } else if host == "api.x.ai" {
            Vendor::XAi
        } else if host == "api.moonshot.cn" || host == "api.moonshot.ai" {
            Vendor::Moonshot
        } else if host == "api.meta.ai" {
            Vendor::Meta
        } else if (host == "localhost" || host == "127.0.0.1") && port == Some(11434) {
            Vendor::Ollama
        } else {
            Vendor::Other
        }
    }
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

    /// The Messages endpoint. Anthropic's own is `/v1/messages` at the origin of
    /// base_url, so a base_url spelled as its compatibility layer (`…/v1/`) reaches
    /// the native one. Every other host gets the Anthropic-format path DeepSeek
    /// documents as `https://api.deepseek.com/anthropic`: the origin plus that path,
    /// so a base_url spelled with `/v1` (which the Chat Completions line accepts)
    /// reaches the same place. The relay exposes the same shape at the same path,
    /// so the hosted base (`…/v1`) needs no special case.
    pub fn anthropic_messages_url(&self) -> String {
        let origin = self
            .parsed_base()
            .and_then(|u| u.host_str().map(|h| format!("{}://{}{}", u.scheme(), h, u.port().map(|p| format!(":{p}")).unwrap_or_default())))
            .unwrap_or_else(|| self.base_url.trim().trim_end_matches('/').to_string());
        if self.is_anthropic() {
            format!("{origin}/v1/messages")
        } else {
            format!("{origin}/anthropic/v1/messages")
        }
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

    /// The vendor of a custom endpoint, read off its host (a missing scheme must not
    /// silently flip the classification back to a plain endpoint). The relay is
    /// `Other`: every predicate answers for it from its catalog instead.
    pub fn vendor(&self) -> Vendor {
        if self.is_hosted() {
            return Vendor::Other;
        }
        match self.parsed_base() {
            Some(u) => Vendor::of(u.host_str().unwrap_or_default(), u.port_or_known_default()),
            None => Vendor::Other,
        }
    }

    /// Whether base_url points at the official DeepSeek endpoint (where the
    /// Anthropic-format line, and with it web search, is available).
    pub fn is_deepseek(&self) -> bool {
        self.vendor() == Vendor::DeepSeek
    }

    /// Whether base_url points at Zhipu's mainland endpoint — the one host the
    /// `ZHIPU_API_KEY` environment variable may be sent to. Decided on the host, like
    /// `vendor`, and narrower than it: a key from the environment is never sent to
    /// the international endpoint, which issues keys of its own.
    pub fn is_zhipu(&self) -> bool {
        self.host_name().is_some_and(|host| host == "open.bigmodel.cn" || host.ends_with(".bigmodel.cn"))
    }

    /// The line this provider's recorded blocks belong to (see
    /// `ChatMessage::blocks_line`).
    pub fn block_line(&self) -> crate::types::BlockLine {
        match self.vendor() {
            Vendor::Anthropic => crate::types::BlockLine::Anthropic,
            Vendor::OpenAi => crate::types::BlockLine::Openai,
            Vendor::XAi => crate::types::BlockLine::Xai,
            Vendor::Meta => crate::types::BlockLine::Meta,
            _ => crate::types::BlockLine::Deepseek,
        }
    }

    /// Whether every request to this provider goes through the Responses API
    /// (`ResponsesTransport`): OpenAI's, whose Chat Completions calls tools only with
    /// reasoning turned off (its newest models not at all); xAI's, whose Chat
    /// Completions is a legacy endpoint; and Meta's, whose own recommendation it is
    /// (its Chat Completions carries no reasoning across turns and no search).
    pub fn speaks_responses(&self) -> bool {
        matches!(self.vendor(), Vendor::OpenAi | Vendor::XAi | Vendor::Meta)
    }

    /// The Responses endpoint: base_url (`https://api.openai.com/v1`,
    /// `https://api.x.ai/v1`, `https://api.meta.ai/v1`) plus `/responses`.
    pub fn responses_url(&self) -> String {
        format!("{}/responses", self.base_url.trim().trim_end_matches('/'))
    }

    /// Whether base_url points at Anthropic's own API. Every request to it goes
    /// through the native Messages API (`AnthropicTransport`), never the Chat
    /// Completions compatibility layer: only the native one carries prompt caching,
    /// the thinking text and the thinking blocks a tool loop must send back.
    pub fn is_anthropic(&self) -> bool {
        self.vendor() == Vendor::Anthropic
    }

    /// The official DeepSeek endpoint with the pro model selected (see
    /// [`DEEPSEEK_PRO_MODEL`] for why this one model is singled out).
    pub fn is_deepseek_pro(&self) -> bool {
        self.is_deepseek() && self.model.trim() == DEEPSEEK_PRO_MODEL
    }

    /// Whether the server-side web_search can take effect. Custom: the DeepSeek
    /// endpoint, any of its models (the Anthropic-format line runs the search for
    /// flash and pro alike — measured 2026-09-14); Anthropic's, whose Messages API
    /// runs the same server tool; OpenAI's, xAI's and Meta's, whose Responses APIs
    /// run their own; and Zhipu's (both sites) for GLM-5.3, whose Chat Completions
    /// takes a search tool of its own. Hosted: the catalog says so for the selected model. Everything that shows "search is on" or sends the request with the
    /// tool reads this one predicate, so the composer chip and the request cannot
    /// disagree.
    pub fn web_search_available(&self) -> bool {
        match self.kind {
            ProviderKind::Custom => match self.vendor() {
                Vendor::DeepSeek | Vendor::Anthropic | Vendor::OpenAi | Vendor::XAi | Vendor::Meta => true,
                Vendor::Zhipu => glm_searches(self.model.trim()),
                _ => false,
            },
            ProviderKind::Hosted => self.cap().is_some_and(|m| m.search),
        }
    }

    /// Whether an attached image would be refused by the provider. Custom: the
    /// DeepSeek pro model and Zhipu's text-only models have no image understanding;
    /// every other model is left to answer for itself. Hosted: the catalog's
    /// `images` flag.
    pub fn images_unsupported(&self) -> bool {
        match self.kind {
            ProviderKind::Custom => {
                self.is_deepseek_pro()
                    || (self.vendor() == Vendor::Zhipu && ZHIPU_TEXT_ONLY.contains(&self.model.trim()))
            }
            ProviderKind::Hosted => self.cap().is_some_and(|m| !m.images),
        }
    }

    /// Whether `stream_options.include_usage` is sent: the vendors whose Chat
    /// Completions stream carries usage only when asked (DeepSeek, Moonshot; the
    /// relay's upstream is DeepSeek). A Zhipu stream carries it by default, an
    /// endpoint none of the vendors names is not sent a parameter it may refuse, and
    /// the Messages and Responses lines report usage in their final event, unasked.
    pub fn include_usage(&self) -> bool {
        self.is_hosted() || matches!(self.vendor(), Vendor::DeepSeek | Vendor::Moonshot)
    }

    /// Whether a thinking level does something on this model — what shows the
    /// level's controls and sends it at all. Where it travels and how each level is
    /// spelled is per vendor and model (`chat_effort_fields`, `responses_scale`, the
    /// Messages body); a model none of the tables below knows is sent nothing.
    pub fn reasoning_supported(&self) -> bool {
        match self.kind {
            ProviderKind::Hosted => true,
            ProviderKind::Custom => match self.vendor() {
                Vendor::DeepSeek => true,
                Vendor::Anthropic => claude_adaptive(&self.model),
                Vendor::OpenAi | Vendor::XAi | Vendor::Meta => self.responses_scale().is_some(),
                Vendor::Moonshot => kimi_reasoning(self.model.trim()).is_some(),
                Vendor::Zhipu => glm_reasoning(self.model.trim()).is_some(),
                Vendor::Ollama | Vendor::Other => false,
            },
        }
    }

    /// A thinking level (none/low/high/max, or None for the provider's default) as
    /// the two Chat Completions fields this vendor and model take: the effort
    /// (`reasoning_effort`) and, on DeepSeek, the thinking switch (`thinking`). The
    /// two are never both set. A level a model has no word for is moved to the
    /// nearest it has: "none" on a model that always thinks becomes its lowest
    /// effort, "max" on one whose scale ends lower becomes its top.
    pub fn chat_effort_fields(&self, level: Option<&str>) -> (Option<String>, Option<serde_json::Value>) {
        let Some(level) = level else { return (None, None) };
        let off = || Some(serde_json::json!({ "type": "disabled" }));
        let effort = |s: &str| (Some(s.to_string()), None);
        let model = self.model.trim();
        let vendor = if self.is_hosted() { Vendor::DeepSeek } else { self.vendor() };
        let scale = match vendor {
            // DeepSeek's own spelling: the levels as they are, off as the switch
            Vendor::DeepSeek => {
                return match level {
                    "none" => (None, off()),
                    l => effort(l),
                }
            }
            Vendor::Moonshot => kimi_reasoning(model),
            Vendor::Zhipu => glm_reasoning(model),
            // OpenAI, xAI, Meta and Anthropic speak their own APIs; nothing else is sent a level
            _ => None,
        };
        match scale {
            Some(scale) => effort(scale.word(level)),
            None => (None, None),
        }
    }

    /// This model's effort scale on the Responses line (OpenAI, xAI, Meta): None for
    /// a model the tables do not name, which is sent no reasoning at all.
    pub(crate) fn responses_scale(&self) -> Option<Scale> {
        let model = self.model.trim();
        match self.vendor() {
            Vendor::OpenAi => openai_reasoning(model),
            Vendor::XAi => grok_reasoning(model),
            Vendor::Meta => muse_reasoning(model),
            _ => None,
        }
    }

    /// Whether the thinking text of earlier answers (`reasoning_content`) goes back
    /// with the history on the Chat Completions line. DeepSeek requires it on a turn
    /// with tool calls, and so does the relay in front of it; a vendor whose API has
    /// no such field is not sent it, since a strict endpoint refuses an unknown one
    /// (a session that moved from a thinking model would fail on its next message).
    /// An endpoint none of the vendors names keeps getting it, as it always has.
    pub fn replays_reasoning(&self) -> bool {
        match self.kind {
            ProviderKind::Hosted => true,
            ProviderKind::Custom => !matches!(self.vendor(), Vendor::OpenAi | Vendor::XAi | Vendor::Meta | Vendor::Anthropic),
        }
    }

    /// The stream-idle limit: with no SSE event for this long, the stream is
    /// considered dead. 600 s through the relay — the vendor's peak-time queue
    /// sends only `: keep-alive` comment lines for up to ten minutes, and the read
    /// loop counts events, not comments — and 120 s otherwise.
    pub fn stream_idle_timeout(&self) -> std::time::Duration {
        std::time::Duration::from_secs(if self.is_hosted() { 600 } else { 120 })
    }
}

/// Zhipu's current models that read no images: the GLM-5.3 flagship (its flash
/// models do).
const ZHIPU_TEXT_ONLY: &[&str] = &["glm-5.3"];

/// A model's effort scale: the word for the lowest level ("none" where the model
/// can stop thinking, its lowest effort where it cannot) and for the highest.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Scale {
    off: &'static str,
    top: &'static str,
}

impl Scale {
    /// The word for one of vemry's levels (none/low/high/max).
    pub(crate) fn word(self, level: &str) -> &'static str {
        match level {
            "none" => self.off,
            "low" => "low",
            "max" => self.top,
            _ => "high",
        }
    }
}

/// A dotted version at the head of `s`: `6.1-sol` → (6, 1), `6-astra` → (6, 0).
/// None when `s` starts with no digit.
fn dotted_version(s: &str) -> Option<(u32, u32)> {
    let digits = |t: &str| t.chars().take_while(|c| c.is_ascii_digit()).collect::<String>();
    let major = digits(s);
    let rest = &s[major.len()..];
    let minor = rest.strip_prefix('.').map(digits).unwrap_or_default();
    Some((major.parse().ok()?, minor.parse().unwrap_or(0)))
}

// The tables below cover each vendor's current generation (2026-10) and nothing
// older: a model they do not name is sent no level and shows none.

/// OpenAI's GPT-6 models (its model pages): every one reasons, up to "max";
/// gpt-6-sol and gpt-6-luna can stop thinking, gpt-6-astra and gpt-6.1-sol cannot.
fn openai_reasoning(model: &str) -> Option<Scale> {
    let rest = model.strip_prefix("gpt-")?;
    let (major, minor) = dotted_version(rest)?;
    if major != 6 {
        return None;
    }
    let variant = rest.split('-').nth(1).unwrap_or_default();
    let off = if minor == 0 && matches!(variant, "sol" | "luna") { "none" } else { "low" };
    Some(Scale { off, top: "max" })
}

/// xAI's current models (its reasoning guide): grok-4.7, which cannot stop
/// thinking, and grok-4.3, which can; both go up to "xhigh".
fn grok_reasoning(model: &str) -> Option<Scale> {
    match dotted_version(model.strip_prefix("grok-")?)? {
        (4, 7) => Some(Scale { off: "low", top: "xhigh" }),
        (4, 3) => Some(Scale { off: "none", top: "xhigh" }),
        _ => None,
    }
}

/// Meta's Muse Spark 1.3 (its reasoning guide): always thinks, from "minimal" (it
/// refuses "none"); "max" on the standard tier alone, the contributor tier tops
/// out at "xhigh".
fn muse_reasoning(model: &str) -> Option<Scale> {
    let rest = model.strip_prefix("muse-spark-")?;
    if dotted_version(rest)? != (1, 3) {
        return None;
    }
    let top = if rest.contains("contributor") { "xhigh" } else { "max" };
    Some(Scale { off: "minimal", top })
}

/// Moonshot's kimi-k3 (its model overview): always thinks, takes an effort.
fn kimi_reasoning(model: &str) -> Option<Scale> {
    (dotted_version(model.strip_prefix("kimi-k")?)?.0 == 3).then_some(Scale { off: "low", top: "max" })
}

/// Zhipu's GLM-5.3 models (its thinking-mode guide): always think, take an effort.
fn glm_reasoning(model: &str) -> Option<Scale> {
    (dotted_version(model.strip_prefix("glm-")?)? == (5, 3)).then_some(Scale { off: "low", top: "max" })
}

/// Whether a Zhipu model takes the `web_search` tool: GLM-5.3 itself. Its flash
/// models are vision models to the API, whose requests take function tools only.
fn glm_searches(model: &str) -> bool {
    glm_reasoning(model).is_some() && !model.contains("flash")
}

/// A Claude model id's family and version: `claude-opus-5-5` → ("opus", 5, 5),
/// `claude-sonnet-5` → ("sonnet", 5, 0), a dated id read without its date. None for
/// anything else.
fn claude_version(model: &str) -> Option<(&str, u32, u32)> {
    let mut parts = model.trim().strip_prefix("claude-")?.split('-');
    let family = parts.next().filter(|f| f.chars().all(|c| c.is_ascii_alphabetic()) && !f.is_empty())?;
    let major: u32 = parts.next()?.parse().ok()?;
    // A minor is one or two digits; eight are a date
    let minor = parts.next().filter(|m| m.len() <= 2).and_then(|m| m.parse().ok()).unwrap_or(0);
    Some((family, major, minor))
}

/// Whether a Claude model is of the current generation, 5, every model of which
/// takes adaptive thinking (`thinking: {type: "adaptive"}`) and an effort level.
pub(crate) fn claude_adaptive(model: &str) -> bool {
    claude_version(model).is_some_and(|(_, major, _)| major >= 5)
}

/// Whether a Claude model takes the dynamic-filtering web search tool
/// (`web_search_20260209`): Opus and Sonnet. Haiku and Fable keep the basic one.
pub(crate) fn claude_dynamic_search(model: &str) -> bool {
    matches!(claude_version(model), Some(("opus" | "sonnet", major, _)) if major >= 5)
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

    /// The vendor is read off the host (with or without a scheme, a path never
    /// counts); Anthropic's endpoint reaches its native Messages path whatever path
    /// was typed, and runs the search.
    #[test]
    fn vendor_is_read_off_the_host() {
        let cfg = |base: &str| ProviderConfig::custom(base, "", "");
        for (base, vendor) in [
            ("https://api.openai.com/v1", Vendor::OpenAi),
            ("api.anthropic.com/v1/", Vendor::Anthropic),
            ("https://api.x.ai/v1", Vendor::XAi),
            ("https://api.moonshot.cn/v1", Vendor::Moonshot),
            ("https://api.moonshot.ai/v1", Vendor::Moonshot),
            ("https://api.meta.ai/v1", Vendor::Meta),
            ("https://api.llama.com/v1", Vendor::Other),
            ("https://api.z.ai/api/paas/v4", Vendor::Zhipu),
            (ZHIPU_BASE_URL, Vendor::Zhipu),
            ("https://api.deepseek.com", Vendor::DeepSeek),
            ("http://localhost:11434/v1", Vendor::Ollama),
            ("http://localhost:8080/v1", Vendor::Other),
            ("https://evil.com/api.openai.com", Vendor::Other),
            ("https://api.openai.com.evil.com/v1", Vendor::Other),
            ("", Vendor::Other),
        ] {
            assert_eq!(cfg(base).vendor(), vendor, "{base}");
        }
        // The environment's Zhipu key goes to the mainland host alone
        assert!(!cfg("https://api.z.ai/api/paas/v4").is_zhipu());
        let claude = cfg("https://api.anthropic.com/v1/");
        assert_eq!(claude.anthropic_messages_url(), "https://api.anthropic.com/v1/messages");
        assert!(claude.web_search_available() && !claude.is_deepseek());
        assert_eq!(claude.block_line(), crate::types::BlockLine::Anthropic);
        assert_eq!(cfg("https://api.deepseek.com").block_line(), crate::types::BlockLine::Deepseek);
        // The relay is no vendor of its own: the catalog answers for it
        assert_eq!(ProviderConfig::hosted("t".into(), "", catalog()).vendor(), Vendor::Other);
    }

    #[test]
    fn claude_generations() {
        for m in ["claude-opus-5-5", "claude-sonnet-5-5", "claude-haiku-5-5", "claude-fable-5-1", "claude-opus-5"] {
            assert!(claude_adaptive(m), "{m}");
        }
        for m in ["claude-opus-4-6", "claude-haiku-4-5", "claude-sonnet-4-5-20250929", "claude-3-5-haiku-20241022", "gpt-6", ""] {
            assert!(!claude_adaptive(m), "{m}");
        }
        assert!(claude_dynamic_search("claude-opus-5-5") && claude_dynamic_search("claude-sonnet-5-5"));
        assert!(!claude_dynamic_search("claude-haiku-5-5") && !claude_dynamic_search("claude-fable-5-1"));
        assert!(!claude_dynamic_search("claude-sonnet-4-6"));
    }

    /// Each vendor's current models get the level in their own words, through the
    /// fields they take; a model of an earlier generation, or one no table knows,
    /// gets nothing and shows no level.
    #[test]
    fn thinking_levels_are_spelled_per_vendor_and_model() {
        let at = |base: &str, model: &str| ProviderConfig::custom(base, model, "");
        let off = Some(serde_json::json!({ "type": "disabled" }));
        let effort = |s: &str| (Some(s.to_string()), None);
        type Fields = (Option<String>, Option<serde_json::Value>);
        let nothing: Fields = (None, None);
        let cases: Vec<(&str, &str, &str, Fields)> = vec![
            ("https://api.deepseek.com", "deepseek-flash", "none", (None, off.clone())),
            ("https://api.deepseek.com", "deepseek-flash", "max", effort("max")),
            ("https://api.moonshot.cn/v1", "kimi-k3", "none", effort("low")),
            ("https://api.moonshot.ai/v1", "kimi-k3", "max", effort("max")),
            ("https://api.moonshot.cn/v1", "kimi-k2.6", "none", nothing.clone()),
            (ZHIPU_BASE_URL, "glm-5.3", "none", effort("low")),
            ("https://api.z.ai/api/paas/v4", "glm-5.3-flash", "high", effort("high")),
            (ZHIPU_BASE_URL, "glm-4.7-flash", "none", nothing.clone()),
            (ZHIPU_BASE_URL, "glm-5.2", "none", nothing.clone()),
            // The Responses and Messages vendors take no Chat Completions field
            ("https://api.openai.com/v1", "gpt-6-sol", "high", nothing.clone()),
            ("https://api.x.ai/v1", "grok-4.7", "high", nothing.clone()),
            ("http://localhost:11434/v1", "qwen3", "none", nothing.clone()),
        ];
        for (base, model, level, want) in cases {
            assert_eq!(at(base, model).chat_effort_fields(Some(level)), want, "{model} at {level}");
        }
        assert_eq!(at(ZHIPU_BASE_URL, "glm-5.3").chat_effort_fields(None), nothing, "no level, no field");
        let scale = |base: &str, model: &str, level: &str| at(base, model).responses_scale().map(|s| s.word(level));
        assert_eq!(scale("https://api.openai.com/v1", "gpt-6-sol", "none"), Some("none"));
        assert_eq!(scale("https://api.openai.com/v1", "gpt-6-luna", "max"), Some("max"));
        assert_eq!(scale("https://api.openai.com/v1", "gpt-6-astra", "none"), Some("low"), "astra cannot stop thinking");
        assert_eq!(scale("https://api.openai.com/v1", "gpt-6.1-sol", "none"), Some("low"));
        assert_eq!(scale("https://api.x.ai/v1", "grok-4.3", "none"), Some("none"));
        assert_eq!(scale("https://api.x.ai/v1", "grok-4.7", "none"), Some("low"));
        assert_eq!(scale("https://api.x.ai/v1", "grok-4.7", "max"), Some("xhigh"));
        assert_eq!(scale("https://api.meta.ai/v1", "muse-spark-1.3", "none"), Some("minimal"), "Muse refuses none");
        assert_eq!(scale("https://api.meta.ai/v1", "muse-spark-1.3", "max"), Some("max"));
        assert_eq!(scale("https://api.meta.ai/v1", "muse-spark-1.3-contributor", "max"), Some("xhigh"));
        assert_eq!(scale("https://api.meta.ai/v1", "muse-spark-1.2", "high"), None, "an earlier generation");
        for (base, model) in [("https://api.openai.com/v1", "gpt-5.6-sol"), ("https://api.openai.com/v1", "o3"), ("https://api.x.ai/v1", "grok-4.5"), ("https://api.deepseek.com", "gpt-6-sol")] {
            assert_eq!(at(base, model).responses_scale(), None, "{model} at {base}");
        }
        for (base, model) in [
            ("https://api.deepseek.com", "deepseek-flash"),
            ("https://api.x.ai/v1", "grok-4.7"),
            ("https://api.moonshot.cn/v1", "kimi-k3"),
            (ZHIPU_BASE_URL, "glm-5.3-flash"),
            ("https://api.openai.com/v1", "gpt-6-astra"),
            ("https://api.anthropic.com", "claude-opus-5-5"),
        ] {
            assert!(at(base, model).reasoning_supported(), "{model}");
        }
        for (base, model) in [
            ("https://api.x.ai/v1", "grok-4.20-0309-non-reasoning"),
            ("https://api.moonshot.cn/v1", "kimi-k2.6"),
            (ZHIPU_BASE_URL, "glm-4.7-flash"),
            ("https://api.openai.com/v1", "gpt-5.6-sol"),
            ("https://api.anthropic.com", "claude-sonnet-4-6"),
            ("http://localhost:11434/v1", "qwen3"),
        ] {
            assert!(!at(base, model).reasoning_supported(), "{model}");
        }
    }

    /// Usage is asked for where the Chat Completions stream carries it only when
    /// asked; images are refused up front on the current models known to read none;
    /// OpenAI's and xAI's endpoints speak the Responses API on lines of their own.
    #[test]
    fn usage_images_and_routing_per_vendor() {
        let at = |base: &str, model: &str| ProviderConfig::custom(base, model, "");
        for base in ["https://api.deepseek.com", "https://api.moonshot.cn/v1"] {
            assert!(at(base, "m").include_usage(), "{base}");
        }
        for base in [ZHIPU_BASE_URL, "https://api.x.ai/v1", "http://localhost:11434/v1", "https://example.com/v1"] {
            assert!(!at(base, "m").include_usage(), "{base}");
        }
        assert!(at(ZHIPU_BASE_URL, " glm-5.3 ").images_unsupported());
        assert!(!at(ZHIPU_BASE_URL, "glm-5.3-flash").images_unsupported());
        assert!(!at(ZHIPU_BASE_URL, "glm-4.7-flash").images_unsupported(), "an earlier generation is left to answer for itself");
        assert!(!at("https://api.openai.com/v1", "glm-5.3").images_unsupported(), "decided per vendor");
        let openai = at("https://api.openai.com/v1", "gpt-6-luna");
        assert!(openai.web_search_available() && openai.speaks_responses());
        assert_eq!(openai.responses_url(), "https://api.openai.com/v1/responses");
        assert_eq!(openai.block_line(), crate::types::BlockLine::Openai);
        let meta = at("https://api.meta.ai/v1", "muse-spark-1.3");
        assert!(meta.speaks_responses() && meta.web_search_available() && meta.reasoning_supported());
        assert_eq!(meta.responses_url(), "https://api.meta.ai/v1/responses");
        assert_eq!(meta.block_line(), crate::types::BlockLine::Meta);
        let xai = at("https://api.x.ai/v1", "grok-4.7");
        assert!(xai.speaks_responses());
        assert_eq!(xai.responses_url(), "https://api.x.ai/v1/responses");
        assert_eq!(xai.block_line(), crate::types::BlockLine::Xai);
        for base in ["https://api.moonshot.cn/v1", ZHIPU_BASE_URL, "https://api.deepseek.com"] {
            assert!(!at(base, "m").speaks_responses(), "{base}");
        }
        assert!(xai.web_search_available());
        // Zhipu, both sites: GLM-5.3 searches, its flash models (vision models to the API) do not
        assert!(at(ZHIPU_BASE_URL, "glm-5.3").web_search_available());
        assert!(at("https://api.z.ai/api/paas/v4", " glm-5.3 ").web_search_available());
        for model in ["glm-5.3-flash", "glm-5.3-flashx", "glm-4.7-flash"] {
            assert!(!at(ZHIPU_BASE_URL, model).web_search_available(), "{model}");
        }
        assert!(!at("https://api.moonshot.cn/v1", "kimi-k3").web_search_available());
    }

    /// The thinking text goes back where the vendor reads it, and nowhere a strict
    /// API would refuse the field.
    #[test]
    fn reasoning_is_replayed_where_the_vendor_reads_it() {
        let cfg = |base: &str| ProviderConfig::custom(base, "", "");
        for base in ["https://api.deepseek.com", ZHIPU_BASE_URL, "https://api.moonshot.cn/v1", "http://localhost:8080/v1"] {
            assert!(cfg(base).replays_reasoning(), "{base}");
        }
        for base in ["https://api.openai.com/v1", "https://api.x.ai/v1", "https://api.anthropic.com"] {
            assert!(!cfg(base).replays_reasoning(), "{base}");
        }
        assert!(ProviderConfig::hosted("t".into(), "", catalog()).replays_reasoning());
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
