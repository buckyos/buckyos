# AICC E2E Acceptance Report

- Run: `aicc-2026-10-09T12-23-31-487Z-aa38be64`
- Commit: `0c32470f0ebaa0f14a1a262bc2bcb0eea553d8fc`
- Capability baseline: `2026-10-09.5`
- Real model calls: 6/6
- Planned maximum cost: $0.060000
- Known actual / unknown estimated exposure: $0.000000 / $0.060000
- Total exposure / budget: $0.060000 / $0.060000
- Unknown-cost calls: 6; budget exceeded: false
- Results: passed=2, failed=1, provider_restricted=0, skipped=0, not_applicable=0, review=0

## Targeted retest

Run after fixing the reported defect; repeat `--case` to select additional cases:

```bash
pnpm run acceptance:gateway -- --config aicc_acceptance.local.toml --allow-credential-mutation --timeout-ms 900000 --provider qwen --case t2.qwen.qwen-t2.wan2.6-t2v.video.txt2video
```

## Cases

| Case | Layer | Provider | Model | API | Status |
|---|---|---|---|---|---|
| t2.qwen.qwen-t2.qwen-image-edit.image.img2img | T2 | qwen/qwen-t2 | qwen-image-edit@qwen-t2 | image.img2img | passed |
| t2.qwen.qwen-t2.wan2.6-t2v.video.txt2video | T2 | qwen/qwen-t2 | wan2.6-t2v@qwen-t2 | video.txt2video | failed |
| t2.qwen.qwen-t2.wanx2.1-t2i-turbo.image.txt2img | T2 | qwen/qwen-t2 | wanx2.1-t2i-turbo@qwen-t2 | image.txt2img | passed |

## Physical model coverage

| Provider | Inventory model | Physical model | Decision | Reason | Retained representative |
|---|---|---|---|---|---|
| qwen/qwen-t2 | qwen-image-edit | qwen-image-edit | included | physical model | - |
| qwen/qwen-t2 | wan2.6-t2v | wan2.6-t2v | included | physical model | - |
| qwen/qwen-t2 | wanx2.1-t2i-turbo | wanx2.1-t2i-turbo | included | physical model | - |
| qwen/qwen-t2 | qwen-image-edit | qwen-image-edit | included | physical model | - |
| qwen/qwen-t2 | wan2.6-t2v | wan2.6-t2v | included | physical model | - |
| qwen/qwen-t2 | wanx2.1-t2i-turbo | wanx2.1-t2i-turbo | included | physical model | - |

## Confirmed product defects

None.

## Cleanup

Status: passed

- removed 0 uploaded fixture object(s)
- removed 0 generated output object(s)
- temporary Provider credentials restored after testing (qwen)
