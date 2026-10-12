//! The transcript body of a headless run: the normalized [`crate::transcript`] entries as one
//! server-rendered list. The live job page fetches it from `/transcript/<session>/<proc>` (and
//! re-fetches while the event stream grows); the job export inlines the very same markup, so
//! the offline snapshot reads exactly like the live page.

use super::escape::esc;
use crate::config::Harness;
use crate::transcript::{Entry, Transcript};

/// The `<ol>` for one run's event stream. `running` adds the "still working" tail line.
#[cfg(test)]
pub(crate) fn transcript_list_html(harness: Harness, stream: &str, running: bool) -> String {
  list_html(&crate::transcript::parse(harness, stream), running)
}

fn list_html(t: &Transcript, running: bool) -> String {
  let mut out = String::from("<ol class=\"tx-list\">\n");
  for entry in &t.entries {
    out.push_str(&entry_html(entry));
  }
  if t.entries.is_empty() {
    let text = if running { "Waiting for the first event…" } else { "The run printed no events." };
    out.push_str(&format!("<li class=\"tx tx--empty dim\">{text}</li>\n"));
  } else if running && !t.finished {
    out.push_str(
      "<li class=\"tx tx--working dim\"><span class=\"tx-tag\">…</span><span class=\"tx-line\">working</span></li>\n",
    );
  }
  out.push_str("</ol>\n");
  out
}

/// The whole transcript body: a toolbar (the raw stream's download link when `raw_href` is
/// given, then message and tool-call counts) above [`transcript_list_html`]. The stream is
/// parsed once for both.
pub(crate) fn transcript_body_html(harness: Harness, stream: &str, running: bool, raw_href: Option<&str>) -> String {
  let t = crate::transcript::parse(harness, stream);
  let download =
    raw_href.map(|href| format!("<a href=\"{}\" download>⬇ events.jsonl</a>", esc(href))).unwrap_or_default();
  format!(
    "<div class=\"cast-toolbar transcript-toolbar\">{download}<span class=\"tx-counts dim\">{}</span>\
<span class=\"cast-keys dim\">{} · headless</span></div>\n{}",
    esc(&counts_phrase(&t)),
    esc(harness.as_str()),
    list_html(&t, running)
  )
}

/// One line for the transcript toolbar: how many messages and tool calls the run made.
#[cfg(test)]
pub(crate) fn transcript_counts(harness: Harness, stream: &str) -> String {
  counts_phrase(&crate::transcript::parse(harness, stream))
}

fn counts_phrase(t: &Transcript) -> String {
  let tools = t.entries.iter().filter(|e| matches!(e, Entry::Tool { .. })).count();
  let failed = t.entries.iter().filter(|e| matches!(e, Entry::Tool { ok: Some(false), .. })).count();
  let messages = t.entries.iter().filter(|e| matches!(e, Entry::Text { .. })).count();
  let mut parts = vec![plural(messages, "message"), plural(tools, "tool call")];
  if failed > 0 {
    parts.push(format!("{failed} failed"));
  }
  parts.join(" · ")
}

fn plural(n: usize, noun: &str) -> String {
  format!("{n} {noun}{}", if n == 1 { "" } else { "s" })
}

fn entry_html(entry: &Entry) -> String {
  match entry {
    Entry::Session { detail } => line("session", "tx--session", detail),
    Entry::Text { text } => format!(
      "<li class=\"tx tx--text\"><span class=\"tx-tag\">agent</span><div class=\"tx-prose\">{}</div></li>\n",
      super::markdown_to_html(text)
    ),
    // One-line reasoning needs no expander: the line is the whole thought.
    Entry::Thinking { text } if !text.contains('\n') && text.chars().count() <= 120 => {
      line("thinking", "tx--thinking", text)
    }
    Entry::Thinking { text } => format!(
      "<li class=\"tx tx--thinking\"><details><summary><span class=\"tx-tag\">thinking</span>\
<span class=\"tx-line dim\">{}</span></summary><div class=\"tx-plain\">{}</div></details></li>\n",
      esc(&first_line(text)),
      esc(text)
    ),
    Entry::Tool { name, summary, input, output, ok, .. } => {
      let (state, mark) = match ok {
        Some(true) => ("tx--ok", "✓"),
        Some(false) => ("tx--fail", "✗"),
        None => ("tx--running", "…"),
      };
      let output_html = match output {
        Some(out) => format!("<div class=\"tx-io-label\">output</div><pre>{}</pre>", esc(out)),
        None if ok.is_some() => "<div class=\"tx-io-label\">output</div><pre class=\"dim\">(none)</pre>".into(),
        None => String::new(),
      };
      format!(
        "<li class=\"tx tx--tool {state}\"><details><summary><span class=\"tx-tag\">tool</span>\
<code class=\"tx-name\">{name}</code><span class=\"tx-line\">{summary}</span><span class=\"tx-mark\">{mark}</span></summary>\
<div class=\"tx-io\"><div class=\"tx-io-label\">input</div><pre>{input}</pre>{output_html}</div></details></li>\n",
        name = esc(name),
        summary = esc(summary),
        input = esc(input),
      )
    }
    Entry::Result { ok, detail } => {
      line("result", if *ok { "tx--result tx--ok" } else { "tx--result tx--fail" }, detail)
    }
    Entry::Error { text } => line("error", "tx--error tx--fail", text),
    Entry::Note { text } => line("stdout", "tx--note", text),
  }
}

fn line(tag: &str, class: &str, text: &str) -> String {
  format!(
    "<li class=\"tx {class}\"><span class=\"tx-tag\">{tag}</span><span class=\"tx-line\">{}</span></li>\n",
    esc(text)
  )
}

fn first_line(text: &str) -> String {
  let line = text.lines().next().unwrap_or("");
  if line.chars().count() > 120 {
    line.chars().take(119).collect::<String>() + "…"
  } else {
    line.to_string()
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn tools_render_collapsed_with_their_status_and_text_as_markdown() {
    let stream = r#"{"type":"assistant","message":{"content":[{"type":"text","text":"**Done** <script>x</script>"},{"type":"tool_use","id":"t1","name":"Bash","input":{"command":"ls <dir>"}}]}}
{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"t1","content":"a\nb","is_error":true}]}}
"#;
    let html = transcript_list_html(Harness::ClaudeHeadless, stream, true);
    assert!(html.contains("<strong>Done</strong>"), "{html}");
    assert!(!html.contains("<script>"), "agent prose is sanitized: {html}");
    assert!(html.contains("tx--tool tx--fail") && html.contains("ls &lt;dir&gt;"), "{html}");
    assert!(html.contains("tx--working"), "a running stream without a result shows it is still working: {html}");
    assert_eq!(transcript_counts(Harness::ClaudeHeadless, stream), "1 message · 1 tool call · 1 failed");
  }

  #[test]
  fn an_empty_stream_says_why_it_is_empty() {
    assert!(transcript_list_html(Harness::CodexHeadless, "", true).contains("Waiting for the first event"));
    assert!(transcript_list_html(Harness::CodexHeadless, "", false).contains("printed no events"));
  }
}
