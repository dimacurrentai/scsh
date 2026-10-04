//! Explicit execution and prompt policies, shared by flat tasks and workflows.

use crate::config::{Harness, Node};
use std::collections::BTreeMap;

/// Claude's process lifecycle; interactive remains the compatibility default.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ClaudeMode {
  #[default]
  Interactive,
  Headless,
}

/// How much machine-contract prose the caller asks the launcher to generate.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum PromptContract {
  #[default]
  Standard,
  /// The caller owns every instruction, including result paths and local Git restrictions.
  Verbatim,
}

/// Orthogonal choices affecting model input and process supervision.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Options {
  /// Print mode exits naturally and streams JSON instead of recording a TUI.
  pub claude_mode: ClaudeMode,
  /// Generated instructions are opt-out; validation is always retained.
  pub prompt_contract: PromptContract,
}

impl Options {
  pub fn parse(fields: &BTreeMap<&str, &Node>, harness: Option<Harness>, at: &str, errors: &mut Vec<String>) -> Self {
    let mut options = Self::default();
    for key in ["claude_mode", "prompt_contract"] {
      let Some(node) = fields.get(key) else { continue };
      let value = match node {
        Node::Scalar(value) => value.trim(),
        Node::Map(_) => "",
      };
      match (key, value) {
        ("claude_mode", "interactive") => options.claude_mode = ClaudeMode::Interactive,
        ("claude_mode", "headless") => options.claude_mode = ClaudeMode::Headless,
        ("prompt_contract", "standard") => options.prompt_contract = PromptContract::Standard,
        ("prompt_contract", "verbatim") => options.prompt_contract = PromptContract::Verbatim,
        _ => errors.push(format!(
          "'{at}.{key}' must be {}",
          if key == "claude_mode" { "interactive or headless" } else { "standard or verbatim" }
        )),
      }
    }
    if fields.contains_key("claude_mode") && harness != Some(Harness::Claude) {
      errors.push(format!("'{at}.claude_mode' requires harness: claude; put it on a Claude route"));
    }
    options
  }

  pub fn mode_name(self) -> &'static str {
    match self.claude_mode {
      ClaudeMode::Interactive => "interactive",
      ClaudeMode::Headless => "headless",
    }
  }

  pub fn contract_name(self) -> &'static str {
    match self.prompt_contract {
      PromptContract::Standard => "standard",
      PromptContract::Verbatim => "verbatim",
    }
  }
}

/// Pure inspection document. Execution and inspection use the same final prompt renderer.
pub fn manifest(skill: &crate::config::ResolvedInvocation) -> crate::json::Value {
  use crate::json::Value;
  let prompt = crate::runtime::agent_task_prompt(skill.harness, &skill.skill_source, &skill.delivery, skill.options);
  let text = |body: &str| {
    object(vec![
      ("text", Value::String(body.to_string())),
      ("bytes", Value::Number(body.len() as f64)),
      ("sha256", Value::String(crate::sha256::sha256_hex(body.as_bytes()))),
    ])
  };
  let authored = match &skill.delivery {
    crate::config::SkillDelivery::WorkflowPrompt { authored, .. } => Some(authored.as_str()),
    other => other.body(),
  };
  let generated = if skill.options.prompt_contract == PromptContract::Verbatim {
    Some("")
  } else {
    authored.and_then(|body| prompt.strip_prefix(body.trim_end()))
  };
  object(vec![(
    "InvocationManifest",
    object(vec![
      ("schema_version", Value::Number(1.0)),
      ("invocation", Value::String(skill.name.clone())),
      ("harness", Value::String(skill.harness.as_str().to_string())),
      ("execution_mode", Value::String(skill.options.mode_name().to_string())),
      ("prompt_contract", Value::String(skill.options.contract_name().to_string())),
      ("requested_model", skill.model.clone().map(Value::String).unwrap_or(Value::Null)),
      ("effort", skill.effort.clone().map(Value::String).unwrap_or(Value::Null)),
      ("effort_source", if skill.effort.is_some() { Value::String("route --effort".into()) } else { Value::Null }),
      ("source_revision", Value::String(crate::version::git_stamp())),
      ("authored_task", authored.map(text).unwrap_or(Value::Null)),
      ("generated_contract", generated.map(text).unwrap_or(Value::Null)),
      ("delivered_body", skill.delivery.body().map(text).unwrap_or(Value::Null)),
      ("submitted_prompt", text(&prompt)),
      (
        "command",
        Value::String(crate::runtime::harness_command_with_options(
          skill.harness,
          skill.model.as_deref(),
          skill.effort.as_deref(),
          &skill.skill_source,
          &skill.result,
          skill.terminal,
          &skill.delivery,
          skill.options,
        )),
      ),
      (
        "argument_vector",
        if skill.harness == Harness::Claude {
          Value::Array(
            crate::runtime::claude_argv(skill.model.as_deref(), skill.effort.as_deref(), &prompt, skill.options)
              .into_iter()
              .map(Value::String)
              .collect(),
          )
        } else {
          Value::Null
        },
      ),
      ("provider_injected_context", Value::Null),
      ("effective_prompt_cache", Value::Null),
      ("exposed_tool_definitions", Value::Null),
    ]),
  )])
}

fn object(fields: Vec<(&str, crate::json::Value)>) -> crate::json::Value {
  crate::json::Value::Object(fields.into_iter().map(|(k, v)| (k.to_string(), v)).collect())
}

/// Print-mode failure messages come from the terminal result event, not a nonexistent cast.
pub fn headless_error(stream: &str) -> Option<String> {
  use crate::json::Value;
  stream
    .lines()
    .filter_map(|line| crate::json::parse(line).ok())
    .filter_map(|value| {
      let Value::Object(fields) = value else { return None };
      let get = |key| fields.iter().find(|(k, _)| k == key).map(|(_, v)| v);
      if get("type") != Some(&Value::String("result".into())) || get("is_error") != Some(&Value::Bool(true)) {
        return None;
      }
      match get("result") {
        Some(Value::String(message)) => Some(message.clone()),
        _ => Some("Claude headless result reports an error".into()),
      }
    })
    .next_back()
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::{config, harness_def, runtime};

  #[test]
  fn headless_errors_require_a_terminal_error_event() {
    assert_eq!(
      headless_error(r#"{"type":"result","is_error":true,"result":"You've hit your weekly limit"}"#),
      Some("You've hit your weekly limit".into())
    );
    assert_eq!(headless_error(r#"{"type":"assistant","is_error":true,"result":"limit"}"#), None);
    assert_eq!(headless_error(r#"{"type":"result","is_error":false,"result":"limit"}"#), None);
    assert_eq!(headless_error("truncated"), None);
  }

  fn workflow(policy: &str) -> harness_def::HarnessDef {
    let yaml = r#"description: Prompt fidelity test
steps:
  solve:
    agent:
      harness: claude
      model: claude-opus-5-5
      effort: medium
      claude_mode: headless
      prompt_contract: POLICY
    prompt: |
      Compute the answer. Preserve '$value', `backticks`, and Unicode: café.
    output:
      answer:
        type: int
    artifacts: answer.json
"#
    .replace("POLICY", policy);
    harness_def::validate("probe", &yaml, harness_def::DefSource::Repo).unwrap()
  }

  #[test]
  fn policies_preserve_contracts_or_exact_authored_bytes() {
    for policy in ["standard", "verbatim"] {
      let def = workflow(policy);
      let step = &def.steps[0];
      let inv = crate::step_invocation(step, "solve", "tmp/results", Vec::new(), None);
      let prompt = runtime::agent_task_prompt(inv.harness, &inv.skill_source, &inv.delivery, inv.options);
      let authored = step.task().unwrap().body();
      assert!(prompt.starts_with(authored.trim_end()));
      assert_eq!(inv.options.mode_name(), "headless");
      assert_eq!(inv.effort.as_deref(), Some("medium"));
      if policy == "verbatim" {
        assert_eq!(prompt.as_bytes(), authored.as_bytes());
      } else {
        assert!(prompt.contains("$SCSH_RESULT"));
        assert!(prompt.contains("answer.json"));
        assert!(prompt.contains("Do not git fetch"));
      }
      let document = crate::json::write_pretty(&manifest(&inv));
      assert!(crate::json::parse(&document).is_ok());
      assert!(document.contains(&crate::sha256::sha256_hex(prompt.as_bytes())));
    }
  }

  #[test]
  fn flat_routes_accept_policies_and_reject_wrong_harness_and_typos() {
    let yaml = r#"description: Flat prompt
task: Write the answer to $SCSH_RESULT.
invocations:
  print:
    harness: claude
    claude_mode: headless
    prompt_contract: verbatim
    effort: medium
"#;
    let def = harness_def::validate("flat", yaml, harness_def::DefSource::Repo).unwrap();
    let cfg = config::Config { skills: vec![def.to_skill()], terminal: config::Terminal::default() };
    assert_eq!(config::expand_invocations(&cfg)[0].options.prompt_contract, PromptContract::Verbatim);
    for bad in [
      yaml.replace("harness: claude", "harness: codex"),
      yaml.replace("headless", "headles"),
      yaml.replace("verbatim", "raw"),
      yaml.replace("claude_mode", "claude_mod"),
    ] {
      assert!(harness_def::validate("flat", &bad, harness_def::DefSource::Repo).is_err());
    }
  }

  #[test]
  fn headless_shell_preserves_exit_status_and_prompt_bytes() {
    use std::os::unix::fs::PermissionsExt;
    let root = std::env::temp_dir().join(format!("scsh-headless-test-{}", runtime::random_nonce_6()));
    std::fs::create_dir_all(&root).unwrap();
    struct Cleanup(std::path::PathBuf);
    impl Drop for Cleanup {
      fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
      }
    }
    let _cleanup = Cleanup(root.clone());
    let fake = root.join("claude");
    std::fs::write(
      &fake,
      r#"#!/bin/sh
if [ "$1" = '--version' ]; then echo 'test-cli'; exit 0; fi
for arg; do last=$arg; done
printf '%s' "$last" > "$PROMPT_CAPTURE"
echo '{"type":"result"}'
exit "$TEST_EXIT"
"#,
    )
    .unwrap();
    std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o700)).unwrap();
    let prompt = "---\n'$(touch should-not-exist)' `literal` café\n\n";
    let options = Options { claude_mode: ClaudeMode::Headless, prompt_contract: PromptContract::Verbatim };
    let command = runtime::harness_command_with_options(
      config::Harness::Claude,
      Some("opus"),
      Some("medium"),
      "test",
      "tmp/result",
      config::Terminal::default(),
      &config::SkillDelivery::DirectPrompt(prompt.into()),
      options,
    );
    assert!(!command.contains("scsh-tui-record"));
    assert!(!command.contains("/exit"));
    for status in [0, 17] {
      let output = runtime::command_output(
        std::process::Command::new("sh")
          .args(["-c", &command])
          .current_dir(&root)
          .env("PATH", format!("{}:/usr/bin:/bin", root.display()))
          .env("SCSH_RUN_LOG", root.join("run.log"))
          .env("PROMPT_CAPTURE", root.join("prompt"))
          .env("TEST_EXIT", status.to_string()),
        std::time::Duration::from_secs(5),
      )
      .unwrap();
      assert_eq!(
        output.status.code(),
        Some(status),
        "stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
      );
      assert_eq!(std::fs::read(root.join("prompt")).unwrap(), prompt.as_bytes());
      assert!(!root.join("should-not-exist").exists());
      assert!(!root.join("run.log.cast").exists());
    }
  }
}
