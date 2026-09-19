//! Local artifact collection; pure accounting and schema validation live in `crate::usage`.

use crate::usage::{cursor_hook_summary, unavailable, Harness, Summary};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// One host-owned deadline for every harness. The container only exits after this
/// controller has saved validated counters or journaled the accounting failure.
pub struct Completion {
  /// First observation of the result starts the accounting deadline.
  started: Option<Instant>,
  /// Authorization starts a separate, bounded graceful-exit allowance.
  released: Option<Instant>,
  /// Throttle native transcript reads without delaying watchdog protection.
  next_poll: Instant,
}

impl Completion {
  pub fn new() -> Self {
    Self { started: None, released: None, next_poll: Instant::now() }
  }

  pub fn poll(&mut self, harness: crate::config::Harness, dir: &Path, result: &Path, required: bool) -> bool {
    let now = Instant::now();
    if let Some(released) = self.released {
      return now.duration_since(released) < Duration::from_secs(15);
    }
    if !result.is_file() {
      return false;
    }
    let started = *self.started.get_or_insert(now);
    if now.duration_since(started) >= Duration::from_secs(45) {
      // Even a full disk preventing the shutdown instruction must not leak a container.
      return false;
    }
    if now < self.next_poll {
      return true;
    }
    self.next_poll = now + Duration::from_millis(250);
    let expired = now.duration_since(started) >= Duration::from_secs(30);
    let ready = if required && !expired { completed_usage(harness, dir, result) } else { None };
    let error = dir.join(format!("{}.usage-error", crate::runtime::RUN_LOG_REL));
    let unsupported = matches!(harness, crate::config::Harness::Grok | crate::config::Harness::Opencode);
    let saved = if let Some(summary) = ready {
      crate::atomic_write(
        &dir.join(format!("{}.usage-final", crate::runtime::RUN_LOG_REL)),
        summary.to_json().as_bytes(),
      )
      .is_ok()
    } else {
      false
    };
    if !required || saved || unsupported || expired {
      if required && !saved {
        // A timeout remains a failure even if late counters appear during teardown.
        let detail = if unsupported {
          "unavailable: this harness has no native accounting adapter; use SCSH_NO_USAGE=1 to run without counters"
        } else {
          "timeout: native accounting did not complete within 30s of the result"
        };
        if crate::atomic_write(&error, detail.as_bytes()).is_err() {
          return false;
        }
      }
      if crate::atomic_write(&dir.join(format!("{}.shutdown", crate::runtime::RUN_LOG_REL)), b"exit").is_ok() {
        self.released = Some(now);
      }
    }
    true
  }
}

/// Each attempt owns a fresh transcript tree. Require it to change at or after the
/// result and require its latest logical turn to be terminal.
fn completed_usage(harness: crate::config::Harness, dir: &Path, result: &Path) -> Option<Summary> {
  let result_time = std::fs::metadata(result).ok()?.modified().ok()?;
  let paths = match harness {
    crate::config::Harness::Cursor => vec![dir.join(format!("{}.cursor-hooks.jsonl", crate::runtime::RUN_LOG_REL))],
    crate::config::Harness::Claude => {
      jsonl_files(&dir.join(crate::runtime::CLAUDE_AUTH_REL).join(".claude/projects")).ok()?
    }
    crate::config::Harness::Codex => jsonl_files(&dir.join(crate::runtime::CODEX_FORWARD_REL).join("sessions")).ok()?,
    _ => return None,
  };
  let mut fresh = false;
  let mut streams = Vec::new();
  for path in &paths {
    let before = std::fs::metadata(path).ok()?;
    let stream = std::fs::read_to_string(path).ok()?;
    let after = std::fs::metadata(path).ok()?;
    if before.len() != after.len() || before.modified().ok()? != after.modified().ok()? {
      return None;
    }
    fresh |= after.modified().ok()? >= result_time;
    if harness != crate::config::Harness::Cursor && !crate::usage::turn_finished(harness, &stream) {
      return None;
    }
    streams.push(Some(stream));
  }
  let summary = match harness {
    crate::config::Harness::Cursor => cursor_hook_summary(streams.first()?.as_deref()?),
    crate::config::Harness::Claude => crate::usage::claude_session_summary(&streams)?,
    crate::config::Harness::Codex => crate::usage::codex_session_summary(&streams)?,
    _ => return None,
  };
  (fresh && summary.complete && summary.tokens.is_some()).then_some(summary)
}

/// Read the native, local accounting artifacts written by one interactive harness run.
/// This performs no network request and no model call.
pub fn from_run_dir(harness: crate::config::Harness, run_dir: &Path) -> Option<Summary> {
  match harness {
    crate::config::Harness::Claude => Some(
      claude_session_summary(&run_dir.join(crate::runtime::CLAUDE_AUTH_REL).join(".claude/projects"))
        .unwrap_or_else(|| unavailable(Harness::ClaudeCode)),
    ),
    crate::config::Harness::Codex => Some(
      codex_session_summary(&run_dir.join(crate::runtime::CODEX_FORWARD_REL).join("sessions"))
        .unwrap_or_else(|| unavailable(Harness::Codex)),
    ),
    crate::config::Harness::Cursor => {
      let path = run_dir.join(format!("{}.cursor-hooks.jsonl", crate::runtime::RUN_LOG_REL));
      Some(
        std::fs::read_to_string(path)
          .ok()
          .map(|stream| cursor_hook_summary(&stream))
          .unwrap_or_else(|| unavailable(Harness::Cursor)),
      )
    }
    crate::config::Harness::Grok | crate::config::Harness::Opencode => None,
  }
}

pub(crate) fn claude_session_summary(root: &Path) -> Option<Summary> {
  let transcripts: Vec<_> =
    jsonl_files(root).ok()?.into_iter().map(|path| std::fs::read_to_string(path).ok()).collect();
  crate::usage::claude_session_summary(&transcripts)
}

pub(crate) fn codex_session_summary(root: &Path) -> Option<Summary> {
  let transcripts: Vec<_> =
    jsonl_files(root).ok()?.into_iter().map(|path| std::fs::read_to_string(path).ok()).collect();
  crate::usage::codex_session_summary(&transcripts)
}

pub(crate) fn jsonl_files(root: &Path) -> std::io::Result<Vec<PathBuf>> {
  fn visit(dir: &Path, out: &mut Vec<PathBuf>, depth: usize) -> std::io::Result<()> {
    if depth > 32 {
      return Err(std::io::Error::other("harness transcript tree exceeds 32 directory levels"));
    }
    for entry in std::fs::read_dir(dir)? {
      let entry = entry?;
      let path = entry.path();
      let kind = entry.file_type()?;
      if kind.is_symlink() {
        return Err(std::io::Error::other("harness transcript tree contains a symlink"));
      }
      if kind.is_dir() {
        visit(&path, out, depth + 1)?;
      } else if kind.is_file() && path.extension().and_then(|ext| ext.to_str()) == Some("jsonl") {
        out.push(path);
      }
    }
    Ok(())
  }
  let mut out = Vec::new();
  visit(root, &mut out, 0)?;
  out.sort();
  Ok(out)
}

#[cfg(test)]
mod completion_tests {
  use super::*;
  use crate::config::Harness as Agent;

  struct Run(PathBuf);
  impl Run {
    fn new() -> Self {
      let dir = std::env::temp_dir().join(format!("scsh-accounting-{}", crate::runtime::random_nonce_6()));
      std::fs::create_dir_all(dir.join("tmp")).unwrap();
      std::fs::write(dir.join("tmp/result.json"), "{}").unwrap();
      Self(dir)
    }
    fn artifact(&self, suffix: &str) -> PathBuf {
      self.0.join(format!("{}.{suffix}", crate::runtime::RUN_LOG_REL))
    }
    fn poll(&self, state: &mut Completion, harness: Agent, required: bool) -> bool {
      state.next_poll = Instant::now();
      state.poll(harness, &self.0, &self.0.join("tmp/result.json"), required)
    }
  }
  impl Drop for Run {
    fn drop(&mut self) {
      let _ = std::fs::remove_dir_all(&self.0);
    }
  }

  #[test]
  fn delayed_native_counters_release_each_supported_harness_only_after_terminal_snapshot() {
    for (harness, path, stream) in [
      (
        Agent::Cursor,
        format!("{}.cursor-hooks.jsonl", crate::runtime::RUN_LOG_REL),
        r#"{"hook_event_name":"stop","conversation_id":"c","status":"completed","input_tokens":10,"output_tokens":2,"cache_read_tokens":3,"cache_write_tokens":0}"#,
      ),
      (
        Agent::Claude,
        format!("{}/.claude/projects/p/run.jsonl", crate::runtime::CLAUDE_AUTH_REL),
        r#"{"type":"assistant","message":{"id":"m","stop_reason":"end_turn","usage":{"input_tokens":10,"output_tokens":2,"cache_read_input_tokens":3,"cache_creation_input_tokens":0}}}"#,
      ),
      (
        Agent::Codex,
        format!("{}/sessions/run.jsonl", crate::runtime::CODEX_FORWARD_REL),
        r#"{"type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":10,"output_tokens":2,"cached_input_tokens":3}}}}
{"type":"event_msg","payload":{"type":"task_complete"}}"#,
      ),
    ] {
      let run = Run::new();
      let mut state = Completion::new();
      assert!(run.poll(&mut state, harness, true));
      assert!(!run.artifact("shutdown").exists());
      state.started = Some(Instant::now() - Duration::from_secs(29));
      assert!(run.poll(&mut state, harness, true));
      assert!(!run.artifact("shutdown").exists(), "the full accounting budget remains available");
      let transcript = run.0.join(path);
      std::fs::create_dir_all(transcript.parent().unwrap()).unwrap();
      std::fs::write(transcript, stream).unwrap();
      assert!(run.poll(&mut state, harness, true));
      assert!(run.artifact("shutdown").exists(), "{harness:?}");
      let saved = Summary::from_json(&std::fs::read_to_string(run.artifact("usage-final")).unwrap()).unwrap();
      assert!(saved.complete);
      assert!(!run.artifact("usage-error").exists());
    }
  }

  #[test]
  fn all_harnesses_obey_required_accounting_and_opt_out() {
    for harness in [Agent::Cursor, Agent::Claude, Agent::Codex, Agent::Grok, Agent::Opencode] {
      let run = Run::new();
      let mut state = Completion::new();
      state.started = Some(Instant::now() - Duration::from_secs(31));
      assert!(run.poll(&mut state, harness, true));
      assert!(run.artifact("usage-error").exists());
      assert!(run.artifact("shutdown").exists());
      state.released = Some(Instant::now() - Duration::from_secs(16));
      assert!(!run.poll(&mut state, harness, true), "teardown remains bounded");
      let fast = Run::new();
      assert!(fast.poll(&mut Completion::new(), harness, false));
      assert!(fast.artifact("shutdown").exists());
      assert!(!fast.artifact("usage-error").exists());
    }
  }

  #[test]
  fn old_cursor_stop_cannot_release_a_new_result() {
    let run = Run::new();
    let hooks = run.artifact("cursor-hooks.jsonl");
    std::fs::write(&hooks, r#"{"hook_event_name":"stop","status":"completed","input_tokens":10,"output_tokens":2,"cache_read_tokens":0,"cache_write_tokens":0}"#).unwrap();
    std::fs::File::options()
      .write(true)
      .open(hooks)
      .unwrap()
      .set_modified(std::time::SystemTime::now() - Duration::from_secs(60))
      .unwrap();
    assert!(run.poll(&mut Completion::new(), Agent::Cursor, true));
    assert!(!run.artifact("shutdown").exists());
  }

  #[test]
  fn real_cursor_hook_order_releases_complete_counters() {
    let run = Run::new();
    std::fs::write(run.artifact("cursor-hooks.jsonl"), r#"{"hook_event_name":"stop","conversation_id":"c","generation_id":"g","status":"completed","input_tokens":294213,"output_tokens":8032,"cache_read_tokens":171008,"cache_write_tokens":0}
{"hook_event_name":"afterAgentResponse","conversation_id":"c","generation_id":"g"}
{"hook_event_name":"sessionEnd","conversation_id":"c","status":"completed"}
"#).unwrap();
    let mut completion = Completion::new();
    assert!(run.poll(&mut completion, Agent::Cursor, true));
    assert!(run.artifact("shutdown").exists());
    let saved = Summary::from_json(&std::fs::read_to_string(run.artifact("usage-final")).unwrap()).unwrap();
    let tokens = saved.tokens.unwrap();
    assert_eq!(tokens.input, 123_205);
    assert_eq!(tokens.output, 8_032);
    assert_eq!(tokens.cache_read, 171_008);
  }
}
