#!/usr/bin/env python3
"""Native metadata/TTL/sequence/lifecycle regression; isolated real Crew server.

No coding agent is launched. Every server/socket/config/state belongs to this test.
Usage: python scripts/metadata_smoke.py [path-to-crew]
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
    with SmokeDirectory(prefix="crew-metadata-") as tmp:
        root = Path(tmp)
        config = root / "config.toml"
        config.write_text('persist = true\nautomatic_rename = false\nshell = ' +
                          json.dumps("powershell -NoProfile" if WIN else "/bin/sh") + "\n", encoding="utf-8")
        log = root / "server.log"
        env = {k: v for k, v in os.environ.items() if not k.upper().startswith(("CREW", "TMUXLIKE"))}
        env.update(CREW_SOCKET="crew-meta-" + uuid.uuid4().hex + ".sock", CREW_CONFIG=str(config),
                   CREW_STATE=str(root / "state.json"), CREW_LOG=str(log), CREW_LANG="en")
        pending = []

        def run(*args, code=0):
            result = subprocess.run([EXE, *args], env=env, cwd=root, capture_output=True,
                                    text=True, encoding="utf-8", errors="replace", timeout=30)
            assert result.returncode == code, (args, result.returncode, result.stdout, result.stderr)
            return result

        def api(*args):
            return json.loads(run(*args).stdout)["result"]

        def report(group, target, *args):
            return api(group, "report-metadata", target, "--source", "meta:test", *args)

        def tokens(group, target):
            return api(group, "get", target)[group]["tokens"]

        def events():
            return api("events", "list", "--limit", "512")["events"]

        def poll(get, predicate, timeout=10):
            deadline = time.monotonic() + timeout
            while True:
                value = get()
                if predicate(value):
                    return value
                assert time.monotonic() < deadline, value
                time.sleep(0.05)

        def wait_off():
            deadline = time.monotonic() + 10
            while time.monotonic() < deadline:
                result = subprocess.run([EXE, "workspace", "list"], env=env, capture_output=True, timeout=10)
                if result.returncode:
                    return
                time.sleep(0.05)
            raise AssertionError("isolated server did not stop")

        try:
            workspace = api("workspace", "create", "metadata.project:中文", "--cwd", str(root))["workspace"]
            wid = workspace["workspace_id"]
            pane = workspace["tabs"][0]["active_pane"]
            if WIN:
                assert api("pane", "wait-output", pane, "--regex", r"PS [^\r\n]*>", "--timeout", "15000")["matched"]
            report("workspace", wid, "--token", "branch=main", "--token", "build=starting", "--seq", "1")
            report("workspace", "metadata.project:中文", "--token", "build=running", "--seq", "2")
            assert tokens("workspace", wid) == {"branch": "main", "build": "running"}
            report("pane", pane, "--token", "task=  build 中文  ", "--token", "literal=a=b c", "--seq", "1")
            assert tokens("pane", pane) == {"task": "build 中文", "literal": "a=b c"}
            report("pane", pane, "--clear-token", "literal", "--token", "empty=  ", "--seq", "2")
            assert tokens("pane", pane) == {"task": "build 中文"}
            listed = api("workspace", "list")["workspaces"]
            assert next(w for w in listed if w["workspace_id"] == wid)["tokens"]["branch"] == "main"
            snapshot = api("api", "snapshot")
            snap_workspace = next(w for w in snapshot["sessions"] if w["workspace_id"] == wid)
            assert snap_workspace["tokens"] == {"branch": "main", "build": "running"}, snapshot
            snap_pane = next(p for t in snap_workspace["windows"] for p in t["panes"]
                             if p["pane_id"] == pane)
            assert snap_pane["tokens"] == {"task": "build 中文"}, snapshot
            other = api("workspace", "create", "metadata.other", "--cwd", str(root))["workspace"]
            moved = api("workspace", "move", wid, "--index", "2")["workspaces"]
            assert [w["workspace_id"] for w in moved] == [other["workspace_id"], wid], moved
            assert moved[0]["tokens"] == {} and moved[1]["tokens"] == snap_workspace["tokens"], moved
            assert tokens("pane", pane) == snap_pane["tokens"]
            print("ok native pane/workspace patch, clear, normalization, list and snapshot", flush=True)

            before = [e for e in events() if e["kind"] == "pane.metadata_updated"][-1]["cursor"]
            stale = report("pane", pane, "--token", "task=stale", "--seq", "2")
            assert stale["accepted"] is False and stale["changed"] is False
            assert tokens("pane", pane)["task"] == "build 中文"
            assert [e for e in events() if e["kind"] == "pane.metadata_updated"][-1]["cursor"] == before
            report("pane", pane, "--clear-token", "task", "--seq", "3")
            assert report("pane", pane, "--token", "task=resurrected", "--seq", "2")["accepted"] is False
            assert not tokens("pane", pane)
            print("ok stale reports have no effect or event and cannot resurrect cleared tokens", flush=True)

            # Expiry must run without terminal output, including workspace tokens.
            report("pane", pane, "--token", "short=temporary", "--ttl-ms", "500", "--seq", "4")
            report("pane", pane, "--token", "kept=persistent", "--seq", "5")
            report("workspace", wid, "--token", "transient=temporary", "--ttl-ms", "500", "--seq", "3")
            expired = poll(events, lambda values: any(e["kind"] == "workspace.metadata_updated" and
                e["data"].get("reason") == "expired" for e in values))
            assert any(e["kind"] == "pane.metadata_updated" and e["data"].get("reason") == "expired" for e in expired)
            assert tokens("pane", pane) == {"kept": "persistent"}
            assert tokens("workspace", wid) == {"branch": "main", "build": "running"}
            assert report("workspace", wid, "--token", "transient=stale", "--seq", "3")["accepted"] is False
            print("ok autonomous per-key expiry, unchanged keys and retained sequence high-water marks", flush=True)

            api("pane", "report-agent", pane, "--source", "lifecycle", "--agent", "test-agent", "--state", "working", "--seq", "1")
            original = api("agent", "get", pane)["agent"]
            waiter = subprocess.Popen([EXE, "agent", "wait", pane, "--until", "idle", "--timeout", "15000"],
                                      env=env, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                                      text=True, encoding="utf-8")
            pending.append(waiter)
            report("pane", pane, "--token", "state=idle", "--token", "task=done", "--ttl-ms", "100", "--seq", "6")
            poll(lambda: tokens("pane", pane), lambda values: "state" not in values)
            assert waiter.poll() is None, "display metadata completed an agent lifecycle wait"
            current = api("agent", "get", pane)["agent"]
            assert current["agent"] == original["agent"], (original, current)
            assert current["status"] == "working"
            assert current["tokens"] == {"kept": "persistent"}
            api("pane", "report-agent", pane, "--source", "lifecycle", "--agent", "test-agent", "--state", "idle", "--seq", "2")
            out, err = waiter.communicate(timeout=20)
            assert waiter.returncode == 0, (out, err)
            print("ok metadata and expiry never replace an agent or satisfy its wait", flush=True)

            # Bridge access uses the same CLI contract, without shell parsing.
            message = {"id": "metadata", "method": "workspace.report-metadata", "params": {"args": [
                wid, "--source", "bridge", "--token", "bridge=literal $value; 中文", "--seq", "0"]}}
            bridged = subprocess.run([EXE, "api", "serve"], input=json.dumps(message) + "\n", env=env,
                                     capture_output=True, text=True, encoding="utf-8", timeout=20)
            assert bridged.returncode == 0, bridged.stderr
            response = json.loads(bridged.stdout)
            assert response["id"] == "metadata" and response["result"]["accepted"]
            assert tokens("workspace", wid)["bridge"] == "literal $value; 中文"
            assert "workspace.move" in json.loads(run("api", "schema").stdout)["methods"]
            print("ok JSON-lines metadata bridge preserves literal arguments", flush=True)

            bounded = api("tab", "create", "capacity", "--workspace", wid)["tab"]["active_pane"]
            for batch in range(2):
                flags = [word for n in range(batch * 16, (batch + 1) * 16) for word in ("--token", f"k{n}=v")]
                report("pane", bounded, *flags, "--seq", str(batch))
            error = run("pane", "report-metadata", bounded, "--source", "meta:test", "--token", "k0=changed",
                        "--token", "overflow=x", "--seq", "2", code=1)
            assert json.loads(error.stderr)["error"]["code"] == "metadata_key_limit"
            assert tokens("pane", bounded)["k0"] == "v" and len(tokens("pane", bounded)) == 32
            assert report("pane", bounded, "--clear-token", "k0", "--token", "replacement=x", "--seq", "2")["accepted"]
            for invalid in [("--ttl-ms", "0"), ("--ttl-ms", "86400001"), ("--seq", "-1")]:
                run("pane", "report-metadata", pane, "--source", "invalid", "--token", "k=v", *invalid, code=2)
            api("pane", "close", bounded)
            missing = run("pane", "report-metadata", bounded, "--source", "test", "--token", "x=y", code=1)
            assert json.loads(missing.stderr)["error"]["code"] == "pane_not_found"
            print("ok capacity rejection is atomic, seq remains usable and closed targets reject reports", flush=True)

            api("server", "stop")
            wait_off()
            api("workspace", "create", "restart-anchor", "--cwd", str(root))
            restored = api("workspace", "get", wid)["workspace"]
            assert restored["tokens"] == {}, restored
            assert all(p["tokens"] == {} for t in restored["tabs"] for p in t["panes"])
            assert report("workspace", wid, "--token", "fresh=ok", "--seq", "0")["accepted"]
            print("ok cold restart preserves workspace layout and clears tokens/sequence state", flush=True)
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

    print("PASS metadata smoke", flush=True)


if __name__ == "__main__":
    main()
