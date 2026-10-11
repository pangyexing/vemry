//! Context compaction (two levels). The first level costs no model call at all:
//! an old successful tool result is replaced by a handle it can be recovered
//! from — the note is on disk, a search result has a URL, and the model can call
//! again when it needs them, which is the structural advantage of a vault as
//! externalised memory. An ERROR result is never cleared (the trace of a failure
//! is what the model corrects its priors against). The second level is an LLM
//! summary triggered by the runner; this module only provides the pure logic
//! (candidate selection, summary replacement, the hard-truncation backstop), all
//! of it testable offline.

use crate::types::ChatMessage;

/// Only tool results longer than this are compacted — a small result saves
/// little and loses information for nothing.
const COMPACT_MIN_CHARS: usize = 500;
/// The most recent N tool results are left alone: the model is very likely about
/// to refer to them.
const PROTECT_RECENT: usize = 4;
/// The prefix marking a compacted result: shared by the idempotence check
/// (already compacted, skip it) and the test assertions.
pub const COMPACTED_MARK: &str = "(compacted:";

/// First-level compaction: turn old tool results into handles and return the
/// number of characters saved. Idempotent — calling it again only processes
/// newly eligible candidates. Skipped: ERROR and cancellation placeholders,
/// anything already compacted, and the most recent PROTECT_RECENT results.
pub fn compact_tool_results(history: &mut [ChatMessage]) -> usize {
    let tool_indices: Vec<usize> =
        (0..history.len()).filter(|&i| history[i].role == "tool").collect();
    let protect_from = tool_indices.len().saturating_sub(PROTECT_RECENT);
    let mut saved = 0usize;
    for &i in &tool_indices[..protect_from] {
        let Some(content) = history[i].text() else { continue };
        if content.len() < COMPACT_MIN_CHARS
            || content.starts_with("ERROR:")
            || content.starts_with("(cancelled")
            || content.starts_with(COMPACTED_MARK)
        {
            continue;
        }
        let call_id = history[i].tool_call_id.clone().unwrap_or_default();
        let (name, arg) = describe_call(&history[..i], &call_id);
        // A skill's text is the method for the whole task and is needed most at its
        // last step (the landing); a few thousand characters kept is cheaper than a
        // note written in the wrong shape
        if name == "read_skill" {
            continue;
        }
        let handle = format!(
            "{COMPACTED_MARK} earlier {name}{arg} result removed to save context — call the tool again if you need it)"
        );
        saved += content.len().saturating_sub(handle.len());
        history[i].content = Some(handle.into());
    }
    saved
}

/// Find this call's tool name and key argument in the preceding
/// assistant(tool_calls) and splice them into the handle, so the model can see at
/// a glance what was dropped and how to get it back.
fn describe_call(history: &[ChatMessage], call_id: &str) -> (String, String) {
    for msg in history.iter().rev() {
        let Some(calls) = &msg.tool_calls else { continue };
        for c in calls {
            if c.id != call_id {
                continue;
            }
            let arg = serde_json::from_str::<serde_json::Value>(&c.function.arguments)
                .ok()
                .and_then(|v| {
                    ["path", "query", "name", "note"].iter().find_map(|k| {
                        v.get(k).and_then(|s| s.as_str()).map(|s| format!(" {s}"))
                    })
                })
                .unwrap_or_default();
            return (c.function.name.clone(), arg);
        }
    }
    ("tool".into(), String::new())
}

/// Heuristic for provider-side "context limit exceeded" errors (the trigger for
/// reactive self-healing).
pub fn is_context_overflow(message: &str) -> bool {
    let m = message.to_lowercase();
    (m.contains("context") && (m.contains("length") || m.contains("window") || m.contains("exceed")))
        || m.contains("maximum context")
        || m.contains("input is too long")
        || m.contains("prompt is too long")
        || m.contains("prompt too long")
        || m.contains("prompt 超长")
        || m.contains("token length too long")
        || m.contains("exceeds the model specification")
        || m.contains("too many tokens")
        || m.contains("exceeds the maximum")
}

/// The instruction for the summary call (second-level compaction). It demands
/// that reasoning progress survive — a text summary must explicitly carry "how
/// far we got and what comes next", or the model loses the task state along with
/// the history.
pub const SUMMARY_INSTRUCTION: &str = "\
Compress the conversation above into a summary someone can keep working from. \
Output the summary itself and nothing else. It must keep:
1. the research question and the user's goal;
2. the conclusions established, each with its source (a URL or a note path);
3. the paths that were tried and failed or were rejected;
4. what is unfinished, how far the reasoning got, and what comes next;
5. the user's explicit instructions, corrections and preferences;
6. the skill (method) in effect, by name, and the steps of its plan not yet done.
You may drop: the conversation verbatim, full tool output, intermediate steps. \
Keep the summary under 500 words.";

/// The index of the last user message in the history (the current turn's
/// envelope). Second-level compaction cuts here: everything before is replaced
/// wholesale by the summary, everything after (the current turn's
/// assistant/tool chain) must survive verbatim — splitting an
/// assistant(tool_calls) from its tool_result earns a 400 from the server.
pub fn last_user_index(history: &[ChatMessage]) -> Option<usize> {
    history.iter().rposition(|m| m.role == "user")
}

/// The cap on source URLs pinned down during compaction: enough to cover the
/// main sources of a piece of research without inflating the summary itself.
const KEEP_SOURCES_MAX: usize = 20;

/// Deterministically collect the source URLs (Markdown links and bare links in
/// the assistant's text) from the prefix about to be replaced.
/// SUMMARY_INSTRUCTION does ask the summary to carry its sources, but that is a
/// best effort; once the search context (extra_items) is dropped along with the
/// compaction, a URL is its only recoverable form, so this has to be guaranteed
/// structurally rather than bet on the model writing the sources into the
/// summary.
fn collect_source_urls(prefix: &[ChatMessage], cap: usize) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    let mut urls = Vec::new();
    for msg in prefix.iter().filter(|m| m.role == "assistant") {
        let Some(text) = msg.text() else { continue };
        let mut pos = 0usize;
        while let Some(at) = text[pos..].find("http") {
            let start = pos + at;
            let rest = &text[start..];
            if !(rest.starts_with("http://") || rest.starts_with("https://")) {
                pos = start + 4;
                continue;
            }
            // A non-ASCII path inside a Markdown link target `](url)` is legal (a
            // Chinese Wikipedia article, say), so it is taken whole — matching the
            // HTTP_TARGET grammar of the frontend's extractCitations; only a bare
            // link uses "stop at the first non-ASCII character" to handle the
            // punctuation or prose pressed against it. In both cases ')' is a
            // delimiter only when there is no matching '(': a Wikipedia
            // disambiguation path `_(...)` must be kept whole, and half a link is
            // worse than no link.
            let in_md_target = text[..start].ends_with("](");
            let mut depth = 0i32;
            let end = rest
                .char_indices()
                .find(|&(_, c)| {
                    if c == '(' {
                        depth += 1;
                        return false;
                    }
                    if c == ')' {
                        if depth > 0 {
                            depth -= 1;
                            return false;
                        }
                        return true;
                    }
                    if c.is_whitespace() || matches!(c, ']' | '>' | '<' | '"' | '\'' | '`') {
                        return true;
                    }
                    !in_md_target && !c.is_ascii()
                })
                .map(|(i, _)| i)
                .unwrap_or(rest.len());
            let url = rest[..end].trim_end_matches(['.', ',', ';', ':', '!', '?']);
            if url.len() > "https://".len() && seen.insert(url.to_string()) {
                urls.push(url.to_string());
                if urls.len() >= cap {
                    return urls;
                }
            }
            pos = start + end;
        }
    }
    urls
}

/// Append the pinned source list after the compaction note (a no-op for an empty
/// list).
fn push_kept_sources(text: &mut String, sources: &[String]) {
    if sources.is_empty() {
        return;
    }
    text.push_str("\n\n(the source links below were kept automatically from the compacted conversation; cite them directly)");
    for u in sources {
        text.push_str("\n- ");
        text.push_str(u);
    }
}

/// Replace the whole history before the current turn with the summary.
/// extra_items (the Responses search context) is dropped with it — what remains
/// recoverable is the summary text plus the URL list pinned down
/// deterministically here (collect_source_urls), which does not depend on the
/// model remembering to write its sources into the summary.
pub fn apply_summary(history: &mut Vec<ChatMessage>, summary: &str) {
    let split = last_user_index(history).unwrap_or(0);
    let sources = collect_source_urls(&history[..split], KEEP_SOURCES_MAX);
    let tail: Vec<ChatMessage> = history.drain(split..).collect();
    history.clear();
    let mut text = format!("(the earlier conversation was compacted; here is the summary — continue from it)\n\n{summary}");
    push_kept_sources(&mut text, &sources);
    history.push(ChatMessage::user(text));
    history.extend(tail);
}

/// The backstop for when the summary call itself fails: keep the current turn
/// only and replace what precedes it with a one-line loss notice. The source URLs
/// are pinned down all the same — a hard truncation loses the detail, and should
/// not also lose what was cited.
pub fn hard_truncate(history: &mut Vec<ChatMessage>) {
    let split = last_user_index(history).unwrap_or(0);
    let sources = collect_source_urls(&history[..split], KEEP_SOURCES_MAX);
    let tail: Vec<ChatMessage> = history.drain(split..).collect();
    history.clear();
    let mut text =
        String::from("(the earlier conversation was truncated because the context limit was exceeded; the detail is lost — search again if you need anything from before.)");
    push_kept_sources(&mut text, &sources);
    history.push(ChatMessage::user(text));
    history.extend(tail);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{FunctionCall, ToolCall};

    fn tool_call(id: &str, name: &str, args: &str) -> ToolCall {
        ToolCall {
            id: id.into(),
            kind: "function".into(),
            function: FunctionCall { name: name.into(), arguments: args.into() },
        }
    }

    fn turn(id: &str, name: &str, args: &str, result: &str) -> Vec<ChatMessage> {
        vec![
            ChatMessage::assistant(None, Some(vec![tool_call(id, name, args)])),
            ChatMessage::tool_result(id, result),
        ]
    }

    #[test]
    fn compacts_old_large_results_keeps_recent_errors_and_small() {
        let big = "x".repeat(2000);
        let mut history = vec![ChatMessage::user("研究一下")];
        history.extend(turn("c1", "read_note", r#"{"path":"projects/Alpha.md"}"#, &big)); // old and large → compacted
        history.extend(turn("c2", "search_notes", r#"{"query":"共识"}"#, &big)); // old and large → compacted
        history.extend(turn("c3", "read_note", r#"{"path":"b.md"}"#, "ERROR: note not found: b.md")); // an error → kept
        history.extend(turn("c4", "read_note", r#"{"path":"c.md"}"#, "short")); // small → kept
        // The four-result protection window: c3/c4 plus the two below
        history.extend(turn("c5", "read_note", r#"{"path":"d.md"}"#, &big));
        history.extend(turn("c6", "read_note", r#"{"path":"e.md"}"#, &big));

        let saved = compact_tool_results(&mut history);
        assert!(saved > 3000, "should save the two old big results, saved={saved}");
        let contents: Vec<&str> =
            history.iter().filter(|m| m.role == "tool").map(|m| m.text().unwrap()).collect();
        assert!(contents[0].starts_with(COMPACTED_MARK));
        assert!(contents[0].contains("read_note projects/Alpha.md"), "handle names the tool+arg: {}", contents[0]);
        assert!(contents[1].contains("search_notes 共识"));
        assert!(contents[2].starts_with("ERROR:"), "errors must survive");
        assert_eq!(contents[3], "short");
        assert!(contents[4].len() > 1000, "recent results protected");
        assert!(contents[5].len() > 1000);

        // Idempotent: compacting again processes nothing already compacted and saves nothing new
        assert_eq!(compact_tool_results(&mut history), 0);
    }

    #[test]
    fn a_read_skill_result_is_never_compacted() {
        let big = "x".repeat(COMPACT_MIN_CHARS + 1);
        let mut history = vec![ChatMessage::user("q")];
        history.extend(turn("s1", "read_skill", r#"{"name":"research"}"#, &big));
        for i in 0..PROTECT_RECENT + 2 {
            history.extend(turn(&format!("c{i}"), "read_note", r#"{"path":"a.md"}"#, &big));
        }
        assert!(compact_tool_results(&mut history) > 0, "the notes were compacted");
        let skill = history.iter().find(|m| m.tool_call_id.as_deref() == Some("s1")).unwrap();
        assert_eq!(skill.text().unwrap(), big, "the skill body stays whole");
        let first_note = history.iter().find(|m| m.tool_call_id.as_deref() == Some("c0")).unwrap();
        assert!(first_note.text().unwrap().starts_with(COMPACTED_MARK));
    }

    #[test]
    fn overflow_detection() {
        assert!(is_context_overflow("HTTP 400: This model's maximum context length is 131072 tokens"));
        assert!(is_context_overflow("context_length_exceeded"));
        assert!(is_context_overflow("Input is too long for this model"));
        // Zhipu's (code 1261, both sites) and Moonshot's wordings
        assert!(is_context_overflow(r#"HTTP 400: {"error":{"code":"1261","message":"Prompt 超长"}}"#));
        assert!(is_context_overflow(r#"HTTP 400: {"error":{"code":"1261","message":"Prompt too long"}}"#));
        assert!(is_context_overflow("Invalid request: Input token length too long"));
        assert!(is_context_overflow("prompt tokens + max_tokens exceeds the model specification"));
        // Anthropic's wording
        assert!(is_context_overflow(r#"HTTP 400 Bad Request: {"type":"error","error":{"type":"invalid_request_error","message":"prompt is too long: 1048577 tokens > 1000000 maximum"}}"#));
        assert!(!is_context_overflow("HTTP 401: invalid api key"));
        assert!(!is_context_overflow("rate limit exceeded"));
    }

    #[test]
    fn summary_replaces_prefix_keeps_current_turn_intact() {
        let mut history = vec![
            ChatMessage::user("第一问"),
            ChatMessage::assistant(Some("第一答".into()), None),
            ChatMessage::user("当前问"),
            ChatMessage::assistant(None, Some(vec![tool_call("c1", "read_note", "{}")])),
            ChatMessage::tool_result("c1", "结果"),
        ];
        apply_summary(&mut history, "总结内容");
        assert_eq!(history.len(), 4);
        assert!(history[0].text().unwrap().contains("总结内容"));
        assert_eq!(history[1].text(), Some("当前问"));
        // The current turn's tool_calls / tool_result pairing survives verbatim
        assert!(history[2].tool_calls.is_some());
        assert_eq!(history[3].tool_call_id.as_deref(), Some("c1"));
    }

    #[test]
    fn hard_truncate_keeps_only_current_turn() {
        let mut history = vec![
            ChatMessage::user("旧"),
            ChatMessage::assistant(Some("旧答".into()), None),
            ChatMessage::user("新"),
        ];
        hard_truncate(&mut history);
        assert_eq!(history.len(), 2);
        assert!(history[0].text().unwrap().contains("truncated"));
        assert_eq!(history[1].text(), Some("新"));
    }

    #[test]
    fn summary_preserves_source_urls_from_compacted_prefix() {
        let mut history = vec![
            ChatMessage::user("查一下"),
            ChatMessage::assistant(
                Some(
                    "据 [报道](https://example.com/a) 与裸链 https://news.cn/b。重复引用 https://example.com/a 不重列。"
                        .into(),
                ),
                None,
            ),
            ChatMessage::user("当前问"),
        ];
        apply_summary(&mut history, "总结内容");
        let head = history[0].text().unwrap();
        assert!(head.contains("总结内容"));
        assert!(head.contains("- https://example.com/a"), "a Markdown link must be pinned down: {head}");
        assert!(head.contains("- https://news.cn/b"), "a bare link must be pinned down: {head}");
        // A CJK full stop is not swallowed into the URL; one URL appears once in the list
        assert!(!head.contains("https://news.cn/b。"), "{head}");
        assert_eq!(head.matches("- https://example.com/a").count(), 1);
        // The current turn takes no part in the collection and is not replaced
        assert_eq!(history[1].text(), Some("当前问"));
    }

    #[test]
    fn summary_keeps_cjk_and_parenthesized_urls_whole() {
        let mut history = vec![
            ChatMessage::user("查一下"),
            ChatMessage::assistant(
                Some(
                    "见 [人工智能](https://zh.wikipedia.org/wiki/人工智能) 与裸链 \
                     https://en.wikipedia.org/wiki/Rust_(programming_language) 的说明。"
                        .into(),
                ),
                None,
            ),
            ChatMessage::user("当前问"),
        ];
        apply_summary(&mut history, "总结内容");
        let head = history[0].text().unwrap();
        // A non-ASCII path in a Markdown target survives whole and is not cut at the first CJK character
        assert!(head.contains("- https://zh.wikipedia.org/wiki/人工智能"), "{head}");
        assert!(!head.contains("- https://zh.wikipedia.org/wiki/\n"), "{head}");
        // Wikipedia-style balanced parentheses are not cut in half
        assert!(
            head.contains("- https://en.wikipedia.org/wiki/Rust_(programming_language)"),
            "{head}"
        );
    }

    #[test]
    fn summary_without_urls_adds_no_source_section() {
        let mut history = vec![
            ChatMessage::user("问"),
            ChatMessage::assistant(Some("纯文字回答,没有链接".into()), None),
            ChatMessage::user("当前问"),
        ];
        apply_summary(&mut history, "总结");
        let head = history[0].text().unwrap();
        assert!(!head.contains("kept automatically"), "{head}");
    }

    #[test]
    fn hard_truncate_also_keeps_sources() {
        let mut history = vec![
            ChatMessage::user("旧"),
            ChatMessage::assistant(Some("见 https://example.com/x".into()), None),
            ChatMessage::user("新"),
        ];
        hard_truncate(&mut history);
        let head = history[0].text().unwrap();
        assert!(head.contains("- https://example.com/x"), "{head}");
    }
}
