#!/usr/bin/env python3
"""Run Crew's required native checks and preserve local, source-bound evidence.

This does not call GitHub, publish artifacts, or claim a complete platform matrix.
Python 3.11+ and the repository's pinned Rust toolchain are required.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import platform
import re
import shutil
import stat
import subprocess
import sys
import time
import tomllib
from datetime import datetime, timezone

sys.dont_write_bytecode = True

ROOT = Path(__file__).resolve().parents[1]
SCHEMA = 1
TARGETS = ("x86_64-unknown-linux-gnu", "x86_64-pc-windows-msvc",
           "aarch64-apple-darwin", "x86_64-apple-darwin")
SMOKES = (
    ("legacy-pty", "smoke.py", "tmuxlike"),
    ("api", "api_smoke.py", "crew"),
    ("workspace", "workspace_smoke.py", "crew"),
    ("workspace-move", "workspace_move_smoke.py", "crew"),
    ("metadata", "metadata_smoke.py", "crew"),
    ("worktree", "worktree_smoke.py", "crew"),
    ("agent-identity", "agent_identity_smoke.py", "crew"),
    ("bridge-events", "bridge_events_smoke.py", "crew"),
    ("workspace-transfer", "workspace_transfer_smoke.py", "crew"),
    ("presentation", "presentation_smoke.py", "crew"),
    ("agent-detection", "agent_detection_smoke.py", "crew"),
    ("plugin", "plugin_smoke.py", "crew"),
)


def require(condition, message):
    if not condition:
        raise ValueError(message)


def json_bytes(value):
    return (json.dumps(value, sort_keys=True, indent=2, ensure_ascii=True) + "\n").encode("utf-8")


def digest(content):
    return hashlib.sha256(content).hexdigest()


def utc_now():
    return datetime.now(timezone.utc).isoformat()


def file_record(path):
    path = Path(path)
    require(path.is_file() and not path.is_symlink(), "missing or unsafe file: " + str(path))
    content = path.read_bytes()
    return {"bytes": len(content), "sha256": digest(content)}


def git(root, *args):
    return subprocess.run(["git", *args], cwd=root, check=True, capture_output=True,
                          timeout=30).stdout


def source_snapshot(root):
    """Hash tracked and nonignored untracked files, without hiding dirty sources.

    Outputs may only be outside the checkout or in a git-ignored directory. Git's
    assume-unchanged/skip-worktree flags cannot turn modified sources into clean
    packaging input. Symlink targets are hashed as links, never followed.
    """
    root = Path(root).resolve()
    require(Path(os.fsdecode(git(root, "rev-parse", "--show-toplevel")).strip()).resolve() == root,
            "source root must be the Git worktree root")
    tracked = set(filter(None, git(root, "ls-files", "--cached", "-z").split(b"\0")))
    untracked = set(filter(None, git(root, "ls-files", "--others", "--exclude-standard", "-z").split(b"\0")))
    flags = [os.fsdecode(item) for item in git(root, "ls-files", "-v", "-z").split(b"\0")
             if item and not item.startswith(b"H ")]
    status = os.fsdecode(git(root, "status", "--porcelain=v1", "-z", "--untracked-files=all"))
    files = []
    for raw in sorted(tracked | untracked):
        name = os.fsdecode(raw)
        path = root / name
        record = {"path": name, "tracked": raw in tracked}
        try:
            info = path.lstat()
        except FileNotFoundError:
            record["kind"] = "missing"
        else:
            if stat.S_ISLNK(info.st_mode):
                content = os.fsencode(os.readlink(path))
                record.update(kind="symlink", bytes=len(content), sha256=digest(content))
            else:
                require(stat.S_ISREG(info.st_mode), "unsupported source entry (including submodules): " + name)
                record.update(kind="file", executable=bool(info.st_mode & stat.S_IXUSR), **file_record(path))
        files.append(record)
    result = {
        "root": str(root),
        "head_sha": git(root, "rev-parse", "HEAD").decode().strip(),
        "tree_sha": git(root, "rev-parse", "HEAD^{tree}").decode().strip(),
        "index_sha256": digest(git(root, "ls-files", "--stage", "-z")),
        "dirty": bool(status or flags), "status_porcelain_z": status,
        "nonstandard_index_flags": flags, "files": files,
    }
    result["snapshot_sha256"] = digest(json_bytes(result))
    return result


def source_identity(source):
    return {key: source[key] for key in ("head_sha", "tree_sha", "snapshot_sha256", "dirty")}


def artifact_directory(root, value, *, empty=False):
    """Reject an output path that could hide or overwrite repository sources."""
    root = Path(root).resolve()
    path = Path(value).absolute()
    require(not path.is_symlink(), "artifact directory cannot be a symlink")
    path = path.resolve()
    require(path != root and path not in root.parents,
            "artifact directory cannot be the source root or its ancestor")
    if path.is_relative_to(root):
        relative = path.relative_to(root).as_posix()
        require(relative != ".git" and not relative.startswith(".git/"), "artifact directory cannot be Git metadata")
        tracked = [os.fsdecode(item) for item in git(root, "ls-files", "--cached", "-z").split(b"\0") if item]
        require(not any(name == relative or name.startswith(relative + "/") for name in tracked),
                "artifact directory contains tracked source files")
        ignored = subprocess.run(["git", "check-ignore", "--quiet", "--", relative + "/"], cwd=root)
        require(ignored.returncode == 0,
                "artifact directories inside the checkout must be git-ignored; prefer a sibling directory")
    require(not path.exists() or path.is_dir(), "artifact path is not a directory")
    if empty:
        require(not path.exists() or not any(path.iterdir()), "output directory must be new or empty")
    return path


def native_target():
    machine = platform.machine().lower()
    if sys.platform == "linux" and machine in ("x86_64", "amd64"):
        return TARGETS[0]
    if sys.platform == "win32" and machine in ("x86_64", "amd64"):
        return TARGETS[1]
    if sys.platform == "darwin" and machine in ("arm64", "aarch64"):
        return TARGETS[2]
    if sys.platform == "darwin" and machine in ("x86_64", "amd64"):
        return TARGETS[3]
    raise ValueError(f"unsupported native platform: {sys.platform}/{machine}; no cross-platform PASS is possible")


def binary_paths(target_dir, target):
    suffix = ".exe" if target == TARGETS[1] else ""
    return {name: Path(target_dir) / target / "release" / (name + suffix) for name in ("crew", "tmuxlike")}


def check_binary_format(content, target):
    if target == TARGETS[1]:
        from windows_installer_contract import require_static_crt
        require_static_crt(content)
    elif target == TARGETS[0]:
        require(len(content) >= 20 and content[:6] == b"\x7fELF\x02\x01"
                and int.from_bytes(content[18:20], "little") == 62, "expected a native Linux x86_64 ELF binary")
    else:
        cpu = 0x0100000C if target == TARGETS[2] else 0x01000007
        require(len(content) >= 8 and content[:4] == b"\xcf\xfa\xed\xfe"
                and int.from_bytes(content[4:8], "little") == cpu, "expected the native macOS Mach-O architecture")


def binary_records(target_dir, target):
    records = {}
    for name, path in binary_paths(target_dir, target).items():
        record = file_record(path)
        check_binary_format(path.read_bytes(), target)
        records[name] = {"path": str(path.resolve()), **record}
    return records


def command_plan(root, output, target_dir, target, offline, python, ssh_fixture_user=None):
    root, output, target_dir = (Path(item).resolve() for item in (root, output, target_dir))
    binaries = binary_paths(target_dir, target)
    cargo_options = ["--locked", *(["--offline"] if offline else [])]
    steps = []

    def add(name, argv, *, needs_build=False, timeout=1800):
        steps.append({"name": name, "argv": [str(item) for item in argv],
                      "needs_build": needs_build, "timeout_seconds": timeout})

    add("tool-rustc", ["rustc", "-vV"], timeout=60)
    add("tool-cargo", ["cargo", "--version"], timeout=60)
    add("tool-git", ["git", "--version"], timeout=60)
    add("tool-python", [python, "--version"], timeout=60)
    add("format", ["cargo", "fmt", "--all", "--check"])
    add("clippy", ["cargo", "clippy", *cargo_options, "--all-targets", "--target-dir", target_dir, "--", "-D", "warnings"])
    add("unit", ["cargo", "test", *cargo_options, "--target-dir", target_dir])
    add("release-tooling", [python, root / "scripts/package_release.py", "self-test"])
    add("local-tooling", [python, root / "scripts/test_local_validation.py"])
    add("build", ["cargo", "build", *cargo_options, "--release", "--bins", "--target", target, "--target-dir", target_dir])
    for name, path in binaries.items():
        add("version-" + name, [path, "--version"], needs_build=True, timeout=30)
    for name, script, binary in SMOKES:
        add(name, [python, root / "scripts" / script, binaries[binary]], needs_build=True, timeout=900)
    if target == TARGETS[0]:
        add("ssh-federation", [python, root / "scripts/ssh_federation_smoke.py", binaries["crew"],
                               "--logs", output / "logs/ssh",
                               *(["--fixture-user", ssh_fixture_user] if ssh_fixture_user else [])], needs_build=True, timeout=900)
    if target == TARGETS[1]:
        add("windows-installer-build", [python, root / "scripts/windows_installer_local.py", "--binary-dir", binaries["crew"].parent,
                                         "--output", output / "installer", "--logs", output / "logs/installer",
                                         *(["--offline"] if offline else [])], needs_build=True)
        add("windows-installer-lifecycle", [python, root / "scripts/windows_installer_smoke.py", "--local-provider",
                                             "--input", output / "installer", "--logs", output / "logs/installer",
                                             *(["--offline"] if offline else [])], needs_build=True)
    return steps


def unavailable_checks(target):
    from windows_installer_contract import CHECKS
    entries = []
    if target != TARGETS[1]:
        entries.extend({"name": name, "result": "NOT_RUN", "reason": "requires native Windows"}
                       for name in ("windows-installer-build", "windows-installer-lifecycle"))
        entries.extend({"name": "installer:" + name, "result": "NOT_RUN", "reason": "requires native Windows"} for name in CHECKS)
    if target != TARGETS[0]:
        entries.append({"name": "ssh-federation", "result": "NOT_RUN",
                        "reason": "real sshd/PTY smoke currently supports native Linux only"})
    return entries


def run_command(spec, root, output, env, index, *, blocked=None):
    log = output / "logs" / f"{index:02d}-{spec['name']}.log"
    started = utc_now()
    start_clock = time.monotonic()
    result, exit_code, error = "NOT_RUN", None, blocked
    print(f"{spec['name']}: {'NOT_RUN: ' + blocked if blocked else 'running'}", flush=True)
    with log.open("wb") as stream:
        if blocked:
            stream.write(("NOT_RUN: " + blocked + "\n").encode())
        else:
            try:
                process = subprocess.run(spec["argv"], cwd=root, env=env, stdout=stream,
                                         stderr=subprocess.STDOUT, timeout=spec["timeout_seconds"])
                exit_code = process.returncode
                result = "PASS" if exit_code == 0 else "FAIL"
            except (OSError, subprocess.TimeoutExpired) as exc:
                error = str(exc)
                result = "BLOCKED" if isinstance(exc, OSError) else "FAIL"
                stream.write(("\n" + result + ": " + error + "\n").encode("utf-8", errors="replace"))
    record = {**spec, "cwd": str(root), "started_at": started, "ended_at": utc_now(),
              "duration_seconds": round(time.monotonic() - start_clock, 3), "exit_code": exit_code,
              "result": result, "error": error,
              "log": {"path": log.relative_to(output).as_posix(), **file_record(log)}}
    if result == "PASS" and re.search(r"(?im)^\s*(?:NOT_RUN|SKIP(?:PED)?)(?:\s|:|$)", log.read_text(encoding="utf-8", errors="replace")):
        record.update(result="FAIL", error="command exited zero but reported skipped or unexecuted checks")
    print(f"{spec['name']}: {record['result']} (exit {exit_code}; {log})", flush=True)
    return record


def artifact_records(folder):
    folder = Path(folder)
    return {path.relative_to(folder).as_posix(): file_record(path) for path in sorted(folder.rglob("*"))
            if path.is_file() or path.is_symlink()}


def verify_artifacts(folder, records):
    require(isinstance(records, dict), "artifact records are missing")
    require(artifact_records(folder) == records, "artifact files are missing, added or changed")


def read_log(folder, record):
    name = record.get("log", {}).get("path", "")
    relative = Path(name)
    require(name and not relative.is_absolute() and ".." not in relative.parts, "unsafe evidence log path")
    path = Path(folder) / relative
    require(path.resolve().is_relative_to(Path(folder).resolve()), "evidence log escapes its directory")
    require({"path": name, **file_record(path)} == record["log"], "command log is missing or changed")
    return path.read_text(encoding="utf-8", errors="replace")


def verify_evidence(root, evidence_path, *, target_dir=None):
    """Fail closed before packaging; local consistency evidence is not a signature."""
    root, evidence_path = Path(root).resolve(), Path(evidence_path).resolve()
    report = json.loads(evidence_path.read_bytes())
    require(report.get("schema") == SCHEMA and report.get("provider") == "local"
            and report.get("kind") == "crew-local-validation" and report.get("scope") == "native-platform",
            "expected local native validation evidence")
    require(not any(key in report for key in ("ci_run_id", "ci_attempt")), "local evidence must not invent CI provenance")
    require(report.get("result") == "PASS" and report.get("source_unchanged") is True
            and report.get("binaries_unchanged") is True, "validation did not pass with stable sources and binaries")
    config = report["configuration"]
    target = native_target()
    require(report.get("platform", {}).get("target") == target and report["platform"].get("simulation") is False,
            "evidence is not for this native platform")
    require(Path(config["root"]).resolve() == root and Path(config["output"]).resolve() == evidence_path.parent,
            "evidence belongs to another checkout or output directory")
    require(isinstance(config["offline"], bool), "invalid offline setting")
    output = artifact_directory(root, config["output"])
    build_dir = artifact_directory(root, config["target_dir"])
    if target_dir is not None:
        require(Path(target_dir).resolve() == build_dir, "requested target directory differs from the tested binaries")
    source = source_snapshot(root)
    require(not source["dirty"] and source == report.get("source_start") == report.get("source_end"),
            "packaging requires the same clean immutable source; commit changes and validate again")
    plan = command_plan(root, output, build_dir, target, config["offline"], config["python"], config.get("ssh_fixture_user"))
    require(report.get("required_steps") == [step["name"] for step in plan], "required check list is missing or differs")
    records = report.get("commands", [])
    require(len(records) == len(plan), "required checks are missing or duplicated")
    logs = {}
    for spec, record in zip(plan, records):
        require(all(record.get(key) == value for key, value in spec.items()) and record.get("cwd") == str(root),
                "command plan differs: " + spec["name"])
        require(record.get("result") == "PASS" and type(record.get("exit_code")) is int and record["exit_code"] == 0,
                "required check is failed, missing or skipped: " + spec["name"])
        log = read_log(output, record)
        logs[spec["name"]] = log
        if len(spec["argv"]) > 1 and spec["argv"][1].endswith(".py"):
            file_record(spec["argv"][1])
        require(not re.search(r"(?im)^\s*(?:NOT_RUN|SKIP(?:PED)?)(?:\s|:|$)", log), "command reported skipped checks")
    with (root / "Cargo.toml").open("rb") as stream:
        version = tomllib.load(stream)["package"]["version"]
    with (root / "rust-toolchain.toml").open("rb") as stream:
        rust = tomllib.load(stream)["toolchain"]["channel"]
    require(report.get("errors") == [] and report.get("version") == version, "evidence contains errors or a different version")
    require("release: " + rust in logs["tool-rustc"].splitlines() and "host: " + target in logs["tool-rustc"].splitlines(),
            "Rust toolchain evidence differs from the pinned native toolchain")
    require(all(logs["version-" + name].strip() in ("crew " + version, "tmuxlike " + version) for name in ("crew", "tmuxlike")),
            "tested binary version differs from Cargo.toml")
    require(report.get("unavailable_checks") == unavailable_checks(target), "platform coverage declaration differs")
    require(report.get("matrix_result") == "NOT_RUN", "one native run cannot claim a complete platform matrix")
    binaries = binary_records(build_dir, target)
    require(binaries == report.get("binaries_after_build") == report.get("binaries_end"), "tested binary is missing or changed")
    verify_artifacts(output / "logs", report.get("log_files"))
    if target == TARGETS[1]:
        from windows_installer_local import verify_local_installer
        verify_artifacts(output / "installer", report.get("installer_files"))
        verify_local_installer(output / "installer", source, binaries, require_smoke=True)
    else:
        require(report.get("installer_files") == {}, "non-Windows evidence cannot claim native installer artifacts")
    return report


def validate(args):
    root = ROOT
    target = native_target()
    output = artifact_directory(root, args.output, empty=True)
    target_dir = artifact_directory(root, args.target_dir)
    require(output != target_dir and not target_dir.is_relative_to(output), "output cannot contain the build target directory")
    with (root / "Cargo.toml").open("rb") as stream:
        version = tomllib.load(stream)["package"]["version"]
    with (root / "rust-toolchain.toml").open("rb") as stream:
        rust_version = tomllib.load(stream)["toolchain"]["channel"]
    source_start = source_snapshot(root)
    output.mkdir(parents=True, exist_ok=True)
    (output / "logs").mkdir()
    env = dict(os.environ, PYTHONDONTWRITEBYTECODE="1", CARGO_TARGET_DIR=str(target_dir))
    if args.offline:
        env["CARGO_NET_OFFLINE"] = "true"
    if target == TARGETS[1]:
        require(not env.get("CARGO_ENCODED_RUSTFLAGS"), "use RUSTFLAGS, not CARGO_ENCODED_RUSTFLAGS, for native Windows validation")
        env["RUSTFLAGS"] = (env.get("RUSTFLAGS", "") + " -C target-feature=+crt-static").strip()
    require(not args.ssh_fixture_user or target == TARGETS[0], "--ssh-fixture-user requires native Linux")
    require(not args.ssh_fixture_user or re.fullmatch(r"[A-Za-z_][A-Za-z0-9_-]{0,63}", args.ssh_fixture_user),
            "invalid SSH fixture username")
    plan = command_plan(root, output, target_dir, target, args.offline, sys.executable, args.ssh_fixture_user)
    report = {"schema": SCHEMA, "provider": "local", "kind": "crew-local-validation", "scope": "native-platform",
              "started_at": utc_now(), "result": "NOT_RUN", "matrix_result": "NOT_RUN",
              "configuration": {"root": str(root), "output": str(output), "target_dir": str(target_dir),
                                "offline": args.offline, "python": sys.executable, "ssh_fixture_user": args.ssh_fixture_user},
              "platform": {"target": target, "sys_platform": sys.platform, "system": platform.system(),
                           "release": platform.release(), "machine": platform.machine(), "simulation": False},
              "tools": {"python": sys.version, "rustc_path": shutil.which("rustc"), "cargo_path": shutil.which("cargo"),
                        "git_path": shutil.which("git"), "expected_rust": rust_version},
              "build_environment": {name: env.get(name) for name in ("RUSTFLAGS", "CARGO_ENCODED_RUSTFLAGS", "CARGO_BUILD_JOBS")},
              "version": version, "source_start": source_start, "source_end": None,
              "source_unchanged": False, "binaries_unchanged": False,
              "binaries_after_build": {}, "binaries_end": {}, "errors": [], "commands": [],
              "required_steps": [step["name"] for step in plan], "unavailable_checks": unavailable_checks(target)}
    evidence = output / "validation.json"
    build_ok, toolchain_ok, installer_ok = False, False, False
    for index, spec in enumerate(plan, 1):
        blocked = None
        if spec["needs_build"] and not build_ok:
            blocked = "build did not produce verified native binaries"
        if spec["name"] in ("format", "clippy", "unit", "build") and not toolchain_ok:
            blocked = "pinned native Rust toolchain was not verified"
        if spec["name"] == "windows-installer-lifecycle" and not installer_ok:
            blocked = "local installer build did not pass"
        record = run_command(spec, root, output, env, index, blocked=blocked)
        log = read_log(output, record)
        if spec["name"] == "tool-rustc":
            report["tools"]["rustc"] = log.strip()
            toolchain_ok = record["result"] == "PASS" and f"release: {rust_version}\n" in log and f"host: {target}\n" in log
            if not toolchain_ok:
                record.update(result="FAIL", error="rustc must match the pinned toolchain and native target")
        elif spec["name"].startswith("tool-"):
            report["tools"][spec["name"][5:]] = log.strip()
        if spec["name"].startswith("version-") and record["result"] == "PASS":
            if log.strip() not in ("crew " + version, "tmuxlike " + version):
                record.update(result="FAIL", error="binary version differs from Cargo.toml")
        if spec["name"] == "build" and record["result"] == "PASS":
            try:
                report["binaries_after_build"] = binary_records(target_dir, target)
                build_ok = True
            except (ValueError, OSError) as exc:
                record.update(result="FAIL", error=str(exc))
        if spec["name"] == "windows-installer-build":
            installer_ok = record["result"] == "PASS"
        report["commands"].append(record)
        evidence.write_bytes(json_bytes(report))
    try:
        report["source_end"] = source_snapshot(root)
        report["source_unchanged"] = report["source_end"] == source_start
        if build_ok:
            report["binaries_end"] = binary_records(target_dir, target)
            report["binaries_unchanged"] = report["binaries_end"] == report["binaries_after_build"]
        if target == TARGETS[1] and installer_ok:
            from windows_installer_local import verify_local_installer
            verify_local_installer(output / "installer", report["source_end"], report["binaries_end"], require_smoke=True)
    except (ValueError, OSError, KeyError, subprocess.SubprocessError) as exc:
        report["errors"].append(str(exc))
    if not report["source_unchanged"]:
        report["errors"].append("source changed during validation")
    if not report["binaries_unchanged"]:
        report["errors"].append("native binaries were not built or changed during validation")
    report["log_files"] = artifact_records(output / "logs")
    report["installer_files"] = artifact_records(output / "installer") if target == TARGETS[1] else {}
    report["result"] = "PASS" if not report["errors"] and all(item["result"] == "PASS" for item in report["commands"]) else "FAIL"
    report["ended_at"] = utc_now()
    evidence.write_bytes(json_bytes(report))
    print(f"{report['result']} native local validation: {evidence}; complete platform matrix: NOT_RUN", flush=True)
    if source_start["dirty"]:
        print("Source is dirty; package_local.py will reject this evidence. Commit and validate again.", flush=True)
    return 0 if report["result"] == "PASS" else 1


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--offline", action="store_true", help="disable Cargo and installer dependency downloads; loopback SSH tests still run")
    parser.add_argument("--output", required=True, help="new evidence directory outside source, or in a git-ignored directory")
    parser.add_argument("--target-dir", default=str(ROOT / "target"), help="Cargo build directory, outside source or git-ignored")
    parser.add_argument("--ssh-fixture-user", help="Linux only: run the isolated SSH fixture as this existing local user")
    args = parser.parse_args()
    try:
        raise SystemExit(validate(args))
    except (ValueError, KeyError, OSError, subprocess.SubprocessError) as exc:
        print("Local validation error: " + str(exc), file=sys.stderr)
        raise SystemExit(1) from exc


if __name__ == "__main__":
    main()
