//! The Cursor/Grok review lane: relevant pools, then reset-aware selection.
use super::{HarnessPlan, LONG_WINDOW_MAX_USED, VERDICT_ALTERNATIVE_SELECTED};
use crate::config::Harness;
use crate::quota::{HarnessQuota, QuotaWindow};

/// Keep this aligned with the explicit native Cursor model in gh-gorgeous-review.yml.
/// Cursor's aggregate includes unrelated models; Grok's aggregate is a shared credit cap.
pub(super) fn relevant(harness: Harness, id: &str) -> bool {
  match harness {
    Harness::Cursor => id == "auto_pool",
    Harness::Grok => matches!(id, "weekly" | "weekly_grokbuild"),
    _ => true,
  }
}

struct Budget {
  spare: f64,
  daily: Option<f64>,
}

fn budget(quotas: &[HarnessQuota], harness: Harness, now: u64) -> Option<Budget> {
  let q = quotas.iter().find(|q| q.harness == harness && q.status == "ok")?;
  // Missing primary pools are unknown, never free quota or a substitute aggregate.
  let primary = if harness == Harness::Cursor { "auto_pool" } else { "weekly" };
  q.windows.iter().find(|w| w.id == primary)?;
  let windows: Vec<&QuotaWindow> = q.windows.iter().filter(|w| relevant(harness, &w.id)).collect();
  if windows.iter().any(|w| !w.used_percent.is_finite() || w.used_percent < 0.0) {
    return None;
  }
  let spare = windows.iter().map(|w| (LONG_WINDOW_MAX_USED - w.used_percent).max(0.0)).fold(100.0, f64::min);
  let daily = windows.iter().try_fold(f64::INFINITY, |limit, w| {
    let reset = reset_epoch(w.resets_at.as_deref()?)?;
    // An elapsed reset is stale data, not evidence of a replenished allowance.
    let seconds = reset.checked_sub(now).filter(|s| *s > 0)?;
    let days = seconds.max(3600) as f64 / 86400.0;
    Some(limit.min((LONG_WINDOW_MAX_USED - w.used_percent).max(0.0) / days))
  });
  Some(Budget { spare, daily })
}

/// Higher spendable percentage per day consumes expiring allowance first. Percentages are
/// a scheduling heuristic, not comparable token balances; provider capacities are unknown.
/// Preserve credential/quota exclusions and never run both routes to satisfy the fleet minimum.
pub(super) fn select(plans: &mut [HarnessPlan], quotas: &[HarnessQuota], now: u64) {
  let cursor = plans.iter().position(|p| p.harness == Harness::Cursor && p.runs());
  let grok = plans.iter().position(|p| p.harness == Harness::Grok && p.runs());
  let (Some(cursor), Some(grok)) = (cursor, grok) else { return };
  let c = budget(quotas, Harness::Cursor, now);
  let g = budget(quotas, Harness::Grok, now);
  let (use_cursor, reason) = match (&c, &g) {
    (Some(c), Some(g)) => match (c.daily, g.daily) {
      (Some(cd), Some(gd)) => (
        cd >= gd,
        format!("spendable quota/day: cursor {cd:.2}%, grok {gd:.2}% (10% reserve; reset horizon at least 1h)"),
      ),
      _ => (
        c.spare >= g.spare,
        format!(
          "reset unavailable or stale; spendable quota: cursor {:.1}%, grok {:.1}% (10% reserve)",
          c.spare, g.spare
        ),
      ),
    },
    (Some(_), None) => (true, "only cursor has a usable quota reading".into()),
    (None, Some(_)) => (false, "only grok has a usable quota reading".into()),
    (None, None) => (true, "quota unavailable for both; deterministic cursor preference".into()),
  };
  let (winner, loser) = if use_cursor { (cursor, grok) } else { (grok, cursor) };
  let chosen = plans[winner].harness.as_str();
  let explanation = format!("Grok 4.5 lane selects {chosen}; {reason}; ties prefer cursor");
  plans[winner].note.push_str(&format!("; {explanation}"));
  plans[loser].verdict = VERDICT_ALTERNATIVE_SELECTED;
  plans[loser].note.push_str(&format!("; {explanation}"));
}

/// Parse the normalized UTC timestamps emitted by quota providers. Unsupported offsets
/// and invalid dates use the documented missing-reset fallback rather than inventing a date.
fn reset_epoch(iso: &str) -> Option<u64> {
  if iso.len() != 20
    || !iso.is_ascii()
    || &iso[4..5] != "-"
    || &iso[7..8] != "-"
    || &iso[10..11] != "T"
    || &iso[13..14] != ":"
    || &iso[16..17] != ":"
    || &iso[19..] != "Z"
  {
    return None;
  }
  let number = |a, b| {
    let part = &iso[a..b];
    part.bytes().all(|c| c.is_ascii_digit()).then(|| part.parse::<u64>().ok()).flatten()
  };
  let (year, month, day) = (number(0, 4)?, number(5, 7)?, number(8, 10)?);
  let (hour, minute, second) = (number(11, 13)?, number(14, 16)?, number(17, 19)?);
  if year < 1970 || !(1..=12).contains(&month) || hour > 23 || minute > 59 || second > 59 {
    return None;
  }
  let leap = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
  let months = [31, if leap { 29 } else { 28 }, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
  if day == 0 || day > months[(month - 1) as usize] {
    return None;
  }
  let leaps = |y: u64| y / 4 - y / 100 + y / 400;
  let days =
    (year - 1970) * 365 + leaps(year - 1) - leaps(1969) + months[..(month - 1) as usize].iter().sum::<u64>() + day - 1;
  Some(days * 86400 + hour * 3600 + minute * 60 + second)
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::gh_review::{decide, runnable, MIN_HARNESSES, VERDICT_LOW_QUOTA, VERDICT_RUN};

  const NOW: u64 = 1789948800; // 2026-09-21T00:00:00Z

  fn quota(harness: Harness, used: f64, reset: Option<&str>) -> HarnessQuota {
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
        resets_at: reset.map(str::to_string),
      }],
    }
  }

  fn selected(quotas: &[HarnessQuota]) -> Vec<HarnessPlan> {
    let mut plans: Vec<_> = [Harness::Cursor, Harness::Grok]
      .into_iter()
      .map(|h| decide(h, Ok(()), quotas.iter().find(|q| q.harness == h)))
      .collect();
    select(&mut plans, quotas, NOW);
    assert!(runnable(&plans).len() <= 1, "never duplicate the Grok lane");
    plans
  }

  #[test]
  fn native_pool_gates_cursor_even_when_other_pools_are_exhausted() {
    let mut c = quota(Harness::Cursor, 40.0, None);
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
    assert!(budget(&[c], Harness::Cursor, NOW).is_none());
  }

  #[test]
  fn grok_uses_shared_and_build_limits_but_not_chat_or_voice() {
    let mut g = quota(Harness::Grok, 40.0, None);
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
  fn reset_horizon_can_outweigh_remaining_percentage_in_either_direction() {
    let mut c = quota(Harness::Cursor, 10.0, Some("2026-10-21T00:00:00Z"));
    let mut g = quota(Harness::Grok, 50.0, Some("2026-09-23T00:00:00Z"));
    let plans = selected(&[c.clone(), g.clone()]);
    assert_eq!(runnable(&plans), vec![Harness::Grok]);
    assert_eq!(plans[0].verdict, VERDICT_ALTERNATIVE_SELECTED);
    assert!(plans[0].note.contains("cursor 2.67%, grok 20.00%"));
    c.windows[0].resets_at = Some("2026-09-22T00:00:00Z".into());
    g.windows[0].resets_at = Some("2026-09-28T00:00:00Z".into());
    assert_eq!(runnable(&selected(&[c, g])), vec![Harness::Cursor]);
  }

  #[test]
  fn unknown_and_stale_resets_fall_back_to_spare_quota_and_ties_to_cursor() {
    let c = quota(Harness::Cursor, 40.0, Some("2026-10-21T00:00:00Z"));
    for reset in [None, Some("garbage"), Some("2026-09-20T00:00:00Z"), Some("2026-09-21T00:00:00Z")] {
      let g = quota(Harness::Grok, 30.0, reset);
      let plans = selected(&[c.clone(), g]);
      assert_eq!(runnable(&plans), vec![Harness::Grok]);
      assert!(plans[1].note.contains("reset unavailable or stale"));
    }
    let g = quota(Harness::Grok, 40.0, None);
    assert_eq!(runnable(&selected(&[c, g])), vec![Harness::Cursor]);
    assert_eq!(runnable(&selected(&[])), vec![Harness::Cursor]);
    assert_eq!(runnable(&selected(&[quota(Harness::Grok, 40.0, None)])), vec![Harness::Grok]);
  }

  #[test]
  fn exclusions_survive_selection_and_do_not_count_as_a_second_opinion() {
    let c = quota(Harness::Cursor, 40.0, None);
    let g = quota(Harness::Grok, 94.0, None);
    let plans = selected(&[c, g]);
    assert_eq!(plans[1].verdict, VERDICT_LOW_QUOTA);
    assert_eq!(runnable(&plans), vec![Harness::Cursor]);
    assert!(runnable(&plans).len() < MIN_HARNESSES);
    let mut plans = vec![decide(Harness::Cursor, Err("expired".into()), None), decide(Harness::Grok, Ok(()), None)];
    let original = plans.clone();
    select(&mut plans, &[], NOW);
    assert_eq!(plans, original);
  }

  #[test]
  fn binding_window_and_near_reset_scores_remain_bounded() {
    let mut g = quota(Harness::Grok, 10.0, Some("2026-09-21T00:00:01Z"));
    let b = budget(&[g.clone()], Harness::Grok, NOW).unwrap();
    assert_eq!(b.daily, Some(80.0 * 24.0));
    g.windows.push(QuotaWindow {
      id: "weekly_grokbuild".into(),
      label: "build".into(),
      used_percent: 70.0,
      resets_at: Some("2026-09-23T00:00:00Z".into()),
    });
    let b = budget(&[g], Harness::Grok, NOW).unwrap();
    assert_eq!(b.spare, 20.0);
    assert_eq!(b.daily, Some(10.0));
    for invalid in [f64::NAN, f64::INFINITY, -1.0] {
      assert!(budget(&[quota(Harness::Cursor, invalid, None)], Harness::Cursor, NOW).is_none());
    }
  }

  #[test]
  fn reset_dates_validate_calendar_and_roundtrip_runtime_formatter() {
    assert_eq!(reset_epoch("2026-09-21T00:00:00Z"), Some(NOW));
    assert_eq!(reset_epoch("1970-01-01T00:00:00Z"), Some(0));
    for timestamp in [0, 951782400, NOW, 4107542400] {
      let s = crate::runtime::format_utc_timestamp(timestamp);
      let iso = format!("{}-{}-{}T{}:{}:{}Z", &s[..4], &s[4..6], &s[6..8], &s[9..11], &s[11..13], &s[13..15]);
      assert_eq!(reset_epoch(&iso), Some(timestamp));
    }
    for invalid in [
      "",
      "🤷",
      "2026-02-29T00:00:00Z",
      "2100-02-29T00:00:00Z",
      "2026-00-01T00:00:00Z",
      "2026-13-01T00:00:00Z",
      "2026-01-00T00:00:00Z",
      "2026-04-31T00:00:00Z",
      "2026-01-01T24:00:00Z",
      "2026-01-01T00:60:00Z",
      "2026-01-01T00:00:60Z",
      "2026-01-01T00:00:00+01:00",
    ] {
      assert_eq!(reset_epoch(invalid), None, "{invalid}");
    }
  }
}
