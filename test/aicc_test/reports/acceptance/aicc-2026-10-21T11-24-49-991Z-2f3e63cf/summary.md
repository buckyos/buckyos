# AICC E2E Acceptance Report

- Run: `aicc-2026-10-21T11-24-49-991Z-2f3e63cf`
- Commit: `53b4431e2d46ec731598ed0343d4a6596bee8d15`
- Capability baseline: `2026-10-21.2`
- Real model calls: 2/3
- Planned maximum cost: $0.030000
- Known actual / unknown estimated exposure: $0.000000 / $0.020000
- Total exposure / budget: $0.020000 / $30.000000
- Unknown-cost calls: 2; budget exceeded: false
- Results: passed=0, failed=2, provider_restricted=0, skipped=0, not_applicable=0, review=0

## Targeted retest

Run after fixing the reported defect; repeat `--case` to select additional cases:

```bash
pnpm run acceptance:gateway -- --config aicc_acceptance.sn.local.toml --allow-credential-mutation --timeout-ms 900000 --provider sn-ai-provider --case t2.sn-ai-provider.sn-issues-45-46-t2.qwen-image-3-pro.image.img2img --case t2.sn-ai-provider.sn-issues-45-46-t2.qwen-image-3-pro.image.txt2img
```

## Cases

| Case | Layer | Provider | Model | API | Status |
|---|---|---|---|---|---|
| t2.sn-ai-provider.sn-issues-45-46-t2.qwen-image-3-pro.image.img2img | T2 | sn-ai-provider/sn-issues-45-46-t2 | qwen-image-3-pro@sn-issues-45-46-t2 | image.img2img | failed |
| t2.sn-ai-provider.sn-issues-45-46-t2.qwen-image-3-pro.image.txt2img | T2 | sn-ai-provider/sn-issues-45-46-t2 | qwen-image-3-pro@sn-issues-45-46-t2 | image.txt2img | failed |

## Physical model coverage

| Provider | Inventory model | Physical model | Decision | Reason | Retained representative |
|---|---|---|---|---|---|
| sn-ai-provider/sn-issues-45-46-t2 | qwen-image-3-pro | qwen-image-3-pro | included | physical model | - |
| sn-ai-provider/sn-issues-45-46-t2 | qwen-image-3-pro | qwen-image-3-pro | included | physical model | - |

## Confirmed product defects

None.

## Cleanup

Status: passed

- removed 0 uploaded fixture object(s)
- removed 0 generated output object(s)
- temporary Provider credentials restored after testing (sn-ai-provider)
