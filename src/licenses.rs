//! The MIT-only dependency policy, as code.
//!
//! scsh's own code is MIT, and every dependency that ships — in the binary or in a page it
//! serves — must be usable under plain MIT too: either MIT outright, or a dual/multi license
//! whose `OR` lets the consumer elect MIT (scsh does — see LICENSE.md "Third-party
//! licenses"). Apache-only material is what this repo just spent effort removing (the
//! vendored asciinema-player); the test below keeps a future `cargo update` or new
//! dependency from quietly reintroducing a license MIT cannot cover.
//!
//! What ships is the runtime dependency tree: the graph walked from scsh along normal
//! edges. Compile-time tooling — proc-macro crates, whatever only they depend on, and build
//! dependencies — runs inside the compiler and contributes no code to the binary, so it is
//! outside the audit by construction, not by exception.

use crate::json::Value;

/// Whether a Cargo SPDX license expression lets a consumer take the crate under plain MIT:
/// one of its top-level `OR` alternatives (Cargo's legacy `/` separator reads as `OR`) must
/// be exactly `MIT`. `MIT AND Apache-2.0` therefore does NOT qualify — both apply at once —
/// and neither does an empty expression (`license-file`-only crates need a human look).
pub fn mit_choosable(expr: &str) -> bool {
  expr.replace('/', " OR ").split(" OR ").any(|alt| alt.trim() == "MIT")
}

fn field<'a>(value: &'a Value, key: &str) -> Option<&'a Value> {
  match value {
    Value::Object(fields) => fields.iter().find(|(k, _)| k == key).map(|(_, v)| v),
    _ => None,
  }
}

fn text(value: Option<&Value>) -> Option<&str> {
  match value {
    Some(Value::String(s)) => Some(s),
    _ => None,
  }
}

fn items(value: Option<&Value>) -> Option<&Vec<Value>> {
  match value {
    Some(Value::Array(items)) => Some(items),
    _ => None,
  }
}

/// The crates whose code can end up in the scsh binary, as `(name, license)` pairs sorted
/// by name, from a `cargo metadata --format-version 1` document.
///
/// The walk starts at the root package and follows normal dependency edges only — `dev`
/// edges are test-only and `build` edges feed build scripts. A proc-macro crate is a
/// compiler plugin: it is reached, but it ships nothing and neither does anything reached
/// only through it, so the walk records nothing for it and does not continue past it.
/// Target-specific edges are followed regardless of target: what ships on any platform is
/// audited on every platform.
pub fn shipped_dependencies(metadata: &Value) -> Result<Vec<(String, String)>, String> {
  let packages = items(field(metadata, "packages")).ok_or("metadata has no `packages` array")?;
  let mut by_id: std::collections::HashMap<&str, (&str, &str, bool)> = std::collections::HashMap::new();
  for package in packages {
    let id = text(field(package, "id")).ok_or("package without `id`")?;
    let name = text(field(package, "name")).ok_or("package without `name`")?;
    let license = text(field(package, "license")).unwrap_or("");
    let proc_macro = items(field(package, "targets")).ok_or("package without `targets`")?.iter().any(|target| {
      items(field(target, "kind")).is_some_and(|kinds| kinds.iter().any(|k| text(Some(k)) == Some("proc-macro")))
    });
    by_id.insert(id, (name, license, proc_macro));
  }
  let resolve = field(metadata, "resolve").ok_or("metadata has no `resolve`")?;
  let root = text(field(resolve, "root")).ok_or("resolve has no `root`")?;
  let mut edges: std::collections::HashMap<&str, Vec<&str>> = std::collections::HashMap::new();
  for node in items(field(resolve, "nodes")).ok_or("resolve has no `nodes`")? {
    let id = text(field(node, "id")).ok_or("node without `id`")?;
    let deps = items(field(node, "deps")).ok_or("node without `deps`")?;
    let mut normal = Vec::new();
    for dep in deps {
      let kinds = items(field(dep, "dep_kinds")).ok_or("dep without `dep_kinds`")?;
      if kinds.iter().any(|k| matches!(field(k, "kind"), Some(Value::Null))) {
        normal.push(text(field(dep, "pkg")).ok_or("dep without `pkg`")?);
      }
    }
    edges.insert(id, normal);
  }
  let mut seen = std::collections::HashSet::from([root]);
  let mut queue = vec![root];
  let mut shipped = Vec::new();
  while let Some(id) = queue.pop() {
    let (name, license, proc_macro) = *by_id.get(id).ok_or_else(|| format!("resolved `{id}` is not in `packages`"))?;
    if proc_macro {
      continue;
    }
    if id != root {
      shipped.push((name.to_string(), license.to_string()));
    }
    for next in edges.get(id).into_iter().flatten() {
      if seen.insert(next) {
        queue.push(next);
      }
    }
  }
  shipped.sort();
  Ok(shipped)
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn mit_choosable_reads_spdx_or_alternatives() {
    assert!(mit_choosable("MIT"));
    assert!(mit_choosable("MIT OR Apache-2.0"));
    assert!(mit_choosable("Apache-2.0 OR MIT"));
    assert!(mit_choosable("Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT"));
    assert!(mit_choosable("Apache-2.0/MIT")); // legacy separator
    assert!(!mit_choosable("Apache-2.0"));
    assert!(!mit_choosable("MIT AND Apache-2.0")); // both apply — not electable
    assert!(!mit_choosable("MITigation-1.0")); // exact token, not a prefix
    assert!(!mit_choosable(""));
  }

  /// A hand-built graph: the root ships `serde` and, through it, `itoa`; `serde_derive`
  /// is a proc macro, so neither it nor its own `unicode-ident` ships; `cc` is a build
  /// dependency and `insta` a dev dependency, so neither ships either.
  #[test]
  fn shipped_dependencies_follow_normal_edges_and_stop_at_proc_macros() {
    let metadata = crate::json::parse(
      r#"{
        "packages": [
          { "id": "scsh", "name": "scsh", "license": "MIT", "targets": [{ "kind": ["bin"] }] },
          { "id": "serde", "name": "serde", "license": "MIT OR Apache-2.0", "targets": [{ "kind": ["lib"] }] },
          { "id": "itoa", "name": "itoa", "license": "MIT OR Apache-2.0", "targets": [{ "kind": ["lib"] }] },
          { "id": "serde_derive", "name": "serde_derive", "license": "MIT OR Apache-2.0", "targets": [{ "kind": ["proc-macro"] }] },
          { "id": "unicode-ident", "name": "unicode-ident", "license": "(MIT OR Apache-2.0) AND Unicode-3.0", "targets": [{ "kind": ["lib"] }] },
          { "id": "cc", "name": "cc", "license": "Apache-2.0", "targets": [{ "kind": ["lib"] }] },
          { "id": "insta", "name": "insta", "license": "Apache-2.0", "targets": [{ "kind": ["lib"] }] }
        ],
        "resolve": {
          "root": "scsh",
          "nodes": [
            { "id": "scsh", "deps": [
              { "pkg": "serde", "dep_kinds": [{ "kind": null, "target": null }] },
              { "pkg": "cc", "dep_kinds": [{ "kind": "build", "target": null }] },
              { "pkg": "insta", "dep_kinds": [{ "kind": "dev", "target": null }] }
            ] },
            { "id": "serde", "deps": [
              { "pkg": "serde_derive", "dep_kinds": [{ "kind": null, "target": null }] },
              { "pkg": "itoa", "dep_kinds": [{ "kind": null, "target": "cfg(windows)" }] }
            ] },
            { "id": "serde_derive", "deps": [{ "pkg": "unicode-ident", "dep_kinds": [{ "kind": null, "target": null }] }] },
            { "id": "itoa", "deps": [] },
            { "id": "unicode-ident", "deps": [] },
            { "id": "cc", "deps": [] },
            { "id": "insta", "deps": [] }
          ]
        }
      }"#,
    )
    .unwrap();
    let shipped = shipped_dependencies(&metadata).unwrap();
    assert_eq!(
      shipped,
      [("itoa".to_string(), "MIT OR Apache-2.0".to_string()), ("serde".to_string(), "MIT OR Apache-2.0".to_string())]
    );
    assert!(shipped.iter().all(|(_, license)| mit_choosable(license)));
    assert_eq!(
      shipped_dependencies(&crate::json::parse("{}").unwrap()),
      Err("metadata has no `packages` array".into())
    );
  }

  /// Every crate that ships — the runtime dependency tree, target-specific deps included —
  /// must be usable under plain MIT. Runs `cargo metadata` (locked, no network beyond what
  /// a build already fetched) and checks each shipped package's SPDX expression. If this
  /// fails, either pick a different crate or bring the question to a human; do NOT weaken
  /// `mit_choosable` or widen what `shipped_dependencies` skips.
  #[test]
  fn every_shipped_dependency_is_usable_under_plain_mit() {
    let out = std::process::Command::new(env!("CARGO"))
      .args(["metadata", "--format-version", "1", "--locked"])
      .current_dir(env!("CARGO_MANIFEST_DIR"))
      .output()
      .expect("cargo metadata runs");
    assert!(out.status.success(), "cargo metadata failed: {}", String::from_utf8_lossy(&out.stderr));
    let metadata = crate::json::parse(&String::from_utf8_lossy(&out.stdout)).expect("metadata is JSON");
    let shipped = shipped_dependencies(&metadata).expect("metadata has the documented shape");
    assert!(shipped.len() > 5, "suspiciously few shipped crates — did metadata change shape?");
    let bad: Vec<String> = shipped
      .iter()
      .filter(|(_, license)| !mit_choosable(license))
      .map(|(name, license)| format!("{name}: '{license}'"))
      .collect();
    assert!(bad.is_empty(), "shipped dependencies not usable under plain MIT:\n  {}", bad.join("\n  "));
  }
}
