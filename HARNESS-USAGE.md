# Harness usage accounting

`scsh` collects token usage after every Claude Code, Codex, and Cursor attempt without
changing the interactive TUI or making another provider request. Collection happens before
the attempt's forwarded credentials and temporary config are scrubbed:

- Claude Code: sum the `usage` object on each unique assistant response in the fresh
  transcript tree, including subagent transcripts.
- Codex: keep the last cumulative `total_token_usage` record in each fresh session file,
  then sum independent parent and subagent sessions. Forked sessions with inherited
  accounting are marked unavailable rather than double-counting the parent.
- Cursor: keep the last cumulative token payload for each conversation from user-level
  TUI hooks. The hooks live in the forwarded config home, never in the skill repository.

The session browser shows one small usage line below the recording only after the attempt
has finished. A cache hit launches no harness and therefore creates no new usage record.

## Stable schema

The same strict object is available at `procs[].usage` from
`GET /api/v1/session/{id}`, in fleet route JSON, and in
`$SCSH_HOME/sessions/<session>/results/<invocation>.usage.json`:

```json
{
  "TokenUsage": {
    "schema_version": 1,
    "harness": "claude_code",
    "source": "claude_session_jsonl",
    "complete": true,
    "tokens": {
      "input": 120,
      "output": 30,
      "cache_read": 400,
      "cache_write": 20
    },
    "llm_round_trips": 3,
    "tool_calls": 2
  }
}
```

`harness` is exactly `claude_code`, `codex`, or `cursor`; its corresponding `source` is
exactly `claude_session_jsonl`, `codex_session_jsonl`, or `cursor_hooks`. Unknown fields,
versions, harnesses, and harness/source combinations are rejected by the reader.

`tokens.input` is uncached input. Claude already reports that bucket separately; Codex and
Cursor input counters include cached input, so `scsh` subtracts the cache buckets. Missing
accounting is `tokens: null` with `complete: false`, never a fabricated zero. Counters are
observable client records rather than a billing statement. Where a harness records
subagents locally, their session files are included.

Every field is required. Counts are nonnegative integers no greater than
9,007,199,254,740,991. `cache_write`, `llm_round_trips`, and `tool_calls` may be `null`
when the harness does not expose them reliably. In particular, Codex token snapshots
do not identify model round-trips or tool calls. An interrupted attempt has
`complete: false` even when some counters were recovered. Unsupported harnesses and
cache hits have `usage: null`; they do not pretend to have measured usage.

Collection reads local files once after the process exits; it makes no provider request.
Cursor additionally appends one local record per hook. The cost scales with the run's
transcript size. The results-side file contains the latest attempt; uniquely named log
files and individual session procs preserve each attempt separately.

The original Cursor hook stream is retained under `logs/<stem>.cursor-hooks.jsonl` for
verification. Every harness's normalized summary is retained as `logs/<stem>.usage.json`.

## Verification

Offline parser and schema fixtures run with `cargo test --bin scsh usage::`; UI placement
is covered by `usage_appears_only_after_a_finished_player`. `HARNESS-SMOKE.md` describes the
live, human-followable check across all configured routes.
