# PR → review → CI workflow

`crew workflow` observes one GitHub pull request and records its progress locally. Each `advance` refreshes GitHub evidence; invoke it again to resume as reviews and checks arrive. It does not push a branch, merge, or mark a draft ready for review. `ready` means the configured evidence has passed, not that a merge occurred.

Create a UTF-8 JSON config file, for example `workflow.json`:

```json
{
  "repo": "owner/project",
  "repo_path": ".",
  "state_path": ".crew/workflows/feature-state.json",
  "head": "feature/my-change",
  "base": "main",
  "title": "Describe the change",
  "body": "Why this change is needed",
  "draft": true,
  "reviewers": ["reviewer-login"],
  "required_checks": ["test", "lint"],
  "ci": {
    "workflow": "ci.yml",
    "ref": "feature/my-change",
    "inputs": { "allow_hosted_minutes": "true" }
  }
}
```

`repo_path` and `state_path` are relative to the configuration file directory; absolute paths also work. `repo` is `owner/name`; `head` and `base` are branch names. The branch must already be pushed before PR creation. `draft` defaults to `true`; `body`, `reviewers`, and `ci` are optional. `required_checks` must contain at least one non-empty check name. For an existing PR set `pr_number`; otherwise the engine first searches for a unique open PR with the configured head and base. `gh_command` optionally overrides the executable and its initial arguments as a JSON array, defaulting to `["gh"]`. The account used by `gh` needs permission to read the PR, reviews, checks and statuses; mutations require corresponding write permissions.

```text
crew workflow plan workflow.json
crew workflow advance workflow.json
crew workflow advance workflow.json --execute
crew workflow advance workflow.json --execute --allow-ci
crew workflow run workflow.json --execute --allow-ci --poll-ms 30000 --timeout-ms 3600000
```

`plan` reads local configuration and state only. `advance` reads GitHub and saves state once. `run` holds the state lock and polls in the foreground until ready or its timeout, defaulting to 30 seconds between polls and a one-hour timeout; both intervals must be positive milliseconds. It exits with an error on timeout and retains the last state. `--execute` authorizes creating a PR and requesting configured reviewers. A request is never treated as approval: at least one current-head approval from someone other than the PR author is required. The most recent substantive review per reviewer determines approval, and an approval for an older commit does not count. Any outstanding latest changes-requested review by a non-author blocks readiness. Configure the actual required check names explicitly: absent, pending, failed, cancelled, skipped, neutral, and other non-success results do not satisfy them. The head is refreshed before readiness is reported; a new head requires fresh evidence. These configured review and check gates do not determine GitHub branch protection or mergeability.

CI dispatch is optional and occurs only after approval when configured checks are still incomplete. It requires **both** `--execute` and `--allow-ci`, a configured workflow and inputs, and a `ref` equal to the PR head branch. Workflow-dispatch inputs must meet the target repository's own requirements; for Crew's hosted workflow, explicitly set `allow_hosted_minutes` to `true` only when those minutes are authorized. The workflow does not alter existing GitHub Actions settings. It dispatches at most once per observed head and then waits for check evidence; it does not retry a failed run automatically. Because GitHub dispatch uses a branch ref, a branch update concurrent with dispatch can cause a different commit to run; readiness still requires checks on the currently observed PR head.

The state file binds to the exact config bytes. After changing the config, use a new state path or deliberately reconcile the existing state and its config hash. Before any external mutation, the engine saves `recovery_needed`; it clears that marker only after the action returns successfully and the result is recorded. If a command fails or the process stops in that interval, inspect the PR, review requests, or Actions runs on GitHub. Reconcile `pr_number`, `requested_for_head`, or `dispatched_for_head` with what actually happened, then clear `recovery_needed` in the state file before resuming. Do not clear it without checking: a timed-out request may have succeeded remotely.

Only one `advance` may operate on a state file at a time. It creates an adjacent `state.json.lock` containing its process ID and removes it on normal exit. After an abnormal termination, first verify that the recorded process is no longer running and inspect the state and GitHub for an interrupted mutation; then remove the stale lock and follow the recovery procedure above.
