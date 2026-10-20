#!/usr/bin/env python3
"""Native PTY process-kind detection using harmless local executable fixtures.

Usage: python scripts/agent_detection_smoke.py [path-to-crew]
Linux/macOS copy the system sleep executable; Windows copies cmd.exe and runs
its built-in pause. No vendor agent, authentication or paid service is used.
"""
import json
import os
from pathlib import Path
import shlex
import shutil
import subprocess
import sys
import tempfile
import time
import uuid

WIN = os.name == "nt"
EXE = str(Path(sys.argv[1] if len(sys.argv) > 1 else
               "target/release/crew" + (".exe" if WIN else "")).resolve())


def main():
    if sys.platform not in ("linux", "darwin", "win32"):
        raise SystemExit("NOT_RUN agent detection smoke: no native backend for " + sys.platform)
    with tempfile.TemporaryDirectory(prefix="crew-detection-") as tmp:
        root = Path(tmp)
        config = root / "config.toml"
        config.write_text('persist = false\nautomatic_rename = false\nagent_detection = true\nshell = ' +
                          json.dumps("powershell -NoProfile" if WIN else "/bin/bash --noprofile --norc") + "\n", encoding="utf-8")
        log = root / "server.log"
        env = {k: v for k, v in os.environ.items() if not k.upper().startswith(("CREW", "TMUXLIKE"))}
        env.update(CREW_SOCKET="crew-detection-" + uuid.uuid4().hex + ".sock",
                   CREW_CONFIG=str(config), CREW_STATE=str(root / "state.json"),
                   CREW_LOG=str(log), CREW_LANG="en")
        pending = []
        fixture_source = Path(os.environ.get("SystemRoot", "C:/Windows")) / "System32/cmd.exe" if WIN else Path("/bin/sleep")
        fixtures = {}
        for name in ("codex", "claude"):
            destination = root / (name + (".exe" if WIN else ""))
            shutil.copy2(fixture_source, destination)
            fixtures[name] = destination

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

        def assert_no_detection_for(seconds):
            until = time.monotonic() + seconds
            while time.monotonic() < until:
                assert state()["detected_agent"] is None, state()
                time.sleep(0.08)

        def wait_off():
            poll(lambda: subprocess.run([EXE, "workspace", "list"], env=env,
                                        capture_output=True, timeout=10).returncode,
                 lambda code: code != 0)

        try:
            workspace = api("workspace", "create", "detection", "--cwd", str(root))["workspace"]
            pane = workspace["tabs"][0]["active_pane"]

            def state():
                return api("pane", "get", pane)["pane"]

            def start(kind):
                command = "& '" + str(fixtures[kind]).replace("'", "''") + "' /d /c pause" if WIN else shlex.quote(str(fixtures[kind])) + " 30"
                api("pane", "run", pane, command)
                return poll(state, lambda p: (p["detected_agent"] or {}).get("agent") == kind)

            def stop():
                api("pane", "send-keys", pane, "enter" if WIN else "ctrl+c")
                return poll(state, lambda p: p["detected_agent"] is None)

            if WIN:
                assert api("pane", "wait-output", pane, "--regex", r"PS [^\r\n]*>", "--timeout", "15000")["matched"]
            else:
                assert api("pane", "wait-output", pane, "--regex", r"[$#] ", "--timeout", "10000")["matched"]
            assert state()["detected_agent"] is None
            # Merely having codex/claude files in cwd or printing their names is not evidence.
            api("pane", "run", pane, "Write-Output 'codex claude working'" if WIN else "printf 'codex claude working\\n'")
            assert_no_detection_for(1.2)
            if not WIN:
                api("pane", "run", pane, "/bin/bash -c 'exec -a codex /bin/sleep 3'")
                assert_no_detection_for(1.2)
                api("pane", "send-keys", pane, "ctrl+c")
                # An unrelated background process is not the foreground group evidence.
                if sys.platform == "linux":
                    api("pane", "run", pane, shlex.quote(str(fixtures["codex"])) + " 2 &")
                    assert_no_detection_for(1.2)
            print("ok installed fixture/printed name/argv spoof do not claim a live agent", flush=True)

            first = start("codex")
            backend = "windows-descendant-process" if WIN else ("macos-foreground-process" if sys.platform == "darwin" else "linux-foreground-process")
            assert first["detected_agent"]["evidence"] == backend, first
            assert first["agent"] is None and first["display"]["kind_source"] == "process-detection", first
            assert first["display"]["agent"] == "codex"
            assert any(p["pane_id"] == pane for p in api("agent", "list")["agents"])
            explained = api("integration", "explain", pane)
            assert explained["classification_source"] == "process-detection" and explained["process_detection"] is True
            missing = run("agent", "wait", pane, "--until", "idle", "--timeout", "500", code=1)
            assert json.loads(missing.stderr)["error"]["code"] == "agent_not_found", missing.stderr
            stop()
            second = start("claude")
            assert second["detected_agent"]["process_id"] != first["detected_agent"]["process_id"]
            stop()
            assert state()["display"]["kind_source"] == "none"
            print("ok native kind appearance, exit, replacement and integration diagnostics", flush=True)

            api("pane", "report-agent", pane, "--source", "explicit", "--agent", "custom-agent",
                "--state", "working", "--seq", "1")
            api("agent", "rename", pane, "reported")
            original = state()["agent"]
            waiter = subprocess.Popen([EXE, "agent", "wait", pane, "--until", "idle", "--timeout", "15000"],
                                      env=env, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                                      text=True, encoding="utf-8")
            pending.append(waiter)
            detected = start("codex")
            assert detected["agent"] == original and detected["display"]["agent"] == "reported"
            assert detected["display"]["kind_source"] == "explicit-report" and detected["status"] == "working"
            assert api("integration", "explain", pane)["classification_source"] == "explicit-report"
            run("set", "agent-detection", "off")
            cleared = poll(state, lambda p: p["detected_agent"] is None)
            assert cleared["agent"] == original and cleared["process_detection"]["enabled"] is False
            assert_no_detection_for(0.7)
            run("set", "agent-detection", "on")
            poll(state, lambda p: (p["detected_agent"] or {}).get("agent") == "codex")
            stop()
            assert state()["agent"] == original and waiter.poll() is None
            api("pane", "report-agent", pane, "--source", "explicit", "--agent", "custom-agent",
                "--state", "idle", "--seq", "2")
            out, err = waiter.communicate(timeout=20)
            assert waiter.returncode == 0, (out, err)
            events = api("events", "list", "--limit", "512")["events"]
            detection_events = [e for e in events if e["kind"] == "agent.detected"]
            assert any(e["data"]["detected_agent"] is None for e in detection_events)
            assert any((e["data"]["detected_agent"] or {}).get("agent") == "claude" for e in detection_events)
            print("ok explicit identity/state/wait priority and live enable/disable without starting agents", flush=True)
            print("PASS agent detection smoke (native " + sys.platform + ")", flush=True)
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
