# AICC E2E Acceptance Report

- Run: `t15-20261014093215-179644`
- Commit: `e742bc1cf5be6b716ef849e624d5b381a495adb6`
- Capability baseline: `official-provider-protocols-2026-10-13.1`
- Real model calls: 0/0
- Planned maximum cost: $0.000000
- Known actual / unknown estimated exposure: $0.000000 / $0.000000
- Total exposure / budget: $0.000000 / $0.000000
- Unknown-cost calls: 0; budget exceeded: false
- Results: passed=56, failed=0, provider_restricted=0, skipped=0, not_applicable=0, review=0
- Manifest coverage: 56/56 (100.00%); passed=56, failed=0

## Cases

| Case | Layer | Provider | Model | API | Status |
|---|---|---|---|---|---|
| t1.5.fal.fal.esrgan.queue-v1.image.upscale.success | T1.5 | fal/t15-20261014093215-179644-fal | fal-ai/esrgan@t15-20261014093215-179644-fal | image.upscale | passed |
| t1.5.fal.fal.esrgan.queue-v1.image.upscale.ta[REDACTED] | T1.5 | fal/t15-20261014093215-179644-fal | fal-ai/esrgan@t15-20261014093215-179644-fal | image.upscale | passed |
| t1.5.fal.fal.esrgan.queue-v1.image.upscale.async | T1.5 | fal/t15-20261014093215-179644-fal | fal-ai/esrgan@t15-20261014093215-179644-fal | image.upscale | passed |
| t1.5.fal.fal.esrgan.queue-v1.image.upscale.async-failed | T1.5 | fal/t15-20261014093215-179644-fal | fal-ai/esrgan@t15-20261014093215-179644-fal | image.upscale | passed |
| t1.5.fal.fal.esrgan.queue-v1.image.upscale.async_poll_timeout | T1.5 | fal/t15-20261014093215-179644-fal | fal-ai/esrgan@t15-20261014093215-179644-fal | image.upscale | passed |
| t1.5.fal.fal.esrgan.queue-v1.image.upscale.async_artifact_unavailable | T1.5 | fal/t15-20261014093215-179644-fal | fal-ai/esrgan@t15-20261014093215-179644-fal | image.upscale | passed |
| t1.5.fal.fal.esrgan.queue-v1.image.upscale.async-cancel | T1.5 | fal/t15-20261014093215-179644-fal | fal-ai/esrgan@t15-20261014093215-179644-fal | image.upscale | passed |
| t1.5.fal.fal.esrgan.queue-v1.image.upscale.error.validation | T1.5 | fal/t15-20261014093215-179644-fal | fal-ai/esrgan@t15-20261014093215-179644-fal | image.upscale | passed |
| t1.5.fal.fal.esrgan.queue-v1.image.upscale.error.authentication | T1.5 | fal/t15-20261014093215-179644-fal | fal-ai/esrgan@t15-20261014093215-179644-fal | image.upscale | passed |
| t1.5.fal.fal.esrgan.queue-v1.image.upscale.error.rate_limit | T1.5 | fal/t15-20261014093215-179644-fal | fal-ai/esrgan@t15-20261014093215-179644-fal | image.upscale | passed |
| t1.5.fal.fal.esrgan.queue-v1.image.upscale.error.unavailable | T1.5 | fal/t15-20261014093215-179644-fal | fal-ai/esrgan@t15-20261014093215-179644-fal | image.upscale | passed |
| t1.5.fal.fal.esrgan.queue-v1.image.upscale.response.malformed_response | T1.5 | fal/t15-20261014093215-179644-fal | fal-ai/esrgan@t15-20261014093215-179644-fal | image.upscale | passed |
| t1.5.fal.fal.esrgan.queue-v1.image.upscale.response.wrong_content_type | T1.5 | fal/t15-20261014093215-179644-fal | fal-ai/esrgan@t15-20261014093215-179644-fal | image.upscale | passed |
| t1.5.fal.fal.esrgan.queue-v1.image.upscale.response.missing_required_response_field | T1.5 | fal/t15-20261014093215-179644-fal | fal-ai/esrgan@t15-20261014093215-179644-fal | image.upscale | passed |
| t1.5.fal.fal.imageutils-rembg.queue-v1.image.bg_remove.success | T1.5 | fal/t15-20261014093215-179644-fal | fal-ai/imageutils/rembg@t15-20261014093215-179644-fal | image.bg_remove | passed |
| t1.5.fal.fal.imageutils-rembg.queue-v1.image.bg_remove.ta[REDACTED] | T1.5 | fal/t15-20261014093215-179644-fal | fal-ai/imageutils/rembg@t15-20261014093215-179644-fal | image.bg_remove | passed |
| t1.5.fal.fal.imageutils-rembg.queue-v1.image.bg_remove.async | T1.5 | fal/t15-20261014093215-179644-fal | fal-ai/imageutils/rembg@t15-20261014093215-179644-fal | image.bg_remove | passed |
| t1.5.fal.fal.imageutils-rembg.queue-v1.image.bg_remove.async-failed | T1.5 | fal/t15-20261014093215-179644-fal | fal-ai/imageutils/rembg@t15-20261014093215-179644-fal | image.bg_remove | passed |
| t1.5.fal.fal.imageutils-rembg.queue-v1.image.bg_remove.async_poll_timeout | T1.5 | fal/t15-20261014093215-179644-fal | fal-ai/imageutils/rembg@t15-20261014093215-179644-fal | image.bg_remove | passed |
| t1.5.fal.fal.imageutils-rembg.queue-v1.image.bg_remove.async_artifact_unavailable | T1.5 | fal/t15-20261014093215-179644-fal | fal-ai/imageutils/rembg@t15-20261014093215-179644-fal | image.bg_remove | passed |
| t1.5.fal.fal.imageutils-rembg.queue-v1.image.bg_remove.async-cancel | T1.5 | fal/t15-20261014093215-179644-fal | fal-ai/imageutils/rembg@t15-20261014093215-179644-fal | image.bg_remove | passed |
| t1.5.fal.fal.imageutils-rembg.queue-v1.image.bg_remove.error.validation | T1.5 | fal/t15-20261014093215-179644-fal | fal-ai/imageutils/rembg@t15-20261014093215-179644-fal | image.bg_remove | passed |
| t1.5.fal.fal.imageutils-rembg.queue-v1.image.bg_remove.error.authentication | T1.5 | fal/t15-20261014093215-179644-fal | fal-ai/imageutils/rembg@t15-20261014093215-179644-fal | image.bg_remove | passed |
| t1.5.fal.fal.imageutils-rembg.queue-v1.image.bg_remove.error.rate_limit | T1.5 | fal/t15-20261014093215-179644-fal | fal-ai/imageutils/rembg@t15-20261014093215-179644-fal | image.bg_remove | passed |
| t1.5.fal.fal.imageutils-rembg.queue-v1.image.bg_remove.error.unavailable | T1.5 | fal/t15-20261014093215-179644-fal | fal-ai/imageutils/rembg@t15-20261014093215-179644-fal | image.bg_remove | passed |
| t1.5.fal.fal.imageutils-rembg.queue-v1.image.bg_remove.response.malformed_response | T1.5 | fal/t15-20261014093215-179644-fal | fal-ai/imageutils/rembg@t15-20261014093215-179644-fal | image.bg_remove | passed |
| t1.5.fal.fal.imageutils-rembg.queue-v1.image.bg_remove.response.wrong_content_type | T1.5 | fal/t15-20261014093215-179644-fal | fal-ai/imageutils/rembg@t15-20261014093215-179644-fal | image.bg_remove | passed |
| t1.5.fal.fal.imageutils-rembg.queue-v1.image.bg_remove.response.missing_required_response_field | T1.5 | fal/t15-20261014093215-179644-fal | fal-ai/imageutils/rembg@t15-20261014093215-179644-fal | image.bg_remove | passed |
| t1.5.fal.fal.deepfilternet3.queue-v1.audio.enhance.success | T1.5 | fal/t15-20261014093215-179644-fal | fal-ai/deepfilternet3@t15-20261014093215-179644-fal | audio.enhance | passed |
| t1.5.fal.fal.deepfilternet3.queue-v1.audio.enhance.ta[REDACTED] | T1.5 | fal/t15-20261014093215-179644-fal | fal-ai/deepfilternet3@t15-20261014093215-179644-fal | audio.enhance | passed |
| t1.5.fal.fal.deepfilternet3.queue-v1.audio.enhance.async | T1.5 | fal/t15-20261014093215-179644-fal | fal-ai/deepfilternet3@t15-20261014093215-179644-fal | audio.enhance | passed |
| t1.5.fal.fal.deepfilternet3.queue-v1.audio.enhance.async-failed | T1.5 | fal/t15-20261014093215-179644-fal | fal-ai/deepfilternet3@t15-20261014093215-179644-fal | audio.enhance | passed |
| t1.5.fal.fal.deepfilternet3.queue-v1.audio.enhance.async_poll_timeout | T1.5 | fal/t15-20261014093215-179644-fal | fal-ai/deepfilternet3@t15-20261014093215-179644-fal | audio.enhance | passed |
| t1.5.fal.fal.deepfilternet3.queue-v1.audio.enhance.async_artifact_unavailable | T1.5 | fal/t15-20261014093215-179644-fal | fal-ai/deepfilternet3@t15-20261014093215-179644-fal | audio.enhance | passed |
| t1.5.fal.fal.deepfilternet3.queue-v1.audio.enhance.async-cancel | T1.5 | fal/t15-20261014093215-179644-fal | fal-ai/deepfilternet3@t15-20261014093215-179644-fal | audio.enhance | passed |
| t1.5.fal.fal.deepfilternet3.queue-v1.audio.enhance.error.validation | T1.5 | fal/t15-20261014093215-179644-fal | fal-ai/deepfilternet3@t15-20261014093215-179644-fal | audio.enhance | passed |
| t1.5.fal.fal.deepfilternet3.queue-v1.audio.enhance.error.authentication | T1.5 | fal/t15-20261014093215-179644-fal | fal-ai/deepfilternet3@t15-20261014093215-179644-fal | audio.enhance | passed |
| t1.5.fal.fal.deepfilternet3.queue-v1.audio.enhance.error.rate_limit | T1.5 | fal/t15-20261014093215-179644-fal | fal-ai/deepfilternet3@t15-20261014093215-179644-fal | audio.enhance | passed |
| t1.5.fal.fal.deepfilternet3.queue-v1.audio.enhance.error.unavailable | T1.5 | fal/t15-20261014093215-179644-fal | fal-ai/deepfilternet3@t15-20261014093215-179644-fal | audio.enhance | passed |
| t1.5.fal.fal.deepfilternet3.queue-v1.audio.enhance.response.malformed_response | T1.5 | fal/t15-20261014093215-179644-fal | fal-ai/deepfilternet3@t15-20261014093215-179644-fal | audio.enhance | passed |
| t1.5.fal.fal.deepfilternet3.queue-v1.audio.enhance.response.wrong_content_type | T1.5 | fal/t15-20261014093215-179644-fal | fal-ai/deepfilternet3@t15-20261014093215-179644-fal | audio.enhance | passed |
| t1.5.fal.fal.deepfilternet3.queue-v1.audio.enhance.response.missing_required_response_field | T1.5 | fal/t15-20261014093215-179644-fal | fal-ai/deepfilternet3@t15-20261014093215-179644-fal | audio.enhance | passed |
| t1.5.fal.fal.video-upscaler.queue-v1.video.upscale.success | T1.5 | fal/t15-20261014093215-179644-fal | fal-ai/video-upscaler@t15-20261014093215-179644-fal | video.upscale | passed |
| t1.5.fal.fal.video-upscaler.queue-v1.video.upscale.ta[REDACTED] | T1.5 | fal/t15-20261014093215-179644-fal | fal-ai/video-upscaler@t15-20261014093215-179644-fal | video.upscale | passed |
| t1.5.fal.fal.video-upscaler.queue-v1.video.upscale.async | T1.5 | fal/t15-20261014093215-179644-fal | fal-ai/video-upscaler@t15-20261014093215-179644-fal | video.upscale | passed |
| t1.5.fal.fal.video-upscaler.queue-v1.video.upscale.async-failed | T1.5 | fal/t15-20261014093215-179644-fal | fal-ai/video-upscaler@t15-20261014093215-179644-fal | video.upscale | passed |
| t1.5.fal.fal.video-upscaler.queue-v1.video.upscale.async_poll_timeout | T1.5 | fal/t15-20261014093215-179644-fal | fal-ai/video-upscaler@t15-20261014093215-179644-fal | video.upscale | passed |
| t1.5.fal.fal.video-upscaler.queue-v1.video.upscale.async_artifact_unavailable | T1.5 | fal/t15-20261014093215-179644-fal | fal-ai/video-upscaler@t15-20261014093215-179644-fal | video.upscale | passed |
| t1.5.fal.fal.video-upscaler.queue-v1.video.upscale.async-cancel | T1.5 | fal/t15-20261014093215-179644-fal | fal-ai/video-upscaler@t15-20261014093215-179644-fal | video.upscale | passed |
| t1.5.fal.fal.video-upscaler.queue-v1.video.upscale.error.validation | T1.5 | fal/t15-20261014093215-179644-fal | fal-ai/video-upscaler@t15-20261014093215-179644-fal | video.upscale | passed |
| t1.5.fal.fal.video-upscaler.queue-v1.video.upscale.error.authentication | T1.5 | fal/t15-20261014093215-179644-fal | fal-ai/video-upscaler@t15-20261014093215-179644-fal | video.upscale | passed |
| t1.5.fal.fal.video-upscaler.queue-v1.video.upscale.error.rate_limit | T1.5 | fal/t15-20261014093215-179644-fal | fal-ai/video-upscaler@t15-20261014093215-179644-fal | video.upscale | passed |
| t1.5.fal.fal.video-upscaler.queue-v1.video.upscale.error.unavailable | T1.5 | fal/t15-20261014093215-179644-fal | fal-ai/video-upscaler@t15-20261014093215-179644-fal | video.upscale | passed |
| t1.5.fal.fal.video-upscaler.queue-v1.video.upscale.response.malformed_response | T1.5 | fal/t15-20261014093215-179644-fal | fal-ai/video-upscaler@t15-20261014093215-179644-fal | video.upscale | passed |
| t1.5.fal.fal.video-upscaler.queue-v1.video.upscale.response.wrong_content_type | T1.5 | fal/t15-20261014093215-179644-fal | fal-ai/video-upscaler@t15-20261014093215-179644-fal | video.upscale | passed |
| t1.5.fal.fal.video-upscaler.queue-v1.video.upscale.response.missing_required_response_field | T1.5 | fal/t15-20261014093215-179644-fal | fal-ai/video-upscaler@t15-20261014093215-179644-fal | video.upscale | passed |

## Confirmed product defects

None.

## Cleanup

Status: passed

- temporary Provider instances were removed and the Mock selection was reset
