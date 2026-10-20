#!/usr/bin/env python3
"""Build Crew with a pinned, authenticated Inno Setup compiler on Windows.

Only downloads public, digest-pinned dependencies. Never publishes or installs Crew.
The compiler itself is extracted in portable/current-user mode under the job temp dir.
"""
import argparse
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import tomllib
import urllib.request
import zipfile

from windows_installer_contract import APP_ID, INNO, LEGACY, TARGET, installer_name, pe_imports, require, require_static_crt, sha256, validate_record

ROOT = Path(__file__).resolve().parents[1]


def execute(argv, *, env=None, timeout=180, log=None):
    result = subprocess.run([str(arg) for arg in argv], env=env, capture_output=True,
                            text=True, encoding="utf-8", errors="replace", timeout=timeout)
    if log is not None:
        Path(log).parent.mkdir(parents=True, exist_ok=True)
        Path(log).write_text(result.stdout + result.stderr, encoding="utf-8")
    if result.returncode:
        raise ValueError(f"Command failed ({result.returncode}): {argv[0]}\n{result.stdout}\n{result.stderr}")
    return result.stdout.strip()


def download(record, destination):
    destination = Path(destination)
    if destination.exists():
        require(destination.is_file() and not destination.is_symlink(), "unsafe dependency cache entry")
        content = destination.read_bytes()
    else:
        request = urllib.request.Request(record["url"], headers={"User-Agent": "Crew-installer-build"})
        with urllib.request.urlopen(request, timeout=120) as response:
            require(response.url.startswith("https://"), "dependency download lost HTTPS")
            content = response.read(record["bytes"] + 1)
    require(len(content) == record["bytes"] and sha256(content) == record["sha256"],
            "pinned dependency size or SHA-256 differs: " + destination.name)
    if not destination.exists():
        destination.parent.mkdir(parents=True, exist_ok=True)
        with destination.open("xb") as stream:
            stream.write(content)
    return destination


def compiler(logs):
    require(sys.platform == "win32", "Inno Setup compilation requires native Windows")
    home = Path(os.environ.get("RUNNER_TEMP", tempfile.gettempdir())) / "crew-inno-7.1.0"
    home.mkdir(parents=True, exist_ok=True)
    source = download(INNO, home / "innosetup-7.1.0-x64.exe")
    env = os.environ.copy()
    env["CREW_INNO_DOWNLOAD"] = str(source)
    execute(["powershell", "-NoLogo", "-NoProfile", "-NonInteractive", "-Command",
             "$ErrorActionPreference='Stop'; "
             "$s=Get-AuthenticodeSignature -LiteralPath $env:CREW_INNO_DOWNLOAD; "
             "if ($s.Status -ne 'Valid' -or $null -eq $s.SignerCertificate -or "
             "$s.SignerCertificate.GetNameInfo([System.Security.Cryptography.X509Certificates.X509NameType]::SimpleName,$false) -ne 'Pyrsys B.V.') "
             "{ throw 'Inno Setup signature/publisher validation failed' }; "
             "Write-Output 'Inno Setup Authenticode: Valid / Pyrsys B.V.'"], env=env,
            log=logs / "inno-signature.log")
    directory = home / "compiler"
    iscc = directory / "ISCC.exe"
    if not iscc.exists():
        execute([source, "/VERYSILENT", "/SUPPRESSMSGBOXES", "/NORESTART", "/SP-",
                 "/CURRENTUSER", "/PORTABLE=1", "/NOICONS", "/TASKS=",
                 "/DIR=" + str(directory), "/LOG=" + str(logs / "inno-extract.log")])
    require(iscc.is_file() and not iscc.is_symlink(), "pinned Inno Setup compiler was not extracted")
    env["CREW_ISCC"] = str(iscc)
    execute(["powershell", "-NoLogo", "-NoProfile", "-NonInteractive", "-Command",
             "$ErrorActionPreference='Stop'; $v=(Get-Item -LiteralPath $env:CREW_ISCC).VersionInfo; "
             "if ($v.FileMajorPart -ne 7 -or $v.FileMinorPart -ne 1 -or $v.FileBuildPart -ne 0) "
             "{ throw 'Unexpected Inno Setup compiler version' }; Write-Output $v.FileVersion"],
            env=env, log=logs / "inno-version.log")
    return iscc


def compile_payload(payloads, version, output, filename, iscc, logs):
    require(sys.platform == "win32", "installer compilation requires native Windows")
    installer_name("v" + version)
    require(all(int(part) <= 65535 for part in version.split(".")), "installer version component is too large")
    require(set(payloads) == {"crew.exe", "tmuxlike.exe", "README.md", "LICENSE", "manifest.json"},
            "installer payload must be the complete portable package")
    output = Path(output).resolve()
    output.mkdir(parents=True, exist_ok=True)
    destination = output / filename
    require(destination.parent == output and destination.suffix == ".exe" and not destination.exists(),
            "installer output must be a new executable in the output directory")
    with tempfile.TemporaryDirectory(prefix="crew-inno-payload-") as temporary:
        payload_dir = Path(temporary)
        for name, content in payloads.items():
            (payload_dir / name).write_bytes(content)
        execute([iscc, "/Qp", "/DCrewVersion=" + version,
                 "/DPayloadDir=" + str(payload_dir), "/DInstallerOutputDir=" + str(output),
                 "/DInstallerBaseName=" + destination.stem, ROOT / "installer/windows/crew.iss"],
                log=logs / (destination.stem + "-compile.log"))
    require(destination.is_file(), "Inno Setup did not produce the expected installer")
    pe_imports(destination.read_bytes())
    return destination


def portable_payload(archive, package):
    import package_release as release
    release.validate_archive(archive, package)
    top = f"Crew-{package['tag']}-{TARGET}/"
    with zipfile.ZipFile(archive) as stream:
        return {name: stream.read(top + name) for name in [*package["files"], "manifest.json"]}


def legacy_fixture(iscc, output, logs):
    """Compile a test-only old-version installer from real, pinned 0.3.0 binaries.

    This is not a historical public installer. Crew 0.3.0 only published a portable ZIP.
    """
    archive = download(LEGACY, output / "Crew-v0.3.0-windows-portable.zip")
    with zipfile.ZipFile(archive) as stream:
        package = json.loads(stream.read(f"Crew-v0.3.0-{TARGET}/manifest.json"))
    require(package.get("repository") == "urtiger101-tw/Crew" and package.get("version") == "0.3.0"
            and package.get("tag") == "v0.3.0" and package.get("target") == TARGET
            and package.get("sha") == LEGACY["source_sha"], "legacy fixture provenance differs")
    payloads = portable_payload(archive, package)
    path = compile_payload(payloads, "0.3.0", output, "Crew-0.3.0-test-fixture.exe", iscc, logs)
    return path, payloads


def build(args):
    import package_release as release
    require(sys.platform == "win32", "Windows installer build is not applicable on this platform")
    require(Path.cwd().resolve() == ROOT, "run the installer build from the repository root")
    logs = Path(args.logs).resolve()
    logs.mkdir(parents=True, exist_ok=True)
    output = Path(args.output).resolve()
    with (ROOT / "Cargo.toml").open("rb") as stream:
        version = tomllib.load(stream)["package"]["version"]
    tag = "v" + version
    if args.package_dir:
        folder = Path(args.package_dir).resolve()
    else:
        # CI uses its actual run/attempt and tested checkout. Release reuses its verified package.
        folder = output
        ci_run_id = int(os.environ.get("GITHUB_RUN_ID", "0"))
        ci_attempt = int(os.environ.get("GITHUB_RUN_ATTEMPT", "0"))
        release.package(argparse.Namespace(tag=tag, sha=release.run("git", "rev-parse", "HEAD"),
                                          ci_run_id=ci_run_id, ci_attempt=ci_attempt,
                                          target=TARGET, output=str(folder)))
    sidecar = json.loads((folder / f"Crew-{tag}-{TARGET}.json").read_bytes())
    package = sidecar["package"]
    meta = {key: package[key] for key in ("repository", "tag", "version", "sha", "ci_run_id", "ci_attempt")}
    require(meta["repository"] == "urtiger101-tw/Crew" and meta["tag"] == tag
            and package["target"] == TARGET and meta["ci_run_id"] > 0 and meta["ci_attempt"] > 0,
            "portable package provenance differs")
    release.checked_source(tag, meta["sha"])
    archive = folder / release.archive_name(tag, TARGET)
    content = archive.read_bytes()
    require(sidecar["archive"] == {"name": archive.name, "bytes": len(content), "sha256": sha256(content)},
            "portable package archive hash differs")
    payloads = portable_payload(archive, package)
    for name in ("crew.exe", "tmuxlike.exe"):
        require_static_crt(payloads[name])
    path = compile_payload(payloads, version, output, installer_name(tag), compiler(logs), logs)
    content = path.read_bytes()
    record = {**meta, "name": path.name, "sha256": sha256(content), "bytes": len(content),
              "target": TARGET, "kind": "inno-setup", "app_id": APP_ID, "scope": "current-user",
              "modifies_path": False, "runtime": "static-msvc-crt", "signed": False, "compiler": INNO,
              "files": {name: {"bytes": len(data), "sha256": sha256(data)} for name, data in payloads.items()}}
    validate_record(output, record, meta, package["files"], require_smoke=False)
    record_path = path.with_suffix(".json")
    require(not record_path.exists(), "installer manifest already exists")
    record_path.write_bytes(release.json_bytes(record))
    print(f"Built {path}; native lifecycle validation is still required", flush=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--package-dir", help="reuse the verified release portable ZIP; omitted during CI")
    parser.add_argument("--output", default="installer-dist")
    parser.add_argument("--logs", default="installer-logs")
    args = parser.parse_args()
    try:
        build(args)
    except (ValueError, KeyError, OSError, subprocess.SubprocessError) as error:
        print("Installer build error: " + str(error), file=sys.stderr)
        for path in sorted(Path(args.logs).glob("*.log")):
            print(f"--- {path.name} ---\n" + "\n".join(path.read_text(encoding="utf-8", errors="replace").splitlines()[-50:]),
                  file=sys.stderr)
        raise SystemExit(1) from error


if __name__ == "__main__":
    main()
