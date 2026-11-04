#!/usr/bin/env python3
"""Exercise the system panel through an isolated native Crew daemon.

Usage: python scripts/system_status_smoke.py --binary target/release/crew.exe
"""
import argparse
import json
import os
from pathlib import Path
import platform
import re
import shutil
import socket
import struct
import subprocess
import sys
import tempfile
import threading
import time
import uuid

from smoke_temp import SmokeDirectory


LABELS = {
    "en": {
        "title": "SYSTEM",
        "launch": "Launch dir:",
        "disk": "Disk free:",
        "ram": "RAM used:",
        "os": "OS:",
        "panes": "Panes:",
    },
    "zh-TW": {
        "title": "系統狀態",
        "launch": "啟動目錄：",
        "disk": "磁碟可用：",
        "ram": "記憶體已用：",
        "os": "作業系統：",
        "panes": "窗格：",
    },
}
SIZES = ((120, 30, "roomy"), (80, 12, "short"), (60, 18, "narrow"))
IEC = {"B": 0, "KiB": 1, "MiB": 2, "GiB": 3, "TiB": 4, "PiB": 5}


def parse_amounts(line):
    """Parse a pair of compact IEC values from one rendered system row."""
    match = re.search(
        r"(\d+(?:\.\d+)?)\s*(PiB|TiB|GiB|MiB|KiB|B)?\s*/\s*"
        r"(\d+(?:\.\d+)?)\s*(PiB|TiB|GiB|MiB|KiB|B)\b", line)
    assert match, line
    first, first_unit, second, second_unit = match.groups()
    units = (first_unit or second_unit, second_unit)
    values = (first, second)
    parsed = []
    for number, unit in zip(values, units):
        assert unit in IEC, (line, unit)
        scale = 1024 ** IEC[unit]
        value = float(number) * scale
        decimals = len(number.split(".", 1)[1]) if "." in number else 0
        rounding = (0.05 if decimals else 0.5) * scale
        parsed.append((value, rounding))
    return tuple(parsed)


def row_for(frame, label):
    matches = [line.strip() for line in frame.splitlines() if label in line]
    assert len(matches) == 1, (label, matches, frame)
    return matches[0]


def self_test():
    assert parse_amounts("Disk free: 1.5 GiB / 10 GiB") == (
        (1.5 * 1024 ** 3, 0.05 * 1024 ** 3),
        (10 * 1024 ** 3, 0.5 * 1024 ** 3),
    )
    assert parse_amounts("RAM used: 12 / 32 GiB")[0] == (12 * 1024 ** 3, 0.5 * 1024 ** 3)
    assert row_for("header\n啟動目錄：C:\\temp\\測試\n", "啟動目錄：").endswith("測試")
    print("PASS system-status smoke self-test", flush=True)


class AttachedClient:
    """Send only size and mouse events to this smoke's private server."""

    def __init__(self, socket_name):
        if os.name == "nt":
            pipe = open("//./pipe/" + socket_name, "r+b", buffering=0)
            self.read, self.write = pipe.read, pipe.write
            self.close = pipe.close
        else:
            sock = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
            if sys.platform.startswith("linux"):
                sock.connect(chr(0) + socket_name)
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

    def click(self, column, row):
        for kind in ({"Down": "Left"}, {"Up": "Left"}):
            self.send({"Event": {"Mouse": {"kind": kind, "column": column, "row": row,
                                             "modifiers": ""}}})

    def resize(self, cols, rows):
        self.send({"Event": {"Resize": [cols, rows]}})


def wait_for(predicate, what, timeout=15):
    deadline = time.monotonic() + timeout
    while not predicate():
        assert time.monotonic() < deadline, "timed out waiting for " + what
        time.sleep(0.1)


def same_path(first, second):
    return (os.path.normcase(os.path.realpath(first)) ==
            os.path.normcase(os.path.realpath(second)))


def pane_state(run, session):
    panes = run("pane", "list", "--session", session)["panes"]
    assert len(panes) == 2, panes
    focused = [pane for pane in panes if pane.get("focused")]
    assert len(focused) == 1, panes
    return panes, focused[0]


def geometry(panes):
    return {pane["pane_id"]: (pane["cols"], pane["rows"]) for pane in panes}


def system_rows(frame, lang, rows):
    labels = LABELS[lang]
    required = ["launch", "disk", "ram"]
    if rows >= 16:
        required.extend(("os", "panes"))
    return {key: row_for(frame, labels[key]) for key in required}


def check_panel(frame, lang, rows, cwd, expected_panes):
    labels = LABELS[lang]
    assert labels["title"] in frame, (labels["title"], frame)
    selected = system_rows(frame, lang, rows)
    launch = selected["launch"]
    basename = Path(cwd).name
    assert basename and basename.casefold() in launch.casefold(), (basename, launch)
    available, _ = parse_amounts(selected["disk"])[0]
    total, _ = parse_amounts(selected["disk"])[1]
    assert total > 0 and 0 <= available <= total, selected["disk"]
    used, _ = parse_amounts(selected["ram"])[0]
    memory_total, _ = parse_amounts(selected["ram"])[1]
    assert memory_total > 0 and 0 <= used <= memory_total, selected["ram"]
    if rows >= 16:
        assert "pending" not in selected["os"].casefold(), selected["os"]
        os_value = selected["os"].split(":", 1)[-1].strip()
        assert os_value and "pending" not in os_value.casefold(), selected["os"]
        assert str(expected_panes) in selected["panes"], selected["panes"]
    return {"disk_available_bytes": available, "disk_total_bytes": total,
            "ram_used_bytes": used, "ram_total_bytes": memory_total,
            "visible_rows": selected}


def run_profile(binary, profile, evidence_root):
    records = []
    with SmokeDirectory(prefix="crew-system-status-") as temporary:
        root = Path(temporary)
        first_dir = root / "launch-one"
        second_dir = root / "啟動二"
        first_dir.mkdir()
        second_dir.mkdir()
        is_aurora = profile == "aurora-left-tree"
        config_text = (
            'persist = false\nautomatic_rename = false\nhints = false\n'
            'toolbar = false\npane_titles = "always"\nsidebar_width = 28\n'
            'workspace_tree = ' + ("true" if is_aurora else "false") + "\n"
            'sidebar = ' + ("false" if is_aurora else "true") + "\n"
            'lang = "en"\nshell = ' +
            json.dumps("powershell.exe -NoProfile" if os.name == "nt" else "/bin/sh") + "\n"
        )
        if is_aurora:
            config_text += '[theme]\npreset = "aurora"\n'
        (root / "config.toml").write_text(config_text, encoding="utf-8")
        log = root / "server.log"
        socket_name = "crew-system-status-" + uuid.uuid4().hex + ".sock"
        env = {key: value for key, value in os.environ.items()
               if not key.upper().startswith("CREW")}
        env.update(CREW_SOCKET=socket_name, CREW_CONFIG=str(root / "config.toml"),
                   CREW_STATE=str(root / "state.json"), CREW_LOG=str(log), CREW_LANG="en")
        client = None
        server_started = False

        def command(*args, timeout=30, code=0):
            result = subprocess.run([binary, *args], env=env, cwd=root, capture_output=True,
                                    text=True, encoding="utf-8", errors="replace", timeout=timeout)
            assert result.returncode == code, (args, result.returncode, result.stdout, result.stderr)
            return result.stdout

        def api(*args):
            result = json.loads(command(*args))
            assert "result" in result, (args, result)
            return result["result"]

        try:
            server_started = True
            workspace = api("workspace", "create", "system-status", "--cwd", str(first_dir))["workspace"]
            first_pane = workspace["tabs"][0]["active_pane"]
            api("pane", "split", first_pane, "--direction", "right", "--cwd", str(second_dir),
                "--focus")
            panes, active = pane_state(api, "system-status")
            assert same_path(panes[0]["cwd"], str(first_dir)), panes
            second_pane = next(pane["pane_id"] for pane in panes if pane["pane_id"] != first_pane)
            assert same_path(next(pane["cwd"] for pane in panes if pane["pane_id"] == second_pane),
                             str(second_dir)), panes

            client = AttachedClient(socket_name)
            client.send({"Hello": {"Attach": {"session": "system-status", "create": False,
                                                "cols": 120, "rows": 30, "cwd": None,
                                                "detached": False}}})
            client.start()
            wait_for(lambda: "SYSTEM" in "".join(client.output) or "系統狀態" in "".join(client.output),
                     "attached renderer's first frame")

            for cols, rows, size_name in SIZES:
                client.resize(cols, rows)
                wait_for(lambda: (lambda ws: ws["cols"] == cols and ws["rows"] == rows)
                         (api("workspace", "get", "system-status")["workspace"]),
                         f"{cols}x{rows} resize")
                panes, active = pane_state(api, "system-status")
                base_geometry = geometry(panes)
                assert active["pane_id"] == second_pane, (active, second_pane)
                disk = shutil.disk_usage(active["cwd"])
                sampled = None

                for lang in LABELS:
                    deadline = time.monotonic() + 25
                    while True:
                        frame = command("debug-frame", "-t", "system-status", "--lang", lang,
                                        "--text", timeout=30)
                        try:
                            sampled = check_panel(frame, lang, rows, active["cwd"], len(panes))
                        except (AssertionError, IndexError) as error:
                            if time.monotonic() >= deadline:
                                raise AssertionError(f"{profile}/{lang}/{size_name}: sample not ready: {error}\n{frame}")
                            time.sleep(0.25)
                            continue
                        break

                    expected_available = disk.free
                    expected_total = disk.total
                    disk_values = parse_amounts(sampled["visible_rows"]["disk"])
                    available_tolerance = max(disk_values[0][1], 2 * 1024 ** 2)
                    total_tolerance = max(disk_values[1][1], 2 * 1024 ** 2)
                    assert abs(sampled["disk_available_bytes"] - expected_available) <= available_tolerance, (
                        active["cwd"], sampled, disk)
                    assert abs(sampled["disk_total_bytes"] - expected_total) <= total_tolerance, (
                        active["cwd"], sampled, disk)
                    after_panes, after_active = pane_state(api, "system-status")
                    assert after_active["pane_id"] == second_pane
                    assert geometry(after_panes) == base_geometry, (base_geometry, geometry(after_panes))

                    if lang == next(iter(LABELS)):
                        title_row = next(i for i, line in enumerate(frame.splitlines())
                                         if LABELS[lang]["title"] in line)
                        panel_x = 0 if is_aurora else cols - 28 + 2
                        client.click(panel_x, title_row)
                        wait_for(lambda: pane_state(api, "system-status")[1]["pane_id"] == second_pane,
                                 "system section click to leave pane focus unchanged", timeout=3)
                        clicked_panes, _ = pane_state(api, "system-status")
                        assert geometry(clicked_panes) == base_geometry, (
                            base_geometry, geometry(clicked_panes))

                        # The known body cell in pane one must retain its ordinary focus target.
                        pane_x = 30 if is_aurora else 2
                        client.click(pane_x, max(2, rows // 2))
                        wait_for(lambda: pane_state(api, "system-status")[1]["pane_id"] == first_pane,
                                 "pane body click target", timeout=5)
                        focused_panes, _ = pane_state(api, "system-status")
                        assert geometry(focused_panes) == base_geometry

                        # Restore pane two to verify its launch directory remains the displayed one.
                        api("pane", "focus", second_pane)
                    restored_panes, restored = pane_state(api, "system-status")
                    assert restored["pane_id"] == second_pane
                    focused_frame = command("debug-frame", "-t", "system-status", "--lang", lang,
                                            "--text", timeout=30)
                    launch_row = row_for(focused_frame, LABELS[lang]["launch"])
                    assert Path(restored["cwd"]).name.casefold() in launch_row.casefold(), (
                        restored, launch_row)
                    assert geometry(restored_panes) == base_geometry

                    saved = False
                    if evidence_root is not None and size_name in ("roomy", "narrow"):
                        # Keep one representative locale per size and profile, plus the narrow
                        # Traditional Chinese frame. Other combinations remain in the summary.
                        saved = lang == "en" or (size_name == "narrow" and lang == "zh-TW")
                        if saved:
                            stem = f"{profile}-{lang}-{cols}x{rows}"
                            (evidence_root / f"{stem}.txt").write_text(frame, encoding="utf-8")
                            ansi = command("debug-frame", "-t", "system-status", "--lang", lang,
                                          timeout=30)
                            (evidence_root / f"{stem}.ansi").write_text(ansi, encoding="utf-8")
                    records.append({"profile": profile, "lang": lang, "size": [cols, rows],
                                    "active_pane": restored["pane_id"], "launch_dir": restored["cwd"],
                                    "geometry": base_geometry, "sample": sampled,
                                    "click_checks": "PASS", "frame_saved": saved})
                    print(f"ok {profile} {lang} {cols}x{rows}: live samples, pane geometry and click targets",
                          flush=True)
        except BaseException:
            if log.exists():
                print(log.read_text(encoding="utf-8", errors="replace")[-4000:], file=sys.stderr)
            raise
        finally:
            if client is not None:
                client.close()
            if server_started:
                present = subprocess.run([binary, "workspace", "list"], env=env, cwd=root,
                                         capture_output=True, timeout=10)
                if present.returncode == 0:
                    stopped = subprocess.run([binary, "kill-server"], env=env, cwd=root,
                                             capture_output=True, timeout=15)
                    assert stopped.returncode == 0, ("isolated server stop failed", stopped.returncode,
                                                     stopped.stderr)
                deadline = time.monotonic() + 15
                while subprocess.run([binary, "workspace", "list"], env=env, cwd=root,
                                     capture_output=True, timeout=10).returncode == 0:
                    assert time.monotonic() < deadline, "isolated system-status server did not stop"
                    time.sleep(0.05)
    return records


def run_smoke(binary, evidence_dir=None):
    binary_path = Path(binary).resolve()
    assert binary_path.is_file(), f"explicit Crew executable does not exist: {binary_path}"
    evidence_root = None
    if evidence_dir:
        parent = Path(evidence_dir).resolve()
        parent.mkdir(parents=True, exist_ok=True)
        evidence_root = parent / f"crew-system-status-{uuid.uuid4().hex}"
        evidence_root.mkdir()
    records = []
    for profile in ("classic-right-sidebar", "aurora-left-tree"):
        records.extend(run_profile(str(binary_path), profile, evidence_root))
    if evidence_root is not None:
        summary = {"result": "PASS", "binary": str(binary_path), "platform": platform.platform(),
                   "profiles": ["classic-right-sidebar", "aurora-left-tree"],
                   "locales": list(LABELS), "sizes": [list(size[:2]) for size in SIZES],
                   "records": records}
        (evidence_root / "summary.json").write_text(
            json.dumps(summary, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
        print(f"evidence: {evidence_root}", flush=True)
    print("PASS isolated system-status sidebar smoke", flush=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", help="explicit path to the shipping Crew executable")
    parser.add_argument("--evidence-dir", help="optional parent directory for retained frame evidence")
    parser.add_argument("--self-test", action="store_true", help="run parser checks without a Crew binary")
    args = parser.parse_args()
    if args.self_test:
        self_test()
        return
    if not args.binary:
        parser.error("--binary is required")
    run_smoke(args.binary, args.evidence_dir)


if __name__ == "__main__":
    main()
