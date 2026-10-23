# AICC E2E Acceptance Report

- Run: `aicc-2026-10-23T07-57-10-959Z-cfd52338`
- Commit: `53b4431e2d46ec731598ed0343d4a6596bee8d15`
- Capability baseline: `2026-10-23.1`
- Real model calls: 4/4
- Planned maximum cost: $0.040000
- Known actual / unknown estimated exposure: $0.000000 / $0.040000
- Total exposure / budget: $0.040000 / $30.000000
- Unknown-cost calls: 4; budget exceeded: false
- Results: passed=0, failed=3, provider_restricted=0, skipped=0, not_applicable=0, review=2

## Targeted retest

Run after fixing the reported defect; repeat `--case` to select additional cases:

```bash
pnpm run acceptance:gateway -- --config aicc_acceptance.native.local.toml --allow-credential-mutation --timeout-ms 900000 --provider fal --case t2.fal.native-fal-t2.fal-ai-imageutils-rembg.image.bg_remove --case t2.fal.native-fal-t2.fal-ai-video-upscaler.video.upscale
```

## Cases

| Case | Layer | Provider | Model | API | Status |
|---|---|---|---|---|---|
| t2.fal.native-fal-t2.fal-ai-deepfilternet3.audio.enhance | T2 | fal/native-fal-t2 | fal-ai/deepfilternet3@native-fal-t2 | audio.enhance | review |
| t2.fal.native-fal-t2.fal-ai-esrgan.image.upscale | T2 | fal/native-fal-t2 | fal-ai/esrgan@native-fal-t2 | image.upscale | review |
| t2.fal.native-fal-t2.fal-ai-imageutils-rembg.image.bg_remove | T2 | fal/native-fal-t2 | fal-ai/imageutils/rembg@native-fal-t2 | image.bg_remove | failed |
| t2.fal.native-fal-t2.fal-ai-video-upscaler.video.upscale | T2 | fal/native-fal-t2 | fal-ai/video-upscaler@native-fal-t2 | video.upscale | failed |
| t2.usage.durable_audit | T2 | -/- | - | usage.query | failed |

## Physical model coverage

| Provider | Inventory model | Physical model | Decision | Reason | Retained representative |
|---|---|---|---|---|---|
| fal/native-fal-t2 | fal-ai/deepfilternet3 | fal-ai/deepfilternet3 | included | physical model | - |
| fal/native-fal-t2 | fal-ai/esrgan | fal-ai/esrgan | included | physical model | - |
| fal/native-fal-t2 | fal-ai/imageutils/rembg | fal-ai/imageutils/rembg | included | physical model | - |
| fal/native-fal-t2 | fal-ai/video-upscaler | fal-ai/video-upscaler | included | physical model | - |
| fal/native-fal-t2 | fal-ai/deepfilternet3 | fal-ai/deepfilternet3 | included | physical model | - |
| fal/native-fal-t2 | fal-ai/esrgan | fal-ai/esrgan | included | physical model | - |
| fal/native-fal-t2 | fal-ai/imageutils/rembg | fal-ai/imageutils/rembg | included | physical model | - |
| fal/native-fal-t2 | fal-ai/video-upscaler | fal-ai/video-upscaler | included | physical model | - |

## Confirmed product defects

- **defect.aicc.t2.fal.native-fal-t2.fal-ai-video-upscaler.video.upscale** (AICC, assertion_failed): expected the selected exact model satisfies the case routing, capability, resource, and security assertions; observed RPCError: RPC call error: 500. Evidence: cases/t2.fal.native-fal-t2.fal-ai-video-upscaler.video.upscale.json

## Cleanup

Status: passed

- removed 0 uploaded fixture object(s)
- removed 0 generated output object(s)
- temporary Provider credentials restored after testing (fal)
