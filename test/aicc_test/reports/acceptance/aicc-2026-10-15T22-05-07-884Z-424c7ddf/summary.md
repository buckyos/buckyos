# AICC E2E Acceptance Report

- Run: `aicc-2026-10-15T22-05-07-884Z-424c7ddf`
- Commit: `e897c75d6ec5aaa78f72325a27fca221bbd9409e`
- Capability baseline: `2026-10-15.1`
- Real model calls: 1/1
- Planned maximum cost: $0.010000
- Known actual / unknown estimated exposure: $0.000000 / $0.010000
- Total exposure / budget: $0.010000 / $0.050000
- Unknown-cost calls: 1; budget exceeded: false
- Results: passed=1, failed=0, provider_restricted=0, skipped=0, not_applicable=0, review=0

## Targeted retest

Run after fixing the reported defect; repeat `--case` to select additional cases:

```bash
pnpm run acceptance:gateway -- --config aicc_acceptance.local.toml --allow-credential-mutation --timeout-ms 900000 --no-real-model-calls
```

## Cases

| Case | Layer | Provider | Model | API | Status |
|---|---|---|---|---|---|
| t2.openrouter.openrouter-t2.cohere-rerank-v3.5.rerank | T2 | openrouter/openrouter-t2 | cohere/rerank-v3.5@openrouter-t2 | rerank | passed |

## Physical model coverage

| Provider | Inventory model | Physical model | Decision | Reason | Retained representative |
|---|---|---|---|---|---|
| openrouter/openrouter-t2 | cohere/rerank-v3.5 | cohere/rerank-v3.5 | included | physical model | - |
| openrouter/openrouter-t2 | cohere/rerank-v3.5 | cohere/rerank-v3.5 | included | physical model | - |

## Confirmed product defects

None.

## Cleanup

Status: passed

- removed 0 uploaded fixture object(s)
- removed 0 generated output object(s)
- temporary Provider credentials restored after testing (openrouter)
