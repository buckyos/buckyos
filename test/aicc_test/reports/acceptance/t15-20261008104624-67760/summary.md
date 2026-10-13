# AICC E2E Acceptance Report

- Run: `t15-20261008104624-67760`
- Commit: `7612d141be9938f5d1c4a89b41f20ae80459c1c1`
- Capability baseline: `official-provider-protocols-2026-10-08.1`
- Real model calls: 0/0
- Planned maximum cost: $0.000000
- Known actual / unknown estimated exposure: $0.000000 / $0.000000
- Total exposure / budget: $0.000000 / $0.000000
- Unknown-cost calls: 0; budget exceeded: false
- Results: passed=82, failed=0, provider_restricted=0, skipped=0, not_applicable=0, review=0
- Manifest coverage: 82/82 (100.00%); passed=82, failed=0

## Cases

| Case | Layer | Provider | Model | API | Status |
|---|---|---|---|---|---|
| t1.5.minimax.minimax.anthropic.messages.v1.llm.success | T1.5 | minimax/t15-20261008104624-67760-minimax | MiniMax-M3@t15-20261008104624-67760-minimax | llm | passed |
| t1.5.minimax.minimax.anthropic.messages.v1.llm.stream | T1.5 | minimax/t15-20261008104624-67760-minimax | MiniMax-M3@t15-20261008104624-67760-minimax | llm | passed |
| t1.5.minimax.minimax.anthropic.messages.v1.llm.stream-interrupted | T1.5 | minimax/t15-20261008104624-67760-minimax | MiniMax-M3@t15-20261008104624-67760-minimax | llm | passed |
| t1.5.minimax.minimax.anthropic.messages.v1.llm.error.invalid_parameters | T1.5 | minimax/t15-20261008104624-67760-minimax | MiniMax-M3@t15-20261008104624-67760-minimax | llm | passed |
| t1.5.minimax.minimax.anthropic.messages.v1.llm.error.authentication | T1.5 | minimax/t15-20261008104624-67760-minimax | MiniMax-M3@t15-20261008104624-67760-minimax | llm | passed |
| t1.5.minimax.minimax.anthropic.messages.v1.llm.error.rate_limit | T1.5 | minimax/t15-20261008104624-67760-minimax | MiniMax-M3@t15-20261008104624-67760-minimax | llm | passed |
| t1.5.minimax.minimax.anthropic.messages.v1.llm.error.insufficient_balance | T1.5 | minimax/t15-20261008104624-67760-minimax | MiniMax-M3@t15-20261008104624-67760-minimax | llm | passed |
| t1.5.minimax.minimax.anthropic.messages.v1.llm.error.timeout | T1.5 | minimax/t15-20261008104624-67760-minimax | MiniMax-M3@t15-20261008104624-67760-minimax | llm | passed |
| t1.5.minimax.minimax.anthropic.messages.v1.llm.error.content_policy | T1.5 | minimax/t15-20261008104624-67760-minimax | MiniMax-M3@t15-20261008104624-67760-minimax | llm | passed |
| t1.5.minimax.minimax.anthropic.messages.v1.llm.error.server_error | T1.5 | minimax/t15-20261008104624-67760-minimax | MiniMax-M3@t15-20261008104624-67760-minimax | llm | passed |
| t1.5.minimax.minimax.anthropic.messages.v1.llm.response.malformed_response | T1.5 | minimax/t15-20261008104624-67760-minimax | MiniMax-M3@t15-20261008104624-67760-minimax | llm | passed |
| t1.5.minimax.minimax.anthropic.messages.v1.llm.response.wrong_content_type | T1.5 | minimax/t15-20261008104624-67760-minimax | MiniMax-M3@t15-20261008104624-67760-minimax | llm | passed |
| t1.5.minimax.minimax.anthropic.messages.v1.llm.response.missing_required_response_field | T1.5 | minimax/t15-20261008104624-67760-minimax | MiniMax-M3@t15-20261008104624-67760-minimax | llm | passed |
| t1.5.minimax.minimax.anthropic.messages.v1.vision.ocr.success | T1.5 | minimax/t15-20261008104624-67760-minimax | MiniMax-M3@t15-20261008104624-67760-minimax | vision.ocr | passed |
| t1.5.minimax.minimax.anthropic.messages.v1.vision.caption.success | T1.5 | minimax/t15-20261008104624-67760-minimax | MiniMax-M3@t15-20261008104624-67760-minimax | vision.caption | passed |
| t1.5.minimax.minimax.t2a.v2.audio.tts.success | T1.5 | minimax/t15-20261008104624-67760-minimax | speech-2.8-hd@t15-20261008104624-67760-minimax | audio.tts | passed |
| t1.5.minimax.minimax.t2a.v2.audio.tts.ta[REDACTED] | T1.5 | minimax/t15-20261008104624-67760-minimax | speech-2.8-hd@t15-20261008104624-67760-minimax | audio.tts | passed |
| t1.5.minimax.minimax.t2a.v2.audio.tts.error.invalid_parameters | T1.5 | minimax/t15-20261008104624-67760-minimax | speech-2.8-hd@t15-20261008104624-67760-minimax | audio.tts | passed |
| t1.5.minimax.minimax.t2a.v2.audio.tts.error.authentication | T1.5 | minimax/t15-20261008104624-67760-minimax | speech-2.8-hd@t15-20261008104624-67760-minimax | audio.tts | passed |
| t1.5.minimax.minimax.t2a.v2.audio.tts.error.rate_limit | T1.5 | minimax/t15-20261008104624-67760-minimax | speech-2.8-hd@t15-20261008104624-67760-minimax | audio.tts | passed |
| t1.5.minimax.minimax.t2a.v2.audio.tts.error.insufficient_balance | T1.5 | minimax/t15-20261008104624-67760-minimax | speech-2.8-hd@t15-20261008104624-67760-minimax | audio.tts | passed |
| t1.5.minimax.minimax.t2a.v2.audio.tts.error.timeout | T1.5 | minimax/t15-20261008104624-67760-minimax | speech-2.8-hd@t15-20261008104624-67760-minimax | audio.tts | passed |
| t1.5.minimax.minimax.t2a.v2.audio.tts.error.content_policy | T1.5 | minimax/t15-20261008104624-67760-minimax | speech-2.8-hd@t15-20261008104624-67760-minimax | audio.tts | passed |
| t1.5.minimax.minimax.t2a.v2.audio.tts.error.server_error | T1.5 | minimax/t15-20261008104624-67760-minimax | speech-2.8-hd@t15-20261008104624-67760-minimax | audio.tts | passed |
| t1.5.minimax.minimax.t2a.v2.audio.tts.response.malformed_response | T1.5 | minimax/t15-20261008104624-67760-minimax | speech-2.8-hd@t15-20261008104624-67760-minimax | audio.tts | passed |
| t1.5.minimax.minimax.t2a.v2.audio.tts.response.wrong_content_type | T1.5 | minimax/t15-20261008104624-67760-minimax | speech-2.8-hd@t15-20261008104624-67760-minimax | audio.tts | passed |
| t1.5.minimax.minimax.t2a.v2.audio.tts.response.missing_required_response_field | T1.5 | minimax/t15-20261008104624-67760-minimax | speech-2.8-hd@t15-20261008104624-67760-minimax | audio.tts | passed |
| t1.5.minimax.minimax.speech-to-text.v1.audio.asr.success | T1.5 | minimax/t15-20261008104624-67760-minimax | asr-1.0@t15-20261008104624-67760-minimax | audio.asr | passed |
| t1.5.minimax.minimax.speech-to-text.v1.audio.asr.error.invalid_parameters | T1.5 | minimax/t15-20261008104624-67760-minimax | asr-1.0@t15-20261008104624-67760-minimax | audio.asr | passed |
| t1.5.minimax.minimax.speech-to-text.v1.audio.asr.error.authentication | T1.5 | minimax/t15-20261008104624-67760-minimax | asr-1.0@t15-20261008104624-67760-minimax | audio.asr | passed |
| t1.5.minimax.minimax.speech-to-text.v1.audio.asr.error.rate_limit | T1.5 | minimax/t15-20261008104624-67760-minimax | asr-1.0@t15-20261008104624-67760-minimax | audio.asr | passed |
| t1.5.minimax.minimax.speech-to-text.v1.audio.asr.error.insufficient_balance | T1.5 | minimax/t15-20261008104624-67760-minimax | asr-1.0@t15-20261008104624-67760-minimax | audio.asr | passed |
| t1.5.minimax.minimax.speech-to-text.v1.audio.asr.error.timeout | T1.5 | minimax/t15-20261008104624-67760-minimax | asr-1.0@t15-20261008104624-67760-minimax | audio.asr | passed |
| t1.5.minimax.minimax.speech-to-text.v1.audio.asr.error.content_policy | T1.5 | minimax/t15-20261008104624-67760-minimax | asr-1.0@t15-20261008104624-67760-minimax | audio.asr | passed |
| t1.5.minimax.minimax.speech-to-text.v1.audio.asr.error.server_error | T1.5 | minimax/t15-20261008104624-67760-minimax | asr-1.0@t15-20261008104624-67760-minimax | audio.asr | passed |
| t1.5.minimax.minimax.speech-to-text.v1.audio.asr.response.malformed_response | T1.5 | minimax/t15-20261008104624-67760-minimax | asr-1.0@t15-20261008104624-67760-minimax | audio.asr | passed |
| t1.5.minimax.minimax.speech-to-text.v1.audio.asr.response.wrong_content_type | T1.5 | minimax/t15-20261008104624-67760-minimax | asr-1.0@t15-20261008104624-67760-minimax | audio.asr | passed |
| t1.5.minimax.minimax.speech-to-text.v1.audio.asr.response.missing_required_response_field | T1.5 | minimax/t15-20261008104624-67760-minimax | asr-1.0@t15-20261008104624-67760-minimax | audio.asr | passed |
| t1.5.minimax.minimax.image-generation.v1.image.txt2img.success | T1.5 | minimax/t15-20261008104624-67760-minimax | image-01@t15-20261008104624-67760-minimax | image.txt2img | passed |
| t1.5.minimax.minimax.image-generation.v1.image.txt2img.ta[REDACTED] | T1.5 | minimax/t15-20261008104624-67760-minimax | image-01@t15-20261008104624-67760-minimax | image.txt2img | passed |
| t1.5.minimax.minimax.image-generation.v1.image.txt2img.error.invalid_parameters | T1.5 | minimax/t15-20261008104624-67760-minimax | image-01@t15-20261008104624-67760-minimax | image.txt2img | passed |
| t1.5.minimax.minimax.image-generation.v1.image.txt2img.error.authentication | T1.5 | minimax/t15-20261008104624-67760-minimax | image-01@t15-20261008104624-67760-minimax | image.txt2img | passed |
| t1.5.minimax.minimax.image-generation.v1.image.txt2img.error.rate_limit | T1.5 | minimax/t15-20261008104624-67760-minimax | image-01@t15-20261008104624-67760-minimax | image.txt2img | passed |
| t1.5.minimax.minimax.image-generation.v1.image.txt2img.error.insufficient_balance | T1.5 | minimax/t15-20261008104624-67760-minimax | image-01@t15-20261008104624-67760-minimax | image.txt2img | passed |
| t1.5.minimax.minimax.image-generation.v1.image.txt2img.error.timeout | T1.5 | minimax/t15-20261008104624-67760-minimax | image-01@t15-20261008104624-67760-minimax | image.txt2img | passed |
| t1.5.minimax.minimax.image-generation.v1.image.txt2img.error.content_policy | T1.5 | minimax/t15-20261008104624-67760-minimax | image-01@t15-20261008104624-67760-minimax | image.txt2img | passed |
| t1.5.minimax.minimax.image-generation.v1.image.txt2img.error.server_error | T1.5 | minimax/t15-20261008104624-67760-minimax | image-01@t15-20261008104624-67760-minimax | image.txt2img | passed |
| t1.5.minimax.minimax.image-generation.v1.image.txt2img.response.malformed_response | T1.5 | minimax/t15-20261008104624-67760-minimax | image-01@t15-20261008104624-67760-minimax | image.txt2img | passed |
| t1.5.minimax.minimax.image-generation.v1.image.txt2img.response.wrong_content_type | T1.5 | minimax/t15-20261008104624-67760-minimax | image-01@t15-20261008104624-67760-minimax | image.txt2img | passed |
| t1.5.minimax.minimax.image-generation.v1.image.txt2img.response.missing_required_response_field | T1.5 | minimax/t15-20261008104624-67760-minimax | image-01@t15-20261008104624-67760-minimax | image.txt2img | passed |
| t1.5.minimax.minimax.image-generation.v1.image.img2img.success | T1.5 | minimax/t15-20261008104624-67760-minimax | image-01@t15-20261008104624-67760-minimax | image.img2img | passed |
| t1.5.minimax.minimax.image-generation.v1.image.img2img.ta[REDACTED] | T1.5 | minimax/t15-20261008104624-67760-minimax | image-01@t15-20261008104624-67760-minimax | image.img2img | passed |
| t1.5.minimax.minimax.video-generation.v2.video.txt2video.success | T1.5 | minimax/t15-20261008104624-67760-minimax | MiniMax-H3@t15-20261008104624-67760-minimax | video.txt2video | passed |
| t1.5.minimax.minimax.video-generation.v2.video.txt2video.ta[REDACTED] | T1.5 | minimax/t15-20261008104624-67760-minimax | MiniMax-H3@t15-20261008104624-67760-minimax | video.txt2video | passed |
| t1.5.minimax.minimax.video-generation.v2.video.txt2video.async | T1.5 | minimax/t15-20261008104624-67760-minimax | MiniMax-H3@t15-20261008104624-67760-minimax | video.txt2video | passed |
| t1.5.minimax.minimax.video-generation.v2.video.txt2video.async-failed | T1.5 | minimax/t15-20261008104624-67760-minimax | MiniMax-H3@t15-20261008104624-67760-minimax | video.txt2video | passed |
| t1.5.minimax.minimax.video-generation.v2.video.txt2video.async_poll_timeout | T1.5 | minimax/t15-20261008104624-67760-minimax | MiniMax-H3@t15-20261008104624-67760-minimax | video.txt2video | passed |
| t1.5.minimax.minimax.video-generation.v2.video.txt2video.async-cancel | T1.5 | minimax/t15-20261008104624-67760-minimax | MiniMax-H3@t15-20261008104624-67760-minimax | video.txt2video | passed |
| t1.5.minimax.minimax.video-generation.v2.video.txt2video.error.invalid_parameters | T1.5 | minimax/t15-20261008104624-67760-minimax | MiniMax-H3@t15-20261008104624-67760-minimax | video.txt2video | passed |
| t1.5.minimax.minimax.video-generation.v2.video.txt2video.error.authentication | T1.5 | minimax/t15-20261008104624-67760-minimax | MiniMax-H3@t15-20261008104624-67760-minimax | video.txt2video | passed |
| t1.5.minimax.minimax.video-generation.v2.video.txt2video.error.rate_limit | T1.5 | minimax/t15-20261008104624-67760-minimax | MiniMax-H3@t15-20261008104624-67760-minimax | video.txt2video | passed |
| t1.5.minimax.minimax.video-generation.v2.video.txt2video.error.insufficient_balance | T1.5 | minimax/t15-20261008104624-67760-minimax | MiniMax-H3@t15-20261008104624-67760-minimax | video.txt2video | passed |
| t1.5.minimax.minimax.video-generation.v2.video.txt2video.error.timeout | T1.5 | minimax/t15-20261008104624-67760-minimax | MiniMax-H3@t15-20261008104624-67760-minimax | video.txt2video | passed |
| t1.5.minimax.minimax.video-generation.v2.video.txt2video.error.content_policy | T1.5 | minimax/t15-20261008104624-67760-minimax | MiniMax-H3@t15-20261008104624-67760-minimax | video.txt2video | passed |
| t1.5.minimax.minimax.video-generation.v2.video.txt2video.error.server_error | T1.5 | minimax/t15-20261008104624-67760-minimax | MiniMax-H3@t15-20261008104624-67760-minimax | video.txt2video | passed |
| t1.5.minimax.minimax.video-generation.v2.video.txt2video.response.malformed_response | T1.5 | minimax/t15-20261008104624-67760-minimax | MiniMax-H3@t15-20261008104624-67760-minimax | video.txt2video | passed |
| t1.5.minimax.minimax.video-generation.v2.video.txt2video.response.wrong_content_type | T1.5 | minimax/t15-20261008104624-67760-minimax | MiniMax-H3@t15-20261008104624-67760-minimax | video.txt2video | passed |
| t1.5.minimax.minimax.video-generation.v2.video.txt2video.response.missing_required_response_field | T1.5 | minimax/t15-20261008104624-67760-minimax | MiniMax-H3@t15-20261008104624-67760-minimax | video.txt2video | passed |
| t1.5.minimax.minimax.video-generation.v2.video.img2video.success | T1.5 | minimax/t15-20261008104624-67760-minimax | MiniMax-H3@t15-20261008104624-67760-minimax | video.img2video | passed |
| t1.5.minimax.minimax.video-generation.v2.video.img2video.ta[REDACTED] | T1.5 | minimax/t15-20261008104624-67760-minimax | MiniMax-H3@t15-20261008104624-67760-minimax | video.img2video | passed |
| t1.5.minimax.minimax.music-generation.v1.audio.music.success | T1.5 | minimax/t15-20261008104624-67760-minimax | music-3.0@t15-20261008104624-67760-minimax | audio.music | passed |
| t1.5.minimax.minimax.music-generation.v1.audio.music.ta[REDACTED] | T1.5 | minimax/t15-20261008104624-67760-minimax | music-3.0@t15-20261008104624-67760-minimax | audio.music | passed |
| t1.5.minimax.minimax.music-generation.v1.audio.music.error.invalid_parameters | T1.5 | minimax/t15-20261008104624-67760-minimax | music-3.0@t15-20261008104624-67760-minimax | audio.music | passed |
| t1.5.minimax.minimax.music-generation.v1.audio.music.error.authentication | T1.5 | minimax/t15-20261008104624-67760-minimax | music-3.0@t15-20261008104624-67760-minimax | audio.music | passed |
| t1.5.minimax.minimax.music-generation.v1.audio.music.error.rate_limit | T1.5 | minimax/t15-20261008104624-67760-minimax | music-3.0@t15-20261008104624-67760-minimax | audio.music | passed |
| t1.5.minimax.minimax.music-generation.v1.audio.music.error.insufficient_balance | T1.5 | minimax/t15-20261008104624-67760-minimax | music-3.0@t15-20261008104624-67760-minimax | audio.music | passed |
| t1.5.minimax.minimax.music-generation.v1.audio.music.error.timeout | T1.5 | minimax/t15-20261008104624-67760-minimax | music-3.0@t15-20261008104624-67760-minimax | audio.music | passed |
| t1.5.minimax.minimax.music-generation.v1.audio.music.error.content_policy | T1.5 | minimax/t15-20261008104624-67760-minimax | music-3.0@t15-20261008104624-67760-minimax | audio.music | passed |
| t1.5.minimax.minimax.music-generation.v1.audio.music.error.server_error | T1.5 | minimax/t15-20261008104624-67760-minimax | music-3.0@t15-20261008104624-67760-minimax | audio.music | passed |
| t1.5.minimax.minimax.music-generation.v1.audio.music.response.malformed_response | T1.5 | minimax/t15-20261008104624-67760-minimax | music-3.0@t15-20261008104624-67760-minimax | audio.music | passed |
| t1.5.minimax.minimax.music-generation.v1.audio.music.response.wrong_content_type | T1.5 | minimax/t15-20261008104624-67760-minimax | music-3.0@t15-20261008104624-67760-minimax | audio.music | passed |
| t1.5.minimax.minimax.music-generation.v1.audio.music.response.missing_required_response_field | T1.5 | minimax/t15-20261008104624-67760-minimax | music-3.0@t15-20261008104624-67760-minimax | audio.music | passed |

## Confirmed product defects

None.

## Cleanup

Status: passed

- temporary Provider instances were removed and the Mock selection was reset
