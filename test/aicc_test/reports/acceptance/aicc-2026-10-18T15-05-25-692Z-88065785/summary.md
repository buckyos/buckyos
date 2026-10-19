# AICC E2E Acceptance Report

- Run: `aicc-2026-10-18T15-05-25-692Z-88065785`
- Commit: `1e8f93081e1a1b9561e77a78e3f7cb67f9922330`
- Capability baseline: `2026-10-18.2`
- Real model calls: 0/2
- Planned maximum cost: $0.020000
- Known actual / unknown estimated exposure: $0.000000 / $0.000000
- Total exposure / budget: $0.000000 / $0.100000
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
| t2.qwen.qwen-t2.qwen-image-3.0-pro.image.img2img | T2 | qwen/qwen-t2 | qwen-image-3.0-pro@qwen-t2 | image.img2img | skipped |
| t2.qwen.qwen-t2.qwen-image-3.0-pro.image.txt2img | T2 | qwen/qwen-t2 | qwen-image-3.0-pro@qwen-t2 | image.txt2img | skipped |

## Physical model coverage

| Provider | Inventory model | Physical model | Decision | Reason | Retained representative |
|---|---|---|---|---|---|
| qwen/qwen-t2 | qwen-image-3.0-pro | qwen-image-3.0-pro | included | physical model | - |
| qwen/qwen-t2 | qwen-image-3.0-pro | qwen-image-3.0-pro | included | physical model | - |

## Confirmed product defects

None.

## Cleanup

Status: passed

- removed 0 uploaded fixture object(s)
- removed 0 generated output object(s)
- temporary Provider credentials restored after testing (qwen)
