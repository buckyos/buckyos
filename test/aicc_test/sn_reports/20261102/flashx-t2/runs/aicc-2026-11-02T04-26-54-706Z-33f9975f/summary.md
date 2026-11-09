# AICC E2E Acceptance Report

- Run: `aicc-2026-11-02T04-26-54-706Z-33f9975f`
- Commit: `7c9472fda19935f8070a63943361063b8ad2e376`
- Capability baseline: `2026-11-02.2`
- Real model calls: 3/3
- Planned maximum cost: $0.060000
- Known actual / unknown estimated exposure: $0.000000 / $0.060000
- Total exposure / budget: $0.060000 / $0.500000
- Unknown-cost calls: 3; budget exceeded: false
- Results: passed=1, failed=0, provider_restricted=0, skipped=0, not_applicable=0, review=2

## Targeted retest

Run after fixing the reported defect; repeat `--case` to select additional cases:

```bash
pnpm run acceptance:gateway -- --config aicc_acceptance.sn-flashx-global-open-20261102.local.toml --allow-credential-mutation --timeout-ms 900000 --no-real-model-calls
```

## Cases

| Case | Layer | Provider | Model | API | Status |
|---|---|---|---|---|---|
| t2.sn-ai-provider.sn-global-open-20261030-t2.glm-5.3-flashx.llm | T2 | sn-ai-provider/sn-global-open-20261030-t2 | glm-5.3-flashx@sn-global-open-20261030-t2 | llm | passed |
| t2.sn-ai-provider.sn-global-open-20261030-t2.glm-5.3-flashx.vision.caption | T2 | sn-ai-provider/sn-global-open-20261030-t2 | glm-5.3-flashx@sn-global-open-20261030-t2 | vision.caption | review |
| t2.sn-ai-provider.sn-global-open-20261030-t2.glm-5.3-flashx.vision.ocr | T2 | sn-ai-provider/sn-global-open-20261030-t2 | glm-5.3-flashx@sn-global-open-20261030-t2 | vision.ocr | review |

## Physical model coverage

| Provider | Inventory model | Physical model | Decision | Reason | Retained representative |
|---|---|---|---|---|---|
| sn-ai-provider/sn-global-open-20261030-t2 | glm-5.3-flashx | glm-5.3-flashx | included | physical model | - |
| sn-ai-provider/sn-global-open-20261030-t2 | glm-5 | glm-5 | included | physical model | - |
| sn-ai-provider/sn-global-open-20261030-t2 | glm-5.3-flashx | glm-5.3-flashx | included | physical model | - |
| sn-ai-provider/sn-global-open-20261030-t2 | glm-5 | glm-5 | included | physical model | - |

## Confirmed product defects

None.

## Cleanup

Status: passed

- removed 0 uploaded fixture object(s)
- removed 0 generated output object(s)
- temporary Provider credentials restored after testing (sn-ai-provider)
