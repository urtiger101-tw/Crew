# Independent plugin public-network check

Reviewer: `/root/v05_transfer`; plugin author: `/root`.
**Result: PASS for the tested owner-repository HTTPS/Git download path and real
GitHub search response.** This supplements the earlier offline plugin review.

The fixture uses the owner-reviewed manifest at immutable commit
`c6156b3d635fe9df016d77ba16cb4a5e5a07563f` in
`urtiger101-tw/Crew/plugins/crew.workspaces`. Raw HTTPS returned 200. Its 514 bytes
match the reviewed root manifest exactly:
`9777ac91514c21db69f4474ca879880b0026f08db25c1e82701fccc6a453c92a`.
It declares three workspace actions and no build commands.

The initial `git ls-remote` tool call observed the pinned commit on
`feat/crew-v0.5.0`. During the recorded run the owner had advanced that branch to
`05393483fa6d44136a1db4175981900d27afc8cd`; every manifest download, inspect,
installation and update remained explicitly pinned to `c6156b3`.

## Actual Linux execution

All nine recorded subprocess commands returned exit 0. Exact argv, cwd, timing,
stdout/stderr file hashes, and assertions are in the companion JSON.

| Check | Observed result |
| --- | --- |
| Public branch `git ls-remote` | Read succeeded; no remote writes |
| Fixed-SHA raw manifest | HTTP 200; exact reviewed SHA256 match |
| `plugin inspect SOURCE --ref SHA --json` | GitHub source/ref/actual commit all match; `code_executed=false`; no registration |
| `plugin install SOURCE --ref SHA --yes --disabled` | Only `crew.workspaces` registered, disabled; installed manifest bytes match; confirmation says `builds_will_run=false` |
| `plugin list --json` | Exact pinned GitHub source and disabled state preserved |
| `plugin update crew.workspaces --yes` | `status=unchanged`; registry bytes and installed root unchanged |
| List after update | Identical to the initial installed listing |
| `plugin marketplace search crew --online --json` | Success; `online_requested=true`; two bundled entries, zero GitHub discovery entries |
| Direct equivalent GitHub search GET | HTTP 200; valid `total_count`, `incomplete_results`, and `items` schema; zero results |

The public search successfully exercised the real endpoint and empty-response
schema. With zero discovery items, per-entry discovery transformation is
**NOT_OBSERVED**, and third-party plugin validation is **NOT_RUN**. No
third-party plugin source was downloaded or executed.

## Isolation and provenance

Execution used the supplied reviewed root binary:
`/workspace/scratch/21396cda8665/target-v05-root/debug/crew`, SHA256
`f7d343be5deb7fff29ebc5cf01a190b5a461fbcca83ef54ba06c65a20ef1644b`.
The hash remained unchanged. A `TemporaryDirectory` held `CREW_PLUGIN_HOME`, an
empty Git config, and isolated Crew paths; it was removed after the test. The
plugin stayed disabled, no action/build or daemon was invoked, and discovery
left the registry unchanged. There were no network failures in this run; all
stdout/stderr, including empty streams, remain available.

Evidence: `/tmp/crew-v05-plugin-network-review.json` and
`/tmp/crew-v05-plugin-network-review/`. Reproduction harness:
`/tmp/crew-v05-plugin-network-check.py`.

No GitHub write, Actions run, release publication, or account configuration
change was performed. Native Windows/macOS execution remains **NOT_RUN**. This
focused check does not approve the separate SSH slice or a full platform release.
