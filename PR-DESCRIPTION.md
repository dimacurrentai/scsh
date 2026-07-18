# Harden the gorgeous fix step; remove the demo-beautiful-loop workflow

## Summary

A gorgeous-pipeline run now survives its own hardest step: the `fix` step — one agent absorbing feedback from all 15 reviewer routes — declares an hour-long screen-novelty window, runs on codex gpt-5.6-sol, narrates its progress so the watchdog always sees a live screen, and reports a one-line changes summary as the job-page headline. Two production jobs (`ehrajj`, `qtsiuf`) had failed precisely here: a healthy fix pass thinks and edits quietly for longer than the default 30-minute window, rendering only spinner frames and a creeping token counter — exactly the output the novelty watchdog deliberately ignores.

With the gorgeous pipeline established as the one canonical review loop, the superseded `demo-beautiful-loop` workflow and its `init-beautiful-demo` scaffold are removed in the same change set — a deliberate product decision: the scaffolded word-counting demo taught nothing the canonical workflow does not, and keeping two review loops meant every fix-step improvement would either fork the demo or silently leave it behind. Removing a public command and a builtin workflow is a breaking change, so the release is a minor bump: **1.31.0**.

## What This Changes

- Workflow def steps accept `inactivity_timeout:` (plain seconds, same semantics as the skill-level key; `retry_for`'s sibling), plumbed through `step_invocation` to the screen-novelty watchdog.
- The gorgeous `fix` step runs on codex gpt-5.6-sol with `inactivity_timeout: 3600`, prints one short progress line per reviewer profile taken up and per meaningful change, and reports a one-line changes summary in a new `message` result field alongside the detailed `actions` paragraphs.
- The Setup page's codex catalog offers gpt-5.6-sol as a built-in (opt-in) smoke model, landing one commit before the workflow depends on it.
- `demo-beautiful-loop`, `init-beautiful-demo`, `src/demo_beautiful/`, `DEMO-BEAUTIFUL-LOOP.md`, their tests, and every doc reference are removed. The beautiful skills family (big-beautiful-build and friends) is untouched.

## Implementation Details

The step parser gains `inactivity_timeout` beside `retry_for` (shared `parse_positive_secs_at` helper so step errors read `steps.<id>.…`, not `skills.…`), `step_invocation` forwards it (asserted by a unit test), and `config::effective_inactivity_timeout` consumes it unchanged. The fix step's `message` field is picked up by `json::message` as the proc headline because it prefers `result`/`message` keys. The loop-carried-input unit test that exercised the demo def now pins the same channel in `gorgeous-pipeline` — `decide` reading the previous round's `collect.feedback`. Catalog primaries are unchanged, preserving the catalog-primaries == doctor.yml invariant.
