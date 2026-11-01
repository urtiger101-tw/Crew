#!/usr/bin/env python3
"""Native metadata display/API/TUI regression, without paid coding agents.

Usage: python scripts/presentation_smoke.py [path-to-crew]
Runs on Linux, macOS or Windows against the platform's native Crew binary.
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
    with tempfile.TemporaryDirectory(prefix="crew-presentation-") as tmp:
        root = Path(tmp)
        config = root / "config.toml"
        config.write_text('persist = true\nautomatic_rename = false\nsidebar = true\n'
                          'pane_titles = "always"\nhints = false\nlang = "en"\n'
                          'pane_template = "P:$task"\nsidebar_pane_template = "A:$agent"\n'
                          'sidebar_workspace_template = "W:$title"\nshell = ' +
                          json.dumps("powershell -NoProfile" if WIN else "/bin/sh") + "\n", encoding="utf-8")
        log = root / "server.log"
        env = {k: v for k, v in os.environ.items() if not k.upper().startswith("CREW")}
        env.update(CREW_SOCKET="crew-presentation-" + uuid.uuid4().hex + ".sock",
                   CREW_CONFIG=str(config), CREW_STATE=str(root / "state.json"),
                   CREW_LOG=str(log), CREW_LANG="en")
        pending = []

        def run(*args, code=0):
            result = subprocess.run([EXE, *args], env=env, cwd=root, capture_output=True,
                                    text=True, encoding="utf-8", errors="replace", timeout=30)
            assert result.returncode == code, (args, result.returncode, result.stdout, result.stderr)
            return result

        def api(*args):
            return json.loads(run(*args).stdout)["result"]

        def poll(get, predicate, timeout=10):
            deadline = time.monotonic() + timeout
            while True:
                value = get()
                if predicate(value):
                    return value
                assert time.monotonic() < deadline, value
                time.sleep(0.05)

        def wait_off():
            poll(lambda: subprocess.run([EXE, "workspace", "list"], env=env,
                                        capture_output=True, timeout=10).returncode,
                 lambda code: code != 0)

        try:
            workspace = api("workspace", "create", "presentation", "--cwd", str(root))["workspace"]
            wid = workspace["workspace_id"]
            pane = workspace["tabs"][0]["active_pane"]
            if WIN:
                assert api("pane", "wait-output", pane, "--regex", r"PS [^\r\n]*>", "--timeout", "15000")["matched"]
            api("pane", "split", pane, "--direction", "right", "--no-focus")

            def state():
                return api("pane", "get", pane)["pane"]

            def report(*args):
                return api("pane", "report-metadata", pane, "--source", "ui", *args)

            def lifecycle(kind="fixture-agent", source="life", status="working", seq="1"):
                return api("pane", "report-agent", pane, "--source", source,
                           "--agent", kind, "--state", status, "--seq", seq)

            lifecycle()
            report("--title", "TITLE", "--display-agent", "BOT", "--state-label", "working=BUILD",
                   "--agent", "fixture-agent", "--applies-to-source", "life", "--token", "task=BUILD", "--seq", "1")
            api("workspace", "report-metadata", wid, "--source", "ui", "--title", "TEAM", "--seq", "1")
            p = state()
            assert p["display"]["title"] == "P:BUILD" and p["display"]["sidebar"] == "A:BOT", p
            assert p["display"]["state_label"] == "BUILD" and p["status"] == "working", p
            assert p["display"]["presentation"]["title"] == "TITLE", p
            assert api("workspace", "get", wid)["workspace"]["display"]["sidebar"] == "W:TEAM"
            frame = run("debug-frame", "-t", "presentation", "--text").stdout
            assert all(value in frame for value in ("P:BUILD", "A:BOT", "W:TEAM")), frame
            assert "\x1b[" in run("debug-frame", "-t", "presentation").stdout
            snapshot = api("api", "snapshot")
            sw = next(w for w in snapshot["sessions"] if w["workspace_id"] == wid)
            sp = next(p for w in sw["windows"] for p in w["panes"] if p["pane_id"] == pane)
            assert sp["display"]["title"] == "P:BUILD" and sw["display"]["title"] == "TEAM", snapshot
            print("ok real PTY TUI titles/sidebar and snapshot use the same display projection", flush=True)

            stale = report("--title", "STALE", "--seq", "1")
            assert stale["accepted"] is False and state()["display"]["presentation"]["title"] == "TITLE"
            lifecycle(kind="other-agent", source="other")
            assert state()["display"]["presentation"]["title"] is None
            assert state()["tokens"]["task"] == "BUILD", "guards incorrectly hid independent tokens"
            lifecycle(seq="2")
            assert state()["display"]["presentation"]["title"] == "TITLE"
            print("ok stale sequence rejection and agent/source guards preserve token visibility", flush=True)

            waiter = subprocess.Popen([EXE, "agent", "wait", pane, "--until", "idle", "--timeout", "15000"],
                                      env=env, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                                      text=True, encoding="utf-8")
            pending.append(waiter)
            report("--state-label", "working=IDLE", "--seq", "2")
            original = state()["agent"]
            report("--title", "TEMP", "--ttl-ms", "300", "--seq", "3")
            report("--token", "task=KEPT", "--seq", "4")
            expired = poll(state, lambda p: p["display"]["presentation"]["title"] is None)
            assert expired["tokens"]["task"] == "KEPT"
            assert expired["agent"] == original and expired["status"] == "working"
            assert waiter.poll() is None, "presentation label or expiry completed an agent wait"
            assert report("--title", "RESURRECTED", "--seq", "3")["accepted"] is False
            lifecycle(status="idle", seq="3")
            out, err = waiter.communicate(timeout=20)
            assert waiter.returncode == 0, (out, err)
            events = api("events", "list", "--limit", "512")["events"]
            assert any(e["kind"] == "pane.metadata_updated" and e["data"].get("reason") == "expired"
                       and e["data"].get("presentation_changed") for e in events)
            print("ok autonomous presentation TTL, token-only isolation and unchanged explicit wait identity", flush=True)

            report("--title", "CLEAR", "--display-agent", "LABEL", "--seq", "5")
            report("--clear-title", "--seq", "6")
            assert state()["display"]["presentation"]["title"] is None
            assert state()["display"]["presentation"]["display_agent"] == "LABEL"
            report("--clear-presentation", "--seq", "7")
            assert state()["display"]["presentation"]["display_agent"] is None
            assert state()["tokens"]["task"] == "KEPT"
            run("set", "pane-template", "$missing")
            assert state()["display"]["title"] == "fixture-agent", state()
            run("set", "pane-template", "$$ $task $(literal)")
            assert state()["display"]["title"] == "$ KEPT $(literal)", state()
            run("set", "pane-template", "\x1b[31m", code=1)
            assert state()["display"]["title"] == "$ KEPT $(literal)"
            for invalid in ("\t$title", "$title\n", "\r$title"):
                run("set", "pane-template", invalid, code=1)
                assert state()["display"]["title"] == "$ KEPT $(literal)"
            run('set', 'pane-template', r'"$task"\tail#value')
            assert state()['display']['title'] == r'"KEPT"\tail#value', state()
            run('set', 'pane-template', "$task")
            assert state()['display']['title'] == 'KEPT'
            print("ok field clears, whole-source clear, fallback, literal templates and rejected control characters", flush=True)

            def guard_report(*args):
                return api("pane", "report-metadata", pane, "--source", "guard-edge", *args)

            guard_report("--agent", "fixture-agent", "--applies-to-source", "life", "--seq", "1")
            guard_report("--title", "FIRST", "--seq", "2")
            assert state()["display"]["presentation"]["title"] == "FIRST"
            guard_report("--clear-title", "--seq", "3")
            guard_report("--title", "GUARDED", "--seq", "4")
            lifecycle(kind="other-agent", source="other", seq="2")
            assert state()["display"]["presentation"]["title"] is None
            lifecycle(seq="4")
            assert state()["display"]["presentation"]["title"] == "GUARDED"
            guard_report("--clear-presentation", "--seq", "5")
            guard_report("--title", "UNGUARDED", "--seq", "6")
            lifecycle(kind="other-agent", source="other", seq="3")
            assert state()["display"]["presentation"]["title"] == "UNGUARDED"
            guard_report("--clear-presentation", "--seq", "7")
            print("ok guard-only patches and clearing the final field retain guards until clear-presentation", flush=True)

            request = {"id": "presentation", "method": "pane.report-metadata", "params": {"args": [
                pane, "--source", "bridge", "--title", "literal $value; 中文", "--seq", "0"]}}
            bridge = subprocess.run([EXE, "api", "serve"], input=json.dumps(request) + "\n", env=env,
                                    capture_output=True, text=True, encoding="utf-8", timeout=20)
            assert bridge.returncode == 0 and json.loads(bridge.stdout)["result"]["accepted"], bridge.stderr
            assert state()["display"]["presentation"]["title"] == "literal $value; 中文"
            api("server", "stop")
            wait_off()
            api("workspace", "create", "restart", "--cwd", str(root))
            restored = api("workspace", "get", wid)["workspace"]
            assert restored["display"]["presentation"]["title"] is None
            assert all(p["display"]["presentation"]["title"] is None for t in restored["tabs"] for p in t["panes"])
            assert api("workspace", "report-metadata", wid, "--source", "ui", "--title", "FRESH", "--seq", "0")["accepted"]
            print("ok JSON-lines display patch and cold restart discard ephemeral presentation/sequence", flush=True)
            print("PASS presentation smoke", flush=True)
        except BaseException:
            if log.exists():
                print(log.read_text(encoding="utf-8", errors="replace")[-5000:], file=sys.stderr)
            raise
        finally:
            for process in pending:
                if process.poll() is None:
                    process.kill()
                    process.wait(timeout=5)
            subprocess.run([EXE, "kill-server"], env=env, capture_output=True, timeout=15)
            wait_off()


if __name__ == "__main__":
    main()
    from sidebar_style_smoke import run_smoke
    run_smoke(EXE)
