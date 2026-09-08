//! Publish a completed browser review using the host's GitHub credentials.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::process::Command;

use super::github::PullRequest;
use super::model::{ProcStatus, Session};
use crate::json::{parse, quote, Value};

fn field<'a>(value: &'a Value, key: &str) -> Option<&'a Value> {
  match value {
    Value::Object(fields) => fields.iter().find(|(k, _)| k == key).map(|(_, v)| v),
    _ => None,
  }
}

fn string(value: &Value, key: &str) -> Result<String, String> {
  match field(value, key) {
    Some(Value::String(s)) => Ok(s.clone()),
    _ => Err(format!("missing string field '{key}'")),
  }
}

fn gh(args: &[&str]) -> Result<Value, String> {
  let out = Command::new("gh").args(args).output().map_err(|e| format!("could not run gh: {e}"))?;
  if !out.status.success() {
    return Err(String::from_utf8_lossy(&out.stderr).trim().to_string());
  }
  parse(&String::from_utf8_lossy(&out.stdout))
}

/// Added and context lines on the right side of a GitHub file patch.
fn right_lines(patch: &str) -> BTreeSet<u64> {
  let mut line = 0;
  let mut lines = BTreeSet::new();
  for text in patch.lines() {
    if text.starts_with("@@ ") {
      line = text
        .split_whitespace()
        .find_map(|s| s.strip_prefix('+'))
        .and_then(|s| s.split(',').next())
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    } else if line > 0 && (text.starts_with('+') || text.starts_with(' ')) {
      lines.insert(line);
      line += 1;
    }
  }
  lines
}

/// Reject incomplete fleets, including successful processes with malformed/missing results.
fn findings(root: &Path, session: &Session) -> Result<(Vec<Value>, bool), String> {
  if session.skills.is_empty() {
    return Err("no expected review routes were recorded".into());
  }
  let mut issues = Vec::new();
  let mut excellent = 0;
  let mut good = 0;
  for skill in &session.skills {
    let proc = session
      .procs
      .iter()
      .rev()
      .find(|p| p.skill_name.as_deref() == Some(&skill.name))
      .ok_or_else(|| format!("missing route {}", skill.name))?;
    if !matches!(proc.status, ProcStatus::Ok | ProcStatus::Graceful) {
      return Err(format!("route {} did not succeed", skill.name));
    }
    let path = root.join(format!("{}.json", skill.name));
    let text = std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    let value = parse(&text)?;
    let result = field(&value, "result").ok_or("missing review result")?;
    // Shared scores: 5 = excellent, 4 = good; lower valid grades cannot approve.
    match crate::fleet::grade_score(&string(result, "grade")?) {
      Some(5) => excellent += 1,
      Some(4) => good += 1,
      Some(_) => {}
      _ => return Err(format!("route {} has an invalid grade", skill.name)),
    }
    let Some(Value::Array(found)) = field(&value, "issues") else {
      return Err(format!("route {} omitted its issues", skill.name));
    };
    if field(result, "issues_found") != Some(&Value::Number(found.len() as f64)) {
      return Err(format!("route {} has an inconsistent issue count", skill.name));
    }
    issues.extend(found.clone());
  }
  Ok((issues, excellent + good == session.skills.len() && excellent >= good))
}

// The local review input is a file; GitHub readers know it as the PR description.
fn github_wording(text: &str) -> String {
  text.replace("`PR-DESCRIPTION.md`", "PR description").replace("PR-DESCRIPTION.md", "PR description")
}

fn payload(head: &str, event: &str, issues: &[Value], files: &[Value], marker: &str) -> Result<String, String> {
  let mut anchors = BTreeMap::new();
  for file in files {
    if let (Ok(path), Ok(patch)) = (string(file, "filename"), string(file, "patch")) {
      anchors.insert(path, right_lines(&patch));
    }
  }
  // Group repeated locations into one comment, preserving distinct observations and suggestions.
  let mut grouped: BTreeMap<(String, u64), BTreeSet<String>> = BTreeMap::new();
  for issue in issues {
    let path = string(issue, "file")?;
    let line = match field(issue, "line") {
      Some(Value::Number(n)) if n.is_finite() && *n >= 0.0 && n.fract() == 0.0 => *n as u64,
      _ => return Err("review issue has an invalid line".into()),
    };
    let description = github_wording(&string(issue, "description")?);
    if description.trim().is_empty() {
      return Err("review issue has an empty description".into());
    }
    let suggestion = github_wording(&string(issue, "suggestion").unwrap_or_default());
    let body = if suggestion.is_empty() { description } else { format!("{description}\n\nSuggestion: {suggestion}") };
    grouped.entry((path, line)).or_default().insert(body);
  }
  let mut body = if issues.is_empty() {
    "Looks good to me. I did not find any issues to raise.".to_string()
  } else {
    "Thanks for the change. Here are the observations and suggestions from the review.".to_string()
  };
  let mut comments = Vec::new();
  for ((path, line), observations) in grouped {
    let text = observations.into_iter().collect::<Vec<_>>().join("\n\n");
    if path != "PR-DESCRIPTION.md"
      && anchors.get(&path).is_some_and(|lines| lines.contains(&line))
      && comments.len() < 50
    {
      comments.push(format!(
        "{{\"path\":{},\"line\":{line},\"side\":\"RIGHT\",\"body\":{}}}",
        quote(&path),
        quote(&text)
      ));
    } else {
      let location = if path == "PR-DESCRIPTION.md" {
        "PR description".to_string()
      } else if path.starts_with('<') {
        "Overall change".to_string()
      } else {
        format!("{path}:{line}")
      };
      body.push_str(&format!("\n\n{location}\n\n{text}"));
    }
  }
  body.push_str(&format!("\n\n{marker}"));
  Ok(format!(
    "{{\"commit_id\":{},\"event\":{},\"body\":{},\"comments\":[{}]}}",
    quote(head),
    quote(event),
    quote(&body),
    comments.join(",")
  ))
}

/// One review per authenticated user and reviewed revision. A network error is not retried:
/// a later attempt checks GitHub first, covering a successful POST whose response was lost.
pub fn publish(root: &Path, pr: &PullRequest, session: &Session) -> Result<String, String> {
  publish_with(root, &crate::runtime::session_results_dir(&session.id), pr, session, gh)
}

fn publish_with(
  root: &Path, results: &Path, pr: &PullRequest, session: &Session,
  mut gh: impl FnMut(&[&str]) -> Result<Value, String>,
) -> Result<String, String> {
  let (issues, approval_bar) = findings(results, session)?;
  let receipt =
    parse(&std::fs::read_to_string(root.join("tmp/gh-gorgeous-review-browser.json")).map_err(|e| e.to_string())?)?;
  let head = string(&receipt, "reviewed_head")?;
  let base = string(&receipt, "base_head")?;
  let metadata = gh(&["pr", "view", &pr.url, "--json", "headRefOid,baseRefOid,state,isDraft,author"])?;
  if string(&metadata, "headRefOid")? != head || string(&metadata, "baseRefOid")? != base {
    return Err("the PR head or base changed since review; start a fresh review before publishing".into());
  }
  let login = string(&gh(&["api", "user"])?, "login")?;
  let endpoint = format!("repos/{}/{}/pulls/{}/reviews", pr.reference.owner, pr.reference.repo, pr.reference.number);
  let marker = format!("<!-- review-head:{head} -->");
  let reviews = gh(&["api", &endpoint, "--paginate", "--slurp"])?;
  let mut approved = false;
  if let Value::Array(pages) = reviews {
    for page in pages {
      if let Value::Array(rows) = page {
        for review in rows {
          if field(&review, "user").and_then(|u| string(u, "login").ok()).as_deref() != Some(&login) {
            continue;
          }
          let state = string(&review, "state")?;
          if state != "PENDING" && string(&review, "body")?.contains(&marker) {
            std::fs::write(root.join("tmp/gh-review-published.json"), crate::json::write_pretty(&review))
              .map_err(|e| e.to_string())?;
            return string(&review, "html_url");
          }
          approved = state == "APPROVED";
        }
      }
    }
  }
  let author = field(&metadata, "author").and_then(|a| string(a, "login").ok()).ok_or("missing PR author")?;
  let event = if approval_bar
    && !approved
    && author != login
    && string(&metadata, "state")? == "OPEN"
    && field(&metadata, "isDraft") == Some(&Value::Bool(false))
  {
    "APPROVE"
  } else {
    "COMMENT"
  };
  let files_endpoint =
    format!("repos/{}/{}/pulls/{}/files", pr.reference.owner, pr.reference.repo, pr.reference.number);
  let Value::Array(pages) = gh(&["api", &files_endpoint, "--paginate", "--slurp"])? else {
    return Err("invalid PR files response".into());
  };
  let files = pages
    .into_iter()
    .filter_map(|p| if let Value::Array(rows) = p { Some(rows) } else { None })
    .flatten()
    .collect::<Vec<_>>();
  let body = payload(&head, event, &issues, &files, &marker)?;
  let path = root.join("tmp/gh-review-payload.json");
  std::fs::write(&path, &body).map_err(|e| e.to_string())?;
  // Recheck immediately before the external write; commit_id keeps anchors tied to this head.
  let current = gh(&["pr", "view", &pr.url, "--json", "headRefOid,baseRefOid"])?;
  if string(&current, "headRefOid")? != head || string(&current, "baseRefOid")? != base {
    return Err("PR changed before publication".into());
  }
  if super::paths::session_cancelled(&session.id) {
    return Err("publication cancelled".into());
  }
  let response = match gh(&["api", &endpoint, "--input", &path.to_string_lossy()]) {
    Ok(response) => response,
    Err(error) if error.contains("HTTP 422") || (event == "APPROVE" && error.contains("HTTP 403")) => {
      // A definite validation/approval rejection created no review. Retry once as a
      // comment with every observation in the summary; never retry an ambiguous POST.
      let current = gh(&["pr", "view", &pr.url, "--json", "headRefOid,baseRefOid"])?;
      if string(&current, "headRefOid")? != head
        || string(&current, "baseRefOid")? != base
        || super::paths::session_cancelled(&session.id)
      {
        return Err("PR changed or publication was cancelled before retry".into());
      }
      std::fs::write(&path, payload(&head, "COMMENT", &issues, &[], &marker)?).map_err(|e| e.to_string())?;
      gh(&["api", &endpoint, "--input", &path.to_string_lossy()])?
    }
    Err(error) => return Err(error),
  };
  let url = string(&response, "html_url")?;
  std::fs::write(root.join("tmp/gh-review-published.json"), crate::json::write_pretty(&response))
    .map_err(|e| e.to_string())?;
  Ok(url)
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn publication_checks_results_revisions_and_duplicates_before_posting() {
    let root = std::env::temp_dir().join(format!("review-publish-{}", crate::runtime::random_nonce_6()));
    std::fs::create_dir_all(root.join("tmp")).unwrap();
    std::fs::write(root.join("tmp/gh-gorgeous-review-browser.json"), r#"{"reviewed_head":"head","base_head":"base"}"#)
      .unwrap();
    let mut session = super::super::jsonio::parse_session_json(
      r#"{
      "id":"test-publication", "skills":[{"name":"reviewer","source":"reviewer"}],
      "procs":[{"index":0,"status":"ok","skill_name":"reviewer","lines":[]}]
    }"#,
    )
    .unwrap();
    let pr = PullRequest {
      reference: super::super::github::PullRequestRef { owner: "o".into(), repo: "r".into(), number: 1 },
      title: "Change".into(),
      body: String::new(),
      base_ref: "main".into(),
      base_oid: "a".repeat(40),
      head_oid: "b".repeat(40),
      url: "https://github.com/o/r/pull/1".into(),
    };
    for scenario in [
      "publish",
      "duplicate",
      "stale",
      "failed",
      "missing",
      "self",
      "draft",
      "good",
      "average",
      "poor",
      "bad",
      "invalid",
    ] {
      let grade = match scenario {
        "good" | "average" | "poor" | "bad" => scenario,
        "invalid" => "ok",
        _ => "excellent",
      };
      std::fs::write(
        root.join("reviewer.json"),
        format!(r#"{{"result":{{"grade":"{grade}","issues_found":0}},"issues":[]}}"#),
      )
      .unwrap();
      session.procs[0].status = if scenario == "failed" { ProcStatus::Fail } else { ProcStatus::Ok };
      let mut posted = 0;
      let missing = root.join("missing");
      let result = publish_with(&root, if scenario == "missing" { &missing } else { &root }, &pr, &session, |args| {
        let value = if args[0] == "pr" {
          format!(
            r#"{{"headRefOid":"{}","baseRefOid":"base","state":"OPEN","isDraft":{},"author":{{"login":"{}"}}}}"#,
            if scenario == "stale" { "new-head" } else { "head" },
            scenario == "draft",
            if scenario == "self" { "me" } else { "author" }
          )
        } else if args[1] == "user" {
          r#"{"login":"me"}"#.into()
        } else if args.contains(&"--input") {
          posted += 1;
          let body = parse(&std::fs::read_to_string(args[args.len() - 1]).unwrap()).unwrap();
          assert_eq!(
            string(&body, "event").unwrap(),
            if matches!(scenario, "self" | "draft" | "good" | "average" | "poor" | "bad") {
              "COMMENT"
            } else {
              "APPROVE"
            }
          );
          r#"{"html_url":"https://github.com/o/r/pull/1#review"}"#.into()
        } else if args[1].ends_with("/reviews") && scenario == "duplicate" {
          r#"[[{"user":{"login":"me"},"state":"APPROVED","body":"<!-- review-head:head -->","html_url":"https://github.com/o/r/pull/1#review"}]]"#.into()
        } else {
          "[[]]".into()
        };
        parse(&value)
      });
      assert_eq!(
        result.is_ok(),
        !matches!(scenario, "stale" | "failed" | "missing" | "invalid"),
        "{scenario}: {result:?}"
      );
      assert_eq!(
        posted,
        usize::from(matches!(scenario, "publish" | "self" | "draft" | "good" | "average" | "poor" | "bad")),
        "{scenario}"
      );
    }
    std::fs::remove_dir_all(root).unwrap();
  }

  #[test]
  fn patch_anchors_track_additions_context_and_deletions() {
    assert_eq!(right_lines("@@ -3,3 +3,3 @@\n context\n-old\n+new\n end\n"), BTreeSet::from([3, 4, 5]));
  }

  #[test]
  fn publication_calls_the_local_description_file_the_pr_description() {
    for path in ["PR-DESCRIPTION.md", "a.rs"] {
      let issue = parse(&format!(
        r#"{{"file":"{path}","line":1,"description":"Clarify `PR-DESCRIPTION.md`.","suggestion":"Update PR-DESCRIPTION.md."}}"#
      ))
      .unwrap();
      let files = [parse(r#"{"filename":"a.rs","patch":"@@ -1 +1 @@\n+new"}"#).unwrap()];
      let published = payload("abc", "COMMENT", &[issue], &files, "marker").unwrap();
      assert!(!published.contains("PR-DESCRIPTION.md"));
      assert!(published.contains("Clarify PR description."));
      assert!(published.contains("Update PR description."));
      let value = parse(&published).unwrap();
      if path == "PR-DESCRIPTION.md" {
        assert!(string(&value, "body").unwrap().contains("\n\nPR description\n\n"));
        assert_eq!(field(&value, "comments"), Some(&Value::Array(vec![])));
      } else {
        assert!(matches!(field(&value, "comments"), Some(Value::Array(comments)) if comments.len() == 1));
      }
    }
  }

  #[test]
  fn comments_outside_the_diff_become_summary_notes() {
    let issue = parse(r#"{"file":"a.rs","line":9,"description":"Check this","suggestion":"Try that"}"#).unwrap();
    let value = parse(&payload("abc", "COMMENT", &[issue], &[], "marker").unwrap()).unwrap();
    assert_eq!(field(&value, "comments"), Some(&Value::Array(vec![])));
    assert!(string(&value, "body").unwrap().contains("a.rs:9"));
  }
}
