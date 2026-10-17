# AICC E2E Acceptance Report

- Run: `t15-20261016160803-453890`
- Commit: `6d025991be1a359de271a110191c273a17c0fcfe`
- Capability baseline: `official-provider-protocols-2026-10-15.1`
- Real model calls: 0/0
- Planned maximum cost: $0.000000
- Known actual / unknown estimated exposure: $0.000000 / $0.000000
- Total exposure / budget: $0.000000 / $0.000000
- Unknown-cost calls: 0; budget exceeded: false
- Results: passed=0, failed=1, provider_restricted=0, skipped=0, not_applicable=0, review=0
- Manifest coverage: 0/43 (0.00%); passed=0, failed=0

## Targeted retest

Run after fixing the reported defect; repeat `--case` to select additional cases:

```bash
AICC_T15_ALLOW_CONFIG_MUTATION=true pnpm run acceptance:t1.5 -- --gateway-url "http://127.0.0.1:3180" --mock-base-url "http://127.0.0.1:18080" --mock-control-url "http://127.0.0.1:18080" --allow-config-mutation
```

## Cases

| Case | Layer | Provider | Model | API | Status |
|---|---|---|---|---|---|
| t1.5.runner | T1.5 | -/- | - | provider.add/models.list | failed |

## Unexecuted manifest cases

- t1.5.custom.google-gemini.gemini.interactions.background.v1beta.audio.asr.success
- t1.5.custom.google-gemini.gemini.interactions.background.v1beta.llm.success
- t1.5.custom.google-gemini.gemini.interactions.background.v1beta.vision.caption.success
- t1.5.custom.google-gemini.gemini.interactions.background.v1beta.vision.detect.success
- t1.5.custom.google-gemini.gemini.interactions.background.v1beta.vision.ocr.success
- t1.5.custom.google-gemini.gemini.interactions.background.v1beta.vision.segment.success
- t1.5.custom.google-gemini.gemini.interactions.transcription.v1beta.audio.asr.success
- t1.5.google-gemini.gemini.interactions.background.v1beta.audio.asr.success
- t1.5.google-gemini.gemini.interactions.background.v1beta.llm.async
- t1.5.google-gemini.gemini.interactions.background.v1beta.llm.async-cancel
- t1.5.google-gemini.gemini.interactions.background.v1beta.llm.async-failed
- t1.5.google-gemini.gemini.interactions.background.v1beta.llm.async_poll_timeout
- t1.5.google-gemini.gemini.interactions.background.v1beta.llm.error.authentication
- t1.5.google-gemini.gemini.interactions.background.v1beta.llm.error.invalid_argument
- t1.5.google-gemini.gemini.interactions.background.v1beta.llm.error.not_found
- t1.5.google-gemini.gemini.interactions.background.v1beta.llm.error.permission
- t1.5.google-gemini.gemini.interactions.background.v1beta.llm.error.quota
- t1.5.google-gemini.gemini.interactions.background.v1beta.llm.error.unavailable
- t1.5.google-gemini.gemini.interactions.background.v1beta.llm.history
- t1.5.google-gemini.gemini.interactions.background.v1beta.llm.response.malformed_response
- t1.5.google-gemini.gemini.interactions.background.v1beta.llm.response.missing_required_response_field
- t1.5.google-gemini.gemini.interactions.background.v1beta.llm.response.wrong_content_type
- t1.5.google-gemini.gemini.interactions.background.v1beta.llm.success
- t1.5.google-gemini.gemini.interactions.background.v1beta.llm.tool-history
- t1.5.google-gemini.gemini.interactions.background.v1beta.vision.caption.success
- t1.5.google-gemini.gemini.interactions.background.v1beta.vision.detect.success
- t1.5.google-gemini.gemini.interactions.background.v1beta.vision.ocr.success
- t1.5.google-gemini.gemini.interactions.background.v1beta.vision.segment.success
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
- t1.5.openrouter.openrouter.audio.transcriptions.v1.audio.asr.model.google-gemini-3.5-transcribe.success
- t1.5.openrouter.openrouter.responses.v1.llm.model.google-gemini-4-argon.success
- t1.5.openrouter.openrouter.responses.v1.vision.caption.model.google-gemini-4-argon.success
- t1.5.openrouter.openrouter.responses.v1.vision.ocr.model.google-gemini-4-argon.success
- t1.5.switch.openrouter.openrouter.responses.v1.llm.to.google-gemini.gemini.interactions.background.v1beta.llm

## Confirmed product defects

- **defect.aicc.t1.5.runner** (AICC, provider_protocol_failed): expected AICC maps the official Provider protocol fixture through its real Adapter; observed captured_requests=0; provider:google-gemini:inventory: Error: google-gemini metadata variant gemini-2.5-flash-lite:reasoning-high has no T1.5 contract for audio.asr. Evidence: cases/t1.5.runner.json

## Cleanup

Status: passed

- temporary Provider instances were removed and the Mock selection was reset
