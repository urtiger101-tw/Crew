#!/usr/bin/env python3
"""Native, isolated Inno Setup install/upgrade/uninstall acceptance.

Uses real published 0.3.0 executables in an explicitly test-only old installer.
Refuses to run if Crew is already registered for the current user. Never kills a
user's daemon: the only server stopped by this test has its own random socket.
"""
import argparse
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import time
import uuid

from windows_installer import compiler, execute, legacy_fixture
from windows_installer_contract import APP_ID, CHECKS, LEGACY, TARGET, installer_name, require, require_static_crt, sha256, validate_record


def wait_until(predicate, message, seconds=15):
    deadline = time.monotonic() + seconds
    while time.monotonic() < deadline:
        if predicate():
            return
        time.sleep(0.05)
    raise ValueError(message)


def native_smoke(args):
    require(sys.platform == "win32", "installer lifecycle smoke requires native Windows; it cannot be simulated")
    import winreg
    import package_release as release
    folder = Path(args.input).resolve()
    records = list(folder.glob("Crew-v*-" + TARGET + "-setup.json"))
    require(len(records) == 1, "expected exactly one installer build manifest")
    record_path = records[0]
    record = json.loads(record_path.read_bytes())
    meta = {key: record[key] for key in ("repository", "tag", "version", "sha", "ci_run_id", "ci_attempt")}
    portable = json.loads((folder / f"Crew-{meta['tag']}-{TARGET}.json").read_bytes())["package"]
    validate_record(folder, record, meta, portable["files"], require_smoke=False)
    require(tuple(int(n) for n in meta["version"].split(".")) > (0, 3, 0), "upgrade smoke needs a genuinely newer version")
    setup = folder / installer_name(meta["tag"])
    logs = Path(args.logs).resolve()
    logs.mkdir(parents=True, exist_ok=True)
    registry_path = r"Software\Microsoft\Windows\CurrentVersion\Uninstall" + "\\" + APP_ID + "_is1"

    def registry(root, key, name):
        try:
            with winreg.OpenKey(root, key, 0, winreg.KEY_READ | winreg.KEY_WOW64_64KEY) as handle:
                return winreg.QueryValueEx(handle, name)
        except FileNotFoundError:
            return None

    def registered(root=winreg.HKEY_CURRENT_USER):
        return registry(root, registry_path, "InstallLocation")

    def paths():
        return (registry(winreg.HKEY_CURRENT_USER, "Environment", "Path"),
                registry(winreg.HKEY_LOCAL_MACHINE, r"SYSTEM\CurrentControlSet\Control\Session Manager\Environment", "Path"))

    require(registered() is None and registered(winreg.HKEY_LOCAL_MACHINE) is None,
            "Crew is already registered; installer smoke must use an isolated Windows user/runner")
    original_paths = paths()
    shortcut = Path(os.environ["APPDATA"]) / "Microsoft/Windows/Start Menu/Programs/Crew/Crew.lnk"
    require(not shortcut.exists(), "Crew shortcut already exists; refusing to overwrite it")
    checks = []

    def passed(name):
        require(name in CHECKS and name not in checks, "invalid or duplicate acceptance check")
        checks.append(name)
        print("PASS installer: " + name, flush=True)

    with tempfile.TemporaryDirectory(prefix="crew-installer-") as temporary:
        root = Path(temporary)
        user_data = root / "設定與資料"
        user_data.mkdir()
        config = user_data / "config.toml"
        state = user_data / "state.json"
        config.write_bytes(b'persist = true\n# User-owned configuration sentinel\n')
        state.write_bytes(b'{"installer-sentinel": "preserve bytes"}\n')
        user_files = {path: path.read_bytes() for path in (config, state)}
        env = {key: value for key, value in os.environ.items()
               if key not in ("CREW", "TMUXLIKE") and not key.startswith(("CREW_", "TMUXLIKE_"))}
        env.update(CREW_CONFIG=str(config), CREW_STATE=str(state), CREW_LANG="en")
        current = None
        daemon_env = None
        daemon_binary = None

        def preserved():
            require(paths() == original_paths, "installer changed the PATH registry value or type")
            require(all(path.read_bytes() == content for path, content in user_files.items()),
                    "installer changed user configuration or state")
            require(registered(winreg.HKEY_LOCAL_MACHINE) is None, "installer registered for all users")

        def installer_run(exe, name, directory=None, *, success=True):
            log = logs / (name + ".log")
            command = [str(exe), "/VERYSILENT", "/SUPPRESSMSGBOXES", "/NORESTART", "/SP-", "/LOG=" + str(log)]
            if directory is not None:
                command.extend(["/DIR=" + str(directory), "/RESTARTEXITCODE=3010"])
            result = subprocess.run(command, env=env, capture_output=True, text=True,
                                    encoding="utf-8", errors="replace", timeout=120)
            # A requested reboot is not success. No continue-on-error or restart fallback.
            require((result.returncode == 0) if success else (result.returncode != 0),
                    f"{name}: unexpected exit {result.returncode}; {result.stdout} {result.stderr}; see {log}")
            require(log.is_file(), "installer did not preserve its native log: " + name)
            preserved()
            return result

        def payload_snapshot(directory):
            return {name: (directory / name).read_bytes() for name in record["files"]}

        def commands(directory, version):
            # This PATH only belongs to a test subprocess; the installer does not register PATH.
            command_env = dict(env, PATH=str(directory) + os.pathsep + os.environ["PATH"])
            for name in ("crew", "tmuxlike"):
                binary = directory / (name + ".exe")
                text = execute([binary, "--version"], env=env, timeout=30)
                require(text in {"crew " + version, "tmuxlike " + version}, "installed command version differs")
                command_env.update(CREW_COMMAND=name, CREW_EXPECTED_BINARY=str(binary))
                # Compare Unicode paths inside PowerShell. Only the ASCII version crosses stdout;
                # Windows PowerShell/native command output code pages vary with the system locale.
                resolved = execute(["powershell", "-NoLogo", "-NoProfile", "-NonInteractive", "-Command",
                                    "$ErrorActionPreference='Stop'; "
                                    "$c=Get-Command -Name $env:CREW_COMMAND -CommandType Application | Select-Object -First 1; "
                                    "if (-not [string]::Equals([IO.Path]::GetFullPath($c.Source), "
                                    "[IO.Path]::GetFullPath($env:CREW_EXPECTED_BINARY), [StringComparison]::OrdinalIgnoreCase)) "
                                    "{ throw 'Test PATH resolved a different executable' }; "
                                    "& $c.Source --version; if ($LASTEXITCODE -ne 0) { throw 'Resolved command failed' }"],
                                   env=command_env, timeout=30)
                require(resolved == text,
                        "installed command lookup result differs")

        def registered_at(directory, version):
            entry = registered()
            require(entry is not None and os.path.samefile(entry[0], directory), "HKCU install directory differs")
            require(registry(winreg.HKEY_CURRENT_USER, registry_path, "DisplayVersion")[0] == version,
                    "HKCU installed version differs")
            require(shortcut.is_file(), "per-user Start Menu shortcut is missing")
            shortcut_env = dict(env, CREW_SHORTCUT=str(shortcut), CREW_EXPECTED_BINARY=str(directory / "crew.exe"))
            target = execute(["powershell", "-NoLogo", "-NoProfile", "-NonInteractive", "-Command",
                              "$ErrorActionPreference='Stop'; "
                              # Keep the raw COM values: quoting/prefixes must not be guessed away.
                              # Escape every UTF-16 code unit so diagnostics survive any console code page.
                              "function Trace-ShortcutValue($name,$value) { "
                              "$kind=if ($null -eq $value) { '<null>' } else { $value.GetType().FullName }; "
                              "$text=[string]$value; "
                              "$escaped=($text.ToCharArray() | ForEach-Object { '\\u{0:X4}' -f [int]$_ }) -join ''; "
                              "[Console]::Error.WriteLine(('CREW_SHORTCUT_VALUE name={0}; null={1}; type={2}; "
                              "utf16_length={3}; escaped_utf16={4}' -f $name,($null -eq $value),$kind,$text.Length,$escaped)) }; "
                              "$s=(New-Object -ComObject WScript.Shell).CreateShortcut($env:CREW_SHORTCUT); $t=$s.TargetPath; "
                              "$w=$s.WorkingDirectory; $a=$s.Arguments; "
                              "$p=[Environment]::GetFolderPath([Environment+SpecialFolder]::UserProfile); "
                              "try { "
                              "if (-not [string]::Equals([IO.Path]::GetFullPath($t), "
                              "[IO.Path]::GetFullPath($env:CREW_EXPECTED_BINARY), [StringComparison]::OrdinalIgnoreCase)) "
                              "{ throw 'Start Menu shortcut points elsewhere' }; "
                              "if ([string]::IsNullOrWhiteSpace($p) -or [string]::IsNullOrWhiteSpace($w) -or "
                              "-not [IO.Directory]::Exists($p) -or -not [string]::Equals([IO.Path]::GetFullPath($w), "
                              "[IO.Path]::GetFullPath($p), [StringComparison]::OrdinalIgnoreCase)) "
                              "{ throw 'Start Menu shortcut working directory is not the user profile' }; "
                              "Write-Output 'CREW_SHORTCUT_OK' "
                              "} catch { "
                              "Trace-ShortcutValue 'Shortcut' $env:CREW_SHORTCUT; "
                              "Trace-ShortcutValue 'TargetPath' $t; "
                              "Trace-ShortcutValue 'ExpectedBinary' $env:CREW_EXPECTED_BINARY; "
                              "Trace-ShortcutValue 'WorkingDirectory' $w; "
                              "Trace-ShortcutValue 'ExpectedWorkingDirectory' $p; "
                              "Trace-ShortcutValue 'Arguments' $a; throw }"],
                             env=shortcut_env, log=logs / f"shortcut-{len(checks):02d}-{version}.log")
            require(target == "CREW_SHORTCUT_OK", "Start Menu shortcut validation did not complete")

        def uninstaller(directory):
            found = list(directory.glob("unins*.exe"))
            require(len(found) == 1, "expected one stable uninstaller after upgrade")
            return found[0]

        def uninstall(directory, label):
            installer_run(uninstaller(directory), label)
            wait_until(lambda: registered() is None and not shortcut.exists()
                       and not any((directory / name).exists() for name in record["files"])
                       and not list(directory.glob("unins*")), "uninstall left registered/owned files behind")
            preserved()

        try:
            fresh = root / "fresh 安裝 space"
            current = fresh
            installer_run(setup, "fresh-install", fresh)
            registered_at(fresh, meta["version"])
            passed("fresh-install")
            commands(fresh, meta["version"])
            passed("command-versions")
            for name, expected in record["files"].items():
                content = (fresh / name).read_bytes()
                require(expected == {"bytes": len(content), "sha256": sha256(content)}, "installed payload differs: " + name)
                if name.endswith(".exe"):
                    require_static_crt(content)
            passed("payload-digests")
            passed("shortcut-and-hkcu")
            extra = fresh / "user-notes.txt"
            extra.write_bytes(b"not owned by this installer\n")
            uninstall(fresh, "fresh-uninstall")
            require(extra.read_bytes() == b"not owned by this installer\n", "fresh uninstall removed an unowned file")
            current = None

            foreign = root / "foreign-directory"
            foreign.mkdir()
            (foreign / "crew.exe").write_bytes(b"existing user file")
            installer_run(setup, "foreign-rejected", foreign, success=False)
            require({path.name for path in foreign.iterdir()} == {"crew.exe"}
                    and (foreign / "crew.exe").read_bytes() == b"existing user file" and registered() is None,
                    "installer changed an unregistered nonempty destination")
            passed("foreign-directory-rejected")

            fixture_dir = root / "legacy-fixture"
            fixture_dir.mkdir()
            old_setup, old_payloads = legacy_fixture(compiler(logs), fixture_dir, logs)
            upgrade = root / "upgrade 安裝 space"
            current = upgrade
            installer_run(old_setup, "legacy-fixture-install", upgrade)
            commands(upgrade, "0.3.0")
            registered_at(upgrade, "0.3.0")
            old_snapshot = payload_snapshot(upgrade)
            require(all(old_snapshot[name] == content for name, content in old_payloads.items()), "legacy fixture payload differs")
            extra = upgrade / "user-notes.txt"
            extra.write_bytes(b"keep this after upgrade and uninstall\n")

            daemon_config = root / "daemon-config.toml"
            daemon_config.write_text('persist = true\nautomatic_rename = false\nshell = "powershell -NoProfile"\n', encoding="utf-8")
            daemon_env = dict(env, CREW_SOCKET="crew-installer-" + uuid.uuid4().hex + ".sock",
                              CREW_CONFIG=str(daemon_config), CREW_STATE=str(root / "daemon-state.json"),
                              CREW_LOG=str(logs / "isolated-daemon.log"))
            daemon_binary = upgrade / "crew.exe"
            created = json.loads(execute([daemon_binary, "workspace", "create", "installer-probe", "--cwd", root],
                                        env=daemon_env, timeout=30))["result"]["workspace"]
            wid = created["workspace_id"]
            epoch = json.loads(execute([daemon_binary, "events", "list"], env=daemon_env))["result"]["epoch"]
            installer_run(setup, "busy-upgrade-rejected", upgrade, success=False)
            require(payload_snapshot(upgrade) == old_snapshot, "rejected upgrade changed installed payload")
            passed("busy-upgrade-rejected")
            installer_run(uninstaller(upgrade), "busy-uninstall-rejected", success=False)
            require(payload_snapshot(upgrade) == old_snapshot and registered() is not None,
                    "rejected uninstall changed installed payload/registration")
            passed("busy-uninstall-rejected")
            alive = json.loads(execute([daemon_binary, "workspace", "get", wid], env=daemon_env, timeout=30))
            require(alive["result"]["workspace"]["workspace_id"] == wid, "installer stopped the running daemon")
            still_running = json.loads(execute([daemon_binary, "events", "list", "--epoch", epoch], env=daemon_env))
            require(still_running["result"]["epoch"] == epoch,
                    "installer restarted the daemon; restored workspaces do not count as survival")
            passed("daemon-survives")
            execute([daemon_binary, "server", "stop"], env=daemon_env, timeout=30)

            def stopped():
                try:
                    with daemon_binary.open("r+b"):
                        return True
                except PermissionError:
                    return False

            wait_until(stopped, "isolated daemon executable remained busy after explicit server stop")
            daemon_env = None
            installer_run(setup, "upgrade-to-current", upgrade)
            registered_at(upgrade, meta["version"])
            commands(upgrade, meta["version"])
            for name, expected in record["files"].items():
                data = (upgrade / name).read_bytes()
                require(expected == {"bytes": len(data), "sha256": sha256(data)}, "upgrade payload differs: " + name)
            require(all((upgrade / name).read_bytes() != old_snapshot[name] for name in ("crew.exe", "tmuxlike.exe")),
                    "upgrade did not change both actual 0.3.0 executables")
            passed("real-0.3.0-upgrade")
            upgraded = payload_snapshot(upgrade)
            installer_run(old_setup, "downgrade-rejected", upgrade, success=False)
            require(payload_snapshot(upgrade) == upgraded, "rejected downgrade changed installed payload")
            registered_at(upgrade, meta["version"])
            passed("downgrade-rejected")
            uninstall(upgrade, "upgrade-uninstall")
            passed("uninstall-owned-files")
            require(extra.read_bytes() == b"keep this after upgrade and uninstall\n", "uninstall removed an unowned file")
            passed("unowned-files-preserved")
            current = None
            preserved()
            passed("path-value-and-type-preserved")
            passed("config-and-state-preserved")
        finally:
            # Cleanup is limited to this test's random socket and temp install directory.
            original_failure = sys.exc_info()[1]
            try:
                if daemon_env is not None and daemon_binary is not None and daemon_binary.exists():
                    result = subprocess.run([str(daemon_binary), "server", "stop"], env=daemon_env,
                                            capture_output=True, timeout=30)
                    if result.returncode:
                        print("Cleanup: isolated server stop returned " + str(result.returncode), file=sys.stderr)
                    else:
                        def released():
                            try:
                                with daemon_binary.open("r+b"):
                                    return True
                            except PermissionError:
                                return False
                        wait_until(released, "cleanup daemon did not release its executable")
                if current is not None and current.exists() and registered() is not None:
                    entry = registered()[0]
                    require(os.path.samefile(entry, current), "cleanup refused an unrelated installation")
                    found = list(current.glob("unins*.exe"))
                    if len(found) == 1:
                        uninstall(current, "cleanup-uninstall")
                preserved()
            except (ValueError, KeyError, OSError, subprocess.SubprocessError) as cleanup_error:
                if original_failure is None:
                    raise
                print("Cleanup also failed (original failure is preserved): " + str(cleanup_error), file=sys.stderr)

    require(set(checks) == set(CHECKS) and len(checks) == len(CHECKS), "required installer acceptance was not executed")
    record["validation"] = {
        "platform": "win32", "simulation": False, "installer_sha256": record["sha256"],
        "previous_version": LEGACY["version"], "previous_portable_sha256": LEGACY["sha256"],
        "installed_version": meta["version"],
        "checks": [{"name": name, "result": "PASS"} for name in CHECKS],
    }
    validate_record(folder, record, meta, portable["files"])
    record_path.write_bytes(release.json_bytes(record))
    (logs / "installer-validation.json").write_bytes(release.json_bytes(record["validation"]))
    print("PASS native Windows installer lifecycle: real 0.3.0 upgrade, busy guards, data preservation and uninstall", flush=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--input", default="installer-dist")
    parser.add_argument("--logs", default="installer-logs")
    args = parser.parse_args()
    try:
        native_smoke(args)
    except (ValueError, KeyError, OSError, subprocess.SubprocessError) as error:
        import traceback
        traceback.print_exc()
        print("Installer lifecycle error: " + str(error), file=sys.stderr)
        for path in sorted(Path(args.logs).glob("*.log")):
            print(f"--- {path.name} ---\n" + "\n".join(path.read_text(encoding="utf-8", errors="replace").splitlines()[-65:]),
                  file=sys.stderr)
        raise SystemExit(1) from error


if __name__ == "__main__":
    main()
