//! The DeepSeek Responses API transport layer (the v0.4 carrier for web_search).
//!
//! It disguises itself as Chat Completions to the runner: on the request side it
//! maps a `ChatRequest` into a Responses body (messages→input items,
//! system→instructions, tools passed straight through), and on the response side
//! it synthesises `StreamChunk`s out of the semantic SSE events
//! (response.output_text.delta / …). The runner, MockTransport and the event
//! stream need no change at all — the `ChatTransport` trait is the seam.
//!
//! Two key conventions of the mapping and the replay:
//! - the multi-turn tool loop: an assistant's tool_calls → `function_call`
//!   items, a role:"tool" result → `function_call_output` items;
//! - a `web_search_call` output item is replayed verbatim through
//!   `ChatMessage.extra_items`, as the official documentation requires ("passed
//!   back as-is", after which the server restores the search results);
//! - a `reasoning` output item is replayed through the same channel — in
//!   thinking mode the server insists on it (a 400 when it is missing: "The
//!   `reasoning_text` … must be passed back").
//!
//! [to verify · live] The exact fields of a citation or a search result are not
//! documented — the parser extracts tolerantly from both the url_citation
//! annotation and the results embedded in a web_search_call, and live testing
//! has the final word.

use async_trait::async_trait;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::atomic::AtomicBool;

use crate::client::ChatTransport;
use crate::provider::ProviderConfig;
use crate::types::{
    Content, Part,
    ChatRequest, Delta, FunctionDelta, StreamChoice, StreamChunk, ToolCallDelta, Usage,
    WebSearchResult, WebSearchStatus,
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
        let body = responses_request_body(request);
        let resp = crate::client::post_streaming(
            crate::client::shared_client(),
            config.responses_url(),
            config,
            &body,
        )
        .await?;

        let mut parser = ResponsesParser::default();
        crate::client::read_sse(resp, cancel, &mut |event_name, data| {
            let Ok(data) = serde_json::from_str::<Value>(data) else {
                return Ok(true); // tolerate an unknown event shape
            };
            // The event type: data.type first (every Responses event carries one),
            // with the SSE event name as a fallback
            let ty = data
                .get("type")
                .and_then(|t| t.as_str())
                .map(String::from)
                .unwrap_or_else(|| event_name.to_string());
            if ty == "response.failed" || ty == "response.incomplete" {
                let msg = data
                    .pointer("/response/error/message")
                    .or_else(|| data.pointer("/response/incomplete_details/reason"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("response failed");
                return Err(AgentError::Api(format!("{ty}: {msg}")));
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

/// A user message → the Responses content item array: plain text becomes a single
/// `input_text`; multi-modal parts are mapped one by one, an image being an
/// `input_image` plus a data URL string (the OpenAI Responses shape). An
/// `ImageRef` should never reach here (the runner fills it in before assembling
/// the request), so it defensively degrades to a one-line note rather than
/// earning a 400 from the server.
fn user_input_content(content: Option<&Content>) -> Value {
    match content {
        None => json!([{ "type": "input_text", "text": "" }]),
        Some(Content::Text(t)) => json!([{ "type": "input_text", "text": t }]),
        Some(Content::Parts(parts)) => Value::Array(
            parts
                .iter()
                .map(|p| match p {
                    Part::Text { text } => json!({ "type": "input_text", "text": text }),
                    Part::ImageUrl { image_url } => json!({ "type": "input_image", "image_url": image_url.url }),
                    Part::ImageRef { path } => json!({ "type": "input_text", "text": format!("(the image {path} was not loaded)") }),
                })
                .collect(),
        ),
    }
}

/// `ChatRequest` → a Responses request body. A pure function, covered by offline
/// tests.
pub fn responses_request_body(request: &ChatRequest) -> Value {
    let mut instructions: Vec<String> = vec![];
    let mut input: Vec<Value> = vec![];
    for msg in &request.messages {
        match msg.role.as_str() {
            "system" => {
                if let Some(c) = msg.text() {
                    instructions.push(c.to_string());
                }
            }
            "user" => input.push(json!({
                "type": "message",
                "role": "user",
                "content": user_input_content(msg.content.as_ref()),
            })),
            "assistant" => {
                // Opaque output items such as web_search_call are replayed first
                // (the server restores the search results from them)
                for item in &msg.extra_items {
                    input.push(item.clone());
                }
                if let Some(c) = msg.text() {
                    if !c.is_empty() {
                        input.push(json!({
                            "type": "message",
                            "role": "assistant",
                            "content": [{ "type": "output_text", "text": c }]
                        }));
                    }
                }
                if let Some(calls) = &msg.tool_calls {
                    for call in calls {
                        input.push(json!({
                            "type": "function_call",
                            "call_id": call.id,
                            "name": call.function.name,
                            "arguments": call.function.arguments,
                        }));
                    }
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
    });
    // Thinking level: the Responses shape is reasoning:{effort}, and the "none"
    // level is one of them — the Chat Completions line's
    // thinking:{"type":"disabled"} is restored to effort:"none" here
    if request.thinking.as_ref().and_then(|t| t.get("type")).and_then(|v| v.as_str()) == Some("disabled") {
        body["reasoning"] = json!({ "effort": "none" });
    } else if let Some(e) = &request.reasoning_effort {
        body["reasoning"] = json!({ "effort": e });
    }
    if !instructions.is_empty() {
        body["instructions"] = json!(instructions.join("\n\n"));
    }
    if let Some(tools) = &request.tools {
        if !tools.is_empty() {
            // The Responses API's function tools take the flat shape (verified
            // live: a nested {"function":{…}} is a 400 "missing field `name`");
            // Raw passes straight through
            let mapped: Vec<Value> = tools
                .iter()
                .map(|t| match t.as_function() {
                    Some(f) => json!({
                        "type": "function",
                        "name": f.name,
                        "description": f.description,
                        "parameters": f.parameters,
                    }),
                    None => match t {
                        crate::types::ToolDef::Raw(v) => v.clone(),
                        crate::types::ToolDef::Function { .. } => unreachable!(),
                    },
                })
                .collect();
            body["tools"] = Value::Array(mapped);
        }
    }
    body
}

/// Semantic SSE events → a `StreamChunk` synthesiser. A pure state machine,
/// tested offline against a fixture stream.
#[derive(Default)]
pub struct ResponsesParser {
    /// item_id → the tool call slot (ToolCallAccumulator's index semantics).
    call_slots: HashMap<String, usize>,
    /// call_id → the slot (used to align when added carries no item id, or done
    /// carries a different one, so nothing is registered twice).
    call_id_slots: HashMap<String, usize>,
    /// Whether each slot ever received an arguments delta (if not, the whole thing
    /// is emitted at done).
    slot_got_args: Vec<bool>,
    /// Accumulated citation sources (deduplicated by url), emitted in one go at
    /// response.completed.
    citations: Vec<WebSearchResult>,
    searching_emitted: bool,
}

impl ResponsesParser {
    pub fn handle(&mut self, ty: &str, data: &Value) -> Vec<StreamChunk> {
        match ty {
            "response.output_text.delta" => text_delta(data).map(content_chunk).into_iter().collect(),
            "response.reasoning_text.delta" => {
                text_delta(data).map(reasoning_chunk).into_iter().collect()
            }
            "response.output_item.added" => self.on_item_added(data),
            "response.function_call_arguments.delta" => self.on_args_delta(data),
            "response.function_call_arguments.done" => self.on_args_done(data),
            "response.output_item.done" => self.on_item_done(data),
            "response.web_search_call.in_progress" | "response.web_search_call.searching" => {
                if self.searching_emitted {
                    vec![]
                } else {
                    self.searching_emitted = true;
                    vec![StreamChunk {
                        web_search_status: Some(WebSearchStatus::Searching),
                        ..Default::default()
                    }]
                }
            }
            "response.web_search_call.completed" => vec![StreamChunk {
                web_search_status: Some(WebSearchStatus::Completed),
                ..Default::default()
            }],
            "response.output_text.annotation.added" => {
                if let Some(annotation) = data.get("annotation") {
                    self.collect_citation(annotation);
                }
                vec![]
            }
            "response.completed" => self.on_completed(data),
            _ => vec![],
        }
    }

    fn on_item_added(&mut self, data: &Value) -> Vec<StreamChunk> {
        let Some(item) = data.get("item") else { return vec![] };
        if item.get("type").and_then(|t| t.as_str()) == Some("function_call") {
            let slot = self.slot_got_args.len();
            if let Some(id) = item.get("id").and_then(|v| v.as_str()) {
                self.call_slots.insert(id.to_string(), slot);
            }
            self.slot_got_args.push(false);
            let call_id = item.get("call_id").and_then(|v| v.as_str()).unwrap_or_default();
            if !call_id.is_empty() {
                self.call_id_slots.insert(call_id.to_string(), slot);
            }
            let name = item.get("name").and_then(|v| v.as_str()).unwrap_or_default();
            return vec![tool_call_chunk(slot, Some(call_id), Some(name), None)];
        }
        vec![]
    }

    fn on_args_delta(&mut self, data: &Value) -> Vec<StreamChunk> {
        let (Some(item_id), Some(delta)) = (
            data.get("item_id").and_then(|v| v.as_str()),
            data.get("delta").and_then(|v| v.as_str()),
        ) else {
            return vec![];
        };
        let Some(&slot) = self.call_slots.get(item_id) else { return vec![] };
        self.slot_got_args[slot] = true;
        vec![tool_call_chunk(slot, None, None, Some(delta))]
    }

    /// The fallback for a provider that only sends done and never a delta: emit the
    /// whole arguments at once.
    fn on_args_done(&mut self, data: &Value) -> Vec<StreamChunk> {
        let (Some(item_id), Some(args)) = (
            data.get("item_id").and_then(|v| v.as_str()),
            data.get("arguments").and_then(|v| v.as_str()),
        ) else {
            return vec![];
        };
        let Some(&slot) = self.call_slots.get(item_id) else { return vec![] };
        if self.slot_got_args[slot] {
            return vec![];
        }
        self.slot_got_args[slot] = true;
        vec![tool_call_chunk(slot, None, None, Some(args))]
    }

    fn on_item_done(&mut self, data: &Value) -> Vec<StreamChunk> {
        let Some(item) = data.get("item") else { return vec![] };
        match item.get("type").and_then(|t| t.as_str()) {
            Some("web_search_call") => {
                // Keep the whole output item verbatim for the next turn's replay;
                // any embedded results are extracted tolerantly as citations
                // (measured live, DeepSeek gives only action.queries and no
                // results — the queries are reported separately, and the
                // extraction is kept in case a later version fills the field in)
                self.collect_results_from_item(item);
                let queries: Vec<String> = item
                    .pointer("/action/queries")
                    .and_then(|v| v.as_array())
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|q| q.as_str())
                            // A correlation-id pseudo query the server slips in; not a query a user can read
                            .filter(|q| !q.starts_with("ws_call_id="))
                            .map(String::from)
                            .collect()
                    })
                    .unwrap_or_default();
                vec![StreamChunk {
                    raw_items: Some(vec![item.clone()]),
                    web_search_queries: if queries.is_empty() { None } else { Some(queries) },
                    ..Default::default()
                }]
            }
            // The body is consumed as output_text deltas, and the rebuilt assistant
            // message carries it on replay — replaying the original item too would
            // duplicate it; a malformed item with no type is not passed back either
            Some("message") | None => vec![],
            Some("function_call") => {
                let item_id = item.get("id").and_then(|v| v.as_str()).unwrap_or_default();
                let call_id = item.get("call_id").and_then(|v| v.as_str()).unwrap_or_default();
                // Align by item_id first; when added has no id, or the ids differ
                // between events, align by call_id so one logical call is not
                // fallback-registered into a second slot
                let known_slot = self.call_slots.get(item_id).copied().or_else(|| {
                    if call_id.is_empty() {
                        None
                    } else {
                        self.call_id_slots.get(call_id).copied()
                    }
                });
                if let Some(slot) = known_slot {
                    if !item_id.is_empty() {
                        self.call_slots.entry(item_id.to_string()).or_insert(slot);
                    }
                    // No arguments event ever arrived: emit the complete arguments the done item carries
                    if !self.slot_got_args[slot] {
                        if let Some(args) = item
                            .get("arguments")
                            .and_then(|v| v.as_str())
                            .filter(|a| !a.is_empty())
                        {
                            self.slot_got_args[slot] = true;
                            return vec![tool_call_chunk(slot, None, None, Some(args))];
                        }
                    }
                    return vec![];
                }
                // Fallback registration when the added event never arrived, plus the whole arguments
                let slot = self.slot_got_args.len();
                self.call_slots.insert(item_id.to_string(), slot);
                if !call_id.is_empty() {
                    self.call_id_slots.insert(call_id.to_string(), slot);
                }
                self.slot_got_args.push(false);
                let name = item.get("name").and_then(|v| v.as_str()).unwrap_or_default();
                let args = item.get("arguments").and_then(|v| v.as_str());
                self.slot_got_args[slot] = true;
                vec![tool_call_chunk(slot, Some(call_id), Some(name), args)]
            }
            // Every other output item (reasoning, future server-side tool items) is
            // kept verbatim and replayed through the same extra_items channel as
            // web_search_call. The provider's contract is "an output item you did
            // not consume must be passed back verbatim" — reasoning is the hard
            // constraint measured live (a 400 when missing: "The `reasoning_text`
            // in the thinking mode must be passed back to the API"), and an
            // allowlist enumerated by type would simply replay the same 400 on the
            // next new type
            _ => vec![StreamChunk { raw_items: Some(vec![item.clone()]), ..Default::default() }],
        }
    }

    fn on_completed(&mut self, data: &Value) -> Vec<StreamChunk> {
        // A final-state backstop: scan the complete response of completed once more
        // for web_search_call items and annotations
        if let Some(output) = data.pointer("/response/output").and_then(|v| v.as_array()) {
            for item in output {
                if item.get("type").and_then(|t| t.as_str()) == Some("web_search_call") {
                    self.collect_results_from_item(item);
                }
                if let Some(content) = item.get("content").and_then(|v| v.as_array()) {
                    for part in content {
                        if let Some(annotations) = part.get("annotations").and_then(|v| v.as_array()) {
                            for a in annotations {
                                self.collect_citation(a);
                            }
                        }
                    }
                }
            }
        }
        let mut chunks = vec![];
        if !self.citations.is_empty() {
            chunks.push(StreamChunk {
                web_search: Some(std::mem::take(&mut self.citations)),
                ..Default::default()
            });
        }
        // usage: input/output_tokens are normalised by Usage's serde aliases
        let usage = data
            .pointer("/response/usage")
            .and_then(|v| serde_json::from_value::<Usage>(v.clone()).ok());
        let finish = if self.slot_got_args.is_empty() { "stop" } else { "tool_calls" };
        chunks.push(StreamChunk {
            choices: vec![StreamChoice { delta: Delta::default(), finish_reason: Some(finish.into()) }],
            usage,
            ..Default::default()
        });
        chunks
    }

    fn collect_citation(&mut self, annotation: &Value) {
        // The url_citation shape: {type:"url_citation", url, title, …}
        let Some(url) = annotation.get("url").and_then(|v| v.as_str()) else { return };
        if url.is_empty() || self.citations.iter().any(|c| c.url == url) {
            return;
        }
        self.citations.push(WebSearchResult {
            title: annotation.get("title").and_then(|v| v.as_str()).unwrap_or_default().to_string(),
            url: url.to_string(),
            snippet: String::new(),
            publish_date: None,
        });
    }

    /// The results embedded in a web_search_call output item (the field shape is
    /// still to be verified live, so extraction is tolerant).
    fn collect_results_from_item(&mut self, item: &Value) {
        let candidates = ["results", "sources"]
            .iter()
            .filter_map(|k| item.get(*k).and_then(|v| v.as_array()))
            .chain(["results", "sources"].iter().filter_map(|k| {
                item.get("action").and_then(|a| a.get(*k)).and_then(|v| v.as_array())
            }));
        for arr in candidates {
            for entry in arr {
                if let Ok(r) = serde_json::from_value::<WebSearchResult>(entry.clone()) {
                    if !r.url.is_empty() && !self.citations.iter().any(|c| c.url == r.url) {
                        self.citations.push(r);
                    }
                }
            }
        }
    }
}

fn text_delta(data: &Value) -> Option<String> {
    data.get("delta").and_then(|v| v.as_str()).map(String::from)
}

fn content_chunk(text: String) -> StreamChunk {
    StreamChunk {
        choices: vec![StreamChoice {
            delta: Delta { content: Some(text), ..Default::default() },
            finish_reason: None,
        }],
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
                    function: Some(FunctionDelta {
                        name: name.map(String::from),
                        arguments: args.map(String::from),
                    }),
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
    use crate::types::{ChatMessage, FunctionSpec, ToolCall, ToolCallAccumulator, ToolDef};
    use crate::types::FunctionCall;

    #[test]
    fn effort_maps_to_reasoning_object() {
        let base = ChatRequest {
            model: "deepseek-v4-flash".into(),
            messages: vec![ChatMessage::user("问")],
            tools: None,
            stream: true,
            stream_options: None,
            reasoning_effort: Some("max".into()),
            thinking: None,
        };
        assert_eq!(responses_request_body(&base)["reasoning"]["effort"], "max");
        // The Chat Completions way of turning thinking off (thinking disabled) is restored to the none level on the Responses side
        let off = ChatRequest {
            reasoning_effort: None,
            thinking: Some(json!({ "type": "disabled" })),
            ..base.clone()
        };
        assert_eq!(responses_request_body(&off)["reasoning"]["effort"], "none");
        // Not configured: no reasoning key at all (never send a parameter that was not confirmed)
        let none = ChatRequest { reasoning_effort: None, thinking: None, ..base };
        assert!(responses_request_body(&none).get("reasoning").is_none());
    }

    #[test]
    fn request_maps_roles_tools_and_replays_extra_items() {
        let mut assistant = ChatMessage::assistant(
            Some("查到了".into()),
            Some(vec![ToolCall {
                id: "call_1".into(),
                kind: "function".into(),
                function: FunctionCall { name: "read_note".into(), arguments: r#"{"path":"a.md"}"#.into() },
            }]),
        );
        assistant.extra_items = vec![
            json!({"type": "reasoning", "id": "rs_1", "content": [{"type": "reasoning_text", "text": "推理"}]}),
            json!({"type": "web_search_call", "id": "ws_1", "status": "completed"}),
        ];
        let request = ChatRequest {
            model: "deepseek-v4-flash".into(),
            messages: vec![
                ChatMessage::system("系统规则"),
                ChatMessage::user("查一下"),
                assistant,
                ChatMessage::tool_result("call_1", "笔记内容"),
            ],
            tools: Some(vec![
                ToolDef::function(FunctionSpec {
                    name: "read_note".into(),
                    description: "读笔记".into(),
                    parameters: json!({"type": "object"}),
                }),
                ToolDef::Raw(json!({"type": "web_search"})),
            ]),
            stream: true,
            stream_options: None,
            reasoning_effort: None,
            thinking: None,
        };
        let body = responses_request_body(&request);
        assert_eq!(body["model"], "deepseek-v4-flash");
        assert_eq!(body["instructions"], "系统规则");
        assert_eq!(body["stream"], true);
        let input = body["input"].as_array().unwrap();
        // user, the reasoning/web_search_call replay, the assistant text, function_call, function_call_output
        assert_eq!(input.len(), 6);
        assert_eq!(input[0]["role"], "user");
        assert_eq!(input[0]["content"][0]["type"], "input_text");
        assert_eq!(input[1]["type"], "reasoning"); // replayed verbatim, keeping the stream order and preceding the text
        assert_eq!(input[2]["type"], "web_search_call");
        assert_eq!(input[3]["type"], "message");
        assert_eq!(input[4]["type"], "function_call");
        assert_eq!(input[4]["call_id"], "call_1");
        assert_eq!(input[5]["type"], "function_call_output");
        assert_eq!(input[5]["output"], "笔记内容");
        // tools: the flat Responses function shape (verified live) plus web_search passed straight through
        let tools = body["tools"].as_array().unwrap();
        assert_eq!(tools[0]["type"], "function");
        assert_eq!(tools[0]["name"], "read_note");
        assert_eq!(tools[0]["parameters"]["type"], "object");
        assert!(tools[0].get("function").is_none(), "must be flat, not nested");
        assert_eq!(tools[1], json!({"type": "web_search"}));
    }

    /// A multi-modal user message: Text → input_text, ImageUrl → input_image (a
    /// data URL string); an ImageRef that was never filled in degrades to a
    /// one-line note rather than earning a 400 from the server.
    #[test]
    fn request_maps_image_parts_to_input_image() {
        let request = ChatRequest {
            model: "deepseek-v4-flash".into(),
            messages: vec![ChatMessage::user_content(Content::Parts(vec![
                Part::Text { text: "看图".into() },
                Part::ImageUrl { image_url: crate::types::ImageUrl { url: "data:image/png;base64,AA==".into() } },
                Part::ImageRef { path: "late.png".into() },
            ]))],
            tools: None,
            stream: true,
            stream_options: None,
            reasoning_effort: None,
            thinking: None,
        };
        let body = responses_request_body(&request);
        let content = body["input"][0]["content"].as_array().unwrap();
        assert_eq!(content.len(), 3);
        assert_eq!(content[0], json!({ "type": "input_text", "text": "看图" }));
        assert_eq!(content[1], json!({ "type": "input_image", "image_url": "data:image/png;base64,AA==" }));
        assert_eq!(content[2]["type"], "input_text");
        assert!(content[2]["text"].as_str().unwrap().contains("late.png"));
    }

    /// A complete fixture stream: text and reasoning deltas, a function call, the
    /// web_search status and its citations.
    #[test]
    fn parser_synthesizes_chat_completions_chunks() {
        let mut p = ResponsesParser::default();
        let mut text = String::new();
        let mut reasoning = String::new();
        let mut acc = ToolCallAccumulator::default();
        let mut statuses = vec![];
        let mut citations: Vec<WebSearchResult> = vec![];
        let mut raw_items: Vec<Value> = vec![];
        let mut queries: Vec<String> = vec![];
        let mut finish = None;

        let events: Vec<(&str, Value)> = vec![
            ("response.created", json!({})),
            ("response.web_search_call.in_progress", json!({})),
            ("response.web_search_call.searching", json!({})), // deduplicated: Searching is emitted once
            ("response.web_search_call.completed", json!({})),
            (
                "response.output_item.done",
                json!({"item": {"type": "web_search_call", "id": "ws_1", "status": "completed",
                    "action": {"type": "search", "queries": ["AI 新闻", "ws_call_id=call_x"]},
                    "results": [{"title": "新闻一", "link": "https://a.example/1", "content": "摘要一"}]}}),
            ),
            ("response.reasoning_text.delta", json!({"delta": "思考"})),
            (
                "response.output_item.done",
                json!({"item": {"type": "reasoning", "id": "rs_1",
                    "content": [{"type": "reasoning_text", "text": "思考"}]}}),
            ),
            ("response.output_text.delta", json!({"delta": "根据"})),
            ("response.output_text.delta", json!({"delta": "搜索"})),
            (
                "response.output_text.annotation.added",
                json!({"annotation": {"type": "url_citation", "url": "https://b.example/2", "title": "来源二"}}),
            ),
            (
                "response.output_item.added",
                json!({"item": {"type": "function_call", "id": "item_fc", "call_id": "call_9", "name": "read_note", "arguments": ""}}),
            ),
            ("response.function_call_arguments.delta", json!({"item_id": "item_fc", "delta": "{\"path\":"})),
            ("response.function_call_arguments.delta", json!({"item_id": "item_fc", "delta": "\"a.md\"}"})),
            ("response.function_call_arguments.done", json!({"item_id": "item_fc", "arguments": "{\"path\":\"a.md\"}"})), // deltas already arrived; must not duplicate
            ("response.completed", json!({"response": {"output": []}})),
        ];

        for (ty, data) in events {
            for chunk in p.handle(ty, &data) {
                if let Some(s) = chunk.web_search_status {
                    statuses.push(s);
                }
                if let Some(rs) = chunk.web_search {
                    citations.extend(rs);
                }
                if let Some(items) = chunk.raw_items {
                    raw_items.extend(items);
                }
                if let Some(qs) = chunk.web_search_queries {
                    queries.extend(qs);
                }
                for choice in &chunk.choices {
                    if let Some(t) = &choice.delta.content {
                        text.push_str(t);
                    }
                    if let Some(t) = &choice.delta.reasoning_content {
                        reasoning.push_str(t);
                    }
                    if let Some(tcs) = &choice.delta.tool_calls {
                        for tc in tcs {
                            acc.push(tc);
                        }
                    }
                    if let Some(f) = &choice.finish_reason {
                        finish = Some(f.clone());
                    }
                }
            }
        }

        assert_eq!(text, "根据搜索");
        assert_eq!(reasoning, "思考");
        assert_eq!(statuses, vec![WebSearchStatus::Searching, WebSearchStatus::Completed]);
        // Citations merged: the results embedded in web_search_call (via the link/content aliases) plus the url_citation annotation
        assert_eq!(citations.len(), 2);
        assert_eq!(citations[0].url, "https://a.example/1");
        assert_eq!(citations[0].snippet, "摘要一");
        assert_eq!(citations[1].title, "来源二");
        // Both the web_search_call and the reasoning item are kept verbatim for replay (a hard constraint of thinking mode)
        assert_eq!(raw_items.len(), 2);
        assert_eq!(raw_items[0]["id"], "ws_1");
        assert_eq!(raw_items[1]["id"], "rs_1");
        assert_eq!(raw_items[1]["type"], "reasoning");
        // The queries are extracted, and the ws_call_id= pseudo query is filtered out
        assert_eq!(queries, vec!["AI 新闻"]);
        // The tool call is reconstructed whole by ToolCallAccumulator, with no duplicated arguments
        let calls = acc.finish();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].id, "call_9");
        assert_eq!(calls[0].function.name, "read_note");
        assert_eq!(calls[0].function.arguments, r#"{"path":"a.md"}"#);
        assert_eq!(finish.as_deref(), Some("tool_calls"));
    }

    /// A provider that sends done but never a delta: the whole arguments is emitted.
    #[test]
    fn parser_recovers_args_from_done_only() {
        let mut p = ResponsesParser::default();
        let mut acc = ToolCallAccumulator::default();
        let events: Vec<(&str, Value)> = vec![
            (
                "response.output_item.added",
                json!({"item": {"type": "function_call", "id": "i1", "call_id": "c1", "name": "list_notes", "arguments": ""}}),
            ),
            ("response.function_call_arguments.done", json!({"item_id": "i1", "arguments": "{}"})),
            ("response.completed", json!({})),
        ];
        for (ty, data) in events {
            for chunk in p.handle(ty, &data) {
                for choice in &chunk.choices {
                    if let Some(tcs) = &choice.delta.tool_calls {
                        for tc in tcs {
                            acc.push(tc);
                        }
                    }
                }
            }
        }
        let calls = acc.finish();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].function.arguments, "{}");
    }

    /// A provider with no arguments events at all, where only output_item.done
    /// carries the complete arguments: an already registered slot must have them
    /// emitted too, rather than finishing with empty arguments.
    #[test]
    fn parser_backfills_args_from_item_done_on_registered_slot() {
        let mut p = ResponsesParser::default();
        let mut acc = ToolCallAccumulator::default();
        let events: Vec<(&str, Value)> = vec![
            (
                "response.output_item.added",
                json!({"item": {"type": "function_call", "id": "i1", "call_id": "c1", "name": "list_notes", "arguments": ""}}),
            ),
            (
                "response.output_item.done",
                json!({"item": {"type": "function_call", "id": "i1", "call_id": "c1", "name": "list_notes", "arguments": "{\"dir\":\"/\"}"}}),
            ),
            ("response.completed", json!({})),
        ];
        for (ty, data) in events {
            for chunk in p.handle(ty, &data) {
                for choice in &chunk.choices {
                    if let Some(tcs) = &choice.delta.tool_calls {
                        for tc in tcs {
                            acc.push(tc);
                        }
                    }
                }
            }
        }
        let calls = acc.finish();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].id, "c1");
        assert_eq!(calls[0].function.name, "list_notes");
        assert_eq!(calls[0].function.arguments, r#"{"dir":"/"}"#);
    }

    /// added has no item id and only done carries one: align onto the same slot by
    /// call_id, and never fallback-register a second ToolCall sharing that call_id.
    #[test]
    fn parser_reconciles_done_by_call_id_when_added_lacks_item_id() {
        let mut p = ResponsesParser::default();
        let mut acc = ToolCallAccumulator::default();
        let events: Vec<(&str, Value)> = vec![
            (
                "response.output_item.added",
                json!({"item": {"type": "function_call", "call_id": "c1", "name": "read_note", "arguments": ""}}),
            ),
            (
                "response.output_item.done",
                json!({"item": {"type": "function_call", "id": "i1", "call_id": "c1", "name": "read_note", "arguments": "{\"path\":\"a.md\"}"}}),
            ),
            ("response.completed", json!({})),
        ];
        for (ty, data) in events {
            for chunk in p.handle(ty, &data) {
                for choice in &chunk.choices {
                    if let Some(tcs) = &choice.delta.tool_calls {
                        for tc in tcs {
                            acc.push(tc);
                        }
                    }
                }
            }
        }
        let calls = acc.finish();
        assert_eq!(calls.len(), 1, "must reconcile to one slot, not register a duplicate");
        assert_eq!(calls[0].id, "c1");
        assert_eq!(calls[0].function.name, "read_note");
        assert_eq!(calls[0].function.arguments, r#"{"path":"a.md"}"#);
    }
}
