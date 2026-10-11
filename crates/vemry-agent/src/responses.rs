//! The Responses API transport: every request to OpenAI's, xAI's and Meta's own
//! endpoints. OpenAI's Chat Completions calls tools only with reasoning turned off,
//! and its newest models not at all (its function-calling guide, 2026-10); xAI's is
//! a legacy endpoint; Meta's carries no reasoning across turns and no search, and
//! Meta recommends this one. The agent loop needs tools and reasoning at once, so
//! all three are spoken to in the shape they reason in — the same shape, each on a
//! line of its own (`BlockLine::Openai`, `Xai`, `Meta`), since one's encrypted
//! reasoning means nothing to another.
//!
//! Like the Messages transport, it disguises itself as Chat Completions to the
//! runner: a `ChatRequest` becomes a Responses body on the way out (system messages
//! → `instructions`, the history → `input` items, function tools → flat
//! non-strict function tools, the runner's `{"type":"web_search"}` passed through as
//! OpenAI's own search tool), and the semantic SSE events become `StreamChunk`s on
//! the way back.
//!
//! Nothing is kept on the vendor's side (`store: false`), so the conversation travels
//! whole in every request, as on the other lines, and a reasoning model's reasoning
//! comes back encrypted (`include: ["reasoning.encrypted_content"]`) for the next
//! request to carry — asked for explicitly even where a model returns it unasked,
//! since without the include xAI keeps the reasoning trace on its side. The replay
//! contract:
//! - every output item of a response (reasoning, message, function call, search) is
//!   recorded verbatim and in order in `extra_items`, and a turn that has them is
//!   replayed from them alone — output items are valid input, and a reasoning item
//!   must arrive with the item it led to;
//! - a turn from another line (`ChatMessage::blocks_line`) is composed: its text as
//!   an assistant message, its calls as function calls. Its thinking is left out:
//!   another vendor's reasoning is nothing this API can read.
//!
//! Search: the runner's `{"type":"web_search"}` is all three vendors' spelling of
//! their own search tool. OpenAI and xAI are asked for its sources with the search
//! call (`web_search_call.action.sources`; Meta documents no such include and
//! rejects what it does not know, so its sources come from the annotations), and
//! xAI's inline `[[N]](url)` citations are turned off (`no_inline_citations`): in a
//! note that spelling is a wiki link, and the sources reach the source card from the
//! call and the annotations instead.
//!
//! Thinking text: Meta streams a summary when asked (`reasoning.summary`); OpenAI's
//! summaries are not asked for (an organisation must be verified for them, and an
//! unverified one is refused the whole request); xAI streams its reasoning text
//! unasked.

use async_trait::async_trait;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::atomic::AtomicBool;

use crate::anthropic::{content_chunk, reasoning_chunk, tool_call_chunk};
use crate::client::{Auth, ChatTransport};
use crate::provider::{ProviderConfig, Scale};
use crate::types::{
    BlockLine, ChatRequest, Content, Delta, Part, StreamChoice, StreamChunk, ToolDef, Usage, WebSearchResult,
    WebSearchStatus,
};
use crate::{AgentError, Result};

pub struct ResponsesTransport;

impl ResponsesTransport {
    pub fn new() -> Self {
        ResponsesTransport
    }
}

impl Default for ResponsesTransport {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl ChatTransport for ResponsesTransport {
    async fn stream_chat(
        &self,
        config: &ProviderConfig,
        request: &ChatRequest,
        cancel: &AtomicBool,
        on_chunk: &mut (dyn FnMut(StreamChunk) -> bool + Send),
    ) -> Result<()> {
        let body = responses_request_body(request, config.block_line(), config.responses_scale());
        let resp = crate::client::post_streaming(
            crate::client::shared_client(),
            config.responses_url(),
            config,
            &body,
            Auth::Bearer,
            request.feature,
            request.priority,
        )
        .await?;

        let mut parser = ResponsesParser::default();
        crate::client::read_sse(resp, cancel, config.stream_idle_timeout(), &mut |event_name, data| {
            let Ok(data) = serde_json::from_str::<Value>(data) else {
                return Ok(true); // tolerate an unknown event shape
            };
            let ty = data
                .get("type")
                .and_then(|t| t.as_str())
                .map(String::from)
                .unwrap_or_else(|| event_name.to_string());
            if let Some(e) = stream_error(&ty, &data) {
                return Err(e);
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

/// An `error` or `response.failed` event as the error the runner classifies: a rate
/// limit or a server error carries its status (so it is retried), anything else its
/// message (an overflow is recognised by its wording).
fn stream_error(ty: &str, data: &Value) -> Option<AgentError> {
    let error = match ty {
        "error" => data.get("error").unwrap_or(data),
        "response.failed" => data.pointer("/response/error")?,
        _ => return None,
    };
    let message = error.get("message").and_then(|v| v.as_str()).unwrap_or("response failed");
    let code = error.get("code").and_then(|v| v.as_str()).unwrap_or_default();
    let status = match code {
        "rate_limit_exceeded" | "rate_limit_error" => Some(429),
        "server_error" | "server_is_overloaded" | "service_overloaded" | "backend_unavailable" => Some(503),
        _ => None,
    };
    let message = format!("{ty}: {code}: {message}");
    Some(match status {
        Some(code) => AgentError::Status { code, message },
        None => AgentError::Api(message),
    })
}

// ---- request mapping ----

/// A user message → its content items: plain text is one `input_text`; a
/// multi-modal body maps part by part, an image becoming an `input_image` with the
/// data URL the runner filled in. An `ImageRef` should never reach here, so it
/// degrades to a one-line note rather than a 400.
fn user_content(content: Option<&Content>) -> Value {
    match content {
        None => json!([{ "type": "input_text", "text": "" }]),
        Some(Content::Text(t)) => json!([{ "type": "input_text", "text": t }]),
        Some(Content::Parts(parts)) => Value::Array(
            parts
                .iter()
                .map(|p| match p {
                    Part::Text { text } => json!({ "type": "input_text", "text": text }),
                    Part::ImageUrl { image_url } => json!({ "type": "input_image", "image_url": image_url.url }),
                    Part::ImageRef { path } => {
                        json!({ "type": "input_text", "text": format!("(the image {path} was not loaded)") })
                    }
                })
                .collect(),
        ),
    }
}

/// A function tool → the flat shape, explicitly not strict (strict is this API's
/// default, and it refuses a schema with optional properties); the runner's search
/// tool (`{"type":"web_search"}`, which is OpenAI's own spelling) passes through as
/// it is; another Raw tool likewise.
fn tool_value(tool: &ToolDef) -> Value {
    match tool {
        ToolDef::Function { function, .. } => json!({
            "type": "function",
            "name": function.name,
            "description": function.description,
            "parameters": function.parameters,
            "strict": false,
        }),
        ToolDef::Raw(v) => v.clone(),
    }
}

/// `ChatRequest` → a Responses request body for one vendor's line, with the
/// model's effort scale (None: a model without reasoning, or one the tables do not
/// name). A pure function, covered by offline tests.
pub(crate) fn responses_request_body(request: &ChatRequest, line: BlockLine, scale: Option<Scale>) -> Value {
    let mut instructions: Vec<&str> = vec![];
    let mut input: Vec<Value> = vec![];
    for msg in &request.messages {
        match msg.role.as_str() {
            "system" => {
                if let Some(c) = msg.text().filter(|c| !c.is_empty()) {
                    instructions.push(c);
                }
            }
            "user" => input.push(json!({ "type": "message", "role": "user", "content": user_content(msg.content.as_ref()) })),
            "assistant" => {
                if msg.blocks_line == line && !msg.extra_items.is_empty() {
                    input.extend(msg.extra_items.iter().cloned());
                    continue;
                }
                if let Some(text) = msg.text().filter(|t| !t.is_empty()) {
                    input.push(json!({ "type": "message", "role": "assistant", "content": [{ "type": "output_text", "text": text }] }));
                }
                for call in msg.tool_calls.iter().flatten() {
                    input.push(json!({
                        "type": "function_call",
                        "call_id": call.id,
                        "name": call.function.name,
                        "arguments": call.function.arguments,
                    }));
                }
            }
            "tool" => input.push(json!({
                "type": "function_call_output",
                "call_id": msg.tool_call_id.clone().unwrap_or_default(),
                "output": msg.text().unwrap_or_default(),
            })),
            _ => {}
        }
    }
    let mut body = json!({
        "model": request.model,
        "input": input,
        "stream": true,
        "store": false,
    });
    if !instructions.is_empty() {
        body["instructions"] = json!(instructions.join("\n\n"));
    }
    // A reasoning model: its reasoning comes back encrypted for the next request to
    // carry, and the level, when one is set, in the model's own words. A model
    // without reasoning is sent neither (it refuses both).
    let mut include: Vec<&str> = vec![];
    if let Some(scale) = scale {
        include.push("reasoning.encrypted_content");
        let mut reasoning = serde_json::Map::new();
        if let Some(level) = request.thinking_level() {
            reasoning.insert("effort".into(), json!(scale.word(level)));
        }
        if line == BlockLine::Meta {
            reasoning.insert("summary".into(), json!("auto"));
        }
        if !reasoning.is_empty() {
            body["reasoning"] = Value::Object(reasoning);
        }
    }
    let tools = request.tools.as_deref().unwrap_or_default();
    if tools.iter().any(is_search_tool) {
        match line {
            BlockLine::Openai => include.push("web_search_call.action.sources"),
            BlockLine::Xai => include.extend(["web_search_call.action.sources", "no_inline_citations"]),
            _ => {}
        }
    }
    if !include.is_empty() {
        body["include"] = json!(include);
    }
    if !tools.is_empty() {
        body["tools"] = Value::Array(tools.iter().map(tool_value).collect());
    }
    body
}

/// The runner's provider-neutral search tool.
fn is_search_tool(tool: &ToolDef) -> bool {
    matches!(tool, ToolDef::Raw(v) if v.get("type").and_then(|t| t.as_str()) == Some("web_search"))
}

// ---- streaming ----

/// Semantic SSE events → a `StreamChunk` synthesiser. A pure state machine, tested
/// offline against a fixture stream.
#[derive(Default)]
pub struct ResponsesParser {
    /// item id → the tool-call slot (the runner's accumulator index).
    call_slots: HashMap<String, usize>,
    /// Whether each slot has had its arguments (if not, they go out whole at done).
    slot_got_args: Vec<bool>,
    /// Citation sources (deduplicated by url), sent at the end.
    citations: Vec<WebSearchResult>,
    searching_emitted: bool,
    refused: bool,
}

impl ResponsesParser {
    pub fn handle(&mut self, ty: &str, data: &Value) -> Vec<StreamChunk> {
        let delta = || data.get("delta").and_then(|v| v.as_str()).filter(|d| !d.is_empty()).map(String::from);
        match ty {
            "response.output_text.delta" => delta().map(content_chunk).into_iter().collect(),
            "response.reasoning_text.delta" | "response.reasoning_summary_text.delta" => {
                delta().map(reasoning_chunk).into_iter().collect()
            }
            "response.refusal.delta" => {
                self.refused = true;
                vec![]
            }
            "response.output_item.added" => self.on_item_added(data),
            "response.function_call_arguments.delta" => self.on_args(data, "delta", false),
            "response.function_call_arguments.done" => self.on_args(data, "arguments", true),
            "response.output_item.done" => self.on_item_done(data),
            "response.web_search_call.in_progress" | "response.web_search_call.searching" => self.searching(),
            "response.web_search_call.completed" => {
                vec![StreamChunk { web_search_status: Some(WebSearchStatus::Completed), ..Default::default() }]
            }
            "response.output_text.annotation.added" => {
                if let Some(a) = data.get("annotation") {
                    self.collect_citation(a);
                }
                vec![]
            }
            "response.completed" | "response.incomplete" => self.on_finished(data),
            _ => vec![],
        }
    }

    /// A search starting (the item is the signal on a stream that sends no search
    /// events of its own), or a function call opening its slot.
    fn searching(&mut self) -> Vec<StreamChunk> {
        if std::mem::replace(&mut self.searching_emitted, true) {
            vec![]
        } else {
            vec![StreamChunk { web_search_status: Some(WebSearchStatus::Searching), ..Default::default() }]
        }
    }

    fn on_item_added(&mut self, data: &Value) -> Vec<StreamChunk> {
        let kind = data.pointer("/item/type").and_then(|t| t.as_str());
        if kind == Some("web_search_call") {
            return self.searching();
        }
        let Some(item) = data.get("item").filter(|_| kind == Some("function_call")) else {
            return vec![];
        };
        let slot = self.slot_got_args.len();
        self.slot_got_args.push(false);
        if let Some(id) = item.get("id").and_then(|v| v.as_str()) {
            self.call_slots.insert(id.to_string(), slot);
        }
        let call_id = item.get("call_id").and_then(|v| v.as_str()).unwrap_or_default();
        let name = item.get("name").and_then(|v| v.as_str()).unwrap_or_default();
        vec![tool_call_chunk(slot, Some(call_id), Some(name), None)]
    }

    /// An arguments delta, or the whole arguments at done — the latter only when no
    /// delta ever came, so nothing is sent twice.
    fn on_args(&mut self, data: &Value, field: &str, whole: bool) -> Vec<StreamChunk> {
        let (Some(item_id), Some(args)) = (data.get("item_id").and_then(|v| v.as_str()), data.get(field).and_then(|v| v.as_str())) else {
            return vec![];
        };
        let Some(&slot) = self.call_slots.get(item_id) else { return vec![] };
        if whole && self.slot_got_args[slot] {
            return vec![];
        }
        self.slot_got_args[slot] = true;
        vec![tool_call_chunk(slot, None, None, Some(args))]
    }

    /// Every finished output item is recorded verbatim for the replay (see the
    /// module comment). A function call whose `added` never came is registered here
    /// with its whole arguments; a search reports its queries and any sources.
    fn on_item_done(&mut self, data: &Value) -> Vec<StreamChunk> {
        let Some(item) = data.get("item") else { return vec![] };
        let mut chunks = vec![];
        match item.get("type").and_then(|t| t.as_str()) {
            Some("function_call") => {
                let item_id = item.get("id").and_then(|v| v.as_str()).unwrap_or_default();
                let args = item.get("arguments").and_then(|v| v.as_str()).filter(|a| !a.is_empty());
                match self.call_slots.get(item_id).copied() {
                    Some(slot) if !self.slot_got_args[slot] => {
                        self.slot_got_args[slot] = true;
                        chunks.push(tool_call_chunk(slot, None, None, Some(args.unwrap_or("{}"))));
                    }
                    Some(_) => {}
                    None => {
                        let slot = self.slot_got_args.len();
                        self.slot_got_args.push(true);
                        self.call_slots.insert(item_id.to_string(), slot);
                        let call_id = item.get("call_id").and_then(|v| v.as_str()).unwrap_or_default();
                        let name = item.get("name").and_then(|v| v.as_str()).unwrap_or_default();
                        chunks.push(tool_call_chunk(slot, Some(call_id), Some(name), Some(args.unwrap_or("{}"))));
                    }
                }
            }
            Some("web_search_call") => {
                let action = item.get("action");
                let mut queries: Vec<String> = vec![];
                if let Some(q) = action.and_then(|a| a.get("query")).and_then(|v| v.as_str()) {
                    queries.push(q.to_string());
                }
                for q in action.and_then(|a| a.get("queries")).and_then(|v| v.as_array()).into_iter().flatten() {
                    if let Some(q) = q.as_str().filter(|q| !queries.iter().any(|known| known == q)) {
                        queries.push(q.to_string());
                    }
                }
                for source in action.and_then(|a| a.get("sources")).and_then(|v| v.as_array()).into_iter().flatten() {
                    self.collect_citation(source);
                }
                if !queries.is_empty() {
                    chunks.push(StreamChunk { web_search_queries: Some(queries), ..Default::default() });
                }
            }
            _ => {}
        }
        chunks.push(StreamChunk { raw_items: Some(vec![item.clone()]), ..Default::default() });
        chunks
    }

    /// The end of the response: the sources, the usage and the finish — a refusal,
    /// a cut at the output ceiling (`incomplete` for `max_output_tokens`), a content
    /// filter's cut (a refusal too), the tool calls, or a plain stop.
    fn on_finished(&mut self, data: &Value) -> Vec<StreamChunk> {
        let mut chunks = vec![];
        if !self.citations.is_empty() {
            chunks.push(StreamChunk { web_search: Some(std::mem::take(&mut self.citations)), ..Default::default() });
        }
        let usage = data.pointer("/response/usage").and_then(|v| serde_json::from_value::<Usage>(v.clone()).ok());
        let incomplete = data.pointer("/response/incomplete_details/reason").and_then(|v| v.as_str());
        let finish = if self.refused || incomplete == Some("content_filter") {
            "refusal"
        } else if incomplete == Some("max_output_tokens") {
            "length"
        } else if !self.slot_got_args.is_empty() {
            "tool_calls"
        } else {
            "stop"
        };
        chunks.push(StreamChunk {
            choices: vec![StreamChoice { delta: Delta::default(), finish_reason: Some(finish.into()) }],
            usage,
            ..Default::default()
        });
        chunks
    }

    /// A `url_citation` annotation, or a source of a search: its url and title.
    fn collect_citation(&mut self, source: &Value) {
        let Some(url) = source.get("url").and_then(|v| v.as_str()).filter(|u| !u.is_empty()) else { return };
        if self.citations.iter().any(|c| c.url == url) {
            return;
        }
        self.citations.push(WebSearchResult {
            title: source.get("title").and_then(|v| v.as_str()).unwrap_or_default().to_string(),
            url: url.to_string(),
            snippet: String::new(),
            publish_date: None,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{ChatMessage, Feature, FunctionCall, FunctionSpec, ToolCall, ToolCallAccumulator};

    fn openai(model: &str) -> ProviderConfig {
        ProviderConfig::custom("https://api.openai.com/v1", model, "")
    }

    fn request(model: &str, messages: Vec<ChatMessage>) -> ChatRequest {
        ChatRequest {
            feature: Default::default(),
            priority: Feature::default().priority(),
            model: model.into(),
            messages,
            tools: None,
            stream: true,
            stream_options: None,
            reasoning_effort: None,
            thinking: None,
        }
    }

    /// The tool loop in one body: system → instructions; a turn from another line
    /// composed (text, then its call), its thinking left out; a turn recorded here
    /// replayed verbatim from its items; tool results as function_call_output;
    /// function tools flat and not strict, the search tool as it is; nothing stored.
    #[test]
    fn request_maps_the_loop_and_replays_its_own_items() {
        let mut elsewhere = ChatMessage::assistant(
            Some("Reading.".into()),
            Some(vec![ToolCall { id: "call_9".into(), kind: "function".into(), function: FunctionCall { name: "read_note".into(), arguments: r#"{"path":"A.md"}"#.into() } }]),
        );
        elsewhere.reasoning_content = Some("deepseek thoughts".into());
        elsewhere.extra_items = vec![json!({ "type": "thinking", "thinking": "x", "signature": "s" })];
        let mut here = ChatMessage::assistant(None, Some(vec![ToolCall { id: "call_1".into(), kind: "function".into(), function: FunctionCall { name: "week".into(), arguments: "{}".into() } }]));
        here.blocks_line = BlockLine::Openai;
        here.extra_items = vec![
            json!({ "type": "reasoning", "id": "rs_1", "summary": [], "encrypted_content": "enc" }),
            json!({ "type": "function_call", "id": "fc_1", "call_id": "call_1", "name": "week", "arguments": "{}" }),
        ];
        let req = ChatRequest {
            reasoning_effort: Some("max".into()),
            tools: Some(vec![
                ToolDef::function(FunctionSpec { name: "read_note".into(), description: "d".into(), parameters: json!({ "type": "object" }) }),
                ToolDef::Raw(json!({ "type": "web_search" })),
            ]),
            ..request("gpt-6-astra", vec![
                ChatMessage::system("Be terse."),
                ChatMessage::user("hi"),
                elsewhere,
                ChatMessage::tool_result("call_9", "# A"),
                here,
                ChatMessage::tool_result("call_1", "Mon"),
            ])
        };
        let body = responses_request_body(&req, BlockLine::Openai, openai("gpt-6-astra").responses_scale());
        assert_eq!(body["instructions"], "Be terse.");
        assert_eq!(body["store"], false);
        assert_eq!(body["reasoning"], json!({ "effort": "max" }));
        let input = body["input"].as_array().unwrap();
        let kinds: Vec<&str> = input.iter().map(|i| i["type"].as_str().unwrap()).collect();
        assert_eq!(kinds, ["message", "message", "function_call", "function_call_output", "reasoning", "function_call", "function_call_output"]);
        assert_eq!(input[0]["content"], json!([{ "type": "input_text", "text": "hi" }]));
        assert_eq!(input[1]["content"], json!([{ "type": "output_text", "text": "Reading." }]));
        assert_eq!(input[2], json!({ "type": "function_call", "call_id": "call_9", "name": "read_note", "arguments": r#"{"path":"A.md"}"# }));
        assert_eq!(input[4]["encrypted_content"], "enc", "the reasoning travels back verbatim");
        assert_eq!(input[6], json!({ "type": "function_call_output", "call_id": "call_1", "output": "Mon" }));
        let tools = body["tools"].as_array().unwrap();
        assert_eq!(tools[0], json!({ "type": "function", "name": "read_note", "description": "d", "parameters": { "type": "object" }, "strict": false }));
        assert_eq!(tools[1], json!({ "type": "web_search" }));
        assert_eq!(body["include"], json!(["reasoning.encrypted_content", "web_search_call.action.sources"]));
        // On xAI's line the inline [[N]](url) citations are turned off as well
        let xai = ProviderConfig::custom("https://api.x.ai/v1", "grok-4.7", "");
        let body = responses_request_body(&ChatRequest { model: "grok-4.7".into(), ..req.clone() }, xai.block_line(), xai.responses_scale());
        assert_eq!(body["include"], json!(["reasoning.encrypted_content", "web_search_call.action.sources", "no_inline_citations"]));
        // Meta documents no sources include: only the encrypted reasoning
        let meta = ProviderConfig::custom("https://api.meta.ai/v1", "muse-spark-1.3", "");
        let body = responses_request_body(&ChatRequest { model: "muse-spark-1.3".into(), ..req.clone() }, meta.block_line(), meta.responses_scale());
        assert_eq!(body["include"], json!(["reasoning.encrypted_content"]));
        assert_eq!(body["tools"][1], json!({ "type": "web_search" }));
    }

    /// The level in each model's words; a model the tables do not name is sent no
    /// reasoning and no encrypted-content include; no level, no reasoning field. A
    /// turn recorded on OpenAI's line is not replayed to xAI's, and the other way.
    #[test]
    fn effort_is_spelled_per_model_and_items_stay_on_their_line() {
        let at = |base: &str, model: &str, effort: Option<&str>, thinking: Option<Value>| {
            let cfg = ProviderConfig::custom(base, model, "");
            let req = ChatRequest { reasoning_effort: effort.map(String::from), thinking, ..request(model, vec![ChatMessage::user("x")]) };
            responses_request_body(&req, cfg.block_line(), cfg.responses_scale())
        };
        let (gpt, grok) = ("https://api.openai.com/v1", "https://api.x.ai/v1");
        let off = Some(json!({ "type": "disabled" }));
        assert_eq!(at(gpt, "gpt-6-sol", None, off.clone())["reasoning"]["effort"], "none");
        assert_eq!(at(gpt, "gpt-6-astra", None, off.clone())["reasoning"]["effort"], "low", "astra refuses none");
        assert_eq!(at(gpt, "gpt-6.1-sol", Some("max"), None)["reasoning"]["effort"], "max");
        assert_eq!(at(grok, "grok-4.7", Some("max"), None)["reasoning"]["effort"], "xhigh");
        assert_eq!(at(grok, "grok-4.3", None, off.clone())["reasoning"]["effort"], "none");
        let earlier = at(gpt, "gpt-5.6-sol", Some("high"), None);
        assert!(earlier.get("reasoning").is_none() && earlier.get("include").is_none());
        let unset = at(gpt, "gpt-6-luna", None, None);
        assert!(unset.get("reasoning").is_none() && unset.get("include").is_some());
        // Meta: "none" is its lowest effort, and the summary is asked for even with no level set
        let meta = "https://api.meta.ai/v1";
        assert_eq!(at(meta, "muse-spark-1.3", None, off.clone())["reasoning"], json!({ "effort": "minimal", "summary": "auto" }));
        assert_eq!(at(meta, "muse-spark-1.3", None, None)["reasoning"], json!({ "summary": "auto" }));

        let mut answered = ChatMessage::assistant(Some("ok".into()), None);
        answered.blocks_line = BlockLine::Openai;
        answered.extra_items = vec![json!({ "type": "reasoning", "id": "rs_1", "encrypted_content": "enc" }), json!({ "type": "message", "id": "msg_1", "role": "assistant", "content": [{ "type": "output_text", "text": "ok" }] })];
        let req = request("grok-4.7", vec![ChatMessage::user("x"), answered, ChatMessage::user("y")]);
        let xai = ProviderConfig::custom(grok, "grok-4.7", "");
        let body = responses_request_body(&req, xai.block_line(), xai.responses_scale());
        assert_eq!(body["input"][1], json!({ "type": "message", "role": "assistant", "content": [{ "type": "output_text", "text": "ok" }] }), "composed, the foreign reasoning left out");
        assert_eq!(body["input"].as_array().unwrap().len(), 3);
    }

    /// A captured shape of a reasoning model's stream: a reasoning item, a function
    /// call whose arguments stream, a message, the completion with its usage. Every
    /// item is recorded in order, the call is assembled, the finish is tool_calls.
    #[test]
    fn parser_walks_a_tool_call_stream() {
        let events = vec![
            json!({ "type": "response.output_item.added", "item": { "type": "reasoning", "id": "rs_1" } }),
            json!({ "type": "response.output_item.done", "item": { "type": "reasoning", "id": "rs_1", "summary": [], "encrypted_content": "enc" } }),
            json!({ "type": "response.output_item.added", "item": { "type": "message", "id": "msg_1", "role": "assistant", "content": [] } }),
            json!({ "type": "response.output_text.delta", "item_id": "msg_1", "delta": "Checking." }),
            json!({ "type": "response.output_item.done", "item": { "type": "message", "id": "msg_1", "role": "assistant", "status": "completed", "content": [{ "type": "output_text", "text": "Checking.", "annotations": [] }] } }),
            json!({ "type": "response.output_item.added", "item": { "type": "function_call", "id": "fc_1", "call_id": "call_1", "name": "read_note", "arguments": "" } }),
            json!({ "type": "response.function_call_arguments.delta", "item_id": "fc_1", "delta": "{\"path\":" }),
            json!({ "type": "response.function_call_arguments.delta", "item_id": "fc_1", "delta": "\"A.md\"}" }),
            json!({ "type": "response.function_call_arguments.done", "item_id": "fc_1", "arguments": "{\"path\":\"A.md\"}" }),
            json!({ "type": "response.output_item.done", "item": { "type": "function_call", "id": "fc_1", "call_id": "call_1", "name": "read_note", "arguments": "{\"path\":\"A.md\"}", "status": "completed" } }),
            json!({ "type": "response.completed", "response": { "usage": { "input_tokens": 900, "input_tokens_details": { "cached_tokens": 512 }, "output_tokens": 40 } } }),
        ];
        let mut parser = ResponsesParser::default();
        let (mut text, mut raw, mut acc, mut finish, mut usage) = (String::new(), vec![], ToolCallAccumulator::default(), None, None);
        for ev in &events {
            for chunk in parser.handle(ev["type"].as_str().unwrap(), ev) {
                raw.extend(chunk.raw_items.unwrap_or_default());
                usage = chunk.usage.or(usage);
                for c in chunk.choices {
                    if let Some(t) = c.delta.content {
                        text.push_str(&t);
                    }
                    for tc in c.delta.tool_calls.iter().flatten() {
                        acc.push(tc);
                    }
                    finish = c.finish_reason.or(finish);
                }
            }
        }
        assert_eq!(text, "Checking.");
        let kinds: Vec<&str> = raw.iter().map(|i| i["type"].as_str().unwrap()).collect();
        assert_eq!(kinds, ["reasoning", "message", "function_call"]);
        let calls = acc.finish();
        assert_eq!(calls.len(), 1);
        assert_eq!((calls[0].id.as_str(), calls[0].function.name.as_str(), calls[0].function.arguments.as_str()), ("call_1", "read_note", r#"{"path":"A.md"}"#));
        assert_eq!(finish.as_deref(), Some("tool_calls"));
        let u = usage.unwrap();
        assert_eq!((u.prompt_tokens, u.completion_tokens, u.cache_hit_tokens()), (900, 40, 512));
    }

    /// A search's query, status and citations; a cut at the output ceiling is a
    /// length finish, a refusal a refusal; a failure with a rate-limit code is
    /// retried as a 429.
    #[test]
    fn parser_search_incomplete_refusal_and_errors() {
        let mut parser = ResponsesParser::default();
        let mut statuses = vec![];
        let mut queries = vec![];
        let mut results = vec![];
        let mut finish = None;
        for ev in [
            json!({ "type": "response.output_item.added", "item": { "type": "web_search_call", "id": "ws_1", "status": "in_progress" } }),
            json!({ "type": "response.web_search_call.in_progress", "item_id": "ws_1" }),
            json!({ "type": "response.web_search_call.searching", "item_id": "ws_1" }),
            json!({ "type": "response.output_item.done", "item": { "type": "web_search_call", "id": "ws_1", "status": "completed", "action": { "type": "search", "query": "pears", "sources": [{ "type": "url", "url": "https://a.b/p", "title": "Pears, a guide" }] } } }),
            json!({ "type": "response.web_search_call.completed", "item_id": "ws_1" }),
            json!({ "type": "response.output_text.annotation.added", "annotation": { "type": "url_citation", "url": "https://a.b/p", "title": "Pears" } }),
            json!({ "type": "response.output_text.annotation.added", "annotation": { "type": "url_citation", "url": "https://a.b/p", "title": "dup" } }),
            json!({ "type": "response.incomplete", "response": { "incomplete_details": { "reason": "max_output_tokens" } } }),
        ] {
            for chunk in parser.handle(ev["type"].as_str().unwrap(), &ev) {
                statuses.extend(chunk.web_search_status);
                queries.extend(chunk.web_search_queries.unwrap_or_default());
                results.extend(chunk.web_search.unwrap_or_default());
                for c in chunk.choices {
                    finish = c.finish_reason.or(finish);
                }
            }
        }
        assert_eq!(statuses, [WebSearchStatus::Searching, WebSearchStatus::Completed]);
        assert_eq!(queries, ["pears"]);
        assert_eq!(results.len(), 1, "the annotations repeat the source");
        assert_eq!(results[0].title, "Pears, a guide", "the source's title, not an annotation's label");
        assert_eq!(finish.as_deref(), Some("length"));

        let mut parser = ResponsesParser::default();
        parser.handle("response.refusal.delta", &json!({ "delta": "I can't" }));
        let done = parser.handle("response.completed", &json!({ "response": {} }));
        assert_eq!(done[0].choices[0].finish_reason.as_deref(), Some("refusal"));

        let failed = json!({ "type": "response.failed", "response": { "error": { "code": "rate_limit_exceeded", "message": "slow down" } } });
        assert!(matches!(stream_error("response.failed", &failed), Some(AgentError::Status { code: 429, .. })));
        let overloaded = json!({ "type": "error", "code": "service_overloaded", "message": "try again" });
        assert!(matches!(stream_error("error", &overloaded), Some(AgentError::Status { code: 503, .. })), "Meta's overload is retried");
        let meta_overflow = json!({ "type": "error", "code": null, "message": "This request has 1100000 tokens, but the model's context length is only 1048576 tokens." });
        assert!(crate::compact::is_context_overflow(&stream_error("error", &meta_overflow).unwrap().to_string()));
        let bad = json!({ "type": "error", "code": "context_length_exceeded", "message": "Your input exceeds the context window of this model." });
        let e = stream_error("error", &bad).unwrap();
        assert!(crate::compact::is_context_overflow(&e.to_string()), "{e}");
        assert!(stream_error("response.completed", &json!({})).is_none());
    }
}
