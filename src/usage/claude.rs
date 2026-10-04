//! One native parser for both aggregate counters and auditable response ledgers.

use super::*;

/// A source record contributing to a logical provider response.
#[derive(Debug, Clone)]
struct Source {
  /// Index into the manifest's ordered transcript inventory.
  transcript: usize,
  /// One-based line number; repeated streamed updates retain their provenance.
  line: usize,
  /// Native session/subagent identifier, absent when the CLI omitted it.
  session: Option<String>,
  /// Native timestamp, retained without guessing a missing time.
  timestamp: Option<String>,
}

/// One response, identified by the provider message and request IDs.
#[derive(Debug, Clone)]
struct Response {
  /// Observed model, independent of the requested alias.
  model: Option<String>,
  /// Maximum streamed counters, with disjoint input/cache buckets.
  tokens: Tokens,
  /// Tool IDs map to observed tool names; arguments and results are never exported.
  tools: BTreeMap<String, Option<String>>,
  /// Every contributing source record, including streaming duplicates.
  sources: Vec<Source>,
}

/// Native response evidence for exactly one attempt, including subagent files.
pub(crate) struct Ledger {
  /// Whether supervision observed the attempt's final accounting boundary.
  accounting_complete: bool,
  /// Stable provider identities, shared with aggregate deduplication.
  responses: BTreeMap<(String, String), Response>,
  /// Unreadable files or malformed records prevent an exact total.
  invalid: Vec<(usize, Option<usize>)>,
}

impl Ledger {
  pub fn parse(transcripts: &[Option<String>]) -> Self {
    let mut ledger = Self { accounting_complete: true, responses: BTreeMap::new(), invalid: Vec::new() };
    for (transcript, text) in transcripts.iter().enumerate() {
      let Some(text) = text else {
        ledger.invalid.push((transcript, None));
        continue;
      };
      for (index, line) in text.lines().enumerate().filter(|(_, line)| !line.trim().is_empty()) {
        if ledger.record(transcript, index + 1, line).is_err() {
          ledger.invalid.push((transcript, Some(index + 1)));
        }
      }
    }
    ledger
  }

  pub fn with_accounting_complete(mut self, complete: bool) -> Self {
    self.accounting_complete = complete;
    self
  }

  fn record(&mut self, transcript: usize, line: usize, text: &str) -> Result<(), ()> {
    let event = json::parse(text).map_err(|_| ())?;
    if string(&event, "type") != Some("assistant") {
      return Ok(());
    }
    let Some(message) = field(&event, "message") else { return Ok(()) };
    if string(message, "model") == Some("<synthetic>") {
      return Ok(());
    }
    let id = string(message, "id").or_else(|| string(&event, "uuid")).ok_or(())?;
    let tokens = field(message, "usage").and_then(claude_tokens).ok_or(())?;
    let key = (id.to_string(), string(&event, "requestId").unwrap_or("").to_string());
    let response = self.responses.entry(key).or_insert_with(|| Response {
      model: string(message, "model").map(str::to_string),
      tokens: tokens.clone(),
      tools: BTreeMap::new(),
      sources: Vec::new(),
    });
    response.tokens.input = response.tokens.input.max(tokens.input);
    response.tokens.output = response.tokens.output.max(tokens.output);
    response.tokens.cache_read = response.tokens.cache_read.max(tokens.cache_read);
    response.tokens.cache_write = response.tokens.cache_write.max(tokens.cache_write);
    response.sources.push(Source {
      transcript,
      line,
      session: string(&event, "sessionId").map(str::to_string),
      timestamp: string(&event, "timestamp").map(str::to_string),
    });
    if let Some(Value::Array(content)) = field(message, "content") {
      for block in content {
        if string(block, "type") == Some("tool_use") {
          if let Some(id) = string(block, "id") {
            response.tools.insert(id.to_string(), string(block, "name").map(str::to_string));
          }
        }
      }
    }
    Ok(())
  }

  pub fn summary(&self) -> Option<Summary> {
    if self.responses.is_empty() {
      return None;
    }
    let mut tokens = Tokens { input: 0, output: 0, cache_read: 0, cache_write: Some(0) };
    let mut tools = BTreeSet::new();
    for response in self.responses.values() {
      if !add_tokens(&mut tokens, &response.tokens) {
        return Some(unavailable(Harness::ClaudeCode));
      }
      tools.extend(response.tools.keys());
    }
    Some(Summary {
      harness: Harness::ClaudeCode,
      complete: self.invalid.is_empty() && self.accounting_complete,
      tokens: self.invalid.is_empty().then_some(tokens),
      llm_round_trips: Some(self.responses.len() as u64),
      tool_calls: Some(tools.len() as u64),
    })
  }

  /// Versioned diagnostic, separate from the public aggregate API. Null means unknown.
  pub fn to_json(&self, attempt: &str, paths: &[String]) -> String {
    let rows = self
      .responses
      .iter()
      .map(|((message, request), response)| {
        obj(vec![
          ("message_id", Value::String(message.clone())),
          ("request_id", optional((!request.is_empty()).then_some(request.as_str()))),
          ("model", optional(response.model.as_deref())),
          ("input", number(response.tokens.input)),
          ("output", number(response.tokens.output)),
          ("cache_read", number(response.tokens.cache_read)),
          ("cache_write", response.tokens.cache_write.map(number).unwrap_or(Value::Null)),
          (
            "tools",
            Value::Array(
              response
                .tools
                .iter()
                .map(|(id, name)| {
                  obj(vec![
                    ("id", Value::String(id.clone())),
                    ("name", optional(name.as_deref())),
                    ("mcp", name.as_ref().map(|n| Value::Bool(n.starts_with("mcp__"))).unwrap_or(Value::Null)),
                  ])
                })
                .collect(),
            ),
          ),
          (
            "sources",
            Value::Array(
              response
                .sources
                .iter()
                .map(|source| {
                  obj(vec![
                    ("transcript", number(source.transcript as u64)),
                    ("line", number(source.line as u64)),
                    ("session_id", optional(source.session.as_deref())),
                    ("timestamp", optional(source.timestamp.as_deref())),
                  ])
                })
                .collect(),
            ),
          ),
        ])
      })
      .collect();
    json::write_pretty(&obj(vec![(
      "ClaudeUsageLedger",
      obj(vec![
        ("schema_version", number(1)),
        ("attempt", Value::String(attempt.to_string())),
        ("transcripts", Value::Array(paths.iter().cloned().map(Value::String).collect())),
        ("responses", Value::Array(rows)),
        (
          "invalid_records",
          Value::Array(
            self
              .invalid
              .iter()
              .map(|(file, line)| {
                obj(vec![
                  ("transcript", number(*file as u64)),
                  ("line", line.map(|n| number(n as u64)).unwrap_or(Value::Null)),
                ])
              })
              .collect(),
          ),
        ),
        ("aggregate", self.summary().map(|s| summary_value(&s)).unwrap_or(Value::Null)),
      ]),
    )]))
  }
}

fn obj(fields: Vec<(&str, Value)>) -> Value {
  Value::Object(fields.into_iter().map(|(key, value)| (key.to_string(), value)).collect())
}

fn optional(value: Option<&str>) -> Value {
  value.map(|s| Value::String(s.to_string())).unwrap_or(Value::Null)
}

fn number(value: u64) -> Value {
  Value::Number(value as f64)
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn ledger_reconciles_streams_requests_subagents_and_mcp_calls() {
    let record = r#"{"type":"assistant","sessionId":"parent","requestId":"r1","timestamp":"2026-10-04T00:00:00Z","message":{"id":"m1","model":"opus","usage":{"input_tokens":100,"output_tokens":4,"cache_read_input_tokens":20,"cache_creation_input_tokens":30},"content":[{"type":"tool_use","id":"t1","name":"mcp__facts__lookup"}]}}"#;
    let updated = record.replace("\"output_tokens\":4", "\"output_tokens\":9");
    let child = record
      .replace("parent", "child")
      .replace("r1", "r2")
      .replace("m1", "m2")
      .replace("t1", "t2")
      .replace("mcp__facts__lookup", "Read");
    let transcripts = vec![Some(format!("{record}\n{updated}\n")), Some(child.clone()), Some(child)];
    let ledger = Ledger::parse(&transcripts);
    let summary = ledger.summary().unwrap();
    assert_eq!(summary.tokens, Some(Tokens { input: 200, output: 13, cache_read: 40, cache_write: Some(60) }));
    assert_eq!(summary.llm_round_trips, Some(2));
    assert_eq!(summary.tool_calls, Some(2));
    assert_eq!(super::super::claude_session_summary(&transcripts), Some(summary));
    let json = ledger.to_json("attempt-1", &["parent.jsonl".into(), "child.jsonl".into(), "copy.jsonl".into()]);
    assert_eq!(json.matches("\"mcp\": true").count(), 1);
    assert_eq!(json.matches("\"mcp\": false").count(), 1);
    assert_eq!(ledger.responses[&("m1".into(), "r1".into())].sources.len(), 2);
    let mut malformed = transcripts;
    malformed.extend([None, Some("broken JSON".into())]);
    let incomplete = Ledger::parse(&malformed);
    assert!(!incomplete.summary().unwrap().complete);
    assert_eq!(incomplete.summary().unwrap().tokens, None);
    assert_eq!(incomplete.invalid.len(), 2);
    assert!(Ledger::parse(&[None]).summary().is_none());
    assert_ne!(ledger.to_json("attempt-1", &[]), ledger.to_json("attempt-2", &[]));
    let interrupted = Ledger::parse(&[Some(updated)]).with_accounting_complete(false).summary().unwrap();
    assert!(!interrupted.complete);
    assert_eq!(interrupted.tokens.unwrap().input, 100, "retain observed counters on an interrupted attempt");
  }
}
