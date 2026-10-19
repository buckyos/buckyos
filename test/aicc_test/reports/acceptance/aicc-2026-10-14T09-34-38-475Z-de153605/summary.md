# AICC E2E Acceptance Report

- Run: `aicc-2026-10-14T09-34-38-475Z-de153605`
- Commit: `e742bc1cf5be6b716ef849e624d5b381a495adb6`
- Capability baseline: `2026-10-13.1`
- Real model calls: 4/4
- Planned maximum cost: $0.040000
- Known actual / unknown estimated exposure: $0.000000 / $0.040000
- Total exposure / budget: $0.040000 / $0.100000
- Unknown-cost calls: 4; budget exceeded: false
- Results: passed=0, failed=0, provider_restricted=0, skipped=0, not_applicable=0, review=4

## Targeted retest

Run after fixing the reported defect; repeat `--case` to select additional cases:

```bash
pnpm run acceptance:gateway -- --config aicc_acceptance.local.toml --allow-credential-mutation --timeout-ms 180000 --no-real-model-calls
```

## Cases

| Case | Layer | Provider | Model | API | Status |
|---|---|---|---|---|---|
| t2.fal.fal-t2.fal-ai-deepfilternet3.audio.enhance | T2 | fal/fal-t2 | fal-ai/deepfilternet3@fal-t2 | audio.enhance | review |
| t2.fal.fal-t2.fal-ai-esrgan.image.upscale | T2 | fal/fal-t2 | fal-ai/esrgan@fal-t2 | image.upscale | review |
| t2.fal.fal-t2.fal-ai-imageutils-rembg.image.bg_remove | T2 | fal/fal-t2 | fal-ai/imageutils/rembg@fal-t2 | image.bg_remove | review |
| t2.fal.fal-t2.fal-ai-video-upscaler.video.upscale | T2 | fal/fal-t2 | fal-ai/video-upscaler@fal-t2 | video.upscale | review |

## Physical model coverage

| Provider | Inventory model | Physical model | Decision | Reason | Retained representative |
|---|---|---|---|---|---|
| fal/fal-t2 | fal-ai/deepfilternet3 | fal-ai/deepfilternet3 | included | physical model | - |
| fal/fal-t2 | fal-ai/esrgan | fal-ai/esrgan | included | physical model | - |
| fal/fal-t2 | fal-ai/imageutils/rembg | fal-ai/imageutils/rembg | included | physical model | - |
| fal/fal-t2 | fal-ai/video-upscaler | fal-ai/video-upscaler | included | physical model | - |
| fal/fal-t2 | fal-ai/deepfilternet3 | fal-ai/deepfilternet3 | included | physical model | - |
| fal/fal-t2 | fal-ai/esrgan | fal-ai/esrgan | included | physical model | - |
| fal/fal-t2 | fal-ai/imageutils/rembg | fal-ai/imageutils/rembg | included | physical model | - |
| fal/fal-t2 | fal-ai/video-upscaler | fal-ai/video-upscaler | included | physical model | - |

## Confirmed product defects

None.

## Cleanup

Status: passed

- removed 26 uploaded fixture object(s)
- removed 0 generated output object(s)
- temporary Provider credentials restored after testing (fal)
