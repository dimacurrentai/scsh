# Claude prompts and accounting in `scsh`

Claude routes default to interactive execution and the existing generated contract. Set `claude_mode: headless` to run the pinned Claude CLI with `--print --verbose --output-format stream-json --include-partial-messages`. It exits naturally, streams progress to the run log, and produces no fabricated terminal recording. `effort: medium` now reaches Claude as `--effort medium` in either mode.

Choose `prompt_contract: standard`, `compact`, or `verbatim`. Standard retains the existing prose. Compact keeps required outputs, artifacts, input variable names, script instructions, loop controls, commit obligations, and the local-Git guard while omitting empty inputs and optional dashboard advertising. Verbatim sends the authored task exactly as parsed, including its trailing whitespace, without appending any instructions. In verbatim mode the caller owns result-path instructions, local-Git restrictions, and any workflow contract. Result and artifact validation still run, and invalid-result retries do not silently rewrite a verbatim prompt.

For a small task, use the existing flat definition format in `.harness/answer.yml`:

```yaml
description: Answer a small question
task: |
  Compute 19 * 23. Write a JSON object with answer and summary to $SCSH_RESULT.
invocations:
  claude:
    harness: claude
    model: claude-opus-5-5
    effort: medium
    claude_mode: headless
    prompt_contract: verbatim
```

Run `scsh inspect-prompt --def answer` to inspect the final prompt, its byte count and SHA-256, the execution policy, and the actual argument vector before calling a model. Then run `scsh run --def answer --retries 0`. Inspection needs neither a container nor model credentials. Workflow inspection uses `tmp/inspection` as a placeholder result directory; actual input values are supplied through the declared environment bindings when the workflow runs.

Workflow steps take the same options inside `agent:`. A `.scsh.yml` skill accepts the execution mode directly or on an individual invocation route; put policy options on individual routes in a matrix. Verbatim requires a definition with an authored task or prompt. Claude execution options on other harnesses are rejected, as are unknown policy values.

## Diagnostic artifacts

Every real attempt saves an `*.invocation.json` alongside its durable session logs. This is a separate `InvocationManifest` document with `schema_version: 1`; existing aggregate usage JSON is unchanged. It distinguishes authored text, generated contract, delivered body, and submitted prompt, records requested model and effort, identifies the image and source build, and hashes available project instructions and configuration. Missing provider context and unobservable effective settings remain null. The project MCP inventory is configuration evidence, not proof that a server loaded. Environment dumps, authentication files, and MCP endpoint URLs are excluded.

Claude attempts also save `*.usage-ledger.json`. This `ClaudeUsageLedger` is produced by the same native parser as the aggregate usage summary. Each logical response is keyed by provider message/request identity, takes the maximum of streamed counters, and retains source file/line references, session IDs, observed model, timestamps, and tool identities/names. Malformed or unreadable records make aggregate tokens unavailable rather than zero. Native transcript bodies are not exported by this ledger. Headless output logs contain the CLI's streamed messages and should be handled as task data.

Use `scsh inspect-claude-usage <native-transcript-directory>` to apply this same parser to direct-Claude control runs. Exit zero means complete native accounting; missing or malformed evidence exits nonzero while preserving the diagnostic document.

The four token buckets are disjoint: uncached input, output, cache reads, and cache writes. Total processed tokens are their sum, not a dollar estimate. Rounds count logical provider responses, not tools. Tool calls deduplicate tool IDs across the attempt; names beginning with `mcp__` identify observed MCP calls. An attempt's ledger does not include other attempts. Whole-job cost must sum every attempt, including failures and retries; the successful result's sidecar alone is insufficient.

## Remote verification

Use only `ubuntu@mcpclaude` (`170.75.160.233`), rootless Podman, and a remote `tmux` session for verification. Finish the frozen mock-memory campaign and its queued report validation before starting another workload. Install the candidate under a separate name, for example `~/scsh-token-lab/bin/scsh-token-lab`; preserve the installed `scsh` executable and use a separate `SCSH_HOME` and daemon port.

Run the repository gates in that separate source checkout: formatting, debug/release clippy and builds, both Rust test profiles, and the Python gate tests. Bound unit-test execution to 30 seconds after compilation and integration tests to a few minutes. Preserve their full output with `tee`.

The remote-only comparison script prepares committed, deterministic fixtures without model calls by default:

```sh
python3 scripts/claude-parity.py \
  --binary "$HOME/scsh-token-lab/bin/scsh-token-lab" \
  --output "$HOME/scsh-token-lab/prepared-fixtures"
```

To execute five samples per cell, supply a fresh output directory and a private JSON credential file with `access_token` and `expires_at` (Unix seconds):

```sh
python3 scripts/claude-parity.py \
  --binary "$HOME/scsh-token-lab/bin/scsh-token-lab" \
  --output "$HOME/scsh-token-lab/comparison-001" \
  --auth-file "$HOME/scsh-token-lab/private/auth.json" \
  --samples 5 --execute
```

The runner rotates six arms deterministically: direct host print, direct container print, direct container interactive, interactive `scsh`, headless `scsh`, and compact headless `scsh`. All runs are sequential and use fresh clones/config directories, the same pinned model and medium effort, a fixed image ID, and explicitly disabled prompt caching. A nonce changes between repetitions but stays identical within matched controls so prompt bytes remain comparable. Cache counters, actual models, correctness, failures, and per-attempt ledgers remain in the report. Cache hits or unavailable usage stop the campaign without discarding its failing sample.

The one-turn fixture requests `PARITY_OK` without tools. A deterministic project Stop hook serializes the final response, so a result file does not itself require an LLM tool call; this uses Claude's documented [`last_assistant_message` hook input](https://code.claude.com/docs/en/hooks#stop-input). Confirm zero native tool calls before treating a sample as a one-turn attribution control. Its A–E arms use verbatim prompts; F adds a compact contract, so that particular contrast measures adding a contract, not shortening the standard one.

The repository fixture computes a small dependency closure and asks the model to write its result file, checked externally against `api,core,ui`. Its Stop hook leaves the file untouched: extra prose in the final chat reply cannot overwrite a valid result, and a missing result remains a failure. A–E use the standard rendered contract and F uses compact rendering with the same task and validation. Protocol 2 also matches the direct controls' Git branch and identity to the workflow clone because Claude can include Git context in its input. The plan records the protocol and runner hash. These are launcher controls; they make no claim about naturally acquired memory.

Inspect every sample in `comparison.json`, including paired differences, medians, and ranges. Host paths/configuration are an intentional environmental difference in the first arm. The direct interactive control uses the same recorded CLI command and releases `/exit` only after a terminal native turn. If tool counts, effective settings, or prompt hashes differ in an otherwise matched pair, investigate those differences before attributing tokens to orchestration. No exact parity or performance threshold is promised before measurement.

The completed five-repeat comparison, including every sample and its limits, is recorded in [CLAUDE-PROMPTS-RESULTS.md](CLAUDE-PROMPTS-RESULTS.md).
