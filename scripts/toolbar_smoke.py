#!/usr/bin/env python3
"""Exercise the function-key toolbar through a native isolated daemon: defaults, runtime
toggles, localized and ASCII labels, pane geometry, clicks and F-keys from a real attached
client, custom and invalid `[[toolbar_buttons]]`."""
import json
import os
from pathlib import Path
import socket
import struct
import subprocess
import sys
import tempfile
import threading
import time
import uuid

from smoke_temp import SmokeDirectory


def first_line(frame):
    return frame.splitlines()[0]


class AttachedClient:
    """Speak Crew's client protocol (u32-LE length + JSON) on the daemon's local socket, like
    `crew attach` does, so mouse and key events reach the toolbar handlers for real."""

    def __init__(self, socket_name):
        # src/ipc.rs: named pipe on Windows, abstract socket on Linux, temp-dir path elsewhere
        if os.name == "nt":
            pipe = open("//./pipe/" + socket_name, "r+b", buffering=0)
            self.read, self.write = pipe.read, pipe.write
            self.close = pipe.close
        else:
            sock = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
            if sys.platform.startswith("linux"):
                sock.connect(chr(0) + socket_name)  # abstract namespace
            else:
                sock.connect(os.path.join(tempfile.gettempdir(), socket_name))
            stream = sock.makefile("rwb", buffering=0)
            self.read, self.write = stream.read, stream.write
            self.close = lambda: (stream.close(), sock.close())
        self.output = []
        self.lock = threading.Lock()

    def send(self, message):
        data = json.dumps(message).encode()
        self.write(struct.pack("<I", len(data)) + data)

    def start(self):
        def pump():
            try:
                while True:
                    header = self.read(4)
                    if len(header) < 4:
                        return
                    (size,) = struct.unpack("<I", header)
                    body = b""
                    while len(body) < size:
                        chunk = self.read(size - len(body))
                        if not chunk:
                            return
                        body += chunk
                    message = json.loads(body)
                    with self.lock:
                        self.output.append(message.get("Output", ""))
            except (OSError, ValueError):
                return
        threading.Thread(target=pump, daemon=True).start()

    def mark(self):
        with self.lock:
            return len(self.output)

    def since(self, mark):
        with self.lock:
            return "".join(self.output[mark:])

    def click(self, column, row):
        for kind in ({"Down": "Left"}, {"Up": "Left"}):
            self.send({"Event": {"Mouse": {"kind": kind, "column": column, "row": row, "modifiers": ""}}})

    def key(self, code):
        self.send({"Event": {"Key": {"code": code, "modifiers": "", "kind": "Press", "state": ""}}})


def wait_for(predicate, what, timeout=10):
    deadline = time.monotonic() + timeout
    while not predicate():
        assert time.monotonic() < deadline, "timed out waiting for " + what
        time.sleep(0.05)


def run_smoke(binary):
    binary = str(Path(binary).resolve())
    with SmokeDirectory(prefix="crew-toolbar-") as temporary:
        root = Path(temporary)
        base = (
            'persist = false\nautomatic_rename = false\nhints = false\nlang = "en"\n'
            'shell = ' + json.dumps("powershell.exe -NoProfile" if os.name == "nt" else "/bin/sh") + "\n"
        )
        (root / "config.toml").write_text(base, encoding="utf-8")
        custom = root / "custom.toml"
        custom.write_text(
            base + '[[toolbar_buttons]]\nlabel = "build"\ncommand = "display built"\n'
            '[[toolbar_buttons]]\nlabel = "關閉"\ncommand = "kill-pane"\n',
            encoding="utf-8",
        )
        many = root / "too-many.toml"
        many.write_text(
            base + "".join(f'[[toolbar_buttons]]\nlabel = "b{i}"\ncommand = "display x"\n' for i in range(13)),
            encoding="utf-8",
        )
        bad_command = root / "bad-command.toml"
        bad_command.write_text(
            base + '[[toolbar_buttons]]\nlabel = "x"\ncommand = "no-such-command"\n',
            encoding="utf-8",
        )
        log = root / "server.log"
        env = {key: value for key, value in os.environ.items()
               if not key.upper().startswith(("CREW", "TMUXLIKE"))}
        socket_name = "crew-toolbar-" + uuid.uuid4().hex + ".sock"
        env.update(CREW_SOCKET=socket_name,
                   CREW_CONFIG=str(root / "config.toml"), CREW_STATE=str(root / "state.json"),
                   CREW_LOG=str(log), CREW_LANG="en")

        def proc(*args):
            return subprocess.run([binary, *args], env=env, cwd=root, capture_output=True,
                                  text=True, encoding="utf-8", errors="strict", timeout=30)

        def run(*args, code=0):
            result = proc(*args)
            assert result.returncode == code, (args, result.returncode, result.stdout, result.stderr)
            return result.stdout

        def api(*args):
            return json.loads(run(*args))["result"]

        def window_count():
            snapshot = api("api", "snapshot")
            return len(next(s for s in snapshot["sessions"] if s["name"] == "toolbar")["windows"])

        def session_rows():
            snapshot = api("api", "snapshot")
            return next(s for s in snapshot["sessions"] if s["name"] == "toolbar")["rows"]

        try:
            workspace = api("workspace", "create", "toolbar", "--cwd", str(root))["workspace"]
            pane = workspace["tabs"][0]["active_pane"]
            if os.name == "nt":
                assert api("pane", "wait-output", pane, "--regex", r"PS [^\r\n]*>",
                           "--timeout", "15000")["matched"]

            # 1. Defaults: toolbar on, English labels, status bar kept as the last row, pane = rows-2.
            frame = run("debug-frame", "-t", "toolbar", "--text")
            top = first_line(frame)
            assert top.startswith("F1 help") and "F2 new" in top, top
            lines = frame.splitlines()
            assert "F1" not in lines[-1] and "0:" in lines[-1], lines[-1]
            rows = session_rows()
            assert len(lines) == rows, (len(lines), rows)
            pane_rows = api("pane", "get", pane)["pane"]["rows"]
            assert pane_rows == rows - 2, (pane_rows, rows)
            print("ok default toolbar row on top with F1 help F2 new, status bar last, pane rows = rows-2",
                  flush=True)

            # 2. Runtime toggle removes and restores the row; pane height follows (rows-1 when off).
            run("set", "toolbar", "off")
            off = run("debug-frame", "-t", "toolbar", "--text")
            assert "F1 help" not in first_line(off), first_line(off)
            assert api("pane", "get", pane)["pane"]["rows"] == rows - 1
            run("set", "toolbar", "on")
            assert first_line(run("debug-frame", "-t", "toolbar", "--text")).startswith("F1 help")
            assert api("pane", "get", pane)["pane"]["rows"] == rows - 2
            print("ok set toolbar off hides the row (pane rows-1); set toolbar on restores it (rows-2)",
                  flush=True)

            # 3. Localized labels; all twelve zh-TW buttons fit the 120-column row.
            run("set", "lang", "zh-TW")
            zh = first_line(run("debug-frame", "-t", "toolbar", "--text"))
            assert "說明" in zh and "新視窗" in zh, zh
            assert "F12 離開" in zh and "…" not in zh, zh
            run("set", "lang", "en")
            assert first_line(run("debug-frame", "-t", "toolbar", "--text")).startswith("F1 help")
            print("ok set lang zh-TW shows 說明 新視窗 (all twelve fit in 120 columns); en restored",
                  flush=True)

            # 4. ASCII glyphs: box-drawing labels swap to split| / split- and the row is pure ASCII.
            run("set", "ascii-glyphs", "on")
            ascii_top = first_line(run("debug-frame", "-t", "toolbar", "--text"))
            assert "split|" in ascii_top and ascii_top.rstrip().isascii(), ascii_top
            run("set", "ascii-glyphs", "off")
            assert "split│" in first_line(run("debug-frame", "-t", "toolbar", "--text"))
            print("ok set ascii-glyphs on swaps to split| and an ASCII-only row; off restores box glyphs",
                  flush=True)

            # 5. A real attached client: clicks and F-keys run the buttons through the daemon.
            client = AttachedClient(socket_name)
            try:
                client.send({"Hello": {"Attach": {"session": "toolbar", "create": False, "cols": 120,
                                                  "rows": 30, "cwd": None, "detached": False}}})
                client.start()
                wait_for(lambda: "help" in client.since(0), "the attached client's first frame")
                assert window_count() == 1
                client.click(10, 0)  # F2 new spans columns 8..13 of row 0
                wait_for(lambda: window_count() == 2, "F2 new to create a window")
                mark = client.mark()
                client.click(78, 0)  # F10 close spans columns 76..84
                wait_for(lambda: "(y/n)" in client.since(mark), "F10 close to ask y/n")
                assert window_count() == 2, "close waits for the answer"
                client.key({"Char": "y"})
                wait_for(lambda: window_count() == 1, "y to close the new window")
                mark = client.mark()
                client.click(2, 0)  # F1 help opens the key list popup
                wait_for(lambda: "keys" in client.since(mark) and "split-window" in client.since(mark),
                         "F1 help to open the key list")
                client.key({"Char": "q"})
                run("set", "toolbar-fkeys", "on")
                client.key({"F": 2})
                wait_for(lambda: window_count() == 2, "plain F2 with toolbar-fkeys on")
                run("set", "toolbar-fkeys", "off")
                client.key({"F": 2})
                time.sleep(0.5)
                assert window_count() == 2, "plain F2 reaches the pane when toolbar-fkeys is off"
            finally:
                client.close()
            print("ok attached client: F2 click creates a window, F10 asks y/n then closes, "
                  "F1 opens keys, plain F2 runs only with toolbar-fkeys", flush=True)

            # 6. Custom buttons via source-file; invalid configs are rejected and keep the old row.
            run("source-file", str(custom))
            custom_top = first_line(run("debug-frame", "-t", "toolbar", "--text"))
            assert custom_top.rstrip() == "F1 build F2 關閉", custom_top
            assert "F3" not in custom_top, custom_top
            print("ok custom [[toolbar_buttons]] (CJK label) shows exactly F1 build F2 關閉 with no F3",
                  flush=True)

            for bad, expected in ((many, "at most 12 buttons"), (bad_command, "unknown command")):
                result = proc("source-file", str(bad))
                assert result.returncode == 1 and expected in result.stderr, (bad, result.returncode, result.stderr)
                kept = first_line(run("debug-frame", "-t", "toolbar", "--text")).rstrip()
                assert kept == "F1 build F2 關閉", kept
            print("ok invalid toolbar configs (13 buttons, bad command) exit 1 with a message and keep the previous row",
                  flush=True)
        except BaseException:
            if log.exists():
                print(log.read_text(encoding="utf-8", errors="replace")[-4000:], file=sys.stderr)
            raise
        finally:
            stopped = subprocess.run([binary, "kill-server"], env=env, capture_output=True, timeout=15)
            assert stopped.returncode == 0, ("isolated server stop failed", stopped.returncode, stopped.stderr)
            deadline = time.monotonic() + 15
            while subprocess.run([binary, "workspace", "list"], env=env, capture_output=True, timeout=10).returncode == 0:
                assert time.monotonic() < deadline, "isolated toolbar server did not stop"
                time.sleep(0.05)
    print("PASS toolbar smoke", flush=True)


if __name__ == "__main__":
    run_smoke(sys.argv[1] if len(sys.argv) > 1 else "target/release/crew" + (".exe" if os.name == "nt" else ""))
