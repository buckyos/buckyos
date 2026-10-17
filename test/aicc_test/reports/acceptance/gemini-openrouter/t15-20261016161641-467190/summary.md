# AICC E2E Acceptance Report

- Run: `t15-20261016161641-467190`
- Commit: `6d025991be1a359de271a110191c273a17c0fcfe`
- Capability baseline: `official-provider-protocols-2026-10-15.1`
- Real model calls: 0/0
- Planned maximum cost: $0.000000
- Known actual / unknown estimated exposure: $0.000000 / $0.000000
- Total exposure / budget: $0.000000 / $0.000000
- Unknown-cost calls: 0; budget exceeded: false
- Results: passed=42, failed=0, provider_restricted=0, skipped=0, not_applicable=0, review=0
- Manifest coverage: 42/42 (100.00%); passed=42, failed=0

## Cases

| Case | Layer | Provider | Model | API | Status |
|---|---|---|---|---|---|
| t1.5.google-gemini.gemini.interactions.background.v1beta.llm.success | T1.5 | google-gemini/t15-20261016161641-467190-google-gemini | gemini-4-argon@t15-20261016161641-467190-google-gemini | llm | passed |
| t1.5.google-gemini.gemini.interactions.background.v1beta.llm.async | T1.5 | google-gemini/t15-20261016161641-467190-google-gemini | gemini-4-argon@t15-20261016161641-467190-google-gemini | llm | passed |
| t1.5.google-gemini.gemini.interactions.background.v1beta.llm.async-failed | T1.5 | google-gemini/t15-20261016161641-467190-google-gemini | gemini-4-argon@t15-20261016161641-467190-google-gemini | llm | passed |
| t1.5.google-gemini.gemini.interactions.background.v1beta.llm.async_poll_timeout | T1.5 | google-gemini/t15-20261016161641-467190-google-gemini | gemini-4-argon@t15-20261016161641-467190-google-gemini | llm | passed |
| t1.5.google-gemini.gemini.interactions.background.v1beta.llm.async-cancel | T1.5 | google-gemini/t15-20261016161641-467190-google-gemini | gemini-4-argon@t15-20261016161641-467190-google-gemini | llm | passed |
| t1.5.google-gemini.gemini.interactions.background.v1beta.llm.error.invalid_argument | T1.5 | google-gemini/t15-20261016161641-467190-google-gemini | gemini-4-argon@t15-20261016161641-467190-google-gemini | llm | passed |
| t1.5.google-gemini.gemini.interactions.background.v1beta.llm.error.authentication | T1.5 | google-gemini/t15-20261016161641-467190-google-gemini | gemini-4-argon@t15-20261016161641-467190-google-gemini | llm | passed |
| t1.5.google-gemini.gemini.interactions.background.v1beta.llm.error.permission | T1.5 | google-gemini/t15-20261016161641-467190-google-gemini | gemini-4-argon@t15-20261016161641-467190-google-gemini | llm | passed |
| t1.5.google-gemini.gemini.interactions.background.v1beta.llm.error.not_found | T1.5 | google-gemini/t15-20261016161641-467190-google-gemini | gemini-4-argon@t15-20261016161641-467190-google-gemini | llm | passed |
| t1.5.google-gemini.gemini.interactions.background.v1beta.llm.error.quota | T1.5 | google-gemini/t15-20261016161641-467190-google-gemini-isolation-1 | gemini-4-argon@t15-20261016161641-467190-google-gemini-isolation-1 | llm | passed |
| t1.5.google-gemini.gemini.interactions.background.v1beta.llm.error.unavailable | T1.5 | google-gemini/t15-20261016161641-467190-google-gemini-isolation-1 | gemini-4-argon@t15-20261016161641-467190-google-gemini-isolation-1 | llm | passed |
| t1.5.google-gemini.gemini.interactions.background.v1beta.llm.response.malformed_response | T1.5 | google-gemini/t15-20261016161641-467190-google-gemini-isolation-1 | gemini-4-argon@t15-20261016161641-467190-google-gemini-isolation-1 | llm | passed |
| t1.5.google-gemini.gemini.interactions.background.v1beta.llm.response.wrong_content_type | T1.5 | google-gemini/t15-20261016161641-467190-google-gemini-isolation-1 | gemini-4-argon@t15-20261016161641-467190-google-gemini-isolation-1 | llm | passed |
| t1.5.google-gemini.gemini.interactions.background.v1beta.llm.response.missing_required_response_field | T1.5 | google-gemini/t15-20261016161641-467190-google-gemini-isolation-1 | gemini-4-argon@t15-20261016161641-467190-google-gemini-isolation-1 | llm | passed |
| t1.5.google-gemini.gemini.interactions.background.v1beta.vision.ocr.success | T1.5 | google-gemini/t15-20261016161641-467190-google-gemini-isolation-1 | gemini-4-argon@t15-20261016161641-467190-google-gemini-isolation-1 | vision.ocr | passed |
| t1.5.google-gemini.gemini.interactions.background.v1beta.vision.caption.success | T1.5 | google-gemini/t15-20261016161641-467190-google-gemini-isolation-1 | gemini-4-argon@t15-20261016161641-467190-google-gemini-isolation-1 | vision.caption | passed |
| t1.5.google-gemini.gemini.interactions.background.v1beta.vision.detect.success | T1.5 | google-gemini/t15-20261016161641-467190-google-gemini-isolation-1 | gemini-4-argon@t15-20261016161641-467190-google-gemini-isolation-1 | vision.detect | passed |
| t1.5.google-gemini.gemini.interactions.background.v1beta.vision.segment.success | T1.5 | google-gemini/t15-20261016161641-467190-google-gemini-isolation-1 | gemini-4-argon@t15-20261016161641-467190-google-gemini-isolation-1 | vision.segment | passed |
| t1.5.google-gemini.gemini.interactions.background.v1beta.audio.asr.success | T1.5 | google-gemini/t15-20261016161641-467190-google-gemini-isolation-1 | gemini-4-argon@t15-20261016161641-467190-google-gemini-isolation-1 | audio.asr | passed |
| t1.5.google-gemini.gemini.interactions.transcription.v1beta.audio.asr.success | T1.5 | google-gemini/t15-20261016161641-467190-google-gemini-isolation-1 | gemini-3.5-transcribe@t15-20261016161641-467190-google-gemini-isolation-1 | audio.asr | passed |
| t1.5.google-gemini.gemini.interactions.transcription.v1beta.audio.asr.error.invalid_argument | T1.5 | google-gemini/t15-20261016161641-467190-google-gemini-isolation-1 | gemini-3.5-transcribe@t15-20261016161641-467190-google-gemini-isolation-1 | audio.asr | passed |
| t1.5.google-gemini.gemini.interactions.transcription.v1beta.audio.asr.error.authentication | T1.5 | google-gemini/t15-20261016161641-467190-google-gemini-isolation-1 | gemini-3.5-transcribe@t15-20261016161641-467190-google-gemini-isolation-1 | audio.asr | passed |
| t1.5.google-gemini.gemini.interactions.transcription.v1beta.audio.asr.error.permission | T1.5 | google-gemini/t15-20261016161641-467190-google-gemini-isolation-1 | gemini-3.5-transcribe@t15-20261016161641-467190-google-gemini-isolation-1 | audio.asr | passed |
| t1.5.google-gemini.gemini.interactions.transcription.v1beta.audio.asr.error.not_found | T1.5 | google-gemini/t15-20261016161641-467190-google-gemini-isolation-1 | gemini-3.5-transcribe@t15-20261016161641-467190-google-gemini-isolation-1 | audio.asr | passed |
| t1.5.google-gemini.gemini.interactions.transcription.v1beta.audio.asr.error.quota | T1.5 | google-gemini/t15-20261016161641-467190-google-gemini-isolation-2 | gemini-3.5-transcribe@t15-20261016161641-467190-google-gemini-isolation-2 | audio.asr | passed |
| t1.5.google-gemini.gemini.interactions.transcription.v1beta.audio.asr.error.unavailable | T1.5 | google-gemini/t15-20261016161641-467190-google-gemini-isolation-2 | gemini-3.5-transcribe@t15-20261016161641-467190-google-gemini-isolation-2 | audio.asr | passed |
| t1.5.google-gemini.gemini.interactions.transcription.v1beta.audio.asr.response.malformed_response | T1.5 | google-gemini/t15-20261016161641-467190-google-gemini-isolation-2 | gemini-3.5-transcribe@t15-20261016161641-467190-google-gemini-isolation-2 | audio.asr | passed |
| t1.5.google-gemini.gemini.interactions.transcription.v1beta.audio.asr.response.wrong_content_type | T1.5 | google-gemini/t15-20261016161641-467190-google-gemini-isolation-2 | gemini-3.5-transcribe@t15-20261016161641-467190-google-gemini-isolation-2 | audio.asr | passed |
| t1.5.google-gemini.gemini.interactions.transcription.v1beta.audio.asr.response.missing_required_response_field | T1.5 | google-gemini/t15-20261016161641-467190-google-gemini-isolation-2 | gemini-3.5-transcribe@t15-20261016161641-467190-google-gemini-isolation-2 | audio.asr | passed |
| t1.5.google-gemini.gemini.interactions.background.v1beta.llm.history | T1.5 | google-gemini/t15-20261016161641-467190-google-gemini-isolation-2 | gemini-4-argon@t15-20261016161641-467190-google-gemini-isolation-2 | llm | passed |
| t1.5.google-gemini.gemini.interactions.background.v1beta.llm.tool-history | T1.5 | google-gemini/t15-20261016161641-467190-google-gemini-isolation-2 | gemini-4-argon@t15-20261016161641-467190-google-gemini-isolation-2 | llm | passed |
| t1.5.openrouter.openrouter.responses.v1.llm.model.google-gemini-4-argon.success | T1.5 | openrouter/t15-20261016161641-467190-openrouter | google/gemini-4-argon@t15-20261016161641-467190-openrouter | llm | passed |
| t1.5.openrouter.openrouter.responses.v1.vision.ocr.model.google-gemini-4-argon.success | T1.5 | openrouter/t15-20261016161641-467190-openrouter | google/gemini-4-argon@t15-20261016161641-467190-openrouter | vision.ocr | passed |
| t1.5.openrouter.openrouter.responses.v1.vision.caption.model.google-gemini-4-argon.success | T1.5 | openrouter/t15-20261016161641-467190-openrouter | google/gemini-4-argon@t15-20261016161641-467190-openrouter | vision.caption | passed |
| t1.5.openrouter.openrouter.audio.transcriptions.v1.audio.asr.model.google-gemini-3.5-transcribe.success | T1.5 | openrouter/t15-20261016161641-467190-openrouter | google/gemini-3.5-transcribe@t15-20261016161641-467190-openrouter | audio.asr | passed |
| t1.5.custom.google-gemini.gemini.interactions.background.v1beta.llm.success | T1.5 | google-gemini/t15-20261016161641-467190-custom-google-gemini | gemini-4-argon@t15-20261016161641-467190-custom-google-gemini | llm | passed |
| t1.5.custom.google-gemini.gemini.interactions.background.v1beta.vision.ocr.success | T1.5 | google-gemini/t15-20261016161641-467190-custom-google-gemini | gemini-4-argon@t15-20261016161641-467190-custom-google-gemini | vision.ocr | passed |
| t1.5.custom.google-gemini.gemini.interactions.background.v1beta.vision.caption.success | T1.5 | google-gemini/t15-20261016161641-467190-custom-google-gemini | gemini-4-argon@t15-20261016161641-467190-custom-google-gemini | vision.caption | passed |
| t1.5.custom.google-gemini.gemini.interactions.background.v1beta.vision.detect.success | T1.5 | google-gemini/t15-20261016161641-467190-custom-google-gemini | gemini-4-argon@t15-20261016161641-467190-custom-google-gemini | vision.detect | passed |
| t1.5.custom.google-gemini.gemini.interactions.background.v1beta.vision.segment.success | T1.5 | google-gemini/t15-20261016161641-467190-custom-google-gemini | gemini-4-argon@t15-20261016161641-467190-custom-google-gemini | vision.segment | passed |
| t1.5.custom.google-gemini.gemini.interactions.background.v1beta.audio.asr.success | T1.5 | google-gemini/t15-20261016161641-467190-custom-google-gemini | gemini-4-argon@t15-20261016161641-467190-custom-google-gemini | audio.asr | passed |
| t1.5.custom.google-gemini.gemini.interactions.transcription.v1beta.audio.asr.success | T1.5 | google-gemini/t15-20261016161641-467190-custom-google-gemini | gemini-3.5-transcribe@t15-20261016161641-467190-custom-google-gemini | audio.asr | passed |

## Confirmed product defects

None.

## Cleanup

Status: passed

- temporary Provider instances were removed and the Mock selection was reset
