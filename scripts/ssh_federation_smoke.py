#!/usr/bin/env python3
"""Real OpenSSH loopback, real Crew daemons, and terminal chooser regressions.

Linux only. Requires ssh, sshd, ssh-keygen and permission to start a loopback sshd.
The test uses temporary host/user keys, an exact pinned host key, separate Crew
sockets, config/state files and a temporary machine catalog. It never contacts an
external host or changes the user's SSH config, keys, or existing Crew servers.
Run: python scripts/ssh_federation_smoke.py /absolute/path/to/crew
"""
import argparse
import concurrent.futures
import hashlib
import json
import os
from pathlib import Path
import select
import shlex
import shutil
import signal
import socket
import struct
import subprocess
import sys
import tempfile
import time
import uuid


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("binary", nargs="?", default="target/debug/crew")
    parser.add_argument("--logs", type=Path)
    parser.add_argument("--fixture-user", help="Root only: run the isolated fixture as an existing unprivileged user")
    options = parser.parse_args()
    if not sys.platform.startswith("linux"):
        raise SystemExit("NOT_RUN: real loopback SSH/PTY harness requires Linux")
    import fcntl
    import pty
    import pwd
    import termios
    exe = Path(options.binary).resolve()
    fixture_user = pwd.getpwuid(os.geteuid()).pw_name
    print(json.dumps({"harness": "ssh_federation_smoke", "argv": sys.argv,
                      "user": fixture_user, "uid": os.geteuid(),
                      "requested_fixture_user": options.fixture_user}), flush=True)
    if not exe.is_file():
        raise SystemExit("NOT_RUN: Crew binary does not exist: " + str(exe))
    if options.fixture_user:
        if os.geteuid() != 0:
            raise SystemExit("NOT_RUN: --fixture-user requires root; run without it as your current user")
        identity = pwd.getpwnam(options.fixture_user)
        if identity.pw_uid == 0:
            raise SystemExit("NOT_RUN: --fixture-user must name an existing unprivileged user")
        launcher = shutil.which("runuser")
        if launcher is None:
            raise SystemExit("NOT_RUN: --fixture-user requires runuser")
        # Copy only this script and the binary into a disposable owned directory.
        # Never edit the existing account, home, shell, SSH keys or SSH config.
        with tempfile.TemporaryDirectory(prefix="crew-ssh-user-") as temporary:
            isolated = Path(temporary)
            for source, name in ((exe, "crew"), (Path(__file__).resolve(), "smoke.py")):
                target = isolated / name
                shutil.copy2(source, target)
                os.chown(target, identity.pw_uid, identity.pw_gid)
            os.chown(isolated, identity.pw_uid, identity.pw_gid)
            child_logs = isolated / "logs"
            child = subprocess.run([launcher, "-u", identity.pw_name, "--", sys.executable,
                                    str(isolated / "smoke.py"), str(isolated / "crew"),
                                    "--logs", str(child_logs)], cwd=isolated)
            if options.logs and child_logs.exists():
                shutil.copytree(child_logs, options.logs, dirs_exist_ok=True)
        raise SystemExit(child.returncode)
    for tool in ("ssh", "ssh-keygen", "sshd"):
        if shutil.which(tool) is None and not (tool == "sshd" and Path("/usr/sbin/sshd").exists()):
            raise SystemExit("NOT_RUN: missing " + tool)
    if not Path("/run/sshd").is_dir():
        raise SystemExit("NOT_RUN: OpenSSH requires the standard /run/sshd directory")
    checks = []

    def passed(name):
        checks.append(name)
        print("ok", name, flush=True)

    with tempfile.TemporaryDirectory(prefix="crew-ssh-") as tmp:
        root = Path(tmp)
        base_env = {k: v for k, v in os.environ.items()
                    if k not in ("CREW", "TMUXLIKE") and not k.startswith(("CREW_", "TMUXLIKE_"))}
        base_env.update(CREW_LANG="en", TERM="xterm-256color")
        envs = []
        for name in ("local", "remote", "absent", "restart"):
            config = root / (name + ".toml")
            config.write_text('persist = false\nshell = "/bin/sh"\nsidebar = true\n', encoding="utf-8")
            env = dict(base_env, CREW_SOCKET="crew-ssh-" + name + uuid.uuid4().hex,
                       CREW_CONFIG=str(config), CREW_STATE=str(root / (name + ".json")),
                       CREW_LOG=str(root / (name + ".log")), CREW_MACHINES=str(root / "machines.json"))
            envs.append(env)
        local_env, remote_env, absent_env, restart_env = envs
        restart_env["CREW_MACHINES"] = str(root / "restart-machines.json")

        def run(*args, env=local_env, timeout=30):
            return subprocess.run([str(exe), *args], env=env, cwd=root, capture_output=True,
                                  text=True, encoding="utf-8", timeout=timeout)

        def api(*args, env=local_env):
            r = run(*args, env=env)
            assert r.returncode == 0, (args, r.returncode, r.stdout, r.stderr)
            return json.loads(r.stdout)["result"]

        def key(path):
            subprocess.run(["ssh-keygen", "-q", "-t", "ed25519", "-N", "", "-f", str(path)],
                           check=True, capture_output=True)

        for name in ("host", "identity", "wrong-host"):
            key(root / name)
        authorized = root / "authorized_keys"
        authorized.write_bytes((root / "identity.pub").read_bytes())
        authorized.chmod(0o600)
        with socket.socket() as sock:
            sock.bind(("127.0.0.1", 0))
            port = sock.getsockname()[1]
        with socket.socket() as sock:
            sock.bind(("127.0.0.1", 0))
            offline_port = sock.getsockname()[1]
        known_hosts = root / "known_hosts"
        host_key = " ".join((root / "host.pub").read_text().split()[:2])
        pinned = f"[127.0.0.1]:{port} {host_key}\n"
        known_hosts.write_text(pinned)
        ssh_config = root / "ssh_config"
        ssh_config.write_text(f'''Host crew-loop
    HostName 127.0.0.1
    Port {port}
    User {fixture_user}
    IdentityFile {root / 'identity'}
    IdentitiesOnly yes
    UserKnownHostsFile {known_hosts}
    GlobalKnownHostsFile /dev/null
    StrictHostKeyChecking yes
    LogLevel ERROR
Host crew-offline
    HostName 127.0.0.1
    Port {offline_port}
    User {fixture_user}
    IdentityFile {root / 'identity'}
    IdentitiesOnly yes
    UserKnownHostsFile {known_hosts}
    GlobalKnownHostsFile /dev/null
    StrictHostKeyChecking yes
    LogLevel ERROR
''')
        ssh_config_hash = hashlib.sha256(ssh_config.read_bytes()).hexdigest()
        server_config = root / "sshd_config"
        server_config.write_text(f'''Port {port}
ListenAddress 127.0.0.1
HostKey {root / 'host'}
PidFile {root / 'sshd.pid'}
AuthorizedKeysFile none
AuthorizedKeysCommand {Path(shutil.which('cat')).resolve()} {authorized}
AuthorizedKeysCommandUser {fixture_user}
PasswordAuthentication no
KbdInteractiveAuthentication no
PubkeyAuthentication yes
AuthenticationMethods publickey
PermitRootLogin prohibit-password
StrictModes yes
UsePAM no
PrintMotd no
AllowUsers {fixture_user}
LogLevel VERBOSE
''')
        sshd_log = (root / "sshd.log").open("w+")
        sshd = subprocess.Popen([shutil.which("sshd") or "/usr/sbin/sshd", "-D", "-e", "-f", str(server_config)],
                                stdout=sshd_log, stderr=sshd_log)
        chooser = None
        master = slave = None
        try:
            deadline = time.monotonic() + 5
            while time.monotonic() < deadline:
                if sshd.poll() is not None:
                    sshd_log.flush()
                    raise AssertionError("sshd failed: " + (root / "sshd.log").read_text())
                try:
                    with socket.create_connection(("127.0.0.1", port), timeout=.2):
                        break
                except OSError:
                    time.sleep(.025)
            else:
                raise AssertionError("loopback sshd did not listen")
            local = api("workspace", "create", "local-work")["workspace"]
            remote = api("workspace", "create", "remote-work", env=remote_env)["workspace"]
            lp = local["tabs"][0]["active_pane"]
            rp = remote["tabs"][0]["active_pane"]
            assert local["workspace_id"] == remote["workspace_id"] == "w1"
            api("pane", "report-agent", lp, "--source", "ssh-smoke", "--agent", "codex", "--state", "idle")
            api("pane", "report-agent", rp, "--source", "ssh-smoke", "--agent", "claude", "--state", "blocked", env=remote_env)
            binary_dir = root / "binary space ' quote"
            binary_dir.mkdir()
            binary = binary_dir / "crew"
            binary.symlink_to(exe)
            remote_profile = api("machine", "add", "crew-loop", "--label", "Build one", "--binary", str(binary),
                                 "--socket", remote_env["CREW_SOCKET"], "--ssh-config", str(ssh_config))["machine"]
            mid = remote_profile["id"]
            assert api("--machine", mid, "workspace", "list")["workspaces"][0]["name"] == "remote-work"
            passed("real SSH + quoted remote executable + existing remote daemon")

            injection = root / "MUST_NOT_EXIST"
            name = f"remote 中文 ' $(touch {injection}) ; &"
            api("--machine", "Build one", "workspace", "rename", "w1", name)
            assert not injection.exists()
            assert api("workspace", "get", "w1")["workspace"]["name"] == "local-work"
            assert api("--machine", mid, "workspace", "get", "w1")["workspace"]["name"] == name
            api("--machine", mid, "workspace", "rename", "w1", "remote-work")
            passed("stdin JSON data is not shell code and machine-local IDs stay isolated")

            # Absent server rejects even a create operation; no implicit daemon spawn.
            missing = api("machine", "add", "crew-loop", "--label", "No daemon", "--binary", str(binary),
                          "--socket", absent_env["CREW_SOCKET"], "--ssh-config", str(ssh_config), "--no-probe")["machine"]["id"]
            r = run("--machine", missing, "workspace", "create", "must-not-start")
            assert r.returncode == 1 and "server_not_running" in r.stderr, r
            assert run("workspace", "list", env=absent_env).returncode != 0
            passed("remote create cannot auto-start a missing server")

            offline = api("machine", "add", "crew-offline", "--label", "Offline", "--ssh-config", str(ssh_config), "--no-probe")["machine"]["id"]
            start = time.monotonic()
            merged = api("federation", "list", "--timeout-ms", "1000", "--json")
            assert time.monotonic() - start < 4
            assert merged["complete"] is False
            assert {(w["machine_id"], w["workspace_id"]) for w in merged["workspaces"]} == {("local", "w1"), (mid, "w1")}
            assert {a["machine_id"] for a in merged["agents"]} == {"local", mid}
            assert {m["machine_id"] for m in merged["machines"] if m["state"] == "unavailable"} == {missing, offline}
            epochs = {m["machine_id"]: m["data"]["epoch"] for m in merged["machines"] if m["state"] == "reachable"}
            assert epochs == {"local": api("events", "list")["epoch"],
                              mid: api("events", "list", env=remote_env)["epoch"]}
            assert all(w["machine_epoch"] == epochs[w["machine_id"]] for w in merged["workspaces"])
            assert all(a["machine_epoch"] == epochs[a["machine_id"]] for a in merged["agents"])
            passed("combined workspace/agent views preserve healthy machines beside offline hosts")

            r = run("--machine", mid, "--timeout-ms", "500", "pane", "wait-output", rp, "--regex", "NEVER_SSH_MARKER", "--timeout", "20000")
            assert r.returncode == 1 and "not retried" in r.stderr, r
            assert api("--machine", mid, "workspace", "list")["workspaces"]
            passed("bounded transport timeout does not prevent later independent requests")

            # Real OpenSSH exits while a ProxyCommand descendant still owns its
            # stderr pipe. No network is used by this separate failure fixture.
            proxy_pid = root / "proxy-descendant.pid"
            proxy = root / "proxy.py"
            proxy.write_text("import subprocess\nfrom pathlib import Path\n"
                             "child = subprocess.Popen(['/bin/sleep', '30'], "
                             "stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL)\n"
                             f"Path({str(proxy_pid)!r}).write_text(str(child.pid))\n")
            proxy_config = root / "proxy_config"
            proxy_config.write_text("Host crew-proxy-cleanup\n HostName 127.0.0.1\n"
                                    " BatchMode yes\n StrictHostKeyChecking yes\n"
                                    f" ProxyCommand {shlex.quote(sys.executable)} {shlex.quote(str(proxy))}\n")
            proxy_machine = api("machine", "add", "crew-proxy-cleanup", "--no-probe",
                                "--ssh-config", str(proxy_config))["machine"]["id"]
            try:
                start = time.monotonic()
                result = run("--machine", proxy_machine, "--timeout-ms", "1000", "workspace", "list")
                elapsed = time.monotonic() - start
                pid = int(proxy_pid.read_text())

                def descendant_state():
                    try:
                        stat = Path(f"/proc/{pid}/stat").read_text()
                        return stat[stat.rfind(")") + 2:].split()[0]
                    except FileNotFoundError:
                        return "gone"

                deadline = time.monotonic() + .5
                while descendant_state() not in ("Z", "X", "gone") and time.monotonic() < deadline:
                    time.sleep(.01)
                state = descendant_state()
                (root / "proxy-cleanup.json").write_text(json.dumps({
                    "transport": "real OpenSSH ProxyCommand", "network": False,
                    "exit_code": result.returncode, "elapsed_seconds": elapsed,
                    "stderr": result.stderr, "descendant_pid": pid,
                    "descendant_state": state, "descendant_alive_after_return": state not in ("Z", "X", "gone"),
                }, indent=2) + "\n")
                assert result.returncode == 1 and "transport stderr did not close" in result.stderr, result
                assert elapsed < 3 and state in ("Z", "X", "gone"), (elapsed, pid, state)
            finally:
                if proxy_pid.exists():
                    try:
                        os.kill(int(proxy_pid.read_text()), signal.SIGKILL)
                    except ProcessLookupError:
                        pass
                api("machine", "remove", proxy_machine)
            passed("exited OpenSSH transport cleans a ProxyCommand descendant holding stderr")

            wrong = " ".join((root / "wrong-host.pub").read_text().split()[:2])
            known_hosts.write_text(f"[127.0.0.1]:{port} {wrong}\n")
            r = run("--machine", mid, "workspace", "list")
            assert r.returncode == 1 and ("HOST IDENTIFICATION" in r.stderr or "Host key verification failed" in r.stderr), r
            known_hosts.write_text(pinned)
            assert api("machine", "reconnect", mid)["complete"] is True
            assert hashlib.sha256(ssh_config.read_bytes()).hexdigest() == ssh_config_hash
            passed("changed host key is rejected; explicit reconnect recovers without altering SSH config")

            # Concurrent registry changes must not lose a profile or reuse an id.
            def add(i):
                return api("machine", "add", "crew-offline", "--label", f"concurrent-{i}", "--no-probe")["machine"]["id"]
            with concurrent.futures.ThreadPoolExecutor(max_workers=6) as pool:
                concurrent_ids = list(pool.map(add, range(6)))
            assert len(set(concurrent_ids)) == 6
            saved = api("machine", "list")["machines"]
            assert len(saved) == 9
            for ident in concurrent_ids:
                api("machine", "remove", ident)
            passed("serialized atomic catalog updates preserve concurrent machine additions")

            api("machine", "disable", mid)
            assert run("--machine", mid, "workspace", "list").returncode != 0
            assert api("workspace", "list", env=remote_env)["workspaces"]
            api("machine", "enable", mid)
            passed("disable changes routing only and keeps remote processes alive")

            # Real PTY: choose the second (remote) workspace, attach via ssh -tt,
            # type into that remote shell, detach and return to the chooser.
            api("pane", "run", rp, "printf '%s\\n' 'REMOTE_ATTACH_READY'", env=remote_env)
            api("pane", "wait-output", rp, "--regex", "^REMOTE_ATTACH_READY$", "--timeout", "10000", env=remote_env)
            master, slave = pty.openpty()
            fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack("HHHH", 40, 140, 0, 0))
            initial_tty = termios.tcgetattr(slave)

            def controlling_tty():
                os.setsid()
                fcntl.ioctl(slave, termios.TIOCSCTTY, 0)

            chooser = subprocess.Popen([str(exe), "federation", "watch", "--timeout-ms", "1000"], env=local_env,
                                       cwd=root, stdin=slave, stdout=slave, stderr=slave, preexec_fn=controlling_tty)
            transcript = bytearray()

            def until(needle, timeout=10):
                end = time.monotonic() + timeout
                segment = bytearray()
                while time.monotonic() < end:
                    if select.select([master], [], [], .1)[0]:
                        data = os.read(master, 65536)
                        segment.extend(data)
                        transcript.extend(data)
                        if needle.encode() in segment:
                            return
                        if needle == "REMOTE_ATTACH_READY" and b"Attach failed" in segment:
                            raise AssertionError(("native attachment failed", segment.decode(errors="replace")[-8000:]))
                    if chooser.poll() is not None:
                        raise AssertionError(("chooser exited", chooser.returncode, segment.decode(errors="replace")))
                raise AssertionError(("missing chooser output", needle, segment.decode(errors="replace")[-6000:]))

            until("remote-work")
            os.write(master, b"\x1b[B")
            until(">   w1  remote-work")
            cached_at = time.monotonic()
            api("workspace", "rename", "w1", "remote-renamed", env=remote_env)
            replacement = api("workspace", "create", "remote-work", env=remote_env)["workspace"]
            replacement_pane = replacement["tabs"][0]["active_pane"]
            assert time.monotonic() - cached_at < 10  # before the selector's 15-second refresh
            os.write(master, b"\r")
            until("REMOTE_ATTACH_READY")
            os.write(master, b"printf '%s\\n' 'DASH_REMOTE_ONLY'\r")
            api("pane", "wait-output", rp, "--regex", "^DASH_REMOTE_ONLY$", "--timeout", "10000", env=remote_env)
            local_read = run("pane", "read", lp)
            assert "DASH_REMOTE_ONLY" not in local_read.stdout
            assert "DASH_REMOTE_ONLY" not in run("pane", "read", replacement_pane, env=remote_env).stdout
            passed("cached selector uses the original remote workspace ID after rename and name reuse")
            os.write(master, b"\x02d")
            until("Returned from " + mid)
            until("0 connection checks pending")
            api("workspace", "close", replacement["workspace_id"], env=remote_env)
            api("workspace", "rename", "w1", "remote-work", env=remote_env)
            os.write(master, b"r")
            until("Refreshing each machine independently")
            os.write(master, b"q")
            assert chooser.wait(timeout=5) == 0
            assert termios.tcgetattr(slave) == initial_tty
            passed("real federation PTY chooser attaches remote input, detaches, refreshes and restores terminal")

            # A removed ID must fail, even if another workspace now has its name.
            # An empty machine catalog makes this second selector local-only.
            # Keep this daemon alive: with persist=false, closing its last workspace
            # can stop it, and a new daemon is allowed to start its ID counter over.
            keeper = api("workspace", "create", "local-keeper")["workspace"]

            def assert_keeper_present():
                kept = api("workspace", "get", keeper["workspace_id"])["workspace"]
                assert kept["name"] == "local-keeper" and kept["workspace_id"] == keeper["workspace_id"]
                assert kept["tabs"][0]["active_pane"] == keeper["tabs"][0]["active_pane"]

            assert_keeper_present()
            while select.select([master], [], [], 0)[0]:
                transcript.extend(os.read(master, 65536))
            deleted_env = dict(local_env, CREW_MACHINES=str(root / "deleted-machines.json"))
            chooser = subprocess.Popen([str(exe), "federation", "watch", "--timeout-ms", "1000"], env=deleted_env,
                                       cwd=root, stdin=slave, stdout=slave, stderr=slave, preexec_fn=controlling_tty)
            until("0 connection checks pending")
            until(">   w1  local-work")
            cached_at = time.monotonic()
            api("workspace", "close", local["workspace_id"])
            assert_keeper_present()
            replacement = api("workspace", "create", "local-work")["workspace"]
            assert replacement["workspace_id"] != local["workspace_id"]
            assert_keeper_present()
            assert time.monotonic() - cached_at < 10
            os.write(master, b"\r")
            until("Attach failed with exit 1")
            assert api("workspace", "get", replacement["workspace_id"])["workspace"]["attached"] == 0
            assert_keeper_present()
            os.write(master, b"q")
            assert chooser.wait(timeout=5) == 0
            assert termios.tcgetattr(slave) == initial_tty
            passed("removed selected ID refuses attachment despite workspace name reuse")

            # A cold daemon restart may reuse both ID and name. The cached
            # selector must carry its observed epoch into the daemon's attach
            # lock, not pair old rows with a fresh epoch in a preflight request.
            while select.select([master], [], [], 0)[0]:
                transcript.extend(os.read(master, 65536))
            first = api("workspace", "create", "restart-work", env=restart_env)["workspace"]
            initial_epoch = api("events", "list", env=restart_env)["epoch"]
            chooser = subprocess.Popen([str(exe), "federation", "watch", "--timeout-ms", "1000"], env=restart_env,
                                       cwd=root, stdin=slave, stdout=slave, stderr=slave, preexec_fn=controlling_tty)
            until("0 connection checks pending")
            until(">   w1  restart-work")
            cached_at = time.monotonic()
            api("workspace", "close", first["workspace_id"], env=restart_env)
            deadline = time.monotonic() + 5
            while run("workspace", "list", env=restart_env).returncode == 0:
                assert time.monotonic() < deadline, "old restart fixture daemon did not exit"
                time.sleep(.02)
            replacement = api("workspace", "create", "restart-work", env=restart_env)["workspace"]
            replacement_epoch = api("events", "list", env=restart_env)["epoch"]
            assert first["workspace_id"] == replacement["workspace_id"] == "w1"
            assert initial_epoch != replacement_epoch
            assert time.monotonic() - cached_at < 10
            restart_record = {"selected_workspace_id": first["workspace_id"], "selected_epoch": initial_epoch,
                              "replacement_workspace_id": replacement["workspace_id"],
                              "replacement_epoch": replacement_epoch, "rejected": False}
            (root / "restart-guard.json").write_text(json.dumps(restart_record, indent=2) + "\n")
            os.write(master, b"\r")
            until("Attach failed with exit 1")
            assert b"server changed since selection" in transcript
            assert api("workspace", "get", "w1", env=restart_env)["workspace"]["attached"] == 0
            os.write(master, b"q")
            assert chooser.wait(timeout=5) == 0
            assert termios.tcgetattr(slave) == initial_tty
            restart_record.update(rejected=True, replacement_attached=0, terminal_restored=True)
            (root / "restart-guard.json").write_text(json.dumps(restart_record, indent=2) + "\n")
            passed("cached selector rejects a restarted daemon despite workspace ID and name reuse")

            original = (root / "machines.json").read_bytes()
            (root / "machines.json").write_text('{"broken":', encoding="utf-8")
            bad = (root / "machines.json").read_bytes()
            assert run("machine", "add", "bad", "--no-probe").returncode != 0
            assert (root / "machines.json").read_bytes() == bad
            (root / "machines.json").write_bytes(original)
            api("machine", "remove", mid)
            assert api("workspace", "get", "w1", env=remote_env)["workspace"]["name"] == "remote-work"
            passed("invalid catalog is preserved; removing a machine leaves remote workspace alive")
            print(json.dumps({"result":"PASS", "simulation":False, "platform":sys.platform,
                              "transport":"OpenSSH loopback with pinned host key", "checks":checks}, ensure_ascii=False), flush=True)
        finally:
            if options.logs:
                options.logs.mkdir(parents=True, exist_ok=True)
                sshd_log.flush()
                for name in ("sshd.log", "local.log", "remote.log", "absent.log", "restart.log",
                             "proxy-cleanup.json", "restart-guard.json"):
                    if (root / name).is_file():
                        shutil.copyfile(root / name, options.logs / name)
                if "transcript" in locals():
                    (options.logs / "chooser-transcript.bin").write_bytes(transcript)
                (options.logs / "result.json").write_text(json.dumps({
                    "result": "FAIL" if sys.exc_info()[0] else "PASS", "simulation": False,
                    "checks": checks, "error": str(sys.exc_info()[1]) if sys.exc_info()[0] else None,
                    "platform": sys.platform, "binary": str(exe),
                    "argv": sys.argv, "fixture_user": fixture_user, "fixture_uid": os.geteuid(),
                    "binary_sha256": hashlib.sha256(exe.read_bytes()).hexdigest(),
                }, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
            if chooser is not None and chooser.poll() is None:
                chooser.kill()
                chooser.wait(timeout=5)
            for fd in (master, slave):
                if fd is not None:
                    os.close(fd)
            for env in envs:
                run("server", "stop", env=env, timeout=10)
            sshd.terminate()
            try:
                sshd.wait(timeout=5)
            except subprocess.TimeoutExpired:
                sshd.kill()
                sshd.wait(timeout=5)
            sshd_log.close()


if __name__ == "__main__":
    main()
