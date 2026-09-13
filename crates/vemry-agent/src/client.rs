//! The LLM channel boundary: streaming Chat Completions over SSE.
//! `ChatTransport` is a trait so the agent loop is fully testable with a mock;
//! anthropic.rs is the second implementation (the Anthropic-format line DeepSeek
//! carries web search on).

use async_trait::async_trait;
use eventsource_stream::Eventsource;
use futures_util::StreamExt;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use crate::provider::ProviderConfig;
use crate::types::{ChatRequest, StreamChunk};
use crate::{AgentError, Result};

/// Idle limit for a streaming read: with no SSE event for this long, the stream
/// is considered dead. Cancellation is polled on CANCEL_TICK — "stop" has to work
/// on a stalled stream too, and cannot wait for the next chunk to observe the
/// cancel flag.
const STREAM_IDLE_TIMEOUT: Duration = Duration::from_secs(120);
const CANCEL_TICK: Duration = Duration::from_secs(2);

/// The process-wide shared HTTP client: the connection pool is reused across
/// turns (previously every turn built a new Client and redid the TLS handshake
/// and connection), with a connect timeout as a backstop. Both transports share
/// this one.
pub(crate) fn shared_client() -> &'static reqwest::Client {
    static CLIENT: std::sync::OnceLock<reqwest::Client> = std::sync::OnceLock::new();
    CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(15))
            .build()
            .unwrap_or_default()
    })
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
pub(crate) async fn post_streaming(
    client: &reqwest::Client,
    url: String,
    config: &ProviderConfig,
    body: &(impl serde::Serialize + Sync),
    auth: Auth,
) -> Result<reqwest::Response> {
    if config.api_key.is_empty() {
        return Err(AgentError::NoApiKey);
    }
    let req = client.post(url);
    let req = match auth {
        Auth::Bearer => req.bearer_auth(&config.api_key),
        Auth::AnthropicKey => req.header("x-api-key", &config.api_key).header("anthropic-version", "2023-06-01"),
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
/// and nowhere else.
pub(crate) async fn read_sse(
    resp: reqwest::Response,
    cancel: &AtomicBool,
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
                if last_activity.elapsed() >= STREAM_IDLE_TIMEOUT {
                    return Err(AgentError::Http(format!(
                        "stream idle timeout: no data for {}s",
                        STREAM_IDLE_TIMEOUT.as_secs()
                    )));
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
        let resp =
            post_streaming(shared_client(), config.chat_completions_url(), config, request, Auth::Bearer).await?;
        read_sse(resp, cancel, &mut |_name, data| {
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
