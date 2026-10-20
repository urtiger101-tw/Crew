"""Pure validation shared by the Windows installer and the release publisher."""
import hashlib
import re
import struct

TARGET = "x86_64-pc-windows-msvc"
APP_ID = "urtiger101-tw.Crew"
INNO = {
    "version": "7.1.0",
    "url": "https://github.com/jrsoftware/issrc/releases/download/is-7_1_0/innosetup-7.1.0-x64.exe",
    "sha256": "0362a383ed217d4c4239b5933866dd96d3eb2102737da92f80f6057a4b40df2f",
    "bytes": 14304168,
    "publisher": "Pyrsys B.V.",
}
LEGACY = {
    "version": "0.3.0",
    "url": "https://github.com/urtiger101-tw/Crew/releases/download/v0.3.0/Crew-v0.3.0-x86_64-pc-windows-msvc.zip",
    "sha256": "2d677062f83823e264ab8391ced4aa4fab1e21764d3af6032f359c917ce3dc02",
    "bytes": 2756573,
    "source_sha": "bf2190f8b15849fb1ad703110fdc53981fa0455c",
}
CHECKS = (
    "fresh-install", "command-versions", "payload-digests", "shortcut-and-hkcu",
    "path-value-and-type-preserved", "config-and-state-preserved",
    "foreign-directory-rejected", "busy-upgrade-rejected", "busy-uninstall-rejected",
    "daemon-survives", "real-0.3.0-upgrade", "downgrade-rejected",
    "uninstall-owned-files", "unowned-files-preserved",
)


def require(condition, message):
    if not condition:
        raise ValueError(message)


def sha256(content):
    return hashlib.sha256(content).hexdigest()


def installer_name(tag):
    require(re.fullmatch(r"v(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)", tag or ""),
            "installer tag must be stable SemVer")
    return f"Crew-{tag}-{TARGET}-setup.exe"


def pe_imports(data):
    """Read actual PE32+ AMD64 imports, including delay imports; fail on malformed data."""
    def unpack(fmt, offset):
        require(0 <= offset <= len(data) - struct.calcsize(fmt), "truncated PE structure")
        return struct.unpack_from(fmt, data, offset)

    require(data[:2] == b"MZ", "expected a Windows executable")
    pe, = unpack("<I", 0x3c)
    require(data[pe:pe + 4] == b"PE\0\0", "invalid PE signature")
    machine, count = unpack("<HH", pe + 4)
    require(machine == 0x8664 and 1 <= count <= 96, "expected PE AMD64 architecture")
    opt_size, = unpack("<H", pe + 20)
    opt = pe + 24
    magic, = unpack("<H", opt)
    require(magic == 0x20b and opt_size >= 112 + 14 * 8, "expected PE32+ optional header")
    directory_count, = unpack("<I", opt + 108)
    require(directory_count >= 14, "PE import directory table is incomplete")
    headers_size, = unpack("<I", opt + 60)
    sections = []
    for index in range(count):
        section = opt + opt_size + index * 40
        virtual_size, address, raw_size, raw_offset = unpack("<IIII", section + 8)
        require(raw_offset + raw_size <= len(data), "PE section is truncated")
        sections.append((address, virtual_size, raw_size, raw_offset))

    def offset(rva, length=1):
        if 0 <= rva < headers_size and rva + length <= min(headers_size, len(data)):
            return rva
        for address, _, raw_size, raw_offset in sections:
            if address <= rva and rva + length <= address + raw_size:
                return raw_offset + rva - address
        raise ValueError("PE RVA is outside backed file data")

    def name_at(rva):
        start = offset(rva)
        end = data.find(b"\0", start, min(start + 512, len(data)))
        require(end > start, "invalid PE DLL name")
        offset(rva, end - start + 1)
        name = data[start:end].decode("ascii")
        require(re.fullmatch(r"[A-Za-z0-9_.-]+\.dll", name, re.IGNORECASE), "unsafe PE DLL name")
        return name.lower()

    imports = set()
    for directory, entry_size, name_index in ((1, 20, 3), (13, 32, 1)):
        rva, size = unpack("<II", opt + 112 + directory * 8)
        if not rva and not size:
            continue
        require(rva and entry_size <= size <= 1024 * 1024, "invalid PE import directory")
        terminated = False
        for position in range(0, size - entry_size + 1, entry_size):
            values = unpack("<" + "I" * (entry_size // 4), offset(rva + position, entry_size))
            if not any(values):
                terminated = True
                break
            if directory == 13:
                require(values[0] == 1, "unsupported delay import addressing")
            imports.add(name_at(values[name_index]))
        require(terminated, "unterminated PE import directory")
    require(imports, "Windows executable has no imports")
    return sorted(imports)


def require_static_crt(data):
    imports = pe_imports(data)
    external = [name for name in imports if re.match(
        r"^(?:vcruntime|msvcp|concrt|vcomp|msvcr|ucrtbase|api-ms-win-crt-)", name)]
    require(not external, "Windows release requires static CRT; imported: " + ", ".join(external))
    return imports


def validate_record(folder, record, meta, portable_files, *, require_smoke=True):
    require(all(record.get(key) == value for key, value in meta.items()), "installer source/version/CI provenance differs")
    require(record.get("name") == installer_name(meta["tag"])
            and record.get("target") == TARGET and record.get("kind") == "inno-setup"
            and record.get("app_id") == APP_ID and record.get("scope") == "current-user"
            and record.get("modifies_path") is False and record.get("signed") is False
            and record.get("runtime") == "static-msvc-crt" and record.get("compiler") == INNO,
            "installer contract or pinned compiler differs")
    expected_names = set(portable_files) | {"manifest.json"}
    require(set(record.get("files", {})) == expected_names
            and all(record["files"].get(name) == value for name, value in portable_files.items()),
            "installer and portable payloads differ")
    for value in record["files"].values():
        require(set(value) == {"bytes", "sha256"} and isinstance(value["bytes"], int) and value["bytes"] > 0
                and re.fullmatch(r"[0-9a-f]{64}", value["sha256"]), "invalid installer payload digest")
    path = folder / record["name"]
    require(path.is_file() and not path.is_symlink(), "installer is missing or is a symlink")
    content = path.read_bytes()
    require(record.get("bytes") == len(content) and record.get("sha256") == sha256(content),
            "installer size or digest differs")
    pe_imports(content)
    if require_smoke:
        validation = record.get("validation", {})
        require(validation.get("platform") == "win32" and validation.get("simulation") is False
                and validation.get("installer_sha256") == record["sha256"]
                and validation.get("previous_portable_sha256") == LEGACY["sha256"]
                and validation.get("previous_version") == LEGACY["version"]
                and validation.get("installed_version") == meta["version"]
                and tuple(int(n) for n in meta["version"].split(".")) > (0, 3, 0),
                "native installer lifecycle evidence is missing or does not match the payload")
        checks = validation.get("checks", [])
        require(len(checks) == len(CHECKS) and {item.get("name") for item in checks} == set(CHECKS)
                and all(item.get("result") == "PASS" for item in checks),
                "installer lifecycle check is missing, skipped or unsuccessful")
    return record
