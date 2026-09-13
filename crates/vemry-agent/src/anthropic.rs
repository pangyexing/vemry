//! The Anthropic-format transport (DeepSeek's `/anthropic/v1/messages`): the
//! carrier of web search since 2026-09-14, when the provider's Responses API
//! began listing its built-in tools as ignored while its Anthropic-format API
//! kept `server_tool_use` and `web_search_tool_result` as supported — and, unlike
//! the Responses line ever did, returns **structured** results (title, url, page
//! age) and a per-request search count in the usage.
//!
//! Like the Responses transport before it, it disguises itself as Chat
//! Completions to the runner: `ChatRequest` → a Messages body on the way out
//! (system messages → `system`, a tool result → a `tool_result` block in a user
//! message, function tools → `input_schema` tools, the runner's provider-neutral
//! `{"type":"web_search"}` → the `web_search_20250305` server tool), and the
//! semantic SSE events → `StreamChunk`s on the way back. The runner, the mock
//! transport and the event stream need no change — `ChatTransport` is the seam.
//!
//! The replay contract (what an assistant turn carries into the next request):
//! - the text and the function calls live in `ChatMessage.content`/`tool_calls`
//!   as on every line (so a session can move between lines when the search
//!   switch changes);
//! - the thinking text lives in `reasoning_content` as on the Chat Completions
//!   line, and only its **signature** is kept in `extra_items` as a
//!   `{"type":"thinking","signature":…}` stub — the block is rebuilt from the two
//!   on replay, so the sidecar holds the text once;
//! - `server_tool_use` and `web_search_tool_result` blocks are kept verbatim in
//!   `extra_items` (the results travel as `encrypted_content`, which is how the
//!   server restores them without the client ever holding the page text);
//! - an `extra_items` entry of another line's shape (a Responses
//!   `web_search_call` from an older session) is not an Anthropic block and is
//!   left out.
//!
//! Measured live (2026-09-14, deepseek-flash and deepseek-v4-pro): one message can
//! carry a `server_tool_use`, a function `tool_use` and the `web_search_tool_result`
//! together, the stream ends with `stop_reason:"tool_use"`, and the signature of a
//! thinking block is the message id.

use async_trait::async_trait;
use serde_json::{json, Map, Value};
use std::sync::atomic::AtomicBool;

use crate::client::{Auth, ChatTransport};
use crate::provider::ProviderConfig;
use crate::types::{
    ChatRequest, Content, Delta, FunctionDelta, Part, StreamChoice, StreamChunk, ToolCallDelta, ToolDef, Usage,
    WebSearchResult, WebSearchStatus,
};
use crate::{AgentError, Result};

/// The Messages API makes `max_tokens` mandatory. The provider allows up to 384K;
/// this is the ceiling for one model call, high enough that a long note written
/// in one go is not cut off, and only what is produced is billed.
pub const MAX_OUTPUT_TOKENS: u64 = 65_536;

/// Searches one model call may run (the server tool's `max_uses`): a bound on the
/// separately billed search requests, generous enough for a comparison across
/// several queries.
pub const WEB_SEARCH_MAX_USES: u64 = 5;

/// The provider-neutral spelling the runner puts in the tool table; this transport
/// translates it into the Anthropic server tool.
const WEB_SEARCH_TOOL_TYPE: &str = "web_search";
const ANTHROPIC_WEB_SEARCH_TYPE: &str = "web_search_20250305";

pub struct AnthropicTransport;

impl AnthropicTransport {
    pub fn new() -> Self {
        AnthropicTransport
    }
}

impl Default for AnthropicTransport {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl ChatTransport for AnthropicTransport {
    async fn stream_chat(
        &self,
        config: &ProviderConfig,
        request: &ChatRequest,
        cancel: &AtomicBool,
        on_chunk: &mut (dyn FnMut(StreamChunk) -> bool + Send),
    ) -> Result<()> {
        let body = messages_request_body(request);
        let resp = crate::client::post_streaming(
            crate::client::shared_client(),
            config.anthropic_messages_url(),
            config,
            &body,
            Auth::AnthropicKey,
        )
        .await?;

        let mut parser = MessagesParser::default();
        crate::client::read_sse(resp, cancel, &mut |event_name, data| {
            let Ok(data) = serde_json::from_str::<Value>(data) else {
                return Ok(true); // tolerate an unknown event shape
            };
            let ty = data
                .get("type")
                .and_then(|t| t.as_str())
                .map(String::from)
                .unwrap_or_else(|| event_name.to_string());
            if ty == "error" {
                let msg = data
                    .pointer("/error/message")
                    .and_then(|v| v.as_str())
                    .unwrap_or("stream error");
                return Err(AgentError::Api(format!("error: {msg}")));
            }
            for chunk in parser.handle(&ty, &data) {
                if !on_chunk(chunk) {
                    return Ok(false);
                }
            }
            Ok(true)
        })
        .await
    }
}

// ---- request mapping ----

/// A user message body → Anthropic content blocks: plain text is one `text`
/// block; a multi-modal body maps part by part, an image becoming a base64
/// `image` block (the data URL the runner filled in is split into media type and
/// payload). An `ImageRef` should never reach here (the runner fills it in before
/// assembling the request), so it degrades to a one-line note rather than a 400.
fn user_blocks(content: Option<&Content>) -> Vec<Value> {
    match content {
        None => vec![],
        Some(Content::Text(t)) => vec![text_block(t)],
        Some(Content::Parts(parts)) => parts
            .iter()
            .map(|p| match p {
                Part::Text { text } => text_block(text),
                Part::ImageUrl { image_url } => image_block(&image_url.url),
                Part::ImageRef { path } => text_block(&format!("(the image {path} was not loaded)")),
            })
            .collect(),
    }
}

fn text_block(text: &str) -> Value {
    json!({ "type": "text", "text": text })
}

/// `data:<media type>;base64,<payload>` → the base64 image source; anything else
/// (an http URL) goes through as a URL source.
fn image_block(url: &str) -> Value {
    if let Some(rest) = url.strip_prefix("data:") {
        if let Some((meta, data)) = rest.split_once(',') {
            let media_type = meta.strip_suffix(";base64").unwrap_or(meta);
            return json!({
                "type": "image",
                "source": { "type": "base64", "media_type": media_type, "data": data }
            });
        }
    }
    json!({ "type": "image", "source": { "type": "url", "url": url } })
}

/// Whether an `extra_items` entry is a block of this line (see the module
/// comment: another line's items are left out).
fn is_anthropic_block(item: &Value) -> bool {
    matches!(
        item.get("type").and_then(|t| t.as_str()),
        Some("thinking") | Some("redacted_thinking") | Some("server_tool_use") | Some("web_search_tool_result")
    )
}

/// An assistant turn → its content blocks, in the order the API wants them: the
/// thinking first (rebuilt from `reasoning_content` plus the signature stub, or
/// emitted with an empty signature when the turn came from the Chat Completions
/// line and has none), then the server tool blocks verbatim, then the text, then
/// the function calls.
fn assistant_blocks(msg: &crate::types::ChatMessage) -> Vec<Value> {
    let mut blocks = vec![];
    if let Some(reasoning) = msg.reasoning_content.as_deref().filter(|r| !r.is_empty()) {
        let signature = msg
            .extra_items
            .iter()
            .find(|i| i.get("type").and_then(|t| t.as_str()) == Some("thinking"))
            .and_then(|i| i.get("signature"))
            .and_then(|s| s.as_str())
            .unwrap_or_default();
        blocks.push(json!({ "type": "thinking", "thinking": reasoning, "signature": signature }));
    }
    for item in msg.extra_items.iter().filter(|i| is_anthropic_block(i)) {
        if item.get("type").and_then(|t| t.as_str()) == Some("thinking") {
            continue; // the stub was consumed above
        }
        blocks.push(item.clone());
    }
    if let Some(text) = msg.text().filter(|t| !t.is_empty()) {
        blocks.push(text_block(text));
    }
    for call in msg.tool_calls.iter().flatten() {
        let input = serde_json::from_str::<Value>(&call.function.arguments)
            .ok()
            .filter(|v| v.is_object())
            .unwrap_or_else(|| json!({}));
        blocks.push(json!({ "type": "tool_use", "id": call.id, "name": call.function.name, "input": input }));
    }
    blocks
}

/// A function tool → `{name, description, input_schema}`; the runner's
/// `{"type":"web_search"}` → the server tool with `max_uses` (any further key on
/// the Raw body, a probe's `allowed_domains` say, is carried over); another Raw
/// tool passes through untouched.
fn tool_value(tool: &ToolDef) -> Value {
    match tool {
        ToolDef::Function { function, .. } => json!({
            "name": function.name,
            "description": function.description,
            "input_schema": function.parameters,
        }),
        ToolDef::Raw(v) if v.get("type").and_then(|t| t.as_str()) == Some(WEB_SEARCH_TOOL_TYPE) => {
            let mut out = Map::new();
            out.insert("type".into(), json!(ANTHROPIC_WEB_SEARCH_TYPE));
            out.insert("name".into(), json!(WEB_SEARCH_TOOL_TYPE));
            out.insert("max_uses".into(), json!(WEB_SEARCH_MAX_USES));
            if let Some(extra) = v.as_object() {
                for (k, val) in extra {
                    if k != "type" {
                        out.insert(k.clone(), val.clone());
                    }
                }
            }
            Value::Object(out)
        }
        ToolDef::Raw(v) => v.clone(),
    }
}

/// `ChatRequest` → a Messages request body. A pure function, covered by offline
/// tests. Messages of one role that follow each other are merged into one (the
/// API wants user and assistant to alternate, and every tool result of one
/// assistant turn in a single user message).
pub fn messages_request_body(request: &ChatRequest) -> Value {
    let mut system: Vec<String> = vec![];
    let mut messages: Vec<(String, Vec<Value>)> = vec![];
    let mut push = |role: &str, blocks: Vec<Value>| {
        if blocks.is_empty() {
            return;
        }
        match messages.last_mut() {
            Some((last_role, content)) if last_role == role => content.extend(blocks),
            _ => messages.push((role.to_string(), blocks)),
        }
    };
    for msg in &request.messages {
        match msg.role.as_str() {
            "system" => {
                if let Some(c) = msg.text().filter(|c| !c.is_empty()) {
                    system.push(c.to_string());
                }
            }
            "user" => push("user", user_blocks(msg.content.as_ref())),
            "assistant" => push("assistant", assistant_blocks(msg)),
            "tool" => push(
                "user",
                vec![json!({
                    "type": "tool_result",
                    "tool_use_id": msg.tool_call_id.clone().unwrap_or_default(),
                    "content": msg.text().unwrap_or_default(),
                })],
            ),
            _ => {}
        }
    }
    let mut body = json!({
        "model": request.model,
        "max_tokens": MAX_OUTPUT_TOKENS,
        "stream": true,
        "messages": messages
            .into_iter()
            .map(|(role, content)| json!({ "role": role, "content": content }))
            .collect::<Vec<_>>(),
    });
    if !system.is_empty() {
        body["system"] = json!(system.join("\n\n"));
    }
    // Thinking: the Chat Completions line's two fields restored into this line's
    // shape — thinking:{"type":"disabled"} as is, a level as thinking enabled plus
    // output_config.effort; nothing configured sends nothing (the provider's
    // default applies)
    if request.thinking.as_ref().and_then(|t| t.get("type")).and_then(|v| v.as_str()) == Some("disabled") {
        body["thinking"] = json!({ "type": "disabled" });
    } else if let Some(e) = &request.reasoning_effort {
        body["thinking"] = json!({ "type": "enabled" });
        body["output_config"] = json!({ "effort": e });
    }
    if let Some(tools) = request.tools.as_ref().filter(|t| !t.is_empty()) {
        body["tools"] = Value::Array(tools.iter().map(tool_value).collect());
    }
    body
}

// ---- streaming ----

/// What the parser remembers about one content block between its start and stop.
enum BlockState {
    /// Only the signature is collected: the text goes out as reasoning deltas and
    /// the runner keeps it.
    Thinking { signature: String },
    Text,
    /// A function call: its tool-call slot (the runner's accumulator index) and
    /// whether an arguments delta ever arrived (otherwise `{}` is emitted at stop).
    ToolUse { slot: usize, got_args: bool },
    /// A server-side search: the block as it started (the input is filled in from
    /// the JSON deltas at stop).
    ServerToolUse { block: Value, input_json: String },
    Other,
}

/// Semantic SSE events → a `StreamChunk` synthesiser. A pure state machine,
/// tested offline against a captured stream.
#[derive(Default)]
pub struct MessagesParser {
    /// By content block index (the API numbers them from 0 in order).
    blocks: Vec<Option<BlockState>>,
    /// Function calls seen so far — the next one's slot.
    calls: usize,
    /// The usage of message_start, completed by message_delta.
    input_tokens: u64,
    cache_read_tokens: u64,
    cache_creation_tokens: u64,
}

impl MessagesParser {
    pub fn handle(&mut self, ty: &str, data: &Value) -> Vec<StreamChunk> {
        match ty {
            "message_start" => {
                if let Some(u) = data.pointer("/message/usage") {
                    self.take_input_usage(u);
                }
                vec![]
            }
            "content_block_start" => self.on_block_start(data),
            "content_block_delta" => self.on_block_delta(data),
            "content_block_stop" => self.on_block_stop(data),
            "message_delta" => self.on_message_delta(data),
            _ => vec![],
        }
    }

    fn take_input_usage(&mut self, usage: &Value) {
        let n = |k: &str| usage.get(k).and_then(|v| v.as_u64());
        if let Some(v) = n("input_tokens") {
            self.input_tokens = v;
        }
        if let Some(v) = n("cache_read_input_tokens") {
            self.cache_read_tokens = v;
        }
        if let Some(v) = n("cache_creation_input_tokens") {
            self.cache_creation_tokens = v;
        }
    }

    fn set_block(&mut self, index: usize, state: BlockState) {
        while self.blocks.len() <= index {
            self.blocks.push(None);
        }
        self.blocks[index] = Some(state);
    }

    fn on_block_start(&mut self, data: &Value) -> Vec<StreamChunk> {
        let index = data.get("index").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
        let Some(block) = data.get("content_block") else { return vec![] };
        let text_of = |k: &str| block.get(k).and_then(|v| v.as_str()).filter(|s| !s.is_empty()).map(String::from);
        match block.get("type").and_then(|t| t.as_str()) {
            Some("thinking") => {
                let signature = block.get("signature").and_then(|v| v.as_str()).unwrap_or_default().to_string();
                self.set_block(index, BlockState::Thinking { signature });
                text_of("thinking").map(reasoning_chunk).into_iter().collect()
            }
            Some("text") => {
                self.set_block(index, BlockState::Text);
                text_of("text").map(content_chunk).into_iter().collect()
            }
            Some("tool_use") => {
                let slot = self.calls;
                self.calls += 1;
                let id = block.get("id").and_then(|v| v.as_str()).unwrap_or_default();
                let name = block.get("name").and_then(|v| v.as_str()).unwrap_or_default();
                // A complete input given at start (no deltas to follow) is emitted whole
                let whole = block.get("input").filter(|i| i.as_object().is_some_and(|o| !o.is_empty())).map(Value::to_string);
                self.set_block(index, BlockState::ToolUse { slot, got_args: whole.is_some() });
                vec![tool_call_chunk(slot, Some(id), Some(name), whole.as_deref())]
            }
            Some("server_tool_use") => {
                self.set_block(index, BlockState::ServerToolUse { block: block.clone(), input_json: String::new() });
                vec![StreamChunk { web_search_status: Some(WebSearchStatus::Searching), ..Default::default() }]
            }
            Some("web_search_tool_result") => {
                // The block arrives complete at start: the results for the source
                // card, and the block verbatim for the replay
                self.set_block(index, BlockState::Other);
                let results = search_results(block);
                vec![StreamChunk {
                    web_search: (!results.is_empty()).then_some(results),
                    web_search_status: Some(WebSearchStatus::Completed),
                    raw_items: Some(vec![block.clone()]),
                    ..Default::default()
                }]
            }
            // redacted_thinking and whatever comes later: kept verbatim for the replay
            _ => {
                self.set_block(index, BlockState::Other);
                vec![StreamChunk { raw_items: Some(vec![block.clone()]), ..Default::default() }]
            }
        }
    }

    fn on_block_delta(&mut self, data: &Value) -> Vec<StreamChunk> {
        let index = data.get("index").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
        let Some(delta) = data.get("delta") else { return vec![] };
        let Some(Some(state)) = self.blocks.get_mut(index) else { return vec![] };
        match delta.get("type").and_then(|t| t.as_str()) {
            Some("text_delta") => delta.get("text").and_then(|v| v.as_str()).map(|t| content_chunk(t.to_string())).into_iter().collect(),
            Some("thinking_delta") => {
                delta.get("thinking").and_then(|v| v.as_str()).map(|t| reasoning_chunk(t.to_string())).into_iter().collect()
            }
            Some("signature_delta") => {
                if let BlockState::Thinking { signature } = state {
                    signature.push_str(delta.get("signature").and_then(|v| v.as_str()).unwrap_or_default());
                }
                vec![]
            }
            Some("input_json_delta") => {
                let partial = delta.get("partial_json").and_then(|v| v.as_str()).unwrap_or_default();
                match state {
                    BlockState::ToolUse { slot, got_args } => {
                        *got_args = true;
                        vec![tool_call_chunk(*slot, None, None, Some(partial))]
                    }
                    BlockState::ServerToolUse { input_json, .. } => {
                        input_json.push_str(partial);
                        vec![]
                    }
                    _ => vec![],
                }
            }
            _ => vec![],
        }
    }

    fn on_block_stop(&mut self, data: &Value) -> Vec<StreamChunk> {
        let index = data.get("index").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
        let Some(slot) = self.blocks.get_mut(index) else { return vec![] };
        match slot.take() {
            Some(BlockState::Thinking { signature }) => vec![StreamChunk {
                raw_items: Some(vec![json!({ "type": "thinking", "signature": signature })]),
                ..Default::default()
            }],
            Some(BlockState::ToolUse { slot, got_args: false }) => vec![tool_call_chunk(slot, None, None, Some("{}"))],
            Some(BlockState::ServerToolUse { mut block, input_json }) => {
                let input = serde_json::from_str::<Value>(&input_json).ok().filter(|v| v.is_object());
                if let Some(input) = input {
                    block["input"] = input;
                }
                let query = block
                    .pointer("/input/query")
                    .and_then(|q| q.as_str())
                    .filter(|q| !q.trim().is_empty())
                    .map(|q| vec![q.to_string()]);
                vec![StreamChunk { web_search_queries: query, raw_items: Some(vec![block]), ..Default::default() }]
            }
            _ => vec![],
        }
    }

    fn on_message_delta(&mut self, data: &Value) -> Vec<StreamChunk> {
        let mut usage = Usage::default();
        if let Some(u) = data.get("usage") {
            self.take_input_usage(u);
            usage.completion_tokens = u.get("output_tokens").and_then(|v| v.as_u64()).unwrap_or(0);
            usage.web_search_requests = u.pointer("/server_tool_use/web_search_requests").and_then(|v| v.as_u64());
        }
        // Anthropic semantics: input_tokens is what was not served from the cache,
        // so the context is the three added up (measured on this endpoint: a second
        // identical request moves the tokens from input into cache_read)
        usage.prompt_tokens = self.input_tokens + self.cache_read_tokens + self.cache_creation_tokens;
        usage.prompt_cache_hit_tokens = Some(self.cache_read_tokens);
        let finish = match data.pointer("/delta/stop_reason").and_then(|v| v.as_str()) {
            Some("tool_use") => "tool_calls",
            Some("max_tokens") => "length",
            _ => "stop",
        };
        vec![StreamChunk {
            choices: vec![StreamChoice { delta: Delta::default(), finish_reason: Some(finish.into()) }],
            usage: Some(usage),
            ..Default::default()
        }]
    }
}

/// The results of a `web_search_tool_result` block: title, url and page age (an
/// error content, or a result without a url, yields nothing).
fn search_results(block: &Value) -> Vec<WebSearchResult> {
    let Some(items) = block.get("content").and_then(|c| c.as_array()) else { return vec![] };
    let mut out: Vec<WebSearchResult> = vec![];
    for item in items {
        if item.get("type").and_then(|t| t.as_str()) != Some("web_search_result") {
            continue;
        }
        let url = item.get("url").and_then(|v| v.as_str()).unwrap_or_default();
        if url.is_empty() || out.iter().any(|r| r.url == url) {
            continue;
        }
        out.push(WebSearchResult {
            title: item.get("title").and_then(|v| v.as_str()).unwrap_or_default().to_string(),
            url: url.to_string(),
            snippet: String::new(),
            publish_date: item
                .get("page_age")
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty() && *s != "None" && *s != "null")
                .map(String::from),
        });
    }
    out
}

fn content_chunk(text: String) -> StreamChunk {
    StreamChunk {
        choices: vec![StreamChoice { delta: Delta { content: Some(text), ..Default::default() }, finish_reason: None }],
        ..Default::default()
    }
}

fn reasoning_chunk(text: String) -> StreamChunk {
    StreamChunk {
        choices: vec![StreamChoice {
            delta: Delta { reasoning_content: Some(text), ..Default::default() },
            finish_reason: None,
        }],
        ..Default::default()
    }
}

fn tool_call_chunk(index: usize, id: Option<&str>, name: Option<&str>, args: Option<&str>) -> StreamChunk {
    StreamChunk {
        choices: vec![StreamChoice {
            delta: Delta {
                tool_calls: Some(vec![ToolCallDelta {
                    index: Some(index),
                    id: id.map(String::from),
                    function: Some(FunctionDelta { name: name.map(String::from), arguments: args.map(String::from) }),
                }]),
                ..Default::default()
            },
            finish_reason: None,
        }],
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{ChatMessage, FunctionCall, FunctionSpec, ImageUrl, ToolCall, ToolCallAccumulator};

    fn request(messages: Vec<ChatMessage>) -> ChatRequest {
        ChatRequest {
            model: "deepseek-flash".into(),
            messages,
            tools: None,
            stream: true,
            stream_options: None,
            reasoning_effort: None,
            thinking: None,
        }
    }

    /// The whole multi-turn tool loop in one body: system → `system`; the assistant
    /// turn rebuilt as thinking (text from reasoning_content, signature from the
    /// stub) → the search blocks verbatim → text → tool_use; the two tool results
    /// merged into one user message; a Responses-shaped leftover left out.
    #[test]
    fn request_maps_the_tool_loop_and_replays_search_blocks() {
        let mut assistant = ChatMessage::assistant(
            Some("Looking.".into()),
            Some(vec![
                ToolCall { id: "call_1".into(), kind: "function".into(), function: FunctionCall { name: "read_note".into(), arguments: r#"{"path":"A.md"}"#.into() } },
                ToolCall { id: "call_2".into(), kind: "function".into(), function: FunctionCall { name: "week".into(), arguments: "not json".into() } },
            ]),
        );
        assistant.reasoning_content = Some("hmm".into());
        assistant.extra_items = vec![
            json!({ "type": "thinking", "signature": "sig-1" }),
            json!({ "type": "server_tool_use", "id": "srv_1", "name": "web_search", "input": { "query": "q" } }),
            json!({ "type": "web_search_tool_result", "tool_use_id": "srv_1", "content": [{ "type": "web_search_result", "url": "https://x.y", "encrypted_content": "…" }] }),
            json!({ "type": "web_search_call", "id": "ws_old" }),
        ];
        let req = ChatRequest {
            tools: Some(vec![
                ToolDef::function(FunctionSpec { name: "read_note".into(), description: "d".into(), parameters: json!({ "type": "object" }) }),
                ToolDef::Raw(json!({ "type": "web_search" })),
            ]),
            reasoning_effort: Some("max".into()),
            ..request(vec![
                ChatMessage::system("Be terse."),
                ChatMessage::user("hi"),
                assistant,
                ChatMessage::tool_result("call_1", "# A"),
                ChatMessage::tool_result("call_2", "Mon"),
            ])
        };
        let body = messages_request_body(&req);
        assert_eq!(body["system"], "Be terse.");
        assert_eq!(body["max_tokens"], MAX_OUTPUT_TOKENS);
        assert_eq!(body["thinking"], json!({ "type": "enabled" }));
        assert_eq!(body["output_config"], json!({ "effort": "max" }));
        let msgs = body["messages"].as_array().unwrap();
        assert_eq!(msgs.len(), 3, "user, assistant, one merged tool-result user message");
        assert_eq!(msgs[0], json!({ "role": "user", "content": [{ "type": "text", "text": "hi" }] }));
        let blocks = msgs[1]["content"].as_array().unwrap();
        let kinds: Vec<&str> = blocks.iter().map(|b| b["type"].as_str().unwrap()).collect();
        assert_eq!(kinds, ["thinking", "server_tool_use", "web_search_tool_result", "text", "tool_use", "tool_use"]);
        assert_eq!(blocks[0], json!({ "type": "thinking", "thinking": "hmm", "signature": "sig-1" }));
        assert_eq!(blocks[2]["content"][0]["encrypted_content"], "…", "the results travel back verbatim");
        assert_eq!(blocks[4]["input"], json!({ "path": "A.md" }));
        assert_eq!(blocks[5]["input"], json!({}), "unparseable arguments degrade to an empty object");
        assert_eq!(msgs[2]["role"], "user");
        assert_eq!(msgs[2]["content"][0], json!({ "type": "tool_result", "tool_use_id": "call_1", "content": "# A" }));
        assert_eq!(msgs[2]["content"][1]["tool_use_id"], "call_2");
        let tools = body["tools"].as_array().unwrap();
        assert_eq!(tools[0], json!({ "name": "read_note", "description": "d", "input_schema": { "type": "object" } }));
        assert_eq!(tools[1], json!({ "type": "web_search_20250305", "name": "web_search", "max_uses": WEB_SEARCH_MAX_USES }));
    }

    /// thinking:{"type":"disabled"} goes through as is; an unconfigured level sends
    /// neither field; a probe's extra key on the search tool is carried over.
    #[test]
    fn request_thinking_off_and_unconfigured_and_tool_extras() {
        let off = ChatRequest { thinking: Some(json!({ "type": "disabled" })), ..request(vec![ChatMessage::user("x")]) };
        let body = messages_request_body(&off);
        assert_eq!(body["thinking"], json!({ "type": "disabled" }));
        assert!(body.get("output_config").is_none());
        let plain = messages_request_body(&request(vec![ChatMessage::user("x")]));
        assert!(plain.get("thinking").is_none() && plain.get("system").is_none() && plain.get("tools").is_none());
        let probe = ChatRequest {
            tools: Some(vec![ToolDef::Raw(json!({ "type": "web_search", "allowed_domains": ["a.b"] }))]),
            ..request(vec![ChatMessage::user("x")])
        };
        assert_eq!(messages_request_body(&probe)["tools"][0]["allowed_domains"], json!(["a.b"]));
    }

    /// A multi-modal user message: text → text block, a data URL → a base64 image
    /// source with the media type split off; a Chat-Completions-line assistant turn
    /// (reasoning, no signature stub) still gets its thinking block, with an empty
    /// signature.
    #[test]
    fn request_maps_images_and_a_signatureless_thinking() {
        let mut assistant = ChatMessage::assistant(Some("ok".into()), None);
        assistant.reasoning_content = Some("why".into());
        let req = request(vec![
            ChatMessage::user_content(Content::Parts(vec![
                Part::Text { text: "看图".into() },
                Part::ImageUrl { image_url: ImageUrl { url: "data:image/png;base64,AA==".into() } },
            ])),
            assistant,
        ]);
        let body = messages_request_body(&req);
        let user = body["messages"][0]["content"].as_array().unwrap();
        assert_eq!(user[0], json!({ "type": "text", "text": "看图" }));
        assert_eq!(user[1], json!({ "type": "image", "source": { "type": "base64", "media_type": "image/png", "data": "AA==" } }));
        let assistant = body["messages"][1]["content"].as_array().unwrap();
        assert_eq!(assistant[0], json!({ "type": "thinking", "thinking": "why", "signature": "" }));
        assert_eq!(assistant[1], json!({ "type": "text", "text": "ok" }));
    }

    /// The stream captured live on 2026-09-14 (deepseek-flash, thinking on, a
    /// search tool and a function tool): thinking → server_tool_use → tool_use →
    /// web_search_tool_result → message_delta. The parser yields the reasoning and
    /// the signature stub, the search status, query, results and the verbatim
    /// blocks, the function call assembled through the accumulator, the usage with
    /// the search count, and finish_reason tool_calls.
    #[test]
    fn parser_walks_a_captured_search_and_tool_stream() {
        let events: Vec<Value> = vec![
            json!({ "type": "message_start", "message": { "usage": { "input_tokens": 300, "cache_creation_input_tokens": 0, "cache_read_input_tokens": 65, "output_tokens": 0 } } }),
            json!({ "type": "content_block_start", "index": 0, "content_block": { "type": "thinking", "thinking": "", "signature": "" } }),
            json!({ "type": "ping" }),
            json!({ "type": "content_block_delta", "index": 0, "delta": { "type": "thinking_delta", "thinking": "The user" } }),
            json!({ "type": "content_block_delta", "index": 0, "delta": { "type": "thinking_delta", "thinking": " wants" } }),
            json!({ "type": "content_block_delta", "index": 0, "delta": { "type": "signature_delta", "signature": "msg-id-1" } }),
            json!({ "type": "content_block_stop", "index": 0 }),
            json!({ "type": "content_block_start", "index": 1, "content_block": { "type": "server_tool_use", "id": "srv_1", "name": "web_search", "input": {}, "caller": { "type": "direct" } } }),
            json!({ "type": "content_block_delta", "index": 1, "delta": { "type": "input_json_delta", "partial_json": "{\"query\": \"DeepSeek V4.1" } }),
            json!({ "type": "content_block_delta", "index": 1, "delta": { "type": "input_json_delta", "partial_json": " Flash release\"}" } }),
            json!({ "type": "content_block_stop", "index": 1 }),
            json!({ "type": "content_block_start", "index": 2, "content_block": { "type": "tool_use", "id": "call_1", "name": "note_lookup", "input": {} } }),
            json!({ "type": "content_block_delta", "index": 2, "delta": { "type": "input_json_delta", "partial_json": "{\"title\": " } }),
            json!({ "type": "content_block_delta", "index": 2, "delta": { "type": "input_json_delta", "partial_json": "\"DeepSeek\"}" } }),
            json!({ "type": "content_block_stop", "index": 2 }),
            json!({ "type": "content_block_start", "index": 3, "content_block": { "type": "web_search_tool_result", "tool_use_id": "srv_1", "content": [
                { "type": "web_search_result", "title": "36氪", "url": "https://www.36kr.com/1", "encrypted_content": "abc", "page_age": "None" },
                { "type": "web_search_result", "title": "dup", "url": "https://www.36kr.com/1", "encrypted_content": "abc" },
                { "type": "web_search_result", "title": "dated", "url": "https://a.b/2", "encrypted_content": "def", "page_age": "2026-09-10" }
            ] } }),
            json!({ "type": "content_block_stop", "index": 3 }),
            json!({ "type": "content_block_start", "index": 4, "content_block": { "type": "text", "text": "" } }),
            json!({ "type": "content_block_delta", "index": 4, "delta": { "type": "text_delta", "text": "Found it." } }),
            json!({ "type": "content_block_stop", "index": 4 }),
            json!({ "type": "message_delta", "delta": { "stop_reason": "tool_use", "stop_sequence": null }, "usage": { "input_tokens": 300, "cache_read_input_tokens": 65, "output_tokens": 174, "server_tool_use": { "web_search_requests": 1 } } }),
            json!({ "type": "message_stop" }),
        ];
        let mut parser = MessagesParser::default();
        let mut reasoning = String::new();
        let mut text = String::new();
        let mut acc = ToolCallAccumulator::default();
        let mut statuses = vec![];
        let mut queries = vec![];
        let mut results = vec![];
        let mut raw = vec![];
        let mut finish = None;
        let mut usage = None;
        for ev in &events {
            let ty = ev["type"].as_str().unwrap();
            for chunk in parser.handle(ty, ev) {
                if let Some(s) = chunk.web_search_status {
                    statuses.push(s);
                }
                if let Some(q) = chunk.web_search_queries {
                    queries.extend(q);
                }
                if let Some(r) = chunk.web_search {
                    results.extend(r);
                }
                if let Some(items) = chunk.raw_items {
                    raw.extend(items);
                }
                if let Some(u) = chunk.usage {
                    usage = Some(u);
                }
                for c in chunk.choices {
                    if let Some(t) = c.delta.reasoning_content {
                        reasoning.push_str(&t);
                    }
                    if let Some(t) = c.delta.content {
                        text.push_str(&t);
                    }
                    for tc in c.delta.tool_calls.iter().flatten() {
                        acc.push(tc);
                    }
                    if let Some(f) = c.finish_reason {
                        finish = Some(f);
                    }
                }
            }
        }
        assert_eq!(reasoning, "The user wants");
        assert_eq!(text, "Found it.");
        assert_eq!(statuses, [WebSearchStatus::Searching, WebSearchStatus::Completed]);
        assert_eq!(queries, ["DeepSeek V4.1 Flash release"]);
        assert_eq!(results.len(), 2, "deduplicated by url");
        assert_eq!((results[0].title.as_str(), results[0].publish_date.as_deref()), ("36氪", None));
        assert_eq!(results[1].publish_date.as_deref(), Some("2026-09-10"));
        let calls = acc.finish();
        assert_eq!(calls.len(), 1, "the server tool is not a function call");
        assert_eq!((calls[0].id.as_str(), calls[0].function.name.as_str(), calls[0].function.arguments.as_str()), ("call_1", "note_lookup", r#"{"title": "DeepSeek"}"#));
        assert_eq!(finish.as_deref(), Some("tool_calls"));
        let kinds: Vec<&str> = raw.iter().map(|i| i["type"].as_str().unwrap()).collect();
        assert_eq!(kinds, ["thinking", "server_tool_use", "web_search_tool_result"]);
        assert_eq!(raw[0]["signature"], "msg-id-1");
        assert!(raw[0].get("thinking").is_none(), "the stub carries no text");
        assert_eq!(raw[1]["input"], json!({ "query": "DeepSeek V4.1 Flash release" }), "the input is filled in from the deltas");
        assert_eq!(raw[2]["content"][0]["encrypted_content"], "abc");
        let u = usage.unwrap();
        assert_eq!((u.prompt_tokens, u.completion_tokens, u.cache_hit_tokens(), u.web_search_requests), (365, 174, 65, Some(1)));
    }

    /// A function call whose input never streams (an empty object) still gets its
    /// `{}` at stop; a max_tokens stop is a length finish; an error content in a
    /// search result yields no results but the block is still replayed.
    #[test]
    fn parser_edge_cases() {
        let mut parser = MessagesParser::default();
        let mut acc = ToolCallAccumulator::default();
        let mut raw = vec![];
        let mut results = 0;
        let mut finish = None;
        let events = [
            json!({ "type": "content_block_start", "index": 0, "content_block": { "type": "tool_use", "id": "c1", "name": "week", "input": {} } }),
            json!({ "type": "content_block_stop", "index": 0 }),
            json!({ "type": "content_block_start", "index": 1, "content_block": { "type": "web_search_tool_result", "tool_use_id": "s", "content": { "type": "web_search_tool_result_error", "error_code": "max_uses_exceeded" } } }),
            json!({ "type": "content_block_stop", "index": 1 }),
            json!({ "type": "message_delta", "delta": { "stop_reason": "max_tokens" }, "usage": { "output_tokens": 9 } }),
        ];
        for ev in &events {
            for chunk in parser.handle(ev["type"].as_str().unwrap(), ev) {
                if let Some(r) = chunk.web_search {
                    results += r.len();
                }
                if let Some(items) = chunk.raw_items {
                    raw.extend(items);
                }
                for c in chunk.choices {
                    for tc in c.delta.tool_calls.iter().flatten() {
                        acc.push(tc);
                    }
                    if let Some(f) = c.finish_reason {
                        finish = Some(f);
                    }
                }
            }
        }
        let calls = acc.finish();
        assert_eq!(calls[0].function.arguments, "{}");
        assert_eq!(results, 0);
        assert_eq!(raw.len(), 1);
        assert_eq!(finish.as_deref(), Some("length"));
    }
}
