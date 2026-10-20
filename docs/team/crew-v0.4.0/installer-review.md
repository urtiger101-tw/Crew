# INSTALL-04 independent review

Result: **PASS for the immutable source and offline contract scope**. No open P1/P2 finding. Windows native compilation and install/upgrade/uninstall acceptance are **NOT_RUN on this Linux reviewer host** and remain required release gates.

- Author: `/root/worktree_worker`; independent reviewer: `/root/reference_audit`.
- Commit: `f7b4e88867302d965198c646735e48e1e0c7ab31`
- Base dependency: `e0007a893df7fe768ba85fc6094353e45c4b37b6`
- Tree: `0275ef019a98b9ba0786b9134f8bdd02a4d6e275`
- All eight working files match their commit objects and the author's declared hashes. Worktree clean before and after checks.
- No author source changes or remote writes were performed by this reviewer.

## Source assessment

The `.iss` defines a stable `urtiger101-tw.Crew` identity, current-user mode, native x64-only eligibility, and five enumerated package files. Its install and uninstall code neither edits PATH nor stops a daemon. Existing registered installations must retain their directory and cannot be downgraded; an unregistered nonempty destination is refused before payload mutation. The busy-file check opens existing files for read/write with exclusive sharing without truncation, and returns an error before installation/uninstallation proceeds. Inno's native behavior for that guard will be exercised by the required Windows lifecycle test.

The installer compiler download is pinned by HTTPS URL, size and SHA-256, then checked for Valid Authenticode/Pyrsys B.V. before execution. An independently retrieved official immutable release API sample matches the pin. Compiler validation is distinct from the resulting Crew installer, which remains explicitly unsigned.

Windows release builds use explicit target selection with static CRT flags. Packaging reads PE32+ AMD64 imports and delay imports and rejects dynamic CRT dependencies. Portable ZIP output remains; the setup is a seventh public asset. Aggregate and publish validate exact asset inventories, actual bytes/digests, portable and installer payload identity including embedded manifest, exact source/CI provenance and complete native lifecycle evidence. Missing or skipped Windows installer steps cannot satisfy main-CI release eligibility. Failed source/artifact checks precede tag or release mutations.

The native lifecycle script uses real pinned public 0.3.0 binaries in a clearly labeled test-only installer fixture. It checks a fresh Unicode/spaced-path install, both commands, payload hashes, current-user shortcut/registration, upgrade, busy install/uninstall refusal with unchanged bytes and daemon epoch, downgrade refusal, owned-file removal and preservation of PATH values/types, settings/state and unowned files. Only its isolated test daemon is explicitly stopped after the survival checks. The original error is preserved if cleanup also fails.

The working-tree precheck identified reliance on native Unicode stdout encoding. The frozen candidate compares paths in PowerShell and sends only ASCII markers/versions across stdout, while retaining subprocess PATH resolution and execution of the resolved binary. This is closed; it was not reported as an observed Windows failure.

## Independently executed checks

| Check | Result | Evidence |
|---|---|---|
| Release tooling tests, including seven-asset aggregate/publish negative cases | 9 tests, exit 0 | `self-test.log` |
| actionlint, CI and Release workflows | exit 0 | `actionlint.log` |
| Four Python AST parses | exit 0 | `syntax.log` |
| Exact candidate diff whitespace check | exit 0 | `diff-check.log` |
| Parse both actual public 0.3.0 PE files and reject their known dynamic CRT imports | exit 0, read-only | `real-old-pe.log` |

Offline fixtures validate contract rejection behavior; they do not represent a native installer run. The old public binaries are used only as real parser inputs in this review.

## Required next evidence

Integrate this exact patch with the accepted CORE/META source, preserving the reviewed hashes or reviewing any changed resolution. The two new smoke files referenced in the workflows come from that integration. Native Windows CI must compile the `.iss`, execute all 14 lifecycle checks, and validate actual static-CRT output; Release must repeat them for its exact installer digest. Interactive UI/SmartScreen and a clean VM are not covered by this local review.

## Primary references

- [Inno Setup architecture](https://jrsoftware.org/ishelp/topic_setup_setuparchitecture.htm) and [non-administrative mode](https://jrsoftware.org/ishelp/topic_setup_privilegesrequired.htm).
- [Stable AppId and uninstall key](https://jrsoftware.org/ishelp/topic_setup_appid.htm), [append uninstall logs](https://jrsoftware.org/ishelp/topic_setup_uninstalllogmode.htm).
- [CloseApplications behavior](https://jrsoftware.org/ishelp/topic_setup_closeapplications.htm), [command-line overrides](https://jrsoftware.org/ishelp/topic_setupcmdline.htm), [TFileStream support](https://jrsoftware.org/ishelp/topic_scriptclasses.htm).
- [Microsoft Console.OutputEncoding](https://learn.microsoft.com/en-us/dotnet/api/system.console.outputencoding?view=netframework-4.8.1).

Complete source hashes, explicit NOT_RUN coverage and exact commands are in `review.json`.
