//! Local artifact collection; pure accounting and schema validation live in `crate::usage`.

use crate::usage::{cursor_hook_summary, unavailable, Harness, Summary};
use std::path::{Path, PathBuf};

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
