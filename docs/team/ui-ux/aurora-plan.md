# Aurora workspace chrome

Source of truth: `docs/collab/claude-codex/log/104-claude.crl`, superseding the
layout suggested in §99; the supplied phone screenshot is a color reference only.
Surface: terminal workspace / operate. Aurora is an opt-in
preset; Studio and existing API lifecycle states remain compatible. All pane
output colors stay owned by the child terminal. Shared daemon switching stays
with Claude, and no public release is requested.

## Ownership and build order

- Terra xhigh: B1 `pane submit`, bounded settle, exactly one Enter, conservative
  observed-composer confirmation and isolated PTY fixtures. Separate worktree.
- Sol xhigh: V1/V2/V3 palette, chrome color capability fallback, shared semantic
  descriptors and stable agent identity accents. Separate worktree.
- Main: V4/V6 tab pills and attention footer; V5 quiet headers and toolbar;
  B2 restored shell markers; V7 left tree and optional read-only changed files;
  integration, real terminal screenshots, documentation and native CI-equivalent
  validation. Authors do not review their own parts.

## Acceptance

- `pane submit` preserves existing run/send commands, never retries Enter and
  distinguishes an observed composer-clear heuristic from an unconfirmed result.
- Aurora supports truecolor, 256/16 color fallback, NO_COLOR and ASCII. Palette
  tests enforce body/surface contrast >=4.5:1 and muted/surface >=3:1.
- State glyph/word/color comes from one descriptor; identity color comes from a
  stable agent key, with user overrides. Canonical API states and waits unchanged.
- Aurora keeps the v0.6.0 layout. Tabs, session pill and attention chips share the
  existing bottom row. Left workspace tree and files panel default off; the legacy
  right agents sidebar retains its default. F1 toolbar defaults off in Aurora.
  The optional left tree uses `workspace_tree` / prefix b independently of the
  right agents sidebar. Pane-area budgets at 120x30 and 80x24 cannot shrink.
  Drawing and hit testing share layout, including the existing drag operations.
- Focus, keyboard navigation and narrow layouts remain usable. Inactive dimming
  affects chrome only. F1 retains toolbar toggle; Aurora configs with no explicit
  toolbar preference start with it hidden. Bare restored shells are labeled, can use the existing
  close key, and are not counted as active agent work.
- Git/inbox reads run outside the UI lock. The changed-files panel is read-only,
  disabled by default, and displays errors instead of implying a clean repository.
  Per accepted scope in §103, it explicitly displays the pane's starting directory;
  tracking shell `cd` is deferred to a separate opt-in OSC 7 integration task.
- Native screenshots at 120x30 and 200x50, plus mono/ASCII, are retained under
  `target/verification-*`. Synthetic event checks are labeled separately.
- Final gate: format, Clippy, Rust tests, local native validation, and one scoped
  independent non-author review per authored part. Fix/recheck findings only.
  Native Windows evidence does not claim Linux/macOS or hosted CI acceptance.

## Review and native evidence

- B1: Sol independent scope review PASS; Terra implemented its fixes before handoff.
- Aurora/theme/main: Terra independent review identified six concrete issues.
  Main fixed completion timing, restored-shell attention filtering, runtime theme
  bindings, toolbar defaults, and semantic display labels; limited recheck left
  only current-directory tracking, now explicitly deferred by §103.
- Native isolated daemon ran actual Claude Code 2.1.316 and Codex 0.166.0, plus
  PowerShell. Both replied exactly to benign `pane submit` prompts. Codex's short
  one-line composer returned `submitted`; wrapped input and Claude returned
  `unconfirmed`, then actual replies were read back independently. No Enter retry.
- Actual Conhost captures are in `target/verification-20261102-aurora/` at 120x30,
  200x50 and NO_COLOR+ASCII. Mono applies only to Crew chrome; child colors persist.
  Conhost's default CJK font renders some ambiguous glyphs wide; Consolas captures
  avoid overlap in Crew chrome, while missing child glyphs remain child/font-owned.
- The grey Codex bands belong to its child screen cells. Aurora leaves both
  explicit child colors and default child colors untouched.
- Commit/push ownership is main Codex per §103; no release or shared restart.
- Final gate found a pre-existing ConPTY startup mismatch: a requested 24-column
  split was created at 30 columns before resize. Replaying the captured bytes
  reproduced the wrapped-marker failure. New panes now start at the prospective
  layout's final content size. Sol limited independent review and the unchanged
  API smoke both PASS; evidence is in `target/diagnostic-wrap-width-20261102/`.
- Aurora tab and title drag protocol events were exercised against the real
  isolated Windows daemon/PTY; order, focus and all process IDs were verified.
  This is synthetic event evidence, separate from native captures and F1 input.

## Compact layout final gate (§104)

- Default Aurora keeps one bottom row, no tree/files/agents side columns, and no
  toolbar unless selected. At 120x30 pane area is 120x29 (v0.6.0: 120x27);
  at 80x24 it is 80x23 (v0.6.0: 80x21). One-line pane headers are unchanged.
- Native same-font comparisons at 120x30 and 200x50 are retained in
  `target/verification-20261102-layout-comparison/comparison.html`. These use
  two PowerShell panes with default UI settings; earlier real-agent/mono
  evidence above remains separate. Source and binary hashes accompany captures.
- PASS: format, Clippy, release build, all 12 native feature smokes and Windows
  installer lifecycle. Full serial Rust suite: 608 passed, four existing ignored.
  Initial parallel suite exited during a Windows host-loss test without assertion
  output; focused serial and full serial reruns pass.
- Raw local validation remains FAIL: the initial test exit and an unrelated
  `.sites-runtime` npm log changing during capture. No compiled source changed.
  `target/verification-20261102-compact-final/merge-gate.json` records resolution
  without rewriting the raw evidence or claiming release-package validation.
- Independent bounded layout review and finding recheck PASS: sidebar semantic
  glyph/color and identity accents, tiny-height tree budget, manual tab names.
  Physical mouse dragging and non-Windows/hosted-CI acceptance remain NOT_RUN.
