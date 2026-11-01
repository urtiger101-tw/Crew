#!/usr/bin/env python3
"""Linux R3/R6 regressions against a real Crew server and disposable Git repository.

Usage: python3 worktree_race_linux.py /absolute/path/to/crew
Optional: --case r3|r6 --expect-vulnerable reproduces the corresponding old defect.
The Git wrapper only provides a deterministic test barrier after real git status.
No user repository, socket, state, global Git configuration or network is used.
"""
import argparse
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import time
import uuid


def wait_for(predicate, description, timeout=10):
    end = time.monotonic() + timeout
    while time.monotonic() < end:
        if predicate():
            return
        time.sleep(0.01)
    raise AssertionError("Timed out: " + description)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("binary", type=lambda p: str(Path(p).resolve()))
    parser.add_argument("--case", choices=("r3", "r6", "all"), default="all")
    parser.add_argument("--expect-vulnerable", action="store_true")
    args = parser.parse_args()
    if not sys.platform.startswith("linux"):
        print("SKIP: requires Linux live cwd inspection and executable Git barrier")
        return
    real_git = shutil.which("git")
    assert real_git, "Git is required"
    assert Path(args.binary).is_file(), args.binary

    with tempfile.TemporaryDirectory(prefix="crew-review-worktree-race-") as temporary:
        root = Path(temporary).resolve()
        repository = root / "repository"
        repository.mkdir()
        config = root / "config.toml"
        config.write_text("persist = false\nshell = '/bin/sh'\n", encoding="utf-8")
        bin_dir = root / "bin"
        bin_dir.mkdir()
        enabled = root / "enable-barrier"
        ready = root / "status-completed"
        proceed = root / "continue-remove"
        wrapper = bin_dir / "git"
        wrapper.write_text(
            "#!" + sys.executable + "\n"
            "import pathlib, subprocess, sys, time\n"
            "r = subprocess.run([" + repr(real_git) + ", *sys.argv[1:]])\n"
            "if 'status' in sys.argv[1:] and pathlib.Path(" + repr(str(enabled)) + ").exists():\n"
            "    pathlib.Path(" + repr(str(ready)) + ").write_text('ready')\n"
            "    end = time.monotonic() + 20\n"
            "    while not pathlib.Path(" + repr(str(proceed)) + ").exists():\n"
            "        if time.monotonic() >= end: sys.exit(99)\n"
            "        time.sleep(0.01)\n"
            "sys.exit(r.returncode)\n",
            encoding="utf-8",
        )
        wrapper.chmod(0o755)
        env = {key: value for key, value in os.environ.items()
               if not key.upper().startswith(("CREW", "GIT_"))}
        env.update(CREW_SOCKET="crew-race-" + uuid.uuid4().hex + ".sock",
                   CREW_CONFIG=str(config), CREW_STATE=str(root / "state.json"),
                   CREW_LOG=str(root / "server.log"),
                   PATH=str(bin_dir) + os.pathsep + env.get("PATH", ""))

        def git(*arguments):
            result = subprocess.run([real_git, "-C", str(repository), *arguments], env=env,
                                    capture_output=True, text=True, timeout=20)
            assert result.returncode == 0, result.stderr
            return result

        def cli(*arguments):
            return subprocess.run([args.binary, *arguments], env=env, cwd=root,
                                  capture_output=True, text=True, timeout=20)

        def api(*arguments):
            result = cli(*arguments)
            assert result.returncode == 0, (arguments, result.returncode, result.stdout, result.stderr)
            return json.loads(result.stdout)["result"]

        def error(code, *arguments):
            result = cli(*arguments)
            assert result.returncode == 1, (arguments, result.returncode, result.stdout, result.stderr)
            value = json.loads(result.stderr)["error"]
            assert value["code"] == code, (arguments, value)
            return value

        def new_checkout(path, branch):
            api("worktree", "create", "--repo", str(repository), "--path", str(path),
                "--branch", branch, "--no-open")

        git("init", "-b", "main")
        (repository / "nested").mkdir()
        (repository / "nested" / "tracked").write_text("preserve fixture\n", encoding="utf-8")
        git("add", "nested/tracked")
        git("-c", "user.name=Crew review", "-c", "user.email=review@example.invalid",
            "commit", "-m", "Disposable fixture")
        removal = None
        try:
            sentinel = api("workspace", "create", "sentinel", "--cwd", str(root))["workspace"]
            pane = sentinel["tabs"][0]["active_pane"]
            if args.case in ("r3", "all"):
                checkout = root / "live-cwd"
                new_checkout(checkout, "review-live-cwd")
                proof = root / "cwd-proof"
                api("pane", "run", pane, "cd '" + str(checkout) + "'; pwd > '" + str(proof) + "'")
                wait_for(proof.exists, "shell cwd proof")
                assert Path(proof.read_text().strip()).samefile(checkout)
                if args.expect_vulnerable:
                    assert api("worktree", "remove", "--repo", str(repository), "--path", str(checkout))["removed"]
                    assert not checkout.exists()
                    print("REPRODUCED R3: live shell cwd was removed", flush=True)
                else:
                    error("worktree_in_use", "worktree", "remove", "--repo", str(repository), "--path", str(checkout))
                    assert (checkout / "nested" / "tracked").read_text() == "preserve fixture\n"
                    print("PASS R3: live shell cwd protects checkout and tracked data", flush=True)
                proof.unlink()
                api("pane", "run", pane, "cd '" + str(root) + "'; pwd > '" + str(proof) + "'")
                wait_for(proof.exists, "shell leaving checkout")
                assert Path(proof.read_text().strip()).samefile(root)
                if not args.expect_vulnerable:
                    assert api("worktree", "remove", "--repo", str(repository), "--path", str(checkout))["removed"]
                    print("PASS R3: in-use error releases reservation; removal succeeds after leaving", flush=True)

            if args.case in ("r6", "all"):
                checkout = root / "concurrent-open"
                new_checkout(checkout, "review-concurrent")
                enabled.write_text("enabled")
                removal = subprocess.Popen(
                    [args.binary, "worktree", "remove", "--repo", str(repository), "--path", str(checkout)],
                    env=env, cwd=root, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
                wait_for(ready.exists, "removal passing active check and real Git status")
                if args.expect_vulnerable:
                    opened = api("workspace", "create", "opened-during-remove", "--cwd", str(checkout))["workspace"]
                else:
                    before = api("pane", "list")["panes"]
                    for command in [
                        ("workspace", "create", "opened-during-remove", "--cwd", str(checkout)),
                        ("tab", "create", "opened-during-remove", "--workspace", sentinel["workspace_id"],
                         "--cwd", str(checkout)),
                        ("pane", "split", pane, "--direction", "right", "--cwd", str(checkout / "nested")),
                    ]:
                        result = error("spawn_failed", *command)
                        assert "removal is in progress" in result["message"], result
                    assert len(api("pane", "list")["panes"]) == len(before), "failed creations leaked panes"
                    unaffected = api("workspace", "create", "unrelated", "--cwd", str(root))["workspace"]
                    api("workspace", "close", unaffected["workspace_id"])
                    print("PASS R6: workspace/tab/split rejected under reserved path; unrelated work proceeds", flush=True)
                proceed.write_text("continue")
                stdout, stderr = removal.communicate(timeout=20)
                assert removal.returncode == 0, (stdout, stderr)
                assert json.loads(stdout)["result"]["removed"] and not checkout.exists()
                removal = None
                enabled.unlink()
                if args.expect_vulnerable:
                    assert api("workspace", "get", opened["workspace_id"])["workspace"]["name"] == "opened-during-remove"
                    print("REPRODUCED R6: concurrently opened workspace remains live after checkout removal", flush=True)
                else:
                    new_checkout(checkout, "review-concurrent")
                    opened = api("workspace", "create", "after-removal", "--cwd", str(checkout))["workspace"]
                    api("workspace", "close", opened["workspace_id"])
                    dirty = checkout / "untracked-data"
                    dirty.write_text("preserve dirty data", encoding="utf-8")
                    error("worktree_dirty", "worktree", "remove", "--repo", str(repository), "--path", str(checkout))
                    assert dirty.read_text() == "preserve dirty data"
                    api("workspace", "create", "after-removal-error", "--cwd", str(checkout))
                    print("PASS R6: reservation released after successful removal and dirty-error return", flush=True)
            print("REPRODUCTION COMPLETE (expected vulnerable revision)" if args.expect_vulnerable
                  else "PASS worktree race checks (Linux)", flush=True)
        finally:
            proceed.write_text("continue")
            if removal and removal.poll() is None:
                removal.communicate(timeout=20)
            cli("kill-server")


if __name__ == "__main__":
    main()
