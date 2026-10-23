# AICC E2E Acceptance Report

- Run: `aicc-2026-10-23T07-43-48-586Z-19566e78`
- Commit: `53b4431e2d46ec731598ed0343d4a6596bee8d15`
- Capability baseline: `2026-10-23.1`
- Real model calls: 6/6
- Planned maximum cost: $0.060000
- Known actual / unknown estimated exposure: $0.000000 / $0.060000
- Total exposure / budget: $0.060000 / $30.000000
- Unknown-cost calls: 6; budget exceeded: false
- Results: passed=0, failed=1, provider_restricted=0, skipped=0, not_applicable=0, review=4

## Targeted retest

Run after fixing the reported defect; repeat `--case` to select additional cases:

```bash
pnpm run acceptance:gateway -- --config aicc_acceptance.native.local.toml --allow-credential-mutation --timeout-ms 900000 --provider qwen --case t2.qwen.native-qwen-t2.qwen-image-edit.image.img2img
```

## Cases

| Case | Layer | Provider | Model | API | Status |
|---|---|---|---|---|---|
| t2.qwen.native-qwen-t2.qwen-image-3.0-pro.image.img2img | T2 | qwen/native-qwen-t2 | qwen-image-3.0-pro@native-qwen-t2 | image.img2img | review |
| t2.qwen.native-qwen-t2.qwen-image-3.0-pro.image.txt2img | T2 | qwen/native-qwen-t2 | qwen-image-3.0-pro@native-qwen-t2 | image.txt2img | review |
| t2.qwen.native-qwen-t2.qwen-image-edit.image.img2img | T2 | qwen/native-qwen-t2 | qwen-image-edit@native-qwen-t2 | image.img2img | failed |
| t2.qwen.native-qwen-t2.wan2.6-t2v.video.txt2video | T2 | qwen/native-qwen-t2 | wan2.6-t2v@native-qwen-t2 | video.txt2video | review |
| t2.qwen.native-qwen-t2.wanx2.1-t2i-turbo.image.txt2img | T2 | qwen/native-qwen-t2 | wanx2.1-t2i-turbo@native-qwen-t2 | image.txt2img | review |

## Physical model coverage

| Provider | Inventory model | Physical model | Decision | Reason | Retained representative |
|---|---|---|---|---|---|
| qwen/native-qwen-t2 | qwen-image-3.0-pro | qwen-image-3.0-pro | included | physical model | - |
| qwen/native-qwen-t2 | qwen-image-edit | qwen-image-edit | included | physical model | - |
| qwen/native-qwen-t2 | wan2.6-t2v | wan2.6-t2v | included | physical model | - |
| qwen/native-qwen-t2 | wanx2.1-t2i-turbo | wanx2.1-t2i-turbo | included | physical model | - |
| qwen/native-qwen-t2 | qwen-image-3.0-pro | qwen-image-3.0-pro | included | physical model | - |
| qwen/native-qwen-t2 | qwen-image-edit | qwen-image-edit | included | physical model | - |
| qwen/native-qwen-t2 | wan2.6-t2v | wan2.6-t2v | included | physical model | - |
| qwen/native-qwen-t2 | wanx2.1-t2i-turbo | wanx2.1-t2i-turbo | included | physical model | - |

## Confirmed product defects

None.

## Cleanup

Status: passed

- removed 0 uploaded fixture object(s)
- removed 0 generated output object(s)
- temporary Provider credentials restored after testing (qwen)
