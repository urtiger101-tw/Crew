#!/usr/bin/env python3
"""Real Crew/Git worktree acceptance checks, isolated from user repositories/servers.

Usage: python scripts/worktree_smoke.py [path-to-crew]
Uses Python stdlib, local Git commits and a unique socket/config/state directory.
No network, global Git configuration, forced removal or branch deletion.
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

WIN = os.name == "nt"
EXE = Path(sys.argv[1] if len(sys.argv) > 1 else "target/release/crew" + (".exe" if WIN else "")).resolve()
CHECKS = 0


def check(condition, message):
    if not condition:
        raise AssertionError(message)


def passed(message):
    global CHECKS
    CHECKS += 1
    print("ok " + message, flush=True)


def command(argv, *, env, cwd, expected=0):
    result = subprocess.run([str(x) for x in argv], env=env, cwd=cwd,
                            capture_output=True, text=True, encoding="utf-8",
                            errors="replace", timeout=45)
    check(result.returncode == expected,
          f"{argv}: expected exit {expected}, got {result.returncode}\n"
          f"stdout: {result.stdout}\nstderr: {result.stderr}")
    return result


def main():
    check(EXE.is_file(), f"Crew executable not found: {EXE}")
    check(shutil.which("git"), "Git is required")
    with tempfile.TemporaryDirectory(prefix="crew-worktree-") as temporary:
        root = Path(temporary).resolve()
        env = {k: v for k, v in os.environ.items()
               if not k.upper().startswith(("CREW", "TMUXLIKE", "GIT_"))}
        config = root / "config.toml"
        config.write_text("persist = false\n" + ("" if WIN else "shell = '/bin/sh'\n"), encoding="utf-8")
        env.update(CREW_CONFIG=str(config), CREW_STATE=str(root / "state.json"),
                   CREW_LOG=str(root / "server.log"),
                   CREW_SOCKET=f"crew-worktree-{os.getpid()}-{uuid.uuid4().hex[:8]}.sock")

        def cli(*args, expected=0, cwd=root, use_env=env):
            return command([EXE, *args], env=use_env, cwd=cwd, expected=expected)

        def data(*args, cwd=root, use_env=env):
            result = cli(*args, cwd=cwd, use_env=use_env)
            return json.loads(result.stdout)["result"]

        def error(code, *args, cwd=root, use_env=env):
            result = cli(*args, expected=1, cwd=cwd, use_env=use_env)
            value = json.loads(result.stderr)["error"]
            check(value["code"] == code, f"expected {code}, got {value}")
            return value

        def git(repo, *args):
            return command(["git", "-C", repo, *args], env=env, cwd=root).stdout.strip()

        def init_repo(path):
            path.mkdir()
            git(path, "init", "-b", "main")
            (path / "tracked.txt").write_text("original\n", encoding="utf-8")
            (path / ".gitignore").write_text("ignored-cache\n", encoding="utf-8")
            git(path, "add", "tracked.txt", ".gitignore")
            git(path, "-c", "user.name=Crew smoke", "-c", "user.email=crew@example.invalid",
                "commit", "-m", "Initial fixture")

        repo = root / "repository 中文 with spaces"
        checkout = root / "checkout 中文 with spaces"
        other = root / "other repository"
        init_repo(repo)
        init_repo(other)
        bad_env = None
        try:
            # Keep one unrelated workspace alive while closing test workspaces.
            cli("new", "-d", "-s", "sentinel")
            sentinel_pane = data("pane", "current")["pane"]["pane_id"]
            listing = data("worktree", "list", cwd=repo)
            check(len(listing["worktrees"]) == 1 and listing["worktrees"][0]["primary"], listing)
            check(Path(listing["repository"]).samefile(repo), listing)
            passed("list defaults to caller cwd, preserves Unicode and spaces")

            # Git checkout hooks must not execute implicitly through the Crew API.
            hooks = root / "custom-hooks"
            hooks.mkdir()
            marker = root / "hook-was-run"
            hook = hooks / "post-checkout"
            git(repo, "config", "core.hooksPath", str(hooks))
            hook.write_text("#!/bin/sh\nprintf executed > '" + marker.as_posix().replace("'", "'\\''") + "'\n", encoding="utf-8")
            hook.chmod(0o755)

            result = data("worktree", "create", "--repo", str(repo), "--path", str(checkout),
                          "--branch", "feat/中文", "--name", "worktree-one")
            check(result["created"] and result["workspace"]["name"] == "worktree-one", result)
            check((checkout / "tracked.txt").read_text(encoding="utf-8") == "original\n", result)
            check(git(checkout, "branch", "--show-current") == "feat/中文", result)
            check(not marker.exists(), "checkout hook unexpectedly ran")
            check(data("pane", "current")["pane"]["pane_id"] == sentinel_pane, "create stole focus")
            passed("create commits a real worktree and opens a workspace without Git hooks")

            panes = data("pane", "list", "--session", "worktree-one")["panes"]
            check(len(panes) == 1 and Path(panes[0]["cwd"]).samefile(checkout), panes)
            pid = panes[0]["pane_id"]
            # A command executed by the real PTY verifies its cwd independently of JSON.
            cwd_probe = "(Get-Location).Path | Set-Content -Encoding utf8 cwd-probe.txt" if WIN else "pwd > cwd-probe.txt"
            completion_prefix, completion_token = "CREW_CWD_DONE_", uuid.uuid4().hex
            completion = completion_prefix + completion_token
            # Emit only after the write succeeds and returns. Split literals keep the
            # complete marker out of the echoed command, including wrapped PTY rows.
            if WIN:
                cwd_probe += f"; if ($?) {{ Write-Output ('{completion_prefix}' + '{completion_token}') }}"
            else:
                cwd_probe += f" && printf '%s%s\\n' '{completion_prefix}' '{completion_token}'"
            try:
                if WIN:
                    # A spawned PowerShell process can still be initializing its input reader.
                    # wait-output joins wrapped rows, including long provider-qualified prompts.
                    ready = data("pane", "wait-output", pid, "--regex", r"PS [^\r\n]*>",
                                 "--timeout", "15000")
                    check(ready.get("matched") is True, f"PowerShell prompt not ready: {ready}")
                data("pane", "run", pid, cwd_probe)
                completed = data("pane", "wait-output", pid, "--match", completion, "--timeout", "15000")
                check(completed.get("matched") is True, f"terminal cwd write did not complete: {completed}")
                check((checkout / "cwd-probe.txt").exists(), "terminal did not run at the worktree cwd")
                actual = (checkout / "cwd-probe.txt").read_text(encoding="utf-8-sig").strip()
                check(Path(actual).samefile(checkout), actual)
                (checkout / "cwd-probe.txt").unlink()
            except Exception:
                try:
                    output = cli("pane", "read", pid, "--source", "recent").stdout
                    print(f"cwd probe pane {pid}:\n{output}", file=sys.stderr)
                except Exception as diagnostic_error:
                    print(f"cwd probe pane {pid} could not be read: {diagnostic_error}", file=sys.stderr)
                raise
            passed("terminal process starts at the actual worktree cwd")

            reopened = data("worktree", "open", "--repo", str(repo), "--branch", "feat/中文")
            check(reopened["reused"] and reopened["workspace"]["name"] == "worktree-one", reopened)
            reopened = data("worktree", "open", "--repo", str(repo), "--path", str(checkout), "--focus")
            check(reopened["reused"], reopened)
            check(data("pane", "current")["pane"]["pane_id"] == pid, "open --focus did not focus its pane")
            check(len(data("pane", "list", "--session", "worktree-one")["panes"]) == 1, reopened)
            passed("open by branch/path reuses the workspace; focus/no-focus are respected")

            error("worktree_in_use", "worktree", "remove", "--repo", str(repo), "--path", str(checkout))
            cli("kill-session", "-t", "worktree-one")
            check(checkout.exists(), "closing a workspace removed its checkout")
            passed("active worktree removal refused; closing workspace keeps files")

            for filename, value in [("untracked.txt", "keep me"), ("tracked.txt", "changed"), ("ignored-cache", "precious cache")]:
                path = checkout / filename
                original = path.read_bytes() if path.exists() else None
                path.write_text(value, encoding="utf-8")
                error("worktree_dirty", "worktree", "remove", "--repo", str(repo), "--path", str(checkout))
                check(path.read_text(encoding="utf-8") == value, f"{filename} was damaged")
                if original is None:
                    path.unlink()
                else:
                    path.write_bytes(original)
            passed("modified, untracked and ignored files each prevent removal")

            git(repo, "worktree", "lock", "--reason", "smoke test", str(checkout))
            error("worktree_locked", "worktree", "remove", "--repo", str(repo), "--path", str(checkout))
            git(repo, "worktree", "unlock", str(checkout))
            error("worktree_primary", "worktree", "remove", "--repo", str(repo), "--path", str(repo))
            error("worktree_not_found", "worktree", "remove", "--repo", str(other), "--path", str(checkout))
            passed("locked, primary and foreign worktrees cannot be removed")

            gitfile = checkout / ".git"
            original_pointer = gitfile.read_bytes()
            if WIN:
                print(f"gitfile attributes before mutation: {getattr(gitfile.stat(), 'st_file_attributes', None)!r}", flush=True)

            def write_pointer(contents):
                # Avoid CREATE_ALWAYS with normal attributes, which rejects hidden files on Windows.
                with gitfile.open("r+b") as stream:
                    stream.seek(0)
                    stream.write(contents)
                    stream.truncate()

            try:
                write_pointer(("gitdir: " + str(other / ".git") + "\n").encode("utf-8"))
                error("worktree_mismatch", "worktree", "remove", "--repo", str(repo), "--path", str(checkout))
            finally:
                write_pointer(original_pointer)
                check(gitfile.read_bytes() == original_pointer, "restored .git pointer bytes changed")
                if WIN:
                    print(f"gitfile attributes after restore: {getattr(gitfile.stat(), 'st_file_attributes', None)!r}", flush=True)
            passed("changed .git pointer prevents removal")

            removed = data("worktree", "remove", "--repo", str(repo), "--path", str(checkout))
            check(removed["removed"] and removed["branch_deleted"] is False and not checkout.exists(), removed)
            git(repo, "show-ref", "--verify", "refs/heads/feat/中文")
            passed("clean worktree removal keeps the branch")

            existing = data("worktree", "create", "--repo", str(repo), "--path", str(checkout),
                            "--branch", "feat/中文", "--no-open")
            check(existing["workspace"] is None, existing)
            check(git(checkout, "branch", "--show-current") == "feat/中文", existing)
            data("worktree", "remove", "--repo", str(repo), "--path", str(checkout))
            passed("existing branch checkout and --no-open work")

            rejected = root / "must-not-exist"
            error("invalid_argument", "worktree", "create", "--repo", str(repo), "--path", str(rejected), "--branch=--upload-pack=malicious", "--no-open")
            error("invalid_base", "worktree", "create", "--repo", str(repo), "--path", str(rejected), "--branch", "safe-name", "--base=--help", "--no-open")
            error("invalid_argument", "worktree", "create", "--repo", str(repo), "--path", str(rejected), "--branch", "feat/中文", "--base", "main", "--no-open")
            error("worktree_exists", "worktree", "create", "--repo", str(repo), "--path", str(repo), "--branch", "safe-name", "--no-open")
            error("not_git_repository", "worktree", "list", "--repo", str(root))
            cli("worktree", "remove", "--repo", str(repo), "--path", str(rejected), "--force", expected=2)
            check(not rejected.exists(), "rejected request created a checkout")
            passed("invalid refs/options/repos and existing paths are rejected without side effects")

            events = data("events", "list", "--limit", "512")["events"]
            kinds = {event["kind"] for event in events}
            check({"worktree.created", "worktree.opened", "worktree.removed"} <= kinds, kinds)
            passed("worktree lifecycle events are recorded")

            # A second isolated server has an invalid shell: successful checkout must survive.
            bad_config = root / "bad-shell.toml"
            bad_config.write_text("persist = false\nshell = 'crew-nonexistent-shell-smoke'\n", encoding="utf-8")
            bad_env = dict(env, CREW_CONFIG=str(bad_config), CREW_SOCKET=env["CREW_SOCKET"] + "-bad",
                           CREW_STATE=str(root / "bad-state.json"), CREW_LOG=str(root / "bad-server.log"))
            kept = root / "preserved after spawn failure"
            failed = error("workspace_open_failed", "worktree", "create", "--repo", str(repo),
                           "--path", str(kept), "--branch", "feat/kept", use_env=bad_env)
            check(kept.exists() and (kept / "tracked.txt").exists() and str(kept) in failed["message"], failed)
            data("worktree", "remove", "--repo", str(repo), "--path", str(kept))
            passed("checkout survives terminal spawn failure and its path is reported")
        finally:
            if sys.exc_info()[0] is not None:
                for log in [root / "server.log", root / "bad-server.log"]:
                    if log.exists():
                        print(log.name + ":\n" + log.read_text(encoding="utf-8", errors="replace")[-6000:], file=sys.stderr)
            for cleanup_env in [env, bad_env]:
                if cleanup_env:
                    subprocess.run([str(EXE), "kill-server"], env=cleanup_env, cwd=root,
                                   stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, timeout=15)
        print(f"PASS: {CHECKS} real worktree checks ({sys.platform})", flush=True)


if __name__ == "__main__":
    main()
    if sys.platform.startswith("linux"):
        subprocess.run([sys.executable, str(Path(__file__).with_name("worktree_race_linux.py")),
                        str(EXE)], check=True, timeout=120)
