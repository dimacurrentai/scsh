"""Deterministic fixture, result serialization, and report tests; never call a model."""

import importlib.util
import json
import os
from pathlib import Path
import subprocess
import tempfile
import unittest


SPEC = importlib.util.spec_from_file_location("claude_parity", Path(__file__).with_name("claude-parity.py"))
PARITY = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(PARITY)


class ClaudeParityTests(unittest.TestCase):
    def test_fixture_is_committed_and_all_definitions_inspect_without_models(self):
        binary = Path(__file__).resolve().parents[1] / "target/debug/scsh"
        if not binary.exists():
            self.skipTest("Build the debug binary to verify CLI inspection of every fixture.")
        with tempfile.TemporaryDirectory(prefix="scsh-parity-fixture-") as tmp:
            source, plan = PARITY.prepare(Path(tmp) / "campaign", binary, 1)
            self.assertEqual(len(plan), 12)
            self.assertEqual(PARITY.checked(["git", "status", "--porcelain"], cwd=source), "")
            self.assertEqual(PARITY.checked(["git", "branch", "--show-current"], cwd=source).strip(), PARITY.WORKFLOW_BRANCH)
            for definition in sorted((source / ".harness").glob("*.yml")):
                value = json.loads(PARITY.checked([str(binary), "inspect-prompt", "--def", definition.stem], cwd=source))
                manifest = value["InvocationInspection"][0]["InvocationManifest"]
                self.assertEqual(manifest["requested_model"], PARITY.MODEL)
                self.assertEqual(manifest["effort"], "medium")
                self.assertEqual(manifest["argument_vector"][-1], manifest["submitted_prompt"]["text"])

    def test_stop_hook_serializes_only_the_final_answer(self):
        with tempfile.TemporaryDirectory(prefix="scsh-parity-hook-") as tmp:
            result = Path(tmp) / "result.json"
            for workload, message, expected in [
                ("one-turn", "PARITY_OK\n", {"answer": "PARITY_OK"}),
            ]:
                subprocess.run(["python3", "-c", PARITY.HOOK], input=json.dumps({"last_assistant_message": message}),
                               text=True, env=dict(os.environ, SCSH_RESULT=str(result), PARITY_TASK=workload),
                               check=True, timeout=5)
                self.assertEqual(json.loads(result.read_text()), expected)
                self.assertFalse(result.with_suffix(".part").exists())

    def test_repository_hook_neither_creates_nor_overwrites_the_required_result(self):
        with tempfile.TemporaryDirectory(prefix="scsh-parity-hook-") as tmp:
            result = Path(tmp) / "result.json"
            def stop():
                subprocess.run(["python3", "-c", PARITY.HOOK],
                               input=json.dumps({"last_assistant_message": "A prose summary with no JSON."}),
                               text=True, env=dict(os.environ, SCSH_RESULT=str(result), PARITY_TASK="repository"),
                               check=True, timeout=5)
            stop()
            self.assertFalse(result.exists(), "a missing model-written result must remain a failure")
            expected = '{"answer":"api,core,ui"}\n'
            result.write_text(expected)
            stop()
            self.assertEqual(result.read_text(), expected)

    def test_report_retains_failures_and_only_pairs_matching_prompts(self):
        with tempfile.TemporaryDirectory(prefix="scsh-parity-report-") as tmp:
            rows = [dict(workload="one-turn", repeat=0, arm="container-print", eligible=True,
                         total_tokens=100, prompt_sha256="same"),
                    dict(workload="one-turn", repeat=0, arm="scsh-headless", eligible=True,
                         total_tokens=103, prompt_sha256="same"),
                    dict(workload="one-turn", repeat=1, arm="scsh-headless", eligible=False,
                         total_tokens=None, prompt_sha256="failed")]
            PARITY.report(Path(tmp), rows)
            report = json.loads((Path(tmp) / "comparison.json").read_text())
            self.assertEqual(len(report["samples"]), 3)
            comparison = next(c for c in report["comparisons"] if c["workload"] == "one-turn"
                              and c["left"] == "container-print" and c["right"] == "scsh-headless")
            self.assertEqual(comparison["paired_differences"], [3])
            rows[1]["prompt_sha256"] = "different"
            PARITY.report(Path(tmp), rows)
            report = json.loads((Path(tmp) / "comparison.json").read_text())
            self.assertFalse(any(c["paired_differences"] for c in report["comparisons"]))


if __name__ == "__main__":
    unittest.main()
