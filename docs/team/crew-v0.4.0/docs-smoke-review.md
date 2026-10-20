# DOC-SMOKE-REVIEW-04

**PASS — independent static review of root-authored documentation and metadata smoke.**

- Initial documentation: `bf7da3317e102b44d3d9d3a38e7406f35c570ba0`.
- Metadata implementation: `36e8aa5766abb99053873d76f7f9e6656fd745f1`.
- Incremental fixes reviewed: `421e9dce422be99de77e64a3c7e5bdcd6db6ccf0`.
- CORE reference: `8c76435ed93ece84297b66a71f231487da7c94c5`; consistency reference only, not independent approval of my own code/document.

## Findings closed

1. DOC04-1 / P2: README had claimed Linux API cwd was live. The fix now distinguishes recorded startup cwd in ordinary pane JSON from Linux live-directory persistence and worktree checks. Source and revised Chinese/English text agree.
2. DOC04-2 / P2: README had told users all configuration values were strings. The fix limits that rule to `[theme.colors]` and retains typed TOML numbers/booleans for other settings.

Both were retained older wording, not introduced by the 0.4 feature patch. There are no remaining P1/P2 findings in this reviewed slice.

## Metadata smoke portability

All subprocesses receive argument arrays, with no shell=True. Text responses use explicit UTF-8. The temporary config selects PowerShell -NoProfile on Windows and the script checks its prompt before proceeding; no text command is pasted into the shell. CLI replies use the result envelope, JSON-lines replies use id/result, and api schema is correctly read as a raw object. Python AST parsing passed.

The update replaces a loose substring snapshot check with exact workspace and pane token assertions, and adds a real workspace move with metadata ownership assertions by stable ID. The requested insertion slot and response projection match CORE. Adding the second workspace introduces no extra first-shell-input race.

## Boundaries

No source, product documentation or smoke files were modified by this reviewer. No remote writes were performed. Native metadata smoke execution is NOT_RUN by this reviewer; root runs the integrated Linux suite and native CI. The installer statements in README/CHANGELOG match the assigned design, but final installer guide and native implementation evidence remain pending from its author.

Exact file hashes, source locations, check scopes and finding history are in review.json and findings.json.
