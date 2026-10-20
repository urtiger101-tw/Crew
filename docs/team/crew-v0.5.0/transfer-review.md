# Independent review: live cross-workspace tab transfer

Reviewer: `/root`; author: `/root/v05_transfer`. The root reviewer did not author
this feature. The later integration-only addition of default metadata fields in
`src/workspace/transfer_tests.rs` is not a production behavior change.

## Scope and decision

- Base: `5e7e208bca5bf388aded7631aff0d8846b47363f`.
- Author head: `3e498edbbc7ccb4b7f71f7c5daf76c4f3212e495`.
- Integrated transfer commit: `e03ac1872cffca2c67daf91912636ba57a5e3e7a`.
- Later integration compile fix: `14a4f64608134f56022ea6abbe10cb209b11e692`.
- Production review: workspace request parsing, dispatch, transfer, stable
  target resolution, shared chooser/confirmation restoration, focus and layout
  changes, metadata sweep, events, and existing persistence integration.

**Decision: no outstanding blocking finding in this transfer slice.** Static
review and the independent native smoke below support the decision. This does
not approve root's own plugin implementation or claim a complete platform matrix.

## Reviewed invariants

All destination, slot, source and manually named tab checks occur before any
mutation while State is locked. A transfer moves the existing Window object; it
does not recreate panes, change process identity, rewrite running process cwd/env,
or replace agent occupants. Same-workspace transfer delegates to the original
ordering implementation and retains its no-op behavior.

Destination selection follows its existing tab after insertion. Source selection
uses the next or previous neighbour. Removing an empty source adjusts the shifted
destination index before subsequent access, moves affected clients to the existing
destination, and never calls session termination. Chooser selections and pending
close confirmations retain stable object identity. Old drag geometry is cleared.

The transfer emits one tab movement event, then an empty-source workspace closure
when applicable. Closing that source drops only its workspace metadata; live pane
metadata, sequence high-water marks and agent wait identities survive. The
existing dirty-state/persistence path saves the new ownership and ordering.

## Independent execution

On Linux x86_64, root ran:

```sh
python3 -B scripts/workspace_transfer_smoke.py \
  /workspace/scratch/21396cda8665/target-v05-root/debug/crew
```

Exit code: **0**. Seven native fixture groups passed: two live shell PTYs and
process identity; atomic invalid-input rejection; layout/zoom/agent/focus
preservation; exact names and caller scope; empty-source removal with unchanged
PIDs, state, cwd/env and token sequence; pending agent wait completion; and cold
restart persistence. The test uses isolated temporary Crew configuration/state
and an actual daemon. It does not consume GitHub Actions or a model API.

The executable was built from the integrated `14a4f64` source and remained
unchanged at the invocation under `5fd9423`; the intervening commit only changes
local Python packaging and its tests.

- Binary SHA-256:
  `f7d343be5deb7fff29ebc5cf01a190b5a461fbcca83ef54ba06c65a20ef1644b`.
- Raw log: `/tmp/crew-v05-root-transfer-review-native.log`.
- Log SHA-256:
  `2ef9349e63436ee38928e00b9bd60c9481da16542a26135c58a229f392511529`.

The complete source-bound release validation will run again on the final clean
candidate and is recorded separately. Native Windows/macOS execution and Windows
installer lifecycle remain **NOT_RUN** in this review. Existing-shell environment
and cwd deliberately remain unchanged after transfer; this is not process
migration to a different host.
