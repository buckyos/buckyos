# AICC E2E Acceptance Report

- Run: `aicc-2026-10-30T09-26-46-334Z-41139d0e`
- Commit: `d40291a9a9085d1cec7ccb37d519d9115baf534c`
- Capability baseline: `2026-10-30.1`
- Real model calls: 1/1
- Planned maximum cost: $0.020000
- Known actual / unknown estimated exposure: $0.000000 / $0.020000
- Total exposure / budget: $0.020000 / $20.000000
- Unknown-cost calls: 1; budget exceeded: false
- Results: passed=0, failed=0, provider_restricted=0, skipped=0, not_applicable=0, review=1

## Targeted retest

Run after fixing the reported defect; repeat `--case` to select additional cases:

```bash
pnpm run acceptance:gateway -- --config aicc_acceptance.sn-us-full-20261030.local.toml --allow-credential-mutation --timeout-ms 900000 --no-real-model-calls
```

## Cases

| Case | Layer | Provider | Model | API | Status |
|---|---|---|---|---|---|
| t2.sn-ai-provider.sn-us-full-20261030-t2.gpt-5.4-pro.vision.caption | T2 | sn-ai-provider/sn-us-full-20261030-t2 | gpt-5.4-pro@sn-us-full-20261030-t2 | vision.caption | review |

## Physical model coverage

| Provider | Inventory model | Physical model | Decision | Reason | Retained representative |
|---|---|---|---|---|---|
| sn-ai-provider/sn-us-full-20261030-t2 | gpt-5.4-pro | gpt-5.4-pro | included | physical model | - |
| sn-ai-provider/sn-us-full-20261030-t2 | gpt-5.4-pro | gpt-5.4-pro | included | physical model | - |

## Confirmed product defects

None.

## Cleanup

Status: passed

- removed 0 uploaded fixture object(s)
- removed 0 generated output object(s)
- temporary Provider credentials restored after testing (sn-ai-provider)
