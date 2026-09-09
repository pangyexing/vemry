//! The offline evaluation suite: every `tests/evals/*.json` fixture builds a small
//! vault, replays scripted model output through the real runner (tools, gates, the
//! source table, the audit log) and checks the product standards that can be
//! checked without a provider — citations resolve, sources are settled with the
//! right verdict, the plan is state rather than prose, tools succeed, no write
//! slips past the gate. A fixture is data: adding a case is a JSON file.
//!
//! Fixture shape (see the files for examples):
//! ```json
//! { "name": "…", "vault": { "path.md": "content" }, "plan_confirm": false, "follow": "note.md",
//!   "turns": [ { "user": "…", "page_url": "…", "skill": "research", "script": [ [chunk…], [chunk…] ] } ],
//!   "expect": { "citations_resolve": true, "max_tool_calls": 2, "tools_ok": true,
//!               "no_writes": true, "sources": [{"url": "…", "verified": false}],
//!               "plan": ["done", "doing"], "no_plan_in_prose": true,
//!               "tool_output_contains": [{"id": "c1", "text": "…"}],
//!               "answer_contains": ["…"], "note_contains": [{"path": "…", "text": "…"}],
//!               "skill_events": ["research"], "skill_in_envelope": "research" } }
//! ```
//! Writes go through an auto-denying gate, so a fixture that expects a write to land
//! must set `follow` (the followed note takes its local writes without a dialog) or
//! `allow_writes` (every write allowed, for moves and other non-local writes).
//! ```
//! ```
//! A chunk is one of `{"text": "…"}`, `{"finish": "stop"}`,
//! `{"tool": {"id", "name", "args"}}`, `{"search": ["query"]}`,
//! `{"results": [{"title", "url"}]}`.

use std::path::Path;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};

use serde_json::Value;
use vemry_agent::permission::{AutoApproveActions, AutoApprovePlan, AutoDeny, FetchedPage, ReadDecision, ReadGate, ReadRequest};

/// The eval's read gate: the fixture's `pages` map (url → markdown) is what the user
/// "allowed"; a URL not in it fails, so a fixture states exactly what may be read.
struct PagesFromFixture(Value);

#[async_trait::async_trait]
impl ReadGate for PagesFromFixture {
    async fn request(&self, req: &ReadRequest) -> ReadDecision {
        let mut pages = vec![];
        let mut failed = vec![];
        for u in &req.urls {
            match self.0.get(u).and_then(|v| v.as_str()) {
                Some(md) => pages.push(FetchedPage { requested: u.clone(), url: u.clone(), title: format!("Page {}", pages.len() + 1), markdown: md.into() }),
                None => failed.push(u.clone()),
            }
        }
        ReadDecision::Pages { pages, failed }
    }
}
use vemry_agent::testing::*;
use vemry_agent::types::{StreamChunk, WebSearchResult};
use vemry_agent::{AgentEvent, AgentRunner, PageContext, ProviderConfig, RunnerConfig};
use vemry_agent::session::SessionLog;
use vemry_core::{RefResolution, Vault};

fn chunk_of(v: &Value, index: usize) -> StreamChunk {
    if let Some(t) = v.get("text").and_then(|t| t.as_str()) {
        return text_chunk(t);
    }
    if let Some(f) = v.get("finish").and_then(|t| t.as_str()) {
        return finish_chunk(f);
    }
    if let Some(tc) = v.get("tool") {
        let args = tc.get("args").map(|a| if a.is_string() { a.as_str().unwrap().to_string() } else { a.to_string() });
        return tool_call_chunk(
            index,
            tc["id"].as_str().expect("tool.id"),
            tc["name"].as_str().expect("tool.name"),
            &args.unwrap_or_else(|| "{}".into()),
        );
    }
    if let Some(q) = v.get("search").and_then(|q| q.as_array()) {
        let qs: Vec<&str> = q.iter().filter_map(|x| x.as_str()).collect();
        return search_queries_chunk(&qs);
    }
    if let Some(r) = v.get("results").and_then(|r| r.as_array()) {
        return search_results_chunk(
            r.iter()
                .map(|x| WebSearchResult {
                    title: x["title"].as_str().unwrap_or("").into(),
                    url: x["url"].as_str().unwrap_or("").into(),
                    snippet: String::new(),
                    publish_date: None,
                })
                .collect(),
        );
    }
    panic!("unknown chunk: {v}");
}

/// A scripted response: tool calls are numbered in order, and a script that holds
/// tool calls but no finish gets `tool_calls` appended, a text-only one `stop`.
fn script_of(v: &Value) -> Vec<StreamChunk> {
    let arr = v.as_array().expect("a script is an array of chunks");
    let mut tool_index = 0;
    let mut out = Vec::new();
    let mut has_finish = false;
    let mut has_tool = false;
    for c in arr {
        if c.get("tool").is_some() {
            out.push(chunk_of(c, tool_index));
            tool_index += 1;
            has_tool = true;
        } else {
            has_finish |= c.get("finish").is_some();
            out.push(chunk_of(c, 0));
        }
    }
    if !has_finish {
        out.push(finish_chunk(if has_tool { "tool_calls" } else { "stop" }));
    }
    out
}

/// The wiki citations `[[note#frag]]` in a text (note without .md; frag may carry a caret).
fn wiki_citations(text: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut pos = 0;
    while let Some(at) = text[pos..].find("[[") {
        let start = pos + at + 2;
        let Some(end_rel) = text[start..].find("]]") else { break };
        let inner = &text[start..start + end_rel];
        let inner = inner.split('|').next().unwrap_or(inner);
        if let Some((note, frag)) = inner.split_once('#') {
            out.push((note.trim().to_string(), frag.trim().to_string()));
        }
        pos = start + end_rel + 2;
    }
    out
}

struct Outcome {
    events: Vec<AgentEvent>,
    answers: Vec<String>,
    sources: Vec<vemry_agent::Source>,
    jsonl: String,
}

async fn run_fixture(fx: &Value) -> Outcome {
    let dir = tempfile::tempdir().unwrap();
    for (path, content) in fx["vault"].as_object().into_iter().flatten() {
        let abs = dir.path().join(path);
        std::fs::create_dir_all(abs.parent().unwrap()).unwrap();
        std::fs::write(abs, content.as_str().expect("vault file content")).unwrap();
    }
    let vault = Arc::new(Mutex::new(Some(Vault::open(dir.path()).unwrap())));
    let scripts: Vec<Vec<StreamChunk>> = fx["turns"]
        .as_array()
        .expect("turns")
        .iter()
        .flat_map(|t| t["script"].as_array().expect("script").iter().map(script_of))
        .collect();
    let transport = Arc::new(ScriptedTransport::new(scripts));
    let runner = AgentRunner {
        transport: transport.clone(),
        gate: Arc::new(AutoDeny("evals never write".into())),
        plan_gate: Arc::new(AutoApprovePlan),
        actions_gate: Arc::new(AutoApproveActions),
        read_gate: Arc::new(PagesFromFixture(fx["pages"].clone())),
        vault: vault.clone(),
        sources: Arc::new(Mutex::new(vec![])),
        journal: Arc::new(Mutex::new(vec![])),
    };
    let log_dir = tempfile::tempdir().unwrap();
    let log = SessionLog::create(log_dir.path()).unwrap();
    let events = Arc::new(Mutex::new(Vec::new()));
    let ev = events.clone();
    let emit = move |e: AgentEvent| ev.lock().unwrap().push(e);
    let config = RunnerConfig {
        plan_confirm: fx["plan_confirm"].as_bool().unwrap_or(false),
        follow_note: fx["follow"].as_str().map(String::from),
        write_policy: if fx["allow_writes"].as_bool().unwrap_or(false) { vemry_agent::WritePolicy::Allow } else { vemry_agent::WritePolicy::Ask },
        ..Default::default()
    };
    let provider = ProviderConfig::zhipu("eval-key".into());
    let mut history = vec![];
    for turn in fx["turns"].as_array().unwrap() {
        let page = turn["page_url"].as_str().map(|u| PageContext {
            url: u.into(),
            title: "page".into(),
            markdown: turn["page_markdown"].as_str().unwrap_or("page body").into(),
        });
        // `skill`: the user chose one in the composer — the same reference the chip sends
        let refs: Vec<vemry_agent::ContextRef> =
            turn["skill"].as_str().map(|s| vemry_agent::ContextRef::Skill { path: s.into() }).into_iter().collect();
        runner
            .run(
                &provider,
                &config,
                &mut history,
                Some(turn["user"].as_str().expect("user").into()),
                page,
                refs,
                None,
                &emit,
                &AtomicBool::new(false),
                &log,
            )
            .await
            .unwrap_or_else(|e| panic!("{}: the turn failed: {e}", fx["name"]));
    }
    assert_eq!(transport.remaining(), 0, "{}: every scripted response must be consumed", fx["name"]);
    drop(emit);
    let events = Arc::try_unwrap(events).unwrap().into_inner().unwrap();
    let answers: Vec<String> =
        history.iter().filter(|m| m.role == "assistant").filter_map(|m| m.text().map(str::to_string)).collect();
    let sources = runner.sources.lock().unwrap().clone();
    let jsonl = vemry_agent::session::read_session(log_dir.path(), &log.id).unwrap();
    // the vault is needed for the citation check; keep the tempdir alive by leaking it into the outcome
    let vault_dir = dir;
    let vault_guard = vault.lock().unwrap();
    let v = vault_guard.as_ref().unwrap();
    check(fx, v, &events, &answers, &sources, &jsonl, &history);
    if let Some(checks) = fx["expect"]["note_contains"].as_array() {
        for c in checks {
            let path = c["path"].as_str().unwrap();
            let text = c["text"].as_str().unwrap();
            let on_disk = std::fs::read_to_string(vault_dir.path().join(path)).unwrap_or_default();
            assert!(on_disk.contains(text), "[{}] note {path} lacks {text:?}:\n{on_disk}", fx["name"]);
        }
    }
    if let Some(paths) = fx["expect"]["exists"].as_array() {
        for p in paths.iter().filter_map(|p| p.as_str()) {
            assert!(vault_dir.path().join(p).exists(), "[{}] {p} should exist", fx["name"]);
        }
    }
    if let Some(paths) = fx["expect"]["gone"].as_array() {
        for p in paths.iter().filter_map(|p| p.as_str()) {
            assert!(!vault_dir.path().join(p).exists(), "[{}] {p} should be gone", fx["name"]);
        }
    }
    drop(vault_guard);
    drop(vault_dir);
    Outcome { events, answers, sources, jsonl }
}

/// One failure wording for every check: the fixture's name first, then what went wrong.
fn fail(name: &str, what: &str) -> ! {
    panic!("[{name}] {what}")
}

fn check(
    fx: &Value,
    vault: &Vault,
    events: &[AgentEvent],
    answers: &[String],
    sources: &[vemry_agent::Source],
    jsonl: &str,
    history: &[vemry_agent::types::ChatMessage],
) {
    let name = fx["name"].as_str().unwrap_or("?");
    let ex = &fx["expect"];
    let fail = |what: &str| -> ! { fail(name, what) };

    if ex["citations_resolve"].as_bool().unwrap_or(false) {
        for text in answers {
            for (note, frag) in wiki_citations(text) {
                let path = if note.ends_with(".md") { note.clone() } else { format!("{note}.md") };
                let id = vault.note_id(&path).unwrap_or_else(|e| fail(&format!("citation note {note}: {e}")));
                let clean = frag.trim_start_matches('^');
                let ok = if vemry_core::blockid::is_valid_block_id(clean) && !frag.contains(' ') {
                    matches!(vault.block_for_agent(&id, clean), Ok(Some(_)))
                } else {
                    matches!(vault.resolve_ref_for_agent(&format!("{note}#{frag}"), None), Ok(RefResolution::Resolved(_)))
                };
                if !ok {
                    fail(&format!("citation [[{note}#{frag}]] does not resolve"));
                }
            }
        }
    }
    if let Some(max) = ex["max_tool_calls"].as_u64() {
        let n = events.iter().filter(|e| matches!(e, AgentEvent::ToolCallStarted { .. })).count() as u64;
        if n > max {
            fail(&format!("{n} tool calls, more than {max}"));
        }
    }
    if ex["tools_ok"].as_bool().unwrap_or(false) {
        for e in events {
            if let AgentEvent::ToolCallFinished { name: tool, ok: false, output, .. } = e {
                fail(&format!("tool {tool} failed: {output}"));
            }
        }
    }
    if ex["no_writes"].as_bool().unwrap_or(false) && jsonl.contains(r#""kind":"write_prepared""#) {
        fail("a write reached the gate");
    }
    if let Some(expected) = ex["sources"].as_array() {
        if expected.len() != sources.len() {
            fail(&format!("expected {} sources, table has {:?}", expected.len(), sources));
        }
        for want in expected {
            let url = want["url"].as_str().unwrap();
            let s = sources.iter().find(|s| s.url == url).unwrap_or_else(|| fail(&format!("source {url} missing from {sources:?}")));
            if let Some(v) = want["verified"].as_bool() {
                if s.verified != v {
                    fail(&format!("source {url}: verified={} expected {v}", s.verified));
                }
            }
        }
    }
    if let Some(statuses) = ex["plan"].as_array() {
        let last = events.iter().rev().find_map(|e| match e {
            AgentEvent::PlanUpdated { items } => Some(items.clone()),
            _ => None,
        });
        let Some(items) = last else { fail("no plan was published") };
        let got: Vec<String> = items.iter().map(|i| serde_json::to_value(i.status).unwrap().as_str().unwrap().to_string()).collect();
        let want: Vec<&str> = statuses.iter().filter_map(|s| s.as_str()).collect();
        if got != want {
            fail(&format!("plan statuses {got:?}, expected {want:?}"));
        }
    }
    if ex["no_plan_in_prose"].as_bool().unwrap_or(false) {
        for text in answers {
            let lower = text.to_lowercase();
            if lower.contains("## plan") || text.contains("## 计划") || text.lines().any(|l| l.trim_start().starts_with("- [ ]") || l.trim_start().starts_with("- [x]")) {
                fail(&format!("the plan leaked into the prose: {text}"));
            }
        }
    }
    if let Some(checks) = ex["tool_output_contains"].as_array() {
        for c in checks {
            let id = c["id"].as_str().unwrap();
            let text = c["text"].as_str().unwrap();
            let found = events.iter().any(|e| matches!(e, AgentEvent::ToolCallFinished { id: eid, output, .. } if eid == id && output.contains(text)));
            if !found {
                fail(&format!("tool {id} output lacks {text:?}"));
            }
        }
    }
    if let Some(needles) = ex["answer_contains"].as_array() {
        let all = answers.join("\n");
        for n in needles.iter().filter_map(|n| n.as_str()) {
            if !all.contains(n) {
                fail(&format!("answers lack {n:?}"));
            }
        }
    }
    // The skills that came into effect, in order — attached by the user or read by the
    // model — as the events and the ledger both say
    if let Some(names) = ex["skill_events"].as_array() {
        let want: Vec<&str> = names.iter().filter_map(|n| n.as_str()).collect();
        let got: Vec<&str> = events
            .iter()
            .filter_map(|e| match e {
                AgentEvent::SkillActivated { name } => Some(name.as_str()),
                _ => None,
            })
            .collect();
        if got != want {
            fail(&format!("skill events {got:?}, expected {want:?}"));
        }
        for n in &want {
            if !jsonl.contains(&format!(r#""kind":"skill","name":"{n}""#)) && !jsonl.contains(&format!(r#""name":"{n}""#)) {
                fail(&format!("the ledger has no skill line for {n}"));
            }
        }
    }
    // An attached skill's text rides in the user envelope of the last turn (so the plan
    // can come from it, and no compaction of tool results can take it away)
    if let Some(name) = ex["skill_in_envelope"].as_str() {
        let Some(user) = history.iter().rev().find(|m| m.role == "user").and_then(|m| m.text()) else {
            fail("no user envelope in the history")
        };
        let open = format!("<skill-context path=\"{name}\">");
        if !user.contains(&open) || !user.contains("</skill-context>") {
            fail(&format!("the user envelope carries no skill-context for {name}:\n{user}"));
        }
        if !user.contains("## Plan") && !user.contains("# ") {
            fail("the skill envelope has no body");
        }
        let plan_pos = user.find(&open).unwrap();
        let first_material = ["<note-context", "<selection-context", "<web-page-context", "<block-context", "<folder-context", "<file-context"]
            .iter()
            .filter_map(|t| user.find(t))
            .min();
        if first_material.is_some_and(|m| m < plan_pos) {
            fail("the skill envelope must come before the material");
        }
    }
}

#[tokio::test]
async fn offline_evals() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/evals");
    let mut files: Vec<_> = std::fs::read_dir(&dir)
        .expect("tests/evals exists")
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "json"))
        .collect();
    files.sort();
    assert!(!files.is_empty(), "no eval fixtures found");
    for f in files {
        let fx: Value = serde_json::from_str(&std::fs::read_to_string(&f).unwrap()).unwrap_or_else(|e| panic!("{}: {e}", f.display()));
        let out = run_fixture(&fx).await;
        eprintln!(
            "eval {:<28} ok  tools={} sources={} answers={}",
            fx["name"].as_str().unwrap_or("?"),
            out.events.iter().filter(|e| matches!(e, AgentEvent::ToolCallStarted { .. })).count(),
            out.sources.len(),
            out.answers.len()
        );
        let _ = &out.jsonl;
    }
}
