#!/usr/bin/env python3
"""Package only the current native binaries with complete matching local evidence.

No GitHub API, CI run ID, cross-platform claim, or publication is involved.
"""
import argparse
import json
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import tomllib

sys.dont_write_bytecode = True

from validate_local import (ROOT, TARGETS, artifact_directory, binary_records, file_record,
                            digest, json_bytes, require, source_identity, source_snapshot, verify_evidence)


def portable_payload(root, source, target, binaries):
    """Same payload metadata for an installer under test and the final local ZIP."""
    root = Path(root)
    with (root / "Cargo.toml").open("rb") as stream:
        cargo = tomllib.load(stream)["package"]
    with (root / "rust-toolchain.toml").open("rb") as stream:
        rust = tomllib.load(stream)["toolchain"]["channel"]
    require(cargo["name"] == "crew", "source package must be Crew")
    suffix = ".exe" if target == TARGETS[1] else ""
    payloads = {}
    for name in ("crew", "tmuxlike"):
        path = Path(binaries[name]["path"])
        require({"path": str(path), **file_record(path)} == binaries[name], "binary changed before packaging: " + name)
        content = path.read_bytes()
        require({"path": str(path), "bytes": len(content), "sha256": digest(content)} == binaries[name],
                "binary changed while reading payload: " + name)
        payloads[name + suffix] = content
    source_files = {item["path"]: item for item in source["files"]}
    for name in ("README.md", "LICENSE"):
        require((root / name).is_file() and not (root / name).is_symlink(), "missing package source: " + name)
        payloads[name] = (root / name).read_bytes()
        require(source_files[name].get("sha256") == digest(payloads[name]), "source changed while reading payload: " + name)
    meta = {"schema": 1, "provider": "local", "repository": "urtiger101-tw/Crew",
            "tag": "v" + cargo["version"], "version": cargo["version"], "target": target,
            "source": source_identity(source), "rust": rust, "signed": False,
            "runtime": "static-msvc-crt" if target == TARGETS[1] else "system",
            "files": {name: {"bytes": len(content), "sha256": digest(content)} for name, content in payloads.items()}}
    payloads["manifest.json"] = json_bytes(meta)
    return meta, payloads


def write_portable(folder, meta, payloads, *, local_name=True):
    # These helpers validate archive bytes only. Release identity/CI/publication
    # functions are deliberately not called or altered by this local provider.
    from package_release import validate_archive, write_archive
    top = f"Crew-{meta['tag']}-{meta['target']}"
    extension = ".zip" if meta["target"] == TARGETS[1] else ".tar.gz"
    path = Path(folder) / (top + ("-local" if local_name else "") + extension)
    require(not path.exists(), "portable output already exists")
    write_archive(path, top, payloads)
    validate_archive(path, meta, check_windows_runtime=True)
    return path


def package(args, *, root=ROOT):
    root = Path(root).resolve()
    evidence = Path(args.evidence).resolve()
    report = verify_evidence(root, evidence, target_dir=getattr(args, "target_dir", None))
    evidence_digest = file_record(evidence)
    output = artifact_directory(root, args.output, empty=True)
    require(not evidence.is_relative_to(output), "package output cannot contain its input evidence")
    source = report["source_end"]
    target = report["platform"]["target"]
    binaries = report["binaries_end"]
    meta, payloads = portable_payload(root, source, target, binaries)
    output.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix=".crew-local-package-", dir=output.parent) as temporary:
        staging = Path(temporary)
        portable = write_portable(staging, meta, payloads)
        (staging / "local-validation.json").write_bytes(evidence.read_bytes())
        require(file_record(staging / "local-validation.json") == evidence_digest, "validation evidence changed while copying")
        manifest = {"schema": 1, "provider": "local", "scope": "native-platform", "published": False,
                    "source": source_identity(source), "platform": report["platform"], "package": meta,
                    "portable": {"name": portable.name, **file_record(portable)},
                    "validation": {"name": "local-validation.json", **evidence_digest,
                                   "original_path": str(evidence), "result": "PASS",
                                   "required_steps": report["required_steps"], "matrix_result": "NOT_RUN"},
                    "unavailable_checks": report["unavailable_checks"], "installer": None}
        if target == TARGETS[1]:
            from windows_installer_local import verify_local_installer
            record = verify_local_installer(evidence.parent / "installer", source, binaries, require_smoke=True)
            for name in (record["name"], Path(record["name"]).with_suffix(".json").name):
                shutil.copyfile(evidence.parent / "installer" / name, staging / name)
                require(file_record(staging / name) == report["installer_files"][name],
                        "installer artifact changed while copying: " + name)
            manifest["installer"] = record
        (staging / "local-manifest.json").write_bytes(json_bytes(manifest))
        sums = "".join(f"{file_record(path)['sha256']}  {path.name}\n" for path in sorted(staging.iterdir()))
        (staging / "SHA256SUMS").write_text(sums, encoding="utf-8", newline="\n")
        # Recheck after reading and assembling every payload; never ship bytes
        # from sources, evidence, logs, or binaries that drifted during packaging.
        require(file_record(evidence) == evidence_digest, "validation evidence changed during packaging")
        require(source_snapshot(root) == source, "source changed during packaging")
        require(binary_records(report["configuration"]["target_dir"], target) == binaries, "binary changed during packaging")
        verify_evidence(root, evidence)
        output.mkdir(parents=True, exist_ok=True)
        require(not any(output.iterdir()), "package output changed during packaging")
        for path in sorted(staging.iterdir()):
            path.rename(output / path.name)
    print(f"Packaged native local artifacts in {output}; target {target}; complete platform matrix: NOT_RUN", flush=True)
    return output


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--evidence", required=True, help="validation.json from a complete run against this clean checkout")
    parser.add_argument("--output", required=True, help="new or empty native package directory")
    parser.add_argument("--target-dir", help="optional additional check that evidence used this exact build directory")
    args = parser.parse_args()
    try:
        package(args)
    except (ValueError, KeyError, OSError, subprocess.SubprocessError) as exc:
        print("Local package error: " + str(exc), file=sys.stderr)
        raise SystemExit(1) from exc


if __name__ == "__main__":
    main()
