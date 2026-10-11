//! The LLM channel boundary: streaming Chat Completions over SSE.
//! `ChatTransport` is a trait so the agent loop is fully testable with a mock;
//! anthropic.rs is the second implementation (the Anthropic-format line DeepSeek
//! carries web search on).
//!
//! The hosted provider (the relay at api.vemry.app) speaks both shapes verbatim,
//! so neither transport changes for it: the only additions are four request
//! headers — the metering category, a request id, the priority and the client's
//! version — and a longer stream-idle limit (see `ProviderConfig::stream_idle_timeout`).

use async_trait::async_trait;
use eventsource_stream::Eventsource;
use futures_util::StreamExt;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use crate::provider::ProviderConfig;
use crate::types::{ChatRequest, Feature, Priority, StreamChunk};
use crate::{AgentError, Result};

/// Cancellation is polled on CANCEL_TICK — "stop" has to work on a stalled
/// stream too, and cannot wait for the next chunk to observe the cancel flag.
const CANCEL_TICK: Duration = Duration::from_secs(2);

/// The process-wide shared HTTP client: the connection pool is reused across
/// turns (previously every turn built a new Client and redid the TLS handshake
/// and connection), with a connect timeout as a backstop. Both transports share
/// this one, and so do the account calls of the hosted provider.
pub fn shared_client() -> &'static reqwest::Client {
    static CLIENT: std::sync::OnceLock<reqwest::Client> = std::sync::OnceLock::new();
    CLIENT.get_or_init(|| {
        install_tls_provider();
        reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(15))
            .build()
            .unwrap_or_default()
    })
}

/// Installs ring as the process-wide rustls crypto provider, once. reqwest's
/// `rustls-no-provider` feature (the one that keeps aws-lc-rs and its C build
/// out of the tree) refuses to build a client until a provider is installed and
/// does not pick the single compiled-in one by itself. Every client builder in
/// the process calls this first; the app calls it at startup as well, so the
/// order in which the clients are first used does not matter.
pub fn install_tls_provider() {
    if rustls::crypto::CryptoProvider::get_default().is_none() {
        // A concurrent first caller may win the race; that is the same provider.
        let _ = rustls::crypto::ring::default_provider().install_default();
    }
}

/// The client's `User-Agent` towards the relay: `vemry/<version> (<os>; <arch>)`,
/// the version distribution the relay reads and the key it can answer an old
/// protocol with an upgrade hint on.
pub fn user_agent() -> String {
    format!("vemry/{} ({}; {})", env!("CARGO_PKG_VERSION"), std::env::consts::OS, std::env::consts::ARCH)
}

/// `on_chunk` returns false to stop reading (cancellation).
/// `cancel` lets the transport layer honour a stop even while the stream is
/// stalled (no event arriving at all).
#[async_trait]
pub trait ChatTransport: Send + Sync {
    async fn stream_chat(
        &self,
        config: &ProviderConfig,
        request: &ChatRequest,
        cancel: &AtomicBool,
        on_chunk: &mut (dyn FnMut(StreamChunk) -> bool + Send),
    ) -> Result<()>;
}

/// How the key travels: the OpenAI-compatible lines take a bearer token, the
/// Anthropic-format line its own header pair, plus the beta a request body needs
/// when it uses one.
#[derive(Clone, Copy, Debug)]
pub(crate) enum Auth {
    Bearer,
    AnthropicKey { beta: Option<&'static str> },
}

/// Auth check → POST → every non-2xx truncated into an `AgentError::Status`
/// (carrying the status code). Shared by both transports, so the transport-layer
/// error contract (truncation length, message shape) is maintained in one place.
/// `feature` and `priority` only matter on the hosted provider, where they become
/// the `X-Vemry-Feature` and `X-Vemry-Priority` headers beside a fresh request id
/// and the user agent; a custom provider gets the body and the key and nothing
/// else.
pub(crate) async fn post_streaming(
    client: &reqwest::Client,
    url: String,
    config: &ProviderConfig,
    body: &(impl serde::Serialize + Sync),
    auth: Auth,
    feature: Feature,
    priority: Priority,
) -> Result<reqwest::Response> {
    if config.api_key.is_empty() {
        return Err(AgentError::NoApiKey);
    }
    let req = client.post(url);
    let req = match auth {
        Auth::Bearer => req.bearer_auth(&config.api_key),
        Auth::AnthropicKey { beta } => {
            let req = req.header("x-api-key", &config.api_key).header("anthropic-version", "2023-06-01");
            match beta {
                Some(b) => req.header("anthropic-beta", b),
                None => req,
            }
        }
    };
    let req = if config.is_hosted() {
        req.header("X-Vemry-Feature", feature.as_str())
            .header("X-Vemry-Request-Id", uuid::Uuid::new_v4().to_string())
            .header("X-Vemry-Priority", priority.as_str())
            .header(reqwest::header::USER_AGENT, user_agent())
    } else {
        req
    };
    let resp = req
        .json(body)
        .send()
        .await
        .map_err(|e| AgentError::Http(e.to_string()))?;

    let status = resp.status();
    if !status.is_success() {
        let body = resp.text().await.unwrap_or_default();
        let body = body.chars().take(2000).collect::<String>();
        return Err(AgentError::Status { code: status.as_u16(), message: format!("HTTP {status}: {body}") });
    }
    Ok(resp)
}

/// Read an SSE stream event by event, with an idle timeout and cancellation
/// polling. `on_event` receives (event name, data) and returns Ok(false) when the
/// consumer wants to stop. The read loop of both transports is maintained here
/// and nowhere else. `idle` is the provider's limit (`stream_idle_timeout`): with
/// no SSE event for that long the stream is considered dead — comment lines
/// (`: keep-alive`) are not events, which is why the hosted limit is longer.
pub(crate) async fn read_sse(
    resp: reqwest::Response,
    cancel: &AtomicBool,
    idle: Duration,
    on_event: &mut (dyn FnMut(&str, &str) -> Result<bool> + Send),
) -> Result<()> {
    let mut stream = resp.bytes_stream().eventsource();
    let mut last_activity = Instant::now();
    loop {
        if cancel.load(Ordering::Relaxed) {
            return Err(AgentError::Cancelled);
        }
        let event = match tokio::time::timeout(CANCEL_TICK, stream.next()).await {
            Err(_) => {
                if last_activity.elapsed() >= idle {
                    return Err(AgentError::Http(format!("stream idle timeout: no data for {}s", idle.as_secs())));
                }
                continue;
            }
            Ok(None) => return Ok(()),
            Ok(Some(event)) => event.map_err(|e| AgentError::Http(e.to_string()))?,
        };
        last_activity = Instant::now();
        if !on_event(&event.event, &event.data)? {
            return Ok(());
        }
    }
}

pub struct HttpTransport;

impl HttpTransport {
    pub fn new() -> Self {
        HttpTransport
    }
}

impl Default for HttpTransport {
    fn default() -> Self {
        Self::new()
    }
}

/// The transport a request to this provider goes through: Anthropic's own endpoint
/// always speaks its Messages API, and OpenAI's and xAI's their Responses API (see
/// `ProviderConfig::speaks_responses`); elsewhere a turn that searches takes the
/// Anthropic-format line (the one carrying DeepSeek's server-side search), and
/// everything else Chat Completions. The one place the choice is made — the agent,
/// the translator and the page translator all ask here.
pub fn transport_for(config: &ProviderConfig, web_search: bool) -> std::sync::Arc<dyn ChatTransport> {
    if config.speaks_responses() {
        std::sync::Arc::new(crate::responses::ResponsesTransport::new())
    } else if config.is_anthropic() || web_search {
        std::sync::Arc::new(crate::anthropic::AnthropicTransport::new())
    } else {
        std::sync::Arc::new(HttpTransport::new())
    }
}

/// How Zhipu's search is asked to cite: the way every other line's answers cite the
/// web (inline Markdown links, which is what settles into the source table), not by
/// its reference numbers. `{search_result}` is where the API puts the results.
const ZHIPU_SEARCH_PROMPT: &str = "\
Web search results for the user's request:
{search_result}

Use the results that bear on the request. When a sentence draws on a result, cite it with a Markdown link to the result's URL, [title](url), placed inline right after that sentence; never put links on their own line or as list items, and never cite a result by its reference number.";

/// The runner's provider-neutral search tool, as Zhipu's Chat Completions takes it:
/// its own engine on each site (the mainland's pro engine; the international site
/// has one), the sources returned with the answer (`search_result`), and on the
/// mainland the citation prompt above — whose results placeholder only the mainland
/// site documents. Whether to search is left to the API's own intent check.
fn zhipu_search_tool(mainland: bool) -> crate::types::ToolDef {
    let mut search = serde_json::json!({
        "enable": true,
        "search_engine": if mainland { "search_pro" } else { "search_pro_jina" },
        "search_result": true,
    });
    if mainland {
        search["search_prompt"] = serde_json::json!(ZHIPU_SEARCH_PROMPT);
    }
    crate::types::ToolDef::Raw(serde_json::json!({ "type": "web_search", "web_search": search }))
}

/// Whether a tool is the runner's provider-neutral search tool.
fn is_search_tool(tool: &crate::types::ToolDef) -> bool {
    matches!(tool, crate::types::ToolDef::Raw(v) if v.get("type").and_then(|t| t.as_str()) == Some("web_search") && v.get("web_search").is_none())
}

/// The request as this endpoint takes it on Chat Completions: the thinking level
/// re-spelled for the vendor and model (`ProviderConfig::chat_effort_fields`, which
/// sends nothing a model is not known to take); the thinking text of earlier
/// answers sent back only where the vendor reads it
/// (`ProviderConfig::replays_reasoning`) — a strict endpoint refuses the unknown
/// field, and a session that moved from a thinking model would otherwise fail on
/// its next message; and the search tool in Zhipu's shape. Borrowed unchanged when
/// nothing differs.
fn for_endpoint<'a>(config: &ProviderConfig, request: &'a ChatRequest) -> std::borrow::Cow<'a, ChatRequest> {
    let (reasoning_effort, thinking) = config.chat_effort_fields(request.thinking_level());
    let strip = !config.replays_reasoning() && request.messages.iter().any(|m| m.reasoning_content.is_some());
    let zhipu_search = config.vendor() == crate::provider::Vendor::Zhipu
        && request.tools.iter().flatten().any(is_search_tool);
    if !strip && !zhipu_search && reasoning_effort == request.reasoning_effort && thinking == request.thinking {
        return std::borrow::Cow::Borrowed(request);
    }
    let mut owned = request.clone();
    owned.reasoning_effort = reasoning_effort;
    owned.thinking = thinking;
    if strip {
        for m in &mut owned.messages {
            m.reasoning_content = None;
        }
    }
    if zhipu_search {
        for tool in owned.tools.iter_mut().flatten().filter(|t| is_search_tool(t)) {
            *tool = zhipu_search_tool(config.is_zhipu());
        }
    }
    std::borrow::Cow::Owned(owned)
}

/// A finish reason a vendor ends a stream with in place of an HTTP error, turned into
/// the error the runner already knows how to recover from: an overflow compacts
/// (Zhipu), a broken stream or an overloaded server is retried (Zhipu's
/// `network_error`, DeepSeek's `insufficient_system_resource`); a content filter's
/// cut (the standard `content_filter`, Zhipu's `sensitive`) is a refusal.
fn chat_finish(reason: &str) -> std::result::Result<Option<&'static str>, AgentError> {
    match reason {
        "model_context_window_exceeded" => {
            Err(AgentError::Api("model_context_window_exceeded: the context window was exceeded".into()))
        }
        "network_error" | "insufficient_system_resource" => {
            Err(AgentError::Http(format!("{reason}: the provider's stream broke off")))
        }
        "sensitive" | "content_filter" => Ok(Some("refusal")),
        _ => Ok(None),
    }
}

/// One Chat Completions event as a chunk (None for a shape this is not): the usage
/// lifted from inside the choice when a vendor puts it there, and a vendor's
/// finish reason that means an error turned into that error.
fn chat_chunk(data: &str) -> Result<Option<StreamChunk>> {
    let Ok(mut chunk) = serde_json::from_str::<StreamChunk>(data) else { return Ok(None) };
    // Zhipu's search sources ride a top-level `web_search` array (`link`, `content`),
    // a missing date as an empty string
    for r in chunk.web_search.iter_mut().flatten() {
        r.publish_date = r.publish_date.take().filter(|d| !d.trim().is_empty());
    }
    if chunk.usage.is_none() && data.contains("\"usage\"") {
        chunk.usage = serde_json::from_str::<ChoiceUsage>(data).ok().and_then(|c| c.choices.into_iter().find_map(|i| i.usage));
    }
    for choice in &mut chunk.choices {
        if let Some(reason) = choice.finish_reason.as_deref() {
            if let Some(renamed) = chat_finish(reason)? {
                choice.finish_reason = Some(renamed.into());
            }
        }
    }
    Ok(Some(chunk))
}

/// The usage a vendor puts inside the choice rather than beside it (Moonshot's
/// stop chunk), read only when the chunk mentions usage and has none at the top.
#[derive(serde::Deserialize)]
struct ChoiceUsage {
    #[serde(default)]
    choices: Vec<ChoiceUsageItem>,
}

#[derive(serde::Deserialize)]
struct ChoiceUsageItem {
    #[serde(default)]
    usage: Option<crate::types::Usage>,
}

#[async_trait]
impl ChatTransport for HttpTransport {
    async fn stream_chat(
        &self,
        config: &ProviderConfig,
        request: &ChatRequest,
        cancel: &AtomicBool,
        on_chunk: &mut (dyn FnMut(StreamChunk) -> bool + Send),
    ) -> Result<()> {
        let request = for_endpoint(config, request);
        let resp = post_streaming(
            shared_client(),
            config.chat_completions_url(),
            config,
            request.as_ref(),
            Auth::Bearer,
            request.feature,
            request.priority,
        )
        .await?;
        read_sse(resp, cancel, config.stream_idle_timeout(), &mut |_name, data| {
            if data.trim() == "[DONE]" {
                return Ok(false);
            }
            match chat_chunk(data)? {
                Some(chunk) => Ok(on_chunk(chunk)),
                None => Ok(true), // tolerate unknown event shapes
            }
        })
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The thinking text of earlier answers reaches the vendors that read it and
    /// none other; a request with nothing to take out is borrowed, not copied.
    #[test]
    fn reasoning_is_taken_out_for_vendors_without_the_field() {
        let mut answer = crate::types::ChatMessage::assistant(Some("ok".into()), None);
        answer.reasoning_content = Some("why".into());
        let req = ChatRequest {
            feature: Feature::Chat,
            priority: Feature::Chat.priority(),
            model: "m".into(),
            messages: vec![crate::types::ChatMessage::user("hi"), answer],
            tools: None,
            stream: true,
            stream_options: None,
            reasoning_effort: None,
            thinking: None,
        };
        let openai = ProviderConfig::custom("https://api.openai.com/v1", "gpt", "k");
        let sent = serde_json::to_value(for_endpoint(&openai, &req).as_ref()).unwrap();
        assert!(sent["messages"][1].get("reasoning_content").is_none());
        let deepseek = ProviderConfig::custom("https://api.deepseek.com", "deepseek-flash", "k");
        assert!(matches!(for_endpoint(&deepseek, &req), std::borrow::Cow::Borrowed(_)));
        let sent = serde_json::to_value(for_endpoint(&deepseek, &req).as_ref()).unwrap();
        assert_eq!(sent["messages"][1]["reasoning_content"], "why");
    }

    /// The level the runner writes in DeepSeek's spelling reaches each vendor in its
    /// own: kimi-k3 gets an effort for "off", a model of an earlier generation and
    /// an endpoint nobody names nothing at all.
    #[test]
    fn the_level_is_respelled_per_vendor() {
        let req = |effort: Option<&str>, thinking: Option<serde_json::Value>| ChatRequest {
            feature: Feature::Chat,
            priority: Feature::Chat.priority(),
            model: String::new(),
            messages: vec![crate::types::ChatMessage::user("hi")],
            tools: None,
            stream: true,
            stream_options: None,
            reasoning_effort: effort.map(String::from),
            thinking,
        };
        let off = Some(serde_json::json!({ "type": "disabled" }));
        let kimi = ProviderConfig::custom("https://api.moonshot.cn/v1", "kimi-k3", "k");
        let sent = serde_json::to_value(for_endpoint(&kimi, &req(None, off.clone())).as_ref()).unwrap();
        assert_eq!((sent["reasoning_effort"].as_str(), sent.get("thinking")), (Some("low"), None));
        let glm = ProviderConfig::custom(crate::provider::ZHIPU_BASE_URL, "glm-5.3-flash", "k");
        let sent = serde_json::to_value(for_endpoint(&glm, &req(Some("max"), None)).as_ref()).unwrap();
        assert_eq!(sent["reasoning_effort"], "max");
        let earlier = ProviderConfig::custom(crate::provider::ZHIPU_BASE_URL, "glm-4.7-flash", "k");
        let sent = serde_json::to_value(for_endpoint(&earlier, &req(None, off.clone())).as_ref()).unwrap();
        assert!(sent.get("reasoning_effort").is_none() && sent.get("thinking").is_none());
        let other = ProviderConfig::custom("http://localhost:8080/v1", "m", "k");
        let sent = serde_json::to_value(for_endpoint(&other, &req(None, off.clone())).as_ref()).unwrap();
        assert!(sent.get("thinking").is_none());
        let deepseek = ProviderConfig::custom("https://api.deepseek.com", "deepseek-flash", "k");
        let r = req(None, off);
        assert!(matches!(for_endpoint(&deepseek, &r), std::borrow::Cow::Borrowed(_)), "DeepSeek's spelling is the request's own");
    }

    /// The search tool reaches Zhipu in its own shape — the pro engine and the
    /// citation prompt on the mainland, the one engine and no prompt on the
    /// international site — and every other tool unchanged; nobody else is touched.
    #[test]
    fn the_search_tool_takes_zhipus_shape() {
        let req = ChatRequest {
            feature: Feature::Chat,
            priority: Feature::Chat.priority(),
            model: "glm-5.3".into(),
            messages: vec![crate::types::ChatMessage::user("hi")],
            tools: Some(vec![
                crate::types::ToolDef::function(crate::types::FunctionSpec { name: "read_note".into(), description: "d".into(), parameters: serde_json::json!({ "type": "object" }) }),
                crate::types::ToolDef::Raw(serde_json::json!({ "type": "web_search" })),
            ]),
            stream: true,
            stream_options: None,
            reasoning_effort: None,
            thinking: None,
        };
        let mainland = ProviderConfig::custom(crate::provider::ZHIPU_BASE_URL, "glm-5.3", "k");
        let sent = serde_json::to_value(for_endpoint(&mainland, &req).as_ref()).unwrap();
        assert_eq!(sent["tools"][0]["function"]["name"], "read_note");
        let search = &sent["tools"][1];
        assert_eq!(search["type"], "web_search");
        assert_eq!((search["web_search"]["enable"].as_bool(), search["web_search"]["search_engine"].as_str(), search["web_search"]["search_result"].as_bool()), (Some(true), Some("search_pro"), Some(true)));
        assert!(search["web_search"]["search_prompt"].as_str().unwrap().contains("{search_result}"));
        let intl = ProviderConfig::custom("https://api.z.ai/api/paas/v4", "glm-5.3", "k");
        let sent = serde_json::to_value(for_endpoint(&intl, &req).as_ref()).unwrap();
        assert_eq!(sent["tools"][1]["web_search"]["search_engine"], "search_pro_jina");
        assert!(sent["tools"][1]["web_search"].get("search_prompt").is_none());
        let deepseek = ProviderConfig::custom("https://api.deepseek.com", "deepseek-flash", "k");
        assert!(matches!(for_endpoint(&deepseek, &req), std::borrow::Cow::Borrowed(_)));
    }

    /// Zhipu's sources are read off the chunk that carries them, a blank date as none.
    #[test]
    fn zhipu_search_sources_are_read() {
        let data = r#"{"choices":[{"delta":{"content":"x"}}],"web_search":[{"title":"Pears","link":"https://a.b/p","content":"ripen","media":"a.b","refer":"ref_1","publish_date":""},{"title":"Plums","link":"https://a.b/q","publish_date":"2026-10-01"}]}"#;
        let results = chat_chunk(data).unwrap().unwrap().web_search.unwrap();
        assert_eq!((results[0].title.as_str(), results[0].url.as_str(), results[0].snippet.as_str()), ("Pears", "https://a.b/p", "ripen"));
        assert_eq!(results[0].publish_date, None);
        assert_eq!(results[1].publish_date.as_deref(), Some("2026-10-01"));
    }

    /// Moonshot's stop chunk carries its usage inside the choice; the top-level one
    /// wins when both are there.
    #[test]
    fn usage_inside_the_choice_is_lifted() {
        let inside = r#"{"choices":[{"index":0,"delta":{},"finish_reason":"stop","usage":{"prompt_tokens":120,"completion_tokens":8,"cached_tokens":100}}]}"#;
        let chunk = chat_chunk(inside).unwrap().unwrap();
        assert_eq!(chunk.usage.map(|u| (u.prompt_tokens, u.completion_tokens)), Some((120, 8)));
        let both = r#"{"choices":[{"delta":{},"usage":{"prompt_tokens":1}}],"usage":{"prompt_tokens":2,"completion_tokens":3}}"#;
        assert_eq!(chat_chunk(both).unwrap().unwrap().usage.map(|u| u.prompt_tokens), Some(2));
        assert!(chat_chunk("not json").unwrap().is_none());
    }

    /// Zhipu's in-stream finish reasons become what the runner recovers from.
    #[test]
    fn zhipu_finish_reasons_become_errors_the_runner_knows() {
        let overflow = chat_finish("model_context_window_exceeded").unwrap_err();
        assert!(crate::compact::is_context_overflow(&overflow.to_string()));
        assert!(matches!(chat_finish("network_error"), Err(AgentError::Http(_))));
        assert!(matches!(chat_finish("insufficient_system_resource"), Err(AgentError::Http(_))), "DeepSeek's overload is retried");
        assert_eq!(chat_finish("sensitive").unwrap(), Some("refusal"));
        assert_eq!(chat_finish("content_filter").unwrap(), Some("refusal"));
        assert_eq!(chat_finish("stop").unwrap(), None);
    }

    /// The user agent the relay reads: the crate's version (the workspace's, and so
    /// the app's), then the OS and the architecture.
    #[test]
    fn user_agent_names_version_os_and_arch() {
        let ua = user_agent();
        assert!(ua.starts_with(&format!("vemry/{} (", env!("CARGO_PKG_VERSION"))), "{ua}");
        assert!(ua.ends_with(&format!("{}; {})", std::env::consts::OS, std::env::consts::ARCH)), "{ua}");
    }

    /// The feature and the priority never reach the request body: they are headers
    /// on the hosted provider and nothing elsewhere.
    #[test]
    fn feature_is_not_serialised_into_the_body() {
        let req = ChatRequest {
            feature: Feature::Study,
            priority: Feature::Study.priority(),
            model: "m".into(),
            messages: vec![],
            tools: None,
            stream: true,
            stream_options: None,
            reasoning_effort: None,
            thinking: None,
        };
        let body = serde_json::to_value(&req).unwrap();
        assert!(body.get("feature").is_none());
        assert!(body.get("priority").is_none());
        assert_eq!(Feature::Study.as_str(), "study");
        assert_eq!(Feature::Study.priority().as_str(), "interactive");
        assert_eq!(Feature::Translate.priority().as_str(), "background");
    }
}
