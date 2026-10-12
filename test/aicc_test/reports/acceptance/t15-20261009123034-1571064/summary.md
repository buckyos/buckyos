# AICC E2E Acceptance Report

- Run: `t15-20261009123034-1571064`
- Commit: `0c32470f0ebaa0f14a1a262bc2bcb0eea553d8fc`
- Capability baseline: `official-provider-protocols-2026-10-08.1`
- Real model calls: 0/0
- Planned maximum cost: $0.000000
- Known actual / unknown estimated exposure: $0.000000 / $0.000000
- Total exposure / budget: $0.000000 / $0.000000
- Unknown-cost calls: 0; budget exceeded: false
- Results: passed=97, failed=1, provider_restricted=0, skipped=0, not_applicable=0, review=0
- Manifest coverage: 98/98 (100.00%); passed=97, failed=1

## Targeted retest

Run after fixing the reported defect; repeat `--case` to select additional cases:

```bash
AICC_T15_ALLOW_CONFIG_MUTATION=true pnpm run acceptance:t1.5 -- --gateway-url "http://127.0.0.1:3180" --mock-base-url "http://127.0.0.1:18081" --mock-control-url "http://127.0.0.1:18081" --allow-config-mutation --case t1.5.qwen.qwen.responses.compatible-v1.llm.video-input
```

## Cases

| Case | Layer | Provider | Model | API | Status |
|---|---|---|---|---|---|
| t1.5.qwen.qwen.responses.compatible-v1.llm.success | T1.5 | qwen/t15-20261009123034-1571064-qwen | qwen3.8-max@t15-20261009123034-1571064-qwen | llm | passed |
| t1.5.qwen.qwen.responses.compatible-v1.llm.stream | T1.5 | qwen/t15-20261009123034-1571064-qwen | qwen3.8-max@t15-20261009123034-1571064-qwen | llm | passed |
| t1.5.qwen.qwen.responses.compatible-v1.llm.stream-interrupted | T1.5 | qwen/t15-20261009123034-1571064-qwen | qwen3.8-max@t15-20261009123034-1571064-qwen | llm | passed |
| t1.5.qwen.qwen.responses.compatible-v1.llm.error.invalid_request | T1.5 | qwen/t15-20261009123034-1571064-qwen | qwen3.8-max@t15-20261009123034-1571064-qwen | llm | passed |
| t1.5.qwen.qwen.responses.compatible-v1.llm.error.authentication | T1.5 | qwen/t15-20261009123034-1571064-qwen | qwen3.8-max@t15-20261009123034-1571064-qwen | llm | passed |
| t1.5.qwen.qwen.responses.compatible-v1.llm.error.permission | T1.5 | qwen/t15-20261009123034-1571064-qwen | qwen3.8-max@t15-20261009123034-1571064-qwen | llm | passed |
| t1.5.qwen.qwen.responses.compatible-v1.llm.error.not_found | T1.5 | qwen/t15-20261009123034-1571064-qwen | qwen3.8-max@t15-20261009123034-1571064-qwen | llm | passed |
| t1.5.qwen.qwen.responses.compatible-v1.llm.error.rate_limit | T1.5 | qwen/t15-20261009123034-1571064-qwen-isolation-1 | qwen3.8-max@t15-20261009123034-1571064-qwen-isolation-1 | llm | passed |
| t1.5.qwen.qwen.responses.compatible-v1.llm.error.server_error | T1.5 | qwen/t15-20261009123034-1571064-qwen-isolation-1 | qwen3.8-max@t15-20261009123034-1571064-qwen-isolation-1 | llm | passed |
| t1.5.qwen.qwen.responses.compatible-v1.llm.response.malformed_response | T1.5 | qwen/t15-20261009123034-1571064-qwen-isolation-1 | qwen3.8-max@t15-20261009123034-1571064-qwen-isolation-1 | llm | passed |
| t1.5.qwen.qwen.responses.compatible-v1.llm.response.wrong_content_type | T1.5 | qwen/t15-20261009123034-1571064-qwen-isolation-1 | qwen3.8-max@t15-20261009123034-1571064-qwen-isolation-1 | llm | passed |
| t1.5.qwen.qwen.responses.compatible-v1.llm.response.missing_required_response_field | T1.5 | qwen/t15-20261009123034-1571064-qwen-isolation-1 | qwen3.8-max@t15-20261009123034-1571064-qwen-isolation-1 | llm | passed |
| t1.5.qwen.qwen.responses.compatible-v1.vision.ocr.success | T1.5 | qwen/t15-20261009123034-1571064-qwen-isolation-1 | qwen3.8-max@t15-20261009123034-1571064-qwen-isolation-1 | vision.ocr | passed |
| t1.5.qwen.qwen.responses.compatible-v1.vision.caption.success | T1.5 | qwen/t15-20261009123034-1571064-qwen-isolation-1 | qwen3.8-max@t15-20261009123034-1571064-qwen-isolation-1 | vision.caption | passed |
| t1.5.qwen.qwen.dashscope.image_synthesis.v1.image.txt2img.success | T1.5 | qwen/t15-20261009123034-1571064-qwen-isolation-1 | wanx2.1-t2i-turbo@t15-20261009123034-1571064-qwen-isolation-1 | image.txt2img | passed |
| t1.5.qwen.qwen.dashscope.image_synthesis.v1.image.txt2img.ta[REDACTED] | T1.5 | qwen/t15-20261009123034-1571064-qwen-isolation-1 | wanx2.1-t2i-turbo@t15-20261009123034-1571064-qwen-isolation-1 | image.txt2img | passed |
| t1.5.qwen.qwen.dashscope.image_synthesis.v1.image.txt2img.async | T1.5 | qwen/t15-20261009123034-1571064-qwen-isolation-1 | wanx2.1-t2i-turbo@t15-20261009123034-1571064-qwen-isolation-1 | image.txt2img | passed |
| t1.5.qwen.qwen.dashscope.image_synthesis.v1.image.txt2img.async-failed | T1.5 | qwen/t15-20261009123034-1571064-qwen-isolation-1 | wanx2.1-t2i-turbo@t15-20261009123034-1571064-qwen-isolation-1 | image.txt2img | passed |
| t1.5.qwen.qwen.dashscope.image_synthesis.v1.image.txt2img.async_poll_timeout | T1.5 | qwen/t15-20261009123034-1571064-qwen-isolation-1 | wanx2.1-t2i-turbo@t15-20261009123034-1571064-qwen-isolation-1 | image.txt2img | passed |
| t1.5.qwen.qwen.dashscope.image_synthesis.v1.image.txt2img.async_artifact_unavailable | T1.5 | qwen/t15-20261009123034-1571064-qwen-isolation-1 | wanx2.1-t2i-turbo@t15-20261009123034-1571064-qwen-isolation-1 | image.txt2img | passed |
| t1.5.qwen.qwen.dashscope.image_synthesis.v1.image.txt2img.error.invalid_request | T1.5 | qwen/t15-20261009123034-1571064-qwen-isolation-1 | wanx2.1-t2i-turbo@t15-20261009123034-1571064-qwen-isolation-1 | image.txt2img | passed |
| t1.5.qwen.qwen.dashscope.image_synthesis.v1.image.txt2img.error.authentication | T1.5 | qwen/t15-20261009123034-1571064-qwen-isolation-1 | wanx2.1-t2i-turbo@t15-20261009123034-1571064-qwen-isolation-1 | image.txt2img | passed |
| t1.5.qwen.qwen.dashscope.image_synthesis.v1.image.txt2img.error.permission | T1.5 | qwen/t15-20261009123034-1571064-qwen-isolation-1 | wanx2.1-t2i-turbo@t15-20261009123034-1571064-qwen-isolation-1 | image.txt2img | passed |
| t1.5.qwen.qwen.dashscope.image_synthesis.v1.image.txt2img.error.not_found | T1.5 | qwen/t15-20261009123034-1571064-qwen-isolation-1 | wanx2.1-t2i-turbo@t15-20261009123034-1571064-qwen-isolation-1 | image.txt2img | passed |
| t1.5.qwen.qwen.dashscope.image_synthesis.v1.image.txt2img.error.rate_limit | T1.5 | qwen/t15-20261009123034-1571064-qwen-isolation-2 | wanx2.1-t2i-turbo@t15-20261009123034-1571064-qwen-isolation-2 | image.txt2img | passed |
| t1.5.qwen.qwen.dashscope.image_synthesis.v1.image.txt2img.error.server_error | T1.5 | qwen/t15-20261009123034-1571064-qwen-isolation-2 | wanx2.1-t2i-turbo@t15-20261009123034-1571064-qwen-isolation-2 | image.txt2img | passed |
| t1.5.qwen.qwen.dashscope.image_synthesis.v1.image.txt2img.response.malformed_response | T1.5 | qwen/t15-20261009123034-1571064-qwen-isolation-2 | wanx2.1-t2i-turbo@t15-20261009123034-1571064-qwen-isolation-2 | image.txt2img | passed |
| t1.5.qwen.qwen.dashscope.image_synthesis.v1.image.txt2img.response.wrong_content_type | T1.5 | qwen/t15-20261009123034-1571064-qwen-isolation-2 | wanx2.1-t2i-turbo@t15-20261009123034-1571064-qwen-isolation-2 | image.txt2img | passed |
| t1.5.qwen.qwen.dashscope.image_synthesis.v1.image.txt2img.response.missing_required_response_field | T1.5 | qwen/t15-20261009123034-1571064-qwen-isolation-2 | wanx2.1-t2i-turbo@t15-20261009123034-1571064-qwen-isolation-2 | image.txt2img | passed |
| t1.5.qwen.qwen.dashscope.image_edit.v1.image.img2img.success | T1.5 | qwen/t15-20261009123034-1571064-qwen-isolation-2 | qwen-image-edit@t15-20261009123034-1571064-qwen-isolation-2 | image.img2img | passed |
| t1.5.qwen.qwen.dashscope.image_edit.v1.image.img2img.ta[REDACTED] | T1.5 | qwen/t15-20261009123034-1571064-qwen-isolation-2 | qwen-image-edit@t15-20261009123034-1571064-qwen-isolation-2 | image.img2img | passed |
| t1.5.qwen.qwen.dashscope.image_edit.v1.image.img2img.error.invalid_request | T1.5 | qwen/t15-20261009123034-1571064-qwen-isolation-2 | qwen-image-edit@t15-20261009123034-1571064-qwen-isolation-2 | image.img2img | passed |
| t1.5.qwen.qwen.dashscope.image_edit.v1.image.img2img.error.authentication | T1.5 | qwen/t15-20261009123034-1571064-qwen-isolation-2 | qwen-image-edit@t15-20261009123034-1571064-qwen-isolation-2 | image.img2img | passed |
| t1.5.qwen.qwen.dashscope.image_edit.v1.image.img2img.error.permission | T1.5 | qwen/t15-20261009123034-1571064-qwen-isolation-2 | qwen-image-edit@t15-20261009123034-1571064-qwen-isolation-2 | image.img2img | passed |
| t1.5.qwen.qwen.dashscope.image_edit.v1.image.img2img.error.not_found | T1.5 | qwen/t15-20261009123034-1571064-qwen-isolation-2 | qwen-image-edit@t15-20261009123034-1571064-qwen-isolation-2 | image.img2img | passed |
| t1.5.qwen.qwen.dashscope.image_edit.v1.image.img2img.error.rate_limit | T1.5 | qwen/t15-20261009123034-1571064-qwen-isolation-3 | qwen-image-edit@t15-20261009123034-1571064-qwen-isolation-3 | image.img2img | passed |
| t1.5.qwen.qwen.dashscope.image_edit.v1.image.img2img.error.server_error | T1.5 | qwen/t15-20261009123034-1571064-qwen-isolation-3 | qwen-image-edit@t15-20261009123034-1571064-qwen-isolation-3 | image.img2img | passed |
| t1.5.qwen.qwen.dashscope.image_edit.v1.image.img2img.response.malformed_response | T1.5 | qwen/t15-20261009123034-1571064-qwen-isolation-3 | qwen-image-edit@t15-20261009123034-1571064-qwen-isolation-3 | image.img2img | passed |
| t1.5.qwen.qwen.dashscope.image_edit.v1.image.img2img.response.wrong_content_type | T1.5 | qwen/t15-20261009123034-1571064-qwen-isolation-3 | qwen-image-edit@t15-20261009123034-1571064-qwen-isolation-3 | image.img2img | passed |
| t1.5.qwen.qwen.dashscope.image_edit.v1.image.img2img.response.missing_required_response_field | T1.5 | qwen/t15-20261009123034-1571064-qwen-isolation-3 | qwen-image-edit@t15-20261009123034-1571064-qwen-isolation-3 | image.img2img | passed |
| t1.5.qwen.qwen.dashscope.video_synthesis.v1.video.txt2video.success | T1.5 | qwen/t15-20261009123034-1571064-qwen-isolation-3 | wan2.6-t2v@t15-20261009123034-1571064-qwen-isolation-3 | video.txt2video | passed |
| t1.5.qwen.qwen.dashscope.video_synthesis.v1.video.txt2video.ta[REDACTED] | T1.5 | qwen/t15-20261009123034-1571064-qwen-isolation-3 | wan2.6-t2v@t15-20261009123034-1571064-qwen-isolation-3 | video.txt2video | passed |
| t1.5.qwen.qwen.dashscope.video_synthesis.v1.video.txt2video.async | T1.5 | qwen/t15-20261009123034-1571064-qwen-isolation-3 | wan2.6-t2v@t15-20261009123034-1571064-qwen-isolation-3 | video.txt2video | passed |
| t1.5.qwen.qwen.dashscope.video_synthesis.v1.video.txt2video.async-failed | T1.5 | qwen/t15-20261009123034-1571064-qwen-isolation-3 | wan2.6-t2v@t15-20261009123034-1571064-qwen-isolation-3 | video.txt2video | passed |
| t1.5.qwen.qwen.dashscope.video_synthesis.v1.video.txt2video.async_poll_timeout | T1.5 | qwen/t15-20261009123034-1571064-qwen-isolation-3 | wan2.6-t2v@t15-20261009123034-1571064-qwen-isolation-3 | video.txt2video | passed |
| t1.5.qwen.qwen.dashscope.video_synthesis.v1.video.txt2video.async_artifact_unavailable | T1.5 | qwen/t15-20261009123034-1571064-qwen-isolation-3 | wan2.6-t2v@t15-20261009123034-1571064-qwen-isolation-3 | video.txt2video | passed |
| t1.5.qwen.qwen.dashscope.video_synthesis.v1.video.txt2video.error.invalid_request | T1.5 | qwen/t15-20261009123034-1571064-qwen-isolation-3 | wan2.6-t2v@t15-20261009123034-1571064-qwen-isolation-3 | video.txt2video | passed |
| t1.5.qwen.qwen.dashscope.video_synthesis.v1.video.txt2video.error.authentication | T1.5 | qwen/t15-20261009123034-1571064-qwen-isolation-3 | wan2.6-t2v@t15-20261009123034-1571064-qwen-isolation-3 | video.txt2video | passed |
| t1.5.qwen.qwen.dashscope.video_synthesis.v1.video.txt2video.error.permission | T1.5 | qwen/t15-20261009123034-1571064-qwen-isolation-3 | wan2.6-t2v@t15-20261009123034-1571064-qwen-isolation-3 | video.txt2video | passed |
| t1.5.qwen.qwen.dashscope.video_synthesis.v1.video.txt2video.error.not_found | T1.5 | qwen/t15-20261009123034-1571064-qwen-isolation-3 | wan2.6-t2v@t15-20261009123034-1571064-qwen-isolation-3 | video.txt2video | passed |
| t1.5.qwen.qwen.dashscope.video_synthesis.v1.video.txt2video.error.rate_limit | T1.5 | qwen/t15-20261009123034-1571064-qwen-isolation-4 | wan2.6-t2v@t15-20261009123034-1571064-qwen-isolation-4 | video.txt2video | passed |
| t1.5.qwen.qwen.dashscope.video_synthesis.v1.video.txt2video.error.server_error | T1.5 | qwen/t15-20261009123034-1571064-qwen-isolation-4 | wan2.6-t2v@t15-20261009123034-1571064-qwen-isolation-4 | video.txt2video | passed |
| t1.5.qwen.qwen.dashscope.video_synthesis.v1.video.txt2video.response.malformed_response | T1.5 | qwen/t15-20261009123034-1571064-qwen-isolation-4 | wan2.6-t2v@t15-20261009123034-1571064-qwen-isolation-4 | video.txt2video | passed |
| t1.5.qwen.qwen.dashscope.video_synthesis.v1.video.txt2video.response.wrong_content_type | T1.5 | qwen/t15-20261009123034-1571064-qwen-isolation-4 | wan2.6-t2v@t15-20261009123034-1571064-qwen-isolation-4 | video.txt2video | passed |
| t1.5.qwen.qwen.dashscope.video_synthesis.v1.video.txt2video.response.missing_required_response_field | T1.5 | qwen/t15-20261009123034-1571064-qwen-isolation-4 | wan2.6-t2v@t15-20261009123034-1571064-qwen-isolation-4 | video.txt2video | passed |
| t1.5.qwen.qwen.responses.compatible-v1.llm.video-input | T1.5 | qwen/t15-20261009123034-1571064-qwen-isolation-4 | qwen3.8-omni-flash@t15-20261009123034-1571064-qwen-isolation-4 | llm | failed |
| t1.5.qwen.qwen.responses.compatible-v1.llm.variant.qwen3.7-plus-reasoning-none | T1.5 | qwen/t15-20261009123034-1571064-qwen-isolation-4 | qwen3.7-plus:reasoning-none@t15-20261009123034-1571064-qwen-isolation-4 | llm | passed |
| t1.5.qwen.qwen.responses.compatible-v1.vision.caption.variant.qwen3.7-plus-reasoning-none | T1.5 | qwen/t15-20261009123034-1571064-qwen-isolation-4 | qwen3.7-plus:reasoning-none@t15-20261009123034-1571064-qwen-isolation-4 | vision.caption | passed |
| t1.5.qwen.qwen.responses.compatible-v1.vision.ocr.variant.qwen3.7-plus-reasoning-none | T1.5 | qwen/t15-20261009123034-1571064-qwen-isolation-4 | qwen3.7-plus:reasoning-none@t15-20261009123034-1571064-qwen-isolation-4 | vision.ocr | passed |
| t1.5.qwen.qwen.responses.compatible-v1.llm.variant.qwen3.7-plus-reasoning-thinking | T1.5 | qwen/t15-20261009123034-1571064-qwen-isolation-4 | qwen3.7-plus:reasoning-thinking@t15-20261009123034-1571064-qwen-isolation-4 | llm | passed |
| t1.5.qwen.qwen.responses.compatible-v1.vision.caption.variant.qwen3.7-plus-reasoning-thinking | T1.5 | qwen/t15-20261009123034-1571064-qwen-isolation-4 | qwen3.7-plus:reasoning-thinking@t15-20261009123034-1571064-qwen-isolation-4 | vision.caption | passed |
| t1.5.qwen.qwen.responses.compatible-v1.vision.ocr.variant.qwen3.7-plus-reasoning-thinking | T1.5 | qwen/t15-20261009123034-1571064-qwen-isolation-4 | qwen3.7-plus:reasoning-thinking@t15-20261009123034-1571064-qwen-isolation-4 | vision.ocr | passed |
| t1.5.qwen.qwen.responses.compatible-v1.llm.variant.qwen3.8-flash-reasoning-low | T1.5 | qwen/t15-20261009123034-1571064-qwen-isolation-4 | qwen3.8-flash:reasoning-low@t15-20261009123034-1571064-qwen-isolation-4 | llm | passed |
| t1.5.qwen.qwen.responses.compatible-v1.vision.caption.variant.qwen3.8-flash-reasoning-low | T1.5 | qwen/t15-20261009123034-1571064-qwen-isolation-4 | qwen3.8-flash:reasoning-low@t15-20261009123034-1571064-qwen-isolation-4 | vision.caption | passed |
| t1.5.qwen.qwen.responses.compatible-v1.vision.ocr.variant.qwen3.8-flash-reasoning-low | T1.5 | qwen/t15-20261009123034-1571064-qwen-isolation-4 | qwen3.8-flash:reasoning-low@t15-20261009123034-1571064-qwen-isolation-4 | vision.ocr | passed |
| t1.5.qwen.qwen.responses.compatible-v1.llm.variant.qwen3.8-flash-reasoning-medium | T1.5 | qwen/t15-20261009123034-1571064-qwen-isolation-4 | qwen3.8-flash:reasoning-medium@t15-20261009123034-1571064-qwen-isolation-4 | llm | passed |
| t1.5.qwen.qwen.responses.compatible-v1.vision.caption.variant.qwen3.8-flash-reasoning-medium | T1.5 | qwen/t15-20261009123034-1571064-qwen-isolation-4 | qwen3.8-flash:reasoning-medium@t15-20261009123034-1571064-qwen-isolation-4 | vision.caption | passed |
| t1.5.qwen.qwen.responses.compatible-v1.vision.ocr.variant.qwen3.8-flash-reasoning-medium | T1.5 | qwen/t15-20261009123034-1571064-qwen-isolation-4 | qwen3.8-flash:reasoning-medium@t15-20261009123034-1571064-qwen-isolation-4 | vision.ocr | passed |
| t1.5.qwen.qwen.responses.compatible-v1.llm.variant.qwen3.8-flash-reasoning-none | T1.5 | qwen/t15-20261009123034-1571064-qwen-isolation-4 | qwen3.8-flash:reasoning-none@t15-20261009123034-1571064-qwen-isolation-4 | llm | passed |
| t1.5.qwen.qwen.responses.compatible-v1.vision.caption.variant.qwen3.8-flash-reasoning-none | T1.5 | qwen/t15-20261009123034-1571064-qwen-isolation-4 | qwen3.8-flash:reasoning-none@t15-20261009123034-1571064-qwen-isolation-4 | vision.caption | passed |
| t1.5.qwen.qwen.responses.compatible-v1.vision.ocr.variant.qwen3.8-flash-reasoning-none | T1.5 | qwen/t15-20261009123034-1571064-qwen-isolation-4 | qwen3.8-flash:reasoning-none@t15-20261009123034-1571064-qwen-isolation-4 | vision.ocr | passed |
| t1.5.qwen.qwen.responses.compatible-v1.llm.variant.qwen3.8-flash-reasoning-xhigh | T1.5 | qwen/t15-20261009123034-1571064-qwen-isolation-4 | qwen3.8-flash:reasoning-xhigh@t15-20261009123034-1571064-qwen-isolation-4 | llm | passed |
| t1.5.qwen.qwen.responses.compatible-v1.vision.caption.variant.qwen3.8-flash-reasoning-xhigh | T1.5 | qwen/t15-20261009123034-1571064-qwen-isolation-4 | qwen3.8-flash:reasoning-xhigh@t15-20261009123034-1571064-qwen-isolation-4 | vision.caption | passed |
| t1.5.qwen.qwen.responses.compatible-v1.vision.ocr.variant.qwen3.8-flash-reasoning-xhigh | T1.5 | qwen/t15-20261009123034-1571064-qwen-isolation-4 | qwen3.8-flash:reasoning-xhigh@t15-20261009123034-1571064-qwen-isolation-4 | vision.ocr | passed |
| t1.5.qwen.qwen.responses.compatible-v1.llm.variant.qwen3.8-max-reasoning-low | T1.5 | qwen/t15-20261009123034-1571064-qwen-isolation-4 | qwen3.8-max:reasoning-low@t15-20261009123034-1571064-qwen-isolation-4 | llm | passed |
| t1.5.qwen.qwen.responses.compatible-v1.vision.caption.variant.qwen3.8-max-reasoning-low | T1.5 | qwen/t15-20261009123034-1571064-qwen-isolation-4 | qwen3.8-max:reasoning-low@t15-20261009123034-1571064-qwen-isolation-4 | vision.caption | passed |
| t1.5.qwen.qwen.responses.compatible-v1.vision.ocr.variant.qwen3.8-max-reasoning-low | T1.5 | qwen/t15-20261009123034-1571064-qwen-isolation-4 | qwen3.8-max:reasoning-low@t15-20261009123034-1571064-qwen-isolation-4 | vision.ocr | passed |
| t1.5.qwen.qwen.responses.compatible-v1.llm.variant.qwen3.8-max-reasoning-medium | T1.5 | qwen/t15-20261009123034-1571064-qwen-isolation-4 | qwen3.8-max:reasoning-medium@t15-20261009123034-1571064-qwen-isolation-4 | llm | passed |
| t1.5.qwen.qwen.responses.compatible-v1.vision.caption.variant.qwen3.8-max-reasoning-medium | T1.5 | qwen/t15-20261009123034-1571064-qwen-isolation-4 | qwen3.8-max:reasoning-medium@t15-20261009123034-1571064-qwen-isolation-4 | vision.caption | passed |
| t1.5.qwen.qwen.responses.compatible-v1.vision.ocr.variant.qwen3.8-max-reasoning-medium | T1.5 | qwen/t15-20261009123034-1571064-qwen-isolation-4 | qwen3.8-max:reasoning-medium@t15-20261009123034-1571064-qwen-isolation-4 | vision.ocr | passed |
| t1.5.qwen.qwen.responses.compatible-v1.llm.variant.qwen3.8-max-reasoning-none | T1.5 | qwen/t15-20261009123034-1571064-qwen-isolation-4 | qwen3.8-max:reasoning-none@t15-20261009123034-1571064-qwen-isolation-4 | llm | passed |
| t1.5.qwen.qwen.responses.compatible-v1.vision.caption.variant.qwen3.8-max-reasoning-none | T1.5 | qwen/t15-20261009123034-1571064-qwen-isolation-4 | qwen3.8-max:reasoning-none@t15-20261009123034-1571064-qwen-isolation-4 | vision.caption | passed |
| t1.5.qwen.qwen.responses.compatible-v1.vision.ocr.variant.qwen3.8-max-reasoning-none | T1.5 | qwen/t15-20261009123034-1571064-qwen-isolation-4 | qwen3.8-max:reasoning-none@t15-20261009123034-1571064-qwen-isolation-4 | vision.ocr | passed |
| t1.5.qwen.qwen.responses.compatible-v1.llm.variant.qwen3.8-max-reasoning-xhigh | T1.5 | qwen/t15-20261009123034-1571064-qwen-isolation-4 | qwen3.8-max:reasoning-xhigh@t15-20261009123034-1571064-qwen-isolation-4 | llm | passed |
| t1.5.qwen.qwen.responses.compatible-v1.vision.caption.variant.qwen3.8-max-reasoning-xhigh | T1.5 | qwen/t15-20261009123034-1571064-qwen-isolation-4 | qwen3.8-max:reasoning-xhigh@t15-20261009123034-1571064-qwen-isolation-4 | vision.caption | passed |
| t1.5.qwen.qwen.responses.compatible-v1.vision.ocr.variant.qwen3.8-max-reasoning-xhigh | T1.5 | qwen/t15-20261009123034-1571064-qwen-isolation-4 | qwen3.8-max:reasoning-xhigh@t15-20261009123034-1571064-qwen-isolation-4 | vision.ocr | passed |
| t1.5.qwen.qwen.responses.compatible-v1.llm.variant.qwen3.8-omni-flash-reasoning-low | T1.5 | qwen/t15-20261009123034-1571064-qwen-isolation-4 | qwen3.8-omni-flash:reasoning-low@t15-20261009123034-1571064-qwen-isolation-4 | llm | passed |
| t1.5.qwen.qwen.responses.compatible-v1.vision.caption.variant.qwen3.8-omni-flash-reasoning-low | T1.5 | qwen/t15-20261009123034-1571064-qwen-isolation-4 | qwen3.8-omni-flash:reasoning-low@t15-20261009123034-1571064-qwen-isolation-4 | vision.caption | passed |
| t1.5.qwen.qwen.responses.compatible-v1.vision.ocr.variant.qwen3.8-omni-flash-reasoning-low | T1.5 | qwen/t15-20261009123034-1571064-qwen-isolation-4 | qwen3.8-omni-flash:reasoning-low@t15-20261009123034-1571064-qwen-isolation-4 | vision.ocr | passed |
| t1.5.qwen.qwen.responses.compatible-v1.llm.variant.qwen3.8-omni-flash-reasoning-medium | T1.5 | qwen/t15-20261009123034-1571064-qwen-isolation-4 | qwen3.8-omni-flash:reasoning-medium@t15-20261009123034-1571064-qwen-isolation-4 | llm | passed |
| t1.5.qwen.qwen.responses.compatible-v1.vision.caption.variant.qwen3.8-omni-flash-reasoning-medium | T1.5 | qwen/t15-20261009123034-1571064-qwen-isolation-4 | qwen3.8-omni-flash:reasoning-medium@t15-20261009123034-1571064-qwen-isolation-4 | vision.caption | passed |
| t1.5.qwen.qwen.responses.compatible-v1.vision.ocr.variant.qwen3.8-omni-flash-reasoning-medium | T1.5 | qwen/t15-20261009123034-1571064-qwen-isolation-4 | qwen3.8-omni-flash:reasoning-medium@t15-20261009123034-1571064-qwen-isolation-4 | vision.ocr | passed |
| t1.5.qwen.qwen.responses.compatible-v1.llm.variant.qwen3.8-omni-flash-reasoning-none | T1.5 | qwen/t15-20261009123034-1571064-qwen-isolation-4 | qwen3.8-omni-flash:reasoning-none@t15-20261009123034-1571064-qwen-isolation-4 | llm | passed |
| t1.5.qwen.qwen.responses.compatible-v1.vision.caption.variant.qwen3.8-omni-flash-reasoning-none | T1.5 | qwen/t15-20261009123034-1571064-qwen-isolation-4 | qwen3.8-omni-flash:reasoning-none@t15-20261009123034-1571064-qwen-isolation-4 | vision.caption | passed |
| t1.5.qwen.qwen.responses.compatible-v1.vision.ocr.variant.qwen3.8-omni-flash-reasoning-none | T1.5 | qwen/t15-20261009123034-1571064-qwen-isolation-4 | qwen3.8-omni-flash:reasoning-none@t15-20261009123034-1571064-qwen-isolation-4 | vision.ocr | passed |
| t1.5.qwen.qwen.responses.compatible-v1.llm.variant.qwen3.8-omni-flash-reasoning-xhigh | T1.5 | qwen/t15-20261009123034-1571064-qwen-isolation-4 | qwen3.8-omni-flash:reasoning-xhigh@t15-20261009123034-1571064-qwen-isolation-4 | llm | passed |
| t1.5.qwen.qwen.responses.compatible-v1.vision.caption.variant.qwen3.8-omni-flash-reasoning-xhigh | T1.5 | qwen/t15-20261009123034-1571064-qwen-isolation-4 | qwen3.8-omni-flash:reasoning-xhigh@t15-20261009123034-1571064-qwen-isolation-4 | vision.caption | passed |
| t1.5.qwen.qwen.responses.compatible-v1.vision.ocr.variant.qwen3.8-omni-flash-reasoning-xhigh | T1.5 | qwen/t15-20261009123034-1571064-qwen-isolation-4 | qwen3.8-omni-flash:reasoning-xhigh@t15-20261009123034-1571064-qwen-isolation-4 | vision.ocr | passed |

## Confirmed product defects

- **defect.aicc.t1.5.qwen.qwen.responses.compatible-v1.llm.video-input** (AICC, provider_protocol_failed): expected AICC maps the official Provider protocol fixture through its real Adapter; observed contract=qwen.responses.compatible-v1; scenario=success; captured_requests=1; wire contract violations: ["body field input[0].content[1].type=input_video; user message content is invalid"]; Error: AICC returned failed: {"error":{"code":"provider_error","details":{"request_id":"aicc-18dcdc90c7725095-1f1","retry_after_ms":null},"message":"OpenAI http_error: OpenAI request failed"},"event_ref":"/task_mgr/t-6d5ada825fe5415baaa0acf119723c23","route_trace":{"attempts":[],"final_model":"qwen3.8-omni-flash@t15-20261009123034-1571064-qwen-isolation-4"},"status":"failed","task_id":"t-6d5ada825fe5415baaa0acf119723c23"}. Evidence: cases/t1.5.qwen.qwen.responses.compatible-v1.llm.video-input.json

## Cleanup

Status: passed

- temporary Provider instances were removed and the Mock selection was reset
