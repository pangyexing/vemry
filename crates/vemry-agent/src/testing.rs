//! Test support shared by the offline evaluation suite (`tests/evals.rs`): a
//! scripted transport that replays streaming chunks instead of calling a
//! provider, plus chunk constructors. Compiled into the library (an integration
//! test cannot see `#[cfg(test)]` items) and hidden from the documentation.

use std::collections::VecDeque;
use std::sync::atomic::AtomicBool;
use std::sync::Mutex;

use async_trait::async_trait;

use crate::client::ChatTransport;
use crate::provider::ProviderConfig;
use crate::types::{ChatRequest, Delta, FunctionDelta, StreamChoice, StreamChunk, ToolCallDelta, WebSearchResult};
use crate::Result;

/// Each `stream_chat` call replays the next chunk list; the requests it saw are
/// kept for assertions on the wire shape.
pub struct ScriptedTransport {
    scripts: Mutex<VecDeque<Vec<StreamChunk>>>,
    pub requests: Mutex<Vec<ChatRequest>>,
}

impl ScriptedTransport {
    pub fn new(scripts: Vec<Vec<StreamChunk>>) -> Self {
        ScriptedTransport { scripts: Mutex::new(scripts.into()), requests: Mutex::new(vec![]) }
    }

    /// How many scripted responses are still unplayed (an eval asserts zero).
    pub fn remaining(&self) -> usize {
        self.scripts.lock().unwrap().len()
    }
}

#[async_trait]
impl ChatTransport for ScriptedTransport {
    async fn stream_chat(
        &self,
        _config: &ProviderConfig,
        request: &ChatRequest,
        _cancel: &AtomicBool,
        on_chunk: &mut (dyn FnMut(StreamChunk) -> bool + Send),
    ) -> Result<()> {
        self.requests.lock().unwrap().push(request.clone());
        let script = self
            .scripts
            .lock()
            .unwrap()
            .pop_front()
            .ok_or_else(|| crate::AgentError::Api("scripted transport: no more responses".into()))?;
        for chunk in script {
            if !on_chunk(chunk) {
                break;
            }
        }
        Ok(())
    }
}

pub fn text_chunk(t: &str) -> StreamChunk {
    StreamChunk {
        choices: vec![StreamChoice { delta: Delta { content: Some(t.into()), ..Default::default() }, finish_reason: None }],
        ..Default::default()
    }
}

pub fn finish_chunk(reason: &str) -> StreamChunk {
    StreamChunk {
        choices: vec![StreamChoice { delta: Delta::default(), finish_reason: Some(reason.into()) }],
        ..Default::default()
    }
}

/// One complete tool call in a single chunk (`index` orders several calls of one batch).
pub fn tool_call_chunk(index: usize, id: &str, name: &str, args: &str) -> StreamChunk {
    StreamChunk {
        choices: vec![StreamChoice {
            delta: Delta {
                tool_calls: Some(vec![ToolCallDelta {
                    index: Some(index),
                    id: Some(id.into()),
                    function: Some(FunctionDelta { name: Some(name.into()), arguments: Some(args.into()) }),
                }]),
                ..Default::default()
            },
            finish_reason: None,
        }],
        ..Default::default()
    }
}

/// A server-side search's queries (the DeepSeek Responses shape).
pub fn search_queries_chunk(queries: &[&str]) -> StreamChunk {
    StreamChunk { web_search_queries: Some(queries.iter().map(|q| q.to_string()).collect()), ..Default::default() }
}

/// Structured search results (kept for providers that return them).
pub fn search_results_chunk(results: Vec<WebSearchResult>) -> StreamChunk {
    StreamChunk { web_search: Some(results), ..Default::default() }
}
