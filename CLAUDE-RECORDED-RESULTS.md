# Recorded Claude prompt comparisons

Measured on 2026-10-05 against product commit `a94ef17`, after renewing server authentication. Every `scsh` invocation used the real interactive Claude TUI and tmux/asciinema recording. All 50 samples passed external correctness, native task-response accounting, requested-model, prompt-hash, image, and zero-cache checks. All 30 recorded arms produced nonempty asciinema streams. No print-mode route exists in the candidate.

## Reproduction

Run the protocol-3 procedure in [CLAUDE-PROMPTS.md](CLAUDE-PROMPTS.md) with five repetitions. The machine was `ubuntu@mcpclaude` (`170.75.160.233`), using rootless Podman, Claude Code 2.1.287, `claude-opus-5-5`, and medium effort. Each sample used a fresh clone/config tree, one attempt, and no MCP calls. Execution was sequential under the experiment locks.

- Candidate binary SHA-256: `fc6b71cda81eb7753a50abd32a91eec12f108afeff35a377fa0a4c8c3dd99468`.
- Runner SHA-256: `ce721ac42f60b8dedeffc7fb9fffc03c880aca302cdd0a8676d99f93e8a0e1c1`.
- Fixture revision: `94d049637bdeefa40a212f41aef7465c0214568a`.
- Container image: `a1a292372abefa87f5f89b232a0f4e879cb862031ccbd1494becbb7f11edb106`.

Full manifests, native ledgers, recordings, and reports are retained under `~/scsh-token-lab/comparison-20261005-recorded/`. Local aggregate copies are under repository `tmp/token-lab/results-recorded/`.

## Paired differences

Values are right minus left in repetition order. Total tokens are uncached input + output + cache reads + cache writes. Both cache buckets were zero. These are recorded task-response counters; auxiliary CLI requests such as title generation are not included, so they are not an account-wide bill.

| Task | Left → right | Five total-token differences | Median | Range |
| --- | --- | --- | ---: | --- |
| one-turn | `container-print` → `container-interactive` | 16040, 16039, 16039, 16041, 16036 | 16039 | 16036…16041 |
| one-turn | `container-interactive` → `scsh-interactive` | 6, -2, 1, -1, 5 | 1 | -2…6 |
| one-turn | `scsh-interactive` → `scsh-compact` | 83, 89, 95, 89, 86 | 89 | 83…95 |
| one-turn | `container-print` → `scsh-interactive` | 16046, 16037, 16040, 16040, 16041 | 16040 | 16037…16046 |
| repository | `container-print` → `container-interactive` | 48141, 48182, 48133, 48044, 48136 | 48136 | 48044…48182 |
| repository | `container-interactive` → `scsh-interactive` | 33, -40, 84, -51, -9 | -9 | -51…84 |
| repository | `scsh-interactive` → `scsh-compact` | -543, -490, -634, -540, -572 | -543 | -634…-490 |
| repository | `container-print` → `scsh-interactive` | 48174, 48142, 48217, 47993, 48127 | 48142 | 47993…48217 |

The one-turn print and interactive controls had identical submitted prompt hashes, one provider response, zero tool calls, and nine output tokens. The remaining difference is in input context added by Claude, not generated result instructions. The one-turn compact arm adds a contract to an otherwise verbatim task; it is not a shortening comparison. The repository compact arm shortens the standard contract while preserving the same task and external validation.

## Every sample

`Wall` includes model and launcher execution; provider-only timing is unavailable. `Setup` is fixture preparation before launch. All samples were correct and eligible.

| Task | Arm | Repeat | Input | Output | Total | Rounds | Tools | Setup (s) | Wall (s) |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| one-turn | `host-print` | 1 | 16279 | 9 | 16288 | 1 | 0 | 0.069 | 3.458 |
| one-turn | `container-print` | 1 | 16166 | 9 | 16175 | 1 | 0 | 0.081 | 6.495 |
| one-turn | `container-interactive` | 1 | 32206 | 9 | 32215 | 1 | 0 | 0.067 | 8.891 |
| one-turn | `scsh-interactive` | 1 | 32212 | 9 | 32221 | 1 | 0 | 0.067 | 11.493 |
| one-turn | `scsh-compact` | 1 | 32295 | 9 | 32304 | 1 | 0 | 0.093 | 11.151 |
| one-turn | `container-print` | 2 | 16173 | 9 | 16182 | 1 | 0 | 0.063 | 3.977 |
| one-turn | `container-interactive` | 2 | 32212 | 9 | 32221 | 1 | 0 | 0.122 | 9.386 |
| one-turn | `scsh-interactive` | 2 | 32210 | 9 | 32219 | 1 | 0 | 0.067 | 11.594 |
| one-turn | `scsh-compact` | 2 | 32299 | 9 | 32308 | 1 | 0 | 0.064 | 12.361 |
| one-turn | `host-print` | 2 | 16286 | 9 | 16295 | 1 | 0 | 0.069 | 3.556 |
| one-turn | `container-interactive` | 3 | 32200 | 9 | 32209 | 1 | 0 | 0.066 | 8.899 |
| one-turn | `scsh-interactive` | 3 | 32201 | 9 | 32210 | 1 | 0 | 0.080 | 11.093 |
| one-turn | `scsh-compact` | 3 | 32296 | 9 | 32305 | 1 | 0 | 0.070 | 11.408 |
| one-turn | `host-print` | 3 | 16274 | 9 | 16283 | 1 | 0 | 0.064 | 3.478 |
| one-turn | `container-print` | 3 | 16161 | 9 | 16170 | 1 | 0 | 0.070 | 3.853 |
| one-turn | `scsh-interactive` | 4 | 32207 | 9 | 32216 | 1 | 0 | 0.062 | 11.563 |
| one-turn | `scsh-compact` | 4 | 32296 | 9 | 32305 | 1 | 0 | 0.069 | 11.126 |
| one-turn | `host-print` | 4 | 16280 | 9 | 16289 | 1 | 0 | 0.064 | 3.585 |
| one-turn | `container-print` | 4 | 16167 | 9 | 16176 | 1 | 0 | 0.066 | 4.067 |
| one-turn | `container-interactive` | 4 | 32208 | 9 | 32217 | 1 | 0 | 0.061 | 8.797 |
| one-turn | `scsh-compact` | 5 | 32289 | 9 | 32298 | 1 | 0 | 0.065 | 11.713 |
| one-turn | `host-print` | 5 | 16275 | 9 | 16284 | 1 | 0 | 0.069 | 3.557 |
| one-turn | `container-print` | 5 | 16162 | 9 | 16171 | 1 | 0 | 0.062 | 3.836 |
| one-turn | `container-interactive` | 5 | 32198 | 9 | 32207 | 1 | 0 | 0.066 | 8.833 |
| one-turn | `scsh-interactive` | 5 | 32203 | 9 | 32212 | 1 | 0 | 0.063 | 12.323 |
| repository | `host-print` | 1 | 50274 | 355 | 50629 | 3 | 2 | 0.066 | 15.315 |
| repository | `container-print` | 1 | 49991 | 340 | 50331 | 3 | 2 | 0.075 | 8.329 |
| repository | `container-interactive` | 1 | 98113 | 359 | 98472 | 3 | 2 | 0.065 | 12.003 |
| repository | `scsh-interactive` | 1 | 98124 | 381 | 98505 | 3 | 2 | 0.067 | 16.462 |
| repository | `scsh-compact` | 1 | 97571 | 391 | 97962 | 3 | 2 | 0.061 | 17.125 |
| repository | `container-print` | 2 | 49994 | 314 | 50308 | 3 | 2 | 0.065 | 8.745 |
| repository | `container-interactive` | 2 | 98113 | 377 | 98490 | 3 | 2 | 0.064 | 12.234 |
| repository | `scsh-interactive` | 2 | 98105 | 345 | 98450 | 3 | 2 | 0.067 | 16.772 |
| repository | `scsh-compact` | 2 | 97580 | 380 | 97960 | 3 | 2 | 0.064 | 16.526 |
| repository | `host-print` | 2 | 50359 | 389 | 50748 | 3 | 2 | 0.067 | 11.335 |
| repository | `container-interactive` | 3 | 98105 | 373 | 98478 | 3 | 2 | 0.075 | 12.102 |
| repository | `scsh-interactive` | 3 | 98183 | 379 | 98562 | 3 | 2 | 0.071 | 23.356 |
| repository | `scsh-compact` | 3 | 97571 | 357 | 97928 | 3 | 2 | 0.063 | 17.231 |
| repository | `host-print` | 3 | 50359 | 378 | 50737 | 3 | 2 | 0.070 | 7.788 |
| repository | `container-print` | 3 | 49994 | 351 | 50345 | 3 | 2 | 0.068 | 11.521 |
| repository | `scsh-interactive` | 4 | 98128 | 355 | 98483 | 3 | 2 | 0.068 | 19.260 |
| repository | `scsh-compact` | 4 | 97577 | 366 | 97943 | 3 | 2 | 0.067 | 17.385 |
| repository | `host-print` | 4 | 50351 | 390 | 50741 | 3 | 2 | 0.065 | 8.130 |
| repository | `container-print` | 4 | 50070 | 420 | 50490 | 3 | 2 | 0.072 | 9.097 |
| repository | `container-interactive` | 4 | 98170 | 364 | 98534 | 3 | 2 | 0.068 | 11.879 |
| repository | `scsh-compact` | 5 | 97570 | 347 | 97917 | 3 | 2 | 0.068 | 17.893 |
| repository | `host-print` | 5 | 50285 | 324 | 50609 | 3 | 2 | 0.065 | 8.172 |
| repository | `container-print` | 5 | 50000 | 362 | 50362 | 3 | 2 | 0.065 | 7.429 |
| repository | `container-interactive` | 5 | 98123 | 375 | 98498 | 3 | 2 | 0.090 | 12.376 |
| repository | `scsh-interactive` | 5 | 98120 | 369 | 98489 | 3 | 2 | 0.066 | 17.152 |

## Explicit tool-denial control

A separate five-repeat one-turn experiment added the following to the committed fixture’s `.claude/settings.json`. It retained the same model, effort, hooks, result validation, and recorded launch path. This changes available capabilities and is not a default imposed by `scsh`.

```json
{"permissions":{"deny":["Artifact","AskUserQuestion","SendFeedback"]}}
```

The three arms were direct container print, direct container interactive, and interactive `scsh`, rotated sequentially. All 15 samples passed correctness, accounting, model, zero-cache, and prompt checks; all ten recorded samples had terminal output. Each had one response, zero tool calls, and nine output tokens.

| Left → right | Five total-token differences | Median | Range |
| --- | --- | ---: | --- |
| `container-print` → `container-interactive` | 1062, 1061, 1064, 1064, 1065 | 1064 | 1061…1065 |
| `container-interactive` → `scsh-interactive` | 0, 5, 0, 1, -4 | 0 | -4…5 |
| `container-print` → `scsh-interactive` | 1062, 1066, 1064, 1065, 1061 | 1064 | 1061…1066 |

This removes most of the gap without removing recording. It still does not establish token equality with print mode. After denying the three tools, native snapshots show matching tool names with a remaining `Agent` schema difference, plus different system instructions, agent/skill listings, environment data, deferred-tool listings, and CLI prefixes.

The unrestricted repetition-1 snapshots contain 42,629 serialized tool-schema bytes in print mode and 113,701 in interactive mode. The interactive `Artifact` schema alone is 54,964 bytes. These byte counts are not token estimates. The same pinned CLI/image produced a smaller `Artifact` description in the earlier campaign, so those pins alone did not freeze rendered request context. The source of that cross-campaign variation was not established.

- Fixture revision: `5bbc74e8db0fea127d92d9d2e75b18fba126134d`.
- Additional runner SHA-256: `a93c4b8b9caf4d9fc272c760c619f15da8fac35b41194a9e2e1d51773a8605ed`.

The exact helper is preserved at `~/scsh-token-lab/context-ablation.py` and local repository `tmp/token-lab/context-ablation.py`. It imports the committed comparison runner, prepares five repeats, commits the tool-denial settings, selects the three one-turn arms, and uses the same sample/accounting functions. Its reports, fixture, native evidence, and recordings are under `~/scsh-token-lab/comparison-20261005-denied-tools/`; local aggregate copies are in `tmp/token-lab/results-denied-tools/`.

| Arm | Repeat | Input | Output | Total | Wall (s) |
| --- | ---: | ---: | ---: | ---: | ---: |
| `container-print` | 1 | 16189 | 9 | 16198 | 4.072 |
| `container-interactive` | 1 | 17251 | 9 | 17260 | 8.816 |
| `scsh-interactive` | 1 | 17251 | 9 | 17260 | 11.244 |
| `container-interactive` | 2 | 17246 | 9 | 17255 | 9.251 |
| `scsh-interactive` | 2 | 17251 | 9 | 17260 | 11.117 |
| `container-print` | 2 | 16185 | 9 | 16194 | 3.978 |
| `scsh-interactive` | 3 | 17257 | 9 | 17266 | 11.059 |
| `container-print` | 3 | 16193 | 9 | 16202 | 3.811 |
| `container-interactive` | 3 | 17257 | 9 | 17266 | 8.777 |
| `container-print` | 4 | 16188 | 9 | 16197 | 3.958 |
| `container-interactive` | 4 | 17252 | 9 | 17261 | 8.778 |
| `scsh-interactive` | 4 | 17253 | 9 | 17262 | 11.280 |
| `container-interactive` | 5 | 17258 | 9 | 17267 | 8.941 |
| `scsh-interactive` | 5 | 17254 | 9 | 17263 | 11.678 |
| `container-print` | 5 | 16193 | 9 | 16202 | 4.484 |

## Verification after authentication renewal

The previously blocked `claude_add_skill_runs_when_configured` integration test passed separately in both debug and release, using the same compiled artifacts as the credential-free gates. Combined with those gates, each profile has 920 passing Rust tests (852 unit and 68 CLI), with one normally ignored smoke test. The Python gates passed 23 tests with four platform-dependent skips on the server; the local push gate passed all 27 Python tests. Debug/release builds and clippy were warning-free. Both live-test daemons were stopped, and no test containers remained running.

The original campaign runner reported success even though normal shutdown could not identify the renamed candidate’s daemon. Both comparison daemons were subsequently identified by their exact executable and hidden daemon command and stopped. The follow-up runner correction executes an identical copy under a private `bin/scsh` path and propagates shutdown errors; a model-free regression starts and stops a daemon using a renamed input candidate. With that regression, all 28 Python tests pass locally; 24 pass with four platform skips on the server. The measured runner remains archived in local `tmp/token-lab/results-recorded/runner.py`. This correction changes test setup/cleanup, not the submitted prompt or recorded Claude command.
