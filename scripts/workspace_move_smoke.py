#!/usr/bin/env python3
"""Real PTY/ConPTY checks for ordering, identity, events and restored order.

Uses a unique endpoint and temporary config/state. Does not contact a user's server.
Usage: python scripts/workspace_move_smoke.py [path-to-crew]
"""
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import time
import uuid

WIN = os.name == "nt"
EXE = str(Path(sys.argv[1] if len(sys.argv) > 1 else
               "target/release/crew" + (".exe" if WIN else "")).resolve())


def main():
    with tempfile.TemporaryDirectory(prefix="crew-move-") as temporary:
        root = Path(temporary)
        cwd = root / "ordering space 中文"
        cwd.mkdir()
        config, state, log = (root / name for name in ("config.toml", "state.json", "server.log"))
        config.write_text("persist = true\nautomatic_rename = false\nshell = " +
                          json.dumps("powershell -NoProfile" if WIN else "/bin/sh") + "\n",
                          encoding="utf-8")
        env = {k: v for k, v in os.environ.items()
               if k.upper() != "CREW" and not k.upper().startswith("CREW_")}
        env.update(CREW_SOCKET="crew-move-" + uuid.uuid4().hex + ".sock",
                   CREW_CONFIG=str(config), CREW_STATE=str(state),
                   CREW_LOG=str(log), CREW_LANG="en")
        waiter = None
        pane = None

        def run(*args):
            return subprocess.run([EXE, *args], env=env, cwd=root, capture_output=True,
                                  text=True, encoding="utf-8", errors="replace", timeout=30)

        def api(*args):
            result = run(*args)
            assert result.returncode == 0, (args, result.returncode, result.stdout, result.stderr)
            return json.loads(result.stdout)["result"]

        def error(code, *args):
            result = run(*args)
            assert result.returncode == 1, (args, result.returncode, result.stdout, result.stderr)
            assert json.loads(result.stderr)["error"]["code"] == code, result.stderr

        def wait_off():
            end = time.monotonic() + 10
            while time.monotonic() < end:
                if run("workspace", "list").returncode != 0:
                    return
                time.sleep(0.05)
            raise AssertionError("server did not stop")

        def output(pid, pattern):
            value = api("pane", "wait-output", pid, "--regex", pattern, "--timeout", "15000")
            assert value.get("matched") is True, value

        def snapshot():
            return api("workspace", "list")["workspaces"]

        def identity():
            workspaces = snapshot()
            return {
                "focused": [w["workspace_id"] for w in workspaces if w["focused"]],
                "active_tabs": {w["workspace_id"]: w["active_tab"] for w in workspaces},
                "tabs": {t["tab_id"]: {
                    "workspace_id": w["workspace_id"], "name": t["name"],
                    "active": t["active"], "active_pane": t["active_pane"], "zoomed": t["zoomed"],
                    "panes": [{key: p[key] for key in
                               ("pane_id", "cwd", "label", "cols", "rows", "focused")}
                              for p in t["panes"]],
                } for w in workspaces for t in w["tabs"]},
            }

        def move(group, target, slot, expected_ids, *, workspace=None, moved=True):
            before = api("events", "list", "--limit", "512")["latest_cursor"]
            args = [group, "move", target, "--index", str(slot)]
            if workspace is not None:
                args += ["--workspace", workspace]
            reply = api(*args)
            collection = "workspaces" if group == "workspace" else "tabs"
            key = group + "_id"
            assert [item[key] for item in reply[collection]] == expected_ids, reply
            assert reply["moved"] is moved and reply["insert_index"] == slot, reply
            emitted = api("events", "list", "--after", str(before), "--limit", "512")["events"]
            changed = [e for e in emitted if e["kind"] == group + ".moved"]
            assert len(changed) == int(moved), emitted
            assert not any(e["kind"] in ("pane.closed", "tab.closed", "workspace.closed",
                                         "pane.created", "tab.created", "workspace.created")
                           for e in emitted), emitted
            if moved:
                event = changed[0]["data"]
                assert event[group + "_ids"] == expected_ids, event
                for field in (key, "from_index", "to_index", "insert_index"):
                    assert event[field] == reply[field], (field, event, reply)
                if group == "tab":
                    assert event["workspace_id"] == reply["workspace_id"], event
            return reply

        try:
            first = api("workspace", "create", "project.alpha:中文", "--cwd", str(cwd), "--focus")["workspace"]
            second = api("workspace", "create", "project.beta:中文", "--cwd", str(cwd))["workspace"]
            third = api("workspace", "create", "project.gamma:中文", "--cwd", str(cwd))["workspace"]
            a, b, c = (w["workspace_id"] for w in (first, second, third))
            initial = first["tabs"][0]
            pane = initial["active_pane"]
            t1 = initial["tab_id"]
            api("tab", "rename", t1, "terminal.first:中文")
            t2 = api("tab", "create", "review.second:中文", "--workspace", a, "--focus")["tab"]["tab_id"]
            t3 = api("tab", "create", "review.third:中文", "--workspace", a)["tab"]["tab_id"]
            b1 = second["active_tab"]
            b2 = api("tab", "create", "background.tab:中文", "--workspace", b)["tab"]["tab_id"]
            api("pane", "split", pane, "--direction", "right", "--no-focus")
            api("pane", "zoom", pane, "--on")
            if WIN:
                output(pane, r"PS [^\r\n]*>")
            marker = uuid.uuid4().hex
            initialize = ("$global:CREW_MOVE_MARKER = '" + marker + "'; Write-Output 'MOVE_SHELL_READY'"
                          if WIN else "CREW_MOVE_MARKER='" + marker + "'; printf '%s\\n' 'MOVE_SHELL_READY'")
            api("pane", "run", pane, initialize)
            output(pane, "^MOVE_SHELL_READY$")
            reported = api("pane", "report-agent", pane, "--source", "move-smoke",
                           "--agent", "fixture", "--state", "working", "--seq", "1")
            instance = reported["pane"]["agent"]["instance"]
            api("agent", "rename", pane, "ordering-agent")
            unchanged = identity()
            assert unchanged["focused"] == [a]
            waiter = subprocess.Popen([EXE, "agent", "wait", "ordering-agent", "--until", "idle",
                                       "--timeout", "60000"], env=env, cwd=root,
                                      stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                                      text=True, encoding="utf-8")
            time.sleep(0.15)
            assert waiter.poll() is None, "wait should remain pending on the working occupant"
            print("ok isolated PTY, split, zoom, named agent and pending wait established", flush=True)

            move("workspace", b, 3, [a, c, b])
            move("workspace", a, 3, [c, b, a])
            move("workspace", "project.alpha:中文", 0, [a, c, b])
            assert identity() == unchanged
            print("ok workspace first/last slots, exact name and unchanged focus/identity", flush=True)

            move("tab", t1, 3, [t2, t3, t1], workspace=a)
            move("tab", t2, 3, [t3, t1, t2])
            move("tab", t2, 0, [t2, t3, t1])
            move("tab", b2, 0, [b2, b1], workspace=b)
            assert identity() == unchanged
            env["CREW_PANE_ID"] = pane
            try:
                assert api("tab", "current")["tab"]["tab_id"] == t1
                move("tab", "terminal.first:中文", 0, [t1, t2, t3])
            finally:
                env.pop("CREW_PANE_ID")
            assert identity() == unchanged
            print("ok active/background tab moves, stable caller context, panes and zoom", flush=True)

            move("workspace", a, 0, [a, c, b], moved=False)
            move("workspace", a, 1, [a, c, b], moved=False)
            move("tab", t2, 1, [t1, t2, t3], moved=False)
            move("tab", t2, 2, [t1, t2, t3], moved=False)
            error("invalid_argument", "workspace", "move", a, "--index", "4")
            error("invalid_argument", "tab", "move", t1, "--index", "4")
            error("tab_not_found", "tab", "move", t1, "--index", "0", "--workspace", b)
            error("workspace_not_found", "workspace", "move", "project.alpha", "--index", "0")
            for args in (("workspace", "move", a),
                         ("workspace", "move", a, "--index", "-1"),
                         ("tab", "move", t1, "--index", "0", "extra")):
                assert run(*args).returncode == 2, args
            assert [w["workspace_id"] for w in snapshot()] == [a, c, b]
            assert identity() == unchanged
            print("ok adjacent no-ops emit no move events; invalid and cross-workspace requests are atomic", flush=True)

            assert waiter.poll() is None, "reorder unexpectedly completed or replaced the waiter"
            assert api("pane", "get", pane)["pane"]["agent"]["instance"] == instance
            probe = ("Write-Output ('MOVE_PTY=' + $global:CREW_MOVE_MARKER)" if WIN
                     else "printf 'MOVE_PTY=%s\\n' \"$CREW_MOVE_MARKER\"")
            api("pane", "run", pane, probe)
            output(pane, "^MOVE_PTY=" + marker + "$")
            api("pane", "report-agent", pane, "--source", "move-smoke", "--agent", "fixture",
                "--state", "idle", "--seq", "2")
            out, err = waiter.communicate(timeout=15)
            assert waiter.returncode == 0, (out, err)
            assert json.loads(out)["result"]["agent"]["agent"]["instance"] == instance, out
            print("ok shell memory survives every move and the same agent occupant completes its wait", flush=True)

            saved = snapshot()
            saved_ids = [w["workspace_id"] for w in saved]
            saved_tabs = {w["workspace_id"]: [t["tab_id"] for t in w["tabs"]] for w in saved}
            saved_active = {w["workspace_id"]: w["active_tab"] for w in saved}
            api("server", "stop")
            wait_off()
            disk = json.loads(state.read_text(encoding="utf-8"))
            assert ["w" + str(w["id"]) for w in disk["sessions"]] == saved_ids
            anchor = api("workspace", "create", "restart-anchor", "--cwd", str(cwd))["workspace"]
            restored = snapshot()
            assert [w["workspace_id"] for w in restored] == saved_ids + [anchor["workspace_id"]]
            for w in restored[:-1]:
                wid = w["workspace_id"]
                assert [t["tab_id"] for t in w["tabs"]] == saved_tabs[wid]
                assert w["active_tab"] == saved_active[wid]
            assert api("workspace", "current")["workspace"]["workspace_id"] == a
            print("ok cold restart preserves authoritative workspace/tab order, stable IDs and active tabs", flush=True)
            print("PASS workspace move smoke", flush=True)
        except BaseException:
            if pane:
                try:
                    diagnostic = run("pane", "read", pane, "--source", "recent", "--lines", "30")
                    print("pane diagnostic:", diagnostic.stdout, diagnostic.stderr, file=sys.stderr)
                except Exception as diagnostic_error:
                    print("pane diagnostic unavailable:", diagnostic_error, file=sys.stderr)
            if log.exists():
                print(log.read_text(encoding="utf-8", errors="replace")[-6000:], file=sys.stderr)
            raise
        finally:
            if waiter is not None and waiter.poll() is None:
                waiter.kill()
                waiter.communicate(timeout=5)
            run("kill-server")
            wait_off()


if __name__ == "__main__":
    main()
