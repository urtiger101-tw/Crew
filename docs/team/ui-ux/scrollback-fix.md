# Scrollback repair — 2026-11-02

Request: CRL 111. Worktree: `fix/scrollback-20261102`, based on `6937522`.

## Behavior and cause

`wheel_pane` previously converted an alternate-screen wheel event into three
Up/Down escape sequences, irrespective of the child's mouse mode. A baseline
handler regression captured `1b 5b 41` three times for one upward wheel tick.
The baseline also failed to forward a requested SGR mouse event. Ordinary
Up/Down keys intentionally remain application input outside copy mode.

The wheel now browses Crew scrollback unless the application requests mouse
reporting and copy mode is inactive. Requested events use pane-local coordinates
and the application's SGR, UTF-8, or legacy encoding. Entering scrollback with
the wheel focuses the hovered pane so the displayed Esc/q exit instruction
cannot send keys to a different split.

Shift+PgUp/PgDn and `C-b [` provide explicit history entry. While an alternate
screen is active, Crew clones the parser's retained normal screen for browsing;
output continues in the live parser. The view is labeled `saved main`, and q or
Esc without an anchored selection returns to the live application. Resize,
selection copying and visible reads use the view; recent output reads continue
using the live parser. Past alternate-screen redraws were never retained by
vt100 and are not reconstructed.

## Windows runtime

This machine runs Windows 10 build 19045. Its system ConPTY consumes the
fixture's alternate-screen and mouse DECSET sequences before Crew receives
them. Consequently a basic ConPTY test cannot establish those terminal modes.
This was discovered by checking the actual emitted bytes, not by assuming that
the child's requested mode reached Crew.

The official [Microsoft.Windows.Console.ConPTY 1.25.260930003 package](https://www.nuget.org/packages/Microsoft.Windows.Console.ConPTY/1.25.260930003)
preserves those sequences in the isolated test. portable-pty already supports
loading `conpty.dll` and its matching `OpenConsole.exe` beside the executable.
`scripts/stage_windows_conpty.ps1` stages the x64 pair, verifies pinned package
and file SHA-256 values plus Microsoft Authenticode signatures, and includes
the MIT license and runtime provenance record. It does not change system
settings or restart an existing daemon.

The pair must accompany the **daemon binary**. Ordinary `cargo build` does not
stage it. The existing hosted release/installer pipeline has not been modified
to include the sidecars; a local candidate must retain the staged directory's
runtime and license files. Do not claim that an existing installed/shared daemon
uses this runtime until it has been restarted with authorization.

Microsoft's [ConPTY mouse support discussion](https://github.com/microsoft/terminal/issues/376)
and [VT input/mouse issue](https://github.com/microsoft/terminal/issues/15296)
provide upstream context; the acceptance evidence below is from this checkout
and the pinned runtime, not inferred from those issues.

## Evidence

- **RED:** baseline `6937522`, two handler tests failed: wheel generated three Up
  events; requested mouse reporting generated no SGR packet. Diagnostic test
  adaptations only remove the new fixture field and translate the old method
  signature. `target/verification/scrollback-baseline-red.log` records the run.
- **PASS:** complete library suite at the initial fix: 586 passed, 3 ignored.
- **PASS:** final five targeted handler tests cover alternate history, exact
  mouse packets, encoding bounds/modifiers, unfocused splits, snapshot resize,
  selection, and visible/recent separation.
- **PASS:** native Windows isolated daemon/PTY test with
  `CREW_TEST_MODERN_CONPTY=1`: 1 passed, 1 self-executed fixture ignored by the
  harness. All alternate-screen and SGR/default/UTF8 assertions ran, including
  exact child input and live return. Evidence and binary/runtime hashes:
  `target/verification/scrollback-modern-conpty-integration.log`.
- **PASS:** `cargo clippy --all-targets -- -D warnings`, format and diff checks.
- **PASS:** valid and matching-existing-runtime staging; tampered NuGet package
  rejected before runtime copy.
- **PASS (startup only):** separate fresh PowerShell, Claude 2.1.316 and Codex
  0.166.0 processes started in native PTYs. No prompts were submitted; this
  does not prove scrolling through actual agent conversations.
- **NOT_RUN:** physical Windows Terminal wheel/keyboard GUI acceptance for
  shell/Claude/Codex. The isolated WT window launched, but desktop automation
  did not expose it; the remote-desktop capture attempt also timed out.
- **NOT_RUN:** shared-daemon restart, production client acceptance, hosted
  CI, Linux/macOS execution, and hosted archive/installer sidecar integration.

Independent review: GPT-6.1 Terra xhigh; integration test author:
GPT-6.1 Sol xhigh. Main agent implemented the production changes and resolved
review findings. No shared daemon was restarted.
