# AICC E2E Acceptance Report

- Run: `aicc-2026-10-09T11-00-01-736Z-5a78ed8f`
- Commit: `0c32470f0ebaa0f14a1a262bc2bcb0eea553d8fc`
- Capability baseline: `2026-10-09.1`
- Real model calls: 0/2
- Planned maximum cost: $0.020000
- Known actual / unknown estimated exposure: $0.000000 / $0.000000
- Total exposure / budget: $0.000000 / $0.020000
- Unknown-cost calls: 0; budget exceeded: false
- Results: passed=0, failed=0, provider_restricted=0, skipped=2, not_applicable=0, review=0

## Targeted retest

Run after fixing the reported defect; repeat `--case` to select additional cases:

```bash
pnpm run acceptance:gateway -- --config aicc_acceptance.local.toml --allow-credential-mutation --timeout-ms 900000 --no-real-model-calls
```

## Cases

| Case | Layer | Provider | Model | API | Status |
|---|---|---|---|---|---|
| t2.qwen.qwen-t2.qwen3.7-plus.llm | T2 | qwen/qwen-t2 | qwen3.7-plus@qwen-t2 | llm | skipped |
| t2.qwen.qwen-t2.qwen3.8-flash.llm | T2 | qwen/qwen-t2 | qwen3.8-flash@qwen-t2 | llm | skipped |

## Physical model coverage

| Provider | Inventory model | Physical model | Decision | Reason | Retained representative |
|---|---|---|---|---|---|
| qwen/qwen-t2 | qwen3.7-plus | qwen3.7-plus | included | physical model | - |
| qwen/qwen-t2 | qwen3.8-flash | qwen3.8-flash | included | physical model | - |
| qwen/qwen-t2 | qwen3.7-plus | qwen3.7-plus | included | physical model | - |
| qwen/qwen-t2 | qwen3.8-flash | qwen3.8-flash | included | physical model | - |

## Confirmed product defects

None.

## Cleanup

Status: passed

- removed 0 uploaded fixture object(s)
- removed 0 generated output object(s)
- temporary Provider credentials restored after testing (qwen)
