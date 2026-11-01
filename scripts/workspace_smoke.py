#!/usr/bin/env python3
"""Real PTY/ConPTY regression for workspace/tab IDs, focus, env and legacy persistence.

Uses an isolated socket, config and state. Does not operate on a user's server.
Usage: python scripts/workspace_smoke.py [path-to-crew]
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
EXE = str(Path(sys.argv[1] if len(sys.argv) > 1 else "target/release/crew" + (".exe" if WIN else "")).resolve())


def main():
    with tempfile.TemporaryDirectory(prefix="crew-workspace-") as temp:
        root = Path(temp)
        cwd = root / "repo space 中文"
        cwd.mkdir()
        config = root / "config.toml"
        config.write_text('persist = true\nautomatic_rename = false\nshell = ' + json.dumps("powershell -NoProfile" if WIN else "/bin/sh") + '\n', encoding="utf-8")
        state = root / "state.json"
        log = root / "server.log"
        env = {k: v for k, v in os.environ.items() if k != "CREW" and not k.startswith("CREW_")}
        env.update(CREW_SOCKET="crew-workspace-" + uuid.uuid4().hex + ".sock", CREW_CONFIG=str(config), CREW_STATE=str(state), CREW_LOG=str(log), CREW_LANG="en")

        def run(*args):
            return subprocess.run([EXE, *args], env=env, cwd=root, capture_output=True, text=True, encoding="utf-8", errors="replace", timeout=30)

        def api(*args):
            r = run(*args)
            assert r.returncode == 0, (args, r.returncode, r.stdout, r.stderr)
            return json.loads(r.stdout)["result"]

        def error(code, *args):
            r = run(*args)
            assert r.returncode == 1, (args, r.returncode, r.stderr)
            assert json.loads(r.stderr)["error"]["code"] == code, r.stderr

        def wait_off():
            end = time.monotonic() + 10
            while time.monotonic() < end:
                if run("workspace", "list").returncode != 0:
                    return
                time.sleep(0.05)
            raise AssertionError("server did not stop")

        def output(pid, pattern):
            return api("pane", "wait-output", pid, "--regex", pattern, "--timeout", "15000")

        def wait_for_shell(pid):
            if WIN:
                ready = output(pid, r"PS [^\r\n]*>")
                assert ready.get("matched") is True, f"PowerShell prompt not ready: {ready}"

        def print_command(text):
            return "Write-Output '" + text + "'" if WIN else "printf '%s\\n' '" + text + "'"

        counter = root / "startup's counter 中文.txt"
        startup_values = ['workspace "quoted" 中文 $literal', 'tab "quoted" 中文 $literal']

        def startup_command(text, index):
            if not WIN:
                return print_command(text)
            quote = lambda value: "'" + str(value).replace("'", "''") + "'"
            # A single native argv must preserve quotes, dollars, Unicode and newlines.
            return ("$startupValue = " + quote(startup_values[index]) + "\n"
                    "[IO.File]::AppendAllText(" + quote(counter) + ", $startupValue + [Environment]::NewLine)\n"
                    + print_command(text))

        def startup_once(count):
            if WIN:
                assert counter.read_text(encoding="utf-8").splitlines() == startup_values[:count]

        def env_command():
            return "Write-Output ('ENV=' + $env:WORKSPACE_MARKER)" if WIN else "printf 'ENV=%s\\n' \"$WORKSPACE_MARKER\""

        try:
            first = api("workspace", "create", "project.alpha:api", "--cwd", str(cwd), "--env", "WORKSPACE_MARKER=from workspace", "--command", startup_command("START_COMMAND_OK", 0))["workspace"]
            wid = first["workspace_id"]
            initial = first["tabs"][0]
            output(initial["active_pane"], "^START_COMMAND_OK$")
            startup_once(1)
            wait_for_shell(initial["active_pane"])
            api("pane", "run", initial["active_pane"], env_command())
            output(initial["active_pane"], "^ENV=from workspace$")
            assert os.path.samefile(first["cwd"], cwd)
            print("ok create: real cwd, env and startup command", flush=True)

            other = api("workspace", "create", "background", "--cwd", str(cwd))["workspace"]
            assert api("workspace", "current")["workspace"]["workspace_id"] == wid
            api("tab", "create", "side-review", "--workspace", other["workspace_id"])
            assert api("workspace", "current")["workspace"]["workspace_id"] == wid
            review = api("tab", "create", "review", "--workspace", wid, "--env", "WORKSPACE_MARKER=from tab", "--command", startup_command("TAB_COMMAND_OK", 1))["tab"]
            output(review["active_pane"], "^TAB_COMMAND_OK$")
            startup_once(2)
            assert not review["active"]
            assert api("workspace", "get", wid)["workspace"]["active_tab"] == initial["tab_id"]
            for pane_key in ("CREW_PANE_ID",):
                env[pane_key] = review["active_pane"]
                try:
                    assert api("tab", "current")["tab"]["tab_id"] == review["tab_id"]
                    assert api("tab", "get")["tab"]["tab_id"] == review["tab_id"]
                    assert api("tab", "get", initial["tab_id"])["tab"]["tab_id"] == initial["tab_id"]
                    assert api("tab", "get", "--workspace", wid)["tab"]["tab_id"] == initial["tab_id"]
                    assert api("tab", "current", "--workspace", other["workspace_id"])["tab"]["tab_id"] == other["active_tab"]
                    env[pane_key] = "p999999"
                    error("pane_not_found", "tab", "current")
                    assert api("tab", "get", "--workspace", wid)["tab"]["tab_id"] == initial["tab_id"]
                finally:
                    env.pop(pane_key)
            print("ok background caller tab, legacy pane context and explicit workspace scope", flush=True)
            active = api("tab", "create", "selected", "--workspace", wid, "--focus")["tab"]
            assert active["active"]
            api("tab", "close", initial["tab_id"])
            assert api("workspace", "get", wid)["workspace"]["active_tab"] == active["tab_id"]
            print("ok no-focus, explicit focus and close-before-active regression", flush=True)

            api("workspace", "rename", wid, "renamed.project:中文")
            assert api("workspace", "get", "renamed.project:中文")["workspace"]["workspace_id"] == wid
            api("tab", "rename", review["tab_id"], "review.changed:中文")
            assert api("tab", "get", "review.changed:中文", "--workspace", wid)["tab"]["tab_id"] == review["tab_id"]
            error("name_taken", "workspace", "create", "background", "--cwd", str(cwd))
            before = len(api("workspace", "list")["workspaces"])
            error("invalid_argument", "workspace", "create", "bad-cwd", "--cwd", str(root / "missing"))
            assert len(api("workspace", "list")["workspaces"]) == before
            assert run("workspace", "create", "w999").returncode == 2
            error("tab_not_found", "tab", "get", review["tab_id"], "--workspace", other["workspace_id"])
            print("ok exact names, stable IDs, invalid cwd and scope checks", flush=True)

            wait_for_shell(review["active_pane"])
            api("pane", "run", review["active_pane"], env_command())
            output(review["active_pane"], "^ENV=from tab$")
            split = api("pane", "split", review["active_pane"], "--direction", "right")["pane"]
            wait_for_shell(split["pane_id"])
            api("pane", "run", split["pane_id"], env_command())
            output(split["pane_id"], "^ENV=from tab$")
            inherited = api("tab", "create", "inherited", "--workspace", wid)["tab"]
            wait_for_shell(inherited["active_pane"])
            api("pane", "run", inherited["active_pane"], env_command())
            output(inherited["active_pane"], "^ENV=from workspace$")
            print("ok tab overrides, tab inheritance and split inheritance", flush=True)

            gone = api("workspace", "create", "deleted-high-id", "--cwd", str(cwd))["workspace"]
            highest_tab = gone["tabs"][0]["tab_id"]
            api("workspace", "close", gone["workspace_id"])
            ids_before = {w["workspace_id"]: [t["tab_id"] for t in w["tabs"]] for w in api("workspace", "list")["workspaces"]}
            api("server", "stop")
            wait_off()
            after = api("workspace", "create", "after-restart", "--cwd", str(cwd))["workspace"]
            assert int(after["workspace_id"][1:]) > int(gone["workspace_id"][1:])
            assert int(after["tabs"][0]["tab_id"][1:]) > int(highest_tab[1:])
            for wid_before, tab_ids in ids_before.items():
                restored = api("workspace", "get", wid_before)["workspace"]
                assert [t["tab_id"] for t in restored["tabs"]] == tab_ids
            assert api("workspace", "current")["workspace"]["workspace_id"] == wid
            assert api("workspace", "get", wid)["workspace"]["active_tab"] == active["tab_id"]
            restored_review = api("tab", "get", review["tab_id"])["tab"]
            wait_for_shell(restored_review["active_pane"])
            api("pane", "run", restored_review["active_pane"], env_command())
            output(restored_review["active_pane"], "^ENV=from tab$")
            startup_once(2)
            print("ok startup commands execute once; split and restore do not replay them", flush=True)
            print("ok restart preserves IDs, high-water marks, focus and env", flush=True)

            events = api("events", "list", "--limit", "512")["events"]
            assert any(e["kind"] == "workspace.restored" for e in events)
            assert any(e["kind"] == "workspace.created" for e in events)
            api("server", "stop")
            wait_off()
            legacy = json.loads(state.read_text(encoding="utf-8"))
            for key in ("next_workspace_id", "next_tab_id", "focused_workspace_id"):
                legacy.pop(key, None)
            for workspace in legacy["sessions"]:
                workspace.pop("id", None)
                for tab in workspace["windows"]:
                    tab.pop("id", None)
            state.write_text(json.dumps(legacy, ensure_ascii=False), encoding="utf-8")
            api("workspace", "create", "legacy-anchor", "--cwd", str(cwd))
            migrated = api("workspace", "list")["workspaces"]
            workspace_ids = [w["workspace_id"] for w in migrated]
            tab_ids = [t["tab_id"] for w in migrated for t in w["tabs"]]
            assert len(workspace_ids) == len(set(workspace_ids))
            assert len(tab_ids) == len(set(tab_ids))
            assert any(w["name"] == "renamed.project:中文" for w in migrated)
            print("ok legacy version 1 migration and lifecycle events", flush=True)
            if WIN:
                api("server", "stop")
                wait_off()
                invalid_config = root / "execution-mode.toml"
                invalid_config.write_text('persist = false\nshell = "powershell -NoProfile -Command exit"\n', encoding="utf-8")
                env.update(CREW_SOCKET="crew-startup-mode-" + uuid.uuid4().hex + ".sock",
                           CREW_CONFIG=str(invalid_config), CREW_STATE=str(root / "execution-mode-state.json"))
                error("invalid_argument", "workspace", "create", "must-not-exist", "--cwd", str(cwd), "--command", "Write-Output 'not run'")
                assert api("workspace", "list")["workspaces"] == []
                assert not any(e["kind"] == "pane.created" for e in api("events", "list")["events"])
                print("ok conflicting PowerShell execution mode rejected before pane creation", flush=True)
            print("PASS workspace smoke", flush=True)
        except BaseException:
            if log.exists():
                print(log.read_text(encoding="utf-8", errors="replace")[-5000:], file=sys.stderr)
            raise
        finally:
            run("kill-server")
            wait_off()


if __name__ == "__main__":
    main()
