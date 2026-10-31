# AICC E2E Acceptance Report

- Run: `aicc-2026-10-30T09-21-18-381Z-cb64369b`
- Commit: `d40291a9a9085d1cec7ccb37d519d9115baf534c`
- Capability baseline: `2026-10-30.1`
- Real model calls: 6/9
- Planned maximum cost: $0.180000
- Known actual / unknown estimated exposure: $0.000000 / $0.120000
- Total exposure / budget: $0.120000 / $10.000000
- Unknown-cost calls: 6; budget exceeded: false
- Results: passed=0, failed=5, provider_restricted=0, skipped=0, not_applicable=0, review=2

## Targeted retest

Run after fixing the reported defect; repeat `--case` to select additional cases:

```bash
pnpm run acceptance:gateway -- --config aicc_acceptance.sn-global-open-20261030.local.toml --allow-credential-mutation --timeout-ms 900000 --provider sn-ai-provider --case t2.sn-ai-provider.sn-global-open-20261030-t2.qwen-image-3-pro.image.img2img --case t2.sn-ai-provider.sn-global-open-20261030-t2.qwen-image-3-pro.image.txt2img --case t2.sn-ai-provider.sn-global-open-20261030-t2.wan-3.0.video.edit --case t2.sn-ai-provider.sn-global-open-20261030-t2.wan-3.0.video.extend --case t2.sn-ai-provider.sn-global-open-20261030-t2.wan-3.0.video.generate
```

## Cases

| Case | Layer | Provider | Model | API | Status |
|---|---|---|---|---|---|
| t2.sn-ai-provider.sn-global-open-20261030-t2.kimi-k3.1-pro.vision.ocr | T2 | sn-ai-provider/sn-global-open-20261030-t2 | kimi-k3.1-pro@sn-global-open-20261030-t2 | vision.ocr | review |
| t2.sn-ai-provider.sn-global-open-20261030-t2.qwen-image-3-pro.image.img2img | T2 | sn-ai-provider/sn-global-open-20261030-t2 | qwen-image-3-pro@sn-global-open-20261030-t2 | image.img2img | failed |
| t2.sn-ai-provider.sn-global-open-20261030-t2.qwen-image-3-pro.image.txt2img | T2 | sn-ai-provider/sn-global-open-20261030-t2 | qwen-image-3-pro@sn-global-open-20261030-t2 | image.txt2img | failed |
| t2.sn-ai-provider.sn-global-open-20261030-t2.qwen3.7-plus.vision.ocr | T2 | sn-ai-provider/sn-global-open-20261030-t2 | qwen3.7-plus@sn-global-open-20261030-t2 | vision.ocr | review |
| t2.sn-ai-provider.sn-global-open-20261030-t2.wan-3.0.video.edit | T2 | sn-ai-provider/sn-global-open-20261030-t2 | wan-3.0@sn-global-open-20261030-t2 | video.edit | failed |
| t2.sn-ai-provider.sn-global-open-20261030-t2.wan-3.0.video.extend | T2 | sn-ai-provider/sn-global-open-20261030-t2 | wan-3.0@sn-global-open-20261030-t2 | video.extend | failed |
| t2.sn-ai-provider.sn-global-open-20261030-t2.wan-3.0.video.generate | T2 | sn-ai-provider/sn-global-open-20261030-t2 | wan-3.0@sn-global-open-20261030-t2 | video.generate | failed |

## Physical model coverage

| Provider | Inventory model | Physical model | Decision | Reason | Retained representative |
|---|---|---|---|---|---|
| sn-ai-provider/sn-global-open-20261030-t2 | kimi-k3.1-pro | kimi-k3.1-pro | included | physical model | - |
| sn-ai-provider/sn-global-open-20261030-t2 | kimi-k3 | kimi-k3 | included | physical model | - |
| sn-ai-provider/sn-global-open-20261030-t2 | qwen-image-3-pro | qwen-image-3-pro | included | physical model | - |
| sn-ai-provider/sn-global-open-20261030-t2 | qwen3.7-plus | qwen3.7-plus | included | physical model | - |
| sn-ai-provider/sn-global-open-20261030-t2 | wan-3.0 | wan-3.0 | included | physical model | - |
| sn-ai-provider/sn-global-open-20261030-t2 | kimi-k3.1-pro | kimi-k3.1-pro | included | physical model | - |
| sn-ai-provider/sn-global-open-20261030-t2 | kimi-k3 | kimi-k3 | included | physical model | - |
| sn-ai-provider/sn-global-open-20261030-t2 | qwen-image-3-pro | qwen-image-3-pro | included | physical model | - |
| sn-ai-provider/sn-global-open-20261030-t2 | qwen3.7-plus | qwen3.7-plus | included | physical model | - |
| sn-ai-provider/sn-global-open-20261030-t2 | wan-3.0 | wan-3.0 | included | physical model | - |

## Confirmed product defects

None.

## Cleanup

Status: passed

- removed 0 uploaded fixture object(s)
- removed 0 generated output object(s)
- temporary Provider credentials restored after testing (sn-ai-provider)
