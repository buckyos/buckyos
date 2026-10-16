# AICC E2E Acceptance Report

- Run: `aicc-2026-10-09T11-55-24-315Z-5d47408b`
- Commit: `0c32470f0ebaa0f14a1a262bc2bcb0eea553d8fc`
- Capability baseline: `2026-10-09.4`
- Real model calls: 10/11
- Planned maximum cost: $0.110000
- Known actual / unknown estimated exposure: $0.000000 / $0.100000
- Total exposure / budget: $0.100000 / $0.750000
- Unknown-cost calls: 10; budget exceeded: false
- Results: passed=3, failed=3, provider_restricted=0, skipped=0, not_applicable=0, review=0

## Targeted retest

Run after fixing the reported defect; repeat `--case` to select additional cases:

```bash
pnpm run acceptance:gateway -- --config aicc_acceptance.local.toml --allow-credential-mutation --timeout-ms 900000 --provider qwen --case t2.qwen.qwen-t2.qwen-image-edit.image.img2img --case t2.qwen.qwen-t2.wan2.1-t2i-turbo.image.txt2img --case t2.qwen.qwen-t2.wan2.6-t2v.video.txt2video
```

## Cases

| Case | Layer | Provider | Model | API | Status |
|---|---|---|---|---|---|
| t2.qwen.qwen-t2.qwen-image-edit.image.img2img | T2 | qwen/qwen-t2 | qwen-image-edit@qwen-t2 | image.img2img | failed |
| t2.qwen.qwen-t2.qwen3.8-max.llm | T2 | qwen/qwen-t2 | qwen3.8-max@qwen-t2 | llm | passed |
| t2.qwen.qwen-t2.qwen3.8-max.vision.caption | T2 | qwen/qwen-t2 | qwen3.8-max@qwen-t2 | vision.caption | passed |
| t2.qwen.qwen-t2.qwen3.8-max.vision.ocr | T2 | qwen/qwen-t2 | qwen3.8-max@qwen-t2 | vision.ocr | passed |
| t2.qwen.qwen-t2.wan2.1-t2i-turbo.image.txt2img | T2 | qwen/qwen-t2 | wan2.1-t2i-turbo@qwen-t2 | image.txt2img | failed |
| t2.qwen.qwen-t2.wan2.6-t2v.video.txt2video | T2 | qwen/qwen-t2 | wan2.6-t2v@qwen-t2 | video.txt2video | failed |

## Physical model coverage

| Provider | Inventory model | Physical model | Decision | Reason | Retained representative |
|---|---|---|---|---|---|
| qwen/qwen-t2 | qwen-image-edit | qwen-image-edit | included | physical model | - |
| qwen/qwen-t2 | qwen3.8-max | qwen3.8-max | included | physical model | - |
| qwen/qwen-t2 | wan2.1-t2i-turbo | wan2.1-t2i-turbo | included | physical model | - |
| qwen/qwen-t2 | wan2.6-t2v | wan2.6-t2v | included | physical model | - |
| qwen/qwen-t2 | qwen-image-edit | qwen-image-edit | included | physical model | - |
| qwen/qwen-t2 | qwen3.8-max | qwen3.8-max | included | physical model | - |
| qwen/qwen-t2 | wan2.1-t2i-turbo | wan2.1-t2i-turbo | included | physical model | - |
| qwen/qwen-t2 | wan2.6-t2v | wan2.6-t2v | included | physical model | - |

## Confirmed product defects

None.

## Cleanup

Status: passed

- removed 0 uploaded fixture object(s)
- removed 0 generated output object(s)
- temporary Provider credentials restored after testing (qwen)
