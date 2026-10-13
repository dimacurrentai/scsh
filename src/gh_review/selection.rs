//! The Cursor/Grok review lane: relevant pools, then a fixed preference.
use super::{HarnessPlan, VERDICT_ALTERNATIVE_SELECTED};
use crate::config::Harness;

/// Keep this aligned with the explicit native Cursor model in gh-gorgeous-review.yml.
/// Cursor's aggregate includes unrelated models; Grok's aggregate is a shared credit cap.
pub(super) fn relevant(harness: Harness, id: &str) -> bool {
  match harness {
    Harness::Cursor => id == "auto_pool",
    Harness::Grok => matches!(id, "weekly" | "weekly_grokbuild"),
    _ => true,
  }
}

/// Cursor and Grok both run Grok 4.5, so at most one reviews. Cursor takes the lane whenever
/// the plan cleared it; Grok runs only when Cursor sits out for credentials or quota. The
/// exclusions stay as the plan decided them, and both routes never run to make the fleet minimum.
pub(super) fn select(plans: &mut [HarnessPlan]) {
  let cursor = plans.iter().position(|p| p.harness == Harness::Cursor && p.runs());
  let grok = plans.iter().position(|p| p.harness == Harness::Grok && p.runs());
  let (Some(cursor), Some(grok)) = (cursor, grok) else { return };
  let explanation = "Grok 4.5 lane selects cursor; grok runs only when cursor cannot";
  plans[cursor].note.push_str(&format!("; {explanation}"));
  plans[grok].verdict = VERDICT_ALTERNATIVE_SELECTED;
  plans[grok].note.push_str(&format!("; {explanation}"));
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::gh_review::{decide, runnable, MIN_HARNESSES, VERDICT_LOW_QUOTA, VERDICT_RUN};
  use crate::quota::{HarnessQuota, QuotaWindow};

  fn quota(harness: Harness, used: f64) -> HarnessQuota {
    HarnessQuota {
      harness,
      status: "ok",
      plan: None,
      summary: harness.as_str().into(),
      hint: String::new(),
      source: "endpoint",
      observed_at: None,
      windows: vec![QuotaWindow {
        id: if harness == Harness::Cursor { "auto_pool" } else { "weekly" }.into(),
        label: "review pool".into(),
        used_percent: used,
        resets_at: None,
      }],
    }
  }

  fn selected(quotas: &[HarnessQuota]) -> Vec<HarnessPlan> {
    let mut plans: Vec<_> = [Harness::Cursor, Harness::Grok]
      .into_iter()
      .map(|h| decide(h, Ok(()), quotas.iter().find(|q| q.harness == h)))
      .collect();
    select(&mut plans);
    assert!(runnable(&plans).len() <= 1, "never duplicate the Grok 4.5 lane");
    plans
  }

  #[test]
  fn native_pool_gates_cursor_even_when_other_pools_are_exhausted() {
    let mut c = quota(Harness::Cursor, 40.0);
    for id in ["api_pool", "billing_cycle"] {
      c.windows.push(QuotaWindow { id: id.into(), label: id.into(), used_percent: 100.0, resets_at: None });
    }
    assert_eq!(decide(Harness::Cursor, Ok(()), Some(&c)).verdict, VERDICT_RUN);
    c.windows[0].used_percent = 90.0;
    assert_eq!(decide(Harness::Cursor, Ok(()), Some(&c)).verdict, VERDICT_RUN);
    c.windows[0].used_percent = 90.01;
    assert_eq!(decide(Harness::Cursor, Ok(()), Some(&c)).verdict, VERDICT_LOW_QUOTA);
    c.windows.remove(0);
    let plan = decide(Harness::Cursor, Ok(()), Some(&c));
    assert_eq!(plan.verdict, VERDICT_RUN);
    assert!(plan.note.contains("native-model quota unavailable"));
  }

  #[test]
  fn grok_uses_shared_and_build_limits_but_not_chat_or_voice() {
    let mut g = quota(Harness::Grok, 40.0);
    for id in ["weekly_grokchat", "weekly_grokvoice"] {
      g.windows.push(QuotaWindow { id: id.into(), label: id.into(), used_percent: 100.0, resets_at: None });
    }
    assert_eq!(decide(Harness::Grok, Ok(()), Some(&g)).verdict, VERDICT_RUN);
    g.windows.push(QuotaWindow {
      id: "weekly_grokbuild".into(),
      label: "build".into(),
      used_percent: 91.0,
      resets_at: None,
    });
    assert_eq!(decide(Harness::Grok, Ok(()), Some(&g)).verdict, VERDICT_LOW_QUOTA);
    g.windows.pop();
    g.windows[0].used_percent = 94.0;
    assert_eq!(decide(Harness::Grok, Ok(()), Some(&g)).verdict, VERDICT_LOW_QUOTA);
  }

  #[test]
  fn cursor_takes_the_lane_whenever_it_can_run_whatever_grok_has_left() {
    // Grok has far more room; the lane still goes to cursor, and grok is told why it sat out.
    let plans = selected(&[quota(Harness::Cursor, 85.0), quota(Harness::Grok, 0.0)]);
    assert_eq!(runnable(&plans), vec![Harness::Cursor]);
    assert_eq!(plans[1].verdict, VERDICT_ALTERNATIVE_SELECTED);
    assert!(plans[1].note.contains("grok runs only when cursor cannot"));
    assert_eq!(runnable(&selected(&[])), vec![Harness::Cursor], "unknown quota still runs cursor");
  }

  #[test]
  fn grok_takes_the_lane_when_cursor_sits_out() {
    let plans = selected(&[quota(Harness::Cursor, 95.0), quota(Harness::Grok, 40.0)]);
    assert_eq!(plans[0].verdict, VERDICT_LOW_QUOTA);
    assert_eq!(runnable(&plans), vec![Harness::Grok]);
    let mut plans = vec![decide(Harness::Cursor, Err("expired".into()), None), decide(Harness::Grok, Ok(()), None)];
    let original = plans.clone();
    select(&mut plans);
    assert_eq!(plans, original, "a benched cursor leaves grok untouched");
    assert_eq!(runnable(&plans), vec![Harness::Grok]);
  }

  #[test]
  fn exclusions_survive_selection_and_do_not_count_as_a_second_opinion() {
    let plans = selected(&[quota(Harness::Cursor, 40.0), quota(Harness::Grok, 94.0)]);
    assert_eq!(plans[1].verdict, VERDICT_LOW_QUOTA);
    assert_eq!(runnable(&plans), vec![Harness::Cursor]);
    assert!(runnable(&plans).len() < MIN_HARNESSES);
  }
}
