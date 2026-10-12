# AICC E2E Acceptance Report

- Run: `aicc-2026-10-09T13-48-31-282Z-b86b239b`
- Commit: `0c32470f0ebaa0f14a1a262bc2bcb0eea553d8fc`
- Capability baseline: `2026-10-09.6`
- Real model calls: 3/3
- Planned maximum cost: $0.030000
- Known actual / unknown estimated exposure: $0.000000 / $0.030000
- Total exposure / budget: $0.030000 / $0.250000
- Unknown-cost calls: 3; budget exceeded: false
- Results: passed=3, failed=0, provider_restricted=0, skipped=0, not_applicable=0, review=0

## Targeted retest

Run after fixing the reported defect; repeat `--case` to select additional cases:

```bash
pnpm run acceptance:gateway -- --config aicc_acceptance.local.toml --allow-credential-mutation --timeout-ms 900000 --no-real-model-calls
```

## Cases

| Case | Layer | Provider | Model | API | Status |
|---|---|---|---|---|---|
| t2.qwen.qwen-t2.glm-5.1.llm | T2 | qwen/qwen-t2 | glm-5.1@qwen-t2 | llm | passed |
| t2.qwen.qwen-t2.glm-5.3.llm | T2 | qwen/qwen-t2 | glm-5.3@qwen-t2 | llm | passed |
| t2.qwen.qwen-t2.minimax-m2.5.llm | T2 | qwen/qwen-t2 | MiniMax-M2.5@qwen-t2 | llm | passed |

## Physical model coverage

| Provider | Inventory model | Physical model | Decision | Reason | Retained representative |
|---|---|---|---|---|---|
| qwen/qwen-t2 | glm-5.1 | glm-5.1 | included | physical model | - |
| qwen/qwen-t2 | glm-5.3 | glm-5.3 | included | physical model | - |
| qwen/qwen-t2 | glm-5 | glm-5 | included | physical model | - |
| qwen/qwen-t2 | MiniMax-M2.5 | MiniMax-M2.5 | included | physical model | - |
| qwen/qwen-t2 | glm-5.1 | glm-5.1 | included | physical model | - |
| qwen/qwen-t2 | glm-5.3 | glm-5.3 | included | physical model | - |
| qwen/qwen-t2 | glm-5 | glm-5 | included | physical model | - |
| qwen/qwen-t2 | MiniMax-M2.5 | MiniMax-M2.5 | included | physical model | - |

## Confirmed product defects

None.

## Cleanup

Status: passed

- removed 0 uploaded fixture object(s)
- removed 0 generated output object(s)
- temporary Provider credentials restored after testing (qwen)
