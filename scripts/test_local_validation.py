#!/usr/bin/env python3
"""Offline regression tests for local evidence and packaging rejection paths.

Temporary Git repositories and synthetic evidence exercise the validator; these
fixtures are not native product or installer execution evidence.
"""
import argparse
import contextlib
import copy
import io
import json
import os
from pathlib import Path
import subprocess
import sys
import tarfile
import tempfile
import unittest
from unittest import mock

sys.dont_write_bytecode = True

import package_local as package
import validate_local as local
import windows_installer_local as windows
from windows_installer_contract import APP_ID, CHECKS, INNO, LEGACY, TARGET, installer_name


class LocalChecks(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix="crew-local-tests-")
        self.addCleanup(self.temporary.cleanup)
        self.home = Path(self.temporary.name)
        self.root = self.home / "source"
        self.root.mkdir()
        self.output = self.home / "evidence"
        (self.output / "logs").mkdir(parents=True)
        self.build = self.home / "target"
        self.target = local.TARGETS[0]
        self.addCleanup(mock.patch.stopall)
        mock.patch.object(local, "native_target", return_value=self.target).start()
        (self.root / ".gitignore").write_text("target/\n", encoding="utf-8")
        (self.root / "Cargo.toml").write_text('[package]\nname = "crew"\nversion = "0.5.0"\n', encoding="utf-8")
        (self.root / "rust-toolchain.toml").write_text('[toolchain]\nchannel = "1.96.1"\n', encoding="utf-8")
        for name in ("README.md", "LICENSE"):
            (self.root / name).write_text("test fixture " + name + "\n", encoding="utf-8")
        plan = local.command_plan(self.root, self.output, self.build, self.target, True, sys.executable)
        windows_plan = local.command_plan(self.root, self.output, self.build, TARGET, True, sys.executable)
        for spec in [*plan, *windows_plan]:
            if len(spec["argv"]) > 1 and spec["argv"][1].endswith(".py"):
                path = Path(spec["argv"][1])
                path.parent.mkdir(exist_ok=True)
                path.write_text("# Synthetic source fixture. Never executed.\n", encoding="utf-8")
        self.git("init", "-q")
        self.git("add", ".")
        self.git("commit", "-qm", "fixture")
        for name, path in local.binary_paths(self.build, self.target).items():
            path.parent.mkdir(parents=True, exist_ok=True)
            header = bytearray(64)
            header[:6] = b"\x7fELF\x02\x01"
            header[18:20] = (62).to_bytes(2, "little")
            path.write_bytes(bytes(header) + name.encode())
        source = local.source_snapshot(self.root)
        self.assertFalse(source["dirty"])
        records = []
        for index, spec in enumerate(plan, 1):
            content = "fixture command result\n"
            if spec["name"] == "tool-rustc":
                content = "rustc 1.96.1\nrelease: 1.96.1\nhost: " + self.target + "\n"
            if spec["name"].startswith("version-"):
                content = "crew 0.5.0\n"
            path = self.output / "logs" / f"{index:02d}-{spec['name']}.log"
            path.write_text(content, encoding="utf-8")
            records.append({**spec, "cwd": str(self.root), "result": "PASS", "exit_code": 0, "error": None,
                            "started_at": "fixture", "ended_at": "fixture", "duration_seconds": 0,
                            "log": {"path": path.relative_to(self.output).as_posix(), **local.file_record(path)}})
        binaries = local.binary_records(self.build, self.target)
        self.report = {
            "schema": local.SCHEMA, "kind": "crew-local-validation", "provider": "local", "scope": "native-platform",
            "result": "PASS", "matrix_result": "NOT_RUN", "version": "0.5.0", "errors": [],
            "platform": {"target": self.target, "simulation": False},
            "configuration": {"root": str(self.root), "output": str(self.output), "target_dir": str(self.build),
                              "offline": True, "python": sys.executable, "ssh_fixture_user": None},
            "source_start": source, "source_end": source, "source_unchanged": True,
            "binaries_after_build": binaries, "binaries_end": binaries, "binaries_unchanged": True,
            "commands": records, "required_steps": [spec["name"] for spec in plan],
            "unavailable_checks": local.unavailable_checks(self.target), "installer_files": {},
            "log_files": local.artifact_records(self.output / "logs"),
        }
        self.evidence = self.output / "validation.json"
        self.write_report()

    def git(self, *args):
        return subprocess.run(["git", "-c", "user.name=Crew local test", "-c", "user.email=local-test@example.invalid",
                               "-c", "core.hooksPath=" + str(self.home / "no-hooks"), *args], cwd=self.root,
                              check=True, stdout=subprocess.PIPE, stderr=subprocess.PIPE).stdout

    def write_report(self, report=None):
        self.evidence.write_bytes(local.json_bytes(self.report if report is None else report))

    def verify(self):
        return local.verify_evidence(self.root, self.evidence)

    def package(self):
        with contextlib.redirect_stdout(io.StringIO()):
            return package.package(argparse.Namespace(evidence=self.evidence, output=self.home / "packages", target_dir=None), root=self.root)

    def test_required_plan_has_every_smoke_and_native_boundaries(self):
        self.verify()
        names = self.report["required_steps"]
        self.assertTrue({"format", "clippy", "unit", "release-tooling", "local-tooling", "build", "workspace-transfer",
                         "presentation", "agent-detection", "plugin", "ssh-federation"}.issubset(names))
        self.assertEqual(len(local.SMOKES), 12)
        self.assertNotIn("windows-installer-build", names)
        self.assertEqual(len([item for item in self.report["unavailable_checks"] if item["name"].startswith("installer:")]), 14)
        windows_plan = local.command_plan(self.root, self.output, self.build, TARGET, True, sys.executable)
        self.assertEqual(windows_plan[-2]["name"], "windows-installer-build")
        self.assertIn("--local-provider", windows_plan[-1]["argv"])
        self.assertIn("--offline", windows_plan[-1]["argv"])
        self.assertNotIn("ssh-federation", [item["name"] for item in windows_plan])
        delegated = local.command_plan(self.root, self.output, self.build, self.target, True, sys.executable, "fixture-user")
        self.assertEqual(delegated[-1]["argv"][-2:], ["--fixture-user", "fixture-user"])

    def test_missing_duplicate_or_relabelled_required_checks_are_rejected(self):
        for mode in ("missing", "list-reduced", "duplicate", "argv", "skip", "nonzero", "not-run", "boolean-exit"):
            with self.subTest(mode=mode):
                changed = copy.deepcopy(self.report)
                if mode == "missing":
                    changed["commands"].pop()
                elif mode == "list-reduced":
                    changed["commands"].pop()
                    changed["required_steps"].pop()
                elif mode == "duplicate":
                    changed["commands"][-1] = changed["commands"][-2]
                elif mode == "argv":
                    changed["commands"][6]["argv"].append("--skip")
                elif mode == "skip":
                    changed["commands"][-1]["result"] = "SKIP"
                elif mode == "nonzero":
                    changed["commands"][-1]["exit_code"] = 1
                elif mode == "not-run":
                    changed["commands"][-1]["exit_code"] = None
                else:
                    changed["commands"][-1]["exit_code"] = False
                self.write_report(changed)
                with self.assertRaises(ValueError):
                    self.verify()

    def test_provider_platform_and_matrix_claims_are_rejected(self):
        for key, value in (("provider", "github"), ("matrix_result", "PASS"), ("ci_run_id", 123),
                           ("source_unchanged", False), ("binaries_unchanged", False), ("version", "0.6.0"),
                           ("unavailable_checks", []), ("errors", ["a failed check"])):
            with self.subTest(key=key):
                changed = copy.deepcopy(self.report)
                changed[key] = value
                self.write_report(changed)
                with self.assertRaises(ValueError):
                    self.verify()
        changed = copy.deepcopy(self.report)
        changed["platform"]["target"] = TARGET
        self.write_report(changed)
        with self.assertRaises(ValueError):
            self.verify()

    def test_dirty_tracked_and_untracked_sources_cannot_be_packaged(self):
        (self.root / "README.md").write_text("changed after tests", encoding="utf-8")
        with self.assertRaisesRegex(ValueError, "clean immutable source"):
            self.package()
        self.git("checkout", "--", "README.md")
        extra = self.root / "untracked-source.rs"
        extra.write_text("fn added() {}", encoding="utf-8")
        snapshot = local.source_snapshot(self.root)
        self.assertTrue(snapshot["dirty"])
        self.assertEqual(next(item for item in snapshot["files"] if item["path"] == extra.name)["tracked"], False)
        with self.assertRaisesRegex(ValueError, "clean immutable source"):
            self.package()
        self.assertFalse((self.home / "packages").exists())

    def test_changed_commit_rejects_identical_tree_evidence(self):
        self.git("commit", "--allow-empty", "-qm", "different immutable candidate")
        self.assertEqual(local.source_snapshot(self.root)["tree_sha"], self.report["source_start"]["tree_sha"])
        with self.assertRaisesRegex(ValueError, "clean immutable source"):
            self.verify()

    def test_assume_unchanged_does_not_hide_source_changes(self):
        self.git("update-index", "--assume-unchanged", "README.md")
        (self.root / "README.md").write_text("hidden change", encoding="utf-8")
        snapshot = local.source_snapshot(self.root)
        self.assertTrue(snapshot["dirty"])
        self.assertTrue(snapshot["nonstandard_index_flags"])
        with self.assertRaises(ValueError):
            self.verify()

    def test_binary_or_log_replacement_is_rejected(self):
        path = Path(self.report["binaries_end"]["crew"]["path"])
        original = path.read_bytes()
        path.write_bytes(original + b"changed")
        with self.assertRaisesRegex(ValueError, "binary is missing or changed"):
            self.package()
        path.write_bytes(original)
        log = self.output / self.report["commands"][-1]["log"]["path"]
        log.write_text("invented PASS", encoding="utf-8")
        with self.assertRaisesRegex(ValueError, "log is missing or changed"):
            self.package()
        self.assertFalse((self.home / "packages").exists())

    def test_zero_exit_skip_and_wrong_toolchain_logs_are_rejected(self):
        for name, content in (("ssh-federation", "SKIP: fixture unavailable\n"),
                              ("tool-rustc", "release: 1.95.0\nhost: " + self.target + "\n")):
            with self.subTest(name=name):
                changed = copy.deepcopy(self.report)
                record = next(item for item in changed["commands"] if item["name"] == name)
                path = self.output / record["log"]["path"]
                previous = path.read_bytes()
                path.write_text(content, encoding="utf-8")
                record["log"].update(local.file_record(path))
                changed["log_files"] = local.artifact_records(self.output / "logs")
                self.write_report(changed)
                with self.assertRaises(ValueError):
                    self.verify()
                path.write_bytes(previous)

    def test_artifact_directories_cannot_hide_source(self):
        for value in (self.root, self.root.parent, self.root / "scripts", self.root / ".git/logs", self.root / "untracked-output"):
            with self.subTest(value=value), self.assertRaises(ValueError):
                local.artifact_directory(self.root, value)
        accepted = local.artifact_directory(self.root, self.root / "target/local-evidence")
        self.assertEqual(accepted, self.root / "target/local-evidence")
        (self.root / "target").mkdir()
        (self.root / "target/tracked.txt").write_text("source despite ignore", encoding="utf-8")
        self.git("add", "--force", "target/tracked.txt")
        with self.assertRaisesRegex(ValueError, "tracked source"):
            local.artifact_directory(self.root, self.root / "target")

    def test_actual_fail_missing_tool_missing_script_and_skip_logs_are_preserved(self):
        cases = (([sys.executable, "-c", "raise SystemExit(7)"], "FAIL", 7),
                 ([str(self.home / "no-such-command")], "BLOCKED", None),
                 ([sys.executable, str(self.home / "missing.py")], "FAIL", 2),
                 ([sys.executable, "-c", "print('NOT_RUN: no fixture')"], "FAIL", 0))
        for index, (argv, result, code) in enumerate(cases, 100):
            spec = {"name": "failure-fixture", "argv": argv, "timeout_seconds": 5, "needs_build": False}
            with contextlib.redirect_stdout(io.StringIO()):
                record = local.run_command(spec, self.root, self.output, dict(os.environ), index)
            self.assertEqual(record["result"], result)
            self.assertEqual(record["exit_code"], code)
            self.assertTrue((self.output / record["log"]["path"]).is_file())

    def test_native_portable_contains_bound_payload_and_no_hosted_provenance(self):
        output = self.package()
        manifest = json.loads((output / "local-manifest.json").read_bytes())
        self.assertEqual(manifest["provider"], "local")
        self.assertEqual(manifest["source"], local.source_identity(self.report["source_start"]))
        self.assertEqual(manifest["validation"]["matrix_result"], "NOT_RUN")
        self.assertIsNone(manifest["installer"])
        self.assertNotIn("ci_run_id", manifest)
        archive = output / manifest["portable"]["name"]
        with tarfile.open(archive) as stream:
            self.assertEqual({Path(item.name).name for item in stream.getmembers()}, {"crew", "tmuxlike", "README.md", "LICENSE", "manifest.json"})
            for item in stream.getmembers():
                if Path(item.name).name in ("crew", "tmuxlike"):
                    self.assertEqual(item.mode, 0o755)
        for line in (output / "SHA256SUMS").read_text().splitlines():
            value, name = line.split("  ", 1)
            self.assertEqual(local.file_record(output / name)["sha256"], value)

    def test_binary_drift_during_archive_creation_leaves_no_package(self):
        write = package.write_portable

        def changed(*args, **kwargs):
            result = write(*args, **kwargs)
            path = Path(self.report["binaries_end"]["crew"]["path"])
            path.write_bytes(path.read_bytes() + b"late change")
            return result

        with mock.patch.object(package, "write_portable", side_effect=changed), self.assertRaisesRegex(ValueError, "binary changed during packaging"):
            self.package()
        self.assertFalse((self.home / "packages").exists())

    def test_source_drift_during_archive_creation_leaves_no_package(self):
        write = package.write_portable

        def changed(*args, **kwargs):
            result = write(*args, **kwargs)
            (self.root / "README.md").write_text("changed while packaging", encoding="utf-8")
            return result

        with mock.patch.object(package, "write_portable", side_effect=changed), self.assertRaisesRegex(ValueError, "source changed during packaging"):
            self.package()
        self.assertFalse((self.home / "packages").exists())

    def windows_fixture(self):
        from package_release import ReleaseChecks
        for path in local.binary_paths(self.build, TARGET).values():
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_bytes(ReleaseChecks.fixture_pe())
        binaries = local.binary_records(self.build, TARGET)
        source = local.source_snapshot(self.root)
        meta, payloads = package.portable_payload(self.root, source, TARGET, binaries)
        folder = self.home / "installer"
        folder.mkdir()
        archive = package.write_portable(folder, meta, payloads, local_name=False)
        (folder / f"Crew-{meta['tag']}-{TARGET}.json").write_bytes(local.json_bytes({
            "package": meta, "archive": {"name": archive.name, **local.file_record(archive)},
            "binary_dir": str(local.binary_paths(self.build, TARGET)["crew"].parent)}))
        setup = folder / installer_name(meta["tag"])
        setup.write_bytes(ReleaseChecks.fixture_pe())
        record = {**windows.local_meta(meta), "name": setup.name, **local.file_record(setup), "target": TARGET,
                  "kind": "inno-setup", "app_id": APP_ID, "scope": "current-user", "modifies_path": False,
                  "runtime": "static-msvc-crt", "signed": False, "compiler": INNO,
                  "files": {name: {"bytes": len(content), "sha256": local.digest(content)} for name, content in payloads.items()},
                  "validation": {"platform": "win32", "simulation": False, "installer_sha256": local.file_record(setup)["sha256"],
                                 "previous_portable_sha256": LEGACY["sha256"], "previous_version": "0.3.0", "installed_version": "0.5.0",
                                 "checks": [{"name": name, "result": "PASS"} for name in CHECKS]}}
        setup.with_suffix(".json").write_bytes(local.json_bytes(record))
        return folder, source, binaries, record

    def test_local_installer_contract_rejects_ci_ids_missing_checks_and_payload_drift(self):
        folder, source, binaries, record = self.windows_fixture()
        windows.verify_local_installer(folder, source, binaries, require_smoke=True)
        for mode in ("ci-id", "missing-check", "skipped-check", "manifest", "source", "no-validation"):
            with self.subTest(mode=mode):
                changed = copy.deepcopy(record)
                if mode == "ci-id":
                    changed["ci_run_id"] = 123
                elif mode == "missing-check":
                    changed["validation"]["checks"].pop()
                elif mode == "skipped-check":
                    changed["validation"]["checks"][0]["result"] = "NOT_RUN"
                elif mode == "manifest":
                    changed["files"]["manifest.json"]["sha256"] = "0" * 64
                elif mode == "source":
                    changed["source"]["head_sha"] = "0" * 40
                else:
                    del changed["validation"]
                (folder / Path(record["name"]).with_suffix(".json")).write_bytes(local.json_bytes(changed))
                with self.assertRaises(ValueError):
                    windows.verify_local_installer(folder, source, binaries, require_smoke=True)

    def test_installer_copy_race_cannot_package_unverified_setup_or_sidecar(self):
        folder, source, binaries, installer_record = self.windows_fixture()
        folder.rename(self.output / "installer")
        plan = local.command_plan(self.root, self.output, self.build, TARGET, True, sys.executable)
        changed = copy.deepcopy(self.report)
        changed.update(platform={"target": TARGET, "simulation": False}, source_start=source, source_end=source,
                       binaries_after_build=binaries, binaries_end=binaries, required_steps=[item["name"] for item in plan],
                       unavailable_checks=local.unavailable_checks(TARGET), commands=[])
        for path in (self.output / "logs").iterdir():
            path.unlink()
        for index, spec in enumerate(plan, 1):
            content = "synthetic Windows command fixture\n"
            if spec["name"] == "tool-rustc":
                content = "release: 1.96.1\nhost: " + TARGET + "\n"
            if spec["name"].startswith("version-"):
                content = "crew 0.5.0\n"
            path = self.output / "logs" / f"{index:02d}-{spec['name']}.log"
            path.write_text(content, encoding="utf-8")
            changed["commands"].append({**spec, "cwd": str(self.root), "result": "PASS", "exit_code": 0,
                                        "log": {"path": path.relative_to(self.output).as_posix(), **local.file_record(path)}})
        changed["log_files"] = local.artifact_records(self.output / "logs")
        changed["installer_files"] = local.artifact_records(self.output / "installer")
        self.write_report(changed)
        copy_file = package.shutil.copyfile
        for artifact in (installer_record["name"], Path(installer_record["name"]).with_suffix(".json").name):
            with self.subTest(artifact=artifact):
                def copy_during_change(source_path, destination_path):
                    source_path = Path(source_path)
                    if source_path.name != artifact:
                        return copy_file(source_path, destination_path)
                    original = source_path.read_bytes()
                    try:
                        source_path.write_bytes(original + b"temporary change during copy")
                        return copy_file(source_path, destination_path)
                    finally:
                        source_path.write_bytes(original)

                with mock.patch.object(local, "native_target", return_value=TARGET):
                    local.verify_evidence(self.root, self.evidence)
                    with mock.patch.object(package.shutil, "copyfile", side_effect=copy_during_change), \
                         self.assertRaisesRegex(ValueError, "installer artifact changed while copying"):
                        self.package()
                    local.verify_evidence(self.root, self.evidence)
                self.assertFalse((self.home / "packages").exists())

    def test_native_lifecycle_provider_branches_do_not_accept_each_other(self):
        import windows_installer_smoke as smoke
        folder, _, _, record = self.windows_fixture()
        args = argparse.Namespace(input=str(folder), logs=str(self.home / "installer-logs"), local_provider=False, offline=False)
        # Fail before any registry access or installation. Mocking this boundary
        # checks branch separation only; it is not a native lifecycle run.
        with mock.patch.object(smoke.sys, "platform", "win32"), mock.patch.dict(sys.modules, {"winreg": object()}):
            with self.assertRaises(KeyError):
                smoke.native_smoke(args)
            sidecar = folder / "Crew-v0.5.0-x86_64-pc-windows-msvc.json"
            data = json.loads(sidecar.read_bytes())
            data["package"]["provider"] = "github"
            data["package"]["ci_run_id"] = 123
            sidecar.write_bytes(local.json_bytes(data))
            args.local_provider = True
            with self.assertRaisesRegex(ValueError, "local package provenance"):
                smoke.native_smoke(args)

    def test_offline_installer_dependencies_do_not_fall_back_to_network(self):
        with mock.patch.dict(os.environ, {"RUNNER_TEMP": str(self.home / "uncached")}), \
             mock.patch.object(windows.installer, "compiler") as compiler, \
             mock.patch.object(windows.installer, "download") as download:
            with self.assertRaisesRegex(ValueError, "not cached"):
                windows.compiler_for_local(self.home, offline=True)
            with self.assertRaisesRegex(ValueError, "not cached"):
                windows.legacy_fixture_for_local("unused", self.home, self.home, offline=True)
            compiler.assert_not_called()
            download.assert_not_called()


if os.name == "nt":
    class WindowsTempCleanupTests(unittest.TestCase):
        def test_native_current_directory_lock_is_released_before_cleanup_passes(self):
            from smoke_temp import SmokeDirectory
            process = None
            try:
                with SmokeDirectory(prefix="crew-cleanup-native-") as directory:
                    env = dict(os.environ, CREW_TEST_CWD=directory)
                    process = subprocess.Popen([
                        "powershell", "-NoLogo", "-NoProfile", "-NonInteractive", "-Command",
                        "[Environment]::CurrentDirectory=$env:CREW_TEST_CWD; "
                        "[Console]::WriteLine('ready'); [Threading.Thread]::Sleep(500)"],
                        cwd=directory, env=env, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                        text=True, encoding="utf-8")
                    self.assertEqual(process.stdout.readline().strip(), "ready")
                    self.assertIsNone(process.poll())
                self.assertFalse(Path(directory).exists())
                self.assertEqual(process.wait(timeout=5), 0)
            finally:
                if process is not None:
                    if process.poll() is None:
                        process.kill()
                    process.communicate(timeout=5)

        def test_other_permission_errors_are_not_retried(self):
            from smoke_temp import SmokeDirectory
            temporary = SmokeDirectory(prefix="crew-cleanup-error-")
            error = PermissionError("fixture access denied")
            error.winerror = 5
            try:
                with mock.patch.object(tempfile.TemporaryDirectory, "cleanup", side_effect=error) as cleanup, \
                     mock.patch("smoke_temp.time.sleep") as sleep:
                    with self.assertRaises(PermissionError) as caught:
                        temporary.cleanup()
                    self.assertIs(caught.exception, error)
                    cleanup.assert_called_once()
                    sleep.assert_not_called()
            finally:
                temporary.cleanup()

        def test_sharing_violation_still_fails_at_deadline(self):
            from smoke_temp import SmokeDirectory
            temporary = SmokeDirectory(prefix="crew-cleanup-timeout-")
            error = PermissionError("fixture sharing violation")
            error.winerror = 32
            try:
                with mock.patch.object(tempfile.TemporaryDirectory, "cleanup", side_effect=error), \
                     mock.patch("smoke_temp.time.monotonic", side_effect=[0, 6]), \
                     mock.patch("smoke_temp.time.sleep") as sleep:
                    with self.assertRaises(PermissionError) as caught:
                        temporary.cleanup()
                    self.assertIs(caught.exception, error)
                    sleep.assert_not_called()
            finally:
                temporary.cleanup()

        def test_cleanup_refuses_a_replaced_parent_target(self):
            from smoke_temp import SmokeDirectory
            temporary = SmokeDirectory(prefix="crew-cleanup-target-")
            original = temporary.name
            try:
                temporary.name = str(Path(original).parent)
                with mock.patch.object(tempfile.TemporaryDirectory, "cleanup") as cleanup:
                    with self.assertRaisesRegex(ValueError, "escaped"):
                        temporary.cleanup()
                    cleanup.assert_not_called()
            finally:
                temporary.name = original
                temporary.cleanup()


if __name__ == "__main__":
    unittest.main(verbosity=2)
