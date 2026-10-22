# AICC E2E Acceptance Report

- Run: `t15-20261014120240-424758`
- Commit: `e742bc1cf5be6b716ef849e624d5b381a495adb6`
- Capability baseline: `official-provider-protocols-2026-10-14.1`
- Real model calls: 0/0
- Planned maximum cost: $0.000000
- Known actual / unknown estimated exposure: $0.000000 / $0.000000
- Total exposure / budget: $0.000000 / $0.000000
- Unknown-cost calls: 0; budget exceeded: false
- Results: passed=0, failed=1, provider_restricted=0, skipped=0, not_applicable=0, review=0
- Manifest coverage: 0/121 (0.00%); passed=0, failed=0

## Targeted retest

Run after fixing the reported defect; repeat `--case` to select additional cases:

```bash
AICC_T15_ALLOW_CONFIG_MUTATION=true pnpm run acceptance:t1.5 -- --gateway-url "https://zz-dev-7.web3.buckyos.ai" --mock-base-url "http://127.0.0.1:18081" --mock-control-url "http://127.0.0.1:18081" --allow-config-mutation
```

## Cases

| Case | Layer | Provider | Model | API | Status |
|---|---|---|---|---|---|
| t1.5.runner | T1.5 | -/- | - | provider.add/models.list | failed |

## Unexecuted manifest cases

- t1.5.custom.google-gemini.gemini.embed-content.v1beta.embedding.multimodal.success
- t1.5.custom.google-gemini.gemini.embed-content.v1beta.embedding.text.success
- t1.5.custom.google-gemini.gemini.interactions.image.v1beta.image.img2img.success
- t1.5.custom.google-gemini.gemini.interactions.image.v1beta.image.txt2img.success
- t1.5.custom.google-gemini.gemini.interactions.music.v1beta.audio.music.success
- t1.5.custom.google-gemini.gemini.interactions.transcription.v1beta.audio.asr.success
- t1.5.custom.google-gemini.gemini.interactions.tts.v1beta.audio.tts.success
- t1.5.custom.google-gemini.gemini.interactions.v1beta.llm.success
- t1.5.custom.google-gemini.gemini.interactions.v1beta.vision.caption.success
- t1.5.custom.google-gemini.gemini.interactions.v1beta.vision.detect.success
- t1.5.custom.google-gemini.gemini.interactions.v1beta.vision.ocr.success
- t1.5.custom.google-gemini.gemini.interactions.v1beta.vision.segment.success
- t1.5.custom.google-gemini.gemini.interactions.video.v1beta.video.extend.success
- t1.5.custom.google-gemini.gemini.interactions.video.v1beta.video.video2video.success
- t1.5.custom.google-gemini.gemini.predict-long-running.v1beta.video.img2video.success
- t1.5.custom.google-gemini.gemini.predict-long-running.v1beta.video.txt2video.success
- t1.5.google-gemini.gemini.embed-content.v1beta.embedding.multimodal.success
- t1.5.google-gemini.gemini.embed-content.v1beta.embedding.text.error.authentication
- t1.5.google-gemini.gemini.embed-content.v1beta.embedding.text.error.invalid_argument
- t1.5.google-gemini.gemini.embed-content.v1beta.embedding.text.error.not_found
- t1.5.google-gemini.gemini.embed-content.v1beta.embedding.text.error.permission
- t1.5.google-gemini.gemini.embed-content.v1beta.embedding.text.error.quota
- t1.5.google-gemini.gemini.embed-content.v1beta.embedding.text.error.unavailable
- t1.5.google-gemini.gemini.embed-content.v1beta.embedding.text.response.malformed_response
- t1.5.google-gemini.gemini.embed-content.v1beta.embedding.text.response.missing_required_response_field
- t1.5.google-gemini.gemini.embed-content.v1beta.embedding.text.response.wrong_content_type
- t1.5.google-gemini.gemini.embed-content.v1beta.embedding.text.success
- t1.5.google-gemini.gemini.interactions.image.v1beta.image.img2img.generated-artifact-id.same-provider
- t1.5.google-gemini.gemini.interactions.image.v1beta.image.img2img.success
- t1.5.google-gemini.gemini.interactions.image.v1beta.image.img2img.ta[REDACTED]
- t1.5.google-gemini.gemini.interactions.image.v1beta.image.txt2img.error.authentication
- t1.5.google-gemini.gemini.interactions.image.v1beta.image.txt2img.error.invalid_argument
- t1.5.google-gemini.gemini.interactions.image.v1beta.image.txt2img.error.not_found
- t1.5.google-gemini.gemini.interactions.image.v1beta.image.txt2img.error.permission
- t1.5.google-gemini.gemini.interactions.image.v1beta.image.txt2img.error.quota
- t1.5.google-gemini.gemini.interactions.image.v1beta.image.txt2img.error.unavailable
- t1.5.google-gemini.gemini.interactions.image.v1beta.image.txt2img.response.malformed_response
- t1.5.google-gemini.gemini.interactions.image.v1beta.image.txt2img.response.missing_required_response_field
- t1.5.google-gemini.gemini.interactions.image.v1beta.image.txt2img.response.wrong_content_type
- t1.5.google-gemini.gemini.interactions.image.v1beta.image.txt2img.success
- t1.5.google-gemini.gemini.interactions.image.v1beta.image.txt2img.ta[REDACTED]
- t1.5.google-gemini.gemini.interactions.music.v1beta.audio.music.error.authentication
- t1.5.google-gemini.gemini.interactions.music.v1beta.audio.music.error.invalid_argument
- t1.5.google-gemini.gemini.interactions.music.v1beta.audio.music.error.not_found
- t1.5.google-gemini.gemini.interactions.music.v1beta.audio.music.error.permission
- t1.5.google-gemini.gemini.interactions.music.v1beta.audio.music.error.quota
- t1.5.google-gemini.gemini.interactions.music.v1beta.audio.music.error.unavailable
- t1.5.google-gemini.gemini.interactions.music.v1beta.audio.music.response.malformed_response
- t1.5.google-gemini.gemini.interactions.music.v1beta.audio.music.response.missing_required_response_field
- t1.5.google-gemini.gemini.interactions.music.v1beta.audio.music.response.wrong_content_type
- t1.5.google-gemini.gemini.interactions.music.v1beta.audio.music.success
- t1.5.google-gemini.gemini.interactions.music.v1beta.audio.music.ta[REDACTED]
- t1.5.google-gemini.gemini.interactions.transcription.v1beta.audio.asr.error.authentication
- t1.5.google-gemini.gemini.interactions.transcription.v1beta.audio.asr.error.invalid_argument
- t1.5.google-gemini.gemini.interactions.transcription.v1beta.audio.asr.error.not_found
- t1.5.google-gemini.gemini.interactions.transcription.v1beta.audio.asr.error.permission
- t1.5.google-gemini.gemini.interactions.transcription.v1beta.audio.asr.error.quota
- t1.5.google-gemini.gemini.interactions.transcription.v1beta.audio.asr.error.unavailable
- t1.5.google-gemini.gemini.interactions.transcription.v1beta.audio.asr.response.malformed_response
- t1.5.google-gemini.gemini.interactions.transcription.v1beta.audio.asr.response.missing_required_response_field
- t1.5.google-gemini.gemini.interactions.transcription.v1beta.audio.asr.response.wrong_content_type
- t1.5.google-gemini.gemini.interactions.transcription.v1beta.audio.asr.success
- t1.5.google-gemini.gemini.interactions.tts.v1beta.audio.tts.error.authentication
- t1.5.google-gemini.gemini.interactions.tts.v1beta.audio.tts.error.invalid_argument
- t1.5.google-gemini.gemini.interactions.tts.v1beta.audio.tts.error.not_found
- t1.5.google-gemini.gemini.interactions.tts.v1beta.audio.tts.error.permission
- t1.5.google-gemini.gemini.interactions.tts.v1beta.audio.tts.error.quota
- t1.5.google-gemini.gemini.interactions.tts.v1beta.audio.tts.error.unavailable
- t1.5.google-gemini.gemini.interactions.tts.v1beta.audio.tts.response.malformed_response
- t1.5.google-gemini.gemini.interactions.tts.v1beta.audio.tts.response.missing_required_response_field
- t1.5.google-gemini.gemini.interactions.tts.v1beta.audio.tts.response.wrong_content_type
- t1.5.google-gemini.gemini.interactions.tts.v1beta.audio.tts.success
- t1.5.google-gemini.gemini.interactions.tts.v1beta.audio.tts.ta[REDACTED]
- t1.5.google-gemini.gemini.interactions.v1beta.llm.error.authentication
- t1.5.google-gemini.gemini.interactions.v1beta.llm.error.invalid_argument
- t1.5.google-gemini.gemini.interactions.v1beta.llm.error.not_found
- t1.5.google-gemini.gemini.interactions.v1beta.llm.error.permission
- t1.5.google-gemini.gemini.interactions.v1beta.llm.error.quota
- t1.5.google-gemini.gemini.interactions.v1beta.llm.error.unavailable
- t1.5.google-gemini.gemini.interactions.v1beta.llm.history
- t1.5.google-gemini.gemini.interactions.v1beta.llm.inline-artifact-provider-state
- t1.5.google-gemini.gemini.interactions.v1beta.llm.response.malformed_response
- t1.5.google-gemini.gemini.interactions.v1beta.llm.response.missing_required_response_field
- t1.5.google-gemini.gemini.interactions.v1beta.llm.response.wrong_content_type
- t1.5.google-gemini.gemini.interactions.v1beta.llm.stream
- t1.5.google-gemini.gemini.interactions.v1beta.llm.stream-interrupted
- t1.5.google-gemini.gemini.interactions.v1beta.llm.success
- t1.5.google-gemini.gemini.interactions.v1beta.llm.tool-history
- t1.5.google-gemini.gemini.interactions.v1beta.vision.caption.success
- t1.5.google-gemini.gemini.interactions.v1beta.vision.detect.success
- t1.5.google-gemini.gemini.interactions.v1beta.vision.ocr.success
- t1.5.google-gemini.gemini.interactions.v1beta.vision.segment.success
- t1.5.google-gemini.gemini.interactions.video.v1beta.video.extend.success
- t1.5.google-gemini.gemini.interactions.video.v1beta.video.extend.ta[REDACTED]
- t1.5.google-gemini.gemini.interactions.video.v1beta.video.video2video.error.authentication
- t1.5.google-gemini.gemini.interactions.video.v1beta.video.video2video.error.invalid_argument
- t1.5.google-gemini.gemini.interactions.video.v1beta.video.video2video.error.not_found
- t1.5.google-gemini.gemini.interactions.video.v1beta.video.video2video.error.permission
- t1.5.google-gemini.gemini.interactions.video.v1beta.video.video2video.error.quota
- t1.5.google-gemini.gemini.interactions.video.v1beta.video.video2video.error.unavailable
- t1.5.google-gemini.gemini.interactions.video.v1beta.video.video2video.response.malformed_response
- t1.5.google-gemini.gemini.interactions.video.v1beta.video.video2video.response.missing_required_response_field
- t1.5.google-gemini.gemini.interactions.video.v1beta.video.video2video.response.wrong_content_type
- t1.5.google-gemini.gemini.interactions.video.v1beta.video.video2video.success
- t1.5.google-gemini.gemini.interactions.video.v1beta.video.video2video.ta[REDACTED]
- t1.5.google-gemini.gemini.predict-long-running.v1beta.video.img2video.success
- t1.5.google-gemini.gemini.predict-long-running.v1beta.video.img2video.ta[REDACTED]
- t1.5.google-gemini.gemini.predict-long-running.v1beta.video.txt2video.async
- t1.5.google-gemini.gemini.predict-long-running.v1beta.video.txt2video.async-failed
- t1.5.google-gemini.gemini.predict-long-running.v1beta.video.txt2video.async_poll_timeout
- t1.5.google-gemini.gemini.predict-long-running.v1beta.video.txt2video.error.authentication
- t1.5.google-gemini.gemini.predict-long-running.v1beta.video.txt2video.error.invalid_argument
- t1.5.google-gemini.gemini.predict-long-running.v1beta.video.txt2video.error.not_found
- t1.5.google-gemini.gemini.predict-long-running.v1beta.video.txt2video.error.permission
- t1.5.google-gemini.gemini.predict-long-running.v1beta.video.txt2video.error.quota
- t1.5.google-gemini.gemini.predict-long-running.v1beta.video.txt2video.error.unavailable
- t1.5.google-gemini.gemini.predict-long-running.v1beta.video.txt2video.response.malformed_response
- t1.5.google-gemini.gemini.predict-long-running.v1beta.video.txt2video.response.missing_required_response_field
- t1.5.google-gemini.gemini.predict-long-running.v1beta.video.txt2video.response.wrong_content_type
- t1.5.google-gemini.gemini.predict-long-running.v1beta.video.txt2video.success
- t1.5.google-gemini.gemini.predict-long-running.v1beta.video.txt2video.ta[REDACTED]

## Confirmed product defects

- **defect.aicc.t1.5.runner** (AICC, provider_protocol_failed): expected AICC maps the official Provider protocol fixture through its real Adapter; observed captured_requests=0; startup: Error: username and password are required without session_token. Evidence: cases/t1.5.runner.json

## Cleanup

Status: passed

- temporary Provider instances were removed and the Mock selection was reset
