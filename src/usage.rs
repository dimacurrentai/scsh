//! Per-attempt spend from a harness that can report it.
//!
//! Cursor's interactive TUI does not emit `--print` stream-json. scsh therefore
//! installs a user-level hook (in the forwarded config home, never the skill repo)
//! that appends `stop` / `afterAgentResponse` / `postToolUse` payloads. Those are
//! the events exposed by the pinned Cursor CLI in interactive mode.
//!
//! Hook `input_tokens` includes cache. This summary stores the four buckets the
//! same way print-mode did: `input` is uncached only.

use crate::json::{self, Value};
use std::collections::{BTreeMap, BTreeSet};

/// Writer installed into the container's Cursor config home (Python ships in the base image).
/// Reads one hook event on stdin, appends it as a single line, always exits 0.
pub const CURSOR_HOOK_PY: &str = r#"#!/usr/bin/env python3
import fcntl
import json
import os
import sys

try:
    event = json.load(sys.stdin)
    path = os.environ.get("SCSH_CURSOR_HOOKS_LOG") or os.path.join(os.path.dirname(__file__), "scsh-cursor-hooks.jsonl")
    os.umask(0o077)
    with open(path, "a") as log:
        fcntl.flock(log, fcntl.LOCK_EX)
        log.write(json.dumps(event, separators=(",", ":")) + "\n")
        log.flush()
        os.fsync(log.fileno())
except Exception:
    pass  # Accounting must never interrupt the agent loop.
print("{}")
"#;

/// `hooks.json` that points Cursor's interactive TUI at [`CURSOR_HOOK_PY`].
pub fn hooks_json(script_in_container: &str) -> String {
  let cmd = json::quote(script_in_container);
  format!(
    r#"{{
  "version": 1,
  "hooks": {{
    "afterAgentResponse": [{{ "command": {cmd} }}],
    "stop": [{{ "command": {cmd} }}],
    "postToolUse": [{{ "command": {cmd} }}],
    "sessionEnd": [{{ "command": {cmd} }}]
  }}
}}
"#
  )
}

#[derive(Debug, Clone, PartialEq)]
pub struct Tokens {
  /// Uncached prompt/input tokens consumed by the provider.
  pub input: u64,
  /// Model output tokens reported by the provider.
  pub output: u64,
  /// Prompt/input tokens served from a provider cache.
  pub cache_read: u64,
  /// Prompt/input tokens written into a provider cache, if reported by this harness version.
  pub cache_write: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Harness {
  ClaudeCode,
  Codex,
  Cursor,
  Grok,
}

impl Harness {
  fn json_name(self) -> &'static str {
    match self {
      Harness::ClaudeCode => "claude_code",
      Harness::Codex => "codex",
      Harness::Cursor => "cursor",
      Harness::Grok => "grok",
    }
  }

  fn source(self) -> &'static str {
    match self {
      Harness::ClaudeCode => "claude_session_jsonl",
      Harness::Codex => "codex_session_jsonl",
      Harness::Cursor => "cursor_hooks",
      Harness::Grok => "grok_session_jsonl",
    }
  }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Summary {
  /// Harness whose native accounting produced this summary.
  pub harness: Harness,
  /// Whether every native record needed for an exact total parsed successfully.
  pub complete: bool,
  /// Normalized token buckets, or `None` when the harness emitted no usable counters.
  pub tokens: Option<Tokens>,
  /// Observable model responses when the native format identifies them reliably.
  pub llm_round_trips: Option<u64>,
  /// Observable tool calls when the native format identifies them reliably.
  pub tool_calls: Option<u64>,
}

impl Summary {
  pub fn to_json(&self) -> String {
    json::write_pretty(&summary_value(self))
  }

  pub fn compact_json(&self) -> String {
    json::write(&summary_value(self))
  }

  pub fn from_json(text: &str) -> Option<Summary> {
    let value = json::parse(text).ok()?;
    if object_len(&value)? != 1 || object_keys(&value)? != ["TokenUsage"].into_iter().collect() {
      return None;
    }
    let payload = field(&value, "TokenUsage")?;
    if object_len(payload)? != 7
      || object_keys(payload)?
        != ["complete", "harness", "llm_round_trips", "schema_version", "source", "tokens", "tool_calls"]
          .into_iter()
          .collect()
    {
      return None;
    }
    if count(payload, "schema_version") != Some(1) {
      return None;
    }
    let harness = match string(payload, "harness")? {
      "claude_code" => Harness::ClaudeCode,
      "codex" => Harness::Codex,
      "cursor" => Harness::Cursor,
      "grok" => Harness::Grok,
      _ => return None,
    };
    if string(payload, "source") != Some(harness.source()) {
      return None;
    }
    let complete = match field(payload, "complete")? {
      Value::Bool(value) => *value,
      _ => return None,
    };
    let tokens = match field(payload, "tokens")? {
      Value::Null => None,
      usage => {
        if object_len(usage)? != 4
          || object_keys(usage)? != ["cache_read", "cache_write", "input", "output"].into_iter().collect()
        {
          return None;
        }
        Some(Tokens {
          input: count(usage, "input")?,
          output: count(usage, "output")?,
          cache_read: count(usage, "cache_read")?,
          cache_write: optional_count(usage, "cache_write")?,
        })
      }
    };
    if complete && tokens.is_none() {
      return None;
    }
    Some(Summary {
      harness,
      complete,
      tokens,
      llm_round_trips: optional_count(payload, "llm_round_trips")?,
      tool_calls: optional_count(payload, "tool_calls")?,
    })
  }

  /// One-line board / job-page fragment: cache split, trips, tools.
  pub fn phrase(&self) -> String {
    let mut parts = Vec::new();
    if let Some(t) = &self.tokens {
      let total_input = t.input.saturating_add(t.cache_read).saturating_add(t.cache_write.unwrap_or(0));
      if total_input > 0 {
        parts.push(format!("{} input tokens", compact_count(total_input)));
      }
      if t.output > 0 {
        parts.push(format!("{} output tokens", compact_count(t.output)));
      }
      if t.cache_read > 0 {
        parts.push(format!("{} cached tokens", compact_count(t.cache_read)));
      }
    }
    if self.tokens.is_none() {
      parts.push("tokens unavailable".into());
    }
    if let Some(trips) = self.llm_round_trips.filter(|n| *n > 0) {
      parts.push(format!("{trips} LLM call{}", if trips == 1 { "" } else { "s" }));
    }
    if let Some(tools) = self.tool_calls.filter(|n| *n > 0) {
      parts.push(format!("{tools} tool call{}", if tools == 1 { "" } else { "s" }));
    }
    if !self.complete {
      parts.push("partial".into());
    }
    if parts.is_empty() {
      parts.push("No measured cost".into());
    }
    parts.join(" · ")
  }

  /// Exact native buckets for the compact UI summary's hover text.
  pub fn details(&self) -> String {
    let mut parts = Vec::new();
    if let Some(t) = &self.tokens {
      if t.input > 0 {
        parts.push(format!("Uncached input: {}", grouped_count(t.input)));
      }
      if t.cache_read > 0 {
        parts.push(format!("Cache read: {}", grouped_count(t.cache_read)));
      }
      if let Some(write) = t.cache_write.filter(|n| *n > 0) {
        parts.push(format!("Cache write: {}", grouped_count(write)));
      }
      if t.output > 0 {
        parts.push(format!("Output: {}", grouped_count(t.output)));
      }
    }
    if let Some(calls) = self.llm_round_trips.filter(|n| *n > 0) {
      parts.push(format!("Model calls: {}", grouped_count(calls)));
    }
    if let Some(tools) = self.tool_calls.filter(|n| *n > 0) {
      parts.push(format!("Tool calls: {}", grouped_count(tools)));
    }
    if !self.complete {
      parts.push("Partial accounting".into());
    }
    parts.join(" · ")
  }
}

fn compact_count(n: u64) -> String {
  if n < 1_000 {
    return n.to_string();
  }
  let (value, suffix) = if n < 1_000_000 {
    (n, "k")
  } else if n < 1_000_000_000 {
    (n / 1_000, "m")
  } else {
    (n / 1_000_000, "b")
  };
  let whole = value / 1_000;
  let tenth = (value % 1_000) / 100;
  if whole < 10 && tenth > 0 {
    format!("{whole}.{tenth}{suffix}")
  } else {
    format!("{whole}{suffix}")
  }
}

fn grouped_count(n: u64) -> String {
  let digits = n.to_string();
  let mut out = String::with_capacity(digits.len() + digits.len() / 3);
  for (index, ch) in digits.chars().enumerate() {
    if index > 0 && (digits.len() - index).is_multiple_of(3) {
      out.push(',');
    }
    out.push(ch);
  }
  out
}

fn summary_value(summary: &Summary) -> Value {
  object(vec![(
    "TokenUsage",
    object(vec![
      ("schema_version", number(1)),
      ("harness", Value::String(summary.harness.json_name().into())),
      ("source", Value::String(summary.harness.source().into())),
      ("complete", Value::Bool(summary.complete)),
      ("tokens", tokens_json(summary.tokens.as_ref())),
      ("llm_round_trips", optional_number(summary.llm_round_trips)),
      ("tool_calls", optional_number(summary.tool_calls)),
    ]),
  )])
}

fn tokens_json(tokens: Option<&Tokens>) -> Value {
  match tokens {
    None => Value::Null,
    Some(t) => object(vec![
      ("input", Value::Number(t.input as f64)),
      ("output", Value::Number(t.output as f64)),
      ("cache_read", Value::Number(t.cache_read as f64)),
      ("cache_write", optional_number(t.cache_write)),
    ]),
  }
}

#[derive(Default)]
struct Conversation {
  generations: BTreeSet<String>,
  /// Most recent event generation, used to recognize Cursor's trailing response hook.
  active_generation: Option<String>,
  /// Generation named by the last completed stop, when Cursor supplies one.
  stopped_generation: Option<String>,
  responses: u64,
  tools: BTreeSet<String>,
  missing_tool_id: bool,
  tokens: Option<Tokens>,
  stopped: bool,
}

/// Summarize a Cursor TUI hook stream. Missing token fields stay null; partial
/// counts are lower bounds. `stop` and `afterAgentResponse` share totals for one
/// `generation_id` — keep the last complete payload per conversation, never sum them.
pub fn cursor_hook_summary(stream: &str) -> Summary {
  let mut conversations: BTreeMap<String, Conversation> = BTreeMap::new();
  let mut invalid = 0;
  for line in stream.lines().filter(|line| !line.trim().is_empty()) {
    let Ok(event) = json::parse(line) else {
      invalid += 1;
      continue;
    };
    let Some(event_name) = hook_name(&event) else {
      invalid += 1;
      continue;
    };
    let id =
      string(&event, "conversation_id").or_else(|| string(&event, "session_id")).unwrap_or("session").to_string();
    let conversation = conversations.entry(id).or_default();
    if matches!(event_name, "afterAgentResponse" | "stop" | "postToolUse") {
      if let Some(generation) = string(&event, "generation_id") {
        conversation.generations.insert(generation.to_string());
        if event_name != "stop" {
          conversation.active_generation = Some(generation.to_string());
        }
      }
    }
    match event_name {
      "afterAgentResponse" => {
        conversation.responses += 1;
        let generation = string(&event, "generation_id");
        let trailing_completed_response = conversation.stopped
          && match conversation.stopped_generation.as_deref() {
            Some(stopped) => generation == Some(stopped),
            None => generation.is_none(),
          };
        if !trailing_completed_response {
          conversation.stopped = false;
          conversation.stopped_generation = None;
        }
        // Cursor emits `stop` before the same generation's response hook. Preserve
        // real stop counters when that trailing hook omits its token fields.
        if let Some(tokens) = hook_tokens(&event) {
          conversation.tokens = Some(tokens);
        }
      }
      "stop" => {
        if conversation
          .active_generation
          .as_deref()
          .is_some_and(|active| string(&event, "generation_id") != Some(active))
        {
          continue;
        }
        conversation.stopped = string(&event, "status") == Some("completed");
        conversation.stopped_generation = conversation
          .stopped
          .then(|| {
            string(&event, "generation_id").map(str::to_string).or_else(|| conversation.active_generation.clone())
          })
          .flatten();
        conversation.tokens = hook_tokens(&event);
      }
      "sessionEnd" => {
        // Session teardown cannot turn an aborted generation into a completed turn.
      }
      "postToolUse" => {
        conversation.stopped = false;
        conversation.stopped_generation = None;
        if let Some(tool) = string(&event, "tool_use_id").or_else(|| string(&event, "call_id")) {
          conversation.tools.insert(tool.to_string());
        } else {
          conversation.missing_tool_id = true;
        }
      }
      _ => {}
    }
  }
  let llm_round_trips = conversations
    .values()
    .map(|c| if !c.generations.is_empty() { c.generations.len() as u64 } else { c.responses })
    .sum();
  let tool_calls = conversations.values().map(|c| c.tools.len() as u64).sum();
  let tokens = conversations.values().try_fold(
    Tokens { input: 0, output: 0, cache_read: 0, cache_write: Some(0) },
    |mut total, c| {
      let t = c.tokens.as_ref()?;
      total.input = total.input.checked_add(t.input).filter(|n| *n <= 9_007_199_254_740_991)?;
      total.output = total.output.checked_add(t.output).filter(|n| *n <= 9_007_199_254_740_991)?;
      total.cache_read = total.cache_read.checked_add(t.cache_read).filter(|n| *n <= 9_007_199_254_740_991)?;
      total.cache_write = Some(total.cache_write?.checked_add(t.cache_write?).filter(|n| *n <= 9_007_199_254_740_991)?);
      Some(total)
    },
  );
  let tokens = tokens.filter(|_| !conversations.is_empty() && invalid == 0);
  let complete = tokens.is_some()
    && invalid == 0
    && !conversations.is_empty()
    && conversations.values().all(|c| c.stopped && c.tokens.is_some() && !c.missing_tool_id);
  Summary {
    harness: Harness::Cursor,
    complete,
    tokens,
    llm_round_trips: Some(llm_round_trips),
    tool_calls: Some(tool_calls),
  }
}

pub(crate) fn unavailable(harness: Harness) -> Summary {
  Summary { harness, complete: false, tokens: None, llm_round_trips: None, tool_calls: None }
}

/// Native turn boundaries, separate from token snapshots: a tool-call response or
/// cumulative counter alone does not mean the agent has finished its last turn.
pub(crate) fn turn_finished(harness: crate::config::Harness, stream: &str) -> bool {
  if harness == crate::config::Harness::Grok {
    return grok_session_summary(stream).complete;
  }
  let mut finished = false;
  for line in stream.lines().filter(|line| !line.trim().is_empty()) {
    let Ok(event) = json::parse(line) else { return false };
    match harness {
      crate::config::Harness::Claude => match string(&event, "type") {
        Some("user") => finished = false,
        Some("system") if string(&event, "subtype") == Some("turn_duration") => finished = true,
        Some("assistant") => {
          finished = field(&event, "message").and_then(|m| string(m, "stop_reason")) == Some("end_turn");
        }
        _ => {}
      },
      crate::config::Harness::Codex => {
        if let Some(payload) = field(&event, "payload") {
          match string(payload, "type") {
            Some("task_started") => finished = false,
            Some("task_complete") => finished = true,
            _ => {}
          }
        }
      }
      _ => return false,
    }
  }
  finished
}

/// Claude Code records one `usage` object per assistant response. The forwarded config
/// directory starts empty for every attempt, so every transcript below it belongs to this run,
/// including subagents. A model response can appear under several transcript UUIDs, so
/// account by the provider message ID and request ID, retaining the largest streamed counters.
pub fn claude_session_summary(transcripts: &[Option<String>]) -> Option<Summary> {
  let mut tokens = Tokens { input: 0, output: 0, cache_read: 0, cache_write: Some(0) };
  let mut responses: BTreeMap<(String, String), Tokens> = BTreeMap::new();
  let mut tools = BTreeSet::new();
  let mut invalid = false;
  for text in transcripts {
    let Some(text) = text else {
      invalid = true;
      continue;
    };
    for line in text.lines().filter(|line| !line.trim().is_empty()) {
      let Ok(event) = json::parse(line) else {
        invalid = true;
        continue;
      };
      if string(&event, "type") != Some("assistant") {
        continue;
      }
      let Some(message) = field(&event, "message") else {
        continue;
      };
      if string(message, "model") == Some("<synthetic>") {
        continue;
      }
      let Some(id) = string(message, "id").or_else(|| string(&event, "uuid")) else {
        invalid = true;
        continue;
      };
      let Some(t) = field(message, "usage").and_then(claude_tokens) else {
        invalid = true;
        continue;
      };
      let key = (id.to_string(), string(&event, "requestId").unwrap_or("").to_string());
      responses
        .entry(key)
        .and_modify(|previous| {
          previous.input = previous.input.max(t.input);
          previous.output = previous.output.max(t.output);
          previous.cache_read = previous.cache_read.max(t.cache_read);
          previous.cache_write = previous.cache_write.max(t.cache_write);
        })
        .or_insert(t);
      if let Some(Value::Array(content)) = field(message, "content") {
        for block in content {
          if string(block, "type") == Some("tool_use") {
            if let Some(id) = string(block, "id") {
              tools.insert(id.to_string());
            }
          }
        }
      }
    }
  }
  if responses.is_empty() {
    return None;
  }
  for t in responses.values() {
    if !add_tokens(&mut tokens, t) {
      return Some(unavailable(Harness::ClaudeCode));
    }
  }
  Some(Summary {
    harness: Harness::ClaudeCode,
    complete: !invalid,
    tokens: (!invalid).then_some(tokens),
    llm_round_trips: Some(responses.len() as u64),
    tool_calls: Some(tools.len() as u64),
  })
}

/// Codex emits cumulative `total_token_usage` snapshots. Keep the last valid snapshot in
/// each fresh per-run session file. Forks with inherited accounting are unavailable.
pub fn codex_session_summary(transcripts: &[Option<String>]) -> Option<Summary> {
  let mut total = Tokens { input: 0, output: 0, cache_read: 0, cache_write: Some(0) };
  let mut sessions = 0;
  let mut invalid = false;
  for text in transcripts {
    let Some(text) = text else {
      invalid = true;
      continue;
    };
    let mut last = None;
    let mut session_meta_seen = false;
    for line in text.lines().filter(|line| !line.trim().is_empty()) {
      let Ok(event) = json::parse(line) else {
        invalid = true;
        continue;
      };
      if string(&event, "type") == Some("session_meta") {
        // A fork may copy the parent's cumulative counters. Until the format supplies a
        // reliable accounting baseline, refuse to add inherited spend a second time.
        let fork = field(&event, "payload").and_then(|payload| string(payload, "forked_from_id"));
        invalid |= session_meta_seen || fork.is_some();
        session_meta_seen = true;
      }
      let info = field(&event, "payload")
        .filter(|payload| string(payload, "type") == Some("token_count"))
        .and_then(|payload| field(payload, "info"));
      if let Some(usage) = info.and_then(|info| field(info, "total_token_usage")) {
        if let Some(tokens) = codex_tokens(usage) {
          last = Some(tokens);
        } else {
          invalid = true;
        }
      }
    }
    if let Some(tokens) = last {
      sessions += 1;
      if !add_tokens(&mut total, &tokens) {
        invalid = true;
      }
    } else if session_meta_seen {
      invalid = true;
    }
  }
  if sessions == 0 {
    return None;
  }
  Some(Summary {
    harness: Harness::Codex,
    complete: !invalid,
    tokens: (!invalid).then_some(total),
    // Codex's token_count snapshots are not one-per-turn, so do not relabel them as turns.
    llm_round_trips: None,
    tool_calls: None,
  })
}

fn claude_tokens(usage: &Value) -> Option<Tokens> {
  Some(Tokens {
    input: count(usage, "input_tokens")?,
    output: count(usage, "output_tokens")?,
    cache_read: count(usage, "cache_read_input_tokens")?,
    cache_write: Some(count(usage, "cache_creation_input_tokens")?),
  })
}

fn codex_tokens(usage: &Value) -> Option<Tokens> {
  let input = count(usage, "input_tokens")?;
  let cache_read = count(usage, "cached_input_tokens")?;
  let cache_write = match field(usage, "cache_write_input_tokens") {
    None => None, // Older Codex versions do not report cache creation.
    Some(_) => Some(count(usage, "cache_write_input_tokens")?),
  };
  Some(Tokens {
    input: input.checked_sub(cache_read.checked_add(cache_write.unwrap_or(0))?)?,
    output: count(usage, "output_tokens")?,
    cache_read,
    cache_write,
  })
}

/// Identify the primary Grok session from its native event stream. Child usage is
/// folded into its parent's turn totals, so summing every session would double-count.
pub(crate) fn grok_primary_session(events: &str) -> Result<Option<String>, ()> {
  let mut primary = None;
  for line in events.lines().filter(|line| !line.trim().is_empty()) {
    let event = json::parse(line).map_err(|_| ())?;
    if string(&event, "type") == Some("turn_started") {
      let relationship = string(&event, "session_relationship").ok_or(())?;
      if relationship == "primary" {
        let id = string(&event, "session_id").ok_or(())?;
        if primary.as_deref().is_some_and(|previous| previous != id) {
          return Err(());
        }
        primary = Some(id.to_string());
      }
    }
  }
  Ok(primary)
}

/// Grok 1.0.34's `_x.ai/session/update` records contain per-prompt bills, not
/// cumulative session totals. Only the primary stream is supplied: its bills
/// already include subagents. Ignore context-window telemetry and modelUsage
/// breakdowns, and honor usageIsIncomplete (including outstanding subagent bills).
/// Source: xai-org/grok-build, xai-grok-shell/src/extensions/notification.rs.
pub fn grok_session_summary(stream: &str) -> Summary {
  let mut turns = BTreeMap::new();
  let mut session = None;
  let mut finished = false;
  for line in stream.lines().filter(|line| !line.trim().is_empty()) {
    let Ok(event) = json::parse(line) else { return unavailable(Harness::Grok) };
    let Some(params) = field(&event, "params") else { return unavailable(Harness::Grok) };
    let Some(id) = string(params, "sessionId") else { return unavailable(Harness::Grok) };
    if session.as_deref().is_some_and(|previous| previous != id) {
      return unavailable(Harness::Grok);
    }
    session = Some(id.to_string());
    let Some(update) = field(params, "update") else { return unavailable(Harness::Grok) };
    match string(update, "sessionUpdate") {
      Some("turn_completed") => {
        if string(&event, "method") != Some("_x.ai/session/update") {
          return unavailable(Harness::Grok);
        }
        let Some(prompt) = string(update, "prompt_id") else { return unavailable(Harness::Grok) };
        let Some(usage) = field(update, "usage") else { return unavailable(Harness::Grok) };
        let Some(tokens) = grok_tokens(usage) else { return unavailable(Harness::Grok) };
        let Some(calls) = count(usage, "modelCalls") else { return unavailable(Harness::Grok) };
        let bill = (tokens, calls);
        if turns.get(prompt).is_some_and(|previous| previous != &bill || !finished) {
          return unavailable(Harness::Grok);
        }
        turns.insert(prompt.to_string(), bill);
        finished = string(update, "stop_reason") == Some("end_turn");
      }
      Some(
        "user_message_chunk"
        | "agent_message_chunk"
        | "agent_thought_chunk"
        | "tool_call"
        | "tool_call_update"
        | "tool_call_delta_chunk",
      ) => finished = false,
      _ => {}
    }
  }
  if turns.is_empty() {
    return unavailable(Harness::Grok);
  }
  let mut total = Tokens { input: 0, output: 0, cache_read: 0, cache_write: Some(0) };
  let mut calls = 0u64;
  for (tokens, count) in turns.values() {
    if !add_tokens(&mut total, tokens) {
      return unavailable(Harness::Grok);
    }
    let Some(sum) = calls.checked_add(*count).filter(|n| *n <= 9_007_199_254_740_991) else {
      return unavailable(Harness::Grok);
    };
    calls = sum;
  }
  Summary {
    harness: Harness::Grok,
    complete: finished,
    tokens: Some(total),
    llm_round_trips: Some(calls),
    // The primary transcript omits child tool calls; do not present a partial count as exact.
    tool_calls: None,
  }
}

fn grok_tokens(usage: &Value) -> Option<Tokens> {
  if !matches!(field(usage, "usageIsIncomplete"), None | Some(Value::Bool(false))) {
    return None;
  }
  let input = count(usage, "inputTokens")?;
  let output = count(usage, "outputTokens")?;
  let cache_read = count(usage, "cachedReadTokens")?;
  let cache_write = count(usage, "cacheCreationTokens")?;
  if count(usage, "totalTokens")? != input.checked_add(output)? {
    return None;
  }
  Some(Tokens {
    input: input.checked_sub(cache_read.checked_add(cache_write)?)?,
    output,
    cache_read,
    cache_write: Some(cache_write),
  })
}

fn add_tokens(total: &mut Tokens, add: &Tokens) -> bool {
  let Some(input) = total.input.checked_add(add.input) else { return false };
  let Some(output) = total.output.checked_add(add.output) else { return false };
  let Some(cache_read) = total.cache_read.checked_add(add.cache_read) else { return false };
  let cache_write = match (total.cache_write, add.cache_write) {
    (Some(left), Some(right)) => {
      let Some(sum) = left.checked_add(right) else { return false };
      Some(sum)
    }
    _ => None,
  };
  if [input, output, cache_read, cache_write.unwrap_or(0)].iter().any(|n| *n > 9_007_199_254_740_991) {
    return false;
  }
  *total = Tokens { input, output, cache_read, cache_write };
  true
}

fn hook_name(event: &Value) -> Option<&str> {
  string(event, "hook_event_name").or_else(|| string(event, "event_name"))
}

fn hook_tokens(event: &Value) -> Option<Tokens> {
  let input = count(event, "input_tokens").or_else(|| count(event, "inputTokens"))?;
  let output = count(event, "output_tokens").or_else(|| count(event, "outputTokens"))?;
  let cache_read = count(event, "cache_read_tokens").or_else(|| count(event, "cacheReadTokens"))?;
  let cache_write = count(event, "cache_write_tokens").or_else(|| count(event, "cacheWriteTokens"))?;
  Some(Tokens {
    input: input.checked_sub(cache_read.checked_add(cache_write)?)?,
    output,
    cache_read,
    cache_write: Some(cache_write),
  })
}

fn field<'a>(value: &'a Value, key: &str) -> Option<&'a Value> {
  match value {
    Value::Object(fields) => fields.iter().find(|(name, _)| name == key).map(|(_, value)| value),
    _ => None,
  }
}

fn object_keys(value: &Value) -> Option<BTreeSet<&str>> {
  match value {
    Value::Object(fields) => Some(fields.iter().map(|(key, _)| key.as_str()).collect()),
    _ => None,
  }
}

fn object_len(value: &Value) -> Option<usize> {
  match value {
    Value::Object(fields) => Some(fields.len()),
    _ => None,
  }
}

fn string<'a>(value: &'a Value, key: &str) -> Option<&'a str> {
  match field(value, key)? {
    Value::String(value) if !value.is_empty() => Some(value),
    _ => None,
  }
}

fn count(value: &Value, key: &str) -> Option<u64> {
  match field(value, key)? {
    Value::Number(n) if n.is_finite() && *n >= 0.0 && *n <= 9_007_199_254_740_991.0 && n.fract() == 0.0 => {
      Some(*n as u64)
    }
    _ => None,
  }
}

fn optional_count(value: &Value, key: &str) -> Option<Option<u64>> {
  match field(value, key)? {
    Value::Null => Some(None),
    _ => count(value, key).map(Some),
  }
}

fn optional_number(value: Option<u64>) -> Value {
  value.map(|n| Value::Number(n as f64)).unwrap_or(Value::Null)
}

fn object(fields: Vec<(&str, Value)>) -> Value {
  Value::Object(fields.into_iter().map(|(key, value)| (key.to_string(), value)).collect())
}

fn number(n: usize) -> Value {
  Value::Number(n as f64)
}

#[cfg(test)]
mod tests {
  use super::*;

  pub(crate) const GROK: &str = include_str!("../tests/fixtures/grok-usage/updates.jsonl");

  #[test]
  fn grok_real_turn_matches_native_usage_and_round_trips() {
    let summary = grok_session_summary(GROK);
    assert!(summary.complete);
    assert_eq!(summary.tokens, Some(Tokens { input: 20_974, output: 191, cache_read: 19_456, cache_write: Some(0) }));
    assert_eq!(summary.llm_round_trips, Some(2));
    assert_eq!(summary.tool_calls, None);
    assert_eq!(Summary::from_json(&summary.to_json()), Some(summary));
  }

  #[test]
  fn grok_counts_turns_once_and_does_not_add_model_breakdowns() {
    let completed = GROK.lines().last().unwrap();
    let duplicated = format!("{GROK}{completed}");
    assert_eq!(grok_session_summary(&duplicated), grok_session_summary(GROK));
    let two_turns = format!("{GROK}{}", GROK.replace("prompt-1", "prompt-2"));
    let summary = grok_session_summary(&two_turns);
    assert!(summary.complete);
    assert_eq!(summary.tokens.unwrap().input, 41_948);
    assert_eq!(summary.llm_round_trips, Some(4));
  }

  #[test]
  fn grok_rejects_missing_partial_inconsistent_or_invalid_counters() {
    for (from, to) in [
      ("\"inputTokens\":40430", "\"inputTokens\":-1"),
      ("\"outputTokens\":191", "\"outputTokens\":1.5"),
      ("\"cachedReadTokens\":19456", "\"cachedReadTokens\":50000"),
      ("\"cacheCreationTokens\":0,", ""),
      ("\"modelCalls\":2", "\"modelCalls\":null"),
      ("\"totalTokens\":40621", "\"totalTokens\":1"),
      ("\"usage\":{", "\"usage\":{\"usageIsIncomplete\":true,"),
      ("\"usage\":{", "\"usage\":{\"usageIsIncomplete\":\"false\","),
    ] {
      let summary = grok_session_summary(&GROK.replace(from, to));
      assert!(!summary.complete, "{to}");
      assert_eq!(summary.tokens, None, "{to}");
    }
    assert_eq!(grok_session_summary("").tokens, None);
    assert_eq!(grok_session_summary(&format!("{GROK}{{")).tokens, None);
    let conflict = format!("{GROK}{}", GROK.lines().last().unwrap().replace("\"modelCalls\":2", "\"modelCalls\":3"));
    assert_eq!(grok_session_summary(&conflict).tokens, None);
  }

  #[test]
  fn grok_cache_writes_are_disjoint_and_sum_overflow_is_unavailable() {
    let summary = grok_session_summary(&GROK.replace("\"cacheCreationTokens\":0", "\"cacheCreationTokens\":100"));
    let tokens = summary.tokens.unwrap();
    assert_eq!(tokens.input, 20_874);
    assert_eq!(tokens.cache_write, Some(100));
    let large = GROK.replace("\"modelCalls\":2", "\"modelCalls\":9007199254740991");
    assert!(grok_session_summary(&large).complete);
    assert!(!grok_session_summary(&format!("{large}{}", large.replace("prompt-1", "prompt-2"))).complete);
  }

  #[test]
  fn grok_waits_for_final_bill_and_rejects_stale_completion_after_resuming() {
    let open = GROK.lines().filter(|line| !line.contains("turn_completed")).collect::<Vec<_>>().join("\n");
    assert!(!turn_finished(crate::config::Harness::Grok, &open));
    assert!(turn_finished(crate::config::Harness::Grok, GROK));
    let resumed = format!("{GROK}{}\n", GROK.lines().next().unwrap());
    assert!(!grok_session_summary(&resumed).complete);
    assert!(!grok_session_summary(&format!("{resumed}{}", GROK.lines().last().unwrap())).complete);
    assert!(!grok_session_summary(&GROK.replace("end_turn", "cancelled")).complete);
  }

  // Hook input_tokens includes cache. Last stop is the cumulative turn total.
  const HOOKS: &str = r#"{"hook_event_name":"afterAgentResponse","conversation_id":"c","generation_id":"g1","model":"cursor-grok-4.6-high-fast","input_tokens":45461,"output_tokens":80,"cache_read_tokens":30144,"cache_write_tokens":0}
{"hook_event_name":"postToolUse","conversation_id":"c","generation_id":"g1","tool_use_id":"t1"}
{"hook_event_name":"afterAgentResponse","conversation_id":"c","generation_id":"g2","input_tokens":45544,"output_tokens":120,"cache_read_tokens":30144,"cache_write_tokens":0}
{"hook_event_name":"postToolUse","conversation_id":"c","generation_id":"g2","tool_use_id":"t2"}
{"hook_event_name":"afterAgentResponse","conversation_id":"c","generation_id":"g3","input_tokens":45561,"output_tokens":190,"cache_read_tokens":30144,"cache_write_tokens":0}
{"hook_event_name":"stop","conversation_id":"c","generation_id":"g3","status":"completed","input_tokens":45561,"output_tokens":190,"cache_read_tokens":30144,"cache_write_tokens":0}
"#;

  #[test]
  fn hook_tokens_subtract_cache_and_keep_the_last_cumulative_payload() {
    let summary = cursor_hook_summary(HOOKS);
    assert!(summary.complete);
    let tokens = summary.tokens.as_ref().expect("tokens");
    assert_eq!(tokens.input, 45561 - 30144);
    assert_eq!(tokens.output, 190);
    assert_eq!(tokens.cache_read, 30144);
    assert_eq!(tokens.cache_write, Some(0));
    assert_eq!(summary.llm_round_trips, Some(3));
    assert_eq!(summary.tool_calls, Some(2));
    assert!(summary.phrase().contains("190 output tokens"));
    assert!(summary.phrase().contains("3 LLM calls"));
  }

  #[test]
  fn parallel_tools_share_one_generation() {
    let stream = r#"{"hook_event_name":"afterAgentResponse","conversation_id":"c","generation_id":"g1","input_tokens":10,"output_tokens":2,"cache_read_tokens":0,"cache_write_tokens":0}
{"hook_event_name":"postToolUse","conversation_id":"c","generation_id":"g1","tool_use_id":"t1"}
{"hook_event_name":"postToolUse","conversation_id":"c","generation_id":"g1","tool_use_id":"t2"}
{"hook_event_name":"afterAgentResponse","conversation_id":"c","generation_id":"g2","input_tokens":12,"output_tokens":4,"cache_read_tokens":0,"cache_write_tokens":0}
{"hook_event_name":"stop","conversation_id":"c","generation_id":"g2","status":"completed","input_tokens":12,"output_tokens":4,"cache_read_tokens":0,"cache_write_tokens":0}
"#;
    let summary = cursor_hook_summary(stream);
    assert_eq!(summary.llm_round_trips, Some(2));
    assert_eq!(summary.tool_calls, Some(2));
    assert_eq!(summary.tokens.unwrap().input, 12);
  }

  #[test]
  fn missing_or_malformed_tokens_stay_null() {
    for text in [
      String::new(),
      r#"{"hook_event_name":"stop","conversation_id":"c","generation_id":"g","status":"completed","input_tokens":null,"output_tokens":1,"cache_read_tokens":0,"cache_write_tokens":0}
"#
      .into(),
      r#"{"hook_event_name":"stop","conversation_id":"c","generation_id":"g","status":"completed","input_tokens":-1,"output_tokens":1,"cache_read_tokens":0,"cache_write_tokens":0}
"#
      .into(),
      format!("{HOOKS}truncated"),
      format!(r#"{HOOKS}{{"hook_event_name":"stop","conversation_id":"c","generation_id":"g3","status":"completed","input_tokens":null}}"#),
    ] {
      let summary = cursor_hook_summary(&text);
      assert_eq!(summary.tokens, None, "{text}");
      assert!(!summary.complete);
    }
  }

  #[test]
  fn hooks_json_names_the_container_script() {
    let text = hooks_json("/home/agent/repo/tmp/.cursor/scsh-cursor-usage.py");
    assert!(text.contains("afterAgentResponse"));
    assert!(text.contains("postToolUse"));
    assert!(text.contains("/home/agent/repo/tmp/.cursor/scsh-cursor-usage.py"));
  }

  #[test]
  fn claude_transcripts_sum_responses_and_subagents() {
    let root = std::env::temp_dir().join(format!("scsh-claude-usage-{}", crate::runtime::random_nonce_6()));
    let subagent = root.join("projects/repo/subagents");
    std::fs::create_dir_all(&subagent).unwrap();
    std::fs::write(
      root.join("projects/repo/main.jsonl"),
      r#"{"type":"assistant","uuid":"a","message":{"content":[{"type":"tool_use","id":"t"}],"usage":{"input_tokens":10,"output_tokens":3,"cache_read_input_tokens":20,"cache_creation_input_tokens":4}}}
{"type":"user","uuid":"u","message":{"content":"next"}}
{"type":"assistant","uuid":"b","message":{"content":[],"usage":{"input_tokens":2,"output_tokens":5,"cache_read_input_tokens":6,"cache_creation_input_tokens":1}}}
"#,
    )
    .unwrap();
    std::fs::write(
      subagent.join("agent-x.jsonl"),
      r#"{"type":"assistant","uuid":"c","message":{"content":[],"usage":{"input_tokens":7,"output_tokens":11,"cache_read_input_tokens":13,"cache_creation_input_tokens":17}}}
"#,
    )
    .unwrap();
    let summary = crate::daemon::usage::claude_session_summary(&root).unwrap();
    assert_eq!(summary.harness, Harness::ClaudeCode);
    assert_eq!(summary.tokens, Some(Tokens { input: 19, output: 19, cache_read: 39, cache_write: Some(22) }));
    assert_eq!(summary.llm_round_trips, Some(3));
    assert_eq!(summary.tool_calls, Some(1));
    assert!(summary.complete);
    std::fs::remove_dir_all(root).unwrap();
  }

  #[test]
  fn codex_sessions_keep_each_files_last_cumulative_snapshot() {
    let root = std::env::temp_dir().join(format!("scsh-codex-usage-{}", crate::runtime::random_nonce_6()));
    std::fs::create_dir_all(root.join("sessions/2026/09/18")).unwrap();
    let event = |input: u64, cached: u64, output: u64| {
      r#"{"type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":$INPUT,"cached_input_tokens":$CACHED,"cache_write_input_tokens":0,"output_tokens":$OUTPUT}}}}
"#
        .replace("$INPUT", &input.to_string())
        .replace("$CACHED", &cached.to_string())
        .replace("$OUTPUT", &output.to_string())
    };
    std::fs::write(
      root.join("sessions/2026/09/18/parent.jsonl"),
      format!("{}{}", event(100, 80, 5), event(150, 100, 9)),
    )
    .unwrap();
    std::fs::write(root.join("sessions/2026/09/18/subagent.jsonl"), event(40, 30, 7)).unwrap();
    let summary = crate::daemon::usage::codex_session_summary(&root).unwrap();
    assert_eq!(summary.harness, Harness::Codex);
    assert_eq!(summary.tokens, Some(Tokens { input: 60, output: 16, cache_read: 130, cache_write: Some(0) }));
    assert!(summary.complete);
    std::fs::remove_dir_all(root).unwrap();
  }

  #[test]
  fn public_schema_is_single_key_strict_and_round_trips() {
    let summary = Summary {
      harness: Harness::Codex,
      complete: true,
      tokens: Some(Tokens { input: 1, output: 2, cache_read: 3, cache_write: Some(4) }),
      llm_round_trips: None,
      tool_calls: None,
    };
    assert_eq!(Summary::from_json(&summary.to_json()), Some(summary));
    assert!(Summary::from_json(
      r#"{"TokenUsage":{"schema_version":1,"harness":"codex","source":"codex_session_jsonl","complete":true,"tokens":null,"llm_round_trips":0,"tool_calls":0,"surprise":1}}"#
    )
    .is_none());
    assert!(Summary::from_json(
      r#"{"TokenUsage":{"schema_version":1,"schema_version":1,"harness":"codex","source":"codex_session_jsonl","complete":true,"tokens":null,"llm_round_trips":0,"tool_calls":0}}"#
    )
    .is_none());
  }

  #[test]
  fn compact_cost_hides_zero_counters_but_keeps_them_in_json() {
    let zero = Summary {
      harness: Harness::ClaudeCode,
      complete: true,
      tokens: Some(Tokens { input: 0, output: 0, cache_read: 0, cache_write: Some(0) }),
      llm_round_trips: Some(0),
      tool_calls: Some(0),
    };
    assert_eq!(zero.phrase(), "No measured cost");
    assert_eq!(zero.details(), "");
    assert!(zero.to_json().contains("\"input\": 0"));

    let input_only =
      Summary { tokens: Some(Tokens { input: 6, output: 0, cache_read: 0, cache_write: Some(0) }), ..zero };
    assert_eq!(input_only.phrase(), "6 input tokens");
    assert!(!input_only.phrase().contains("output"));
  }

  #[test]
  fn public_schema_rejects_bad_nested_fields_and_types() {
    let good = r#"{"TokenUsage":{"schema_version":1,"harness":"claude_code","source":"claude_session_jsonl","complete":true,"tokens":{"input":10,"output":2,"cache_read":3,"cache_write":4},"llm_round_trips":1,"tool_calls":0}}"#;
    assert!(Summary::from_json(good).is_some());
    for (from, to) in [
      (r#""complete":true"#, r#""complete":"true""#),
      (r#""input":10"#, r#""input":-1"#),
      (r#""input":10"#, r#""input":1.5"#),
      (r#""input":10"#, r#""input":10,"input":10"#),
      (r#""input":10"#, r#""input":10,"typo":0"#),
      (r#""schema_version":1"#, r#""schema_version":2"#),
      (r#""source":"claude_session_jsonl""#, r#""source":"codex_session_jsonl""#),
      (r#""tool_calls":0"#, r#""tool_calls":false"#),
      (r#"{"input":10,"output":2,"cache_read":3,"cache_write":4}"#, "null"),
    ] {
      assert!(Summary::from_json(&good.replace(from, to)).is_none(), "accepted {to}");
    }
  }

  #[test]
  fn claude_counts_a_provider_response_once_across_transcript_blocks() {
    let root = std::env::temp_dir().join(format!("scsh-claude-dedup-{}", crate::runtime::random_nonce_6()));
    std::fs::create_dir_all(&root).unwrap();
    let text = r#"{"type":"assistant","uuid":"a","requestId":"r","message":{"id":"msg1","content":[{"type":"thinking"}],"usage":{"input_tokens":10,"output_tokens":1,"cache_read_input_tokens":20,"cache_creation_input_tokens":4}}}
{"type":"assistant","uuid":"b","requestId":"r","message":{"id":"msg1","content":[{"type":"tool_use","id":"tool1"}],"usage":{"input_tokens":10,"output_tokens":8,"cache_read_input_tokens":20,"cache_creation_input_tokens":4}}}
{"type":"assistant","uuid":"c","requestId":"r","message":{"id":"msg1","content":[{"type":"tool_use","id":"tool2"}],"usage":{"input_tokens":10,"output_tokens":8,"cache_read_input_tokens":20,"cache_creation_input_tokens":4}}}
"#;
    std::fs::write(root.join("parent.jsonl"), text).unwrap();
    std::fs::write(root.join("copied-history.jsonl"), text).unwrap();
    let summary = crate::daemon::usage::claude_session_summary(&root).unwrap();
    assert_eq!(summary.llm_round_trips, Some(1));
    assert_eq!(summary.tool_calls, Some(2));
    assert_eq!(summary.tokens, Some(Tokens { input: 10, output: 8, cache_read: 20, cache_write: Some(4) }));
    std::fs::remove_dir_all(root).unwrap();
  }

  #[test]
  fn native_cache_buckets_are_validated_and_absence_is_not_zero() {
    let old_codex = json::parse(r#"{"input_tokens":100,"cached_input_tokens":40,"output_tokens":3}"#).unwrap();
    assert_eq!(codex_tokens(&old_codex).unwrap().cache_write, None);
    let new_codex =
      json::parse(r#"{"input_tokens":100,"cached_input_tokens":40,"cache_write_input_tokens":5,"output_tokens":3}"#)
        .unwrap();
    assert_eq!(codex_tokens(&new_codex).unwrap().input, 55);
    for text in [
      r#"{"input_tokens":3,"cached_input_tokens":4,"output_tokens":1}"#,
      r#"{"input_tokens":100,"cached_input_tokens":40,"cache_write_input_tokens":-1,"output_tokens":3}"#,
    ] {
      assert!(codex_tokens(&json::parse(text).unwrap()).is_none());
    }
  }

  #[test]
  fn codex_forked_history_is_not_counted_as_fresh_spend() {
    let stream = r#"{"type":"session_meta","payload":{"id":"child","forked_from_id":"parent"}}
{"type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":100,"cached_input_tokens":40,"output_tokens":3}}}}
"#;
    let summary = codex_session_summary(&[Some(stream.into())]).unwrap();
    assert!(!summary.complete);
    assert_eq!(summary.tokens, None);
    assert_eq!(summary.llm_round_trips, None);
  }

  #[cfg(unix)]
  #[test]
  fn transcript_scan_refuses_symlinks_instead_of_following_cycles() {
    let root = std::env::temp_dir().join(format!("scsh-usage-link-{}", crate::runtime::random_nonce_6()));
    std::fs::create_dir_all(&root).unwrap();
    std::os::unix::fs::symlink(&root, root.join("cycle")).unwrap();
    assert!(crate::daemon::usage::jsonl_files(&root).is_err());
    std::fs::remove_dir_all(root).unwrap();
  }

  #[cfg(unix)]
  #[test]
  fn concurrent_cursor_hooks_append_whole_json_records() {
    use std::io::Write;
    use std::process::{Command, Stdio};
    let root = std::env::temp_dir().join(format!("scsh-hooks-{}", crate::runtime::random_nonce_6()));
    std::fs::create_dir_all(&root).unwrap();
    let log = root.join("hooks.jsonl");
    let mut children = Vec::new();
    for index in 0..8 {
      let mut child = Command::new("python3")
        .args(["-c", CURSOR_HOOK_PY])
        .env("SCSH_CURSOR_HOOKS_LOG", &log)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
      let payload = object(vec![("index", number(index)), ("text", Value::String("x".repeat(10_000)))]);
      child.stdin.take().unwrap().write_all(json::write_pretty(&payload).as_bytes()).unwrap();
      children.push(child);
    }
    for child in children {
      let result = child.wait_with_output().unwrap();
      assert!(result.status.success());
      assert_eq!(result.stdout, b"{}\n");
    }
    let text = std::fs::read_to_string(&log).unwrap();
    assert_eq!(text.lines().count(), 8);
    let indexes: BTreeSet<_> = text.lines().map(|line| count(&json::parse(line).unwrap(), "index").unwrap()).collect();
    assert_eq!(indexes.len(), 8);
    std::fs::remove_dir_all(root).unwrap();
  }

  #[cfg(unix)]
  #[test]
  fn cursor_hook_only_records_events_and_never_authorizes_shutdown() {
    use std::io::Write;
    use std::process::{Command, Stdio};
    let root = std::env::temp_dir().join(format!("scsh-hook-ready-{}", crate::runtime::random_nonce_6()));
    std::fs::create_dir_all(&root).unwrap();
    let log = root.join("hooks.jsonl");
    let ready = root.join("usage-ready");
    let run = |payload: &str| {
      let mut child = Command::new("python3")
        .args(["-c", CURSOR_HOOK_PY])
        .env("SCSH_CURSOR_HOOKS_LOG", &log)
        .env("SCSH_CURSOR_USAGE_READY", &ready)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
      child.stdin.take().unwrap().write_all(payload.as_bytes()).unwrap();
      assert!(child.wait_with_output().unwrap().status.success());
    };
    run(
      r#"{"hook_event_name":"stop","status":"completed","input_tokens":null,"output_tokens":2,"cache_read_tokens":3,"cache_write_tokens":0}"#,
    );
    assert!(!ready.exists(), "incomplete counters must not release teardown");
    run(
      r#"{"hook_event_name":"stop","status":"completed","input_tokens":10,"output_tokens":2,"cache_read_tokens":3,"cache_write_tokens":0}"#,
    );
    assert!(!ready.exists(), "hooks must never publish a sticky readiness marker");
    let text = std::fs::read_to_string(&log).unwrap();
    assert_eq!(text.lines().count(), 2);
    assert!(!text.contains("scsh_result_revision"), "the hook log contains only Cursor's event data");
    std::fs::remove_dir_all(root).unwrap();
  }

  #[test]
  fn cursor_completed_stop_survives_its_trailing_response_and_session_end() {
    let stream = r#"{"hook_event_name":"stop","conversation_id":"c","generation_id":"g","status":"completed","input_tokens":294213,"output_tokens":8032,"cache_read_tokens":171008,"cache_write_tokens":0}
{"hook_event_name":"afterAgentResponse","conversation_id":"c","generation_id":"g"}
{"hook_event_name":"sessionEnd","conversation_id":"c","status":"completed"}
"#;
    let summary = cursor_hook_summary(stream);
    assert!(summary.complete);
    assert_eq!(
      summary.tokens,
      Some(Tokens { input: 123_205, output: 8_032, cache_read: 171_008, cache_write: Some(0) })
    );

    let without_generation = stream.replace(r#","generation_id":"g""#, "");
    assert!(cursor_hook_summary(&without_generation).complete, "generation ids are optional in Cursor hook payloads");

    let resumed = stream.replacen(
      r#"{"hook_event_name":"afterAgentResponse","conversation_id":"c","generation_id":"g"}"#,
      r#"{"hook_event_name":"afterAgentResponse","conversation_id":"c","generation_id":"next"}"#,
      1,
    );
    assert!(!cursor_hook_summary(&resumed).complete, "a later generation still invalidates the completed stop");
  }

  #[test]
  fn resumed_cursor_work_invalidates_a_previous_completed_stop() {
    let text = format!(
      r#"{HOOKS}{{"hook_event_name":"postToolUse","conversation_id":"c","generation_id":"g4","tool_use_id":"t4"}}
{{"hook_event_name":"sessionEnd","conversation_id":"c","reason":"completed"}}
"#
    );
    assert!(!cursor_hook_summary(&text).complete);
    let stale_stop = format!(
      r#"{text}{{"hook_event_name":"stop","conversation_id":"c","generation_id":"g3","status":"completed","input_tokens":10,"output_tokens":2,"cache_read_tokens":0,"cache_write_tokens":0}}"#
    );
    assert!(!cursor_hook_summary(&stale_stop).complete, "an old generation's late stop cannot finish new work");
  }

  #[test]
  fn native_turn_boundaries_reject_resumed_work() {
    let claude = r#"{"type":"assistant","message":{"stop_reason":"end_turn"}}
"#;
    assert!(turn_finished(crate::config::Harness::Claude, claude));
    assert!(!turn_finished(crate::config::Harness::Claude, &format!("{claude}{{\"type\":\"user\"}}")));
    let codex = r#"{"type":"event_msg","payload":{"type":"task_complete"}}
"#;
    assert!(turn_finished(crate::config::Harness::Codex, codex));
    assert!(!turn_finished(
      crate::config::Harness::Codex,
      &format!("{codex}{{\"payload\":{{\"type\":\"task_started\"}}}}")
    ));
  }
}
