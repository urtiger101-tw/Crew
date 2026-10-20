#!/usr/bin/env python3
"""Build and verify Crew release archives; publish only with GitHub's job token.

The source must belong to main and have a successful CI run for that exact SHA.
Required platform jobs AND their test steps are checked; skipped checks do not pass.
Only the publish subcommand writes to GitHub, exclusively urtiger101-tw/Crew.
"""
import argparse
import hashlib
import io
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys
import tarfile
import tempfile
import tomllib
import unittest
from unittest import mock
import urllib.error
import urllib.parse
import urllib.request
import zipfile

from windows_installer_contract import (
    APP_ID, CHECKS as INSTALLER_CHECKS, INNO, LEGACY, TARGET as WINDOWS_TARGET,
    installer_name, pe_imports, require_static_crt, validate_record as validate_installer,
)

REPOSITORY = "urtiger101-tw/Crew"
TARGETS = (
    "x86_64-unknown-linux-gnu", "x86_64-pc-windows-msvc",
    "aarch64-apple-darwin", "x86_64-apple-darwin",
)
CI_PLATFORMS = {"Linux x86_64", "Windows x86_64", "macOS arm64"}
CI_STEPS = {
    "Repository guard", "Checkout", "Python", "Rust toolchain", "Format", "Clippy",
    "Unit tests", "Release tooling tests", "Build both executables", "Legacy PTY smoke",
    "Automation API smoke", "Workspace smoke", "Worktree smoke", "Agent identity smoke",
    "Bridge and events smoke", "Workspace move smoke", "Metadata smoke",
    "Local validation tooling tests", "Workspace transfer smoke", "Metadata presentation smoke",
    "Agent detection smoke", "Plugin lifecycle smoke",
}
WINDOWS_CI_STEPS = {"Build Windows installer", "Windows installer lifecycle smoke"}
LINUX_CI_STEPS = {"SSH federation smoke"}


def required_ci_steps(name):
    if name == "Crew CI Gate":
        return {"Require every platform"}
    return (CI_STEPS | (WINDOWS_CI_STEPS if name == "Windows x86_64" else set())
            | (LINUX_CI_STEPS if name == "Linux x86_64" else set()))


def require(condition, message):
    if not condition:
        raise ValueError(message)


def digest(data):
    return hashlib.sha256(data).hexdigest()


def json_bytes(value):
    return (json.dumps(value, indent=2, sort_keys=True) + "\n").encode("utf-8")


def revision(value):
    require(re.fullmatch(r"[0-9a-f]{40}", value or ""), "source SHA must be 40 lowercase hexadecimal characters")
    return value


def version_for(tag):
    require(re.fullmatch(r"v(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)", tag or ""),
            "tag must be a stable SemVer tag, for example v0.3.0")
    return tag[1:]


def run(*args):
    return subprocess.run(args, check=True, capture_output=True, text=True,
                          encoding="utf-8", timeout=30).stdout.strip()


def checked_source(tag, sha):
    version = version_for(tag)
    revision(sha)
    require(run("git", "rev-parse", "HEAD") == sha, "checkout HEAD does not match the verified source SHA")
    with Path("Cargo.toml").open("rb") as stream:
        package = tomllib.load(stream)["package"]
    require(package["name"] == "crew" and package["version"] == version,
            "release tag, Crew package name and Cargo.toml version must agree")
    with Path("rust-toolchain.toml").open("rb") as stream:
        require(tomllib.load(stream)["toolchain"]["channel"] == "1.96.1", "unexpected Rust toolchain")
    return version


def api(path, *, method="GET", value=None, raw=None, missing_ok=False, upload=False):
    token = os.environ.get("GH_TOKEN") or os.environ.get("GITHUB_TOKEN")
    require(token, "GitHub job token is missing")
    require(path.startswith(f"/repos/{REPOSITORY}/"), "GitHub request is outside the Crew repository")
    origin = "https://uploads.github.com" if upload else "https://api.github.com"
    headers = {
        "Authorization": "Bearer " + token,
        "Accept": "application/vnd.github+json",
        "X-GitHub-Api-Version": "2022-11-28",
        "User-Agent": "Crew-release-workflow",
    }
    if raw is not None:
        payload = raw
        headers["Content-Type"] = "application/octet-stream"
    elif value is not None:
        payload = json_bytes(value)
        headers["Content-Type"] = "application/json"
    else:
        payload = None
    request = urllib.request.Request(origin + path, data=payload, headers=headers, method=method)
    try:
        with urllib.request.urlopen(request, timeout=120 if upload else 30) as response:
            return json.load(response)
    except urllib.error.HTTPError as error:
        if missing_ok and error.code == 404:
            return None
        raise ValueError(f"GitHub {method} {path} returned HTTP {error.code}") from error


def tag_commit(tag):
    value = api(f"/repos/{REPOSITORY}/git/ref/tags/{urllib.parse.quote(tag, safe='')}", missing_ok=True)
    if value is None:
        return None
    obj = value["object"]
    for _ in range(8):
        if obj["type"] == "commit":
            return revision(obj["sha"])
        require(obj["type"] == "tag", "release tag must resolve to a commit")
        obj = api(f"/repos/{REPOSITORY}/git/tags/{revision(obj['sha'])}")["object"]
    raise ValueError("release tag contains too many nested annotated tags")


def no_existing_release(tag):
    # Listing also detects unfinished drafts, which must be reviewed instead of overwritten.
    for page in range(1, 101):
        releases = api(f"/repos/{REPOSITORY}/releases?per_page=100&page={page}")
        require(not any(item["tag_name"] == tag for item in releases),
                "a release or draft already exists for this tag; inspect it before retrying")
        if len(releases) < 100:
            return
    raise ValueError("release listing exceeded the verification limit")


def validate_ci_jobs(jobs):
    expected = CI_PLATFORMS | {"Crew CI Gate"}
    require(len(jobs) == len(expected) and {job["name"] for job in jobs} == expected,
            "CI is missing an expected platform/gate, or has an unexpected job set")
    for job in jobs:
        require(job["status"] == "completed" and job["conclusion"] == "success",
                f"required CI job did not succeed: {job['name']}")
        steps = job.get("steps", [])
        required = required_ci_steps(job["name"])
        for name in required:
            matches = [step for step in steps if step["name"] == name]
            require(len(matches) == 1 and matches[0]["status"] == "completed"
                    and matches[0]["conclusion"] == "success",
                    f"required CI step is missing, skipped or unsuccessful: {job['name']} / {name}")


def verify_ci(sha, ci_run_id=None, ci_attempt=None):
    """Read the latest run and its actual jobs, including immediately before publication."""
    revision(sha)
    query = urllib.parse.urlencode({"head_sha": sha, "branch": "main", "per_page": 100})
    runs = api(f"/repos/{REPOSITORY}/actions/workflows/ci.yml/runs?{query}")["workflow_runs"]
    runs = [item for item in runs if item["head_sha"] == sha and item["head_branch"] == "main"
            and item["event"] in {"push", "workflow_dispatch"} and item["head_repository"]["full_name"] == REPOSITORY]
    require(runs, "no main CI run exists for this SHA; a fully successful explicitly authorized hosted run is required for this hosted release path")
    latest = max(runs, key=lambda item: item["id"])
    require(latest["status"] == "completed" and latest["conclusion"] == "success",
            "latest CI run for the source SHA is not successful; wait for or fix CI")
    run_id, attempt = int(latest["id"]), int(latest["run_attempt"])
    require(ci_run_id is None or (int(ci_run_id), int(ci_attempt)) == (run_id, attempt),
            "CI run/attempt changed while packages were being built; reverify the candidate")
    jobs = api(f"/repos/{REPOSITORY}/actions/runs/{run_id}/attempts/{attempt}/jobs?per_page=100")
    require(jobs["total_count"] == len(jobs["jobs"]), "CI job listing is incomplete")
    validate_ci_jobs(jobs["jobs"])
    return {"ci_run_id": run_id, "ci_attempt": attempt,
            "ci_url": f"https://github.com/{REPOSITORY}/actions/runs/{run_id}/attempts/{attempt}"}


def verify_source(tag, sha=None, ci_run_id=None, ci_attempt=None):
    require(os.environ.get("GITHUB_REPOSITORY") == REPOSITORY, "release is restricted to " + REPOSITORY)
    event, ref = os.environ.get("GITHUB_EVENT_NAME"), os.environ.get("GITHUB_REF")
    require((event == "workflow_dispatch" and ref == "refs/heads/main")
            or (event == "push" and ref == "refs/tags/" + tag),
            "dispatch releases from main, or push the matching version tag")
    sha = sha or run("git", "rev-parse", "HEAD")
    version = checked_source(tag, sha)
    trigger_sha = revision(os.environ.get("GITHUB_SHA", ""))
    require(run("git", "rev-parse", trigger_sha + "^{commit}") == sha,
            "workflow trigger and checked-out release source differ")
    main = api(f"/repos/{REPOSITORY}/git/ref/heads/main")["object"]
    require(main["type"] == "commit", "main does not identify a commit")
    main_sha = revision(main["sha"])
    comparison = api(f"/repos/{REPOSITORY}/compare/{sha}...{main_sha}")
    require(comparison["status"] in {"ahead", "identical"}, "release source is not in main history")
    tagged = tag_commit(tag)
    require(tagged in {None, sha}, "existing tag points at another commit")
    require(event != "push" or tagged == sha, "tag push must resolve to the release source")
    no_existing_release(tag)
    return {"repository": REPOSITORY, "tag": tag, "version": version, "sha": sha,
            **verify_ci(sha, ci_run_id, ci_attempt)}


def identity(args):
    require(args.ci_run_id > 0 and args.ci_attempt > 0, "CI run and attempt must be positive integers")
    return {"repository": REPOSITORY, "tag": args.tag, "version": version_for(args.tag),
            "sha": revision(args.sha), "ci_run_id": args.ci_run_id, "ci_attempt": args.ci_attempt}


def archive_name(tag, target):
    return f"Crew-{tag}-{target}" + (".zip" if target.endswith("windows-msvc") else ".tar.gz")


def release_asset_names(tag):
    return {archive_name(tag, target) for target in TARGETS} | {
        installer_name(tag), "release-manifest.json", "SHA256SUMS"}


def write_archive(destination, top, payloads):
    if destination.suffix == ".zip":
        with zipfile.ZipFile(destination, "w", compression=zipfile.ZIP_DEFLATED) as archive:
            for name, content in payloads.items():
                info = zipfile.ZipInfo(top + "/" + name)
                info.external_attr = ((0o100755 if name.startswith(("crew", "tmuxlike")) else 0o100644) << 16)
                info.compress_type = zipfile.ZIP_DEFLATED
                archive.writestr(info, content)
    else:
        with tarfile.open(destination, "w:gz") as archive:
            for name, content in payloads.items():
                info = tarfile.TarInfo(top + "/" + name)
                info.size = len(content)
                info.mode = 0o755 if name in {"crew", "tmuxlike"} else 0o644
                archive.addfile(info, io.BytesIO(content))


def validate_archive(path, package, *, check_windows_runtime=False):
    top = f"Crew-{package['tag']}-{package['target']}/"
    expected = {top + name for name in package["files"]} | {top + "manifest.json"}
    if path.suffix == ".zip":
        with zipfile.ZipFile(path) as archive:
            require(len(archive.namelist()) == len(expected) and set(archive.namelist()) == expected,
                    "archive has unexpected or missing members")
            contents = {name[len(top):]: archive.read(name) for name in expected}
    else:
        with tarfile.open(path, "r:gz") as archive:
            members = archive.getmembers()
            require(len(members) == len(expected) and {item.name for item in members} == expected
                    and all(item.isfile() for item in members), "archive has unexpected, unsafe or missing members")
            contents = {item.name[len(top):]: archive.extractfile(item).read() for item in members}
    require(json.loads(contents.pop("manifest.json")) == package, "embedded package manifest differs")
    for name, content in contents.items():
        require(package["files"][name] == {"sha256": digest(content), "bytes": len(content)},
                "archive member digest differs: " + name)
    if check_windows_runtime and package["target"] == WINDOWS_TARGET:
        for name in ("crew.exe", "tmuxlike.exe"):
            require_static_crt(contents[name])


def package(args):
    meta = identity(args)
    checked_source(args.tag, args.sha)
    target = args.target
    ext = ".exe" if target.endswith("windows-msvc") else ""
    binaries = Path("target") / target / "release"
    payloads = {}
    for name in ["crew", "tmuxlike"]:
        path = binaries / (name + ext)
        require(path.is_file() and not path.is_symlink(), "release binary is missing: " + str(path))
        version = run(str(path.resolve()), "--version")
        require(version in {f"crew {meta['version']}", f"tmuxlike {meta['version']}"},
                "binary version does not match the tag: " + version)
        payloads[name + ext] = path.read_bytes()
        if target == WINDOWS_TARGET:
            require_static_crt(payloads[name + ext])
    for name in ["README.md", "LICENSE"]:
        payloads[name] = Path(name).read_bytes()
    package_meta = {**meta, "target": target, "rust": "1.96.1", "signed": False,
                    "runtime": "static-msvc-crt" if target == WINDOWS_TARGET else "system",
                    "files": {name: {"sha256": digest(data), "bytes": len(data)} for name, data in payloads.items()}}
    payloads["manifest.json"] = json_bytes(package_meta)
    output = Path(args.output)
    output.mkdir(parents=True, exist_ok=True)
    archive = output / archive_name(args.tag, target)
    sidecar = output / f"Crew-{args.tag}-{target}.json"
    require(not archive.exists() and not sidecar.exists(), "package output already exists")
    write_archive(archive, f"Crew-{args.tag}-{target}", payloads)
    validate_archive(archive, package_meta)
    content = archive.read_bytes()
    sidecar.write_bytes(json_bytes({"package": package_meta, "archive": {
        "name": archive.name, "sha256": digest(content), "bytes": len(content)}}))
    print(f"Packaged and verified {archive}")


def collect_packages(folder, meta):
    expected = {archive_name(meta["tag"], target) for target in TARGETS}
    expected |= {f"Crew-{meta['tag']}-{target}.json" for target in TARGETS}
    expected |= {installer_name(meta["tag"]), installer_name(meta["tag"]).replace(".exe", ".json")}
    require({path.name for path in folder.iterdir()} == expected
            and all(path.is_file() and not path.is_symlink() for path in folder.iterdir()),
            "expected exactly four portable archives, one installer and their five manifests")
    records = []
    windows_package = None
    for target in TARGETS:
        sidecar = json.loads((folder / f"Crew-{meta['tag']}-{target}.json").read_bytes())
        package_meta, record = sidecar["package"], sidecar["archive"]
        require(all(package_meta.get(key) == value for key, value in meta.items())
                and package_meta["target"] == target and package_meta["rust"] == "1.96.1"
                and package_meta["signed"] is False
                and package_meta.get("runtime") == ("static-msvc-crt" if target == WINDOWS_TARGET else "system"),
                "package source/version/CI/target/runtime provenance differs")
        ext = ".exe" if target.endswith("windows-msvc") else ""
        require(set(package_meta["files"]) == {"crew" + ext, "tmuxlike" + ext, "README.md", "LICENSE"},
                "package does not contain both executables and license/readme")
        require(record["name"] == archive_name(meta["tag"], target), "unexpected archive name")
        archive = folder / record["name"]
        content = archive.read_bytes()
        require(record == {"name": archive.name, "sha256": digest(content), "bytes": len(content)},
                "package archive digest differs")
        validate_archive(archive, package_meta, check_windows_runtime=True)
        records.append({**record, "target": target, "runtime": package_meta["runtime"], "files": package_meta["files"]})
        if target == WINDOWS_TARGET:
            windows_package = package_meta
    installer = json.loads((folder / installer_name(meta["tag"]).replace(".exe", ".json")).read_bytes())
    validate_installer(folder, installer, meta, windows_package["files"])
    embedded = json_bytes(windows_package)
    require(installer["files"]["manifest.json"] == {"sha256": digest(embedded), "bytes": len(embedded)},
            "installer embedded manifest differs from its portable package")
    return records, [installer]


def aggregate(args):
    meta = identity(args)
    records, installers = collect_packages(Path(args.input), meta)
    output = Path(args.output)
    output.mkdir(parents=True, exist_ok=True)
    require(not list(output.iterdir()), "release output must be empty")
    for record in [*records, *installers]:
        shutil.copyfile(Path(args.input) / record["name"], output / record["name"])
    manifest = {**meta, "schema": 2, "rust": "1.96.1", "signed": False,
                "packages": records, "installers": installers}
    (output / "release-manifest.json").write_bytes(json_bytes(manifest))
    checksums = "".join(f"{digest(path.read_bytes())}  {path.name}\n" for path in sorted(output.iterdir()))
    (output / "SHA256SUMS").write_text(checksums, encoding="utf-8")
    print("Verified four portable archives and the native-tested Windows installer; wrote release-manifest.json and SHA256SUMS")


def finalize_draft(release_id, meta, expected_assets):
    """No publication if uploads or CI changed while the draft was being prepared."""
    assets = api(f"/repos/{REPOSITORY}/releases/{release_id}/assets?per_page=100")
    require(len(assets) == len(expected_assets) and {asset["name"] for asset in assets} == set(expected_assets),
            "draft release is missing assets")
    for asset in assets:
        expected = expected_assets[asset["name"]]
        require(asset.get("state") == "uploaded" and asset.get("size") == expected["size"]
                and asset.get("digest") == expected["digest"],
                "draft asset state, size or digest changed: " + asset["name"])
    require(tag_commit(meta["tag"]) == meta["sha"], "release tag changed during uploads; draft was preserved")
    # This read-only check deliberately excludes no_existing_release: our own draft exists.
    verify_ci(meta["sha"], meta["ci_run_id"], meta["ci_attempt"])
    published = api(f"/repos/{REPOSITORY}/releases/{release_id}", method="PATCH", value={"draft": False})
    require(published["draft"] is False, "GitHub did not publish the completed draft")
    return published


def publish(args):
    meta = identity(args)
    evidence = verify_source(args.tag, args.sha, args.ci_run_id, args.ci_attempt)
    folder = Path(args.input)
    manifest = json.loads((folder / "release-manifest.json").read_bytes())
    require(all(manifest.get(key) == value for key, value in meta.items()), "release manifest provenance differs")
    require(manifest.get("schema") == 2 and manifest.get("rust") == "1.96.1" and manifest.get("signed") is False,
            "release manifest schema/toolchain/signature policy differs")
    require({item["target"] for item in manifest["packages"]} == set(TARGETS)
            and len(manifest["packages"]) == len(TARGETS), "release manifest must contain every target exactly once")
    expected = release_asset_names(args.tag)
    require({path.name for path in folder.iterdir()} == expected
            and all(path.is_file() and not path.is_symlink() for path in folder.iterdir()),
            "release asset set is incomplete, unsafe or has unexpected files")
    lines = (folder / "SHA256SUMS").read_text(encoding="utf-8").splitlines()
    sums = {}
    for line in lines:
        value, name = line.split("  ", 1)
        require(name not in sums and re.fullmatch(r"[0-9a-f]{64}", value), "invalid or duplicate checksum record")
        sums[name] = value
    require(set(sums) == expected - {"SHA256SUMS"}, "checksums do not cover all release assets")
    for name, value in sums.items():
        require(digest((folder / name).read_bytes()) == value, "release checksum differs: " + name)
    for record in manifest["packages"]:
        require(record["name"] == archive_name(args.tag, record["target"])
                and record["sha256"] == sums[record["name"]]
                and record["bytes"] == (folder / record["name"]).stat().st_size,
                "release manifest archive size or digest differs")
        package_meta = {**meta, "target": record["target"], "rust": "1.96.1", "signed": False,
                        "runtime": record["runtime"], "files": record["files"]}
        require(package_meta["runtime"] == ("static-msvc-crt" if record["target"] == WINDOWS_TARGET else "system"),
                "release package runtime policy differs")
        validate_archive(folder / record["name"], package_meta, check_windows_runtime=True)
    installers = manifest.get("installers", [])
    require(len(installers) == 1, "release must contain exactly one Windows installer")
    windows = next(record for record in manifest["packages"] if record["target"] == WINDOWS_TARGET)
    validate_installer(folder, installers[0], meta, windows["files"])
    embedded = json_bytes({**meta, "target": WINDOWS_TARGET, "rust": "1.96.1", "signed": False,
                           "runtime": "static-msvc-crt", "files": windows["files"]})
    require(installers[0]["files"]["manifest.json"] == {"sha256": digest(embedded), "bytes": len(embedded)},
            "installer embedded manifest differs from its portable package")
    require(installers[0]["sha256"] == sums[installer_name(args.tag)], "installer checksum differs")
    expected_assets = {name: {"size": (folder / name).stat().st_size,
                              "digest": "sha256:" + digest((folder / name).read_bytes())}
                       for name in expected}
    # Only after source, CI and all seven assets pass do we create a tag/draft.
    if tag_commit(args.tag) is None:
        api(f"/repos/{REPOSITORY}/git/refs", method="POST", value={"ref": "refs/tags/" + args.tag, "sha": args.sha})
    require(tag_commit(args.tag) == args.sha, "release tag changed before publication")
    notes = (f"Crew {args.tag}\n\nSource: https://github.com/{REPOSITORY}/commit/{args.sha}\n"
             f"Verified CI: {evidence['ci_url']}\n\n"
             "Four native portable packages include the crew command, the tmuxlike compatibility command, "
             "README.md, LICENSE and a package manifest. Checksums and source/CI provenance are "
             "in SHA256SUMS and release-manifest.json.\n\n"
             "Targets: Linux x86_64, Windows x86_64, macOS arm64 and macOS x86_64. "
             "Windows also includes a per-user Inno Setup installer, tested with real 0.3.0 binary upgrades. "
             "It preserves settings/data and does not modify PATH or stop running Crew sessions. "
             "The binaries and installer are unsigned. Windows binaries use the static MSVC runtime.\n\n"
             f"Windows installation guide: https://github.com/{REPOSITORY}/blob/{args.tag}/docs/windows-install.md\n")
    release = api(f"/repos/{REPOSITORY}/releases", method="POST", value={
        "tag_name": args.tag, "target_commitish": args.sha, "name": "Crew " + args.tag,
        "body": notes, "draft": True, "prerelease": False,
    })
    release_id = int(release["id"])
    print(f"Created draft {release['html_url']}; it remains a draft if an upload fails", flush=True)
    for name in sorted(expected):
        content = (folder / name).read_bytes()
        require(expected_assets[name] == {"size": len(content), "digest": "sha256:" + digest(content)},
                "local release asset changed before upload: " + name)
        path = f"/repos/{REPOSITORY}/releases/{release_id}/assets?name={urllib.parse.quote(name, safe='')}"
        asset = api(path, method="POST", raw=content, upload=True)
        require(asset["name"] == name and asset["size"] == len(content) and asset["state"] == "uploaded",
                "release asset upload did not complete: " + name)
        require(asset.get("digest") in {None, "sha256:" + digest(content)}, "GitHub asset digest differs: " + name)
    published = finalize_draft(release_id, meta, expected_assets)
    print("Published " + published["html_url"])


class ReleaseChecks(unittest.TestCase):
    @staticmethod
    def fixture_pe(dll=b"kernel32.dll"):
        # Parser fixture only: it is not executable and never represents a native build.
        import struct
        data = bytearray(0x600)
        data[:2] = b"MZ"
        struct.pack_into("<I", data, 0x3c, 0x80)
        data[0x80:0x84] = b"PE\0\0"
        struct.pack_into("<HH", data, 0x84, 0x8664, 1)
        struct.pack_into("<H", data, 0x94, 240)
        struct.pack_into("<H", data, 0x98, 0x20b)
        struct.pack_into("<I", data, 0x98 + 60, 0x200)
        struct.pack_into("<I", data, 0x98 + 108, 16)
        struct.pack_into("<II", data, 0x98 + 112 + 8, 0x1000, 40)
        struct.pack_into("<IIII", data, 0x188 + 8, 0x400, 0x1000, 0x400, 0x200)
        struct.pack_into("<IIIII", data, 0x200, 0, 0, 0, 0x1080, 0)
        data[0x280:0x280 + len(dll) + 1] = dll + b"\0"
        return bytes(data)

    @classmethod
    def fixture_installer(cls, folder, meta, windows_package):
        binary = cls.fixture_pe()
        name = installer_name(meta["tag"])
        (folder / name).write_bytes(binary)
        embedded = json_bytes(windows_package)
        record = {**meta, "name": name, "sha256": digest(binary), "bytes": len(binary),
                  "target": WINDOWS_TARGET, "kind": "inno-setup", "app_id": APP_ID,
                  "scope": "current-user", "modifies_path": False, "runtime": "static-msvc-crt",
                  "signed": False, "compiler": INNO,
                  "files": {**windows_package["files"], "manifest.json": {
                      "bytes": len(embedded), "sha256": digest(embedded)}},
                  "validation": {"platform": "win32", "simulation": False,
                                 "installer_sha256": digest(binary), "previous_version": "0.3.0",
                                 "previous_portable_sha256": LEGACY["sha256"],
                                 "installed_version": meta["version"],
                                 "checks": [{"name": name, "result": "PASS"} for name in INSTALLER_CHECKS]}}
        # Mock contract data; this helper does not compile/run an installer or emit release evidence.
        (folder / name.replace(".exe", ".json")).write_bytes(json_bytes(record))
        return record

    @classmethod
    def fixture_packages(cls, folder, meta):
        for target in TARGETS:
            ext = ".exe" if target == WINDOWS_TARGET else ""
            binary = cls.fixture_pe() if target == WINDOWS_TARGET else b"fixture"
            payloads = {"crew" + ext: binary, "tmuxlike" + ext: binary,
                        "README.md": b"readme", "LICENSE": b"license"}
            package_meta = {**meta, "target": target, "rust": "1.96.1", "signed": False,
                            "runtime": "static-msvc-crt" if target == WINDOWS_TARGET else "system",
                            "files": {name: {"sha256": digest(data), "bytes": len(data)}
                                      for name, data in payloads.items()}}
            payloads["manifest.json"] = json_bytes(package_meta)
            archive = folder / archive_name(meta["tag"], target)
            write_archive(archive, f"Crew-{meta['tag']}-{target}", payloads)
            content = archive.read_bytes()
            sidecar = folder / f"Crew-{meta['tag']}-{target}.json"
            sidecar.write_bytes(json_bytes({"package": package_meta, "archive": {
                "name": archive.name, "sha256": digest(content), "bytes": len(content)}}))
            if target == WINDOWS_TARGET:
                windows_package = package_meta
        return cls.fixture_installer(folder, meta, windows_package)

    def test_tags_and_revisions_are_literal(self):
        self.assertEqual(version_for("v0.3.0"), "0.3.0")
        for tag in ["v01.3.0", "v0.3", "v0.3.0;echo", "--help", "v0.3.0\n"]:
            with self.assertRaises(ValueError):
                version_for(tag)
        with self.assertRaises(ValueError):
            revision("main")

    def test_gate_requires_every_job_and_test_step(self):
        jobs = [{"name": name, "status": "completed", "conclusion": "success", "steps": [
            {"name": step, "status": "completed", "conclusion": "success"}
            for step in required_ci_steps(name)
        ]} for name in sorted(CI_PLATFORMS | {"Crew CI Gate"})]
        validate_ci_jobs(jobs)
        with self.assertRaises(ValueError):
            validate_ci_jobs(jobs[:-1])
        for state in ["skipped", "cancelled", "failure", "neutral"]:
            changed = json.loads(json.dumps(jobs))
            changed[1]["conclusion"] = state
            with self.assertRaises(ValueError):
                validate_ci_jobs(changed)
            changed = json.loads(json.dumps(jobs))
            changed[1]["steps"][0]["conclusion"] = state
            with self.assertRaises(ValueError):
                validate_ci_jobs(changed)

    def test_windows_installer_steps_are_required_only_on_windows(self):
        jobs = [{"name": name, "status": "completed", "conclusion": "success", "steps": [
            {"name": step, "status": "completed", "conclusion": "success"}
            for step in required_ci_steps(name)
        ]} for name in sorted(CI_PLATFORMS | {"Crew CI Gate"})]
        validate_ci_jobs(jobs)
        for step_name in WINDOWS_CI_STEPS:
            for mode in ["missing", "skipped"]:
                changed = json.loads(json.dumps(jobs))
                windows = next(job for job in changed if job["name"] == "Windows x86_64")
                if mode == "missing":
                    windows["steps"] = [step for step in windows["steps"] if step["name"] != step_name]
                else:
                    next(step for step in windows["steps"] if step["name"] == step_name)["conclusion"] = "skipped"
                with self.assertRaises(ValueError):
                    validate_ci_jobs(changed)

    def test_compiler_version_uses_engine_and_rejects_invalid_results(self):
        import windows_installer as installer
        # Mock subprocess results only: no PowerShell/compiler is run on Linux.
        # The exact upstream launcher's resource is 0.0.0.0; its engine is 7.1.0.
        metadata = "ISCC launcher FileVersion=0.0.0.0; ProductVersion=0.0.0.0\n"
        cases = [("7.1.0\r\n", 0, True), ("0.0.0.0", 0, False), ("7.0.0", 0, False),
                 ("7.1.1", 0, False), ("7.1.0-beta", 0, False), ("", 0, False),
                 ("banner\n7.1.0", 0, False), ("7.1.0", 1, False)]
        for observed, exit_code, success in cases:
            with self.subTest(observed=observed, exit_code=exit_code), tempfile.TemporaryDirectory(prefix="crew-inno-version-") as temporary:
                logs = Path(temporary)
                binary = logs / "compiler space" / "ISCC.exe"
                results = [subprocess.CompletedProcess([], 0, metadata, ""),
                           subprocess.CompletedProcess([], exit_code, observed, "engine load failed" if exit_code else "")]
                stdout = io.StringIO()
                with mock.patch.object(installer.subprocess, "run", side_effect=results) as run_mock, mock.patch("sys.stdout", stdout):
                    if success:
                        installer.verify_compiler_version(binary, logs)
                    else:
                        with self.assertRaises(ValueError) as failure:
                            installer.verify_compiler_version(binary, logs)
                        if exit_code == 0:
                            self.assertIn("observed " + repr(observed), str(failure.exception))
                            self.assertIn("expected '7.1.0'", str(failure.exception))
                self.assertEqual(run_mock.call_count, 2)
                self.assertEqual(run_mock.call_args_list[1].args[0], [str(binary), "--version"])
                self.assertEqual((logs / "inno-version.log").read_text(), metadata)
                self.assertIn(observed.replace("\r\n", "\n"), (logs / "inno-engine-version.log").read_text())
                self.assertIn("FileVersion=0.0.0.0", stdout.getvalue())
                if exit_code == 0:
                    self.assertIn("expected '7.1.0'", stdout.getvalue())

    def test_pe_imports_reject_dynamic_crt_and_delay_imports(self):
        import struct
        self.assertEqual(require_static_crt(self.fixture_pe()), ["kernel32.dll"])
        for name in [b"VCRUNTIME140.dll", b"MSVCP140.dll", b"ucrtbase.dll", b"api-ms-win-crt-runtime-l1-1-0.dll"]:
            with self.subTest(name=name), self.assertRaises(ValueError):
                require_static_crt(self.fixture_pe(name))
        delayed = bytearray(self.fixture_pe())
        struct.pack_into("<II", delayed, 0x98 + 112 + 13 * 8, 0x1100, 64)
        struct.pack_into("<II", delayed, 0x300, 1, 0x1180)
        delayed[0x380:0x380 + len(b"VCRUNTIME140.dll\0")] = b"VCRUNTIME140.dll\0"
        with self.assertRaises(ValueError):
            require_static_crt(bytes(delayed))
        for data in [b"not a PE", self.fixture_pe()[:400], self.fixture_pe().replace(b"PE\0\0", b"NOPE", 1)]:
            with self.assertRaises(ValueError):
                pe_imports(data)

    def test_installer_requires_matching_payload_and_actual_lifecycle_contract(self):
        meta = {"repository": REPOSITORY, "tag": "v0.4.0", "version": "0.4.0", "sha": "a" * 40,
                "ci_run_id": 7, "ci_attempt": 1}
        windows = {**meta, "files": {name: {"bytes": 1, "sha256": digest(b"x")}
                                     for name in ["crew.exe", "tmuxlike.exe", "README.md", "LICENSE"]}}
        with tempfile.TemporaryDirectory(prefix="crew-installer-contract-") as temporary:
            folder = Path(temporary)
            good = self.fixture_installer(folder, meta, windows)
            validate_installer(folder, good, meta, windows["files"])
            for mode in ["missing", "skipped", "simulation", "wrong-version", "wrong-old-archive",
                         "wrong-setup", "wrong-source", "wrong-compiler", "wrong-payload", "wrong-scope"]:
                changed = json.loads(json.dumps(good))
                if mode == "missing":
                    changed["validation"]["checks"].pop()
                elif mode == "skipped":
                    changed["validation"]["checks"][0]["result"] = "NOT_RUN"
                elif mode == "simulation":
                    changed["validation"]["simulation"] = True
                elif mode == "wrong-version":
                    changed["validation"]["installed_version"] = "0.3.0"
                elif mode == "wrong-old-archive":
                    changed["validation"]["previous_portable_sha256"] = "b" * 64
                elif mode == "wrong-setup":
                    changed["validation"]["installer_sha256"] = "b" * 64
                elif mode == "wrong-source":
                    changed["sha"] = "b" * 40
                elif mode == "wrong-compiler":
                    changed["compiler"]["sha256"] = "b" * 64
                elif mode == "wrong-payload":
                    changed["files"]["crew.exe"]["sha256"] = "b" * 64
                elif mode == "wrong-scope":
                    changed["scope"] = "all-users"
                with self.subTest(mode=mode), self.assertRaises(ValueError):
                    validate_installer(folder, changed, meta, windows["files"])

    def test_shortcut_unicode_transport_and_physical_identity(self):
        import base64
        from windows_installer_smoke import (
            SHORTCUT_READ_ENCODED, SHORTCUT_READ_SCRIPT, decode_shortcut_values, validate_shortcut_values,
        )

        self.assertEqual(base64.b64decode(SHORTCUT_READ_ENCODED, validate=True).decode("utf-16le"), SHORTCUT_READ_SCRIPT)

        # These are real filesystem identity checks, not a simulation of native COM.
        with tempfile.TemporaryDirectory(prefix="crew-shortcut-check-") as temporary:
            root = Path(temporary)
            install = root / "fresh 安裝 space"
            install.mkdir()
            binary = install / "crew.exe"
            binary.write_bytes(b"same-content payload")
            alias = root / "binary-alias.exe"
            os.link(binary, alias)
            other = root / "different-file.exe"
            other.write_bytes(binary.read_bytes())
            profile = root / "user profile 使用者"
            profile.mkdir()
            values = {"reader": "IShellLinkW", "expected_user_profile": str(profile),
                      "wide": {"target": str(binary), "working_directory": str(profile), "arguments": ""},
                      "wsh": {"target": str(binary).replace("安裝", "??")}}
            encoded = "CREW_SHORTCUT_DATA " + base64.b64encode(json.dumps(values, ensure_ascii=False).encode("utf-8")).decode("ascii")
            self.assertTrue(encoded.isascii())
            decoded = decode_shortcut_values(encoded)
            self.assertEqual(decoded, values)
            validate_shortcut_values(decoded, binary)
            alias_values = json.loads(json.dumps(values))
            alias_values["wide"]["target"] = str(alias)
            validate_shortcut_values(alias_values, binary)
            for mode in ["different-file", "missing", "lossy", "quoted", "empty", "relative", "wrong-cwd", "missing-profile", "arguments", "wrong-reader"]:
                changed = json.loads(json.dumps(values))
                if mode == "different-file":
                    changed["wide"]["target"] = str(other)
                elif mode == "missing":
                    changed["wide"]["target"] = str(root / "missing.exe")
                elif mode == "lossy":
                    changed["wide"]["target"] = values["wsh"]["target"]
                elif mode == "quoted":
                    changed["wide"]["target"] = '"' + str(binary) + '"'
                elif mode == "empty":
                    changed["wide"]["target"] = ""
                elif mode == "relative":
                    changed["wide"]["target"] = "crew.exe"
                elif mode == "wrong-cwd":
                    changed["wide"]["working_directory"] = str(install)
                elif mode == "missing-profile":
                    changed["expected_user_profile"] = ""
                elif mode == "arguments":
                    changed["wide"]["arguments"] = "--unexpected"
                elif mode == "wrong-reader":
                    changed["reader"] = "WSH"
                with self.subTest(mode=mode), self.assertRaises(ValueError):
                    validate_shortcut_values(changed, binary)
            for invalid in ["CREW_SHORTCUT_OK", "noise\n" + encoded, encoded + "\nnoise", "CREW_SHORTCUT_DATA ***"]:
                with self.subTest(output=invalid), self.assertRaises(ValueError):
                    decode_shortcut_values(invalid)

    def test_archives_preserve_members_and_reject_changed_payloads(self):
        # Offline fixtures test archive validation, never claim a real platform build.
        for suffix in [".tar.gz", ".zip"]:
            with self.subTest(suffix=suffix), tempfile.TemporaryDirectory(prefix="crew-package-test-") as temporary:
                target = TARGETS[1] if suffix == ".zip" else TARGETS[0]
                ext = ".exe" if suffix == ".zip" else ""
                payloads = {"crew" + ext: b"test executable", "tmuxlike" + ext: b"test alias",
                            "README.md": b"readme", "LICENSE": b"license"}
                package_meta = {"tag": "v0.3.0", "target": target, "files": {
                    name: {"sha256": digest(data), "bytes": len(data)} for name, data in payloads.items()}}
                payloads["manifest.json"] = json_bytes(package_meta)
                archive = Path(temporary) / ("fixture" + suffix)
                top = "Crew-v0.3.0-" + target
                write_archive(archive, top, payloads)
                validate_archive(archive, package_meta)
                payloads["README.md"] = b"tampered"
                write_archive(archive, top, payloads)
                with self.assertRaises(ValueError):
                    validate_archive(archive, package_meta)
                payloads["README.md"] = b"readme"
                payloads["../extra"] = b"unsafe member"
                write_archive(archive, top, payloads)
                with self.assertRaises(ValueError):
                    validate_archive(archive, package_meta)

    def test_collection_requires_all_targets_and_matching_provenance(self):
        meta = {"repository": REPOSITORY, "tag": "v0.4.0", "version": "0.4.0", "sha": "a" * 40,
                "ci_run_id": 1, "ci_attempt": 1}
        with tempfile.TemporaryDirectory(prefix="crew-collection-test-") as temporary:
            folder = Path(temporary)
            with self.assertRaises(ValueError):
                collect_packages(folder, meta)
            installer = self.fixture_packages(folder, meta)
            self.assertEqual(len(collect_packages(folder, meta)[0]), 4)
            installer_path = folder / installer_name(meta["tag"]).replace(".exe", ".json")
            for field in ["validation", "compiler"]:
                missing = {key: value for key, value in installer.items() if key != field}
                installer_path.write_bytes(json_bytes(missing))
                with self.assertRaises(ValueError):
                    collect_packages(folder, meta)
            installer_path.write_bytes(json_bytes(installer))
            with self.assertRaises(ValueError):
                collect_packages(folder, {**meta, "sha": "b" * 40})
            with self.assertRaises(ValueError):
                collect_packages(folder, {**meta, "ci_attempt": 2})
            archive = folder / archive_name(meta["tag"], TARGETS[0])
            archive.write_bytes(archive.read_bytes() + b"tampered")
            with self.assertRaises(ValueError):
                collect_packages(folder, meta)
            archive.unlink()
            with self.assertRaises(ValueError):
                collect_packages(folder, meta)

    def test_aggregate_and_publish_require_seven_assets_and_installer_manifest(self):
        # All payloads and GitHub calls are fixtures. This never builds, installs or publishes.
        meta = {"repository": REPOSITORY, "tag": "v0.4.0", "version": "0.4.0", "sha": "a" * 40,
                "ci_run_id": 7, "ci_attempt": 1}
        modes = ["success", "missing-setup", "skipped-native", "wrong-embedded-manifest", "wrong-archive-size"]
        for mode in modes:
            with self.subTest(mode=mode), tempfile.TemporaryDirectory(prefix="crew-publication-fixture-") as temporary:
                root = Path(temporary)
                source = root / "input"
                output = root / "output"
                source.mkdir()
                self.fixture_packages(source, meta)
                args = argparse.Namespace(tag=meta["tag"], sha=meta["sha"], ci_run_id=7, ci_attempt=1,
                                          input=str(source), output=str(output))
                with mock.patch("sys.stdout", new=io.StringIO()):
                    aggregate(args)
                self.assertEqual({path.name for path in output.iterdir()}, release_asset_names(meta["tag"]))
                manifest_path = output / "release-manifest.json"
                manifest = json.loads(manifest_path.read_bytes())
                if mode == "missing-setup":
                    (output / installer_name(meta["tag"])).unlink()
                elif mode == "skipped-native":
                    manifest["installers"][0]["validation"]["checks"][0]["result"] = "NOT_RUN"
                elif mode == "wrong-embedded-manifest":
                    manifest["installers"][0]["files"]["manifest.json"]["sha256"] = "b" * 64
                elif mode == "wrong-archive-size":
                    manifest["packages"][0]["bytes"] += 1
                manifest_path.write_bytes(json_bytes(manifest))
                sums = "".join(f"{digest(path.read_bytes())}  {path.name}\n" for path in sorted(output.iterdir())
                               if path.name != "SHA256SUMS")
                (output / "SHA256SUMS").write_text(sums, encoding="utf-8")
                uploads, writes = [], []

                def fake_api(path, *, method="GET", value=None, raw=None, **unused):
                    self.assertEqual(method, "POST")
                    writes.append(path)
                    if raw is not None:
                        name = urllib.parse.parse_qs(urllib.parse.urlparse(path).query)["name"][0]
                        uploads.append(name)
                        return {"name": name, "size": len(raw), "state": "uploaded", "digest": "sha256:" + digest(raw)}
                    return {"id": 11, "html_url": "https://example.invalid/fixture"}

                final = mock.Mock(return_value={"html_url": "https://example.invalid/fixture"})
                args.input = str(output)
                with mock.patch.dict(globals(), {"api": fake_api, "tag_commit": lambda tag: meta["sha"],
                                                "verify_source": lambda *a: {"ci_url": "https://example.invalid/ci"},
                                                "finalize_draft": final}), mock.patch("sys.stdout", new=io.StringIO()):
                    if mode == "success":
                        publish(args)
                    else:
                        with self.assertRaises(ValueError):
                            publish(args)
                self.assertEqual(set(uploads), release_asset_names(meta["tag"]) if mode == "success" else set())
                self.assertEqual(len(writes), 8 if mode == "success" else 0)
                self.assertEqual(final.call_count, 1 if mode == "success" else 0)

    def test_final_publication_rechecks_ci_and_remote_assets(self):
        # The entire GitHub boundary is mocked. No fixture can make a remote request.
        sha = "a" * 40
        meta = {"tag": "v0.3.0", "sha": sha, "ci_run_id": 7, "ci_attempt": 1}
        names = release_asset_names(meta["tag"])
        expected = {name: {"size": len(name), "digest": "sha256:" + digest(name.encode())} for name in names}
        modes = ["success", "manual_success", "ci_pr_event", "ci_wrong_branch", "ci_wrong_repository",
                 "ci_failed", "ci_running", "ci_new_attempt", "ci_new_run",
                 "ci_job_skipped", "ci_step_skipped", "asset_missing", "asset_state",
                 "asset_size", "asset_digest", "asset_digest_missing", "tag_changed"]
        for mode in modes:
            with self.subTest(mode=mode):
                assets = [{"name": name, "state": "uploaded", **record} for name, record in expected.items()]
                jobs = [{"name": name, "status": "completed", "conclusion": "success", "steps": [
                    {"name": step, "status": "completed", "conclusion": "success"}
                    for step in required_ci_steps(name)
                ]} for name in sorted(CI_PLATFORMS | {"Crew CI Gate"})]
                if mode == "asset_missing":
                    assets.pop()
                elif mode == "asset_state":
                    assets[0]["state"] = "starter"
                elif mode == "asset_size":
                    assets[0]["size"] += 1
                elif mode == "asset_digest":
                    assets[0]["digest"] = "sha256:" + "b" * 64
                elif mode == "asset_digest_missing":
                    assets[0].pop("digest")
                elif mode == "ci_job_skipped":
                    jobs[1]["conclusion"] = "skipped"
                elif mode == "ci_step_skipped":
                    jobs[1]["steps"][0]["conclusion"] = "skipped"
                writes = []

                def fake_api(path, *, method="GET", value=None, **unused):
                    if method == "PATCH":
                        writes.append((path, value))
                        return {"draft": False, "html_url": "https://example.invalid/fixture"}
                    if "/assets?" in path:
                        return assets
                    if "/actions/workflows/ci.yml/runs?" in path:
                        return {"workflow_runs": [{
                            "id": 8 if mode == "ci_new_run" else 7, "head_sha": sha,
                            "head_branch": "feature" if mode == "ci_wrong_branch" else "main",
                            "event": "workflow_dispatch" if mode == "manual_success" else "pull_request" if mode == "ci_pr_event" else "push",
                            "head_repository": {"full_name": "fixture/other" if mode == "ci_wrong_repository" else REPOSITORY},
                            "status": "in_progress" if mode == "ci_running" else "completed",
                            "conclusion": "failure" if mode == "ci_failed" else "success",
                            "run_attempt": 2 if mode in {"ci_failed", "ci_new_attempt"} else 1,
                        }]}
                    if "/actions/runs/7/attempts/1/jobs?" in path:
                        return {"total_count": len(jobs), "jobs": jobs}
                    raise AssertionError("unexpected mocked GitHub request: " + path)

                with mock.patch.dict(globals(), {"api": fake_api,
                                                "tag_commit": lambda tag: "b" * 40 if mode == "tag_changed" else sha}):
                    if mode in {"success", "manual_success"}:
                        self.assertFalse(finalize_draft(11, meta, expected)["draft"])
                    else:
                        with self.assertRaises(ValueError):
                            finalize_draft(11, meta, expected)
                self.assertEqual(len(writes), 1 if mode in {"success", "manual_success"} else 0)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)
    commands.add_parser("self-test", help="run offline release validation regression tests")
    verify = commands.add_parser("verify-source", help="read-only GitHub source and CI verification")
    verify.add_argument("--tag", required=True)
    verify.add_argument("--github-output", type=Path)
    for name in ["package", "aggregate", "publish"]:
        sub = commands.add_parser(name)
        sub.add_argument("--tag", required=True)
        sub.add_argument("--sha", required=True)
        sub.add_argument("--ci-run-id", required=True, type=int)
        sub.add_argument("--ci-attempt", required=True, type=int)
        if name == "package":
            sub.add_argument("--target", choices=TARGETS, required=True)
            sub.add_argument("--output", default="dist")
        else:
            sub.add_argument("--input", required=True)
        if name == "aggregate":
            sub.add_argument("--output", required=True)
    args = parser.parse_args()
    if args.command == "self-test":
        result = unittest.TextTestRunner(verbosity=2).run(unittest.defaultTestLoader.loadTestsFromTestCase(ReleaseChecks))
        require(result.wasSuccessful(), "release tooling regression test failed")
    elif args.command == "verify-source":
        evidence = verify_source(args.tag)
        if args.github_output:
            with args.github_output.open("a", encoding="utf-8") as stream:
                for key in ["sha", "tag", "ci_run_id", "ci_attempt"]:
                    stream.write(f"{key}={evidence[key]}\n")
        print(json.dumps(evidence, indent=2))
    else:
        {"package": package, "aggregate": aggregate, "publish": publish}[args.command](args)


if __name__ == "__main__":
    try:
        main()
    except (ValueError, KeyError, OSError, subprocess.SubprocessError) as error:
        print("Release error: " + str(error), file=sys.stderr)
        sys.exit(1)
