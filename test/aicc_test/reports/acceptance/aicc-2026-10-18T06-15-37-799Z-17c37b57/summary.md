# AICC E2E Acceptance Report

- Run: `aicc-2026-10-18T06-15-37-799Z-17c37b57`
- Commit: `1e8f93081e1a1b9561e77a78e3f7cb67f9922330`
- Capability baseline: `2026-10-18.1`
- Real model calls: 7/7
- Planned maximum cost: $0.070000
- Known actual / unknown estimated exposure: $0.000000 / $0.070000
- Total exposure / budget: $0.070000 / $3.000000
- Unknown-cost calls: 7; budget exceeded: false
- Results: passed=3, failed=0, provider_restricted=0, skipped=0, not_applicable=0, review=4

## Targeted retest

Run after fixing the reported defect; repeat `--case` to select additional cases:

```bash
pnpm run acceptance:gateway -- --config aicc_acceptance.local.toml --allow-credential-mutation --timeout-ms 900000 --no-real-model-calls
```

## Cases

| Case | Layer | Provider | Model | API | Status |
|---|---|---|---|---|---|
| t2.doubao-agent-plan.doubao-agent-plan-t2.glm-5.3.llm | T2 | doubao-agent-plan/doubao-agent-plan-t2 | glm-5.3@doubao-agent-plan-t2 | llm | passed |
| t2.qwen.qwen-t2.qwen3.6-plus.llm | T2 | qwen/qwen-t2 | qwen3.6-plus@qwen-t2 | llm | passed |
| t2.qwen.qwen-t2.qwen3.6-plus.vision.caption | T2 | qwen/qwen-t2 | qwen3.6-plus@qwen-t2 | vision.caption | review |
| t2.qwen.qwen-t2.qwen3.6-plus.vision.ocr | T2 | qwen/qwen-t2 | qwen3.6-plus@qwen-t2 | vision.ocr | review |
| t2.qwen.qwen-t2.qwen3.8-max-0902.llm | T2 | qwen/qwen-t2 | qwen3.8-max-0902@qwen-t2 | llm | passed |
| t2.qwen.qwen-t2.qwen3.8-max-0902.vision.caption | T2 | qwen/qwen-t2 | qwen3.8-max-0902@qwen-t2 | vision.caption | review |
| t2.qwen.qwen-t2.qwen3.8-max-0902.vision.ocr | T2 | qwen/qwen-t2 | qwen3.8-max-0902@qwen-t2 | vision.ocr | review |

## Physical model coverage

| Provider | Inventory model | Physical model | Decision | Reason | Retained representative |
|---|---|---|---|---|---|
| doubao-agent-plan/doubao-agent-plan-t2 | glm-5.3 | glm-5.3 | included | physical model | - |
| qwen/qwen-t2 | glm-5.3 | glm-5.3 | included | physical model | - |
| qwen/qwen-t2 | glm-5 | glm-5 | included | physical model | - |
| qwen/qwen-t2 | qwen3.6-plus | qwen3.6-plus | included | physical model | - |
| qwen/qwen-t2 | qwen3.8-max-0902 | qwen3.8-max-0902 | included | physical model | - |
| doubao-agent-plan/doubao-agent-plan-t2 | glm-5.3 | glm-5.3 | included | physical model | - |
| qwen/qwen-t2 | glm-5.3 | glm-5.3 | included | physical model | - |
| qwen/qwen-t2 | glm-5 | glm-5 | included | physical model | - |
| qwen/qwen-t2 | qwen3.6-plus | qwen3.6-plus | included | physical model | - |
| qwen/qwen-t2 | qwen3.8-max-0902 | qwen3.8-max-0902 | included | physical model | - |

## Confirmed product defects

None.

## Cleanup

Status: passed

- removed 0 uploaded fixture object(s)
- removed 0 generated output object(s)
- temporary Provider credentials restored after testing (doubao-agent-plan, qwen)
