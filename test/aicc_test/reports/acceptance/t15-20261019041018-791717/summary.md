# AICC E2E Acceptance Report

- Run: `t15-20261019041018-791717`
- Commit: `1e8f93081e1a1b9561e77a78e3f7cb67f9922330`
- Capability baseline: `official-provider-protocols-2026-10-19.2`
- Real model calls: 0/0
- Planned maximum cost: $0.000000
- Known actual / unknown estimated exposure: $0.000000 / $0.000000
- Total exposure / budget: $0.000000 / $0.000000
- Unknown-cost calls: 0; budget exceeded: false
- Results: passed=0, failed=1, provider_restricted=0, skipped=0, not_applicable=0, review=0
- Manifest coverage: 0/88 (0.00%); passed=0, failed=0

## Targeted retest

Run after fixing the reported defect; repeat `--case` to select additional cases:

```bash
AICC_T15_ALLOW_CONFIG_MUTATION=true pnpm run acceptance:t1.5 -- --gateway-url "http://127.0.0.1:3180" --mock-base-url "http://127.0.0.1:18081" --mock-control-url "http://127.0.0.1:18081" --allow-config-mutation
```

## Cases

| Case | Layer | Provider | Model | API | Status |
|---|---|---|---|---|---|
| t1.5.runner | T1.5 | -/- | - | provider.add/models.list | failed |

## Unexecuted manifest cases

- t1.5.qwen.qwen.chat-completions.compatible-v1.llm.error.authentication
- t1.5.qwen.qwen.chat-completions.compatible-v1.llm.error.invalid_request
- t1.5.qwen.qwen.chat-completions.compatible-v1.llm.error.not_found
- t1.5.qwen.qwen.chat-completions.compatible-v1.llm.error.permission
- t1.5.qwen.qwen.chat-completions.compatible-v1.llm.error.rate_limit
- t1.5.qwen.qwen.chat-completions.compatible-v1.llm.error.server_error
- t1.5.qwen.qwen.chat-completions.compatible-v1.llm.response.malformed_response
- t1.5.qwen.qwen.chat-completions.compatible-v1.llm.response.missing_required_response_field
- t1.5.qwen.qwen.chat-completions.compatible-v1.llm.response.wrong_content_type
- t1.5.qwen.qwen.chat-completions.compatible-v1.llm.stream
- t1.5.qwen.qwen.chat-completions.compatible-v1.llm.stream-interrupted
- t1.5.qwen.qwen.chat-completions.compatible-v1.llm.success
- t1.5.qwen.qwen.chat-completions.compatible-v1.vision.caption.success
- t1.5.qwen.qwen.chat-completions.compatible-v1.vision.ocr.success
- t1.5.qwen.qwen.dashscope.image_edit.v1.image.img2img.error.authentication
- t1.5.qwen.qwen.dashscope.image_edit.v1.image.img2img.error.invalid_request
- t1.5.qwen.qwen.dashscope.image_edit.v1.image.img2img.error.not_found
- t1.5.qwen.qwen.dashscope.image_edit.v1.image.img2img.error.permission
- t1.5.qwen.qwen.dashscope.image_edit.v1.image.img2img.error.rate_limit
- t1.5.qwen.qwen.dashscope.image_edit.v1.image.img2img.error.server_error
- t1.5.qwen.qwen.dashscope.image_edit.v1.image.img2img.response.malformed_response
- t1.5.qwen.qwen.dashscope.image_edit.v1.image.img2img.response.missing_required_response_field
- t1.5.qwen.qwen.dashscope.image_edit.v1.image.img2img.response.wrong_content_type
- t1.5.qwen.qwen.dashscope.image_edit.v1.image.img2img.success
- t1.5.qwen.qwen.dashscope.image_edit.v1.image.img2img.ta[REDACTED]
- t1.5.qwen.qwen.dashscope.image_generation.v1.image.img2img.success
- t1.5.qwen.qwen.dashscope.image_generation.v1.image.img2img.ta[REDACTED]
- t1.5.qwen.qwen.dashscope.image_generation.v1.image.txt2img.async
- t1.5.qwen.qwen.dashscope.image_generation.v1.image.txt2img.async-failed
- t1.5.qwen.qwen.dashscope.image_generation.v1.image.txt2img.async_artifact_unavailable
- t1.5.qwen.qwen.dashscope.image_generation.v1.image.txt2img.async_poll_timeout
- t1.5.qwen.qwen.dashscope.image_generation.v1.image.txt2img.error.authentication
- t1.5.qwen.qwen.dashscope.image_generation.v1.image.txt2img.error.invalid_request
- t1.5.qwen.qwen.dashscope.image_generation.v1.image.txt2img.error.not_found
- t1.5.qwen.qwen.dashscope.image_generation.v1.image.txt2img.error.permission
- t1.5.qwen.qwen.dashscope.image_generation.v1.image.txt2img.error.rate_limit
- t1.5.qwen.qwen.dashscope.image_generation.v1.image.txt2img.error.server_error
- t1.5.qwen.qwen.dashscope.image_generation.v1.image.txt2img.response.malformed_response
- t1.5.qwen.qwen.dashscope.image_generation.v1.image.txt2img.response.missing_required_response_field
- t1.5.qwen.qwen.dashscope.image_generation.v1.image.txt2img.response.wrong_content_type
- t1.5.qwen.qwen.dashscope.image_generation.v1.image.txt2img.success
- t1.5.qwen.qwen.dashscope.image_generation.v1.image.txt2img.ta[REDACTED]
- t1.5.qwen.qwen.dashscope.image_synthesis.v1.image.txt2img.async
- t1.5.qwen.qwen.dashscope.image_synthesis.v1.image.txt2img.async-failed
- t1.5.qwen.qwen.dashscope.image_synthesis.v1.image.txt2img.async_artifact_unavailable
- t1.5.qwen.qwen.dashscope.image_synthesis.v1.image.txt2img.async_poll_timeout
- t1.5.qwen.qwen.dashscope.image_synthesis.v1.image.txt2img.error.authentication
- t1.5.qwen.qwen.dashscope.image_synthesis.v1.image.txt2img.error.invalid_request
- t1.5.qwen.qwen.dashscope.image_synthesis.v1.image.txt2img.error.not_found
- t1.5.qwen.qwen.dashscope.image_synthesis.v1.image.txt2img.error.permission
- t1.5.qwen.qwen.dashscope.image_synthesis.v1.image.txt2img.error.rate_limit
- t1.5.qwen.qwen.dashscope.image_synthesis.v1.image.txt2img.error.server_error
- t1.5.qwen.qwen.dashscope.image_synthesis.v1.image.txt2img.response.malformed_response
- t1.5.qwen.qwen.dashscope.image_synthesis.v1.image.txt2img.response.missing_required_response_field
- t1.5.qwen.qwen.dashscope.image_synthesis.v1.image.txt2img.response.wrong_content_type
- t1.5.qwen.qwen.dashscope.image_synthesis.v1.image.txt2img.success
- t1.5.qwen.qwen.dashscope.image_synthesis.v1.image.txt2img.ta[REDACTED]
- t1.5.qwen.qwen.dashscope.video_synthesis.v1.video.txt2video.async
- t1.5.qwen.qwen.dashscope.video_synthesis.v1.video.txt2video.async-failed
- t1.5.qwen.qwen.dashscope.video_synthesis.v1.video.txt2video.async_artifact_unavailable
- t1.5.qwen.qwen.dashscope.video_synthesis.v1.video.txt2video.async_poll_timeout
- t1.5.qwen.qwen.dashscope.video_synthesis.v1.video.txt2video.error.authentication
- t1.5.qwen.qwen.dashscope.video_synthesis.v1.video.txt2video.error.invalid_request
- t1.5.qwen.qwen.dashscope.video_synthesis.v1.video.txt2video.error.not_found
- t1.5.qwen.qwen.dashscope.video_synthesis.v1.video.txt2video.error.permission
- t1.5.qwen.qwen.dashscope.video_synthesis.v1.video.txt2video.error.rate_limit
- t1.5.qwen.qwen.dashscope.video_synthesis.v1.video.txt2video.error.server_error
- t1.5.qwen.qwen.dashscope.video_synthesis.v1.video.txt2video.response.malformed_response
- t1.5.qwen.qwen.dashscope.video_synthesis.v1.video.txt2video.response.missing_required_response_field
- t1.5.qwen.qwen.dashscope.video_synthesis.v1.video.txt2video.response.wrong_content_type
- t1.5.qwen.qwen.dashscope.video_synthesis.v1.video.txt2video.success
- t1.5.qwen.qwen.dashscope.video_synthesis.v1.video.txt2video.ta[REDACTED]
- t1.5.qwen.qwen.responses.compatible-v1.llm.error.authentication
- t1.5.qwen.qwen.responses.compatible-v1.llm.error.invalid_request
- t1.5.qwen.qwen.responses.compatible-v1.llm.error.not_found
- t1.5.qwen.qwen.responses.compatible-v1.llm.error.permission
- t1.5.qwen.qwen.responses.compatible-v1.llm.error.rate_limit
- t1.5.qwen.qwen.responses.compatible-v1.llm.error.server_error
- t1.5.qwen.qwen.responses.compatible-v1.llm.inline-artifact-provider-state
- t1.5.qwen.qwen.responses.compatible-v1.llm.response.malformed_response
- t1.5.qwen.qwen.responses.compatible-v1.llm.response.missing_required_response_field
- t1.5.qwen.qwen.responses.compatible-v1.llm.response.wrong_content_type
- t1.5.qwen.qwen.responses.compatible-v1.llm.stream
- t1.5.qwen.qwen.responses.compatible-v1.llm.stream-interrupted
- t1.5.qwen.qwen.responses.compatible-v1.llm.success
- t1.5.qwen.qwen.responses.compatible-v1.llm.video-input
- t1.5.qwen.qwen.responses.compatible-v1.vision.caption.success
- t1.5.qwen.qwen.responses.compatible-v1.vision.ocr.success

## Confirmed product defects

- **defect.aicc.t1.5.runner** (AICC, provider_protocol_failed): expected AICC maps the official Provider protocol fixture through its real Adapter; observed captured_requests=0; provider:qwen:add: Error: qwen add failed: RPCError: RPC call error: Failed due to reason: provider validation failed; settings were not changed; validation={"auth_valid":true,"balance_available":false,"base_url_reachable":true,"error_details":[{"kind":"models","message":"provider validation failed at Inventory: Inventory"}],"errors":["provider validation failed at Inventory: Inventory"],"models_discovered":[],"resolved_protocol_adapter_id":"qwen-responses"}. Evidence: cases/t1.5.runner.json

## Cleanup

Status: passed

- temporary Provider instances were removed and the Mock selection was reset
