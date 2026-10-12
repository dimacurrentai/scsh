//! The transcript of a headless harness run, normalized from the CLI's own event stream.
//!
//! `claude-headless` (`claude -p --output-format stream-json --verbose`), `codex-headless`
//! (`codex exec --json`), and `cline-headless` (`cline --json`) each print one JSON event per
//! line, in three unrelated vocabularies. This module reads any of them into one short list of
//! [`Entry`] values — what the agent said, what it thought, which tools it called with what
//! arguments and what came back, and how the run ended — which the session browser renders as
//! the run's body in place of a terminal recording, and which failure classification reads
//! instead of scraping a screen.
//!
//! Token accounting deliberately does NOT come from here: every headless run is accounted from
//! the same native session files its agent's TUI run writes (see `daemon::usage`), so the two
//! interfaces report directly comparable numbers. The `Result` entry only echoes what the CLI
//! itself printed at the end.

use crate::config::Harness;
use crate::json::{self, Value};

/// Tool output beyond this many characters is cut (the full stream stays in the log).
const MAX_BODY_CHARS: usize = 6000;

/// One normalized step of a headless run.
#[derive(Debug, Clone, PartialEq)]
pub enum Entry {
  /// The CLI's opening event: model, working directory, session id.
  Session { detail: String },
  /// Prose the agent wrote to the user.
  Text { text: String },
  /// Reasoning the CLI chose to expose.
  Thinking { text: String },
  /// One tool call, paired with its output once that arrives. `ok` is `None` while running.
  Tool { id: Option<String>, name: String, summary: String, input: String, output: Option<String>, ok: Option<bool> },
  /// How the run ended, as the CLI reported it.
  Result { ok: bool, detail: String },
  /// An error event (or a failed result's message).
  Error { text: String },
  /// A line that is not a JSON event: CLI warnings printed to stdout.
  Note { text: String },
}

/// A parsed transcript: the entries plus the facts failure classification needs.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Transcript {
  pub entries: Vec<Entry>,
  /// The CLI printed its terminal event (a `result`, `turn.completed`, or `run_result`).
  pub finished: bool,
  /// The message of the last error the stream reported, if any.
  pub error: Option<String>,
}

impl Transcript {
  /// Text for failure classification: the reported error, then every error and note entry.
  /// Classifiers match on provider prose (auth refusals, usage limits) wherever it surfaced.
  pub fn failure_text(&self) -> String {
    let mut out: Vec<&str> = self.error.as_deref().into_iter().collect();
    for entry in &self.entries {
      match entry {
        Entry::Error { text } | Entry::Note { text } => out.push(text),
        Entry::Result { ok: false, detail } => out.push(detail),
        _ => {}
      }
    }
    out.join("\n")
  }
}

/// Parse a headless harness's event stream. Unknown events are skipped, so a CLI that adds an
/// event type still renders everything this module understands. Non-headless harnesses have
/// no event stream and yield an empty transcript.
pub fn parse(harness: Harness, stream: &str) -> Transcript {
  let mut t = Transcript::default();
  for line in stream.lines() {
    let line = line.trim();
    if line.is_empty() {
      continue;
    }
    let event = match json::parse(line) {
      Ok(event @ Value::Object(_)) => event,
      _ => {
        t.entries.push(Entry::Note { text: clip(line) });
        continue;
      }
    };
    match harness {
      Harness::ClaudeHeadless => claude_event(&mut t, &event),
      Harness::CodexHeadless => codex_event(&mut t, &event),
      Harness::ClineHeadless => cline_event(&mut t, &event),
      _ => {}
    }
  }
  t
}

/// `claude -p --output-format stream-json --verbose`: `system`/`init`, then `assistant` and
/// `user` messages carrying Anthropic content blocks, then one `result`.
fn claude_event(t: &mut Transcript, event: &Value) {
  match string(event, "type") {
    Some("system") if string(event, "subtype") == Some("init") => {
      let mut parts = Vec::new();
      if let Some(model) = string(event, "model") {
        parts.push(format!("model {model}"));
      }
      if let Some(Value::Array(tools)) = field(event, "tools") {
        parts.push(format!("{} tools", tools.len()));
      }
      if let Some(session) = string(event, "session_id") {
        parts.push(format!("session {session}"));
      }
      t.entries.push(Entry::Session { detail: parts.join(" · ") });
    }
    Some("assistant") => {
      for block in blocks(field(event, "message").and_then(|m| field(m, "content"))) {
        match string(block, "type") {
          Some("text") => push_text(t, string(block, "text")),
          Some("thinking") => push_thinking(t, string(block, "thinking")),
          Some("tool_use") => {
            let input = field(block, "input").cloned().unwrap_or(Value::Null);
            t.entries.push(Entry::Tool {
              id: string(block, "id").map(str::to_string),
              name: string(block, "name").unwrap_or("tool").to_string(),
              summary: input_summary(&input),
              input: clip(&json::write_pretty(&input)),
              output: None,
              ok: None,
            });
          }
          _ => {}
        }
      }
    }
    Some("user") => {
      for block in blocks(field(event, "message").and_then(|m| field(m, "content"))) {
        if string(block, "type") == Some("tool_result") {
          let ok = !matches!(field(block, "is_error"), Some(Value::Bool(true)));
          let output = content_text(field(block, "content"));
          complete_tool(t, string(block, "tool_use_id"), output, ok);
        }
      }
    }
    Some("result") => {
      t.finished = true;
      let failed = matches!(field(event, "is_error"), Some(Value::Bool(true)))
        || string(event, "subtype").is_some_and(|s| s != "success");
      // `subtype` stays `success` on an API error; `is_error` is what says it failed.
      let subtype = string(event, "subtype").unwrap_or("result").replace('_', " ");
      let mut parts = vec![if failed && subtype == "success" { "error".to_string() } else { subtype }];
      if let Some(turns) = number(event, "num_turns") {
        parts.push(format!("{turns} turn{}", if turns == 1.0 { "" } else { "s" }));
      }
      if let Some(ms) = number(event, "duration_ms") {
        parts.push(format!("{:.1}s", ms / 1000.0));
      }
      if let Some(cost) = number(event, "total_cost_usd") {
        parts.push(format!("${cost:.4}"));
      }
      if failed {
        let message = string(event, "result").unwrap_or("the CLI reported an error").to_string();
        t.error = Some(message.clone());
        parts.push(message);
      }
      t.entries.push(Entry::Result { ok: !failed, detail: parts.join(" · ") });
    }
    _ => {}
  }
}

/// `codex exec --json`: `thread.started`, `turn.started`, `item.started`/`item.completed`
/// for messages, reasoning, commands, file changes, MCP calls, and searches, then
/// `turn.completed` (or `turn.failed` / `error`).
fn codex_event(t: &mut Transcript, event: &Value) {
  match string(event, "type") {
    Some("thread.started") => {
      let detail = string(event, "thread_id").map(|id| format!("thread {id}")).unwrap_or_default();
      t.entries.push(Entry::Session { detail });
    }
    Some(kind @ ("item.started" | "item.completed")) => {
      let Some(item) = field(event, "item") else { return };
      let done = kind == "item.completed";
      let id = string(item, "id").map(str::to_string);
      match string(item, "type") {
        Some("agent_message") if done => push_text(t, string(item, "text")),
        Some("reasoning") if done => push_thinking(t, string(item, "text")),
        Some("command_execution") => {
          let command = string(item, "command").unwrap_or("").to_string();
          if !done {
            t.entries.push(tool_entry(id, "shell", &command, &command));
            return;
          }
          let output = string(item, "aggregated_output").unwrap_or("").to_string();
          let ok = number(item, "exit_code").is_none_or(|code| code == 0.0)
            && string(item, "status").is_none_or(|status| status == "completed");
          if !t.entries.iter().any(|e| matches!(e, Entry::Tool { id: tid, .. } if *tid == id && id.is_some())) {
            t.entries.push(tool_entry(id.clone(), "shell", &command, &command));
          }
          complete_tool(t, id.as_deref(), output, ok);
        }
        Some("file_change") if done => {
          let changes: Vec<String> = match field(item, "changes") {
            Some(Value::Array(changes)) => changes
              .iter()
              .map(|c| format!("{} {}", string(c, "kind").unwrap_or("update"), string(c, "path").unwrap_or("?")))
              .collect(),
            _ => Vec::new(),
          };
          let summary = changes.join(", ");
          t.entries.push(tool_entry(id.clone(), "edit", &summary, &changes.join("\n")));
          let ok = string(item, "status").is_none_or(|status| status == "completed");
          complete_tool(t, id.as_deref(), String::new(), ok);
        }
        Some("mcp_tool_call") if done => {
          let name = format!("{}.{}", string(item, "server").unwrap_or("mcp"), string(item, "tool").unwrap_or("tool"));
          let input = field(item, "arguments").cloned().unwrap_or(Value::Null);
          t.entries.push(tool_entry(id.clone(), &name, &input_summary(&input), &json::write_pretty(&input)));
          let output = field(item, "result").map(json::write_pretty).unwrap_or_default();
          let ok = string(item, "status").is_none_or(|status| status == "completed") && field(item, "error").is_none();
          complete_tool(t, id.as_deref(), output, ok);
        }
        Some("web_search") if done => {
          let query = string(item, "query").unwrap_or("").to_string();
          t.entries.push(tool_entry(id.clone(), "web search", &query, &query));
          complete_tool(t, id.as_deref(), String::new(), true);
        }
        Some("error") if done => push_error(t, string(item, "message")),
        _ => {}
      }
    }
    Some("turn.completed") => {
      t.finished = true;
      let detail = field(event, "usage").map(codex_usage_detail).unwrap_or_else(|| "turn completed".into());
      t.entries.push(Entry::Result { ok: true, detail });
    }
    Some("turn.failed") => {
      t.finished = true;
      let message = field(event, "error").and_then(|e| string(e, "message")).unwrap_or("turn failed").to_string();
      t.error = Some(message.clone());
      t.entries.push(Entry::Result { ok: false, detail: message });
    }
    Some("error") => push_error(t, string(event, "message")),
    _ => {}
  }
}

fn codex_usage_detail(usage: &Value) -> String {
  let mut parts = vec!["turn completed".to_string()];
  for (key, label) in [("input_tokens", "input"), ("cached_input_tokens", "cached"), ("output_tokens", "output")] {
    if let Some(n) = number(usage, key) {
      parts.push(format!("{label} {n}"));
    }
  }
  parts.join(" · ")
}

/// `cline --json`: `agent_event` wrappers around `content_start`/`content_end` blocks for text,
/// reasoning, and tools, plus `done` and one closing `run_result`. `hook_event` lines only
/// mirror the agent events and are skipped.
fn cline_event(t: &mut Transcript, event: &Value) {
  match string(event, "type") {
    Some("agent_event") => {
      let Some(inner) = field(event, "event") else { return };
      match (string(inner, "type"), string(inner, "contentType")) {
        (Some("content_start"), Some("tool")) => {
          let input = field(inner, "input").cloned().unwrap_or(Value::Null);
          t.entries.push(Entry::Tool {
            id: string(inner, "toolCallId").map(str::to_string),
            name: string(inner, "toolName").unwrap_or("tool").to_string(),
            summary: input_summary(&input),
            input: clip(&json::write_pretty(&input)),
            output: None,
            ok: None,
          });
        }
        (Some("content_end"), Some("tool")) => {
          let output = field(inner, "output");
          let ok = match output {
            Some(Value::Array(results)) => {
              results.iter().all(|r| !matches!(field(r, "success"), Some(Value::Bool(false))))
            }
            _ => field(inner, "error").is_none(),
          };
          let text = match output {
            Some(Value::Array(results)) => results
              .iter()
              .map(|r| match (string(r, "result"), string(r, "error")) {
                (_, Some(error)) => error.to_string(),
                (Some(result), _) => result.to_string(),
                _ => json::write(r),
              })
              .collect::<Vec<_>>()
              .join("\n"),
            Some(Value::String(s)) => s.clone(),
            Some(other) => json::write_pretty(other),
            None => string(inner, "error").unwrap_or("").to_string(),
          };
          complete_tool(t, string(inner, "toolCallId"), text, ok);
        }
        (Some("content_end"), Some("text")) => push_text(t, string(inner, "text")),
        (Some("content_end"), Some("reasoning" | "thinking")) => {
          push_thinking(t, string(inner, "text").or_else(|| string(inner, "reasoning")))
        }
        (Some("error"), _) => push_error(t, string(inner, "message").or_else(|| string(inner, "error"))),
        _ => {}
      }
    }
    Some("run_result") => {
      t.finished = true;
      let reason = string(event, "finishReason").unwrap_or("finished");
      let ok = reason == "completed";
      let mut parts = vec![reason.to_string()];
      if let Some(n) = number(event, "iterations") {
        parts.push(format!("{n} iterations"));
      }
      if let Some(ms) = number(event, "durationMs") {
        parts.push(format!("{:.1}s", ms / 1000.0));
      }
      if !ok {
        let message = string(event, "error").or_else(|| string(event, "text")).unwrap_or(reason).to_string();
        t.error = Some(message.clone());
        if message != reason {
          parts.push(message);
        }
      }
      t.entries.push(Entry::Result { ok, detail: parts.join(" · ") });
    }
    Some("error") => push_error(t, string(event, "message").or_else(|| string(event, "error"))),
    _ => {}
  }
}

fn tool_entry(id: Option<String>, name: &str, summary: &str, input: &str) -> Entry {
  Entry::Tool { id, name: name.to_string(), summary: one_line(summary), input: clip(input), output: None, ok: None }
}

/// Attach a tool's output to the call with the same id (the latest unfinished call when the
/// CLI gives none). The backward scan makes a parse O(tool calls × entries); fine at the
/// hundreds of entries a headless run produces, and an id → index map if that ever changes.
fn complete_tool(t: &mut Transcript, id: Option<&str>, output: String, result_ok: bool) {
  let slot = t.entries.iter_mut().rev().find(|e| match e {
    Entry::Tool { id: tid, ok: None, .. } => id.is_none() || tid.as_deref() == id,
    _ => false,
  });
  if let Some(Entry::Tool { output: out, ok, .. }) = slot {
    *out = (!output.is_empty()).then(|| clip(&output));
    *ok = Some(result_ok);
  }
}

fn push_text(t: &mut Transcript, text: Option<&str>) {
  if let Some(text) = text.map(str::trim).filter(|s| !s.is_empty()) {
    t.entries.push(Entry::Text { text: clip(text) });
  }
}

fn push_thinking(t: &mut Transcript, text: Option<&str>) {
  if let Some(text) = text.map(str::trim).filter(|s| !s.is_empty()) {
    t.entries.push(Entry::Thinking { text: clip(text) });
  }
}

fn push_error(t: &mut Transcript, message: Option<&str>) {
  let text = message.unwrap_or("error").to_string();
  t.error = Some(text.clone());
  t.entries.push(Entry::Error { text: clip(&text) });
}

/// One line naming what a tool call is about: the command, path, pattern, or URL when the
/// input has one, otherwise its compact JSON.
fn input_summary(input: &Value) -> String {
  for key in
    ["command", "commands", "file_path", "path", "paths", "files", "pattern", "url", "query", "description", "prompt"]
  {
    match field(input, key) {
      Some(Value::String(s)) if !s.is_empty() => return one_line(s),
      Some(Value::Array(items)) if !items.is_empty() => {
        // Strings, or objects naming a `path` (Cline's `read_files`).
        let parts: Vec<&str> = items
          .iter()
          .filter_map(|v| match v {
            Value::String(s) => Some(s.as_str()),
            other => string(other, "path"),
          })
          .collect();
        if !parts.is_empty() {
          return one_line(&parts.join(" ; "));
        }
      }
      _ => {}
    }
  }
  match input {
    Value::Null => String::new(),
    other => one_line(&json::write(other)),
  }
}

/// A `tool_result` content: a plain string, or an array of `{type: text, text}` blocks.
fn content_text(content: Option<&Value>) -> String {
  match content {
    Some(Value::String(s)) => s.clone(),
    Some(Value::Array(items)) => items
      .iter()
      .map(|item| match item {
        Value::String(s) => s.clone(),
        other => string(other, "text").map(str::to_string).unwrap_or_else(|| json::write(other)),
      })
      .collect::<Vec<_>>()
      .join("\n"),
    Some(other) => json::write(other),
    None => String::new(),
  }
}

fn one_line(s: &str) -> String {
  let flat: String = s.split_whitespace().collect::<Vec<_>>().join(" ");
  if flat.chars().count() > 160 {
    flat.chars().take(159).collect::<String>() + "…"
  } else {
    flat
  }
}

fn clip(s: &str) -> String {
  let count = s.chars().count();
  if count <= MAX_BODY_CHARS {
    return s.to_string();
  }
  let kept: String = s.chars().take(MAX_BODY_CHARS).collect();
  format!("{kept}\n… ({} more characters in the run log)", count - MAX_BODY_CHARS)
}

fn blocks(content: Option<&Value>) -> &[Value] {
  match content {
    Some(Value::Array(items)) => items,
    _ => &[],
  }
}

fn field<'a>(value: &'a Value, key: &str) -> Option<&'a Value> {
  match value {
    Value::Object(fields) => fields.iter().find(|(k, _)| k == key).map(|(_, v)| v),
    _ => None,
  }
}

fn string<'a>(value: &'a Value, key: &str) -> Option<&'a str> {
  match field(value, key) {
    Some(Value::String(s)) => Some(s),
    _ => None,
  }
}

fn number(value: &Value, key: &str) -> Option<f64> {
  match field(value, key) {
    Some(Value::Number(n)) => Some(*n),
    _ => None,
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  /// Trimmed from a real `claude -p --output-format stream-json --verbose` run.
  const CLAUDE: &str = r#"{"type":"system","subtype":"init","cwd":"/home/agent/repo","session_id":"s1","tools":["Bash","Edit"],"model":"claude-opus-5-5"}
{"type":"assistant","message":{"content":[{"type":"thinking","thinking":"Write the file."},{"type":"tool_use","id":"t1","name":"Bash","input":{"command":"echo ok > tmp/r.json"}}]}}
{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"t1","content":[{"type":"text","text":"done"}],"is_error":false}]}}
{"type":"assistant","message":{"content":[{"type":"text","text":"Wrote the result."}]}}
{"type":"result","subtype":"success","is_error":false,"num_turns":2,"duration_ms":4200,"total_cost_usd":0.0123,"result":"Wrote the result."}
"#;

  /// Verbatim shape of a real `codex exec --json` run (codex-cli, 2026-10).
  const CODEX: &str = r#"{"type":"thread.started","thread_id":"th1"}
{"type":"turn.started"}
{"type":"item.completed","item":{"id":"item_0","type":"agent_message","text":"I'll run the command now.\n"}}
{"type":"item.started","item":{"id":"item_1","type":"command_execution","command":"/bin/bash -lc 'echo hello > out.txt'","aggregated_output":"","exit_code":null,"status":"in_progress"}}
{"type":"item.completed","item":{"id":"item_1","type":"command_execution","command":"/bin/bash -lc 'echo hello > out.txt'","aggregated_output":"","exit_code":0,"status":"completed"}}
{"type":"item.completed","item":{"id":"item_2","type":"agent_message","text":"done"}}
{"type":"turn.completed","usage":{"input_tokens":30262,"cached_input_tokens":14848,"output_tokens":52}}
"#;

  /// Verbatim shape of a real `cline --json` run (cline 3.0.70).
  const CLINE: &str = r#"{"ts":"2026-10-12T17:02:18.244Z","type":"hook_event","hookEventName":"agent_start"}
{"ts":"2026-10-12T17:02:22.683Z","type":"agent_event","event":{"type":"usage","inputTokens":5653,"outputTokens":20}}
{"ts":"2026-10-12T17:02:22.687Z","type":"agent_event","event":{"type":"content_start","contentType":"tool","toolCallId":"c1","toolName":"run_commands","input":{"commands":["echo hello > out.txt"]}}}
{"ts":"2026-10-12T17:02:22.694Z","type":"agent_event","event":{"type":"content_end","contentType":"tool","toolCallId":"c1","toolName":"run_commands","output":[{"query":"echo hello > out.txt","result":"","success":true}]}}
{"ts":"2026-10-12T17:02:24.601Z","type":"agent_event","event":{"type":"content_start","contentType":"text","text":"done"}}
{"ts":"2026-10-12T17:02:24.606Z","type":"agent_event","event":{"type":"content_end","contentType":"text","text":"done"}}
{"ts":"2026-10-12T17:02:24.790Z","type":"run_result","finishReason":"completed","iterations":2,"durationMs":6371,"text":"done"}
"#;

  #[test]
  fn claude_stream_pairs_tool_calls_with_their_results() {
    let t = parse(Harness::ClaudeHeadless, CLAUDE);
    assert!(t.finished && t.error.is_none());
    assert!(
      matches!(&t.entries[0], Entry::Session { detail } if detail.contains("claude-opus-5-5") && detail.contains("2 tools"))
    );
    assert!(matches!(&t.entries[1], Entry::Thinking { text } if text == "Write the file."));
    let Entry::Tool { name, summary, output, ok, .. } = &t.entries[2] else { panic!("{:?}", t.entries[2]) };
    assert_eq!((name.as_str(), summary.as_str()), ("Bash", "echo ok > tmp/r.json"));
    assert_eq!((output.as_deref(), *ok), (Some("done"), Some(true)));
    assert!(matches!(&t.entries[3], Entry::Text { text } if text == "Wrote the result."));
    assert!(
      matches!(&t.entries[4], Entry::Result { ok: true, detail } if detail.contains("2 turns") && detail.contains("4.2s"))
    );
  }

  #[test]
  fn claude_error_result_is_the_failure_text() {
    let stream = r#"{"type":"result","subtype":"success","is_error":true,"num_turns":1,"result":"Failed to authenticate: OAuth session expired and could not be refreshed"}"#;
    let t = parse(Harness::ClaudeHeadless, stream);
    assert!(t.finished);
    assert!(t.failure_text().contains("Failed to authenticate"));
    let Entry::Result { ok: false, detail } = &t.entries[0] else { panic!("{:?}", t.entries) };
    assert!(detail.starts_with("error · 1 turn ·"), "a failed run never reads as success: {detail}");
  }

  #[test]
  fn codex_stream_merges_started_and_completed_commands() {
    let t = parse(Harness::CodexHeadless, CODEX);
    assert!(t.finished && t.error.is_none());
    let tools: Vec<_> = t.entries.iter().filter(|e| matches!(e, Entry::Tool { .. })).collect();
    assert_eq!(tools.len(), 1, "item.started and item.completed are one call: {:?}", t.entries);
    assert!(matches!(tools[0], Entry::Tool { name, ok: Some(true), .. } if name == "shell"));
    assert!(matches!(t.entries.last(), Some(Entry::Result { ok: true, detail }) if detail.contains("input 30262")));
    let failed = parse(Harness::CodexHeadless, r#"{"type":"turn.failed","error":{"message":"usage limit reached"}}"#);
    assert_eq!(failed.error.as_deref(), Some("usage limit reached"));
  }

  #[test]
  fn cline_stream_reads_tools_text_and_result() {
    let t = parse(Harness::ClineHeadless, CLINE);
    assert!(t.finished && t.error.is_none());
    let Entry::Tool { name, summary, ok, .. } = &t.entries[0] else { panic!("{:?}", t.entries) };
    assert_eq!((name.as_str(), summary.as_str(), *ok), ("run_commands", "echo hello > out.txt", Some(true)));
    let read = r#"{"type":"agent_event","event":{"type":"content_start","contentType":"tool","toolCallId":"r","toolName":"read_files","input":{"files":[{"path":"/repo/a.md"},{"path":"b.md"}]}}}"#;
    let t2 = parse(Harness::ClineHeadless, read);
    assert!(
      matches!(&t2.entries[0], Entry::Tool { summary, .. } if summary == "/repo/a.md ; b.md"),
      "{:?}",
      t2.entries
    );
    assert!(matches!(&t.entries[1], Entry::Text { text } if text == "done"), "text appears once: {:?}", t.entries);
    assert!(matches!(&t.entries[2], Entry::Result { ok: true, detail } if detail.contains("2 iterations")));
  }

  #[test]
  fn stray_lines_and_long_outputs_stay_readable() {
    let long = "x".repeat(MAX_BODY_CHARS + 10);
    let stream = format!(
      "warning: printed on stdout\n{{\"type\":\"assistant\",\"message\":{{\"content\":[{{\"type\":\"text\",\"text\":\"{long}\"}}]}}}}\n"
    );
    let t = parse(Harness::ClaudeHeadless, &stream);
    assert!(matches!(&t.entries[0], Entry::Note { text } if text.contains("warning")));
    assert!(matches!(&t.entries[1], Entry::Text { text } if text.ends_with("(10 more characters in the run log)")));
    assert!(!t.finished, "no terminal event yet");
  }
}
