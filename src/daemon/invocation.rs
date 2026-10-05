//! Persist sanitized invocation diagnostics beside the durable attempt logs.

fn object(fields: Vec<(&str, crate::json::Value)>) -> crate::json::Value {
  crate::json::Value::Object(fields.into_iter().map(|(k, v)| (k.to_string(), v)).collect())
}

/// Hash only known instruction/configuration paths, without copying any credentials or URLs.
/// Availability is not proof of CLI loading; the manifest labels the distinction explicitly.
pub fn save_manifest(
  root: &std::path::Path, skill: &crate::config::ResolvedInvocation, runtime: &str, tag: &str,
) -> Result<(), String> {
  use crate::json::Value;
  let mut document = crate::invocation::manifest(skill);
  let Value::Object(outer) = &mut document else { unreachable!() };
  let Value::Object(fields) = &mut outer[0].1 else { unreachable!() };
  let mut inventory = Vec::new();
  fn visit(root: &std::path::Path, path: &std::path::Path, depth: usize, inventory: &mut Vec<Value>) {
    if depth > 12 || path.is_symlink() {
      return;
    }
    if path.is_dir() {
      if let Ok(entries) = std::fs::read_dir(path) {
        let mut paths: Vec<_> = entries.filter_map(Result::ok).map(|e| e.path()).collect();
        paths.sort();
        for path in paths {
          visit(root, &path, depth + 1, inventory)
        }
      }
    } else if let Ok(bytes) = std::fs::read(path) {
      inventory.push(object(vec![
        ("path", Value::String(path.strip_prefix(root).unwrap_or(path).to_string_lossy().into_owned())),
        ("sha256", Value::String(crate::sha256::sha256_hex(&bytes))),
        ("bytes", Value::Number(bytes.len() as f64)),
      ]));
    }
  }
  for path in ["CLAUDE.md", "AGENTS.md", ".claude", ".skills", ".mcp.json", crate::config::RUN_SKILLS_REL] {
    visit(root, &root.join(path), 0, &mut inventory);
  }
  let image = crate::runtime::command_output(
    std::process::Command::new(runtime).args(["image", "inspect", "--format", "{{.Id}}", tag]),
    std::time::Duration::from_secs(10),
  )
  .ok()
  .filter(|o| o.status.success())
  .and_then(|o| String::from_utf8(o.stdout).ok());
  fields.extend([
    ("attempt".into(), Value::String(root.file_name().unwrap_or_default().to_string_lossy().into_owned())),
    ("image_id".into(), image.map(|s| Value::String(s.trim().to_string())).unwrap_or(Value::Null)),
    ("available_project_files".into(), Value::Array(inventory)),
    (
      "project_mcp_loading_policy".into(),
      Value::String("Claude project configuration; actual exposure unknown until native evidence".into()),
    ),
    ("raw_transcripts_exported".into(), Value::Bool(false)),
  ]);
  let project_settings =
    std::fs::read_to_string(root.join(".claude/settings.json")).ok().and_then(|text| crate::json::parse(&text).ok());
  let configured_cache = project_settings
    .as_ref()
    .and_then(|v| field(v, "env"))
    .and_then(|v| field(v, "DISABLE_PROMPT_CACHING"))
    .filter(|v| matches!(v, Value::String(s) if s == "1" || s == "0"))
    .cloned()
    .unwrap_or(Value::Null);
  let configured_mcp = std::fs::read_to_string(root.join(".mcp.json"))
    .ok()
    .and_then(|text| crate::json::parse(&text).ok())
    .and_then(|v| match field(&v, "mcpServers") {
      Some(Value::Object(servers)) => {
        Some(Value::Array(servers.iter().map(|(name, _)| Value::String(name.clone())).collect()))
      }
      _ => None,
    })
    .unwrap_or(Value::Null);
  fields.extend([
    ("configured_project_disable_prompt_caching".into(), configured_cache),
    ("configured_project_mcp_servers".into(), configured_mcp),
  ]);
  crate::atomic_write(
    &root.join(format!("{}.invocation.json", crate::runtime::RUN_LOG_REL)),
    crate::json::write_pretty(&document).as_bytes(),
  )
  .map_err(|e| e.to_string())
}

fn field<'a>(value: &'a crate::json::Value, name: &str) -> Option<&'a crate::json::Value> {
  let crate::json::Value::Object(fields) = value else { return None };
  fields.iter().find(|(key, _)| key == name).map(|(_, value)| value)
}

/// Add the observed CLI version without copying terminal output.
pub fn finish_manifest(root: &std::path::Path) -> Result<(), String> {
  use crate::json::Value;
  let path = root.join(format!("{}.invocation.json", crate::runtime::RUN_LOG_REL));
  if !path.exists() {
    return Ok(());
  }
  let mut document = crate::json::parse(&std::fs::read_to_string(&path).map_err(|e| e.to_string())?)?;
  let Value::Object(outer) = &mut document else { return Err("invalid invocation manifest".into()) };
  let Some((_, Value::Object(fields))) = outer.first_mut() else { return Err("invalid invocation payload".into()) };
  let version = std::fs::read_to_string(root.join(format!("{}.cli-version", crate::runtime::RUN_LOG_REL))).ok();
  fields.push(("observed_cli_version".into(), version.map(|s| Value::String(s.trim().into())).unwrap_or(Value::Null)));
  crate::atomic_write(&path, crate::json::write_pretty(&document).as_bytes()).map_err(|e| e.to_string())
}
