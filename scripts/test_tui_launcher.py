"""Exercise the shipped recorder with real tmux/asciinema and no model calls."""
import json
import os
from pathlib import Path
import shlex
import shutil
import subprocess
import sys
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]
TMUX = shutil.which("tmux")
ASCIINEMA = shutil.which("asciinema")


def recorder_source():
  source = (ROOT / "src/Dockerfile").read_text()
  start = source.index("RUN printf '%s\\n'")
  end = source.index("  > /usr/local/bin/scsh-tui-record", start)
  words = shlex.split(source[start:end].replace("\\\n", ""))
  assert words[:3] == ["RUN", "printf", "%s\\n"]
  return "\n".join(words[3:]) + "\n"


@unittest.skipUnless(TMUX and ASCIINEMA, "requires local tmux and asciinema; no model credentials needed")
class TuiLauncherTest(unittest.TestCase):
  def setUp(self):
    self.scratch = tempfile.TemporaryDirectory(prefix="scsh-tui-test-")
    self.addCleanup(self.scratch.cleanup)
    self.root = Path(self.scratch.name)
    self.socket = str(self.root / "socket")
    self.addCleanup(self.stop_server)
    bindir = self.root / "bin"
    bindir.mkdir()
    self.tmux = bindir / "tmux"
    self.tmux.write_text(
      "#!/bin/sh\n"
      'if [ "$1" = -f ]; then for last; do :; done; printf "%s\\n" "$last" > '
      + shlex.quote(str(self.root / "pane-command")) + "; fi\n"
      + "exec " + shlex.join([TMUX, "-S", self.socket]) + ' "$@"\n'
    )
    self.tmux.chmod(0o755)
    self.script = self.root / "recorder"
    self.script.write_text(recorder_source())
    self.result = self.root / "result.json"
    self.fake = self.root / "fake.py"
    self.fake.write_text(
      "import json,sys,time\nfrom pathlib import Path\n"
      "Path(sys.argv[1]).write_text(json.dumps(sys.argv[2]))\n"
      'print("Harness received prompt", flush=True)\ntime.sleep(1)\n'
    )
    self.env = dict(os.environ, PATH=str(bindir) + os.pathsep + os.environ["PATH"],
                    SCSH_RUN_LOG=str(self.root / "run.log"), TERM="xterm-256color")

  def stop_server(self):
    subprocess.run([TMUX, "-S", self.socket, "kill-server"], capture_output=True, timeout=5)

  def launch(self, prompt):
    command = shlex.join([sys.executable, str(self.fake), str(self.result), prompt])
    return subprocess.run(
      ["sh", str(self.script), "200", "50", "slash-exit", "none", str(self.result), command],
      env=self.env, capture_output=True, text=True, timeout=15,
    )

  def assert_prompt_survives(self, prompt):
    run = self.launch(prompt)
    self.assertEqual(run.returncode, 0, run.stderr)
    self.assertEqual(json.loads(self.result.read_text()), prompt)
    self.assertIn("tmux new-session rc=0", (self.root / "run.log.tuidebug").read_text())
    self.assertTrue((self.root / "run.log.cast").stat().st_size > 0)
    pane_command = (self.root / "pane-command").read_text().strip()
    self.assertLess(len(pane_command), 256)
    pane_path = Path(shlex.split(pane_command)[1])
    self.assertFalse(pane_path.exists(), "temporary launch script must be removed")

  def test_short_prompt(self):
    self.assert_prompt_survives("Review this code.")

  def test_long_prompt_preserves_quotes_newlines_and_literal_shell_syntax(self):
    prompt = 'Quotes: "double", \'single\'; $HOME; $(echo literal); `literal`; é\n' * 800
    self.assertGreater(len(prompt.encode()), 40_000)
    self.assert_prompt_survives(prompt)

  def test_tmux_failure_is_not_reported_as_success(self):
    self.tmux.write_text('#!/bin/sh\necho "simulated tmux startup failure" >&2\nexit 71\n')
    run = self.launch("not executed")
    self.assertEqual(run.returncode, 71)
    self.assertFalse(self.result.exists())
    self.assertIn("tmux new-session rc=71", (self.root / "run.log.tuidebug").read_text())

  def test_recorder_failure_stops_its_tmux_session(self):
    asciinema = self.tmux.parent / "asciinema"
    asciinema.write_text('#!/bin/sh\necho "simulated recording failure" >&2\nexit 72\n')
    asciinema.chmod(0o755)
    run = self.launch("recording cannot start")
    self.assertEqual(run.returncode, 72)
    check = subprocess.run([TMUX, "-S", self.socket, "has-session"], capture_output=True, timeout=5)
    self.assertNotEqual(check.returncode, 0)


if __name__ == "__main__":
  unittest.main()
