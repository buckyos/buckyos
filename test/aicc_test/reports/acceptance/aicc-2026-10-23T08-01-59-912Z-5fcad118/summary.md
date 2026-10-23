# AICC E2E Acceptance Report

- Run: `aicc-2026-10-23T08-01-59-912Z-5fcad118`
- Commit: `53b4431e2d46ec731598ed0343d4a6596bee8d15`
- Capability baseline: `2026-10-23.1`
- Real model calls: 1/2
- Planned maximum cost: $0.020000
- Known actual / unknown estimated exposure: $0.000000 / $0.010000
- Total exposure / budget: $0.010000 / $30.000000
- Unknown-cost calls: 1; budget exceeded: false
- Results: passed=0, failed=1, provider_restricted=0, skipped=0, not_applicable=0, review=0

## Targeted retest

Run after fixing the reported defect; repeat `--case` to select additional cases:

```bash
pnpm run acceptance:gateway -- --config aicc_acceptance.sn-openai.local.toml --allow-credential-mutation --timeout-ms 900000 --provider sn-ai-provider --case t2.sn-ai-provider.sn-ai-provider-main.gpt-image-2.image.img2img
```

## Cases

| Case | Layer | Provider | Model | API | Status |
|---|---|---|---|---|---|
| t2.sn-ai-provider.sn-ai-provider-main.gpt-image-2.image.img2img | T2 | sn-ai-provider/sn-ai-provider-main | gpt-image-2@sn-ai-provider-main | image.img2img | failed |

## Physical model coverage

| Provider | Inventory model | Physical model | Decision | Reason | Retained representative |
|---|---|---|---|---|---|
| sn-ai-provider/sn-ai-provider-main | gpt-image-2 | gpt-image-2 | included | physical model | - |
| sn-ai-provider/sn-ai-provider-main | gpt-image-2 | gpt-image-2 | included | physical model | - |

## Confirmed product defects

None.

## Cleanup

Status: passed

- removed 0 uploaded fixture object(s)
- removed 0 generated output object(s)
