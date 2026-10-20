# Independent review: presentation templates and agent detection

Reviewer: `/root/v05_transfer`. This reviewer did not author the presentation,
detection, metadata guard fix, or root CLI transport fix. The review does not
approve the reviewer's own workspace-transfer or local-packaging changes.

## Immutable scope and decision

- Initial base: `5e7e208bca5bf388aded7631aff0d8846b47363f`.
- Initial presentation head: `f722c48fc0d5dd7b558a79e648011767f2dc5e98`.
- Initial read-only source export: `/tmp/crew-review-presentation-f722c48`.
- Author's presentation fix: `2424819fbe905a69c077d8ecce2f1a6caa6ab586`.
- Root CLI follow-up: `14a4f64608134f56022ea6abbe10cb209b11e692`.
- The root follow-up review is limited to `src/commands.rs`, `src/lib.rs`,
  `src/commands/tests.rs`, and `scripts/presentation_smoke.py`. Other plugin
  changes in that commit are assigned to a different independent reviewer.

**Initial decision: request changes. Final decision: no outstanding blocking
findings in the reviewed presentation/detection and CLI slices after the two
fixes.** Three concrete findings were resolved as described below. This is an
independent code and focused Linux behavior review, not a full native matrix or
release authorization. The integrated candidate still requires the root's
source-bound local validation.

## Resolved findings

### P2: clearing the last display field silently removed presentation guards

At the initial head, `src/metadata/presentation.rs:227-228` removed an entry when
its display value became empty, regardless of its agent-kind or authority-source
guard. The same rule discarded an initial guard-only report. A subsequent title
patch that omitted the guard consequently became visible to the wrong agent or
report source. This contradicted the documented omission-preserves-guards
contract. Explicit agent lifecycle identity itself was not overwritten.

Independent native reproduction used an isolated local daemon and three cases:
kind guard followed by clear-title and a title patch; authority-source guard
followed by the same sequence; and an initial guard-only report followed by a
title patch. All three displayed the unintended `LEAK-*` title at the initial
head. In `2424819`, an entry is removed only when both its display value and both
guards are absent. The independent rerun returns a null guarded title for all
three cases and confirms unchanged explicit agent identity.

The fix's new tests were also reviewed for omission of both guards, explicit
clear, guard-only source capacity, atomic rejection, sequence high-water marks,
and TTL expiry before and after a sweep. Clearing a source or its TTL expiry
still removes the guards; token-only reports do not revive or refresh expired
presentation state.

### P3: template validation accepted leading or trailing control whitespace

The initial validator compared cleaned text with `value.trim()`. This accepted
some boundary tab/newline values despite the documented rejection of control
characters. Rendering suppressed control glyphs, so this finding concerns the
validation contract and predictable configuration behavior. `2424819` directly
checks every character for control or bidi-format characters. Source review and
four independent native invalid-template cases confirm rejection.

The first follow-up invocation using raw no-space CLI arguments failed because
the pre-existing argv serializer removed the boundary whitespace before it
reached the validator. That failed run is retained in the evidence. A separate
run containing an ordinary ASCII space forced that serializer to preserve the
raw value and independently verified the validator fix, without changing any
production code.

### P2: legacy CLI serialization changed literal template arguments

The pre-existing fallback in `src/lib.rs` quoted only arguments containing an
ASCII space and did not escape quotes or backslashes. Consequently, controls
could disappear before validation and quoted or backslash-containing template
text could change during tokenization. Root fixed this in `14a4f64` by routing
each argument through `commands::quote`, with backslash included in the quoting
predicate and escaping performed in the required order.

The reviewer read the complete tokenize/quote path and the four scoped changed
files, then independently reran the original no-ASCII-space controls against the
new root binary. All four values returned exit 1. Three additional native
daemon/API cases preserved the exact literal template output: double quotes with
a backslash and hash, a path containing a backslash without whitespace, and
single quotes around Chinese text. All three guard cases still passed, and
explicit agent identity remained unchanged.

## Reviewed invariants

- Template expansion is literal and non-recursive, output is bounded, built-in
  fields retain their defined precedence, and empty results use a fallback.
- Metadata validation and source-capacity checks precede mutations; sequence
  high-water marks remain shared across presentation and token reports.
- Guard matching controls presentation only. The display-agent override and the
  detected-agent hint do not replace explicit agent instances, state, resume
  targets, wait identities, or explicit reporting authority.
- Process enumeration and executable inspection occur outside the main State
  mutex. Results are reconciled against the sampled pane/process identity before
  updating the hint and marking display state dirty.
- Linux detection considers the foreground process group and executable identity
  with bounded command-line reads; it does not accept arbitrary argument text as
  proof of an agent. macOS and Windows implementation paths were read, including
  resource ownership and traversal bounds, but not executed natively.
- Transient presentation, metadata, and detected hints are absent from persisted
  snapshots. API request additions preserve serde defaults and boxing does not
  alter the JSON representation.
- Linux, macOS, and Windows process-detection code, config, pane/workspace API
  surfaces, renderer, bridge integration, existing identity logic, docs, and new
  tests were read within the initial 24-file presentation diff and dependencies.

## Execution evidence and limits

The independent guard and CLI script is
`/tmp/crew-v05-presentation-review-repro.py`. It creates a random isolated daemon
socket and temporary config/state, uses a real `/bin/sh` PTY, and cleans up its
daemon. No GitHub writes, Actions, external agent services, or paid API calls were
used. Root/author supplied build binaries; this reviewer independently hashed and
executed them, but did not rebuild them. The integrated source-bound build and
full smoke plan belong to the root validator.

| Check | Result | Evidence |
| --- | --- | --- |
| Initial three guard-loss cases | Defect confirmed, exit 0 | `/tmp/crew-v05-presentation-review-repro-before.log` |
| First raw CLI follow-up at guard-only fix | Failed as expected from the separately tracked serializer defect, exit 1 | `/tmp/crew-v05-presentation-review-repro-after.log` |
| Guard fix and preserved-raw-value validator cases | PASS, exit 0 | `/tmp/crew-v05-presentation-review-repro-after-quoted.log` |
| Root CLI fix, original raw controls | PASS, exit 0 | `/tmp/crew-v05-presentation-review-cli14a4f64.log` |
| Root CLI fix, raw controls and literal text cases | PASS, exit 0 | `/tmp/crew-v05-presentation-review-cli14a4f64-literals.log` |
| Full integrated Linux validator by this reviewer | NOT_RUN | Root owns the integrated candidate |
| Native Windows and macOS execution | NOT_RUN | No native platform was available to this reviewer |

Initial binary SHA256:
`1713304b919ba0e322b52ecd1dc91903aa8fa085aa8ef164796a1efb82a5813b`.
Guard-fix binary SHA256:
`f6690b6c95377ed7368f28d49efdc48f74d604a32bc984ff99c41f9cc719b4a5`.
Root CLI-fix binary SHA256:
`f7d343be5deb7fff29ebc5cf01a190b5a461fbcca83ef54ba06c65a20ef1644b`.

The companion JSON contains machine-readable scope, findings, result status,
binary hashes, and log hashes. This review makes no claim of native Windows
installer execution, full cross-platform parity, or release readiness.
