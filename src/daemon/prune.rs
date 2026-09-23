//! Backup janitor for `/tmp/scsh-*-run-*` dirs — the `scsh run` client deletes first;
//! the daemon retries later only when the dir still exists and the container is gone.
//!
//! Eligibility is immediate. A busy mount or an unverifiable container stays queued and
//! is retried on the next tick. There is no success grace and no 24-hour failure hold:
//! those were retention, and a completed attempt's scratch does not get a lifetime.

use std::path::Path;

use super::paths::prune_file;
use crate::json::{parse, quote, Value};
use crate::runtime;

const MAX_JOBS: usize = 500;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PruneJob {
  pub run_dir: String,
  pub container_name: String,
  /// Empty → probe docker, podman, and Apple `container` on tick.
  pub runtime: String,
  pub outcome_ok: bool,
  pub scheduled_at: u64,
  pub eligible_at: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PruneQueue {
  pub jobs: Vec<PruneJob>,
}

impl PruneQueue {
  pub fn load(port: u16) -> PruneQueue {
    let path = prune_file(port);
    let Ok(text) = std::fs::read_to_string(&path) else {
      return PruneQueue::default();
    };
    parse_queue(&text).unwrap_or_default()
  }

  pub fn save(&self, port: u16) {
    let path = prune_file(port);
    let _ = std::fs::create_dir_all(path.parent().unwrap_or(Path::new("/tmp")));
    let _ = crate::atomic_write(&path, save_queue(self).as_bytes());
  }

  /// Enqueue a backup delete. Idempotent per `run_dir`. Returns false when the queue is
  /// full — an outstanding record is never dropped to make room.
  pub fn schedule(&mut self, run_dir: &str, container_name: &str, runtime: &str, outcome_ok: bool, now: u64) -> bool {
    if run_dir.is_empty() || container_name.is_empty() {
      return false;
    }
    if !is_scsh_run_dir_path(run_dir) {
      return false;
    }
    if self.jobs.iter().any(|j| j.run_dir == run_dir) {
      return true;
    }
    if self.jobs.len() >= MAX_JOBS {
      return false;
    }
    self.jobs.push(PruneJob {
      run_dir: run_dir.to_string(),
      container_name: container_name.to_string(),
      runtime: runtime.to_string(),
      outcome_ok,
      scheduled_at: now,
      eligible_at: now,
    });
    true
  }

  /// Advance the queue: delete eligible dirs that still exist. Returns how many were removed.
  pub fn tick(&mut self, now: u64) -> usize {
    let deadline = std::time::Instant::now() + crate::cleanup::BUDGET;
    let mut removed = 0;
    let mut remaining = Vec::with_capacity(self.jobs.len());
    for job in self.jobs.drain(..) {
      if now < job.eligible_at || std::time::Instant::now() >= deadline {
        remaining.push(job);
        continue;
      }
      // The journal owns artifact preservation and live-run protection. A legacy
      // orphan event must not bypass it while the host is integrating commits.
      if crate::cleanup::journal_path(Path::new(&job.run_dir)).exists() {
        remaining.push(job);
        continue;
      }
      if !Path::new(&job.run_dir).is_dir() {
        continue;
      }
      // A queued job is an attempt the host has already released. If the container is
      // still there, finish the removal here — waiting does not stop it.
      if container_still_present(&job) {
        if !job.runtime.is_empty() {
          let _ = crate::ui::signals::stop_container_until(&job.runtime, &job.container_name, deadline);
        }
        if container_still_present(&job) {
          remaining.push(job);
          continue;
        }
      }
      if std::fs::remove_dir_all(&job.run_dir).is_ok() {
        removed += 1;
      } else {
        remaining.push(job);
      }
    }
    self.jobs = remaining;
    removed
  }
}

fn is_scsh_run_dir_path(run_dir: &str) -> bool {
  Path::new(run_dir).file_name().and_then(|n| n.to_str()).is_some_and(runtime::is_scsh_run_dir_name)
}

fn container_still_present(job: &PruneJob) -> bool {
  let probe = if job.runtime.is_empty() {
    runtime::container_probe_any(&job.container_name)
  } else {
    runtime::container_probe(&job.runtime, &job.container_name)
  };
  probe != runtime::ContainerProbe::Absent
}

pub fn schedule_from_api(body: &str, queue: &mut PruneQueue, now: u64) -> bool {
  let obj = match parse(body).ok() {
    Some(Value::Object(o)) => o,
    _ => return false,
  };
  let run_dir = field_str(&obj, "run_dir").unwrap_or_default();
  let container_name = field_str(&obj, "container_name").unwrap_or_default();
  let runtime = field_str(&obj, "runtime").unwrap_or_default();
  let outcome = field_str(&obj, "outcome").unwrap_or_default();
  let outcome_ok = outcome == "ok";
  if run_dir.is_empty() || container_name.is_empty() {
    return false;
  }
  queue.schedule(&run_dir, &container_name, &runtime, outcome_ok, now)
}

pub fn schedule_orphans_from_session(queue: &mut PruneQueue, container_names: &[(String, String)], now: u64) {
  for (name, runtime) in container_names {
    let run_dir = format!("/tmp/{name}");
    queue.schedule(&run_dir, name, runtime, false, now);
  }
}

fn field_str(obj: &[(String, Value)], key: &str) -> Option<String> {
  obj.iter().find(|(k, _)| k == key).and_then(|(_, v)| match v {
    Value::String(s) => Some(s.clone()),
    _ => None,
  })
}

fn parse_queue(text: &str) -> Result<PruneQueue, String> {
  let root = parse(text)?;
  let obj = match root {
    Value::Object(o) => o,
    _ => return Err("expected object".into()),
  };
  let jobs = match obj.iter().find(|(k, _)| k == "jobs").map(|(_, v)| v) {
    Some(Value::Array(arr)) => arr.iter().filter_map(parse_job).collect(),
    _ => Vec::new(),
  };
  Ok(PruneQueue { jobs })
}

fn parse_job(v: &Value) -> Option<PruneJob> {
  let obj = match v {
    Value::Object(o) => o,
    _ => return None,
  };
  Some(PruneJob {
    run_dir: field_str(obj, "run_dir")?,
    container_name: field_str(obj, "container_name")?,
    runtime: field_str(obj, "runtime").unwrap_or_default(),
    outcome_ok: field_str(obj, "outcome").as_deref() == Some("ok"),
    scheduled_at: field_num(obj, "scheduled_at")? as u64,
    eligible_at: field_num(obj, "eligible_at")? as u64,
  })
}

fn field_num(obj: &[(String, Value)], key: &str) -> Option<f64> {
  obj.iter().find(|(k, _)| k == key).and_then(|(_, v)| match v {
    Value::Number(n) => Some(*n),
    _ => None,
  })
}

pub fn save_queue(queue: &PruneQueue) -> String {
  let parts: Vec<String> = queue.jobs.iter().map(job_json).collect();
  format!("{{\n  \"jobs\": [\n    {}\n  ]\n}}", parts.join(",\n    "))
}

fn job_json(j: &PruneJob) -> String {
  let outcome = if j.outcome_ok { "ok" } else { "fail" };
  format!(
    "{{ \"run_dir\": {}, \"container_name\": {}, \"runtime\": {}, \"outcome\": {}, \
\"scheduled_at\": {}, \"eligible_at\": {} }}",
    quote(&j.run_dir),
    quote(&j.container_name),
    quote(&j.runtime),
    quote(outcome),
    j.scheduled_at,
    j.eligible_at,
  )
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn schedule_dedupes_by_run_dir() {
    let mut q = PruneQueue::default();
    let now = 1_000;
    assert!(q.schedule("/tmp/scsh-abcdef-run-add", "scsh-abcdef-run-add", "docker", true, now));
    assert!(q.schedule("/tmp/scsh-abcdef-run-add", "scsh-abcdef-run-add", "docker", true, now));
    assert_eq!(q.jobs.len(), 1);
  }

  #[test]
  fn completed_jobs_are_eligible_immediately() {
    let mut q = PruneQueue::default();
    for (i, outcome) in [true, false].into_iter().enumerate() {
      assert!(q.schedule(&format!("/tmp/scsh-abcdef-run-{i}"), "scsh-abcdef-run-add", "docker", outcome, 5000));
      assert_eq!(q.jobs[i].eligible_at, 5000);
    }
  }

  #[test]
  fn a_full_queue_does_not_drop_an_outstanding_job() {
    let mut q = PruneQueue::default();
    for i in 0..MAX_JOBS {
      assert!(q.schedule(&format!("/tmp/scsh-abcdef-run-n{i}"), "scsh-abcdef-run-add", "docker", true, 1));
    }
    assert!(!q.schedule("/tmp/scsh-zzzzzz-run-add", "scsh-abcdef-run-add", "docker", true, 1));
    assert_eq!(q.jobs.len(), MAX_JOBS);
    assert_eq!(q.jobs[0].run_dir, "/tmp/scsh-abcdef-run-n0");
  }

  #[test]
  fn missing_dir_drops_job_without_error() {
    let mut q = PruneQueue::default();
    q.schedule("/tmp/scsh-no-such-run-add", "scsh-no-such-run-add", "docker", true, 0);
    assert_eq!(q.tick(1), 0);
    assert!(q.jobs.is_empty());
  }

  #[test]
  fn roundtrip_json() {
    let mut q = PruneQueue::default();
    q.schedule("/tmp/scsh-abcdef-run-add", "scsh-abcdef-run-add", "docker", false, 100);
    let text = save_queue(&q);
    let back = parse_queue(&text).unwrap();
    assert_eq!(back, q);
  }
}
