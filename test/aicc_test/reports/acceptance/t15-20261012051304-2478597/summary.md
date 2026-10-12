# AICC E2E Acceptance Report

- Run: `t15-20261012051304-2478597`
- Commit: `6a2894a88d79780ce0c3a601ff4b07f91ca135c2`
- Capability baseline: `official-provider-protocols-2026-10-10.2`
- Real model calls: 0/0
- Planned maximum cost: $0.000000
- Known actual / unknown estimated exposure: $0.000000 / $0.000000
- Total exposure / budget: $0.000000 / $0.000000
- Unknown-cost calls: 0; budget exceeded: false
- Results: passed=12, failed=0, provider_restricted=0, skipped=0, not_applicable=0, review=0
- Manifest coverage: 12/12 (100.00%); passed=12, failed=0

## Cases

| Case | Layer | Provider | Model | API | Status |
|---|---|---|---|---|---|
| t1.5.doubao-speech.doubao-speech.tts.v3.audio.tts.success | T1.5 | doubao-speech/t15-20261012051304-2478597-doubao-speech | doubao-seed-tts-2.0@t15-20261012051304-2478597-doubao-speech | audio.tts | passed |
| t1.5.doubao-speech.doubao-speech.asr.flash.v3.audio.asr.error.empty_audio | T1.5 | doubao-speech/t15-20261012051304-2478597-doubao-speech | doubao-seed-asr-2.0-fast@t15-20261012051304-2478597-doubao-speech | audio.asr | passed |
| t1.5.doubao-speech.doubao-speech.asr.flash.v3.audio.asr.error.invalid_audio_format | T1.5 | doubao-speech/t15-20261012051304-2478597-doubao-speech | doubao-seed-asr-2.0-fast@t15-20261012051304-2478597-doubao-speech | audio.asr | passed |
| t1.5.doubao-speech.doubao-speech.asr.flash.v3.audio.asr.error.server_busy | T1.5 | doubao-speech/t15-20261012051304-2478597-doubao-speech | doubao-seed-asr-2.0-fast@t15-20261012051304-2478597-doubao-speech | audio.asr | passed |
| t1.5.doubao-speech.doubao-speech.asr.task.v3.audio.asr.error.empty_audio | T1.5 | doubao-speech/t15-20261012051304-2478597-doubao-speech | doubao-seed-asr-2.0@t15-20261012051304-2478597-doubao-speech | audio.asr | passed |
| t1.5.doubao-speech.doubao-speech.asr.task.v3.audio.asr.error.invalid_audio_format | T1.5 | doubao-speech/t15-20261012051304-2478597-doubao-speech | doubao-seed-asr-2.0@t15-20261012051304-2478597-doubao-speech | audio.asr | passed |
| t1.5.doubao-speech.doubao-speech.asr.task.v3.audio.asr.error.server_busy | T1.5 | doubao-speech/t15-20261012051304-2478597-doubao-speech | doubao-seed-asr-2.0@t15-20261012051304-2478597-doubao-speech | audio.asr | passed |
| t1.5.doubao-agent-plan.doubao-agent-plan.responses.plan-v3.llm.variant.kimi-k2.8-preview-reasoning-high | T1.5 | doubao-agent-plan/t15-20261012051304-2478597-doubao-agent-plan | kimi-k2.8-preview:reasoning-high@t15-20261012051304-2478597-doubao-agent-plan | llm | passed |
| t1.5.doubao-agent-plan.doubao-agent-plan.responses.plan-v3.llm.variant.kimi-k2.8-preview-reasoning-low | T1.5 | doubao-agent-plan/t15-20261012051304-2478597-doubao-agent-plan | kimi-k2.8-preview:reasoning-low@t15-20261012051304-2478597-doubao-agent-plan | llm | passed |
| t1.5.doubao-agent-plan.doubao-agent-plan.responses.plan-v3.llm.variant.kimi-k2.8-preview-reasoning-max | T1.5 | doubao-agent-plan/t15-20261012051304-2478597-doubao-agent-plan | kimi-k2.8-preview:reasoning-max@t15-20261012051304-2478597-doubao-agent-plan | llm | passed |
| t1.5.qwen.qwen.responses.compatible-v1.vision.caption.variant.qwen3.8-omni-flash-reasoning-none | T1.5 | qwen/t15-20261012051304-2478597-qwen | qwen3.8-omni-flash:reasoning-none@t15-20261012051304-2478597-qwen | vision.caption | passed |
| t1.5.qwen.qwen.responses.compatible-v1.vision.ocr.variant.qwen3.8-omni-flash-reasoning-none | T1.5 | qwen/t15-20261012051304-2478597-qwen | qwen3.8-omni-flash:reasoning-none@t15-20261012051304-2478597-qwen | vision.ocr | passed |

## Confirmed product defects

None.

## Cleanup

Status: passed

- temporary Provider instances were removed and the Mock selection was reset
