#!/usr/bin/env python3
"""Sequential remote Claude controls. See CLAUDE-PROMPTS.md before executing."""

import argparse
import hashlib
import json
import os
from pathlib import Path
import selectors
import signal
import socket
import statistics
import subprocess
import sys
import time


MODEL = "claude-opus-5-5"
WORKFLOW_BRANCH = "scsh-workflow"
COMMIT_NAME = "dkorolev-neon-elon-bot"
COMMIT_EMAIL = "dmitry.korolev+elon-presley@gmail.com"
ARMS = ("host-print", "container-print", "container-interactive",
        "scsh-interactive", "scsh-headless", "scsh-compact")
HOOK = '''import json, os, pathlib, sys
event = json.load(sys.stdin)
if os.environ.get("PARITY_TASK") != "one-turn":
    sys.exit(0)
text = event.get("last_assistant_message", "")
result = {"answer": text.strip()}
path = pathlib.Path(os.environ["SCSH_RESULT"])
path.parent.mkdir(parents=True, exist_ok=True)
temporary = path.with_suffix(".part")
temporary.write_text(json.dumps(result) + "\\n")
temporary.replace(path)
'''


def write(path, value):
    path.parent.mkdir(parents=True, exist_ok=True)
    data = value if isinstance(value, str) else json.dumps(value, indent=2) + "\n"
    temporary = path.with_suffix(path.suffix + ".part")
    temporary.write_text(data)
    temporary.replace(path)


def checked(command, **kwargs):
    result = subprocess.run(command, text=True, capture_output=True, timeout=30, **kwargs)
    if result.returncode:
        # Never print an environment or a container command that might contain a credential.
        raise RuntimeError(f"{command[0]} failed with exit {result.returncode}: {result.stderr}")
    return result.stdout


def definition(prompt, mode, contract):
    return ("description: Controlled Claude prompt comparison\n"
            "params:\n  PARITY_TASK:\n    type: string\n    default: one-turn\n"
            "steps:\n  solve:\n    agent:\n      harness: claude\n"
            f"      model: {MODEL}\n      effort: medium\n"
            f"      claude_mode: {mode}\n      prompt_contract: {contract}\n"
            "    inactivity_timeout: 120\n    prompt: |\n" +
            "".join("      " + line + "\n" for line in prompt.splitlines()) +
            "    inputs:\n      PARITY_TASK: params.PARITY_TASK\n"
            "    output:\n      answer:\n        type: string\n")


def prepare(root, binary, samples):
    if root.exists():
        raise RuntimeError("The output directory already exists; choose a fresh directory.")
    root.mkdir(parents=True, mode=0o700)
    source = root / "fixture"
    source.mkdir()
    write(source / ".gitignore", "/tmp\n")
    write(source / "capture.py", HOOK)
    write(source / "graph.json", {"app": ["api", "ui"], "api": ["core"], "ui": ["core"],
                                 "core": [], "unrelated": []})
    write(source / ".mcp.json", {"mcpServers": {}})
    write(source / ".claude/settings.json", {
        "effortLevel": "medium", "enableAllProjectMcpServers": True,
        "env": {"DISABLE_PROMPT_CACHING": "1", "DISABLE_AUTOUPDATER": "1"},
        "hooks": {"Stop": [{"hooks": [{"type": "command", "command": "python3 capture.py"}]}]},
    })
    plan = []
    for workload in ("one-turn", "repository"):
        for repeat in range(samples):
            nonce = hashlib.sha256(f"{root.name}:{workload}:{repeat}".encode()).hexdigest()
            task = ("Reply exactly PARITY_OK. Do not call tools. "
                    "A Stop hook serializes your final response to $SCSH_RESULT. "
                    "Do not write that file yourself." if workload == "one-turn" else
                    "Read graph.json. Compute the transitive dependencies of app, excluding app itself. "
                    "Write a JSON object to $SCSH_RESULT whose answer is the sorted dependency names "
                    "joined by commas. The Stop hook does not write the result for this task.")
            prompt = (f"Task nonce {nonce}.\n{task}\n"
                      "Do not change or commit project files; only the requested result under tmp/ may be written.")
            for mode in ("interactive", "headless"):
                for contract in ("verbatim", "standard", "compact"):
                    name = f"parity_{workload.replace('-', '_')}_{repeat}_{mode}_{contract}"
                    write(source / ".harness" / f"{name}.yml", definition(prompt, mode, contract))
            # Rotate the controls deterministically while keeping all calls sequential.
            order = ARMS[repeat % len(ARMS):] + ARMS[:repeat % len(ARMS)]
            plan.extend({"workload": workload, "repeat": repeat, "arm": arm} for arm in order)
    # Claude includes Git context in its input. Match the branch scsh uses for workflow clones.
    checked(["git", "init", "-q", "-b", WORKFLOW_BRANCH, str(source)])
    checked(["git", "add", "."], cwd=source)
    checked(["git", "-c", "user.name=Claude Parity", "-c", "user.email=parity@localhost",
             "commit", "-qm", "Pin the controlled Claude fixture."], cwd=source)
    write(root / "plan.json", {"samples": samples, "model": MODEL, "effort": "medium", "schedule": plan,
          "binary_sha256": hashlib.sha256(binary.read_bytes()).hexdigest(),
          "runner_sha256": hashlib.sha256(Path(__file__).read_bytes()).hexdigest(),
          "fixture_protocol": 2, "git_branch": WORKFLOW_BRANCH,
          "fixture_revision": checked(["git", "rev-parse", "HEAD"], cwd=source).strip(),
          "nonce_policy": "Identical within matched controls; different for each repetition. Cache disabled in settings."})
    return source, plan


def seed_config(work, host=False):
    config = work / "tmp/.claude-auth/.claude"
    config.mkdir(parents=True)
    identity = {}
    host_json = Path.home() / ".claude.json"
    if host_json.exists():
        values = json.loads(host_json.read_text())
        identity = {key: values[key] for key in ("oauthAccount", "userID") if key in values}
    cwd = str(work) if host else "/home/agent/repo"
    identity.update(autoUpdates=False, hasCompletedOnboarding=True, bypassPermissionsModeAccepted=True,
                    projects={cwd: {"hasTrustDialogAccepted": True, "hasCompletedProjectOnboarding": True,
                                    "bypassPermissionsModeAccepted": True}})
    write(config / ".claude.json", identity)
    script = config / "scsh-quota-statusline.sh"
    write(script, '#!/bin/sh\nd=$(dirname "$0")\ncat > "$d/scsh-quota.json.part" && mv "$d/scsh-quota.json.part" "$d/scsh-quota.json"\nprintf scsh\n')
    script.chmod(0o700)
    script_path = str(script) if host else "/home/agent/repo/tmp/.claude-auth/.claude/scsh-quota-statusline.sh"
    write(config / "settings.json", {"statusLine": {"type": "command", "command": script_path,
                                                  "refreshInterval": 5}, "autoContinueAtUsageLimit": True})
    return config


def terminal_native(config):
    paths = list(config.glob("projects/**/*.jsonl"))
    if not paths:
        return False
    for path in paths:
        finished = False
        for line in path.read_text().splitlines():
            try:
                event = json.loads(line)
            except ValueError:
                return False
            if event.get("type") == "user":
                finished = False
            if event.get("type") == "assistant":
                finished = event.get("message", {}).get("stop_reason") == "end_turn"
            if event.get("type") == "system" and event.get("subtype") == "turn_duration":
                finished = True
        if not finished:
            return False
    return True


def supervise(command, cwd, env, logfile, on_tick=None):
    start = last_progress = time.monotonic()
    process = subprocess.Popen(command, cwd=cwd, env=env, stdout=subprocess.PIPE,
                               stderr=subprocess.STDOUT, start_new_session=True)
    selector = selectors.DefaultSelector()
    selector.register(process.stdout, selectors.EVENT_READ)
    try:
        with logfile.open("wb") as log:
            while selector.get_map():
                for key, _ in selector.select(timeout=0.2):
                    data = os.read(key.fd, 65536)
                    if data:
                        log.write(data)
                        log.flush()
                        sys.stdout.buffer.write(data)
                        sys.stdout.buffer.flush()
                    else:
                        selector.unregister(key.fileobj)
                now = time.monotonic()
                if on_tick:
                    on_tick()
                if now - last_progress >= 10:
                    print(f"  running {int(now-start)}s; live output: {logfile}", flush=True)
                    last_progress = now
                if now - start > 600:
                    raise TimeoutError("600-second sample deadline")
            return process.wait(timeout=10), time.monotonic() - start
    finally:
        selector.close()
        if process.poll() is None:
            os.killpg(process.pid, signal.SIGTERM)
            try:
                process.wait(timeout=5)
            except subprocess.TimeoutExpired:
                os.killpg(process.pid, signal.SIGKILL)
                process.wait(timeout=5)


def sample(root, source, binary, image, spec, env):
    setup_started = time.monotonic()
    arm, workload, repeat = (spec[key] for key in ("arm", "workload", "repeat"))
    out = root / "runs" / f"{workload}-{repeat}-{arm}"
    out.mkdir(parents=True)
    work = out / "repo"
    checked(["git", "clone", "-q", "--no-hardlinks", str(source), str(work)])
    checked(["git", "config", "user.name", COMMIT_NAME], cwd=work)
    checked(["git", "config", "user.email", COMMIT_EMAIL], cwd=work)
    mode = "interactive" if arm.endswith("interactive") else "headless"
    contract = "verbatim" if workload == "one-turn" else "standard"
    if arm == "scsh-compact":
        contract = "compact"
    name = f"parity_{workload.replace('-', '_')}_{repeat}_{mode}_{contract}"
    manifest = json.loads(checked([str(binary), "inspect-prompt", "--def", name], cwd=work, env=env))
    invocation = manifest["InvocationInspection"][0]["InvocationManifest"]
    write(out / "invocation.json", manifest)
    current_env = dict(env, PARITY_TASK=workload)
    state = Path(env["SCSH_HOME"])
    before = set(state.glob("sessions/*/logs/*.usage-ledger.json"))
    container = "scsh-parity-" + hashlib.sha256(str(out).encode()).hexdigest()[:12]
    result_path = work / "tmp/inspection/solve.json"
    on_tick = None
    if arm.startswith("scsh-"):
        command = [str(binary), "run", "--def", name, "--retries", "0"]
    else:
        config = seed_config(work, host=arm == "host-print")
        result_path.parent.mkdir(parents=True)
        current_env.update(SCSH_RESULT="tmp/inspection/solve.json", DISABLE_PROMPT_CACHING="1")
        if arm == "host-print":
            current_env["CLAUDE_CONFIG_DIR"] = str(config)
            command = invocation["argument_vector"]
        else:
            command = ["podman", "run", "--rm", "--name", container, "--userns=keep-id",
                       "--tmpfs", "/tmp:rw,nosuid,nodev,size=256m,mode=1777",
                       "-v", f"{work}:/home/agent/repo"]
            for key in ("CLAUDE_CODE_OAUTH_TOKEN", "SCSH_RESULT", "PARITY_TASK", "DISABLE_PROMPT_CACHING"):
                command.extend(["-e", key])
            command.extend([image, "/bin/sh", "-c", invocation["command"]])
            if mode == "interactive":
                def finish():
                    if result_path.exists() and terminal_native(config):
                        write(work / "tmp/scsh-run.log.shutdown", "exit")
                on_tick = finish
    row = dict(spec, requested_model=MODEL, prompt_sha256=invocation["submitted_prompt"]["sha256"],
               image=image, started_at=time.time(), setup_seconds=time.monotonic() - setup_started,
               model_execution_seconds=None)
    try:
        row["exit_code"], row["wall_seconds"] = supervise(command, work, current_env, out / "output.log", on_tick)
    except TimeoutError as error:
        row.update(exit_code=None, error=str(error), wall_seconds=600)
    finally:
        if not arm.startswith("scsh-") and arm != "host-print":
            exists = subprocess.run(["podman", "container", "exists", container], timeout=15)
            if exists.returncode == 0:
                checked(["podman", "rm", "-f", container])
    if arm.startswith("scsh-"):
        ledgers = sorted(set(state.glob("sessions/*/logs/*.usage-ledger.json")) - before)
        row["attempts"] = len(ledgers)
        if len(ledgers) == 1:
            ledger = json.loads(ledgers[0].read_text())["ClaudeUsageLedger"]
            write(out / "usage-ledger.json", {"ClaudeUsageLedger": ledger})
            manifest_path = ledgers[0].with_name(ledgers[0].name.replace(".usage-ledger.json", ".invocation.json"))
            observed = json.loads(manifest_path.read_text())["InvocationManifest"]
            write(out / "observed-invocation.json", {"InvocationManifest": observed})
            row["image_matches"] = observed.get("image_id") == image
            row["prompt_matches"] = observed["submitted_prompt"]["sha256"] == row["prompt_sha256"]
            result_path = ledgers[0].parent.parent / "results/solve.json"
        else:
            ledger = {}
    else:
        inspected = subprocess.run([str(binary), "inspect-claude-usage", str(config / "projects")],
                                   text=True, capture_output=True, timeout=30)
        write(out / "usage-ledger.json", inspected.stdout)
        ledger = json.loads(inspected.stdout).get("ClaudeUsageLedger", {})
        row["attempts"] = 1
        row.update(image_matches=True, prompt_matches=True)
    aggregate = ledger.get("aggregate") or {}
    usage = aggregate.get("TokenUsage", {})
    row.update(usage=usage, complete=usage.get("complete", False))
    tokens = usage.get("tokens")
    row["total_tokens"] = sum(tokens.values()) if tokens and all(v is not None for v in tokens.values()) else None
    row["mcp_calls"] = len({tool["id"] for response in ledger.get("responses", [])
                            for tool in response["tools"] if tool.get("mcp")})
    row["observed_models"] = sorted({r["model"] for r in ledger.get("responses", []) if r["model"]})
    expected = {"answer": "PARITY_OK" if workload == "one-turn" else "api,core,ui"}
    try:
        actual = json.loads(result_path.read_text())
    except (OSError, ValueError):
        actual = None
    row["correct"] = actual == expected
    row["zero_cache"] = bool(tokens and tokens["cache_read"] == tokens["cache_write"] == 0)
    row["eligible"] = (row["correct"] and row["complete"] and row["zero_cache"] and row["exit_code"] == 0
                       and row.get("image_matches", False) and row.get("prompt_matches", False)
                       and row["observed_models"] == [MODEL] and (workload != "one-turn" or
                       (usage.get("tool_calls") == 0 and usage.get("llm_round_trips") == 1)))
    write(out / "result.json", actual)
    write(out / "summary.json", row)
    print(json.dumps(row), flush=True)
    return row


def report(root, rows):
    comparisons = []
    for workload in ("one-turn", "repository"):
        for left, right in [*zip(ARMS, ARMS[1:]), ("container-print", "scsh-headless")]:
            pairs = []
            for a in rows:
                if a["workload"] != workload or a["arm"] != left:
                    continue
                b = next((b for b in rows if b["workload"] == workload and b["repeat"] == a["repeat"] and b["arm"] == right), None)
                matching_prompt = b and (right == "scsh-compact" or a["prompt_sha256"] == b["prompt_sha256"])
                if matching_prompt and all(r["eligible"] for r in (a, b)):
                    pairs.append(b["total_tokens"] - a["total_tokens"])
            comparisons.append({"workload": workload, "left": left, "right": right, "paired_differences": pairs,
                                "median": statistics.median(pairs) if pairs else None,
                                "range": [min(pairs), max(pairs)] if pairs else None})
    write(root / "comparison.json", {"samples": rows, "comparisons": comparisons,
          "limits": ["Host environment intentionally differs from container controls.",
                     "The one-turn fixture uses a deterministic Stop hook to serialize the answer without an LLM tool call.",
                     "One-turn A-E use verbatim prompts; F adds a compact contract. Repository E-F compares standard with compact.",
                     "No memory savings claim is tested here."]})


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--auth-file", type=Path)
    parser.add_argument("--samples", type=int, default=5)
    parser.add_argument("--execute", action="store_true", help="Run models; default only prepares pinned fixtures")
    args = parser.parse_args()
    if socket.gethostname() != "mcpclaude" or os.getuid() == 0 or not os.environ.get("TMUX"):
        parser.error("Run as ubuntu@mcpclaude under tmux.")
    os.umask(0o077)
    binary = args.binary.resolve()
    root = args.output.resolve()
    source, plan = prepare(root, binary, args.samples)
    if not args.execute:
        print(f"Prepared {len(plan)} sequential samples in {root}; no model calls.")
        return
    if not args.auth_file:
        parser.error("--execute requires --auth-file containing access_token and expires_at")
    auth = json.loads(args.auth_file.read_text())
    if auth["expires_at"] < time.time() + 3600:
        parser.error("Credential must remain valid for at least one hour; supply a refreshed credential file.")
    if checked(["podman", "ps", "-q"]).strip():
        parser.error("A container is already running; refuse overlapping workloads.")
    image = checked(["podman", "image", "inspect", "--format", "{{.Id}}", "scsh-claude:latest"]).strip()
    with socket.socket() as sock:
        sock.bind(("127.0.0.1", 0))
        port = sock.getsockname()[1]
    env = dict(os.environ, CLAUDE_CODE_OAUTH_TOKEN=auth["access_token"], SCSH_RUNTIME="podman",
               SCSH_HOME=str(root / "state"), SCSH_DAEMON_PORT=str(port), SCSH_NO_RETRY="1",
               SCSH_NO_GH_AUTH="1", SCSH_NO_CODEX_AUTH="1", SCSH_NO_CURSOR_AUTH="1", SCSH_NO_OPENCODE_AUTH="1")
    # Automatic cast annotation would be an unrelated model workload in this experiment.
    env["CODEX_HOME"] = str(root / "unused-codex-home")
    env.pop("OPENAI_API_KEY", None)
    rows = []
    try:
        for spec in plan:
            print(f"Starting {spec}", flush=True)
            row = sample(root, source, binary, image, spec, env)
            rows.append(row)
            report(root, rows)
            if not row["complete"] or not row["zero_cache"]:
                raise RuntimeError("Usage unavailable or cache policy violated; sample retained, campaign stopped.")
    finally:
        subprocess.run([str(binary), "daemon", "stop"], env=env, timeout=30)


if __name__ == "__main__":
    main()
