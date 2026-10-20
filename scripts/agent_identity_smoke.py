#!/usr/bin/env python3
"""Real-server regressions for agent identity and Crew's explicit hook contract.

Uses a unique socket and temporary config/state. Never launches a coding agent.
Usage: python scripts/agent_identity_smoke.py [path-to-crew]
"""
import json
import os
from pathlib import Path
import shlex
import subprocess
import sys
import tempfile
import time

WIN = os.name == "nt"
EXE = str(Path(sys.argv[1] if len(sys.argv) > 1 else "target/release/crew" + (".exe" if WIN else "")).resolve())


def main():
    with tempfile.TemporaryDirectory(prefix="crew-agent-identity-") as root:
        root = Path(root)
        env = {k: v for k, v in os.environ.items() if not k.upper().startswith(("CREW", "TMUXLIKE"))}
        config = root / "config.toml"
        config.write_text("persist = false\n", encoding="utf-8")
        env.update(CREW_SOCKET=f"crew-agent-{os.getpid()}.sock", CREW_STATE=str(root / "state.json"),
                   CREW_CONFIG=str(config), CREW_LOG=str(root / "server.log"))
        pending = []

        def run(*args, code=0):
            result = subprocess.run([EXE, *args], env=env, capture_output=True, text=True,
                                    encoding="utf-8", errors="replace", timeout=15)
            assert result.returncode == code, (args, result.returncode, result.stdout, result.stderr)
            return result

        def api(*args):
            return json.loads(run(*args).stdout)["result"]

        def error(*args, codes):
            value = json.loads(run(*args, code=1).stderr)["error"]
            assert value["code"] in codes, value
            return value

        def spawn(*args):
            process = subprocess.Popen([EXE, *args], env=env, stdout=subprocess.PIPE,
                                       stderr=subprocess.PIPE, text=True, encoding="utf-8")
            pending.append(process)
            return process

        def waiting(pane):
            process = spawn("agent", "wait", pane, "--until", "idle", "--timeout", "7000")
            time.sleep(0.2)
            assert process.poll() is None, "wait did not remain pending on working agent"
            return process

        def expect_wait_error(process):
            out, err = process.communicate(timeout=10)
            assert process.returncode == 1, (out, err)
            assert json.loads(err)["error"]["code"] in ("agent_replaced", "agent_not_found"), err

        def report(pane, source, state="working", kind="test-agent", seq=1):
            return api("pane", "report-agent", pane, "--source", source, "--agent", kind,
                       "--state", state, "--seq", str(seq))

        def release(pane, source, kind="test-agent", seq=100):
            return api("pane", "release-agent", pane, "--source", source, "--agent", kind, "--seq", str(seq))

        def event(kind):
            events = api("events", "list", "--limit", "512")["events"]
            return [e for e in events if e["kind"] == kind][-1]

        try:
            run("new", "-d", "-s", "identity")
            pane = api("pane", "list")["panes"][0]["pane_id"]
            error("agent", "wait", pane, "--timeout", "50", codes={"agent_not_found"})
            error("agent", "prompt", pane, "should-not-be-written", codes={"agent_not_found"})
            plain = api("integration", "explain", pane)
            assert plain["classification_source"] == "output-activity-heuristic" and not plain["reported"]
            print("ok ordinary terminals are not reported agents", flush=True)

            report(pane, "alpha", seq=10)
            initial = event("agent.status")
            first_instance = initial["data"]["instance"]
            assert first_instance > 0
            process = waiting(pane)
            stale = report(pane, "alpha", state="idle", seq=9)
            assert stale["applied"] is False
            time.sleep(0.2)
            assert process.poll() is None, "stale idle completed a wait"
            assert event("agent.status")["cursor"] == initial["cursor"], "stale report emitted an event"
            report(pane, "alpha", state="idle", seq=11)
            out, err = process.communicate(timeout=10)
            assert process.returncode == 0, (out, err)
            assert event("agent.status")["data"]["instance"] == first_instance
            print("ok ordinary updates keep identity; stale reports neither finish waits nor emit events", flush=True)

            report(pane, "alpha", seq=12)
            process = waiting(pane)
            report(pane, "beta", state="idle")
            expect_wait_error(process)
            assert event("agent.status")["data"]["instance"] != first_instance
            print("ok another source cannot satisfy the old wait", flush=True)

            report(pane, "beta", seq=2)
            old_instance = event("agent.status")["data"]["instance"]
            process = waiting(pane)
            release(pane, "beta")
            report(pane, "beta", state="idle")
            expect_wait_error(process)
            assert event("agent.status")["data"]["instance"] != old_instance
            print("ok release and identical re-report create a distinct occupant", flush=True)

            report(pane, "beta", seq=2)
            process = waiting(pane)
            report(pane, "beta", state="idle", kind="other-agent", seq=3)
            expect_wait_error(process)
            print("ok changed agent kind cannot satisfy the old wait", flush=True)
            release(pane, "beta", kind="other-agent")

            listing = api("integration", "list")
            assert listing["events"]["done"] == "idle" and listing["automatic_detection"] is False
            doctor = api("integration", "doctor", "codex")
            assert doctor["server_probed"] is False and doctor["agents"][0]["agent_started"] is False
            hook = ["integration", "report", "custom-agent"]
            for seq, (name, status) in enumerate((("start", "working"), ("blocked", "blocked"), ("done", "idle")), 1):
                result = api(*hook, name, "--pane", pane, "--source", "custom:run-1", "--seq", str(seq))
                assert result["pane"]["agent"]["state"] == status
            explained = api("integration", "explain", pane)
            assert explained["classification_source"] == "explicit-report"
            assert explained["pane"]["agent"]["agent"] == "custom-agent"
            api(*hook, "blocked", "--pane", pane, "--source", "custom:run-1", "--seq", "4")
            error("agent", "prompt", pane, "blocked-no-input", codes={"agent_blocked"})
            api(*hook, "exit", "--pane", pane, "--source", "custom:run-1", "--seq", "5")
            assert api("pane", "get", pane)["pane"]["agent"] is None
            assert event("agent.released")["data"]["agent"] == "custom-agent"
            run(*hook, "typo", "--pane", pane, "--source", "custom:run-1", code=2)
            print("ok generic hooks, truthful diagnostics, blocked prompt and release", flush=True)

            # A raw helper records bytes without shell line buffering. As soon as pasted
            # text arrives, replace the report during the server's 150ms Enter delay.
            # No coding agent or provider is involved.
            received = root / "received.bin"
            helper = root / "input_receiver.py"
            helper.write_text('''import os, sys
path = sys.argv[1]
if os.name == "nt":
    import msvcrt
    def receive(): return msvcrt.getwch().encode("utf-8")
else:
    import tty
    tty.setraw(sys.stdin.fileno())
    def receive(): return os.read(sys.stdin.fileno(), 1)
print("CREW_RECEIVER_READY", flush=True)
with open(path, "ab", buffering=0) as output:
    while True:
        data = receive()
        if not data: break
        output.write(data)
''', encoding="utf-8")
            argv = [sys.executable, "-u", str(helper), str(received)]
            command = "& " + " ".join("'" + arg.replace("'", "''") + "'" for arg in argv) if WIN else shlex.join(argv)
            api("pane", "run", pane, command)
            api("pane", "wait-output", pane, "--match", "CREW_RECEIVER_READY", "--timeout", "7000")
            report(pane, "prompt-old", state="idle")
            process = spawn("agent", "prompt", pane, "CREW_PROMPT_RACE")
            deadline = time.monotonic() + 7
            while not received.exists() or b"CREW_PROMPT_RACE" not in received.read_bytes():
                assert time.monotonic() < deadline, "raw helper did not receive the prompt text"
                time.sleep(0.001)
            report(pane, "prompt-new", state="idle")
            expect_wait_error(process)
            time.sleep(0.2)
            assert b"\r" not in received.read_bytes(), received.read_bytes()
            print("ok replaced occupant never receives the delayed Enter", flush=True)

            process = spawn("agent", "prompt", pane, "CREW_BLOCKED_RACE")
            deadline = time.monotonic() + 7
            while b"CREW_BLOCKED_RACE" not in received.read_bytes():
                assert time.monotonic() < deadline, "raw helper did not receive the second prompt text"
                time.sleep(0.001)
            report(pane, "prompt-new", state="blocked", seq=2)
            out, err = process.communicate(timeout=10)
            assert process.returncode == 1 and json.loads(err)["error"]["code"] == "agent_blocked", (out, err)
            time.sleep(0.2)
            assert b"\r" not in received.read_bytes(), received.read_bytes()
            print("ok an agent becoming blocked during paste never receives Enter", flush=True)
            print("PASS agent identity and integration smoke", flush=True)
        finally:
            subprocess.run([EXE, "kill-server"], env=env, capture_output=True, timeout=10)
            for process in pending:
                if process.poll() is None:
                    process.terminate()
                    process.communicate(timeout=5)


if __name__ == "__main__":
    main()
