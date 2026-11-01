#!/usr/bin/env python3
"""Offline plugin lifecycle with real commands, files, failures and a Crew daemon.

No third-party code is fetched or executed. Only temporary Python fixtures and
the two manifests embedded in the tested Crew binary are used.
"""
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import time
import uuid


def main():
    binary = str(Path(sys.argv[1] if len(sys.argv) > 1 else "target/release/crew").resolve())
    with tempfile.TemporaryDirectory(prefix="crew-plugin-smoke-") as temp:
        root = Path(temp)
        project = root / "project space 中文"
        project.mkdir()
        fixture = root / "plugin space 中文"
        fixture.mkdir()
        suffix = '.exe' if os.name == 'nt' else ''
        for folder in (fixture, project):
            (folder / 'bin').mkdir()
        shutil.copy2(binary, fixture / 'bin' / ('entry' + suffix))
        (project / 'bin' / ('entry' + suffix)).write_bytes(b'CALLER_EXECUTABLE_MUST_NOT_RUN')
        plugin_home = root / "plugin-data"
        config = root / "config.toml"
        config.write_text("persist = false\nautomatic_rename = false\nshell = " + json.dumps("powershell -NoProfile" if os.name == "nt" else "/bin/sh") + "\n", encoding="utf-8")
        env = {k: v for k, v in os.environ.items() if k != "CREW" and not k.startswith("CREW_")}
        env.update(CREW_PLUGIN_HOME=str(plugin_home), CREW_SOCKET="crew-plugins-" + uuid.uuid4().hex + ".sock", CREW_CONFIG=str(config), CREW_STATE=str(root / "state.json"), CREW_LANG="en")
        checks = []

        def run(*args, ok=True):
            result = subprocess.run([binary, *args], cwd=project, env=env, input="", capture_output=True, text=True, encoding="utf-8", errors="replace", timeout=35)
            if ok:
                assert result.returncode == 0, (args, result.returncode, result.stdout, result.stderr)
            else:
                assert result.returncode != 0, (args, result.stdout, result.stderr)
            return result

        def data(*args):
            return json.loads(run(*args).stdout)["result"]

        def passed(name):
            checks.append(name)
            print("PASS", name, flush=True)

        def manifest(version="1.0.0", minimum="0.5.0"):
            return ("id='fixture.tools'\nname='Fixture Tools'\nversion=" + json.dumps(version)
                + "\nmin_crew_version=" + json.dumps(minimum) + "\nplatforms=['linux','windows','macos']\n"
                + "[[build]]\ncommand=" + json.dumps([sys.executable, str(fixture / "build.py")]) + "\n"
                + "[[actions]]\nid='list'\ntitle='Inspect literal argv and context'\ncommand="
                + json.dumps([sys.executable, str(fixture / "action.py"), "literal ; $(touch INJECTED) ' \" 中文", "{cwd}"]) + "\n"
                + "[[actions]]\nid='fail'\ntitle='Exit failure fixture'\ncommand="
                + json.dumps([sys.executable, "-c", "import sys; print('FAILURE_EVIDENCE'); sys.exit(7)"]) + "\n"
                + "[[actions]]\nid='slow'\ntitle='Timeout fixture'\ncommand="
                + json.dumps([sys.executable, "-c", "import time; time.sleep(10)"]) + "\n"
                + "[[actions]]\nid='relative'\ntitle='Plugin relative program'\ncommand="
                + json.dumps(['./bin/entry' + suffix, '--version']) + '\n')

        (fixture / "action.py").write_text("""import json, os, sys
from pathlib import Path
config = Path(os.environ['CREW_PLUGIN_CONFIG_DIR'])
state = Path(os.environ['CREW_PLUGIN_STATE_DIR'])
(config / 'keep.txt').write_text('CONFIG', encoding='utf-8')
(state / 'keep.txt').write_text('STATE', encoding='utf-8')
print(json.dumps({'argv': sys.argv[1:], 'cwd': str(Path.cwd()), 'id': os.environ['CREW_PLUGIN_ID'], 'action': os.environ['CREW_PLUGIN_ACTION_ID'], 'context': json.loads(os.environ['CREW_PLUGIN_CONTEXT_JSON']), 'config': str(config), 'state': str(state)}, ensure_ascii=False))
""", encoding="utf-8")
        (fixture / "build.py").write_text("from pathlib import Path\nPath('BUILD_RAN').write_text('unexpected')\n", encoding="utf-8")
        manifest_path = fixture / "crew-plugin.toml"
        manifest_path.write_text(manifest(), encoding="utf-8")

        try:
            market = data("plugin", "marketplace", "list", "--json")["plugins"]
            assert {p["id"] for p in market} == {"crew.workspaces", "crew.worktrees"}
            assert len(data("plugin", "marketplace", "search", "worktree", "--json")["plugins"]) == 1
            assert data("plugin", "marketplace", "show", "crew.workspaces")["code_executed"] is False
            run("plugin", "install", "bad/repo/../escape", "--yes", ok=False)
            passed("offline marketplace, searchable catalog and invalid source rejection")

            run("plugin", "install", "crew.workspaces", ok=False)
            assert data("plugin", "list")["plugins"] == []
            installed = data("plugin", "install", "crew.workspaces", "--yes", "--disabled")
            first_root = Path(installed["root"])
            run("plugin", "action", "invoke", "crew.workspaces.snapshot", ok=False)
            data("plugin", "enable", "crew.workspaces")
            failure = run("plugin", "action", "invoke", "crew.workspaces.snapshot", ok=False)
            assert json.loads(failure.stdout)["result"]["success"] is False
            run("workspace", "list", ok=False)
            passed("approval, disabled actions and no implicit daemon startup")

            workspace = data("workspace", "create", "plugin-test")["workspace"]
            snapshot = data("plugin", "action", "invoke", "crew.workspaces.snapshot")
            assert snapshot["success"] is True
            assert workspace["workspace_id"] in snapshot["output"]["stdout"]
            unchanged = data("plugin", "update", "crew.workspaces", "--yes")
            assert unchanged["updates"][0]["status"] == "unchanged"
            again = data("plugin", "install", "crew.workspaces", "--yes")
            assert Path(again["root"]) != first_root and first_root.is_dir()
            passed("bundled action talks to real daemon, unchanged update and retained old installation")

            preview = data("plugin", "inspect", str(fixture))
            assert preview["code_executed"] is False
            run("plugin", "link", str(fixture), ok=False)
            linked = data("plugin", "link", str(manifest_path), "--yes")
            assert not (fixture / "BUILD_RAN").exists()
            action_list = data("plugin", "action", "list", "--plugin", "fixture.tools")["actions"]
            assert len(action_list) == 4 and all(a['available'] for a in action_list)
            relative = data('plugin', 'action', 'invoke', 'fixture.tools.relative')
            assert relative['success'] and relative['output']['stdout'].strip() == run('--version').stdout.strip()
            run("plugin", "action", "invoke", "list", ok=False)
            result = data("plugin", "action", "invoke", "fixture.tools.list")
            payload = json.loads(result["output"]["stdout"])
            assert payload["argv"] == ["literal ; $(touch INJECTED) ' \" 中文", str(project)]
            assert os.path.samefile(payload["cwd"], fixture)
            assert payload["context"]["cwd"] == str(project)
            assert payload["id"] == "fixture.tools" and payload["action"] == "list"
            assert not (fixture / "INJECTED").exists() and not (project / "INJECTED").exists()
            passed("local trust, no link-time build, qualified actions, literal argv and Unicode context")

            manifest_path.write_text(manifest(version="1.0.1"), encoding="utf-8")
            run("plugin", "action", "invoke", "fixture.tools.list", ok=False)
            assert next(p for p in data("plugin", "list")["plugins"] if p["id"] == "fixture.tools")["manifest_unchanged"] is False
            for disabled in (False, True):
                if disabled:
                    data('plugin', 'disable', 'fixture.tools')
                available = data('plugin', 'action', 'list')['actions']
                healthy = [a for a in available if a['plugin_id'] == 'crew.workspaces']
                stale = [a for a in available if a['plugin_id'] == 'fixture.tools']
                assert len(healthy) == 3 and all(a['available'] for a in healthy)
                assert len(stale) == 4 and all(not a['available'] and a['diagnostic'] for a in stale)
                assert data('plugin', 'action', 'invoke', 'crew.workspaces.snapshot')['success']
            data("plugin", "link", str(fixture), "--yes")
            assert data("plugin", "action", "invoke", "fixture.tools.list")["success"]
            manifest_path.write_text(manifest(minimum="999.0.0"), encoding="utf-8")
            run("plugin", "link", str(fixture), "--yes", ok=False)
            assert next(p for p in data("plugin", "list")["plugins"] if p["id"] == "fixture.tools")["version"] == "1.0.1"
            manifest_path.write_text(manifest(version="1.0.1"), encoding="utf-8")
            passed("changed manifests require new approval and rejected version cannot overwrite registry")

            failure = json.loads(run("plugin", "action", "invoke", "fixture.tools.fail", ok=False).stdout)["result"]
            assert failure["output"]["exit_code"] == 7 and "FAILURE_EVIDENCE" in failure["output"]["stdout"]
            started = time.monotonic()
            timeout = json.loads(run("plugin", "action", "invoke", "fixture.tools.slow", "--timeout-ms", "150", ok=False).stdout)["result"]
            assert timeout["output"]["timed_out"] is True and time.monotonic() - started < 8
            logs = data("plugin", "log", "list", "--plugin", "fixture.tools", "--limit", "10")["logs"]
            assert any(log["output"].get("timed_out") for log in logs)
            assert any(log["output"].get("exit_code") == 7 for log in logs)
            passed("real nonzero exit and timeout are retained as failed command records")

            config_dir = Path(linked["config_dir"])
            state_dir = Path(linked["state_dir"])
            data("plugin", "disable", "fixture.tools")
            run("plugin", "action", "invoke", "fixture.tools.list", ok=False)
            data("plugin", "enable", "fixture.tools")
            assert data("plugin", "update", "fixture.tools", "--yes")["updates"][0]["status"] == "skipped_local_link"
            data("plugin", "uninstall", "fixture.tools")
            assert fixture.is_dir() and (config_dir / "keep.txt").read_text() == "CONFIG"
            assert (state_dir / "keep.txt").read_text() == "STATE"
            assert data("plugin", "log", "list", "--plugin", "fixture.tools")["logs"]
            assert {p["id"] for p in data("plugin", "list")["plugins"]} == {"crew.workspaces"}
            passed("enable/disable, local update isolation, uninstall preserves config/state/source/logs")
            print(json.dumps({"result": "PASS", "simulation": False, "checks": checks, "third_party_code_executed": False}), flush=True)
        finally:
            subprocess.run([binary, "kill-server"], cwd=project, env=env, capture_output=True, timeout=10)


if __name__ == "__main__":
    main()
