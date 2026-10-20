# Windows native acceptance repair

Owner: root / engineering. Baseline: 8064487793d8e7b569b2acb3ce324f64b2775ae2.
Local branch: fix/windows-native-validation. No external writes or hosted runners.

The first actual Windows run passed 23 of 26 required commands. Workspace startup
input raced PowerShell initialization; Inno signature verification selected the
parent PowerShell 7 security module in a Windows PowerShell 5.1 subprocess.
Installer lifecycle was NOT_RUN. Original evidence remains outside this checkout.

## Scope and acceptance

1. Pass one-time workspace/tab startup commands as native PowerShell/pwsh startup
   argv with -NoExit. Preserve configured argv, cwd, env, identifiers, focus and
   normal pane submission. Reject conflicting execution modes before creation.
   Other shells retain their existing input path. Do not guess prompt readiness
   or replay commands. Startup argv must not enter persistent state or splits.
2. Extend the real workspace smoke to verify Unicode/quotes/multiline startup
   commands execute once for workspace and tab creation, the shell remains
   interactive, and split/cold restore never replay them.
3. Select the signing module from Windows PowerShell's own PSHOME. Keep the exact
   digest, Valid signature, publisher and compiler-version checks. Do not change
   global PSModulePath or ExecutionPolicy.
4. Run focused native regression, then all required native commands including
   Inno compilation and all 14 isolated installer lifecycle checks. Preserve all
   failed runs and publish no package as validated until complete evidence passes.
5. Obtain independent review against the final diff and source identity. Commit
   locally before the final source-bound packaging run. No push, release, PR or
   hosted Actions run is authorized.

Write scope: server launch plumbing, workspace startup selection and tests,
workspace_smoke.py, windows_installer.py, and related local documentation.
No dependency updates or broad refactoring. Herdr feature work remains dependent
on successful native acceptance; this repair alone is not a parity claim.

Independent review identified interactive/format aliases and additional server
modes. Regression cases first failed on the initial candidate, then guided the
correction. PowerShell host behavior was checked against the local executable
and the matching [PowerShell 7.6.5 parameter parser](https://github.com/PowerShell/PowerShell/blob/v7.6.5/src/Microsoft.PowerShell.ConsoleHost/host/msh/CommandLineParameterParser.cs).

The next native run exposed transient Windows directory locks at metadata and
tab-transfer fixture teardown. Shared cleanup is bounded to five seconds, retries
only WinError 32 on the exact test-owned directory, and still fails on timeout or
other errors. The product daemon shutdown behavior is unchanged. Inno compilation
and initial install checks ran successfully; the legacy compatibility executable
was denied by Windows App Control. That operation must not be retried without a
new approved execution environment or a relevant policy decision. The full gate
and Herdr feature implementation remain blocked until that prerequisite is met.
