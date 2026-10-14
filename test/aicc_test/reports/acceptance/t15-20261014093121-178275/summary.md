# AICC E2E Acceptance Report

- Run: `t15-20261014093121-178275`
- Commit: `e742bc1cf5be6b716ef849e624d5b381a495adb6`
- Capability baseline: `official-provider-protocols-2026-10-13.1`
- Real model calls: 0/0
- Planned maximum cost: $0.000000
- Known actual / unknown estimated exposure: $0.000000 / $0.000000
- Total exposure / budget: $0.000000 / $0.000000
- Unknown-cost calls: 0; budget exceeded: false
- Results: passed=0, failed=1, provider_restricted=0, skipped=0, not_applicable=0, review=0
- Manifest coverage: 0/56 (0.00%); passed=0, failed=0

## Targeted retest

Run after fixing the reported defect; repeat `--case` to select additional cases:

```bash
AICC_T15_ALLOW_CONFIG_MUTATION=true pnpm run acceptance:t1.5 -- --gateway-url "https://test.buckyos.io" --mock-base-url "http://127.0.0.1:18081" --mock-control-url "http://127.0.0.1:18081" --allow-config-mutation
```

## Cases

| Case | Layer | Provider | Model | API | Status |
|---|---|---|---|---|---|
| t1.5.runner | T1.5 | -/- | - | provider.add/models.list | failed |

## Unexecuted manifest cases

- t1.5.fal.fal.deepfilternet3.queue-v1.audio.enhance.async
- t1.5.fal.fal.deepfilternet3.queue-v1.audio.enhance.async-cancel
- t1.5.fal.fal.deepfilternet3.queue-v1.audio.enhance.async-failed
- t1.5.fal.fal.deepfilternet3.queue-v1.audio.enhance.async_artifact_unavailable
- t1.5.fal.fal.deepfilternet3.queue-v1.audio.enhance.async_poll_timeout
- t1.5.fal.fal.deepfilternet3.queue-v1.audio.enhance.error.authentication
- t1.5.fal.fal.deepfilternet3.queue-v1.audio.enhance.error.rate_limit
- t1.5.fal.fal.deepfilternet3.queue-v1.audio.enhance.error.unavailable
- t1.5.fal.fal.deepfilternet3.queue-v1.audio.enhance.error.validation
- t1.5.fal.fal.deepfilternet3.queue-v1.audio.enhance.response.malformed_response
- t1.5.fal.fal.deepfilternet3.queue-v1.audio.enhance.response.missing_required_response_field
- t1.5.fal.fal.deepfilternet3.queue-v1.audio.enhance.response.wrong_content_type
- t1.5.fal.fal.deepfilternet3.queue-v1.audio.enhance.success
- t1.5.fal.fal.deepfilternet3.queue-v1.audio.enhance.ta[REDACTED]
- t1.5.fal.fal.esrgan.queue-v1.image.upscale.async
- t1.5.fal.fal.esrgan.queue-v1.image.upscale.async-cancel
- t1.5.fal.fal.esrgan.queue-v1.image.upscale.async-failed
- t1.5.fal.fal.esrgan.queue-v1.image.upscale.async_artifact_unavailable
- t1.5.fal.fal.esrgan.queue-v1.image.upscale.async_poll_timeout
- t1.5.fal.fal.esrgan.queue-v1.image.upscale.error.authentication
- t1.5.fal.fal.esrgan.queue-v1.image.upscale.error.rate_limit
- t1.5.fal.fal.esrgan.queue-v1.image.upscale.error.unavailable
- t1.5.fal.fal.esrgan.queue-v1.image.upscale.error.validation
- t1.5.fal.fal.esrgan.queue-v1.image.upscale.response.malformed_response
- t1.5.fal.fal.esrgan.queue-v1.image.upscale.response.missing_required_response_field
- t1.5.fal.fal.esrgan.queue-v1.image.upscale.response.wrong_content_type
- t1.5.fal.fal.esrgan.queue-v1.image.upscale.success
- t1.5.fal.fal.esrgan.queue-v1.image.upscale.ta[REDACTED]
- t1.5.fal.fal.imageutils-rembg.queue-v1.image.bg_remove.async
- t1.5.fal.fal.imageutils-rembg.queue-v1.image.bg_remove.async-cancel
- t1.5.fal.fal.imageutils-rembg.queue-v1.image.bg_remove.async-failed
- t1.5.fal.fal.imageutils-rembg.queue-v1.image.bg_remove.async_artifact_unavailable
- t1.5.fal.fal.imageutils-rembg.queue-v1.image.bg_remove.async_poll_timeout
- t1.5.fal.fal.imageutils-rembg.queue-v1.image.bg_remove.error.authentication
- t1.5.fal.fal.imageutils-rembg.queue-v1.image.bg_remove.error.rate_limit
- t1.5.fal.fal.imageutils-rembg.queue-v1.image.bg_remove.error.unavailable
- t1.5.fal.fal.imageutils-rembg.queue-v1.image.bg_remove.error.validation
- t1.5.fal.fal.imageutils-rembg.queue-v1.image.bg_remove.response.malformed_response
- t1.5.fal.fal.imageutils-rembg.queue-v1.image.bg_remove.response.missing_required_response_field
- t1.5.fal.fal.imageutils-rembg.queue-v1.image.bg_remove.response.wrong_content_type
- t1.5.fal.fal.imageutils-rembg.queue-v1.image.bg_remove.success
- t1.5.fal.fal.imageutils-rembg.queue-v1.image.bg_remove.ta[REDACTED]
- t1.5.fal.fal.video-upscaler.queue-v1.video.upscale.async
- t1.5.fal.fal.video-upscaler.queue-v1.video.upscale.async-cancel
- t1.5.fal.fal.video-upscaler.queue-v1.video.upscale.async-failed
- t1.5.fal.fal.video-upscaler.queue-v1.video.upscale.async_artifact_unavailable
- t1.5.fal.fal.video-upscaler.queue-v1.video.upscale.async_poll_timeout
- t1.5.fal.fal.video-upscaler.queue-v1.video.upscale.error.authentication
- t1.5.fal.fal.video-upscaler.queue-v1.video.upscale.error.rate_limit
- t1.5.fal.fal.video-upscaler.queue-v1.video.upscale.error.unavailable
- t1.5.fal.fal.video-upscaler.queue-v1.video.upscale.error.validation
- t1.5.fal.fal.video-upscaler.queue-v1.video.upscale.response.malformed_response
- t1.5.fal.fal.video-upscaler.queue-v1.video.upscale.response.missing_required_response_field
- t1.5.fal.fal.video-upscaler.queue-v1.video.upscale.response.wrong_content_type
- t1.5.fal.fal.video-upscaler.queue-v1.video.upscale.success
- t1.5.fal.fal.video-upscaler.queue-v1.video.upscale.ta[REDACTED]

## Confirmed product defects

- **defect.aicc.t1.5.runner** (AICC, provider_protocol_failed): expected AICC maps the official Provider protocol fixture through its real Adapter; observed captured_requests=0; startup: Error: username and password are required without session_token. Evidence: cases/t1.5.runner.json

## Cleanup

Status: passed

- temporary Provider instances were removed and the Mock selection was reset
