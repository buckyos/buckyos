# AICC E2E Acceptance Report

- Run: `t15-20261030074133-915081`
- Commit: `d40291a9a9085d1cec7ccb37d519d9115baf534c`
- Capability baseline: `2026-10-30.1`
- Real model calls: 0/0
- Planned maximum cost: $0.000000
- Known actual / unknown estimated exposure: $0.000000 / $0.000000
- Total exposure / budget: $0.000000 / $0.000000
- Unknown-cost calls: 0; budget exceeded: false
- Results: passed=0, failed=1, provider_restricted=0, skipped=0, not_applicable=0, review=0
- Manifest coverage: 0/170 (0.00%); passed=0, failed=0

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

- t1.5.sn-ai-provider.sn.audio-chat.v1.audio.asr.error.authentication
- t1.5.sn-ai-provider.sn.audio-chat.v1.audio.asr.error.insufficient_balance
- t1.5.sn-ai-provider.sn.audio-chat.v1.audio.asr.error.invalid_argument
- t1.5.sn-ai-provider.sn.audio-chat.v1.audio.asr.error.timeout
- t1.5.sn-ai-provider.sn.audio-chat.v1.audio.asr.error.unavailable
- t1.5.sn-ai-provider.sn.audio-chat.v1.audio.asr.response.malformed_response
- t1.5.sn-ai-provider.sn.audio-chat.v1.audio.asr.response.missing_required_response_field
- t1.5.sn-ai-provider.sn.audio-chat.v1.audio.asr.response.wrong_content_type
- t1.5.sn-ai-provider.sn.audio-chat.v1.audio.asr.success
- t1.5.sn-ai-provider.sn.openai-audio-speech.v1.audio.tts.error.authentication
- t1.5.sn-ai-provider.sn.openai-audio-speech.v1.audio.tts.error.insufficient_balance
- t1.5.sn-ai-provider.sn.openai-audio-speech.v1.audio.tts.error.invalid_argument
- t1.5.sn-ai-provider.sn.openai-audio-speech.v1.audio.tts.error.timeout
- t1.5.sn-ai-provider.sn.openai-audio-speech.v1.audio.tts.error.unavailable
- t1.5.sn-ai-provider.sn.openai-audio-speech.v1.audio.tts.response.malformed_response
- t1.5.sn-ai-provider.sn.openai-audio-speech.v1.audio.tts.response.missing_required_response_field
- t1.5.sn-ai-provider.sn.openai-audio-speech.v1.audio.tts.response.wrong_content_type
- t1.5.sn-ai-provider.sn.openai-audio-speech.v1.audio.tts.success
- t1.5.sn-ai-provider.sn.openai-audio-speech.v1.audio.tts.ta[REDACTED]
- t1.5.sn-ai-provider.sn.openai-audio-transcriptions.v1.audio.asr.error.authentication
- t1.5.sn-ai-provider.sn.openai-audio-transcriptions.v1.audio.asr.error.insufficient_balance
- t1.5.sn-ai-provider.sn.openai-audio-transcriptions.v1.audio.asr.error.invalid_argument
- t1.5.sn-ai-provider.sn.openai-audio-transcriptions.v1.audio.asr.error.timeout
- t1.5.sn-ai-provider.sn.openai-audio-transcriptions.v1.audio.asr.error.unavailable
- t1.5.sn-ai-provider.sn.openai-audio-transcriptions.v1.audio.asr.response.malformed_response
- t1.5.sn-ai-provider.sn.openai-audio-transcriptions.v1.audio.asr.response.missing_required_response_field
- t1.5.sn-ai-provider.sn.openai-audio-transcriptions.v1.audio.asr.response.wrong_content_type
- t1.5.sn-ai-provider.sn.openai-audio-transcriptions.v1.audio.asr.success
- t1.5.sn-ai-provider.sn.openai-embeddings.v1.embedding.text.error.authentication
- t1.5.sn-ai-provider.sn.openai-embeddings.v1.embedding.text.error.insufficient_balance
- t1.5.sn-ai-provider.sn.openai-embeddings.v1.embedding.text.error.invalid_argument
- t1.5.sn-ai-provider.sn.openai-embeddings.v1.embedding.text.error.timeout
- t1.5.sn-ai-provider.sn.openai-embeddings.v1.embedding.text.error.unavailable
- t1.5.sn-ai-provider.sn.openai-embeddings.v1.embedding.text.response.malformed_response
- t1.5.sn-ai-provider.sn.openai-embeddings.v1.embedding.text.response.missing_required_response_field
- t1.5.sn-ai-provider.sn.openai-embeddings.v1.embedding.text.response.wrong_content_type
- t1.5.sn-ai-provider.sn.openai-embeddings.v1.embedding.text.success
- t1.5.sn-ai-provider.sn.openai-images-edits.v1.image.img2img.error.authentication
- t1.5.sn-ai-provider.sn.openai-images-edits.v1.image.img2img.error.insufficient_balance
- t1.5.sn-ai-provider.sn.openai-images-edits.v1.image.img2img.error.invalid_argument
- t1.5.sn-ai-provider.sn.openai-images-edits.v1.image.img2img.error.timeout
- t1.5.sn-ai-provider.sn.openai-images-edits.v1.image.img2img.error.unavailable
- t1.5.sn-ai-provider.sn.openai-images-edits.v1.image.img2img.response.malformed_response
- t1.5.sn-ai-provider.sn.openai-images-edits.v1.image.img2img.response.missing_required_response_field
- t1.5.sn-ai-provider.sn.openai-images-edits.v1.image.img2img.response.wrong_content_type
- t1.5.sn-ai-provider.sn.openai-images-edits.v1.image.img2img.success
- t1.5.sn-ai-provider.sn.openai-images-edits.v1.image.img2img.ta[REDACTED]
- t1.5.sn-ai-provider.sn.openai-images-edits.v1.image.inpaint.success
- t1.5.sn-ai-provider.sn.openai-images-edits.v1.image.inpaint.ta[REDACTED]
- t1.5.sn-ai-provider.sn.openai-images-generations.v1.image.txt2img.error.authentication
- t1.5.sn-ai-provider.sn.openai-images-generations.v1.image.txt2img.error.insufficient_balance
- t1.5.sn-ai-provider.sn.openai-images-generations.v1.image.txt2img.error.invalid_argument
- t1.5.sn-ai-provider.sn.openai-images-generations.v1.image.txt2img.error.timeout
- t1.5.sn-ai-provider.sn.openai-images-generations.v1.image.txt2img.error.unavailable
- t1.5.sn-ai-provider.sn.openai-images-generations.v1.image.txt2img.response.malformed_response
- t1.5.sn-ai-provider.sn.openai-images-generations.v1.image.txt2img.response.missing_required_response_field
- t1.5.sn-ai-provider.sn.openai-images-generations.v1.image.txt2img.response.wrong_content_type
- t1.5.sn-ai-provider.sn.openai-images-generations.v1.image.txt2img.success
- t1.5.sn-ai-provider.sn.openai-images-generations.v1.image.txt2img.ta[REDACTED]
- t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.img2img.success
- t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.img2img.ta[REDACTED]
- t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.txt2img.success
- t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.txt2img.ta[REDACTED]
- t1.5.sn-ai-provider.sn.openai-responses.background.v1.vision.ocr.async
- t1.5.sn-ai-provider.sn.openai-responses.background.v1.vision.ocr.async-cancel
- t1.5.sn-ai-provider.sn.openai-responses.background.v1.vision.ocr.async-failed
- t1.5.sn-ai-provider.sn.openai-responses.background.v1.vision.ocr.async_artifact_unavailable
- t1.5.sn-ai-provider.sn.openai-responses.background.v1.vision.ocr.async_poll_timeout
- t1.5.sn-ai-provider.sn.openai-responses.background.v1.vision.ocr.background-ignored
- t1.5.sn-ai-provider.sn.openai-responses.background.v1.vision.ocr.error.authentication
- t1.5.sn-ai-provider.sn.openai-responses.background.v1.vision.ocr.error.insufficient_balance
- t1.5.sn-ai-provider.sn.openai-responses.background.v1.vision.ocr.error.invalid_argument
- t1.5.sn-ai-provider.sn.openai-responses.background.v1.vision.ocr.error.timeout
- t1.5.sn-ai-provider.sn.openai-responses.background.v1.vision.ocr.error.unavailable
- t1.5.sn-ai-provider.sn.openai-responses.background.v1.vision.ocr.response.malformed_response
- t1.5.sn-ai-provider.sn.openai-responses.background.v1.vision.ocr.response.missing_required_response_field
- t1.5.sn-ai-provider.sn.openai-responses.background.v1.vision.ocr.response.wrong_content_type
- t1.5.sn-ai-provider.sn.openai-responses.background.v1.vision.ocr.success
- t1.5.sn-ai-provider.sn.openai-responses.computer.v1.agent.computer_use.error.authentication
- t1.5.sn-ai-provider.sn.openai-responses.computer.v1.agent.computer_use.error.insufficient_balance
- t1.5.sn-ai-provider.sn.openai-responses.computer.v1.agent.computer_use.error.invalid_argument
- t1.5.sn-ai-provider.sn.openai-responses.computer.v1.agent.computer_use.error.timeout
- t1.5.sn-ai-provider.sn.openai-responses.computer.v1.agent.computer_use.error.unavailable
- t1.5.sn-ai-provider.sn.openai-responses.computer.v1.agent.computer_use.response.malformed_response
- t1.5.sn-ai-provider.sn.openai-responses.computer.v1.agent.computer_use.response.missing_required_response_field
- t1.5.sn-ai-provider.sn.openai-responses.computer.v1.agent.computer_use.response.wrong_content_type
- t1.5.sn-ai-provider.sn.openai-responses.computer.v1.agent.computer_use.success
- t1.5.sn-ai-provider.sn.openai-responses.v1.llm.error.authentication
- t1.5.sn-ai-provider.sn.openai-responses.v1.llm.error.insufficient_balance
- t1.5.sn-ai-provider.sn.openai-responses.v1.llm.error.invalid_argument
- t1.5.sn-ai-provider.sn.openai-responses.v1.llm.error.timeout
- t1.5.sn-ai-provider.sn.openai-responses.v1.llm.error.unavailable
- t1.5.sn-ai-provider.sn.openai-responses.v1.llm.inline-artifact-provider-state
- t1.5.sn-ai-provider.sn.openai-responses.v1.llm.response.malformed_response
- t1.5.sn-ai-provider.sn.openai-responses.v1.llm.response.missing_required_response_field
- t1.5.sn-ai-provider.sn.openai-responses.v1.llm.response.wrong_content_type
- t1.5.sn-ai-provider.sn.openai-responses.v1.llm.server-default-route
- t1.5.sn-ai-provider.sn.openai-responses.v1.llm.stream
- t1.5.sn-ai-provider.sn.openai-responses.v1.llm.stream-interrupted
- t1.5.sn-ai-provider.sn.openai-responses.v1.llm.success
- t1.5.sn-ai-provider.sn.openai-responses.v1.vision.caption.success
- t1.5.sn-ai-provider.sn.openai-responses.v1.vision.ocr.success
- t1.5.sn-ai-provider.sn.openai-video-edits.v1.video.edit.async
- t1.5.sn-ai-provider.sn.openai-video-edits.v1.video.edit.async-failed
- t1.5.sn-ai-provider.sn.openai-video-edits.v1.video.edit.async_artifact_unavailable
- t1.5.sn-ai-provider.sn.openai-video-edits.v1.video.edit.async_poll_timeout
- t1.5.sn-ai-provider.sn.openai-video-edits.v1.video.edit.error.authentication
- t1.5.sn-ai-provider.sn.openai-video-edits.v1.video.edit.error.insufficient_balance
- t1.5.sn-ai-provider.sn.openai-video-edits.v1.video.edit.error.invalid_argument
- t1.5.sn-ai-provider.sn.openai-video-edits.v1.video.edit.error.timeout
- t1.5.sn-ai-provider.sn.openai-video-edits.v1.video.edit.error.unavailable
- t1.5.sn-ai-provider.sn.openai-video-edits.v1.video.edit.error.upstream_json_required
- t1.5.sn-ai-provider.sn.openai-video-edits.v1.video.edit.generated-artifact-id.same-provider
- t1.5.sn-ai-provider.sn.openai-video-edits.v1.video.edit.response.malformed_response
- t1.5.sn-ai-provider.sn.openai-video-edits.v1.video.edit.response.missing_required_response_field
- t1.5.sn-ai-provider.sn.openai-video-edits.v1.video.edit.response.wrong_content_type
- t1.5.sn-ai-provider.sn.openai-video-edits.v1.video.edit.success
- t1.5.sn-ai-provider.sn.openai-video-edits.v1.video.edit.ta[REDACTED]
- t1.5.sn-ai-provider.sn.openai-video-extensions.v1.video.extend.async
- t1.5.sn-ai-provider.sn.openai-video-extensions.v1.video.extend.async-failed
- t1.5.sn-ai-provider.sn.openai-video-extensions.v1.video.extend.async_artifact_unavailable
- t1.5.sn-ai-provider.sn.openai-video-extensions.v1.video.extend.async_poll_timeout
- t1.5.sn-ai-provider.sn.openai-video-extensions.v1.video.extend.error.authentication
- t1.5.sn-ai-provider.sn.openai-video-extensions.v1.video.extend.error.insufficient_balance
- t1.5.sn-ai-provider.sn.openai-video-extensions.v1.video.extend.error.invalid_argument
- t1.5.sn-ai-provider.sn.openai-video-extensions.v1.video.extend.error.timeout
- t1.5.sn-ai-provider.sn.openai-video-extensions.v1.video.extend.error.unavailable
- t1.5.sn-ai-provider.sn.openai-video-extensions.v1.video.extend.error.upstream_json_required
- t1.5.sn-ai-provider.sn.openai-video-extensions.v1.video.extend.generated-artifact-id.same-provider
- t1.5.sn-ai-provider.sn.openai-video-extensions.v1.video.extend.response.malformed_response
- t1.5.sn-ai-provider.sn.openai-video-extensions.v1.video.extend.response.missing_required_response_field
- t1.5.sn-ai-provider.sn.openai-video-extensions.v1.video.extend.response.wrong_content_type
- t1.5.sn-ai-provider.sn.openai-video-extensions.v1.video.extend.success
- t1.5.sn-ai-provider.sn.openai-video-extensions.v1.video.extend.ta[REDACTED]
- t1.5.sn-ai-provider.sn.openai-videos.v1.video.generate.async
- t1.5.sn-ai-provider.sn.openai-videos.v1.video.generate.async-failed
- t1.5.sn-ai-provider.sn.openai-videos.v1.video.generate.async_artifact_unavailable
- t1.5.sn-ai-provider.sn.openai-videos.v1.video.generate.async_poll_timeout
- t1.5.sn-ai-provider.sn.openai-videos.v1.video.generate.error.authentication
- t1.5.sn-ai-provider.sn.openai-videos.v1.video.generate.error.insufficient_balance
- t1.5.sn-ai-provider.sn.openai-videos.v1.video.generate.error.invalid_argument
- t1.5.sn-ai-provider.sn.openai-videos.v1.video.generate.error.timeout
- t1.5.sn-ai-provider.sn.openai-videos.v1.video.generate.error.unavailable
- t1.5.sn-ai-provider.sn.openai-videos.v1.video.generate.error.upstream_json_required
- t1.5.sn-ai-provider.sn.openai-videos.v1.video.generate.response.malformed_response
- t1.5.sn-ai-provider.sn.openai-videos.v1.video.generate.response.missing_required_response_field
- t1.5.sn-ai-provider.sn.openai-videos.v1.video.generate.response.wrong_content_type
- t1.5.sn-ai-provider.sn.openai-videos.v1.video.generate.success
- t1.5.sn-ai-provider.sn.openai-videos.v1.video.generate.ta[REDACTED]
- t1.5.sn-ai-provider.sn.openai-videos.v1.video.generate.video-keyframe
- t1.5.sn-ai-provider.sn.qwen-images-edits.v1.image.img2img.error.authentication
- t1.5.sn-ai-provider.sn.qwen-images-edits.v1.image.img2img.error.insufficient_balance
- t1.5.sn-ai-provider.sn.qwen-images-edits.v1.image.img2img.error.invalid_argument
- t1.5.sn-ai-provider.sn.qwen-images-edits.v1.image.img2img.error.timeout
- t1.5.sn-ai-provider.sn.qwen-images-edits.v1.image.img2img.error.unavailable
- t1.5.sn-ai-provider.sn.qwen-images-edits.v1.image.img2img.response.malformed_response
- t1.5.sn-ai-provider.sn.qwen-images-edits.v1.image.img2img.response.missing_required_response_field
- t1.5.sn-ai-provider.sn.qwen-images-edits.v1.image.img2img.response.wrong_content_type
- t1.5.sn-ai-provider.sn.qwen-images-edits.v1.image.img2img.success
- t1.5.sn-ai-provider.sn.qwen-images-edits.v1.image.img2img.ta[REDACTED]
- t1.5.sn-ai-provider.sn.qwen-images-generations.v1.image.txt2img.error.authentication
- t1.5.sn-ai-provider.sn.qwen-images-generations.v1.image.txt2img.error.insufficient_balance
- t1.5.sn-ai-provider.sn.qwen-images-generations.v1.image.txt2img.error.invalid_argument
- t1.5.sn-ai-provider.sn.qwen-images-generations.v1.image.txt2img.error.timeout
- t1.5.sn-ai-provider.sn.qwen-images-generations.v1.image.txt2img.error.unavailable
- t1.5.sn-ai-provider.sn.qwen-images-generations.v1.image.txt2img.response.malformed_response
- t1.5.sn-ai-provider.sn.qwen-images-generations.v1.image.txt2img.response.missing_required_response_field
- t1.5.sn-ai-provider.sn.qwen-images-generations.v1.image.txt2img.response.wrong_content_type
- t1.5.sn-ai-provider.sn.qwen-images-generations.v1.image.txt2img.success
- t1.5.sn-ai-provider.sn.qwen-images-generations.v1.image.txt2img.ta[REDACTED]

## Confirmed product defects

- **defect.aicc.t1.5.runner** (AICC, provider_protocol_failed): expected AICC maps the official Provider protocol fixture through its real Adapter; observed captured_requests=0; startup: RPCError: RPC call failed: fetch failed. Evidence: cases/t1.5.runner.json

## Cleanup

Status: passed

- temporary Provider instances were removed and the Mock selection was reset
