# CORE-04 independent review

- Reviewer instance: `/root`; author: `/root/target_audit`.
- Author head: `8c76435ed93ece84297b66a71f231487da7c94c5`.
- Reviewed changes: workspace/tab request routing and CLI, `src/workspace/order.rs`, ordering tests, native smoke and contract documentation.
- Integration head: `d9b87f1451044e5222971eb192acadb42f2c9faa`, tree `31eede3a1d8fa5fc850516a91a86d294e123d830`.
- Result: PASS for static behavior review; no actionable correctness findings in the author's CORE-04 change.

Review traced original-list insertion slots through validation before mutation, active-tab identity retention, stable workspace selection, caller/explicit scope resolution, chooser row capture and restoration by stable IDs, pending close-confirmation target rewriting, event/no-op behavior, and existing persistence order. The 12 slot cases cover each source position and all boundary/interior slots for both collections. Invalid raw requests and nonexistent/cross-workspace targets are rejected before dirty/event changes.

The author used isolated PTYs and confirmed retained shell memory and agent wait identity. Root independently ran all Rust tests and is running the complete integrated Linux smoke set; exact commands and results are recorded separately in `results.json`. Windows/macOS validation remains NOT_RUN until native CI. Attached-terminal chooser/confirmation behavior has handler-level client fixtures; no manual terminal UI session was performed.

Root's shared `(last_used, id)` tie-breaking and metadata projection changes are outside this author's change and receive separate independent review from `/root/reference_audit`; this record is not root self-approval.
