# Scoped Herdr follow-up: one styled sidebar pane label

Owner: root / engineering. Base: bf7a58d042d495a7ca18b46736a34017ac93fd94.
The user's later instruction permits independently testable low-risk Herdr work
while full installer acceptance remains BLOCKED. This does not permit retrying
the denied 0.3.0 executable or changing Windows security settings.

The pinned Herdr 0.9.5 configuration reference describes sidebar rows with styled
token entries. Crew currently provides one literal text template per pane row.
This batch adds a limited single-line styled-segment option, not row grids,
conditional rules, remote pane support, or Herdr wire/config compatibility.

Acceptance:

1. Optional sidebar_pane_segments config accepts at most 16 string templates or
   tables with template plus optional strict #RGB/#RRGGBB fg and boolean bold/dim.
   Reject unknown style fields and control/bidi patterns at config load. Existing
   config and sidebar_pane_template behavior remain valid when segments are absent.
2. Resolve segments with existing literal metadata substitution, drop missing/empty
   values and their separators, join with ' · ', and cap the complete output at
   512 characters. Empty output falls back to the existing sidebar template/name.
   Metadata values are never re-expanded or executed.
3. TUI and plain API sidebar use the same projection. API spans expose plain text
   and optional styles, never ANSI. Only pane label text is styled; icons, state
   words, detail messages, backgrounds, mouse targets and row layout are unchanged.
4. Omitted modifiers preserve contextual focus style; explicit false removes only
   that modifier for its segment. Separator style remains contextual. NO_COLOR,
   TERM=dumb and mono suppress custom foregrounds. Wide-character clipping and
   ellipsis stay within the existing name budget without leaking style.
5. Verify invalid config, expansion and bounds with meaningful unit tests. Exercise
   real Windows server/ConPTY, metadata changes, API projection and debug-frame SGR,
   including mono and narrow Unicode clipping. Verify NO_COLOR/TERM=dumb for two
   client contexts in native Rust Grid tests; debug-frame uses a default ClientEnv
   and cannot certify a real client's environment merely via subprocess variables.
   Run the 25 unaffected canonical
   native commands again on the final committed source; lifecycle remains omitted
   with an explicit BLOCKED record, not converted to full validation.
6. Obtain independent review by a non-author. Preserve all earlier evidence and
   candidate artifacts. Commit only locally; no push, PR comment or publication.

Write ownership: implementation agent owns config.rs, metadata/display.rs,
render/sidebar.rs and related Rust tests. Root owns the plan, native smoke and
user-facing docs, integration, execution evidence and handoff. Independent reviewer
owns only review evidence outside the checkout.

The first committed feature run at dca7280 retained 24 PASS, transfer FAIL and
lifecycle BLOCKED. The existing transfer probe waited only for a response prefix.
A controlled native ConPTY reproduction demonstrated that this prefix can return
before the marker/environment arrive. The exact partial response from that failed
run was not captured, so its precise sampling trigger is not claimed as proven.
Follow-up scope is limited to complete nonce-scoped short P/M/E response fields,
with a terminal delimiter, retaining the exact PID/marker/env comparisons. A new
destination pane intentionally has an empty marker. No product transfer behavior
or assertions are weakened. Preserve both failed runs and revalidate on a clean
local commit with independent review.
