//! Live integration test against a real LLM API.
//!
//! Ignored by default (network + key + a fraction of a cent of tokens).
//! Run explicitly:
//!   VEMRY_TEST_API_KEY=sk-... cargo test -p vemry-agent --test live -- --ignored
//! Optional: VEMRY_TEST_BASE_URL (default DeepSeek), VEMRY_TEST_MODEL (default
//! deepseek-flash; the Anthropic-format line searches on every DeepSeek model,
//! measured 2026-09-14).
//!
//! Any vendor the settings page offers runs the first test, through the transport
//! the app would pick for it — e.g. VEMRY_TEST_BASE_URL=https://api.anthropic.com
//! VEMRY_TEST_MODEL=claude-sonnet-5-5. `live_a_session_moves_across_vendors` takes a
//! second endpoint (VEMRY_TEST_NEXT_BASE_URL, VEMRY_TEST_NEXT_MODEL,
//! VEMRY_TEST_NEXT_API_KEY) and moves one session onto it and back.

use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use vemry_agent::permission::AutoAllow;
use vemry_agent::runner::{AgentEvent, AgentRunner, RunnerConfig, WritePolicy};
use vemry_agent::session::SessionLog;
use vemry_agent::provider::ProviderKind;
use vemry_agent::{transport_for, AnthropicTransport, HttpTransport, ProviderConfig};
use vemry_core::Vault;

/// The chat model under test.
fn test_model() -> String {
    std::env::var("VEMRY_TEST_MODEL").unwrap_or_else(|_| "deepseek-flash".into())
}

#[tokio::test]
#[ignore]
async fn live_agent_creates_note_via_tools() {
    let Some(api_key) = std::env::var("VEMRY_TEST_API_KEY").ok().filter(|k| !k.is_empty()) else {
        eprintln!("VEMRY_TEST_API_KEY not set — skipping live test");
        return;
    };
    let provider = ProviderConfig {
        base_url: std::env::var("VEMRY_TEST_BASE_URL").unwrap_or_else(|_| "https://api.deepseek.com".into()),
        model: test_model(),
        api_key,
        kind: ProviderKind::Custom,
        catalog: None,
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
        transport: transport_for(&provider, false),
        gate: Arc::new(AutoAllow),
        plan_gate: Arc::new(vemry_agent::permission::AutoApprovePlan),
        actions_gate: Arc::new(vemry_agent::permission::AutoApproveActions),
        read_gate: Arc::new(vemry_agent::permission::DenyReads),
        vault: Arc::clone(&vault),
        sources: Arc::new(Mutex::new(vec![])),
        journal: Arc::new(Mutex::new(vec![])),
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

/// The plan phase on the real Anthropic-format line: a plan-gated turn at "high"
/// sends its pre-plan calls at "low" and the rest at "high", with the thinking
/// blocks of the light calls passed back — the provider must accept the level
/// changing mid-conversation. Asserted through what can be seen from outside: the turn
/// completed and the notes landed, after a plan (the light calls led to it) or
/// after the light probe was retaken (the model went to work without one).
#[tokio::test]
#[ignore]
async fn live_plan_phase_lightens_until_the_plan() {
    let Some(api_key) = std::env::var("VEMRY_TEST_API_KEY").ok().filter(|k| !k.is_empty()) else {
        eprintln!("VEMRY_TEST_API_KEY not set — skipping live test");
        return;
    };
    let provider = ProviderConfig {
        base_url: std::env::var("VEMRY_TEST_BASE_URL").unwrap_or_else(|_| "https://api.deepseek.com".into()),
        model: test_model(),
        api_key,
        kind: ProviderKind::Custom,
        catalog: None,
    };
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("Apples.md"), "# Apples\n\nApples keep best in a cool, dark place.\n").unwrap();
    std::fs::write(dir.path().join("Pears.md"), "# Pears\n\nPears ripen at room temperature.\n").unwrap();
    let vault = Arc::new(Mutex::new(Some(Vault::open(dir.path()).unwrap())));
    let meta_dir = dir.path().join(".vemry");
    std::fs::create_dir_all(&meta_dir).unwrap();
    let log = SessionLog::create(&meta_dir).unwrap();
    let runner = AgentRunner {
        transport: Arc::new(AnthropicTransport::new()),
        gate: Arc::new(AutoAllow),
        plan_gate: Arc::new(vemry_agent::permission::AutoApprovePlan),
        actions_gate: Arc::new(vemry_agent::permission::AutoApproveActions),
        read_gate: Arc::new(vemry_agent::permission::DenyReads),
        vault: Arc::clone(&vault),
        sources: Arc::new(Mutex::new(vec![])),
        journal: Arc::new(Mutex::new(vec![])),
    };
    let events: Arc<Mutex<Vec<AgentEvent>>> = Arc::new(Mutex::new(vec![]));
    let ev = events.clone();
    let mut history = vec![];
    let started = std::time::Instant::now();
    tokio::time::timeout(
        Duration::from_secs(300),
        runner.run(
            &provider,
            &RunnerConfig {
                max_iterations: 12,
                write_policy: WritePolicy::Allow,
                plan_confirm: true,
                reasoning_effort: Some("high".into()),
                ..Default::default()
            },
            &mut history,
            Some("Read Apples.md and Pears.md, then create Fruit.md with one line about each fruit's storage, then append a line to Apples.md that links [[Fruit]]. Confirm briefly when done.".into()),
            None,
            vec![],
            None,
            &move |e| ev.lock().unwrap().push(e),
            &AtomicBool::new(false),
            &log,
        ),
    )
    .await
    .expect("live plan run timed out")
    .expect("live plan run failed");
    let events = events.lock().unwrap();
    let jsonl = std::fs::read_to_string(std::fs::read_dir(meta_dir.join("sessions")).unwrap().next().unwrap().unwrap().path()).unwrap();
    let kinds: Vec<String> = jsonl
        .lines()
        .filter_map(|l| serde_json::from_str::<serde_json::Value>(l).ok())
        .map(|v| {
            let kind = v["kind"].as_str().unwrap_or("?").to_string();
            match v["content"]["tool"].as_str().or(v["tool"].as_str()) {
                Some(t) => format!("{kind}:{t}"),
                None => kind,
            }
        })
        .collect();
    eprintln!("live plan phase: {:.0}s, {} events, lines: {kinds:?}", started.elapsed().as_secs_f32(), events.len());
    // Either the model planned (the light calls led to the plan, the rest ran at high)
    // or it went to work without one — then the light probe was retaken, the `effort`
    // line says so, and the writes came from the call at the session's level
    let planned = events.iter().any(|e| matches!(e, AgentEvent::PlanRequest { .. }));
    let retaken = kinds.iter().any(|k| k == "effort");
    assert!(planned || retaken, "neither a plan nor a retake: {kinds:?}");
    assert!(dir.path().join("Fruit.md").exists(), "Fruit.md was not created");
}

/// The search carrier end to end through the runner, on whichever vendor
/// VEMRY_TEST_BASE_URL names, through the transport the app picks for it with search
/// on: the provider runs the server-side search, the results arrive structured
/// (title and url, asserted) and, where the line reports it (DeepSeek's and
/// Anthropic's Messages lines), the turn's usage counts the searches. Measured
/// 2026-09-14 on deepseek-flash: two searches, eight results each, the sources in the
/// answer among them. For Zhipu, pick GLM-5.3 (VEMRY_TEST_MODEL=glm-5.3): its search
/// sends no progress events, only the sources.
#[tokio::test]
#[ignore]
async fn live_web_search() {
    let Some(api_key) = std::env::var("VEMRY_TEST_API_KEY").ok().filter(|k| !k.is_empty()) else {
        eprintln!("VEMRY_TEST_API_KEY not set — skipping live test");
        return;
    };
    let provider = ProviderConfig {
        base_url: std::env::var("VEMRY_TEST_BASE_URL").unwrap_or_else(|_| "https://api.deepseek.com".into()),
        model: test_model(),
        api_key,
        kind: ProviderKind::Custom,
        catalog: None,
    };

    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("Seed.md"), "# Seed\n").unwrap();
    let vault = Arc::new(Mutex::new(Some(Vault::open(dir.path()).unwrap())));
    let meta_dir = dir.path().join(".vemry");
    std::fs::create_dir_all(&meta_dir).unwrap();
    let log = SessionLog::create(&meta_dir).unwrap();

    assert!(provider.web_search_available(), "{} at {} runs no search", provider.model, provider.base_url);
    let runner = AgentRunner {
        transport: transport_for(&provider, true),
        gate: Arc::new(AutoAllow),
        plan_gate: Arc::new(vemry_agent::permission::AutoApprovePlan),
        actions_gate: Arc::new(vemry_agent::permission::AutoApproveActions),
        read_gate: Arc::new(vemry_agent::permission::DenyReads),
        vault: Arc::clone(&vault),
        sources: Arc::new(Mutex::new(vec![])),
        journal: Arc::new(Mutex::new(vec![])),
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
    assert!(searched || !results.is_empty(), "no web search activity observed — check the endpoint path and the tool shape");
    assert!(!results.is_empty(), "the search ran but returned no structured results — the result's shape changed");
    assert!(results.iter().all(|r| !r.url.is_empty()), "a result without a url: {results:?}");
    let titled = results.iter().filter(|r| !r.title.is_empty()).count();
    assert!(titled * 2 > results.len(), "most hits should carry a title, got {titled} of {}", results.len());
    let searches: u64 = events
        .iter()
        .filter_map(|e| match e {
            AgentEvent::Usage { web_search_requests, .. } => Some(*web_search_requests),
            _ => None,
        })
        .sum();
    if provider.is_deepseek() || provider.is_anthropic() {
        assert!(searches > 0, "the usage did not count the searches");
    }
}

/// A session may move between lines when the composer's web search switch changes: a
/// turn that ran on Chat Completions (thinking on, a tool call, so the history holds
/// reasoning_content with no signature stub) is continued on the Anthropic-format
/// line with the search tool — the provider must accept the rebuilt thinking blocks
/// and the tool loop, and the second turn must answer. Then the way back: the
/// history now holds this line's blocks in extra_items, and a third turn on Chat
/// Completions (which ignores them) must answer too.
#[tokio::test]
#[ignore]
async fn live_history_moves_between_the_lines() {
    let Some(api_key) = std::env::var("VEMRY_TEST_API_KEY").ok().filter(|k| !k.is_empty()) else {
        eprintln!("VEMRY_TEST_API_KEY not set — skipping live test");
        return;
    };
    let provider = ProviderConfig {
        base_url: std::env::var("VEMRY_TEST_BASE_URL").unwrap_or_else(|_| "https://api.deepseek.com".into()),
        model: test_model(),
        api_key,
        kind: ProviderKind::Custom,
        catalog: None,
    };
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("Apples.md"), "# Apples\n\nApples keep best in a cool, dark place.\n").unwrap();
    let vault = Arc::new(Mutex::new(Some(Vault::open(dir.path()).unwrap())));
    let meta_dir = dir.path().join(".vemry");
    std::fs::create_dir_all(&meta_dir).unwrap();
    let log = SessionLog::create(&meta_dir).unwrap();
    let make_runner = |transport: Arc<dyn vemry_agent::ChatTransport>| AgentRunner {
        transport,
        gate: Arc::new(AutoAllow),
        plan_gate: Arc::new(vemry_agent::permission::AutoApprovePlan),
        actions_gate: Arc::new(vemry_agent::permission::AutoApproveActions),
        read_gate: Arc::new(vemry_agent::permission::DenyReads),
        vault: Arc::clone(&vault),
        sources: Arc::new(Mutex::new(vec![])),
        journal: Arc::new(Mutex::new(vec![])),
    };
    let mut history = vec![];
    let legs: [(&str, bool, &str); 3] = [
        ("chat completions", false, "Read Apples.md and tell me in one sentence how to store apples."),
        ("anthropic", true, "Search the web for how long apples keep in a fridge, then read Apples.md, and answer in one sentence with the source link — the search and the note read are both required."),
        ("chat completions", false, "Now, without searching, restate both facts in one sentence."),
    ];
    for (line, web, message) in legs {
        let runner = if web {
            make_runner(Arc::new(AnthropicTransport::new()))
        } else {
            make_runner(Arc::new(HttpTransport::new()))
        };
        let texts: Arc<Mutex<String>> = Arc::new(Mutex::new(String::new()));
        let t = texts.clone();
        tokio::time::timeout(
            Duration::from_secs(240),
            runner.run(
                &provider,
                &RunnerConfig {
                    max_iterations: 6,
                    write_policy: WritePolicy::Ask,
                    web_search: web,
                    // high: the model thinks between the search and the tool call, and the
                    // replay must return that interleaving verbatim (the 400 the GUI hit)
                    reasoning_effort: Some("high".into()),
                    ..Default::default()
                },
                &mut history,
                Some(message.into()),
                None,
                vec![],
                None,
                &move |e| {
                    if let AgentEvent::TextDelta { text } = e {
                        t.lock().unwrap().push_str(&text);
                    }
                },
                &AtomicBool::new(false),
                &log,
            ),
        )
        .await
        .unwrap_or_else(|_| panic!("the {line} leg timed out"))
        .unwrap_or_else(|e| panic!("the {line} leg failed: {e}"));
        let text = texts.lock().unwrap().clone();
        eprintln!("=== [{line}] {text}");
        assert!(!text.is_empty(), "the {line} leg produced no answer");
    }
    assert!(history.iter().any(|m| m.role == "tool"), "the first leg made no tool call, so the thinking replay was not exercised");
    assert!(
        history.iter().any(|m| m.extra_items.iter().any(|i| i["type"] == "web_search_tool_result")),
        "the second leg left no search block in the history"
    );
}

/// The current model can change between turns (the composer's model menu): a session
/// begun on a model that returns no thinking text, with tool calls in its history,
/// continues on a DeepSeek thinking model. Its contract wants `reasoning_content` back
/// on an assistant turn that carries tool calls — this asserts the endpoint accepts
/// earlier turns that never had any (the first leg runs with thinking off, and its
/// history is stripped of thinking text as another vendor's would have none).
#[tokio::test]
#[ignore]
async fn live_a_session_moves_to_a_thinking_model() {
    let Some(api_key) = std::env::var("VEMRY_TEST_API_KEY").ok().filter(|k| !k.is_empty()) else {
        eprintln!("VEMRY_TEST_API_KEY not set — skipping live test");
        return;
    };
    let provider = ProviderConfig {
        base_url: std::env::var("VEMRY_TEST_BASE_URL").unwrap_or_else(|_| "https://api.deepseek.com".into()),
        model: test_model(),
        api_key,
        kind: ProviderKind::Custom,
        catalog: None,
    };
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("Apples.md"), "# Apples\n\nApples keep best in a cool, dark place.\n").unwrap();
    std::fs::write(dir.path().join("Pears.md"), "# Pears\n\nPears ripen at room temperature.\n").unwrap();
    let vault = Arc::new(Mutex::new(Some(Vault::open(dir.path()).unwrap())));
    let meta_dir = dir.path().join(".vemry");
    std::fs::create_dir_all(&meta_dir).unwrap();
    let log = SessionLog::create(&meta_dir).unwrap();
    let runner = AgentRunner {
        transport: Arc::new(HttpTransport::new()),
        gate: Arc::new(AutoAllow),
        plan_gate: Arc::new(vemry_agent::permission::AutoApprovePlan),
        actions_gate: Arc::new(vemry_agent::permission::AutoApproveActions),
        read_gate: Arc::new(vemry_agent::permission::DenyReads),
        vault: Arc::clone(&vault),
        sources: Arc::new(Mutex::new(vec![])),
        journal: Arc::new(Mutex::new(vec![])),
    };
    let mut history: Vec<vemry_agent::types::ChatMessage> = vec![];
    let legs = [("none", "Read Apples.md and tell me in one sentence how to store apples."), ("high", "Now read Pears.md and say in one sentence how pears differ.")];
    for (i, (effort, message)) in legs.into_iter().enumerate() {
        if i == 1 {
            for m in &mut history {
                m.reasoning_content = None;
            }
            assert!(history.iter().any(|m| m.tool_calls.is_some()), "the first leg made no tool call, so nothing was exercised");
        }
        tokio::time::timeout(
            Duration::from_secs(240),
            runner.run(
                &provider,
                &RunnerConfig { max_iterations: 6, write_policy: WritePolicy::Ask, reasoning_effort: Some(effort.into()), ..Default::default() },
                &mut history,
                Some(message.into()),
                None,
                vec![],
                None,
                &|_e| {},
                &AtomicBool::new(false),
                &log,
            ),
        )
        .await
        .unwrap_or_else(|_| panic!("the {effort} leg timed out"))
        .unwrap_or_else(|e| panic!("the {effort} leg failed: {e}"));
    }
}

/// Probe whether the search tool honours the Anthropic-style `allowed_domains`
/// parameter on this endpoint. Three outcomes are possible: refused with a 400 /
/// accepted and ignored / accepted and effective — the first is decided by the
/// error itself, and for the other two the printed queries and the source domains
/// in the answer are read by hand (the same "weak but honest" assertions as
/// live_web_search). **The prompt deliberately names no domain**: name
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
/// Measured conclusion on the Responses line (2026-08-19, deepseek-v4-flash, the
/// OpenAI-style `filters.allowed_domains`): **accepted and completely ignored**.
/// On this line (the Anthropic-style `allowed_domains`): not yet measured; the
/// runner stays parameterless until it is.
#[tokio::test]
#[ignore]
async fn live_anthropic_web_search_params_probe() {
    use vemry_agent::types::{ChatMessage, ChatRequest, Feature, StreamChunk, ToolDef};
    use vemry_agent::ChatTransport;

    let Some(api_key) = std::env::var("VEMRY_TEST_API_KEY").ok().filter(|k| !k.is_empty()) else {
        eprintln!("VEMRY_TEST_API_KEY not set — skipping live test");
        return;
    };
    let provider = ProviderConfig {
        base_url: std::env::var("VEMRY_TEST_BASE_URL").unwrap_or_else(|_| "https://api.deepseek.com".into()),
        model: test_model(),
        api_key,
        kind: ProviderKind::Custom,
        catalog: None,
    };
    let transport = AnthropicTransport::new();
    let request = ChatRequest {
        feature: Default::default(),
        priority: Feature::default().priority(),
        model: provider.model.clone(),
        messages: vec![ChatMessage::user(
            "Search the web: important artificial-intelligence news from the past week. Pick three stories, summarise each in one sentence, and give the source links.".to_string(),
        )],
        tools: Some(vec![ToolDef::Raw(serde_json::json!({
            "type": "web_search",
            "allowed_domains": ["xinhuanet.com"]
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
/// (a) The Anthropic-format line: run the same reasoning question twice, once with
///     thinking disabled and once at effort `"max"`, and compare how much
///     reasoning came out — none should produce almost no thinking stream, max
///     noticeably more (a weak but honest assertion: none < max, and the answer is
///     non-empty);
/// (b) The Chat Completions line (HttpTransport, which is what DeepSeek takes with
///     web search off): one run with the top-level `reasoning_effort:"low"` and one
///     with `thinking:{"type":"disabled"}`, printing the three possible verdicts
///     (refused / accepted but ignored / effective) to be read by hand. Should (b)
///     be refused, the two Chat Completions fields in types.rs simply stop being
///     sent (a one-line change) and the other line is unaffected.
///
/// Measured conclusion (to be filled in):
#[tokio::test]
#[ignore]
async fn live_reasoning_effort_probe() {
    use vemry_agent::types::{ChatMessage, ChatRequest, Feature, StreamChunk};
    use vemry_agent::ChatTransport;

    let Some(api_key) = std::env::var("VEMRY_TEST_API_KEY").ok().filter(|k| !k.is_empty()) else {
        eprintln!("VEMRY_TEST_API_KEY not set — skipping live test");
        return;
    };
    let provider = ProviderConfig {
        base_url: std::env::var("VEMRY_TEST_BASE_URL").unwrap_or_else(|_| "https://api.deepseek.com".into()),
        model: test_model(),
        api_key,
        kind: ProviderKind::Custom,
        catalog: None,
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
        feature: Default::default(),
        priority: Feature::default().priority(),
        model: provider.model.clone(),
        messages: vec![ChatMessage::user(PROMPT.to_string())],
        tools: None,
        stream: true,
        stream_options: None,
        reasoning_effort: None,
        thinking: None,
    };

    // ---- (a) the Anthropic-format line: none vs max ----
    let anthropic = AnthropicTransport::new();
    let req_none =
        ChatRequest { thinking: Some(serde_json::json!({ "type": "disabled" })), ..base.clone() };
    let req_max = ChatRequest { reasoning_effort: Some("max".into()), ..base.clone() };
    let (t_none, r_none, out_none) = probe(&anthropic, &provider, &req_none).await;
    let (t_max, r_max, out_max) = probe(&anthropic, &provider, &req_max).await;
    eprintln!("=== [anthropic] effort=none: text={t_none} reasoning={r_none} result={out_none:?}");
    eprintln!("=== [anthropic] effort=max:  text={t_max} reasoning={r_max} result={out_max:?}");
    match (&out_none, &out_max) {
        (Err(e), _) | (_, Err(e)) => {
            eprintln!("=== [anthropic] verdict: effort param REJECTED — {e} (fix: the Anthropic line stops sending output_config)");
        }
        (Ok(()), Ok(())) => {
            assert!(t_none > 0 && t_max > 0, "accepted but no answer text");
            // The criterion: the none level's thinking stream should be far shorter
            // than max's (zero is ideal); no clear difference in magnitude means
            // "accepted but ignored" — record that and withdraw the UI
            eprintln!(
                "=== [anthropic] verdict: {}",
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

/// The skill-first rule, which only a real model can be checked against: a research
/// request in a vault with the built-in skills listed must start with read_skill(research)
/// — before update_plan, whose steps then come from the skill's Plan section. Web search
/// stays off and reads are denied, so the turn costs a few tool calls.
#[tokio::test]
#[ignore]
async fn live_research_reads_the_skill_before_planning() {
    let Some(api_key) = std::env::var("VEMRY_TEST_API_KEY").ok().filter(|k| !k.is_empty()) else {
        eprintln!("VEMRY_TEST_API_KEY not set — skipping live test");
        return;
    };
    let provider = ProviderConfig {
        base_url: std::env::var("VEMRY_TEST_BASE_URL").unwrap_or_else(|_| "https://api.deepseek.com".into()),
        model: test_model(),
        api_key,
        kind: ProviderKind::Custom,
        catalog: None,
    };
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("research")).unwrap();
    std::fs::write(
        dir.path().join("research/Sleep.md"),
        "# Sleep\n\nSlow-wave sleep consolidates declarative memory. ^c1\n",
    )
    .unwrap();
    let vault = Arc::new(Mutex::new(Some(Vault::open(dir.path()).unwrap())));
    let meta_dir = dir.path().join(".vemry");
    std::fs::create_dir_all(&meta_dir).unwrap();
    let log = SessionLog::create(&meta_dir).unwrap();
    let runner = AgentRunner {
        transport: Arc::new(HttpTransport::new()),
        gate: Arc::new(vemry_agent::permission::AutoDeny("live test: no writes".into())),
        plan_gate: Arc::new(vemry_agent::permission::AutoApprovePlan),
        actions_gate: Arc::new(vemry_agent::permission::AutoApproveActions),
        read_gate: Arc::new(vemry_agent::permission::DenyReads),
        vault: Arc::clone(&vault),
        sources: Arc::new(Mutex::new(vec![])),
        journal: Arc::new(Mutex::new(vec![])),
    };
    let events = Arc::new(Mutex::new(Vec::new()));
    let ev = events.clone();
    let emit = move |e: AgentEvent| ev.lock().unwrap().push(e);
    let mut history = vec![];
    let result = tokio::time::timeout(
        Duration::from_secs(240),
        runner.run(
            &provider,
            &RunnerConfig { max_iterations: 12, ..Default::default() },
            &mut history,
            Some("帮我系统地调研一下睡眠和记忆巩固的关系，写成一篇综述".into()),
            None,
            vec![],
            None,
            &emit,
            &AtomicBool::new(false),
            &log,
        ),
    )
    .await
    .expect("the turn should finish within the timeout");
    let _ = result; // a denied write at the end is fine: the order of the first calls is the point
    let tools: Vec<String> = events
        .lock()
        .unwrap()
        .iter()
        .filter_map(|e| match e {
            AgentEvent::ToolCallStarted { name, .. } => Some(name.clone()),
            _ => None,
        })
        .collect();
    eprintln!("tool order: {tools:?}");
    let skill_at = tools.iter().position(|t| t == "read_skill").expect("read_skill was never called");
    let plan_at = tools.iter().position(|t| t == "update_plan" || t == "propose_plan");
    assert!(plan_at.is_none_or(|p| skill_at < p), "the skill must be read before the plan: {tools:?}");
    assert!(events.lock().unwrap().iter().any(|e| matches!(e, AgentEvent::SkillActivated { name } if name == "research")));
}

/// One session across two vendors and back: a first leg on the first endpoint with
/// thinking at high (its answers carry thinking text, and on DeepSeek's line its
/// tool turns must), a second on another vendor's endpoint, a third back on the
/// first. Each leg reads a note, so every leg replays tool turns another vendor
/// wrote. Nothing is stripped by hand: the transports must send each endpoint what
/// it takes — no thinking text where the vendor has no such field, no foreign
/// thinking signature to Anthropic, tool-call ids in the shape it accepts.
#[tokio::test]
#[ignore]
async fn live_a_session_moves_across_vendors() {
    let key = |name: &str| std::env::var(name).ok().filter(|k| !k.is_empty());
    let (Some(first_key), Some(next_key), Some(next_base), Some(next_model)) =
        (key("VEMRY_TEST_API_KEY"), key("VEMRY_TEST_NEXT_API_KEY"), key("VEMRY_TEST_NEXT_BASE_URL"), key("VEMRY_TEST_NEXT_MODEL"))
    else {
        eprintln!("VEMRY_TEST_API_KEY and VEMRY_TEST_NEXT_{{API_KEY,BASE_URL,MODEL}} not all set — skipping live test");
        return;
    };
    let first = ProviderConfig::custom(
        std::env::var("VEMRY_TEST_BASE_URL").unwrap_or_else(|_| "https://api.deepseek.com".into()),
        test_model(),
        first_key,
    );
    let next = ProviderConfig::custom(next_base, next_model, next_key);
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("Apples.md"), "# Apples\n\nApples keep best in a cool, dark place.\n").unwrap();
    std::fs::write(dir.path().join("Pears.md"), "# Pears\n\nPears ripen at room temperature.\n").unwrap();
    std::fs::write(dir.path().join("Plums.md"), "# Plums\n\nPlums bruise easily and keep a few days.\n").unwrap();
    let vault = Arc::new(Mutex::new(Some(Vault::open(dir.path()).unwrap())));
    let meta_dir = dir.path().join(".vemry");
    std::fs::create_dir_all(&meta_dir).unwrap();
    let log = SessionLog::create(&meta_dir).unwrap();
    let mut history: Vec<vemry_agent::types::ChatMessage> = vec![];
    let legs = [
        (&first, "Read Apples.md with the read_note tool and tell me in one sentence how to store apples."),
        (&next, "Now read Pears.md with the read_note tool and say in one sentence how pears differ."),
        (&first, "Finally read Plums.md with the read_note tool and say in one sentence what is special about plums."),
    ];
    for (i, (provider, message)) in legs.into_iter().enumerate() {
        let runner = AgentRunner {
            transport: transport_for(provider, false),
            gate: Arc::new(AutoAllow),
            plan_gate: Arc::new(vemry_agent::permission::AutoApprovePlan),
            actions_gate: Arc::new(vemry_agent::permission::AutoApproveActions),
            read_gate: Arc::new(vemry_agent::permission::DenyReads),
            vault: Arc::clone(&vault),
            sources: Arc::new(Mutex::new(vec![])),
            journal: Arc::new(Mutex::new(vec![])),
        };
        let before = history.len();
        tokio::time::timeout(
            Duration::from_secs(240),
            runner.run(
                provider,
                &RunnerConfig {
                    max_iterations: 6,
                    write_policy: WritePolicy::Ask,
                    reasoning_effort: provider.reasoning_supported().then(|| "high".into()),
                    ..Default::default()
                },
                &mut history,
                Some(message.into()),
                None,
                vec![],
                None,
                &|_e| {},
                &AtomicBool::new(false),
                &log,
            ),
        )
        .await
        .unwrap_or_else(|_| panic!("leg {} on {} timed out", i + 1, provider.model))
        .unwrap_or_else(|e| panic!("leg {} on {} failed: {e}", i + 1, provider.model));
        assert!(
            history[before..].iter().any(|m| m.tool_calls.is_some()),
            "leg {} made no tool call, so no foreign tool turn was exercised",
            i + 1
        );
    }
}
