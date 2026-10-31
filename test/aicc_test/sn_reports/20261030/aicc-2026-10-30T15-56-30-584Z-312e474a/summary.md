# AICC E2E Acceptance Report

- Run: `aicc-2026-10-30T15-56-30-584Z-312e474a`
- Commit: `d40291a9a9085d1cec7ccb37d519d9115baf534c`
- Capability baseline: `2026-10-30.1`
- Real model calls: 4/4
- Planned maximum cost: $0.080000
- Known actual / unknown estimated exposure: $0.000000 / $0.080000
- Total exposure / budget: $0.080000 / $1.000000
- Unknown-cost calls: 4; budget exceeded: false
- Results: passed=0, failed=1, provider_restricted=0, skipped=0, not_applicable=0, review=3

## Targeted retest

Run after fixing the reported defect; repeat `--case` to select additional cases:

```bash
pnpm run acceptance:gateway -- --config aicc_acceptance.sn-us-full-local-fix-20261030.local.toml --allow-credential-mutation --timeout-ms 900000 --no-real-model-calls
```

## Cases

| Case | Layer | Provider | Model | API | Status |
|---|---|---|---|---|---|
| t2.sn-ai-provider.sn-us-full-20261030-t2.gpt-5.4-pro.vision.ocr | T2 | sn-ai-provider/sn-us-full-20261030-t2 | gpt-5.4-pro@sn-us-full-20261030-t2 | vision.ocr | review |
| t2.sn-ai-provider.sn-us-full-20261030-t2.gpt-5.5-pro.vision.ocr | T2 | sn-ai-provider/sn-us-full-20261030-t2 | gpt-5.5-pro@sn-us-full-20261030-t2 | vision.ocr | review |
| t2.sn-ai-provider.sn-us-full-20261030-t2.gpt-5.5.image.img2img | T2 | sn-ai-provider/sn-us-full-20261030-t2 | gpt-5.5@sn-us-full-20261030-t2 | image.img2img | review |
| t2.cleanup.named_data | T2 | -/- | - | ndm.remove | failed |

## Physical model coverage

| Provider | Inventory model | Physical model | Decision | Reason | Retained representative |
|---|---|---|---|---|---|
| sn-ai-provider/sn-us-full-20261030-t2 | gpt-5.4-pro | gpt-5.4-pro | included | physical model | - |
| sn-ai-provider/sn-us-full-20261030-t2 | gpt-5.5-pro | gpt-5.5-pro | included | physical model | - |
| sn-ai-provider/sn-us-full-20261030-t2 | gpt-5.5 | gpt-5.5 | included | physical model | - |
| sn-ai-provider/sn-us-full-20261030-t2 | gpt-5.4-pro | gpt-5.4-pro | included | physical model | - |
| sn-ai-provider/sn-us-full-20261030-t2 | gpt-5.5-pro | gpt-5.5-pro | included | physical model | - |
| sn-ai-provider/sn-us-full-20261030-t2 | gpt-5.5 | gpt-5.5 | included | physical model | - |

## Confirmed product defects

None.

## Cleanup

Status: failed

- removed 0 uploaded fixture object(s)
- removed 0 generated output object(s)
- temporary Provider credentials restored after testing (sn-ai-provider)
