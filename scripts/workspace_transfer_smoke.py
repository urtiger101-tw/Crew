#!/usr/bin/env python3
"""Exercise real live-tab transfer, shell PIDs, metadata, agent waits and persistence.

Uses a temporary isolated server; never contacts an existing user endpoint.
Usage: python scripts/workspace_transfer_smoke.py [path-to-crew]
"""
import json
import os
from pathlib import Path
import subprocess
import sys
import time
import uuid

from smoke_temp import SmokeDirectory

WIN = os.name == "nt"
EXE = str(Path(sys.argv[1] if len(sys.argv) > 1 else
               "target/release/crew" + (".exe" if WIN else "")).resolve())


def main():
    with SmokeDirectory(prefix="crew-transfer-") as temporary:
        root = Path(temporary)
        source_cwd, destination_cwd = root / "source 中文", root / "destination space"
        source_cwd.mkdir()
        destination_cwd.mkdir()
        config, state, log = (root / name for name in ("config.toml", "state.json", "server.log"))
        config.write_text("persist = true\nautomatic_rename = false\nshell = " +
                          json.dumps("powershell -NoProfile" if WIN else "/bin/sh") + "\n",
                          encoding="utf-8")
        env = {k: v for k, v in os.environ.items()
               if k.upper() != "CREW" and not k.upper().startswith("CREW_")}
        env.update(CREW_SOCKET="crew-transfer-" + uuid.uuid4().hex + ".sock",
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
            result = api("pane", "wait-output", pid, "--regex", pattern, "--timeout", "15000")
            assert result.get("matched") is True, result
            return result["line"]

        def ready(pid):
            if WIN:
                output(pid, r"PS [^\r\n]*>")

        def snapshot():
            return api("workspace", "list")["workspaces"]

        def structure():
            # Activity heuristics can change while commands run; compare durable identity
            # and selection, not elapsed-output status on unrelated fixture shells.
            return [{"workspace_id": w["workspace_id"], "active_tab": w["active_tab"],
                     "focused": w["focused"], "tokens": w["tokens"],
                     "tabs": [{key: t[key] for key in
                               ("tab_id", "workspace_id", "name", "active", "active_pane", "zoomed")}
                              for t in w["tabs"]]} for w in snapshot()]

        def tab_ids(wid):
            return [t["tab_id"] for t in api("tab", "list", "--workspace", wid)["tabs"]]

        def shell_probe(pid, phase):
            # Require complete, nonce-scoped fields. A prefix can match a PTY read
            # before its marker/env arrive; one long response can also wrap at 60 columns.
            nonce = "Q" + uuid.uuid4().hex[:8]
            command = (
                f"Write-Output ('{nonce}:P=' + $PID + ';'); "
                f"Write-Output ('{nonce}:M=' + $global:CREW_TRANSFER_MARKER + ';'); "
                f"Write-Output ('{nonce}:E=' + $env:WORKSPACE_MARKER + ';')"
                if WIN else
                f"printf '{nonce}:P=%s;\\n{nonce}:M=%s;\\n{nonce}:E=%s;\\n' "
                '"$$" "$CREW_TRANSFER_MARKER" "$WORKSPACE_MARKER"'
            )
            api("pane", "run", pid, command)
            fields = [output(pid, f"^{nonce}:{field}={pattern};$").split("=", 1)[1][:-1]
                      for field, pattern in (("P", "[0-9]+"), ("M", "(?:[0-9a-f]{32})?"),
                                             ("E", "(?:origin|destination)"))]
            print(f"probe {phase} {pid}: {':'.join(fields)}", flush=True)
            return ":".join(fields)

        def transfer(tid, target, index=None, source=None, *, closed=False):
            before = api("events", "list", "--limit", "512")["latest_cursor"]
            args = ["tab", "move", tid, "--to-workspace", target]
            if index is not None:
                args += ["--index", str(index)]
            if source is not None:
                args += ["--workspace", source]
            reply = api(*args)
            assert reply["moved"] is True, reply
            assert reply["source_workspace_closed"] is closed, reply
            emitted = api("events", "list", "--after", str(before), "--limit", "512")["events"]
            moved = [e for e in emitted if e["kind"] == "tab.moved"]
            assert len(moved) == 1, emitted
            for field in ("workspace_id", "tab_id", "from_workspace_id", "to_workspace_id",
                          "insert_index", "from_index", "to_index", "source_workspace_closed",
                          "source_tab_ids", "tab_ids"):
                assert moved[0]["data"][field] == reply[field], (field, moved, reply)
            assert not any(e["kind"] in ("pane.closed", "tab.closed", "pane.created", "tab.created")
                           for e in emitted), emitted
            closures = [e for e in emitted if e["kind"] == "workspace.closed"]
            assert len(closures) == int(closed), emitted
            if closed:
                assert closures[0]["data"]["workspace_id"] == reply["from_workspace_id"]
                assert closures[0]["data"]["reason"] == "last_tab_transferred"
                assert closures[0]["cursor"] > moved[0]["cursor"]
            return reply

        try:
            src = api("workspace", "create", "source.project:中文", "--cwd", str(source_cwd),
                      "--env", "WORKSPACE_MARKER=origin", "--focus")["workspace"]
            dst = api("workspace", "create", "destination.project:中文", "--cwd", str(destination_cwd),
                      "--env", "WORKSPACE_MARKER=destination")["workspace"]
            other = api("workspace", "create", "other.project", "--cwd", str(root))["workspace"]
            a, b, c = (w["workspace_id"] for w in (src, dst, other))
            tid, pane = src["active_tab"], src["tabs"][0]["active_pane"]
            api("tab", "rename", tid, "transferred.tab:中文")
            neighbour = api("tab", "create", "source-neighbour", "--workspace", a)["tab"]
            destination_active = dst["active_tab"]
            split = api("pane", "split", pane, "--direction", "right", "--no-focus")["pane"]["pane_id"]
            api("pane", "zoom", pane, "--on")
            token = uuid.uuid4().hex
            for pid in (pane, split):
                ready(pid)
                initialize = ("$global:CREW_TRANSFER_MARKER='" + token + "'; Write-Output 'TRANSFER_READY'"
                              if WIN else "CREW_TRANSFER_MARKER='" + token + "'; printf '%s\\n' 'TRANSFER_READY'")
                api("pane", "run", pid, initialize)
                output(pid, "^TRANSFER_READY$")
            initial_processes = {pid: shell_probe(pid, "BEFORE_TRANSFER") for pid in (pane, split)}
            assert all(value.endswith(":" + token + ":origin") for value in initial_processes.values())
            reported = api("pane", "report-agent", pane, "--source", "transfer-smoke", "--agent", "fixture",
                           "--state", "working", "--seq", "1")
            instance = reported["pane"]["agent"]["instance"]
            api("agent", "rename", pane, "transfer-agent")
            api("pane", "report-metadata", pane, "--source", "transfer-smoke", "--token", "task=keep", "--seq", "7")
            api("workspace", "report-metadata", a, "--source", "transfer-smoke", "--token", "branch=source")
            api("workspace", "report-metadata", b, "--source", "transfer-smoke", "--token", "branch=destination")
            waiter = subprocess.Popen([EXE, "agent", "wait", "transfer-agent", "--until", "idle", "--timeout", "60000"],
                                      env=env, cwd=root, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                                      text=True, encoding="utf-8")
            print("ok isolated two-pane PTY, shell PIDs, source env, metadata and agent wait established", flush=True)

            before = structure()
            error("workspace_not_found", "tab", "move", tid, "--to-workspace", "missing")
            error("tab_not_found", "tab", "move", tid, "--workspace", b, "--to-workspace", c)
            error("invalid_argument", "tab", "move", tid, "--to-workspace", b, "--index", "99")
            assert structure() == before
            collision = api("tab", "create", "transferred.tab:中文", "--workspace", b)["tab"]
            error("name_taken", "tab", "move", tid, "--to-workspace", b)
            api("tab", "rename", collision["tab_id"], "destination-extra")
            print("ok invalid destination, source scope, slot and manual name collision reject before mutation", flush=True)

            moved = transfer(tid, b, 0, a)
            assert moved["from_workspace_id"] == a and moved["to_workspace_id"] == b
            assert tab_ids(a) == [neighbour["tab_id"]]
            assert tab_ids(b) == [tid, destination_active, collision["tab_id"]]
            assert api("workspace", "get", b)["workspace"]["active_tab"] == destination_active
            assert api("workspace", "get", a)["workspace"]["active_tab"] == neighbour["tab_id"]
            assert api("workspace", "current")["workspace"]["workspace_id"] == a
            current = api("tab", "get", tid)["tab"]
            assert current["active_pane"] == pane and current["zoomed"] is True
            assert {p["pane_id"] for p in current["panes"]} == {pane, split}
            assert api("pane", "get", pane)["pane"]["tokens"] == {"task": "keep"}
            assert api("pane", "get", pane)["pane"]["agent"]["instance"] == instance
            print("ok transfer preserves pane/layout identity, zoom, agent instance and destination selection", flush=True)

            env["CREW_PANE_ID"] = pane
            try:
                assert api("workspace", "current")["workspace"]["workspace_id"] == b
                assert api("tab", "current")["tab"]["tab_id"] == tid
                transfer("transferred.tab:中文", "source.project:中文")
                assert api("workspace", "current")["workspace"]["workspace_id"] == a
            finally:
                env.pop("CREW_PANE_ID")
            assert tab_ids(a) == [neighbour["tab_id"], tid]
            before_cursor = api("events", "list", "--limit", "512")["latest_cursor"]
            noop = api("tab", "move", tid, "--to-workspace", a)
            assert noop["moved"] is False
            assert not any(e["kind"] == "tab.moved" for e in
                           api("events", "list", "--after", str(before_cursor), "--limit", "512")["events"])
            print("ok caller pane resolves its new workspace, exact names append and same-workspace no-op works", flush=True)

            transfer(neighbour["tab_id"], c)
            closed = transfer(tid, b, closed=True)
            assert closed["source_tabs"] == []
            error("workspace_not_found", "workspace", "get", a)
            assert api("workspace", "get", b)["workspace"]["tokens"] == {"branch": "destination"}
            assert api("pane", "get", pane)["pane"]["tokens"] == {"task": "keep"}
            stale = api("pane", "report-metadata", pane, "--source", "transfer-smoke",
                        "--token", "task=stale", "--seq", "6")
            assert stale["accepted"] is False and stale["pane"]["tokens"] == {"task": "keep"}
            assert api("workspace", "current")["workspace"]["workspace_id"] == b
            assert api("workspace", "get", b)["workspace"]["active_tab"] == destination_active
            for pid in (pane, split):
                after = shell_probe(pid, "AFTER_TRANSFER")
                assert after == initial_processes[pid], (pid, initial_processes[pid], after)
                assert os.path.samefile(api("pane", "get", pid)["pane"]["cwd"], source_cwd)
            assert waiter.poll() is None, "a move must not complete/replace an agent wait"
            print("ok last-tab move closes only empty source; live PIDs, shell state, original cwd/env and token seq survive", flush=True)

            api("pane", "report-agent", pane, "--source", "transfer-smoke", "--agent", "fixture",
                "--state", "idle", "--seq", "2")
            out, err = waiter.communicate(timeout=15)
            assert waiter.returncode == 0, (out, err)
            assert json.loads(out)["result"]["agent"]["agent"]["instance"] == instance, out
            new_tab = api("tab", "create", "destination-inherited", "--workspace", b)["tab"]
            ready(new_tab["active_pane"])
            assert shell_probe(new_tab["active_pane"], "NEW_DESTINATION").endswith("::destination")
            print("ok same agent occupant completes pending wait; new destination tabs inherit destination env", flush=True)

            saved = snapshot()
            saved_ids = [w["workspace_id"] for w in saved]
            saved_tabs = {w["workspace_id"]: [t["tab_id"] for t in w["tabs"]] for w in saved}
            saved_active = {w["workspace_id"]: w["active_tab"] for w in saved}
            api("server", "stop")
            wait_off()
            disk = json.loads(state.read_text(encoding="utf-8"))
            assert ["w" + str(w["id"]) for w in disk["sessions"]] == saved_ids
            assert all("w" + str(w["id"]) != a for w in disk["sessions"])
            anchor = api("workspace", "create", "restart-anchor", "--cwd", str(root))["workspace"]
            restored = snapshot()
            assert [w["workspace_id"] for w in restored] == saved_ids + [anchor["workspace_id"]]
            for w in restored[:-1]:
                assert [t["tab_id"] for t in w["tabs"]] == saved_tabs[w["workspace_id"]]
                assert w["active_tab"] == saved_active[w["workspace_id"]]
            restored_tab = api("tab", "get", tid)["tab"]
            assert restored_tab["workspace_id"] == b and restored_tab["zoomed"] is True
            assert all(os.path.samefile(p["cwd"], source_cwd) for p in restored_tab["panes"])
            assert int(anchor["workspace_id"][1:]) > max(int(w[1:]) for w in saved_ids + [a])
            print("ok cold restart preserves membership, order, stable IDs, selection, zoom and cwd without resurrecting source", flush=True)
        except BaseException:
            if pane:
                try:
                    result = run("pane", "read", pane, "--source", "recent", "--lines", "40")
                    print("pane diagnostic:", result.stdout, result.stderr, file=sys.stderr)
                except Exception as diagnostic:
                    print("pane diagnostic unavailable:", diagnostic, file=sys.stderr)
            if log.exists():
                print(log.read_text(encoding="utf-8", errors="replace")[-6000:], file=sys.stderr)
            raise
        finally:
            if waiter is not None and waiter.poll() is None:
                waiter.kill()
                waiter.communicate(timeout=5)
            run("kill-server")
            wait_off()

    print("PASS workspace transfer smoke", flush=True)


if __name__ == "__main__":
    main()
