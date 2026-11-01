"""Cross-platform PTY smoke test for the crew binary (run by release.yml on Linux, macOS and Windows).

Starts a detached session, checks the shell prompt shows up in the pane
(PowerShell's "PS " on Windows), types a command with send-keys, and checks
that the server renders real ANSI frames.
Usage: python scripts/smoke.py [path-to-crew]
"""
import os, subprocess, sys, time, re

exe = sys.argv[1] if len(sys.argv) > 1 else os.path.join("target", "release", "crew" + (".exe" if os.name == "nt" else ""))
import tempfile
WORK = tempfile.TemporaryDirectory(prefix="crew-smoke-")
LOG = os.path.join(WORK.name, "server.log")
CONFIG = os.path.join(WORK.name, "config.toml")
with open(CONFIG, "w", encoding="utf-8") as config:
    config.write('lang = "en"\n')
env = {k: v for k, v in os.environ.items() if not k.upper().startswith("CREW")}
env.update(CREW_SOCKET="crew-smoke-%d.sock" % os.getpid(), CREW_LOG=LOG,
           CREW_CONFIG=CONFIG, CREW_STATE=os.path.join(WORK.name, "state.json"))
win = os.name == "nt"


def tl(*args, check=True):
    r = subprocess.run([exe, *args], env=env, capture_output=True, text=True, encoding="utf-8", errors="replace", timeout=30)
    if check and r.returncode != 0:
        raise SystemExit(f"FAIL: crew {' '.join(args)} -> {r.returncode}: {r.stdout}{r.stderr}")
    return r.stdout


def wait_for(pred, what, timeout=45):
    end = time.time() + timeout
    out = ""
    while time.time() < end:
        out = tl("capture-pane", "-p", "-t", "ci")
        if pred(out):
            return out
        time.sleep(0.5)
    print("---- pane ----\n" + out + "\n--------------")
    raise SystemExit(f"FAIL: timed out waiting for {what}")


try:
    tl("new", "-d", "-s", "ci")
    print(tl("ls"))
    if win:
        out = wait_for(lambda o: "PS " in o and ">" in o, "PowerShell prompt")
    else:
        out = wait_for(lambda o: o.strip() != "", "shell prompt")
    print("---- initial pane ----\n" + out)
    tl("send-keys", "-t", "ci", "echo crew-smoke-ok", "Enter")
    out = wait_for(lambda o: any(l.strip() == "crew-smoke-ok" for l in o.splitlines()), "command output")
    print("---- after send-keys ----\n" + out)
    frame = tl("debug-frame", "-t", "ci")
    assert "\x1b[" in frame, "frame has no escape sequences: %r" % frame[:200]
    assert re.search(r"\x1b\[\d+;1H", frame), "frame has no cursor positioning"
    assert " ci " in frame, "status bar missing"
    assert "crew-smoke-ok" in frame, "pane text missing from frame"
    print("frame: %d bytes of ANSI, OK" % len(frame))
    print("PASS")
finally:
    tl("kill-server", check=False)
    if os.path.exists(LOG):
        print("---- server log ----")
        print(open(LOG, encoding="utf-8", errors="replace").read()[-6000:])

    WORK.cleanup()
