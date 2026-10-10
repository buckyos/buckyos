# AICC E2E Acceptance Report

- Run: `aicc-2026-10-08T10-49-33-585Z-a00bd81c`
- Commit: `7612d141be9938f5d1c4a89b41f20ae80459c1c1`
- Capability baseline: `2026-10-08.3`
- Real model calls: 16/15
- Planned maximum cost: $0.150000
- Known actual / unknown estimated exposure: $0.016900 / $0.110000
- Total exposure / budget: $0.126900 / $20.000000
- Unknown-cost calls: 11; budget exceeded: false
- Results: passed=0, failed=8, provider_restricted=0, skipped=0, not_applicable=0, review=7

## Targeted retest

Run after fixing the reported defect; repeat `--case` to select additional cases:

```bash
pnpm run acceptance:gateway -- --config aicc_acceptance.local.toml --allow-credential-mutation --timeout-ms 900000 --provider kimi --provider minimax --case t2.kimi.kimi-t2.kimi-k2.7-code-highspeed.llm --case t2.kimi.kimi-t2.kimi-k2.7-code-highspeed.vision.caption --case t2.kimi.kimi-t2.kimi-k2.7-code-highspeed.vision.ocr --case t2.kimi.kimi-t2.kimi-k3.llm --case t2.kimi.kimi-t2.kimi-k3.vision.caption --case t2.kimi.kimi-t2.kimi-k3.vision.ocr --case t2.minimax.minimax-t2.minimax-h3-max.video.img2video --case t2.minimax.minimax-t2.minimax-h3.video.img2video
```

## Cases

| Case | Layer | Provider | Model | API | Status |
|---|---|---|---|---|---|
| t2.kimi.kimi-t2.kimi-k2.7-code-highspeed.llm | T2 | kimi/kimi-t2 | kimi-k2.7-code-highspeed@kimi-t2 | llm | failed |
| t2.kimi.kimi-t2.kimi-k2.7-code-highspeed.vision.caption | T2 | kimi/kimi-t2 | kimi-k2.7-code-highspeed@kimi-t2 | vision.caption | failed |
| t2.kimi.kimi-t2.kimi-k2.7-code-highspeed.vision.ocr | T2 | kimi/kimi-t2 | kimi-k2.7-code-highspeed@kimi-t2 | vision.ocr | failed |
| t2.kimi.kimi-t2.kimi-k3.llm | T2 | kimi/kimi-t2 | kimi-k3@kimi-t2 | llm | failed |
| t2.kimi.kimi-t2.kimi-k3.vision.caption | T2 | kimi/kimi-t2 | kimi-k3@kimi-t2 | vision.caption | failed |
| t2.kimi.kimi-t2.kimi-k3.vision.ocr | T2 | kimi/kimi-t2 | kimi-k3@kimi-t2 | vision.ocr | failed |
| t2.minimax.minimax-t2.asr-1.0.audio.asr | T2 | minimax/minimax-t2 | asr-1.0@minimax-t2 | audio.asr | review |
| t2.minimax.minimax-t2.image-01.image.img2img | T2 | minimax/minimax-t2 | image-01@minimax-t2 | image.img2img | review |
| t2.minimax.minimax-t2.image-01.image.txt2img | T2 | minimax/minimax-t2 | image-01@minimax-t2 | image.txt2img | review |
| t2.minimax.minimax-t2.minimax-h3-max.video.img2video | T2 | minimax/minimax-t2 | MiniMax-H3-Max@minimax-t2 | video.img2video | failed |
| t2.minimax.minimax-t2.minimax-h3-max.video.txt2video | T2 | minimax/minimax-t2 | MiniMax-H3-Max@minimax-t2 | video.txt2video | review |
| t2.minimax.minimax-t2.minimax-h3.video.img2video | T2 | minimax/minimax-t2 | MiniMax-H3@minimax-t2 | video.img2video | failed |
| t2.minimax.minimax-t2.minimax-h3.video.txt2video | T2 | minimax/minimax-t2 | MiniMax-H3@minimax-t2 | video.txt2video | review |
| t2.minimax.minimax-t2.speech-2.8-hd.audio.tts | T2 | minimax/minimax-t2 | speech-2.8-hd@minimax-t2 | audio.tts | review |
| t2.minimax.minimax-t2.speech-2.8-turbo.audio.tts | T2 | minimax/minimax-t2 | speech-2.8-turbo@minimax-t2 | audio.tts | review |

## Physical model coverage

| Provider | Inventory model | Physical model | Decision | Reason | Retained representative |
|---|---|---|---|---|---|
| kimi/kimi-t2 | kimi-k2.7-code-highspeed | kimi-k2.7-code-highspeed | included | physical model | - |
| kimi/kimi-t2 | kimi-k3 | kimi-k3 | included | physical model | - |
| minimax/minimax-t2 | asr-1.0 | asr-1.0 | included | physical model | - |
| minimax/minimax-t2 | image-01 | image-01 | included | physical model | - |
| minimax/minimax-t2 | MiniMax-H3-Max | MiniMax-H3-Max | included | physical model | - |
| minimax/minimax-t2 | MiniMax-H3 | MiniMax-H3 | included | physical model | - |
| minimax/minimax-t2 | speech-2.8-hd | speech-2.8-hd | included | physical model | - |
| minimax/minimax-t2 | speech-2.8-turbo | speech-2.8-turbo | included | physical model | - |
| kimi/kimi-t2 | kimi-k2.7-code-highspeed | kimi-k2.7-code-highspeed | included | physical model | - |
| kimi/kimi-t2 | kimi-k3 | kimi-k3 | included | physical model | - |
| minimax/minimax-t2 | asr-1.0 | asr-1.0 | included | physical model | - |
| minimax/minimax-t2 | image-01 | image-01 | included | physical model | - |
| minimax/minimax-t2 | MiniMax-H3-Max | MiniMax-H3-Max | included | physical model | - |
| minimax/minimax-t2 | MiniMax-H3 | MiniMax-H3 | included | physical model | - |
| minimax/minimax-t2 | speech-2.8-hd | speech-2.8-hd | included | physical model | - |
| minimax/minimax-t2 | speech-2.8-turbo | speech-2.8-turbo | included | physical model | - |

## Confirmed product defects

None.

## Cleanup

Status: passed

- removed 0 uploaded fixture object(s)
- removed 2 generated output object(s)
- temporary Provider credentials restored after testing (kimi, minimax)
