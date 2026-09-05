//! Live integration test against a real LLM API.
//!
//! Ignored by default (network + key + a fraction of a cent of tokens).
//! Run explicitly:
//!   VEMRY_TEST_API_KEY=sk-... cargo test -p vemry-agent --test live -- --ignored
//! Optional: VEMRY_TEST_BASE_URL (default DeepSeek), VEMRY_TEST_MODEL.

use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use vemry_agent::permission::AutoAllow;
use vemry_agent::runner::{AgentEvent, AgentRunner, RunnerConfig, WritePolicy};
use vemry_agent::session::SessionLog;
use vemry_agent::{HttpTransport, ProviderConfig, ResponsesTransport};
use vemry_core::Vault;

#[tokio::test]
#[ignore]
async fn live_agent_creates_note_via_tools() {
    let Some(api_key) = std::env::var("VEMRY_TEST_API_KEY").ok().filter(|k| !k.is_empty()) else {
        eprintln!("VEMRY_TEST_API_KEY not set — skipping live test");
        return;
    };
    let provider = ProviderConfig {
        base_url: std::env::var("VEMRY_TEST_BASE_URL").unwrap_or_else(|_| "https://api.deepseek.com".into()),
        model: std::env::var("VEMRY_TEST_MODEL").unwrap_or_else(|_| "deepseek-v4-flash".into()),
        api_key,
    };

    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("Existing.md"),
        "# Existing\n\nvault seed note for live test\n",
    )
    .unwrap();
    let vault = Arc::new(Mutex::new(Some(Vault::open(dir.path()).unwrap())));
    let meta_dir = dir.path().join(".vemry");
    std::fs::create_dir_all(&meta_dir).unwrap();
    let log = SessionLog::create(&meta_dir).unwrap();

    let runner = AgentRunner {
        transport: Arc::new(HttpTransport::new()),
        gate: Arc::new(AutoAllow),
        plan_gate: Arc::new(vemry_agent::permission::AutoApprovePlan),
        vault: Arc::clone(&vault),
        sources: Arc::new(Mutex::new(vec![])),
    };
    let mut history = vec![];
    tokio::time::timeout(
        Duration::from_secs(180),
        runner.run(
            &provider,
            &RunnerConfig {
                max_iterations: 10,
                write_policy: WritePolicy::Allow,
                ..Default::default()
            },
            &mut history,
            Some("Use the write_note tool to create a note at notes/auto-test.md whose body is the single line: hello from live test. Confirm briefly once it is done, and do nothing else.".into()),
            None,
            vec![],
            None,
            &|_e| {},
            &AtomicBool::new(false),
            &log,
        ),
    )
    .await
    .expect("live agent run timed out")
    .expect("live agent run failed");

    // the note must exist on disk with the requested content
    let written = std::fs::read_to_string(dir.path().join("notes/auto-test.md"))
        .expect("agent did not create notes/auto-test.md");
    assert!(written.contains("hello from live test"), "unexpected content: {written}");
    // agent-created note carries provenance (unless the model supplied frontmatter itself)
    // history: user + at least one assistant with tool_calls + tool result + final assistant
    assert!(history.len() >= 4, "history too short: {} entries", history.len());
    assert!(history.iter().any(|m| m.role == "tool"), "no tool result in history");
    // audit log recorded the write
    let session_files: Vec<_> = std::fs::read_dir(meta_dir.join("sessions")).unwrap().collect();
    assert_eq!(session_files.len(), 1);
}

/// The v0.4 decision points (items 1-3 of the plan's "to verify" list): the
/// DeepSeek Responses API's `/responses` path, the shape of the streamed citation
/// fields, and the replay behaviour of web_search_call.
/// The findings feed back into the tolerant extraction in responses.rs and into
/// the PR description.
#[tokio::test]
#[ignore]
async fn live_responses_web_search() {
    let Some(api_key) = std::env::var("VEMRY_TEST_API_KEY").ok().filter(|k| !k.is_empty()) else {
        eprintln!("VEMRY_TEST_API_KEY not set — skipping live test");
        return;
    };
    let provider = ProviderConfig {
        base_url: std::env::var("VEMRY_TEST_BASE_URL").unwrap_or_else(|_| "https://api.deepseek.com".into()),
        model: std::env::var("VEMRY_TEST_MODEL").unwrap_or_else(|_| "deepseek-v4-flash".into()),
        api_key,
    };

    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("Seed.md"), "# Seed\n").unwrap();
    let vault = Arc::new(Mutex::new(Some(Vault::open(dir.path()).unwrap())));
    let meta_dir = dir.path().join(".vemry");
    std::fs::create_dir_all(&meta_dir).unwrap();
    let log = SessionLog::create(&meta_dir).unwrap();

    let runner = AgentRunner {
        transport: Arc::new(ResponsesTransport::new()),
        gate: Arc::new(AutoAllow),
        plan_gate: Arc::new(vemry_agent::permission::AutoApprovePlan),
        vault: Arc::clone(&vault),
        sources: Arc::new(Mutex::new(vec![])),
    };
    let events: Arc<Mutex<Vec<AgentEvent>>> = Arc::new(Mutex::new(vec![]));
    let ev = events.clone();
    let mut history = vec![];
    tokio::time::timeout(
        Duration::from_secs(180),
        runner.run(
            &provider,
            &RunnerConfig { max_iterations: 6, write_policy: WritePolicy::Ask, web_search: true, ..Default::default() },
            &mut history,
            Some("Search the web: what is today's most important technology news story worldwide? Summarise it in one sentence and give the source.".into()),
            None,
            vec![],
            None,
            &move |e| ev.lock().unwrap().push(e),
            &AtomicBool::new(false),
            &log,
        ),
    )
    .await
    .expect("live web search run timed out")
    .expect("live web search run failed");

    let events = events.lock().unwrap();
    let searched = events.iter().any(|e| matches!(e, AgentEvent::WebSearching));
    let results: Vec<_> = events
        .iter()
        .filter_map(|e| match e {
            AgentEvent::WebSearchResults { results } => Some(results.clone()),
            _ => None,
        })
        .flatten()
        .collect();
    let text: String = events
        .iter()
        .filter_map(|e| match e {
            AgentEvent::TextDelta { text } => Some(text.as_str()),
            _ => None,
        })
        .collect();
    eprintln!("=== searched: {searched}");
    eprintln!("=== results ({}): {:#?}", results.len(), results);
    eprintln!("=== text: {text}");
    assert!(!text.is_empty(), "no answer text");
    assert!(
        searched || !results.is_empty(),
        "no web search activity observed — check /responses path, tool shape, and citation fields"
    );
}

/// An action item from the comparison study (2026-08-19): probe whether
/// DeepSeek Responses' web_search accepts OpenAI-style tool parameters
/// (`filters.allowed_domains`). Three outcomes are possible: refused with a 400 /
/// accepted and ignored / accepted and effective — the first is decided by the
/// error itself, and for the other two the printed queries and the source domains
/// in the answer are read by hand (the same "weak but honest" assertions as
/// live_responses_web_search). **The prompt deliberately names no domain**: name
/// one and the model writes its own site: query, which makes "the filter worked"
/// indistinguishable from "it copied the prompt" (a confusion the first run on
/// 2026-08-19 walked into). The criterion: queries that carry site:xinhuanet.com
/// of their own accord, or source domains that are constrained → effective;
/// generic queries and sources across many domains → the parameter is silently
/// ignored.
/// This bypasses AgentRunner and talks to the transport directly: a parameterised
/// tool body is not part of the runner's injection surface, and whether that
/// surface should take parameters at all depends on what this probe concludes.
///
/// Measured conclusion (2026-08-19, deepseek-v4-flash): **accepted and completely
/// ignored** — under a prompt that names no domain, all 8 queries carried no site:
/// operator and the source domains (36kr/163/chinadaily/zaobao…) were not
/// constrained by allowed_domains. The runner stays parameterless. Should DeepSeek
/// document parameter support later, re-running this test rechecks it.
#[tokio::test]
#[ignore]
async fn live_responses_web_search_params_probe() {
    use vemry_agent::types::{ChatMessage, ChatRequest, StreamChunk, ToolDef};
    use vemry_agent::ChatTransport;

    let Some(api_key) = std::env::var("VEMRY_TEST_API_KEY").ok().filter(|k| !k.is_empty()) else {
        eprintln!("VEMRY_TEST_API_KEY not set — skipping live test");
        return;
    };
    let provider = ProviderConfig {
        base_url: std::env::var("VEMRY_TEST_BASE_URL").unwrap_or_else(|_| "https://api.deepseek.com".into()),
        model: std::env::var("VEMRY_TEST_MODEL").unwrap_or_else(|_| "deepseek-v4-flash".into()),
        api_key,
    };
    let transport = ResponsesTransport::new();
    let request = ChatRequest {
        model: provider.model.clone(),
        messages: vec![ChatMessage::user(
            "Search the web: important artificial-intelligence news from the past week. Pick three stories, summarise each in one sentence, and give the source links.".to_string(),
        )],
        tools: Some(vec![ToolDef::Raw(serde_json::json!({
            "type": "web_search",
            "filters": { "allowed_domains": ["xinhuanet.com"] }
        }))]),
        stream: true,
        stream_options: None,
        reasoning_effort: None,
        thinking: None,
    };
    let cancel = AtomicBool::new(false);
    let mut text = String::new();
    let mut queries: Vec<String> = vec![];
    let outcome = {
        let text_ref = &mut text;
        let queries_ref = &mut queries;
        let mut on_chunk = move |chunk: StreamChunk| -> bool {
            if let Some(q) = &chunk.web_search_queries {
                queries_ref.extend(q.iter().cloned());
            }
            for c in &chunk.choices {
                if let Some(t) = &c.delta.content {
                    text_ref.push_str(t);
                }
            }
            true
        };
        tokio::time::timeout(
            Duration::from_secs(180),
            transport.stream_chat(&provider, &request, &cancel, &mut on_chunk),
        )
        .await
        .expect("params probe timed out")
    };

    match outcome {
        Err(e) => {
            // Outcome A: the server refuses the parameters — the runner is right to keep its Raw tool body parameterless
            eprintln!("=== verdict: parameters REJECTED by server — {e}");
        }
        Ok(()) => {
            // Outcomes B/C: the parameters were accepted; whether the queries and
            // the source domains in the answer are constrained to xinhuanet.com
            // decides "ignored" versus "effective", to be read by hand
            eprintln!("=== verdict: parameters ACCEPTED");
            eprintln!("=== queries: {queries:?}");
            eprintln!("=== text: {text}");
            assert!(!text.is_empty(), "parameters accepted but no answer text");
        }
    }
}

/// A probe ahead of batch C item ③ (2026-08-24): verify that the effort levels
/// really are honoured by the official endpoint rather than "accepted and ignored"
/// (the trap params_probe walked into with filters.allowed_domains — exposing a
/// control surface that does nothing is worse than having none). Two halves:
/// (a) The Responses line: run the same reasoning question twice, once with
///     `reasoning:{effort:"none"}` and once with `"max"`, and compare how much
///     reasoning came out — none should produce almost no thinking stream, max
///     noticeably more (a weak but honest assertion: none < max, and the answer is
///     non-empty);
/// (b) The Chat Completions line (HttpTransport, which is what DeepSeek takes with
///     web search off): one run with the top-level `reasoning_effort:"low"` and one
///     with `thinking:{"type":"disabled"}`, printing the three possible verdicts
///     (refused / accepted but ignored / effective) to be read by hand. Should (b)
///     be refused, the two Chat Completions fields in types.rs simply stop being
///     sent (a one-line change) and the Responses line is unaffected.
///
/// Measured conclusion (to be filled in):
#[tokio::test]
#[ignore]
async fn live_reasoning_effort_probe() {
    use vemry_agent::types::{ChatMessage, ChatRequest, StreamChunk};
    use vemry_agent::ChatTransport;

    let Some(api_key) = std::env::var("VEMRY_TEST_API_KEY").ok().filter(|k| !k.is_empty()) else {
        eprintln!("VEMRY_TEST_API_KEY not set — skipping live test");
        return;
    };
    let provider = ProviderConfig {
        base_url: std::env::var("VEMRY_TEST_BASE_URL").unwrap_or_else(|_| "https://api.deepseek.com".into()),
        model: std::env::var("VEMRY_TEST_MODEL").unwrap_or_else(|_| "deepseek-v4-flash".into()),
        api_key,
    };
    const PROMPT: &str = "One of 9 identical-looking coins is lighter. What is the smallest number of weighings on a balance that is guaranteed to find it? Give the procedure and prove the lower bound.";

    // One request → (answer length, thinking-stream length, result)
    async fn probe(
        transport: &dyn ChatTransport,
        provider: &ProviderConfig,
        request: &ChatRequest,
    ) -> (usize, usize, vemry_agent::Result<()>) {
        let cancel = AtomicBool::new(false);
        let mut text_len = 0usize;
        let mut reasoning_len = 0usize;
        let outcome = {
            let t = &mut text_len;
            let r = &mut reasoning_len;
            let mut on_chunk = move |chunk: StreamChunk| -> bool {
                for c in &chunk.choices {
                    if let Some(s) = &c.delta.content {
                        *t += s.chars().count();
                    }
                    if let Some(s) = &c.delta.reasoning_content {
                        *r += s.chars().count();
                    }
                }
                true
            };
            tokio::time::timeout(
                Duration::from_secs(240),
                transport.stream_chat(provider, request, &cancel, &mut on_chunk),
            )
            .await
            .expect("effort probe timed out")
        };
        (text_len, reasoning_len, outcome)
    }

    let base = ChatRequest {
        model: provider.model.clone(),
        messages: vec![ChatMessage::user(PROMPT.to_string())],
        tools: None,
        stream: true,
        stream_options: None,
        reasoning_effort: None,
        thinking: None,
    };

    // ---- (a) the Responses line: none vs max ----
    let responses = ResponsesTransport::new();
    let req_none =
        ChatRequest { thinking: Some(serde_json::json!({ "type": "disabled" })), ..base.clone() };
    let req_max = ChatRequest { reasoning_effort: Some("max".into()), ..base.clone() };
    let (t_none, r_none, out_none) = probe(&responses, &provider, &req_none).await;
    let (t_max, r_max, out_max) = probe(&responses, &provider, &req_max).await;
    eprintln!("=== [responses] effort=none: text={t_none} reasoning={r_none} result={out_none:?}");
    eprintln!("=== [responses] effort=max:  text={t_max} reasoning={r_max} result={out_max:?}");
    match (&out_none, &out_max) {
        (Err(e), _) | (_, Err(e)) => {
            eprintln!("=== [responses] verdict: effort param REJECTED — {e} (fix: the Responses line stops sending reasoning)");
        }
        (Ok(()), Ok(())) => {
            assert!(t_none > 0 && t_max > 0, "accepted but no answer text");
            // The criterion: the none level's thinking stream should be far shorter
            // than max's (zero is ideal); no clear difference in magnitude means
            // "accepted but ignored" — record that and withdraw the UI
            eprintln!(
                "=== [responses] verdict: {}",
                if r_none * 4 < r_max.max(1) { "effort EFFECTIVE" } else { "accepted but possibly IGNORED — check by hand" }
            );
            assert!(r_none < r_max, "the none level's thinking stream is not shorter than max's — effort appears to have no effect");
        }
    }

    // ---- (b) the Chat Completions line: reasoning_effort=low / thinking=disabled ----
    let cc = HttpTransport::new();
    let req_low = ChatRequest { reasoning_effort: Some("low".into()), ..base.clone() };
    let req_off =
        ChatRequest { thinking: Some(serde_json::json!({ "type": "disabled" })), ..base.clone() };
    let (t_low, r_low, out_low) = probe(&cc, &provider, &req_low).await;
    let (t_off, r_off, out_off) = probe(&cc, &provider, &req_off).await;
    eprintln!("=== [cc] reasoning_effort=low:   text={t_low} reasoning={r_low} result={out_low:?}");
    eprintln!("=== [cc] thinking=disabled:      text={t_off} reasoning={r_off} result={out_off:?}");
    match (&out_low, &out_off) {
        (Err(e), _) | (_, Err(e)) => {
            eprintln!("=== [cc] verdict: REJECTED — {e} (fix: the two Chat Completions fields in types.rs stop being sent)");
        }
        (Ok(()), Ok(())) => {
            eprintln!(
                "=== [cc] verdict: accepted; thinking=disabled thinking stream {} (expected 0) / low level {}",
                r_off, r_low
            );
            assert!(t_low > 0 && t_off > 0, "accepted but no answer text");
        }
    }
}
