"""Interactive smoke tests using only Python's standard library (Linux/macOS).

Run after cargo build -p semantic-cli:
    python3 apps/semantic-cli/tests/repl_pty.py
"""
import errno
import fcntl
import os
from pathlib import Path
import pty
import re
import select
import signal
import struct
import tempfile
import termios
import time
import unittest

ROOT = Path(__file__).resolve().parents[3]
BINARY = Path(os.environ.get("SEMANTIC_DB_TEST_BINARY", ROOT / "target/debug/semantic-db"))
CSV = ROOT / "examples/geospatial/wells.csv"
SGR = re.compile(rb"\x1b\[[0-9;]*m")


class Session:
    def __init__(self, directory, flags=(), env=None):
        self.pid, self.fd = pty.fork()
        if self.pid == 0:
            os.chdir(directory)
            environment = dict(os.environ, TERM="xterm-256color")
            environment.pop("NO_COLOR", None)
            environment.pop("OPENAI_API_KEY", None)
            environment.update(env or {})
            os.execve(BINARY, [str(BINARY), "--csv", f"wells={CSV}", *flags], environment)
        self.dumb = (env or {}).get("TERM", "").lower() in {"dumb", "cons25", "emacs"}
        self.pending = b""
        self.transcript = b""
        self.resize(24, 100)
        self.expect(b"semantic> ")

    def send(self, text):
        os.write(self.fd, text.encode() if isinstance(text, str) else text)

    def resize(self, rows, columns):
        fcntl.ioctl(self.fd, termios.TIOCSWINSZ, struct.pack("HHHH", rows, columns, 0, 0))

    def expect(self, marker, timeout=10):
        deadline = time.monotonic() + timeout
        while marker not in self.pending:
            if time.monotonic() >= deadline:
                raise AssertionError(f"Timed out waiting for {marker!r}: {self.pending[-3000:]!r}")
            if select.select([self.fd], [], [], 0.1)[0]:
                try:
                    data = os.read(self.fd, 65536)
                except OSError as error:
                    if error.errno == errno.EIO:
                        data = b""
                    else:
                        raise
                if not data:
                    raise AssertionError(f"REPL exited before {marker!r}: {self.pending[-3000:]!r}")
                self.transcript += data
                self.pending += data
                if b"\x1b[6n" in data:
                    self.send(b"\x1b[1;1R")
        end = self.pending.index(marker) + len(marker)
        result, self.pending = self.pending[:end], self.pending[end:]
        return result

    def executed(self, input, marker=b"1 row(s)"):
        self.send(input)
        output = self.expect(marker)
        self.expect(b"semantic> ")
        return output

    def close(self):
        if self.dumb:
            self.send(".quit\r")
        else:
            self.send(b"\x03")
            self.expect(b"semantic> ")
            self.send(b"\x04")
        deadline = time.monotonic() + 5
        while time.monotonic() < deadline:
            pid, status = os.waitpid(self.pid, os.WNOHANG)
            if pid:
                os.close(self.fd)
                assert os.waitstatus_to_exitcode(status) == 0, status
                return
            if select.select([self.fd], [], [], 0.02)[0]:
                try:
                    data = os.read(self.fd, 65536)
                except OSError:
                    data = b""
                self.transcript += data
                if b"\x1b[6n" in data:
                    self.send(b"\x1b[1;1R")
        os.kill(self.pid, signal.SIGKILL)
        os.waitpid(self.pid, 0)
        os.close(self.fd)
        raise AssertionError(f"Ctrl-D did not exit the empty editor: {self.transcript[-1500:]!r}")


class ReplTests(unittest.TestCase):
    def test_editing_completion_search_and_history(self):
        with tempfile.TemporaryDirectory(prefix="semantic-pty-") as directory:
            history = str(Path(directory) / "history")
            session = Session(directory, ["--history-file", history])
            try:
                session.executed(".sch\t wells\r", b"well_id: Utf8")
                query = "SELECT w.well_i FROM wells w LIMIT 1;"
                session.send(query + "\x01" + "\x1b[C" * len("SELECT w.well_i") + "\t\x05\r")
                self.assertIn(b"W-001", session.expect(b"1 row(s)"))
                session.expect(b"semantic> ")
                # All lines remain in one editable buffer: replace the first digit.
                session.send("SELECT 41\r+1;")
                session.send("\x01\x1b[A\x01" + "\x1b[C" * 7 + "\x04" + "5\x05\r")
                self.assertIn(b"52", session.expect(b"1 row(s)"))
                session.expect(b"semantic> ")
                # Recall/reexecute the whole multiline statement.
                self.assertIn(b"52", session.executed("\x1b[A\r"))
                session.executed("SELECT 987 AS history_marker;\r")
                # Reverse search accepts the match with Enter and executes it.
                self.assertIn(b"987", session.executed("\x12history_marker\r"))
                # Cancel incomplete input, resize, then execute normally.
                session.send("SELECT 'unfinished\x03")
                session.expect(b"semantic> ")
                session.resize(14, 50)
                self.assertIn(b"123", session.executed("SELECT 123; -- trailing comment\r"))
                session.executed("SELECT missing;\r", b"Failed after")
                session.executed(".view added=SELECT 77 AS answer\r", b"Registered view")
                self.assertIn(b"77", session.executed("SELECT a.ans FROM added a;\x01" + "\x1b[C" * len("SELECT a.ans") + "\t\x05\r"))
                self.assertTrue(SGR.search(session.transcript))
            finally:
                session.close()
            stored = Path(history).read_text()
            self.assertIn("history_marker", stored)
            self.assertNotIn("unfinished", stored)
            restarted = Session(directory, ["--history-file", history])
            try:
                self.assertIn(b"987", restarted.executed("\x12history_marker\r"))
            finally:
                restarted.close()

    def test_color_controls_and_disabled_history(self):
        for flags, environment in [(["--no-color"], {}), ([], {"NO_COLOR": "1"}), ([], {"TERM": "dumb"}), ([], {"TERM": "emacs"})]:
            with self.subTest(flags=flags, env=environment), tempfile.TemporaryDirectory(prefix="semantic-pty-") as directory:
                session = Session(directory, ["--no-history", *flags], environment)
                try:
                    session.executed("SELECT 42;\r")
                    self.assertFalse(SGR.search(session.transcript), session.transcript)
                    self.assertEqual(list(Path(directory).iterdir()), [])
                finally:
                    session.close()


if __name__ == "__main__":
    unittest.main()
