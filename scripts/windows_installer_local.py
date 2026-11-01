#!/usr/bin/env python3
"""Prepare a native Inno installer for local validation, without CI provenance.

This preparation step is not a release or a successful validation result. The
same 14 lifecycle checks must run before package_local.py accepts the installer.
"""
import argparse
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile

sys.dont_write_bytecode = True

from package_local import portable_payload, write_portable
from validate_local import (ROOT, TARGETS, artifact_directory, binary_records, file_record,
                            json_bytes, require, source_snapshot)
from windows_installer_contract import (APP_ID, INNO, LEGACY, TARGET, installer_name,
                                        require_static_crt, validate_record)
import windows_installer as installer


def compiler_for_local(logs, *, offline=False):
    if offline:
        cached = Path(os.environ.get("RUNNER_TEMP", tempfile.gettempdir())) / "crew-inno-7.1.0/innosetup-7.1.0-x64.exe"
        require(cached.is_file() and not cached.is_symlink(), "offline Inno dependency is not cached: " + str(cached))
        installer.download(INNO, cached)  # Existing files are digest checked and never fetched.
    return installer.compiler(logs)


def legacy_fixture_for_local(iscc, output, logs, *, offline=False):
    name = "Crew-v0.3.0-windows-portable.zip"
    cached = Path(os.environ.get("RUNNER_TEMP", tempfile.gettempdir())) / "crew-local-dependencies" / name
    if offline:
        require(cached.is_file() and not cached.is_symlink(), "offline legacy dependency is not cached: " + str(cached))
    installer.download(LEGACY, cached)
    destination = Path(output) / name
    require(not destination.exists(), "legacy fixture destination already exists")
    destination.write_bytes(cached.read_bytes())
    return installer.legacy_fixture(iscc, output, logs)


def local_meta(package):
    require(package.get("schema") == 1 and package.get("provider") == "local"
            and package.get("repository") == "urtiger101-tw/Crew" and package.get("target") == TARGET,
            "expected native Windows local package provenance")
    require(not any(key in package for key in ("ci_run_id", "ci_attempt")), "local installer cannot contain CI provenance")
    return {key: package[key] for key in ("repository", "provider", "tag", "version", "source")}


def verify_local_installer(folder, source, binaries, *, require_smoke):
    folder = Path(folder)
    expected, _ = portable_payload(source["root"], source, TARGET, binaries)
    meta = local_meta(expected)
    sidecar = folder / f"Crew-{meta['tag']}-{TARGET}.json"
    data = json.loads(sidecar.read_bytes())
    require(data.get("package") == expected, "local installer portable source, binary or payload provenance differs")
    archive = folder / f"Crew-{meta['tag']}-{TARGET}.zip"
    require(data.get("archive") == {"name": archive.name, **file_record(archive)}, "local portable archive changed")
    from package_release import validate_archive
    validate_archive(archive, expected, check_windows_runtime=True)
    record = json.loads((folder / Path(installer_name(meta["tag"])).with_suffix(".json")).read_bytes())
    require(not any(key in record for key in ("ci_run_id", "ci_attempt")), "local installer cannot contain CI provenance")
    validate_record(folder, record, meta, expected["files"], require_smoke=require_smoke)
    from validate_local import digest
    embedded = json_bytes(expected)
    require(record["files"]["manifest.json"] == {"bytes": len(embedded), "sha256": digest(embedded)},
            "local installer embedded manifest differs")
    return record


def local_smoke_inputs(folder):
    """Check local provenance before the shared lifecycle performs any install."""
    folder = Path(folder)
    sidecars = list(folder.glob("Crew-v*-" + TARGET + ".json"))
    require(len(sidecars) == 1, "expected exactly one local portable sidecar")
    sidecar = json.loads(sidecars[0].read_bytes())
    package = sidecar["package"]
    meta = local_meta(package)
    binary_dir = Path(sidecar["binary_dir"]).resolve()
    require(binary_dir.name == "release" and binary_dir.parent.name == TARGET,
            "local installer binary path is not a native Cargo release directory")
    source = source_snapshot(ROOT)
    binaries = binary_records(binary_dir.parent.parent, TARGET)
    record = verify_local_installer(folder, source, binaries, require_smoke=False)
    return record, meta, package


def build(args):
    require(sys.platform == "win32", "NOT_RUN: local Inno compilation requires native Windows")
    require(Path.cwd().resolve() == ROOT, "run the local installer build from the repository root")
    binary_dir = Path(args.binary_dir).resolve()
    require(binary_dir.name == "release" and binary_dir.parent.name == TARGET,
            "expected native Windows Cargo target/release directory")
    source = source_snapshot(ROOT)
    binaries = binary_records(binary_dir.parent.parent, TARGET)
    meta, payloads = portable_payload(ROOT, source, TARGET, binaries)
    observed = installer.execute([binaries["crew"]["path"], "--version"])
    require(observed == "crew " + meta["version"], "binary version differs from local source")
    require_static_crt(payloads["crew.exe"])
    output = artifact_directory(ROOT, args.output, empty=True)
    logs = artifact_directory(ROOT, args.logs)
    output.mkdir(parents=True, exist_ok=True)
    logs.mkdir(parents=True, exist_ok=True)
    archive = write_portable(output, meta, payloads, local_name=False)
    sidecar = {"package": meta, "archive": {"name": archive.name, **file_record(archive)}, "binary_dir": str(binary_dir)}
    (output / f"Crew-{meta['tag']}-{TARGET}.json").write_bytes(json_bytes(sidecar))
    compiler = compiler_for_local(logs, offline=args.offline)
    path = installer.compile_payload(payloads, meta["version"], output, installer_name(meta["tag"]), compiler, logs)
    from validate_local import digest
    record = {**local_meta(meta), "name": path.name, **file_record(path), "target": TARGET,
              "kind": "inno-setup", "app_id": APP_ID, "scope": "current-user", "modifies_path": False,
              "runtime": "static-msvc-crt", "signed": False, "compiler": INNO,
              "files": {name: {"bytes": len(content), "sha256": digest(content)} for name, content in payloads.items()}}
    path.with_suffix(".json").write_bytes(json_bytes(record))
    require(source_snapshot(ROOT) == source, "source changed during local installer build")
    verify_local_installer(output, source, binaries, require_smoke=False)
    print(f"Built local validation installer: {path}; native lifecycle: NOT_RUN", flush=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary-dir", required=True)
    parser.add_argument("--output", required=True)
    parser.add_argument("--logs", required=True)
    parser.add_argument("--offline", action="store_true")
    args = parser.parse_args()
    try:
        build(args)
    except (ValueError, KeyError, OSError, subprocess.SubprocessError) as exc:
        print("Local installer error: " + str(exc), file=sys.stderr)
        raise SystemExit(1) from exc


if __name__ == "__main__":
    main()
