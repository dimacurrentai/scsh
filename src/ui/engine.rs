//! Container-engine liveness and "it's installed but not running" advice.
//!
//! scsh's preflight already locates a runtime *binary* on `$PATH`; this adds the
//! second question a real run needs answered — *is the engine actually up?* — and,
//! when it isn't, the exact command to start it. The decision logic
//! ([`start_command`]) is pure and unit-tested; only [`is_running`] shells out.
//!
//! Everything is keyed by the runtime's name string (`"docker"` / `"podman"` /
//! `"container"`), the same identifier [`crate::runtime::Runtime`] already carries,
//! so there is no parallel runtime enum to keep in sync.

use std::process::{Command, Stdio};

/// What [`ensure_running`] had to do before container work could begin.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EnsureRunning {
  /// The engine already answered its liveness probe.
  AlreadyRunning,
  /// `scsh` started Apple Containers and verified that it became live.
  Started,
}

/// The host operating system, as far as the start-command advice cares.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Os {
  Mac,
  Linux,
  Other,
}

impl Os {
  /// The OS this binary was compiled for.
  pub fn current() -> Os {
    if cfg!(target_os = "macos") {
      Os::Mac
    } else if cfg!(target_os = "linux") {
      Os::Linux
    } else {
      Os::Other
    }
  }
}

/// Human-friendly name for messages (falls back to the raw name for an
/// `SCSH_RUNTIME` scsh doesn't have canned advice for).
pub fn display_name(runtime: &str) -> String {
  match runtime {
    "docker" => "Docker".to_string(),
    "podman" => "Podman".to_string(),
    "container" => "Apple container".to_string(),
    other => other.to_string(),
  }
}

/// Arguments to a cheap "are you actually up?" probe for the engine.
///
/// `info` talks to the docker/podman daemon and fails fast if it isn't reachable;
/// Apple's `container list` fails until its system service is started. Unknown
/// runtimes get `info`, the near-universal convention.
fn liveness_probe(runtime: &str) -> &'static [&'static str] {
  match runtime {
    "container" => &["list"],
    _ => &["info"],
  }
}

/// The command that starts the engine, for the given OS — `None` when scsh has no
/// canned advice for this runtime name. Best-effort and documented as an assumption
/// in DAEMON.md ("A stopped container engine"); callers must handle `None` by asking
/// for a start rather than guessing a command.
pub fn start_command(runtime: &str, os: Os) -> Option<String> {
  let cmd = match (runtime, os) {
    ("docker", Os::Mac) => "open -a Docker",
    ("docker", Os::Linux) => "sudo systemctl start docker",
    ("docker", Os::Other) => "start Docker Desktop",

    ("podman", Os::Mac) => "podman machine start",
    ("podman", Os::Linux) => "systemctl --user start podman.socket",
    ("podman", Os::Other) => "podman machine start",

    // Apple's `container` only exists on macOS, but answer sensibly regardless.
    ("container", _) => "container system start",

    _ => return None,
  };
  Some(cmd.to_string())
}

/// Is the engine up and accepting work? Runs the runtime's liveness probe
/// (`<runtime> info`, or `container list`) quietly and reports whether it
/// succeeded. A missing binary or a probe error both read as "not running".
pub fn is_running(runtime: &str) -> bool {
  Command::new(runtime)
    .args(liveness_probe(runtime))
    .stdout(Stdio::null())
    .stderr(Stdio::null())
    .status()
    .map(|s| s.success())
    .unwrap_or(false)
}

/// Start arguments `scsh` may run without opening a desktop application or escalating.
/// Apple Containers is the only automatic case: its macOS system service is user-scoped,
/// `container system start` is synchronous, and it is the preferred runtime after reboot.
fn automatic_start_args(runtime: &str, os: Os) -> Option<&'static [&'static str]> {
  match (runtime, os) {
    ("container", Os::Mac) => Some(&["system", "start"]),
    _ => None,
  }
}

/// Ensure a selected runtime can accept work. On macOS, a stopped Apple Containers service is
/// started automatically and then probed again; other runtimes preserve the existing explicit
/// start behavior because they may open desktop applications or require privilege escalation.
pub fn ensure_running(runtime: &str, os: Os) -> Result<EnsureRunning, String> {
  ensure_running_with(
    runtime,
    os,
    || is_running(runtime),
    |args| {
      Command::new(runtime)
        .args(args)
        .output()
        .map(|out| (out.status.success(), String::from_utf8_lossy(&out.stderr).trim().to_string()))
        .map_err(|e| e.to_string())
    },
  )
}

/// Testable decision core for [`ensure_running`]. The production wrapper supplies the liveness
/// probe and process runner, keeping reboot behavior deterministic under unit tests.
fn ensure_running_with<R, S>(runtime: &str, os: Os, mut running: R, mut start: S) -> Result<EnsureRunning, String>
where
  R: FnMut() -> bool,
  S: FnMut(&[&str]) -> Result<(bool, String), String>,
{
  if running() {
    return Ok(EnsureRunning::AlreadyRunning);
  }
  let Some(args) = automatic_start_args(runtime, os) else {
    return Err(format!("{} is installed but not running", display_name(runtime)));
  };
  let (success, detail) = start(args).map_err(|e| format!("could not run `container system start`: {e}"))?;
  if running() {
    return Ok(EnsureRunning::Started);
  }
  if success {
    Err("`container system start` completed, but Apple container is still not accepting work".into())
  } else if detail.is_empty() {
    Err("`container system start` failed".into())
  } else {
    Err(format!("`container system start` failed: {detail}"))
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn display_names_are_friendly_with_a_raw_fallback() {
    assert_eq!(display_name("docker"), "Docker");
    assert_eq!(display_name("podman"), "Podman");
    assert_eq!(display_name("container"), "Apple container");
    assert_eq!(display_name("nerdctl"), "nerdctl");
  }

  #[test]
  fn start_commands_are_os_specific() {
    assert_eq!(start_command("docker", Os::Mac).as_deref(), Some("open -a Docker"));
    assert!(start_command("docker", Os::Linux).unwrap().contains("systemctl start docker"));
    assert_eq!(start_command("podman", Os::Mac).as_deref(), Some("podman machine start"));
    assert!(start_command("podman", Os::Linux).unwrap().contains("podman.socket"));
    assert_eq!(start_command("container", Os::Mac).as_deref(), Some("container system start"));
    // Unknown runtimes have no canned start command.
    assert_eq!(start_command("nerdctl", Os::Linux), None);
  }

  #[test]
  fn apple_container_is_the_only_automatic_start() {
    assert_eq!(automatic_start_args("container", Os::Mac), Some(&["system", "start"][..]));
    assert_eq!(automatic_start_args("container", Os::Linux), None);
    assert_eq!(automatic_start_args("docker", Os::Mac), None);
    assert_eq!(automatic_start_args("podman", Os::Mac), None);
  }

  #[test]
  fn ensure_running_starts_and_rechecks_apple_container() {
    let mut probes = [false, true].into_iter();
    let mut started = false;
    let result = ensure_running_with(
      "container",
      Os::Mac,
      || probes.next().unwrap_or(true),
      |args| {
        assert_eq!(args, ["system", "start"]);
        started = true;
        Ok((true, String::new()))
      },
    );
    assert_eq!(result, Ok(EnsureRunning::Started));
    assert!(started);
  }

  #[test]
  fn ensure_running_does_not_start_other_engines() {
    let result = ensure_running_with("docker", Os::Mac, || false, |_| panic!("must not auto-start Docker"));
    assert_eq!(result, Err("Docker is installed but not running".into()));
  }

  #[test]
  fn ensure_running_reports_a_failed_apple_start() {
    let result = ensure_running_with("container", Os::Mac, || false, |_| Ok((false, "service unavailable".into())));
    assert_eq!(result, Err("`container system start` failed: service unavailable".into()));
  }
}
