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
/// Anthropic-format line its own header pair.
#[derive(Clone, Copy, Debug)]
pub(crate) enum Auth {
    Bearer,
    AnthropicKey,
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
        Auth::AnthropicKey => req.header("x-api-key", &config.api_key).header("anthropic-version", "2023-06-01"),
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

#[async_trait]
impl ChatTransport for HttpTransport {
    async fn stream_chat(
        &self,
        config: &ProviderConfig,
        request: &ChatRequest,
        cancel: &AtomicBool,
        on_chunk: &mut (dyn FnMut(StreamChunk) -> bool + Send),
    ) -> Result<()> {
        let resp = post_streaming(
            shared_client(),
            config.chat_completions_url(),
            config,
            request,
            Auth::Bearer,
            request.feature,
            request.priority,
        )
        .await?;
        read_sse(resp, cancel, config.stream_idle_timeout(), &mut |_name, data| {
            if data.trim() == "[DONE]" {
                return Ok(false);
            }
            match serde_json::from_str::<StreamChunk>(data) {
                Ok(chunk) => Ok(on_chunk(chunk)),
                Err(_) => Ok(true), // tolerate unknown event shapes
            }
        })
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
