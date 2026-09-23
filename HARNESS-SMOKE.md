# HARNESS-SMOKE.md — executable harness for the claude, codex, and cursor containers

This file **is** the test. It is an executable Markdown harness (per the repo's testing
principle): an agent — or a human — runs the numbered steps in order and checks each
**Predict** line. `scripts/harness-smoke.sh` is a thin runner that executes exactly these
steps; running the script and following this doc are equivalent.

It exercises the **claude**, **codex**, and **cursor** container harnesses end to end via the
minimal skill [`.skills/harness-smoke/SKILL.md`](.skills/harness-smoke/SKILL.md): each harness
runs its real interactive TUI (recorded to `~/.scsh/sessions/<session>/casts/`) and must write a tiny JSON
`{"result":{"status":"OK",…}}` file. scsh itself probes host auth and **skips** any harness
that is not logged in, so the run succeeds as long as every *available* harness does.

## Prerequisites

1. **A container runtime** — docker, podman, or Apple `container` (macOS), engine running.
2. **At least one harness logged in on the host** (scsh forwards these into the container;
   missing ones are skipped, not failed):
   - **Claude** — `claude` logged in (macOS keychain), `~/.claude/.credentials.json`, or `CLAUDE_CODE_OAUTH_TOKEN`.
   - **Codex** — `~/.codex/auth.json` (`codex login`) or `OPENAI_API_KEY`.
   - **Cursor** — `cursor-agent` logged in (keychain / `~/.cursor/auth.json`) or `CURSOR_API_KEY`.
3. **`jq`** (optional) — validates the result JSON; without it the runner only checks the file exists.

## The harness (run these in order)

Run from the repo root. `SCSH` = this repo's own build (`./target/debug/scsh` or
`./target/release/scsh`), **not** any older `scsh` on `PATH` — an older binary may not know
these harnesses.

1. **Build.** `cargo build`
   - **Predict:** exit 0; `./target/debug/scsh` exists.

2. **Clean tree.** `git status --porcelain` — commit or stash first if dirty.
   - **Predict:** empty output. (`scsh run` clones committed state only.)

3. **Profile exists.** `$SCSH check-profile harness-smoke`
   - **Predict:** exit 0, prints `profile 'harness-smoke' has 3 skills`.

4. **Run.** `$SCSH run --profile harness-smoke`
   - **Predict:** exit 0. Each *available* harness prints `✓ <harness>: harness-smoke-<route>`.
     Unavailable harnesses print an `N/A`/skip line and do **not** fail the run.

5. **Validate results.** For each `tmp/harness-smoke-<route>.json` that exists
   (`claude-opus-4-8`, `codex-luna`, `cursor-composer-fast`): `jq .result.status <file>`
   - **Predict:** every present file reads `"OK"`, and **at least one** file is present.

6. **Screencasts recorded.** `ls -t ~/.scsh/sessions/*/casts/harness-smoke-*.cast`
   - **Predict:** one fresh `<route>-<YYYYMMDD-HHMMSS>-utc-<nonce>.cast` per succeeded route
     (real interactive TUI; replay with `asciinema play <file>` or the session browser).

7. **Harness usage.** Each Claude Code, Codex, and Cursor route has a matching
   `~/.scsh/sessions/*/results/<route>.usage.json`.
   - **Predict:** the strict `TokenUsage` document identifies the harness and source,
     then reports uncached input, output, cache-read, and cache-write tokens. The same
     small line appears below the finished recording in the session browser. A route
     that stopped before its harness emitted accounting says `tokens: null` and
     `complete: false`; it never invents zero spend.

## One-command runner

```sh
cd /path/to/scsh
./scripts/harness-smoke.sh        # runs steps 1–6 and prints PASS / FAIL
```

## Pass / fail

| Check | PASS when |
| --- | --- |
| Preflight | Clean tree; `check-profile harness-smoke` exits 0; a container runtime is up |
| Run | `scsh run --profile harness-smoke` exits 0 (skipped harnesses don't fail it) |
| Results | Every present `tmp/harness-smoke-<route>.json` has `result.status == "OK"`, ≥1 present |

**Overall PASS** = all three rows pass. On failure, inspect the persisted log under
`~/.scsh/sessions/<id>/logs/` (the run clone is removed when the attempt finishes).

## Ephemeral /tmp

After a harness image exists (`scsh build-images`, or a previous run built it):

```sh
(
set -eu
runtime=docker   # or: container, podman
probe="scsh-tmpfs-probe-$$"
cleanup_probe() {
  if [ "$runtime" = container ]; then
    "$runtime" stop "$probe" 2>/dev/null || true
    "$runtime" delete "$probe" 2>/dev/null || true
  else
    "$runtime" rm -f "$probe" 2>/dev/null || true
  fi
}
trap cleanup_probe EXIT
trap 'exit 130' INT
trap 'exit 143' TERM
"$runtime" run --rm --name "$probe" --tmpfs /tmp:rw,nosuid,nodev,size=256m,mode=1777 \
  scsh-claude:latest /bin/sh -ec '
    test "$(id -u)" != 0
    test "$(stat -f -c %T /tmp)" = tmpfs
    test "$(stat -c %a /tmp)" = 1777
    test "$(stat -f -c %b /tmp)" -eq 65536
    test "$(stat -f -c %S /tmp)" -eq 4096
    touch /tmp/scsh-tmpfs-probe
  '
"$runtime" run --rm --name "$probe" --tmpfs /tmp:rw,nosuid,nodev,size=256m,mode=1777 \
  scsh-claude:latest /bin/sh -ec 'test ! -e /tmp/scsh-tmpfs-probe'
cleanup_probe
trap - EXIT INT TERM
)
```

**Predict:** both commands exit zero: the filesystem type is `tmpfs`, its capacity
is 256 MiB, `mode` is `1777`, `touch` succeeds as the image's agent user, and a second
`run` does not see `/tmp/scsh-tmpfs-probe`. The real `scsh run` argv is
`--tmpfs /tmp:rw,nosuid,nodev,size=256m,mode=1777`. This checks the mount. It does not
reproduce a guest root that returned EROFS.

The network-free cleanup regression tests cover setup failures before cloning,
artifact-copy failure and retry, a key-channel sibling directory, protection of
a live runner's commits, commit recovery after clone removal, and an interrupted
runner whose container survives. Run them with `cargo test cleanup::tests`.
Runtime deadline and inspection parsing tests live in `src/runtime.rs`.

After a real `scsh` run, use `scsh prune` to inspect any outstanding ownership
records. Successful teardown leaves neither the attempt's system-temp run directory
nor its key-channel directory. A copy or engine failure must be reported as pending;
once corrected, `scsh prune --now` retries cleanup without rerunning the harness.

## For agents

Execute the numbered steps above (or run `./scripts/harness-smoke.sh`) and report **PASS**
or **FAIL** with the per-route result. The skill under test is intentionally minimal:
[.skills/harness-smoke/SKILL.md](.skills/harness-smoke/SKILL.md).
