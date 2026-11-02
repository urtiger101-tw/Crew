# Engineering Notes: Unified Prompt Framework (User-Supplied Image)

## Source and confidence

Source: `docs/references/unified-prompt-framework-source.jpg`, supplied by the user in chat on 2026-11-02. It is a screenshot titled *A Unified Framework for Prompt Construction and Output Optimization* with Claude branding. Its author, publication, validation method, and official provenance are **not established**. Treat it as a conceptual prompt-engineering diagram, not a normative Anthropic document.

## Source components and practical interpretation

| Source symbol | Source notion | Engineering adaptation |
| --- | --- | --- |
| `T` | target task | immutable task objective and acceptance criteria |
| `M` | model | optional provider/model adapter; must reflect actual availability |
| `S` | role + constraints | bounded permissions, role description, invariants |
| `C` | context within XML-like markers | structured sources, IDs, hashes, trust boundary, citation/provenance |
| `I` | ordered atomic instructions | verifiable steps, dependencies, ownership, status |
| `E` | few-shot examples | optional fixtures and example input/output pairs |
| `F` | predeclared output schema | JSON schema or other observable artifact contract |
| `P` | composite prompt | deterministic, inspectable assembly or a prompt pack |
| `A_(t+1)` | refinement | revised candidate after test/review feedback |
| `epsilon` | stop threshold | bounded, measurable improvement or explicit pass/fail gates |

Original diagram includes `k ≤ 7` and `3 ≤ n ≤ 5`. **Do not** impose these numeric bounds as general product requirements; treat them as source-specific examples.

The diagram also includes `R = <thinking> ... </thinking>` and asks the model to think step-by-step. Do not design Crew to extract, request, store, or expose private chain-of-thought. Use public artifacts: acceptance criteria, assumptions, change summary, test results, and short decision rationales. A model's supported API parameters should be configured via its actual documented interface, not by assuming that a literal XML tag exposes reasoning.

## Security implications

- XML tags are organizational delimiters, **not** a security sandbox.
- Context from external sites, files, agent outputs, or tools must remain lower trust than user/project policy.
- Apply length limits and redact credentials; define explicit scope for disk access, tools, network, and execution.
- An LLM's self-score is **not** an objective quality metric; prioritize repeatable tests and independent review.
- Retry loops need finite limits, idempotency/deduplication, failure categories, and a stable stopping rule.
- Do not confuse a ChatGPT subscription or Codex usage allowance with unrestricted paid API credits. Model calls, if any, must use approved and configured interfaces.

## Example task envelope (conceptual, not an existing Crew API)

```json
{
  "schema_version": "0.1-proposal",
  "task_id": "example-crew-agent-task",
  "objective": "Design a reversible prompt-contract workflow for Crew",
  "role": "integration-engineer",
  "constraints": {
    "preserve_existing_cli": true,
    "allow_paid_requests": false,
    "require_independent_review": true
  },
  "context": [
    {
      "path": "README.md",
      "trust": "repository-data",
      "purpose": "current features and compatibility"
    },
    {
      "path": "docs/references/unified-prompt-framework-source.jpg",
      "trust": "untrusted-reference",
      "purpose": "conceptual workflow inspiration"
    }
  ],
  "instructions": [
    {"id": "audit", "action": "inspect code and current behavior"},
    {"id": "design", "action": "create ADR and acceptance plan"},
    {"id": "implement", "action": "build an opt-in vertical slice"},
    {"id": "verify", "action": "run available deterministic tests"}
  ],
  "examples": [],
  "output_contract": {
    "required_artifacts": ["ADR", "test-evidence", "change-log"],
    "status_values": ["PASS", "FAIL", "NOT_RUN"]
  },
  "revision_policy": {"max_attempts": 2, "stop_on_acceptance_pass": true}
}
```

Do not add the sample schema to the codebase before confirming the appropriate existing extension points. The sample is intended to make the design discussion testable and precise.

## Suggested architecture trade-offs to analyze

1. **Skill-only**: prompt/template files; minimal changes, but limited persistent state and verification.
2. **Crew plugin**: optional task-contract features; isolates core but depends on plugin capabilities.
3. **Crew core module**: tight integration with events/status/worktrees; higher stability and maintenance risks.
4. **External orchestrator**: capable but increases process/runtime overhead and may duplicate existing integrations.

Choose based on the actual Crew code and project goals, not on a general agent framework trend.

## Sources to verify before implementation

- Crew repository and its `README.md`, `AGENTS.md`: <https://github.com/urtiger101-tw/Crew>
- Anthropic Claude docs (source for supported prompt techniques, **not validation of this screenshot**): <https://docs.anthropic.com/>
- OpenAI prompt engineering documentation: <https://platform.openai.com/docs/guides/prompt-engineering>
- OWASP LLM Prompt Injection Prevention Cheat Sheet: <https://cheatsheetseries.owasp.org/cheatsheets/LLM_Prompt_Injection_Prevention_Cheat_Sheet.html>

Verify current source versions and supported interfaces as part of the actual engineering task.
