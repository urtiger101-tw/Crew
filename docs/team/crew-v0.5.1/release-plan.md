# Authorized local Windows preview release

Owner: root / release engineering. Base: dcc28b9d35279fa8b290bc5c3f80ead919393f19.
Version: 0.5.1, new tag v0.5.1, Windows preview / prerelease, not latest stable.

The user explicitly requested Windows PATH setup, pushing the data, packaging and
release publication after manually confirming the local Crew runs. This grants
the requested external actions. Existing data, active Crew sessions, security
settings, disabled workflows, old tags and old release assets must be preserved.

1. Bump only package version / root lock entry and release documentation. Keep
   historical fixtures and minimum plugin versions unchanged.
2. Commit cleanly, then run the 25 unaffected canonical native Windows commands
   in a fresh target directory, including Inno build. Record the known denied
   0.3.0 lifecycle fixture as BLOCKED/executed=false; never retry it or relax a gate.
3. Independently review source version, commands, hashes, coverage, notes and the
   preview policy. Record the user's manual-smoke report separately.
4. Place the verified portable payload in a new versioned per-user directory,
   preserving the running old binary. Append only that directory to user PATH,
   preserve its value/type, keep a local backup, and verify `crew --version`.
5. With the already configured repository-owner credential scoped to the process,
   fast-forward main and create the new tag; do not change the default account.
   Create a draft, upload checksum-bound Windows payloads and a truthful native
   summary, verify them, then publish as a prerelease with latest=false.
6. Publicly download each asset and compare bytes/hashes; confirm the tag/source,
   unchanged older releases, and still-disabled CI/Release workflows with no new
   hosted run. Keep a local evidence handoff and exact publication URLs.

No source-body changes, dependency updates, Linux reruns, macOS execution or new
Herdr feature work are included. Rollback means using the preserved old directory
and PATH backup; it does not delete user files or rewrite public history.
