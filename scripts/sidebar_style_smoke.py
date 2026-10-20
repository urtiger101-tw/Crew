#!/usr/bin/env python3
"""Exercise styled sidebar labels through a native daemon, API and ANSI frames."""
import json
import os
from pathlib import Path
import re
import subprocess
import sys
import time
import unicodedata
import uuid

from smoke_temp import SmokeDirectory


def styled_characters(frame):
    """Read SGR attributes in Crew's emitted frame without interpreting metadata."""
    escape = re.compile(r"\x1b\[([0-9;?]*)([A-Za-z~])")
    result = []
    fg, bold, dim = None, False, False
    row, col = 0, 0
    pos = 0
    while pos < len(frame):
        match = escape.match(frame, pos)
        if match:
            if match[2] == "m":
                codes = [int(value or "0") for value in match[1].split(";")]
                i = 0
                while i < len(codes):
                    code = codes[i]
                    if code == 0:
                        fg, bold, dim = None, False, False
                    elif code == 1:
                        bold = True
                    elif code == 2:
                        dim = True
                    elif code == 22:
                        bold, dim = False, False
                    elif code == 39:
                        fg = None
                    elif code == 38 and codes[i + 1:i + 2] == [2]:
                        fg = tuple(codes[i + 2:i + 5])
                        i += 4
                    elif code == 38 and codes[i + 1:i + 2] == [5]:
                        fg = ("index", codes[i + 2])
                        i += 2
                    elif 30 <= code <= 37 or 90 <= code <= 97:
                        fg = ("ansi", code)
                    i += 1
            elif match[2] in ("H", "f"):
                values = [int(value or "1") for value in match[1].split(";")]
                row, col = values[0] - 1, (values[1] if len(values) > 1 else 1) - 1
            elif match[2] == "G":
                col = int(match[1] or "1") - 1
            pos = match.end()
        else:
            if frame[pos] >= " ":
                char = frame[pos]
                result.append((char, fg, bold, dim, row, col))
                col += 0 if unicodedata.combining(char) else 2 if unicodedata.east_asian_width(char) in ("W", "F") else 1
            elif frame[pos] == "\n":
                row += 1
            elif frame[pos] == "\r":
                col = 0
            pos += 1
    return result


def occurrence(frame, text):
    chars = styled_characters(frame)
    plain = "".join(char[0] for char in chars)
    start = plain.index(text)
    assert plain.find(text, start + 1) == -1, (text, plain)
    found = chars[start:start + len(text)]
    assert all(char[4] == found[0][4] and char[5] == found[0][5] + i
               for i, char in enumerate(found)), ("label crossed screen positions", found)
    return found


def run_smoke(binary):
    binary = str(Path(binary).resolve())
    with SmokeDirectory(prefix="crew-sidebar-style-") as temporary:
        root = Path(temporary)
        config = root / "config.toml"
        config.write_text(
            'persist = false\nautomatic_rename = false\nsidebar = true\n'
            'sidebar_width = 50\nhints = false\nlang = "en"\n'
            'pane_template = "TERMINAL"\nsidebar_pane_template = "FALLBACK"\n'
            'sidebar_pane_segments = ["$missing", '
            '{template="$left", fg="#3af", bold=false, dim=true}, '
            '{template="$right", fg="#123456", bold=true, dim=false}, "$missing"]\n'
            'shell = ' + json.dumps("powershell.exe -NoProfile" if os.name == "nt" else "/bin/sh") + "\n",
            encoding="utf-8",
        )
        log = root / "server.log"
        env = {key: value for key, value in os.environ.items()
               if not key.upper().startswith(("CREW", "TMUXLIKE"))}
        env.update(CREW_SOCKET="crew-sidebar-style-" + uuid.uuid4().hex + ".sock",
                   CREW_CONFIG=str(config), CREW_STATE=str(root / "state.json"),
                   CREW_LOG=str(log), CREW_LANG="en")

        def run(*args, code=0):
            result = subprocess.run([binary, *args], env=env, cwd=root, capture_output=True,
                                    text=True, encoding="utf-8", errors="strict", timeout=30)
            assert result.returncode == code, (args, result.returncode, result.stdout, result.stderr)
            return result.stdout

        def api(*args):
            return json.loads(run(*args))["result"]

        try:
            workspace = api("workspace", "create", "styles", "--cwd", str(root))["workspace"]
            pane = workspace["tabs"][0]["active_pane"]
            if os.name == "nt":
                assert api("pane", "wait-output", pane, "--regex", r"PS [^\r\n]*>",
                           "--timeout", "15000")["matched"]
            api("pane", "report-agent", pane, "--source", "lifecycle", "--agent", "fixture-agent",
                "--state", "blocked", "--seq", "1")
            api("pane", "report-metadata", pane, "--source", "style", "--token", "left=STYLE_A",
                "--token", "right=STYLE_B", "--seq", "1")
            state = api("pane", "get", pane)["pane"]
            display = state["display"]
            assert state["status"] == "blocked" and display["title"] == "TERMINAL", state
            assert display["sidebar"] == "STYLE_A · STYLE_B", display
            assert "".join(span["text"] for span in display["sidebar_spans"]) == display["sidebar"]
            snapshot = api("api", "snapshot")
            saved = next(p for w in snapshot["sessions"] for t in w["windows"] for p in t["panes"]
                         if p["pane_id"] == pane)
            assert saved["display"] == display
            frame = run("debug-frame", "-t", "styles")
            label = occurrence(frame, "STYLE_A · STYLE_B")
            assert all(char[1:4] == ((51, 170, 255), False, True) for char in label[:7]), label
            assert all(char[1:4] == ((18, 52, 86), True, False) for char in label[10:]), label
            assert all(char[1:4] == (None, True, False) for char in label[7:10]), label
            row = sorted((char for char in styled_characters(frame) if char[4] == label[0][4]), key=lambda char: char[5])
            row_text = "".join(char[0] for char in row)
            state_start = row_text.index("blocked")
            assert all(char[1] not in ((51, 170, 255), (18, 52, 86)) and not char[2]
                       for char in row[state_start:state_start + 7]), row
            pointer = next(char for char in row if char[5] == label[0][5] - 4)
            icon = next(char for char in row if char[5] == label[0][5] - 2)
            assert pointer[2] and icon[1] not in ((51, 170, 255), (18, 52, 86)), (pointer, icon)
            assert "STYLE_A · STYLE_B" in run("debug-frame", "-t", "styles", "--text")
            print("ok native styled API/snapshot/plain frame agree; RGB, explicit modifiers and separator verified", flush=True)

            run("set", "theme", "mono")
            mono = run("debug-frame", "-t", "styles")
            assert all(char[1] is None for text in ("STYLE_A", "STYLE_B") for char in occurrence(mono, text))
            assert api("pane", "get", pane)["pane"]["status"] == "blocked"
            print("ok mono suppresses custom RGB without changing semantic agent state", flush=True)

            run("set", "theme", "terminal")
            api("pane", "report-metadata", pane, "--source", "style", "--token", "left=中文LongName",
                "--token", "right=TAIL", "--seq", "2")
            run("set", "sidebar-width", "20")
            assert "中文…" in run("debug-frame", "-t", "styles", "--text")
            run("set", "ascii-glyphs", "true")
            assert "中文~" in run("debug-frame", "-t", "styles", "--text")
            api("pane", "report-metadata", pane, "--source", "style", "--clear-token", "left",
                "--clear-token", "right", "--seq", "3")
            cleared = api("pane", "get", pane)["pane"]["display"]
            assert cleared["sidebar"] == "FALLBACK" and not cleared.get("sidebar_spans"), cleared
            print("ok metadata updates, CJK/ASCII ellipsis and empty-segment fallback through real server", flush=True)
        except BaseException:
            if log.exists():
                print(log.read_text(encoding="utf-8", errors="replace")[-4000:], file=sys.stderr)
            raise
        finally:
            stopped = subprocess.run([binary, "kill-server"], env=env, capture_output=True, timeout=15)
            assert stopped.returncode == 0, ("isolated server stop failed", stopped.returncode, stopped.stderr)
            deadline = time.monotonic() + 15
            while subprocess.run([binary, "workspace", "list"], env=env, capture_output=True, timeout=10).returncode == 0:
                assert time.monotonic() < deadline, "isolated styled-sidebar server did not stop"
                time.sleep(0.05)
    print("PASS styled sidebar smoke", flush=True)


if __name__ == "__main__":
    run_smoke(sys.argv[1] if len(sys.argv) > 1 else "target/release/crew" + (".exe" if os.name == "nt" else ""))
