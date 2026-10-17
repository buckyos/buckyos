# AICC E2E Acceptance Report

- Run: `t15-20261014131325-525898`
- Commit: `e742bc1cf5be6b716ef849e624d5b381a495adb6`
- Capability baseline: `official-provider-protocols-2026-10-14.1`
- Real model calls: 0/0
- Planned maximum cost: $0.000000
- Known actual / unknown estimated exposure: $0.000000 / $0.000000
- Total exposure / budget: $0.000000 / $0.000000
- Unknown-cost calls: 0; budget exceeded: false
- Results: passed=73, failed=3, provider_restricted=0, skipped=0, not_applicable=0, review=0
- Manifest coverage: 75/414 (18.12%); passed=73, failed=2

## Targeted retest

Run after fixing the reported defect; repeat `--case` to select additional cases:

```bash
AICC_T15_ALLOW_CONFIG_MUTATION=true pnpm run acceptance:t1.5 -- --gateway-url "http://127.0.0.1:3180" --mock-base-url "http://127.0.0.1:18081" --mock-control-url "http://127.0.0.1:18081" --allow-config-mutation --case t1.5.google-gemini.gemini.batch-embed-contents.v1beta.embedding.text.success --case t1.5.google-gemini.gemini.batch-embed-contents.v1beta.embedding.text.error.invalid_argument
```

## Cases

| Case | Layer | Provider | Model | API | Status |
|---|---|---|---|---|---|
| t1.5.google-gemini.gemini.interactions.v1beta.llm.success | T1.5 | google-gemini/t15-20261014131325-525898-google-gemini | gemini-3.5-flash@t15-20261014131325-525898-google-gemini | llm | passed |
| t1.5.google-gemini.gemini.interactions.v1beta.llm.inline-artifact-provider-state | T1.5 | google-gemini/t15-20261014131325-525898-google-gemini | gemini-3.5-flash@t15-20261014131325-525898-google-gemini | llm | passed |
| t1.5.google-gemini.gemini.interactions.v1beta.llm.stream | T1.5 | google-gemini/t15-20261014131325-525898-google-gemini | gemini-3.5-flash@t15-20261014131325-525898-google-gemini | llm | passed |
| t1.5.google-gemini.gemini.interactions.v1beta.llm.stream-interrupted | T1.5 | google-gemini/t15-20261014131325-525898-google-gemini | gemini-3.5-flash@t15-20261014131325-525898-google-gemini | llm | passed |
| t1.5.google-gemini.gemini.interactions.v1beta.llm.error.invalid_argument | T1.5 | google-gemini/t15-20261014131325-525898-google-gemini | gemini-3.5-flash@t15-20261014131325-525898-google-gemini | llm | passed |
| t1.5.google-gemini.gemini.interactions.v1beta.llm.error.authentication | T1.5 | google-gemini/t15-20261014131325-525898-google-gemini | gemini-3.5-flash@t15-20261014131325-525898-google-gemini | llm | passed |
| t1.5.google-gemini.gemini.interactions.v1beta.llm.error.permission | T1.5 | google-gemini/t15-20261014131325-525898-google-gemini | gemini-3.5-flash@t15-20261014131325-525898-google-gemini | llm | passed |
| t1.5.google-gemini.gemini.interactions.v1beta.llm.error.not_found | T1.5 | google-gemini/t15-20261014131325-525898-google-gemini | gemini-3.5-flash@t15-20261014131325-525898-google-gemini | llm | passed |
| t1.5.google-gemini.gemini.interactions.v1beta.llm.error.quota | T1.5 | google-gemini/t15-20261014131325-525898-google-gemini-isolation-1 | gemini-3.5-flash@t15-20261014131325-525898-google-gemini-isolation-1 | llm | passed |
| t1.5.google-gemini.gemini.interactions.v1beta.llm.error.unavailable | T1.5 | google-gemini/t15-20261014131325-525898-google-gemini-isolation-1 | gemini-3.5-flash@t15-20261014131325-525898-google-gemini-isolation-1 | llm | passed |
| t1.5.google-gemini.gemini.interactions.v1beta.llm.response.malformed_response | T1.5 | google-gemini/t15-20261014131325-525898-google-gemini-isolation-1 | gemini-3.5-flash@t15-20261014131325-525898-google-gemini-isolation-1 | llm | passed |
| t1.5.google-gemini.gemini.interactions.v1beta.llm.response.wrong_content_type | T1.5 | google-gemini/t15-20261014131325-525898-google-gemini-isolation-1 | gemini-3.5-flash@t15-20261014131325-525898-google-gemini-isolation-1 | llm | passed |
| t1.5.google-gemini.gemini.interactions.v1beta.llm.response.missing_required_response_field | T1.5 | google-gemini/t15-20261014131325-525898-google-gemini-isolation-1 | gemini-3.5-flash@t15-20261014131325-525898-google-gemini-isolation-1 | llm | passed |
| t1.5.google-gemini.gemini.interactions.v1beta.vision.ocr.success | T1.5 | google-gemini/t15-20261014131325-525898-google-gemini-isolation-1 | gemini-3.5-flash@t15-20261014131325-525898-google-gemini-isolation-1 | vision.ocr | passed |
| t1.5.google-gemini.gemini.interactions.v1beta.vision.caption.success | T1.5 | google-gemini/t15-20261014131325-525898-google-gemini-isolation-1 | gemini-3.5-flash@t15-20261014131325-525898-google-gemini-isolation-1 | vision.caption | passed |
| t1.5.google-gemini.gemini.interactions.v1beta.vision.detect.success | T1.5 | google-gemini/t15-20261014131325-525898-google-gemini-isolation-1 | gemini-3.5-flash@t15-20261014131325-525898-google-gemini-isolation-1 | vision.detect | passed |
| t1.5.google-gemini.gemini.interactions.v1beta.vision.segment.success | T1.5 | google-gemini/t15-20261014131325-525898-google-gemini-isolation-1 | gemini-3.5-flash@t15-20261014131325-525898-google-gemini-isolation-1 | vision.segment | passed |
| t1.5.google-gemini.gemini.interactions.transcription.v1beta.audio.asr.success | T1.5 | google-gemini/t15-20261014131325-525898-google-gemini-isolation-1 | gemini-3.5-transcribe@t15-20261014131325-525898-google-gemini-isolation-1 | audio.asr | passed |
| t1.5.google-gemini.gemini.interactions.transcription.v1beta.audio.asr.error.invalid_argument | T1.5 | google-gemini/t15-20261014131325-525898-google-gemini-isolation-1 | gemini-3.5-transcribe@t15-20261014131325-525898-google-gemini-isolation-1 | audio.asr | passed |
| t1.5.google-gemini.gemini.interactions.transcription.v1beta.audio.asr.error.authentication | T1.5 | google-gemini/t15-20261014131325-525898-google-gemini-isolation-1 | gemini-3.5-transcribe@t15-20261014131325-525898-google-gemini-isolation-1 | audio.asr | passed |
| t1.5.google-gemini.gemini.interactions.transcription.v1beta.audio.asr.error.permission | T1.5 | google-gemini/t15-20261014131325-525898-google-gemini-isolation-1 | gemini-3.5-transcribe@t15-20261014131325-525898-google-gemini-isolation-1 | audio.asr | passed |
| t1.5.google-gemini.gemini.interactions.transcription.v1beta.audio.asr.error.not_found | T1.5 | google-gemini/t15-20261014131325-525898-google-gemini-isolation-1 | gemini-3.5-transcribe@t15-20261014131325-525898-google-gemini-isolation-1 | audio.asr | passed |
| t1.5.google-gemini.gemini.interactions.transcription.v1beta.audio.asr.error.quota | T1.5 | google-gemini/t15-20261014131325-525898-google-gemini-isolation-2 | gemini-3.5-transcribe@t15-20261014131325-525898-google-gemini-isolation-2 | audio.asr | passed |
| t1.5.google-gemini.gemini.interactions.transcription.v1beta.audio.asr.error.unavailable | T1.5 | google-gemini/t15-20261014131325-525898-google-gemini-isolation-2 | gemini-3.5-transcribe@t15-20261014131325-525898-google-gemini-isolation-2 | audio.asr | passed |
| t1.5.google-gemini.gemini.interactions.transcription.v1beta.audio.asr.response.malformed_response | T1.5 | google-gemini/t15-20261014131325-525898-google-gemini-isolation-2 | gemini-3.5-transcribe@t15-20261014131325-525898-google-gemini-isolation-2 | audio.asr | passed |
| t1.5.google-gemini.gemini.interactions.transcription.v1beta.audio.asr.response.wrong_content_type | T1.5 | google-gemini/t15-20261014131325-525898-google-gemini-isolation-2 | gemini-3.5-transcribe@t15-20261014131325-525898-google-gemini-isolation-2 | audio.asr | passed |
| t1.5.google-gemini.gemini.interactions.transcription.v1beta.audio.asr.response.missing_required_response_field | T1.5 | google-gemini/t15-20261014131325-525898-google-gemini-isolation-2 | gemini-3.5-transcribe@t15-20261014131325-525898-google-gemini-isolation-2 | audio.asr | passed |
| t1.5.google-gemini.gemini.interactions.image.v1beta.image.txt2img.success | T1.5 | google-gemini/t15-20261014131325-525898-google-gemini-isolation-2 | gemini-3.1-flash-image@t15-20261014131325-525898-google-gemini-isolation-2 | image.txt2img | passed |
| t1.5.google-gemini.gemini.interactions.image.v1beta.image.txt2img.ta[REDACTED] | T1.5 | google-gemini/t15-20261014131325-525898-google-gemini-isolation-2 | gemini-3.1-flash-image@t15-20261014131325-525898-google-gemini-isolation-2 | image.txt2img | passed |
| t1.5.google-gemini.gemini.interactions.image.v1beta.image.txt2img.error.invalid_argument | T1.5 | google-gemini/t15-20261014131325-525898-google-gemini-isolation-2 | gemini-3.1-flash-image@t15-20261014131325-525898-google-gemini-isolation-2 | image.txt2img | passed |
| t1.5.google-gemini.gemini.interactions.image.v1beta.image.txt2img.error.authentication | T1.5 | google-gemini/t15-20261014131325-525898-google-gemini-isolation-2 | gemini-3.1-flash-image@t15-20261014131325-525898-google-gemini-isolation-2 | image.txt2img | passed |
| t1.5.google-gemini.gemini.interactions.image.v1beta.image.txt2img.error.permission | T1.5 | google-gemini/t15-20261014131325-525898-google-gemini-isolation-2 | gemini-3.1-flash-image@t15-20261014131325-525898-google-gemini-isolation-2 | image.txt2img | passed |
| t1.5.google-gemini.gemini.interactions.image.v1beta.image.txt2img.error.not_found | T1.5 | google-gemini/t15-20261014131325-525898-google-gemini-isolation-2 | gemini-3.1-flash-image@t15-20261014131325-525898-google-gemini-isolation-2 | image.txt2img | passed |
| t1.5.google-gemini.gemini.interactions.image.v1beta.image.txt2img.error.quota | T1.5 | google-gemini/t15-20261014131325-525898-google-gemini-isolation-3 | gemini-3.1-flash-image@t15-20261014131325-525898-google-gemini-isolation-3 | image.txt2img | passed |
| t1.5.google-gemini.gemini.interactions.image.v1beta.image.txt2img.error.unavailable | T1.5 | google-gemini/t15-20261014131325-525898-google-gemini-isolation-3 | gemini-3.1-flash-image@t15-20261014131325-525898-google-gemini-isolation-3 | image.txt2img | passed |
| t1.5.google-gemini.gemini.interactions.image.v1beta.image.txt2img.response.malformed_response | T1.5 | google-gemini/t15-20261014131325-525898-google-gemini-isolation-3 | gemini-3.1-flash-image@t15-20261014131325-525898-google-gemini-isolation-3 | image.txt2img | passed |
| t1.5.google-gemini.gemini.interactions.image.v1beta.image.txt2img.response.wrong_content_type | T1.5 | google-gemini/t15-20261014131325-525898-google-gemini-isolation-3 | gemini-3.1-flash-image@t15-20261014131325-525898-google-gemini-isolation-3 | image.txt2img | passed |
| t1.5.google-gemini.gemini.interactions.image.v1beta.image.txt2img.response.missing_required_response_field | T1.5 | google-gemini/t15-20261014131325-525898-google-gemini-isolation-3 | gemini-3.1-flash-image@t15-20261014131325-525898-google-gemini-isolation-3 | image.txt2img | passed |
| t1.5.google-gemini.gemini.interactions.image.v1beta.image.img2img.success | T1.5 | google-gemini/t15-20261014131325-525898-google-gemini-isolation-3 | gemini-3.1-flash-image@t15-20261014131325-525898-google-gemini-isolation-3 | image.img2img | passed |
| t1.5.google-gemini.gemini.interactions.image.v1beta.image.img2img.ta[REDACTED] | T1.5 | google-gemini/t15-20261014131325-525898-google-gemini-isolation-3 | gemini-3.1-flash-image@t15-20261014131325-525898-google-gemini-isolation-3 | image.img2img | passed |
| t1.5.google-gemini.gemini.interactions.image.v1beta.image.img2img.generated-artifact-id.same-provider | T1.5 | google-gemini/t15-20261014131325-525898-google-gemini-isolation-3 | gemini-3.1-flash-image@t15-20261014131325-525898-google-gemini-isolation-3 | image.img2img | passed |
| t1.5.google-gemini.gemini.interactions.tts.v1beta.audio.tts.success | T1.5 | google-gemini/t15-20261014131325-525898-google-gemini-isolation-3 | gemini-3.1-flash-tts-preview@t15-20261014131325-525898-google-gemini-isolation-3 | audio.tts | passed |
| t1.5.google-gemini.gemini.interactions.tts.v1beta.audio.tts.ta[REDACTED] | T1.5 | google-gemini/t15-20261014131325-525898-google-gemini-isolation-3 | gemini-3.1-flash-tts-preview@t15-20261014131325-525898-google-gemini-isolation-3 | audio.tts | passed |
| t1.5.google-gemini.gemini.interactions.tts.v1beta.audio.tts.error.invalid_argument | T1.5 | google-gemini/t15-20261014131325-525898-google-gemini-isolation-3 | gemini-3.1-flash-tts-preview@t15-20261014131325-525898-google-gemini-isolation-3 | audio.tts | passed |
| t1.5.google-gemini.gemini.interactions.tts.v1beta.audio.tts.error.authentication | T1.5 | google-gemini/t15-20261014131325-525898-google-gemini-isolation-3 | gemini-3.1-flash-tts-preview@t15-20261014131325-525898-google-gemini-isolation-3 | audio.tts | passed |
| t1.5.google-gemini.gemini.interactions.tts.v1beta.audio.tts.error.permission | T1.5 | google-gemini/t15-20261014131325-525898-google-gemini-isolation-3 | gemini-3.1-flash-tts-preview@t15-20261014131325-525898-google-gemini-isolation-3 | audio.tts | passed |
| t1.5.google-gemini.gemini.interactions.tts.v1beta.audio.tts.error.not_found | T1.5 | google-gemini/t15-20261014131325-525898-google-gemini-isolation-3 | gemini-3.1-flash-tts-preview@t15-20261014131325-525898-google-gemini-isolation-3 | audio.tts | passed |
| t1.5.google-gemini.gemini.interactions.tts.v1beta.audio.tts.error.quota | T1.5 | google-gemini/t15-20261014131325-525898-google-gemini-isolation-4 | gemini-3.1-flash-tts-preview@t15-20261014131325-525898-google-gemini-isolation-4 | audio.tts | passed |
| t1.5.google-gemini.gemini.interactions.tts.v1beta.audio.tts.error.unavailable | T1.5 | google-gemini/t15-20261014131325-525898-google-gemini-isolation-4 | gemini-3.1-flash-tts-preview@t15-20261014131325-525898-google-gemini-isolation-4 | audio.tts | passed |
| t1.5.google-gemini.gemini.interactions.tts.v1beta.audio.tts.response.malformed_response | T1.5 | google-gemini/t15-20261014131325-525898-google-gemini-isolation-4 | gemini-3.1-flash-tts-preview@t15-20261014131325-525898-google-gemini-isolation-4 | audio.tts | passed |
| t1.5.google-gemini.gemini.interactions.tts.v1beta.audio.tts.response.wrong_content_type | T1.5 | google-gemini/t15-20261014131325-525898-google-gemini-isolation-4 | gemini-3.1-flash-tts-preview@t15-20261014131325-525898-google-gemini-isolation-4 | audio.tts | passed |
| t1.5.google-gemini.gemini.interactions.tts.v1beta.audio.tts.response.missing_required_response_field | T1.5 | google-gemini/t15-20261014131325-525898-google-gemini-isolation-4 | gemini-3.1-flash-tts-preview@t15-20261014131325-525898-google-gemini-isolation-4 | audio.tts | passed |
| t1.5.google-gemini.gemini.interactions.music.v1beta.audio.music.success | T1.5 | google-gemini/t15-20261014131325-525898-google-gemini-isolation-4 | lyria-3-clip-preview@t15-20261014131325-525898-google-gemini-isolation-4 | audio.music | passed |
| t1.5.google-gemini.gemini.interactions.music.v1beta.audio.music.ta[REDACTED] | T1.5 | google-gemini/t15-20261014131325-525898-google-gemini-isolation-4 | lyria-3-clip-preview@t15-20261014131325-525898-google-gemini-isolation-4 | audio.music | passed |
| t1.5.google-gemini.gemini.interactions.music.v1beta.audio.music.error.invalid_argument | T1.5 | google-gemini/t15-20261014131325-525898-google-gemini-isolation-4 | lyria-3-clip-preview@t15-20261014131325-525898-google-gemini-isolation-4 | audio.music | passed |
| t1.5.google-gemini.gemini.interactions.music.v1beta.audio.music.error.authentication | T1.5 | google-gemini/t15-20261014131325-525898-google-gemini-isolation-4 | lyria-3-clip-preview@t15-20261014131325-525898-google-gemini-isolation-4 | audio.music | passed |
| t1.5.google-gemini.gemini.interactions.music.v1beta.audio.music.error.permission | T1.5 | google-gemini/t15-20261014131325-525898-google-gemini-isolation-4 | lyria-3-clip-preview@t15-20261014131325-525898-google-gemini-isolation-4 | audio.music | passed |
| t1.5.google-gemini.gemini.interactions.music.v1beta.audio.music.error.not_found | T1.5 | google-gemini/t15-20261014131325-525898-google-gemini-isolation-4 | lyria-3-clip-preview@t15-20261014131325-525898-google-gemini-isolation-4 | audio.music | passed |
| t1.5.google-gemini.gemini.interactions.music.v1beta.audio.music.error.quota | T1.5 | google-gemini/t15-20261014131325-525898-google-gemini-isolation-5 | lyria-3-clip-preview@t15-20261014131325-525898-google-gemini-isolation-5 | audio.music | passed |
| t1.5.google-gemini.gemini.interactions.music.v1beta.audio.music.error.unavailable | T1.5 | google-gemini/t15-20261014131325-525898-google-gemini-isolation-5 | lyria-3-clip-preview@t15-20261014131325-525898-google-gemini-isolation-5 | audio.music | passed |
| t1.5.google-gemini.gemini.interactions.music.v1beta.audio.music.response.malformed_response | T1.5 | google-gemini/t15-20261014131325-525898-google-gemini-isolation-5 | lyria-3-clip-preview@t15-20261014131325-525898-google-gemini-isolation-5 | audio.music | passed |
| t1.5.google-gemini.gemini.interactions.music.v1beta.audio.music.response.wrong_content_type | T1.5 | google-gemini/t15-20261014131325-525898-google-gemini-isolation-5 | lyria-3-clip-preview@t15-20261014131325-525898-google-gemini-isolation-5 | audio.music | passed |
| t1.5.google-gemini.gemini.interactions.music.v1beta.audio.music.response.missing_required_response_field | T1.5 | google-gemini/t15-20261014131325-525898-google-gemini-isolation-5 | lyria-3-clip-preview@t15-20261014131325-525898-google-gemini-isolation-5 | audio.music | passed |
| t1.5.google-gemini.gemini.embed-content.v1beta.embedding.multimodal.success | T1.5 | google-gemini/t15-20261014131325-525898-google-gemini-isolation-5 | gemini-embedding-2@t15-20261014131325-525898-google-gemini-isolation-5 | embedding.multimodal | passed |
| t1.5.google-gemini.gemini.embed-content.v1beta.embedding.multimodal.error.invalid_argument | T1.5 | google-gemini/t15-20261014131325-525898-google-gemini-isolation-5 | gemini-embedding-2@t15-20261014131325-525898-google-gemini-isolation-5 | embedding.multimodal | passed |
| t1.5.google-gemini.gemini.embed-content.v1beta.embedding.multimodal.error.authentication | T1.5 | google-gemini/t15-20261014131325-525898-google-gemini-isolation-5 | gemini-embedding-2@t15-20261014131325-525898-google-gemini-isolation-5 | embedding.multimodal | passed |
| t1.5.google-gemini.gemini.embed-content.v1beta.embedding.multimodal.error.permission | T1.5 | google-gemini/t15-20261014131325-525898-google-gemini-isolation-5 | gemini-embedding-2@t15-20261014131325-525898-google-gemini-isolation-5 | embedding.multimodal | passed |
| t1.5.google-gemini.gemini.embed-content.v1beta.embedding.multimodal.error.not_found | T1.5 | google-gemini/t15-20261014131325-525898-google-gemini-isolation-5 | gemini-embedding-2@t15-20261014131325-525898-google-gemini-isolation-5 | embedding.multimodal | passed |
| t1.5.google-gemini.gemini.embed-content.v1beta.embedding.multimodal.error.quota | T1.5 | google-gemini/t15-20261014131325-525898-google-gemini-isolation-6 | gemini-embedding-2@t15-20261014131325-525898-google-gemini-isolation-6 | embedding.multimodal | passed |
| t1.5.google-gemini.gemini.embed-content.v1beta.embedding.multimodal.error.unavailable | T1.5 | google-gemini/t15-20261014131325-525898-google-gemini-isolation-6 | gemini-embedding-2@t15-20261014131325-525898-google-gemini-isolation-6 | embedding.multimodal | passed |
| t1.5.google-gemini.gemini.embed-content.v1beta.embedding.multimodal.response.malformed_response | T1.5 | google-gemini/t15-20261014131325-525898-google-gemini-isolation-6 | gemini-embedding-2@t15-20261014131325-525898-google-gemini-isolation-6 | embedding.multimodal | passed |
| t1.5.google-gemini.gemini.embed-content.v1beta.embedding.multimodal.response.wrong_content_type | T1.5 | google-gemini/t15-20261014131325-525898-google-gemini-isolation-6 | gemini-embedding-2@t15-20261014131325-525898-google-gemini-isolation-6 | embedding.multimodal | passed |
| t1.5.google-gemini.gemini.embed-content.v1beta.embedding.multimodal.response.missing_required_response_field | T1.5 | google-gemini/t15-20261014131325-525898-google-gemini-isolation-6 | gemini-embedding-2@t15-20261014131325-525898-google-gemini-isolation-6 | embedding.multimodal | passed |
| t1.5.google-gemini.gemini.batch-embed-contents.v1beta.embedding.text.success | T1.5 | google-gemini/t15-20261014131325-525898-google-gemini-isolation-6 | gemini-embedding-2@t15-20261014131325-525898-google-gemini-isolation-6 | embedding.text | failed |
| t1.5.google-gemini.gemini.batch-embed-contents.v1beta.embedding.text.error.invalid_argument | T1.5 | google-gemini/t15-20261014131325-525898-google-gemini-isolation-6 | gemini-embedding-2@t15-20261014131325-525898-google-gemini-isolation-6 | embedding.text | failed |
| t1.5.runner | T1.5 | -/- | - | provider.add/models.list | failed |

## Unexecuted manifest cases

- t1.5.custom.google-gemini.gemini.batch-embed-contents.v1beta.embedding.text.success
- t1.5.custom.google-gemini.gemini.embed-content.v1beta.embedding.multimodal.success
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
- t1.5.google-gemini.gemini.batch-embed-contents.v1beta.embedding.text.error.authentication
- t1.5.google-gemini.gemini.batch-embed-contents.v1beta.embedding.text.error.not_found
- t1.5.google-gemini.gemini.batch-embed-contents.v1beta.embedding.text.error.permission
- t1.5.google-gemini.gemini.batch-embed-contents.v1beta.embedding.text.error.quota
- t1.5.google-gemini.gemini.batch-embed-contents.v1beta.embedding.text.error.unavailable
- t1.5.google-gemini.gemini.batch-embed-contents.v1beta.embedding.text.response.malformed_response
- t1.5.google-gemini.gemini.batch-embed-contents.v1beta.embedding.text.response.missing_required_response_field
- t1.5.google-gemini.gemini.batch-embed-contents.v1beta.embedding.text.response.wrong_content_type
- t1.5.google-gemini.gemini.interactions.transcription.v1beta.audio.asr.variant.gemini-2.5-flash-lite-reasoning-high
- t1.5.google-gemini.gemini.interactions.transcription.v1beta.audio.asr.variant.gemini-2.5-flash-lite-reasoning-low
- t1.5.google-gemini.gemini.interactions.transcription.v1beta.audio.asr.variant.gemini-2.5-flash-lite-reasoning-medium
- t1.5.google-gemini.gemini.interactions.transcription.v1beta.audio.asr.variant.gemini-2.5-flash-reasoning-high
- t1.5.google-gemini.gemini.interactions.transcription.v1beta.audio.asr.variant.gemini-2.5-flash-reasoning-low
- t1.5.google-gemini.gemini.interactions.transcription.v1beta.audio.asr.variant.gemini-2.5-flash-reasoning-medium
- t1.5.google-gemini.gemini.interactions.transcription.v1beta.audio.asr.variant.gemini-2.5-pro-reasoning-high
- t1.5.google-gemini.gemini.interactions.transcription.v1beta.audio.asr.variant.gemini-2.5-pro-reasoning-low
- t1.5.google-gemini.gemini.interactions.transcription.v1beta.audio.asr.variant.gemini-2.5-pro-reasoning-medium
- t1.5.google-gemini.gemini.interactions.transcription.v1beta.audio.asr.variant.gemini-3-flash-preview-reasoning-high
- t1.5.google-gemini.gemini.interactions.transcription.v1beta.audio.asr.variant.gemini-3-flash-preview-reasoning-low
- t1.5.google-gemini.gemini.interactions.transcription.v1beta.audio.asr.variant.gemini-3-flash-preview-reasoning-medium
- t1.5.google-gemini.gemini.interactions.transcription.v1beta.audio.asr.variant.gemini-3-flash-preview-reasoning-minimal
- t1.5.google-gemini.gemini.interactions.transcription.v1beta.audio.asr.variant.gemini-3.1-flash-lite-reasoning-high
- t1.5.google-gemini.gemini.interactions.transcription.v1beta.audio.asr.variant.gemini-3.1-flash-lite-reasoning-low
- t1.5.google-gemini.gemini.interactions.transcription.v1beta.audio.asr.variant.gemini-3.1-flash-lite-reasoning-medium
- t1.5.google-gemini.gemini.interactions.transcription.v1beta.audio.asr.variant.gemini-3.1-flash-lite-reasoning-minimal
- t1.5.google-gemini.gemini.interactions.transcription.v1beta.audio.asr.variant.gemini-3.1-pro-preview-customtools-reasoning-high
- t1.5.google-gemini.gemini.interactions.transcription.v1beta.audio.asr.variant.gemini-3.1-pro-preview-customtools-reasoning-low
- t1.5.google-gemini.gemini.interactions.transcription.v1beta.audio.asr.variant.gemini-3.1-pro-preview-customtools-reasoning-medium
- t1.5.google-gemini.gemini.interactions.transcription.v1beta.audio.asr.variant.gemini-3.1-pro-preview-reasoning-high
- t1.5.google-gemini.gemini.interactions.transcription.v1beta.audio.asr.variant.gemini-3.1-pro-preview-reasoning-low
- t1.5.google-gemini.gemini.interactions.transcription.v1beta.audio.asr.variant.gemini-3.1-pro-preview-reasoning-medium
- t1.5.google-gemini.gemini.interactions.transcription.v1beta.audio.asr.variant.gemini-3.5-flash-lite-reasoning-high
- t1.5.google-gemini.gemini.interactions.transcription.v1beta.audio.asr.variant.gemini-3.5-flash-lite-reasoning-low
- t1.5.google-gemini.gemini.interactions.transcription.v1beta.audio.asr.variant.gemini-3.5-flash-lite-reasoning-medium
- t1.5.google-gemini.gemini.interactions.transcription.v1beta.audio.asr.variant.gemini-3.5-flash-lite-reasoning-minimal
- t1.5.google-gemini.gemini.interactions.transcription.v1beta.audio.asr.variant.gemini-3.5-flash-reasoning-high
- t1.5.google-gemini.gemini.interactions.transcription.v1beta.audio.asr.variant.gemini-3.5-flash-reasoning-low
- t1.5.google-gemini.gemini.interactions.transcription.v1beta.audio.asr.variant.gemini-3.5-flash-reasoning-medium
- t1.5.google-gemini.gemini.interactions.transcription.v1beta.audio.asr.variant.gemini-3.5-flash-reasoning-minimal
- t1.5.google-gemini.gemini.interactions.transcription.v1beta.audio.asr.variant.gemini-3.6-flash-reasoning-high
- t1.5.google-gemini.gemini.interactions.transcription.v1beta.audio.asr.variant.gemini-3.6-flash-reasoning-low
- t1.5.google-gemini.gemini.interactions.transcription.v1beta.audio.asr.variant.gemini-3.6-flash-reasoning-medium
- t1.5.google-gemini.gemini.interactions.transcription.v1beta.audio.asr.variant.gemini-3.6-flash-reasoning-minimal
- t1.5.google-gemini.gemini.interactions.transcription.v1beta.audio.asr.variant.gemini-3.7-flash-reasoning-high
- t1.5.google-gemini.gemini.interactions.transcription.v1beta.audio.asr.variant.gemini-3.7-flash-reasoning-low
- t1.5.google-gemini.gemini.interactions.transcription.v1beta.audio.asr.variant.gemini-3.7-flash-reasoning-medium
- t1.5.google-gemini.gemini.interactions.transcription.v1beta.audio.asr.variant.gemini-3.8-flash-reasoning-high
- t1.5.google-gemini.gemini.interactions.transcription.v1beta.audio.asr.variant.gemini-3.8-flash-reasoning-low
- t1.5.google-gemini.gemini.interactions.transcription.v1beta.audio.asr.variant.gemini-3.8-flash-reasoning-medium
- t1.5.google-gemini.gemini.interactions.transcription.v1beta.audio.asr.variant.gemini-4-argon-reasoning-high
- t1.5.google-gemini.gemini.interactions.transcription.v1beta.audio.asr.variant.gemini-4-argon-reasoning-low
- t1.5.google-gemini.gemini.interactions.transcription.v1beta.audio.asr.variant.gemini-4-argon-reasoning-medium
- t1.5.google-gemini.gemini.interactions.v1beta.llm.history
- t1.5.google-gemini.gemini.interactions.v1beta.llm.tool-history
- t1.5.google-gemini.gemini.interactions.v1beta.llm.variant.gemini-2.5-flash-lite-reasoning-high
- t1.5.google-gemini.gemini.interactions.v1beta.llm.variant.gemini-2.5-flash-lite-reasoning-low
- t1.5.google-gemini.gemini.interactions.v1beta.llm.variant.gemini-2.5-flash-lite-reasoning-medium
- t1.5.google-gemini.gemini.interactions.v1beta.llm.variant.gemini-2.5-flash-reasoning-high
- t1.5.google-gemini.gemini.interactions.v1beta.llm.variant.gemini-2.5-flash-reasoning-low
- t1.5.google-gemini.gemini.interactions.v1beta.llm.variant.gemini-2.5-flash-reasoning-medium
- t1.5.google-gemini.gemini.interactions.v1beta.llm.variant.gemini-2.5-pro-reasoning-high
- t1.5.google-gemini.gemini.interactions.v1beta.llm.variant.gemini-2.5-pro-reasoning-low
- t1.5.google-gemini.gemini.interactions.v1beta.llm.variant.gemini-2.5-pro-reasoning-medium
- t1.5.google-gemini.gemini.interactions.v1beta.llm.variant.gemini-3-flash-preview-reasoning-high
- t1.5.google-gemini.gemini.interactions.v1beta.llm.variant.gemini-3-flash-preview-reasoning-low
- t1.5.google-gemini.gemini.interactions.v1beta.llm.variant.gemini-3-flash-preview-reasoning-medium
- t1.5.google-gemini.gemini.interactions.v1beta.llm.variant.gemini-3-flash-preview-reasoning-minimal
- t1.5.google-gemini.gemini.interactions.v1beta.llm.variant.gemini-3.1-flash-lite-reasoning-high
- t1.5.google-gemini.gemini.interactions.v1beta.llm.variant.gemini-3.1-flash-lite-reasoning-low
- t1.5.google-gemini.gemini.interactions.v1beta.llm.variant.gemini-3.1-flash-lite-reasoning-medium
- t1.5.google-gemini.gemini.interactions.v1beta.llm.variant.gemini-3.1-flash-lite-reasoning-minimal
- t1.5.google-gemini.gemini.interactions.v1beta.llm.variant.gemini-3.1-pro-preview-customtools-reasoning-high
- t1.5.google-gemini.gemini.interactions.v1beta.llm.variant.gemini-3.1-pro-preview-customtools-reasoning-low
- t1.5.google-gemini.gemini.interactions.v1beta.llm.variant.gemini-3.1-pro-preview-customtools-reasoning-medium
- t1.5.google-gemini.gemini.interactions.v1beta.llm.variant.gemini-3.1-pro-preview-reasoning-high
- t1.5.google-gemini.gemini.interactions.v1beta.llm.variant.gemini-3.1-pro-preview-reasoning-low
- t1.5.google-gemini.gemini.interactions.v1beta.llm.variant.gemini-3.1-pro-preview-reasoning-medium
- t1.5.google-gemini.gemini.interactions.v1beta.llm.variant.gemini-3.5-flash-lite-reasoning-high
- t1.5.google-gemini.gemini.interactions.v1beta.llm.variant.gemini-3.5-flash-lite-reasoning-low
- t1.5.google-gemini.gemini.interactions.v1beta.llm.variant.gemini-3.5-flash-lite-reasoning-medium
- t1.5.google-gemini.gemini.interactions.v1beta.llm.variant.gemini-3.5-flash-lite-reasoning-minimal
- t1.5.google-gemini.gemini.interactions.v1beta.llm.variant.gemini-3.5-flash-reasoning-high
- t1.5.google-gemini.gemini.interactions.v1beta.llm.variant.gemini-3.5-flash-reasoning-low
- t1.5.google-gemini.gemini.interactions.v1beta.llm.variant.gemini-3.5-flash-reasoning-medium
- t1.5.google-gemini.gemini.interactions.v1beta.llm.variant.gemini-3.5-flash-reasoning-minimal
- t1.5.google-gemini.gemini.interactions.v1beta.llm.variant.gemini-3.6-flash-reasoning-high
- t1.5.google-gemini.gemini.interactions.v1beta.llm.variant.gemini-3.6-flash-reasoning-low
- t1.5.google-gemini.gemini.interactions.v1beta.llm.variant.gemini-3.6-flash-reasoning-medium
- t1.5.google-gemini.gemini.interactions.v1beta.llm.variant.gemini-3.6-flash-reasoning-minimal
- t1.5.google-gemini.gemini.interactions.v1beta.llm.variant.gemini-3.7-flash-reasoning-high
- t1.5.google-gemini.gemini.interactions.v1beta.llm.variant.gemini-3.7-flash-reasoning-low
- t1.5.google-gemini.gemini.interactions.v1beta.llm.variant.gemini-3.7-flash-reasoning-medium
- t1.5.google-gemini.gemini.interactions.v1beta.llm.variant.gemini-3.8-flash-reasoning-high
- t1.5.google-gemini.gemini.interactions.v1beta.llm.variant.gemini-3.8-flash-reasoning-low
- t1.5.google-gemini.gemini.interactions.v1beta.llm.variant.gemini-3.8-flash-reasoning-medium
- t1.5.google-gemini.gemini.interactions.v1beta.llm.variant.gemini-4-argon-reasoning-high
- t1.5.google-gemini.gemini.interactions.v1beta.llm.variant.gemini-4-argon-reasoning-low
- t1.5.google-gemini.gemini.interactions.v1beta.llm.variant.gemini-4-argon-reasoning-medium
- t1.5.google-gemini.gemini.interactions.v1beta.llm.variant.gemini-robotics-er-2-preview-reasoning-high
- t1.5.google-gemini.gemini.interactions.v1beta.llm.variant.gemini-robotics-er-2-preview-reasoning-low
- t1.5.google-gemini.gemini.interactions.v1beta.llm.variant.gemini-robotics-er-2-preview-reasoning-medium
- t1.5.google-gemini.gemini.interactions.v1beta.llm.variant.gemini-robotics-er-2-preview-reasoning-minimal
- t1.5.google-gemini.gemini.interactions.v1beta.vision.caption.variant.gemini-2.5-flash-lite-reasoning-high
- t1.5.google-gemini.gemini.interactions.v1beta.vision.caption.variant.gemini-2.5-flash-lite-reasoning-low
- t1.5.google-gemini.gemini.interactions.v1beta.vision.caption.variant.gemini-2.5-flash-lite-reasoning-medium
- t1.5.google-gemini.gemini.interactions.v1beta.vision.caption.variant.gemini-2.5-flash-reasoning-high
- t1.5.google-gemini.gemini.interactions.v1beta.vision.caption.variant.gemini-2.5-flash-reasoning-low
- t1.5.google-gemini.gemini.interactions.v1beta.vision.caption.variant.gemini-2.5-flash-reasoning-medium
- t1.5.google-gemini.gemini.interactions.v1beta.vision.caption.variant.gemini-2.5-pro-reasoning-high
- t1.5.google-gemini.gemini.interactions.v1beta.vision.caption.variant.gemini-2.5-pro-reasoning-low
- t1.5.google-gemini.gemini.interactions.v1beta.vision.caption.variant.gemini-2.5-pro-reasoning-medium
- t1.5.google-gemini.gemini.interactions.v1beta.vision.caption.variant.gemini-3-flash-preview-reasoning-high
- t1.5.google-gemini.gemini.interactions.v1beta.vision.caption.variant.gemini-3-flash-preview-reasoning-low
- t1.5.google-gemini.gemini.interactions.v1beta.vision.caption.variant.gemini-3-flash-preview-reasoning-medium
- t1.5.google-gemini.gemini.interactions.v1beta.vision.caption.variant.gemini-3-flash-preview-reasoning-minimal
- t1.5.google-gemini.gemini.interactions.v1beta.vision.caption.variant.gemini-3.1-flash-lite-reasoning-high
- t1.5.google-gemini.gemini.interactions.v1beta.vision.caption.variant.gemini-3.1-flash-lite-reasoning-low
- t1.5.google-gemini.gemini.interactions.v1beta.vision.caption.variant.gemini-3.1-flash-lite-reasoning-medium
- t1.5.google-gemini.gemini.interactions.v1beta.vision.caption.variant.gemini-3.1-flash-lite-reasoning-minimal
- t1.5.google-gemini.gemini.interactions.v1beta.vision.caption.variant.gemini-3.1-pro-preview-customtools-reasoning-high
- t1.5.google-gemini.gemini.interactions.v1beta.vision.caption.variant.gemini-3.1-pro-preview-customtools-reasoning-low
- t1.5.google-gemini.gemini.interactions.v1beta.vision.caption.variant.gemini-3.1-pro-preview-customtools-reasoning-medium
- t1.5.google-gemini.gemini.interactions.v1beta.vision.caption.variant.gemini-3.1-pro-preview-reasoning-high
- t1.5.google-gemini.gemini.interactions.v1beta.vision.caption.variant.gemini-3.1-pro-preview-reasoning-low
- t1.5.google-gemini.gemini.interactions.v1beta.vision.caption.variant.gemini-3.1-pro-preview-reasoning-medium
- t1.5.google-gemini.gemini.interactions.v1beta.vision.caption.variant.gemini-3.5-flash-lite-reasoning-high
- t1.5.google-gemini.gemini.interactions.v1beta.vision.caption.variant.gemini-3.5-flash-lite-reasoning-low
- t1.5.google-gemini.gemini.interactions.v1beta.vision.caption.variant.gemini-3.5-flash-lite-reasoning-medium
- t1.5.google-gemini.gemini.interactions.v1beta.vision.caption.variant.gemini-3.5-flash-lite-reasoning-minimal
- t1.5.google-gemini.gemini.interactions.v1beta.vision.caption.variant.gemini-3.5-flash-reasoning-high
- t1.5.google-gemini.gemini.interactions.v1beta.vision.caption.variant.gemini-3.5-flash-reasoning-low
- t1.5.google-gemini.gemini.interactions.v1beta.vision.caption.variant.gemini-3.5-flash-reasoning-medium
- t1.5.google-gemini.gemini.interactions.v1beta.vision.caption.variant.gemini-3.5-flash-reasoning-minimal
- t1.5.google-gemini.gemini.interactions.v1beta.vision.caption.variant.gemini-3.6-flash-reasoning-high
- t1.5.google-gemini.gemini.interactions.v1beta.vision.caption.variant.gemini-3.6-flash-reasoning-low
- t1.5.google-gemini.gemini.interactions.v1beta.vision.caption.variant.gemini-3.6-flash-reasoning-medium
- t1.5.google-gemini.gemini.interactions.v1beta.vision.caption.variant.gemini-3.6-flash-reasoning-minimal
- t1.5.google-gemini.gemini.interactions.v1beta.vision.caption.variant.gemini-3.7-flash-reasoning-high
- t1.5.google-gemini.gemini.interactions.v1beta.vision.caption.variant.gemini-3.7-flash-reasoning-low
- t1.5.google-gemini.gemini.interactions.v1beta.vision.caption.variant.gemini-3.7-flash-reasoning-medium
- t1.5.google-gemini.gemini.interactions.v1beta.vision.caption.variant.gemini-3.8-flash-reasoning-high
- t1.5.google-gemini.gemini.interactions.v1beta.vision.caption.variant.gemini-3.8-flash-reasoning-low
- t1.5.google-gemini.gemini.interactions.v1beta.vision.caption.variant.gemini-3.8-flash-reasoning-medium
- t1.5.google-gemini.gemini.interactions.v1beta.vision.caption.variant.gemini-4-argon-reasoning-high
- t1.5.google-gemini.gemini.interactions.v1beta.vision.caption.variant.gemini-4-argon-reasoning-low
- t1.5.google-gemini.gemini.interactions.v1beta.vision.caption.variant.gemini-4-argon-reasoning-medium
- t1.5.google-gemini.gemini.interactions.v1beta.vision.caption.variant.gemini-robotics-er-2-preview-reasoning-high
- t1.5.google-gemini.gemini.interactions.v1beta.vision.caption.variant.gemini-robotics-er-2-preview-reasoning-low
- t1.5.google-gemini.gemini.interactions.v1beta.vision.caption.variant.gemini-robotics-er-2-preview-reasoning-medium
- t1.5.google-gemini.gemini.interactions.v1beta.vision.caption.variant.gemini-robotics-er-2-preview-reasoning-minimal
- t1.5.google-gemini.gemini.interactions.v1beta.vision.detect.variant.gemini-2.5-flash-lite-reasoning-high
- t1.5.google-gemini.gemini.interactions.v1beta.vision.detect.variant.gemini-2.5-flash-lite-reasoning-low
- t1.5.google-gemini.gemini.interactions.v1beta.vision.detect.variant.gemini-2.5-flash-lite-reasoning-medium
- t1.5.google-gemini.gemini.interactions.v1beta.vision.detect.variant.gemini-2.5-flash-reasoning-high
- t1.5.google-gemini.gemini.interactions.v1beta.vision.detect.variant.gemini-2.5-flash-reasoning-low
- t1.5.google-gemini.gemini.interactions.v1beta.vision.detect.variant.gemini-2.5-flash-reasoning-medium
- t1.5.google-gemini.gemini.interactions.v1beta.vision.detect.variant.gemini-2.5-pro-reasoning-high
- t1.5.google-gemini.gemini.interactions.v1beta.vision.detect.variant.gemini-2.5-pro-reasoning-low
- t1.5.google-gemini.gemini.interactions.v1beta.vision.detect.variant.gemini-2.5-pro-reasoning-medium
- t1.5.google-gemini.gemini.interactions.v1beta.vision.detect.variant.gemini-3-flash-preview-reasoning-high
- t1.5.google-gemini.gemini.interactions.v1beta.vision.detect.variant.gemini-3-flash-preview-reasoning-low
- t1.5.google-gemini.gemini.interactions.v1beta.vision.detect.variant.gemini-3-flash-preview-reasoning-medium
- t1.5.google-gemini.gemini.interactions.v1beta.vision.detect.variant.gemini-3-flash-preview-reasoning-minimal
- t1.5.google-gemini.gemini.interactions.v1beta.vision.detect.variant.gemini-3.1-flash-lite-reasoning-high
- t1.5.google-gemini.gemini.interactions.v1beta.vision.detect.variant.gemini-3.1-flash-lite-reasoning-low
- t1.5.google-gemini.gemini.interactions.v1beta.vision.detect.variant.gemini-3.1-flash-lite-reasoning-medium
- t1.5.google-gemini.gemini.interactions.v1beta.vision.detect.variant.gemini-3.1-flash-lite-reasoning-minimal
- t1.5.google-gemini.gemini.interactions.v1beta.vision.detect.variant.gemini-3.1-pro-preview-customtools-reasoning-high
- t1.5.google-gemini.gemini.interactions.v1beta.vision.detect.variant.gemini-3.1-pro-preview-customtools-reasoning-low
- t1.5.google-gemini.gemini.interactions.v1beta.vision.detect.variant.gemini-3.1-pro-preview-customtools-reasoning-medium
- t1.5.google-gemini.gemini.interactions.v1beta.vision.detect.variant.gemini-3.1-pro-preview-reasoning-high
- t1.5.google-gemini.gemini.interactions.v1beta.vision.detect.variant.gemini-3.1-pro-preview-reasoning-low
- t1.5.google-gemini.gemini.interactions.v1beta.vision.detect.variant.gemini-3.1-pro-preview-reasoning-medium
- t1.5.google-gemini.gemini.interactions.v1beta.vision.detect.variant.gemini-3.5-flash-lite-reasoning-high
- t1.5.google-gemini.gemini.interactions.v1beta.vision.detect.variant.gemini-3.5-flash-lite-reasoning-low
- t1.5.google-gemini.gemini.interactions.v1beta.vision.detect.variant.gemini-3.5-flash-lite-reasoning-medium
- t1.5.google-gemini.gemini.interactions.v1beta.vision.detect.variant.gemini-3.5-flash-lite-reasoning-minimal
- t1.5.google-gemini.gemini.interactions.v1beta.vision.detect.variant.gemini-3.5-flash-reasoning-high
- t1.5.google-gemini.gemini.interactions.v1beta.vision.detect.variant.gemini-3.5-flash-reasoning-low
- t1.5.google-gemini.gemini.interactions.v1beta.vision.detect.variant.gemini-3.5-flash-reasoning-medium
- t1.5.google-gemini.gemini.interactions.v1beta.vision.detect.variant.gemini-3.5-flash-reasoning-minimal
- t1.5.google-gemini.gemini.interactions.v1beta.vision.detect.variant.gemini-3.6-flash-reasoning-high
- t1.5.google-gemini.gemini.interactions.v1beta.vision.detect.variant.gemini-3.6-flash-reasoning-low
- t1.5.google-gemini.gemini.interactions.v1beta.vision.detect.variant.gemini-3.6-flash-reasoning-medium
- t1.5.google-gemini.gemini.interactions.v1beta.vision.detect.variant.gemini-3.6-flash-reasoning-minimal
- t1.5.google-gemini.gemini.interactions.v1beta.vision.detect.variant.gemini-3.7-flash-reasoning-high
- t1.5.google-gemini.gemini.interactions.v1beta.vision.detect.variant.gemini-3.7-flash-reasoning-low
- t1.5.google-gemini.gemini.interactions.v1beta.vision.detect.variant.gemini-3.7-flash-reasoning-medium
- t1.5.google-gemini.gemini.interactions.v1beta.vision.detect.variant.gemini-3.8-flash-reasoning-high
- t1.5.google-gemini.gemini.interactions.v1beta.vision.detect.variant.gemini-3.8-flash-reasoning-low
- t1.5.google-gemini.gemini.interactions.v1beta.vision.detect.variant.gemini-3.8-flash-reasoning-medium
- t1.5.google-gemini.gemini.interactions.v1beta.vision.detect.variant.gemini-4-argon-reasoning-high
- t1.5.google-gemini.gemini.interactions.v1beta.vision.detect.variant.gemini-4-argon-reasoning-low
- t1.5.google-gemini.gemini.interactions.v1beta.vision.detect.variant.gemini-4-argon-reasoning-medium
- t1.5.google-gemini.gemini.interactions.v1beta.vision.detect.variant.gemini-robotics-er-2-preview-reasoning-high
- t1.5.google-gemini.gemini.interactions.v1beta.vision.detect.variant.gemini-robotics-er-2-preview-reasoning-low
- t1.5.google-gemini.gemini.interactions.v1beta.vision.detect.variant.gemini-robotics-er-2-preview-reasoning-medium
- t1.5.google-gemini.gemini.interactions.v1beta.vision.detect.variant.gemini-robotics-er-2-preview-reasoning-minimal
- t1.5.google-gemini.gemini.interactions.v1beta.vision.ocr.variant.gemini-2.5-flash-lite-reasoning-high
- t1.5.google-gemini.gemini.interactions.v1beta.vision.ocr.variant.gemini-2.5-flash-lite-reasoning-low
- t1.5.google-gemini.gemini.interactions.v1beta.vision.ocr.variant.gemini-2.5-flash-lite-reasoning-medium
- t1.5.google-gemini.gemini.interactions.v1beta.vision.ocr.variant.gemini-2.5-flash-reasoning-high
- t1.5.google-gemini.gemini.interactions.v1beta.vision.ocr.variant.gemini-2.5-flash-reasoning-low
- t1.5.google-gemini.gemini.interactions.v1beta.vision.ocr.variant.gemini-2.5-flash-reasoning-medium
- t1.5.google-gemini.gemini.interactions.v1beta.vision.ocr.variant.gemini-2.5-pro-reasoning-high
- t1.5.google-gemini.gemini.interactions.v1beta.vision.ocr.variant.gemini-2.5-pro-reasoning-low
- t1.5.google-gemini.gemini.interactions.v1beta.vision.ocr.variant.gemini-2.5-pro-reasoning-medium
- t1.5.google-gemini.gemini.interactions.v1beta.vision.ocr.variant.gemini-3-flash-preview-reasoning-high
- t1.5.google-gemini.gemini.interactions.v1beta.vision.ocr.variant.gemini-3-flash-preview-reasoning-low
- t1.5.google-gemini.gemini.interactions.v1beta.vision.ocr.variant.gemini-3-flash-preview-reasoning-medium
- t1.5.google-gemini.gemini.interactions.v1beta.vision.ocr.variant.gemini-3-flash-preview-reasoning-minimal
- t1.5.google-gemini.gemini.interactions.v1beta.vision.ocr.variant.gemini-3.1-flash-lite-reasoning-high
- t1.5.google-gemini.gemini.interactions.v1beta.vision.ocr.variant.gemini-3.1-flash-lite-reasoning-low
- t1.5.google-gemini.gemini.interactions.v1beta.vision.ocr.variant.gemini-3.1-flash-lite-reasoning-medium
- t1.5.google-gemini.gemini.interactions.v1beta.vision.ocr.variant.gemini-3.1-flash-lite-reasoning-minimal
- t1.5.google-gemini.gemini.interactions.v1beta.vision.ocr.variant.gemini-3.1-pro-preview-customtools-reasoning-high
- t1.5.google-gemini.gemini.interactions.v1beta.vision.ocr.variant.gemini-3.1-pro-preview-customtools-reasoning-low
- t1.5.google-gemini.gemini.interactions.v1beta.vision.ocr.variant.gemini-3.1-pro-preview-customtools-reasoning-medium
- t1.5.google-gemini.gemini.interactions.v1beta.vision.ocr.variant.gemini-3.1-pro-preview-reasoning-high
- t1.5.google-gemini.gemini.interactions.v1beta.vision.ocr.variant.gemini-3.1-pro-preview-reasoning-low
- t1.5.google-gemini.gemini.interactions.v1beta.vision.ocr.variant.gemini-3.1-pro-preview-reasoning-medium
- t1.5.google-gemini.gemini.interactions.v1beta.vision.ocr.variant.gemini-3.5-flash-lite-reasoning-high
- t1.5.google-gemini.gemini.interactions.v1beta.vision.ocr.variant.gemini-3.5-flash-lite-reasoning-low
- t1.5.google-gemini.gemini.interactions.v1beta.vision.ocr.variant.gemini-3.5-flash-lite-reasoning-medium
- t1.5.google-gemini.gemini.interactions.v1beta.vision.ocr.variant.gemini-3.5-flash-lite-reasoning-minimal
- t1.5.google-gemini.gemini.interactions.v1beta.vision.ocr.variant.gemini-3.5-flash-reasoning-high
- t1.5.google-gemini.gemini.interactions.v1beta.vision.ocr.variant.gemini-3.5-flash-reasoning-low
- t1.5.google-gemini.gemini.interactions.v1beta.vision.ocr.variant.gemini-3.5-flash-reasoning-medium
- t1.5.google-gemini.gemini.interactions.v1beta.vision.ocr.variant.gemini-3.5-flash-reasoning-minimal
- t1.5.google-gemini.gemini.interactions.v1beta.vision.ocr.variant.gemini-3.6-flash-reasoning-high
- t1.5.google-gemini.gemini.interactions.v1beta.vision.ocr.variant.gemini-3.6-flash-reasoning-low
- t1.5.google-gemini.gemini.interactions.v1beta.vision.ocr.variant.gemini-3.6-flash-reasoning-medium
- t1.5.google-gemini.gemini.interactions.v1beta.vision.ocr.variant.gemini-3.6-flash-reasoning-minimal
- t1.5.google-gemini.gemini.interactions.v1beta.vision.ocr.variant.gemini-3.7-flash-reasoning-high
- t1.5.google-gemini.gemini.interactions.v1beta.vision.ocr.variant.gemini-3.7-flash-reasoning-low
- t1.5.google-gemini.gemini.interactions.v1beta.vision.ocr.variant.gemini-3.7-flash-reasoning-medium
- t1.5.google-gemini.gemini.interactions.v1beta.vision.ocr.variant.gemini-3.8-flash-reasoning-high
- t1.5.google-gemini.gemini.interactions.v1beta.vision.ocr.variant.gemini-3.8-flash-reasoning-low
- t1.5.google-gemini.gemini.interactions.v1beta.vision.ocr.variant.gemini-3.8-flash-reasoning-medium
- t1.5.google-gemini.gemini.interactions.v1beta.vision.ocr.variant.gemini-4-argon-reasoning-high
- t1.5.google-gemini.gemini.interactions.v1beta.vision.ocr.variant.gemini-4-argon-reasoning-low
- t1.5.google-gemini.gemini.interactions.v1beta.vision.ocr.variant.gemini-4-argon-reasoning-medium
- t1.5.google-gemini.gemini.interactions.v1beta.vision.ocr.variant.gemini-robotics-er-2-preview-reasoning-high
- t1.5.google-gemini.gemini.interactions.v1beta.vision.ocr.variant.gemini-robotics-er-2-preview-reasoning-low
- t1.5.google-gemini.gemini.interactions.v1beta.vision.ocr.variant.gemini-robotics-er-2-preview-reasoning-medium
- t1.5.google-gemini.gemini.interactions.v1beta.vision.ocr.variant.gemini-robotics-er-2-preview-reasoning-minimal
- t1.5.google-gemini.gemini.interactions.v1beta.vision.segment.variant.gemini-2.5-flash-lite-reasoning-high
- t1.5.google-gemini.gemini.interactions.v1beta.vision.segment.variant.gemini-2.5-flash-lite-reasoning-low
- t1.5.google-gemini.gemini.interactions.v1beta.vision.segment.variant.gemini-2.5-flash-lite-reasoning-medium
- t1.5.google-gemini.gemini.interactions.v1beta.vision.segment.variant.gemini-2.5-flash-reasoning-high
- t1.5.google-gemini.gemini.interactions.v1beta.vision.segment.variant.gemini-2.5-flash-reasoning-low
- t1.5.google-gemini.gemini.interactions.v1beta.vision.segment.variant.gemini-2.5-flash-reasoning-medium
- t1.5.google-gemini.gemini.interactions.v1beta.vision.segment.variant.gemini-2.5-pro-reasoning-high
- t1.5.google-gemini.gemini.interactions.v1beta.vision.segment.variant.gemini-2.5-pro-reasoning-low
- t1.5.google-gemini.gemini.interactions.v1beta.vision.segment.variant.gemini-2.5-pro-reasoning-medium
- t1.5.google-gemini.gemini.interactions.v1beta.vision.segment.variant.gemini-3-flash-preview-reasoning-high
- t1.5.google-gemini.gemini.interactions.v1beta.vision.segment.variant.gemini-3-flash-preview-reasoning-low
- t1.5.google-gemini.gemini.interactions.v1beta.vision.segment.variant.gemini-3-flash-preview-reasoning-medium
- t1.5.google-gemini.gemini.interactions.v1beta.vision.segment.variant.gemini-3-flash-preview-reasoning-minimal
- t1.5.google-gemini.gemini.interactions.v1beta.vision.segment.variant.gemini-3.1-flash-lite-reasoning-high
- t1.5.google-gemini.gemini.interactions.v1beta.vision.segment.variant.gemini-3.1-flash-lite-reasoning-low
- t1.5.google-gemini.gemini.interactions.v1beta.vision.segment.variant.gemini-3.1-flash-lite-reasoning-medium
- t1.5.google-gemini.gemini.interactions.v1beta.vision.segment.variant.gemini-3.1-flash-lite-reasoning-minimal
- t1.5.google-gemini.gemini.interactions.v1beta.vision.segment.variant.gemini-3.1-pro-preview-customtools-reasoning-high
- t1.5.google-gemini.gemini.interactions.v1beta.vision.segment.variant.gemini-3.1-pro-preview-customtools-reasoning-low
- t1.5.google-gemini.gemini.interactions.v1beta.vision.segment.variant.gemini-3.1-pro-preview-customtools-reasoning-medium
- t1.5.google-gemini.gemini.interactions.v1beta.vision.segment.variant.gemini-3.1-pro-preview-reasoning-high
- t1.5.google-gemini.gemini.interactions.v1beta.vision.segment.variant.gemini-3.1-pro-preview-reasoning-low
- t1.5.google-gemini.gemini.interactions.v1beta.vision.segment.variant.gemini-3.1-pro-preview-reasoning-medium
- t1.5.google-gemini.gemini.interactions.v1beta.vision.segment.variant.gemini-3.5-flash-lite-reasoning-high
- t1.5.google-gemini.gemini.interactions.v1beta.vision.segment.variant.gemini-3.5-flash-lite-reasoning-low
- t1.5.google-gemini.gemini.interactions.v1beta.vision.segment.variant.gemini-3.5-flash-lite-reasoning-medium
- t1.5.google-gemini.gemini.interactions.v1beta.vision.segment.variant.gemini-3.5-flash-lite-reasoning-minimal
- t1.5.google-gemini.gemini.interactions.v1beta.vision.segment.variant.gemini-3.5-flash-reasoning-high
- t1.5.google-gemini.gemini.interactions.v1beta.vision.segment.variant.gemini-3.5-flash-reasoning-low
- t1.5.google-gemini.gemini.interactions.v1beta.vision.segment.variant.gemini-3.5-flash-reasoning-medium
- t1.5.google-gemini.gemini.interactions.v1beta.vision.segment.variant.gemini-3.5-flash-reasoning-minimal
- t1.5.google-gemini.gemini.interactions.v1beta.vision.segment.variant.gemini-3.6-flash-reasoning-high
- t1.5.google-gemini.gemini.interactions.v1beta.vision.segment.variant.gemini-3.6-flash-reasoning-low
- t1.5.google-gemini.gemini.interactions.v1beta.vision.segment.variant.gemini-3.6-flash-reasoning-medium
- t1.5.google-gemini.gemini.interactions.v1beta.vision.segment.variant.gemini-3.6-flash-reasoning-minimal
- t1.5.google-gemini.gemini.interactions.v1beta.vision.segment.variant.gemini-3.7-flash-reasoning-high
- t1.5.google-gemini.gemini.interactions.v1beta.vision.segment.variant.gemini-3.7-flash-reasoning-low
- t1.5.google-gemini.gemini.interactions.v1beta.vision.segment.variant.gemini-3.7-flash-reasoning-medium
- t1.5.google-gemini.gemini.interactions.v1beta.vision.segment.variant.gemini-3.8-flash-reasoning-high
- t1.5.google-gemini.gemini.interactions.v1beta.vision.segment.variant.gemini-3.8-flash-reasoning-low
- t1.5.google-gemini.gemini.interactions.v1beta.vision.segment.variant.gemini-3.8-flash-reasoning-medium
- t1.5.google-gemini.gemini.interactions.v1beta.vision.segment.variant.gemini-4-argon-reasoning-high
- t1.5.google-gemini.gemini.interactions.v1beta.vision.segment.variant.gemini-4-argon-reasoning-low
- t1.5.google-gemini.gemini.interactions.v1beta.vision.segment.variant.gemini-4-argon-reasoning-medium
- t1.5.google-gemini.gemini.interactions.v1beta.vision.segment.variant.gemini-robotics-er-2-preview-reasoning-high
- t1.5.google-gemini.gemini.interactions.v1beta.vision.segment.variant.gemini-robotics-er-2-preview-reasoning-low
- t1.5.google-gemini.gemini.interactions.v1beta.vision.segment.variant.gemini-robotics-er-2-preview-reasoning-medium
- t1.5.google-gemini.gemini.interactions.v1beta.vision.segment.variant.gemini-robotics-er-2-preview-reasoning-minimal
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

- **defect.aicc.t1.5.google-gemini.gemini.batch-embed-contents.v1beta.embedding.text.success** (AICC, provider_protocol_failed): expected AICC maps the official Provider protocol fixture through its real Adapter; observed contract=gemini.batch-embed-contents.v1beta; scenario=success; captured_requests=1; wire contract violations: ["path=/v1beta/models/gemini-embedding-2:embedContent; expected=/v1beta/models/{model}:batchEmbedContents","missing body field requests","unknown body field content","Gemini batch embeddings require multiple model-qualified EmbedContentRequest entries"]; Error: AICC returned failed: {"error":{"code":"provider_error","details":{"request_id":"aicc-18de67a894d4adc3-13a","retry_after_ms":null},"message":"Gemini http_error: Gemini request failed","provider_code":"400"},"event_ref":"/task_mgr/t-296b8da94cf0431f8c180c0b84db23f1","route_trace":{"attempts":[],"final_model":"gemini-embedding-2@t15-20261014131325-525898-google-gemini-isolation-6"},"status":"failed","task_id":"t-296b8da94cf0431f8c180c0b84db23f1"}. Evidence: cases/t1.5.google-gemini.gemini.batch-embed-contents.v1beta.embedding.text.success.json
- **defect.aicc.t1.5.google-gemini.gemini.batch-embed-contents.v1beta.embedding.text.error.invalid_argument** (AICC, provider_protocol_failed): expected AICC maps the official Provider protocol fixture through its real Adapter; observed contract=gemini.batch-embed-contents.v1beta; scenario=invalid_argument; captured_requests=1; wire contract violations: ["path=/v1beta/models/gemini-embedding-2:embedContent; expected=/v1beta/models/{model}:batchEmbedContents","missing body field requests","unknown body field content","Gemini batch embeddings require multiple model-qualified EmbedContentRequest entries"]. Evidence: cases/t1.5.google-gemini.gemini.batch-embed-contents.v1beta.embedding.text.error.invalid_argument.json
- **defect.aicc.t1.5.runner** (AICC, provider_protocol_failed): expected AICC maps the official Provider protocol fixture through its real Adapter; observed captured_requests=0; provider:google-gemini:cases: Error: health recovery failed after t1.5.google-gemini.gemini.batch-embed-contents.v1beta.embedding.text.error.invalid_argument: wire contract violations: ["path=/v1beta/models/gemini-embedding-2:embedContent; expected=/v1beta/models/{model}:batchEmbedContents","missing body field requests","unknown body field content","Gemini batch embeddings require multiple model-qualified EmbedContentRequest entries"]; Error: AICC returned failed: {"error":{"code":"provider_error","details":{"request_id":"aicc-18de67a8b6b8569b-13c","retry_after_ms":null},"message":"Gemini http_error: Gemini request failed","provider_code":"400"},"event_ref":"/task_mgr/t-992cbc609f6b465fbb9d6ff502b50d9a","route_trace":{"attempts":[],"final_model":"gemini-embedding-2@t15-20261014131325-525898-google-gemini-isolation-6"},"status":"failed","task_id":"t-992cbc609f6b465fbb9d6ff502b50d9a"}. Evidence: cases/t1.5.runner.json

## Cleanup

Status: passed

- temporary Provider instances were removed and the Mock selection was reset
