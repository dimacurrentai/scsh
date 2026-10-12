# HEADLESS-COMPARE.md — interactive vs headless routes, side by side

This file **is** the test. It is an executable Markdown harness: an agent — or a human — runs the numbered steps in order and checks each **Predict** line. There is no runner script.

It proves that the three headless invocation types — `claude-headless`, `codex-headless`, and `cline-headless` — run end to end without a terminal recording, render a transcript in the session browser, and report token usage on the same basis as their interactive twins. The `headless-compare` profile in [`.scsh.yml`](.scsh.yml) runs the minimal [`.skills/harness-smoke/SKILL.md`](.skills/harness-smoke/SKILL.md) workload six times: each of Claude, Codex, and Cline once as its recorded TUI (`cmp-<agent>-tui`) and once headless (`cmp-<agent>-headless`), with the same model and effort. Routes whose agent has no host login are skipped, not failed.

## Prerequisites

1. **A container runtime** — docker, podman, or Apple `container` (macOS), engine running.
2. **At least one of** Claude (`claude` logged in, or `CLAUDE_CODE_OAUTH_TOKEN`), Codex (`codex login`, or `OPENAI_API_KEY`), Cline (`cline auth`, or `CLINE_API_KEY`).
3. **`jq`** for step 7.

## The harness (run these in order)

Run from the repo root. `SCSH` is this repo's own build (`./target/debug/scsh`), not an older `scsh` on `PATH`, which does not know the headless types. The job page must be rendered by that same build, so the harness runs its own daemon on a spare port and stops it at the end; the daemon you normally use is never restarted. Every command below runs with `SCSH_DAEMON_PORT=7399` exported (any free port works).

1. **Build.** `cargo build`
   - **Predict:** exit 0.

2. **Daemon.** `export SCSH_DAEMON_PORT=7399 && $SCSH daemon start`
   - **Predict:** a `✓` line naming `http://127.0.0.1:7399`. If the port is taken, pick another one and use it everywhere below.

3. **Clean tree.** `git status --porcelain`
   - **Predict:** empty output. (`scsh run` clones committed state only.)

4. **Profile.** `$SCSH check-profile headless-compare`
   - **Predict:** `profile 'headless-compare' has 6 skills`.

5. **Inspect, no model call.** `$SCSH inspect-prompt headless-compare | jq -r '.InvocationInspection[] | .InvocationManifest | "\(.harness) \(.execution_mode)"'`
   - **Predict:** six lines; every `*-headless` harness reads `headless`, every other one `interactive`. The two routes of one agent carry the byte-identical `submitted_prompt`.

6. **Run.** `$SCSH run --profile headless-compare`
   - **Predict:** exit 0, one `✓` line per available route, and a final `✓ job http://127.0.0.1:7399/job/<id>` line. No headless route builds an image of its own: the build rows name only `claude`, `codex`, and `cline`. Keep the job id: `JOB=<id>`.

7. **Usage on one basis.** For each agent with both routes present:

   ```sh
   for a in claude codex cline; do
     for m in tui headless; do
       f=~/.scsh/sessions/$JOB/results/harness-smoke-cmp-$a-$m.usage.json
       [ -f "$f" ] && jq -r --arg r "$a-$m" '.TokenUsage | "\($r)\t\(.harness)\t\(.source)\t\(.complete)\t\(.tokens.input)\t\(.tokens.output)\t\(.tokens.cache_read)\t\(.llm_round_trips)\t\(.tool_calls)"' "$f"
     done
   done
   ```

   - **Predict:** both routes of an agent name the same `harness` and `source` (for example `claude_code` / `claude_session_jsonl`), and every row is `complete: true` with numeric tokens. That is the accounting contract: a headless route is read from the same native session files, by the same parser, as its TUI twin. The numbers themselves may differ, and the difference is real — Claude's interactive mode sends tool schemas that print mode does not (about 12k input tokens on a one-turn task, see [CLAUDE-PROMPTS-RESULTS.md](CLAUDE-PROMPTS-RESULTS.md)).

8. **Browser.** Open `http://127.0.0.1:7399/job/$JOB`.
   - **Predict:** each `*-tui` row embeds the terminal player. Each `*-headless` row instead shows a transcript: a toolbar (`⬇ events.jsonl`, message and tool-call counts), the agent's messages, one collapsible row per tool call (expand it: input and output), and a closing `result` row. While a headless route runs, its transcript grows live and ends with an orange `working` line. The usage line under each finished row has the same shape for both routes.

9. **Snapshot.** `$SCSH export-job $JOB -o tmp/headless-compare.html`, then open the file.
   - **Predict:** the headless rows carry the same transcripts, offline.

10. **Stop the daemon.** `$SCSH daemon stop`, still with `SCSH_DAEMON_PORT=7399` exported.
    - **Predict:** a `✓` line; `$SCSH daemon status` then reports nothing listening on 7399. Run this step even when an earlier one failed.

## Pass / fail

| Check | PASS when |
| --- | --- |
| Run | `scsh run --profile headless-compare` exits 0 (skipped agents don't fail it) |
| Accounting | Every present usage file is `complete`, and paired routes share `harness` and `source` |
| Browser | Headless rows show a transcript and no player; TUI rows show the player |
| Cleanup | The spare-port daemon is stopped |

**Overall PASS** = all four rows pass. On failure, the headless route's raw stream is `~/.scsh/sessions/<id>/logs/<route>-….events.jsonl`, beside its stderr log.
