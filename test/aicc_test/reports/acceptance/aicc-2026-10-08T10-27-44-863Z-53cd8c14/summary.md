# AICC E2E Acceptance Report

- Run: `aicc-2026-10-08T10-27-44-863Z-53cd8c14`
- Commit: `7612d141be9938f5d1c4a89b41f20ae80459c1c1`
- Capability baseline: `2026-10-08.2`
- Real model calls: 19/19
- Planned maximum cost: $0.190000
- Known actual / unknown estimated exposure: $0.000222 / $0.160000
- Total exposure / budget: $0.160222 / $20.000000
- Unknown-cost calls: 16; budget exceeded: false
- Results: passed=2, failed=14, provider_restricted=0, skipped=0, not_applicable=0, review=3

## Targeted retest

Run after fixing the reported defect; repeat `--case` to select additional cases:

```bash
pnpm run acceptance:gateway -- --config aicc_acceptance.local.toml --allow-credential-mutation --timeout-ms 900000 --provider deepseek --provider kimi --provider minimax --case t2.deepseek.deepseek-t2.deepseek-flash.vision.caption --case t2.kimi.kimi-t2.kimi-k2.7-code-highspeed.llm --case t2.kimi.kimi-t2.kimi-k2.7-code-highspeed.vision.caption --case t2.kimi.kimi-t2.kimi-k2.7-code-highspeed.vision.ocr --case t2.kimi.kimi-t2.kimi-k3.llm --case t2.kimi.kimi-t2.kimi-k3.vision.caption --case t2.kimi.kimi-t2.kimi-k3.vision.ocr --case t2.minimax.minimax-t2.image-01.image.img2img --case t2.minimax.minimax-t2.image-01.image.txt2img --case t2.minimax.minimax-t2.minimax-h3-max.video.img2video --case t2.minimax.minimax-t2.minimax-h3-max.video.txt2video --case t2.minimax.minimax-t2.minimax-h3.video.img2video --case t2.minimax.minimax-t2.speech-2.8-hd.audio.tts --case t2.minimax.minimax-t2.speech-2.8-turbo.audio.tts
```

## Cases

| Case | Layer | Provider | Model | API | Status |
|---|---|---|---|---|---|
| t2.deepseek.deepseek-t2.deepseek-flash.llm | T2 | deepseek/deepseek-t2 | deepseek-flash@deepseek-t2 | llm | passed |
| t2.deepseek.deepseek-t2.deepseek-flash.vision.caption | T2 | deepseek/deepseek-t2 | deepseek-flash@deepseek-t2 | vision.caption | failed |
| t2.deepseek.deepseek-t2.deepseek-flash.vision.ocr | T2 | deepseek/deepseek-t2 | deepseek-flash@deepseek-t2 | vision.ocr | review |
| t2.deepseek.deepseek-t2.deepseek-v4-pro.llm | T2 | deepseek/deepseek-t2 | deepseek-v4-pro@deepseek-t2 | llm | passed |
| t2.kimi.kimi-t2.kimi-k2.7-code-highspeed.llm | T2 | kimi/kimi-t2 | kimi-k2.7-code-highspeed@kimi-t2 | llm | failed |
| t2.kimi.kimi-t2.kimi-k2.7-code-highspeed.vision.caption | T2 | kimi/kimi-t2 | kimi-k2.7-code-highspeed@kimi-t2 | vision.caption | failed |
| t2.kimi.kimi-t2.kimi-k2.7-code-highspeed.vision.ocr | T2 | kimi/kimi-t2 | kimi-k2.7-code-highspeed@kimi-t2 | vision.ocr | failed |
| t2.kimi.kimi-t2.kimi-k3.llm | T2 | kimi/kimi-t2 | kimi-k3@kimi-t2 | llm | failed |
| t2.kimi.kimi-t2.kimi-k3.vision.caption | T2 | kimi/kimi-t2 | kimi-k3@kimi-t2 | vision.caption | failed |
| t2.kimi.kimi-t2.kimi-k3.vision.ocr | T2 | kimi/kimi-t2 | kimi-k3@kimi-t2 | vision.ocr | failed |
| t2.minimax.minimax-t2.asr-1.0.audio.asr | T2 | minimax/minimax-t2 | asr-1.0@minimax-t2 | audio.asr | review |
| t2.minimax.minimax-t2.image-01.image.img2img | T2 | minimax/minimax-t2 | image-01@minimax-t2 | image.img2img | failed |
| t2.minimax.minimax-t2.image-01.image.txt2img | T2 | minimax/minimax-t2 | image-01@minimax-t2 | image.txt2img | failed |
| t2.minimax.minimax-t2.minimax-h3-max.video.img2video | T2 | minimax/minimax-t2 | MiniMax-H3-Max@minimax-t2 | video.img2video | failed |
| t2.minimax.minimax-t2.minimax-h3-max.video.txt2video | T2 | minimax/minimax-t2 | MiniMax-H3-Max@minimax-t2 | video.txt2video | failed |
| t2.minimax.minimax-t2.minimax-h3.video.img2video | T2 | minimax/minimax-t2 | MiniMax-H3@minimax-t2 | video.img2video | failed |
| t2.minimax.minimax-t2.minimax-h3.video.txt2video | T2 | minimax/minimax-t2 | MiniMax-H3@minimax-t2 | video.txt2video | review |
| t2.minimax.minimax-t2.speech-2.8-hd.audio.tts | T2 | minimax/minimax-t2 | speech-2.8-hd@minimax-t2 | audio.tts | failed |
| t2.minimax.minimax-t2.speech-2.8-turbo.audio.tts | T2 | minimax/minimax-t2 | speech-2.8-turbo@minimax-t2 | audio.tts | failed |

## Physical model coverage

| Provider | Inventory model | Physical model | Decision | Reason | Retained representative |
|---|---|---|---|---|---|
| deepseek/deepseek-t2 | deepseek-flash | deepseek-flash | included | physical model | - |
| deepseek/deepseek-t2 | deepseek-v4-pro | deepseek-v4-pro | included | physical model | - |
| kimi/kimi-t2 | kimi-k2.7-code-highspeed | kimi-k2.7-code-highspeed | included | physical model | - |
| kimi/kimi-t2 | kimi-k3 | kimi-k3 | included | physical model | - |
| minimax/minimax-t2 | asr-1.0 | asr-1.0 | included | physical model | - |
| minimax/minimax-t2 | image-01 | image-01 | included | physical model | - |
| minimax/minimax-t2 | MiniMax-H3-Max | MiniMax-H3-Max | included | physical model | - |
| minimax/minimax-t2 | MiniMax-H3 | MiniMax-H3 | included | physical model | - |
| minimax/minimax-t2 | speech-2.8-hd | speech-2.8-hd | included | physical model | - |
| minimax/minimax-t2 | speech-2.8-turbo | speech-2.8-turbo | included | physical model | - |
| deepseek/deepseek-t2 | deepseek-flash | deepseek-flash | included | physical model | - |
| deepseek/deepseek-t2 | deepseek-v4-pro | deepseek-v4-pro | included | physical model | - |
| kimi/kimi-t2 | kimi-k2.7-code-highspeed | kimi-k2.7-code-highspeed | included | physical model | - |
| kimi/kimi-t2 | kimi-k3 | kimi-k3 | included | physical model | - |
| minimax/minimax-t2 | asr-1.0 | asr-1.0 | included | physical model | - |
| minimax/minimax-t2 | image-01 | image-01 | included | physical model | - |
| minimax/minimax-t2 | MiniMax-H3-Max | MiniMax-H3-Max | included | physical model | - |
| minimax/minimax-t2 | MiniMax-H3 | MiniMax-H3 | included | physical model | - |
| minimax/minimax-t2 | speech-2.8-hd | speech-2.8-hd | included | physical model | - |
| minimax/minimax-t2 | speech-2.8-turbo | speech-2.8-turbo | included | physical model | - |

## Confirmed product defects

- **defect.aicc.t2.deepseek.deepseek-t2.deepseek-flash.vision.caption** (AICC, resource_failed): expected the selected exact model satisfies the case routing, capability, resource, and security assertions; observed RPCError: RPC call error: Failed due to reason: {"code":"resource_invalid","message":"resource content does not match its MIME","details":{"failure":"mime_mismatch"}}. Evidence: cases/t2.deepseek.deepseek-t2.deepseek-flash.vision.caption.json
- **defect.aicc.t2.kimi.kimi-t2.kimi-k2.7-code-highspeed.vision.caption** (AICC, resource_failed): expected the selected exact model satisfies the case routing, capability, resource, and security assertions; observed RPCError: RPC call error: Failed due to reason: {"code":"resource_invalid","message":"resource content does not match its MIME","details":{"failure":"mime_mismatch"}}. Evidence: cases/t2.kimi.kimi-t2.kimi-k2.7-code-highspeed.vision.caption.json
- **defect.aicc.t2.kimi.kimi-t2.kimi-k3.vision.caption** (AICC, resource_failed): expected the selected exact model satisfies the case routing, capability, resource, and security assertions; observed RPCError: RPC call error: Failed due to reason: {"code":"resource_invalid","message":"resource content does not match its MIME","details":{"failure":"mime_mismatch"}}. Evidence: cases/t2.kimi.kimi-t2.kimi-k3.vision.caption.json
- **defect.aicc.t2.minimax.minimax-t2.image-01.image.img2img** (AICC, resource_failed): expected the selected exact model satisfies the case routing, capability, resource, and security assertions; observed Error: PNG artifact has an invalid signature. Evidence: cases/t2.minimax.minimax-t2.image-01.image.img2img.json
- **defect.aicc.t2.minimax.minimax-t2.image-01.image.txt2img** (AICC, resource_failed): expected the selected exact model satisfies the case routing, capability, resource, and security assertions; observed Error: PNG artifact has an invalid signature. Evidence: cases/t2.minimax.minimax-t2.image-01.image.txt2img.json
- **defect.aicc.t2.minimax.minimax-t2.minimax-h3-max.video.img2video** (AICC, resource_failed): expected the selected exact model satisfies the case routing, capability, resource, and security assertions; observed RPCError: RPC call error: Failed due to reason: {"code":"resource_invalid","message":"resource content does not match its MIME","details":{"failure":"mime_mismatch"}}. Evidence: cases/t2.minimax.minimax-t2.minimax-h3-max.video.img2video.json
- **defect.aicc.t2.minimax.minimax-t2.minimax-h3.video.img2video** (AICC, resource_failed): expected the selected exact model satisfies the case routing, capability, resource, and security assertions; observed RPCError: RPC call error: Failed due to reason: {"code":"resource_invalid","message":"resource content does not match its MIME","details":{"failure":"mime_mismatch"}}. Evidence: cases/t2.minimax.minimax-t2.minimax-h3.video.img2video.json
- **defect.aicc.t2.minimax.minimax-t2.speech-2.8-hd.audio.tts** (AICC, assertion_failed): expected the selected exact model satisfies the case routing, capability, resource, and security assertions; observed RPCError: RPC call error: Failed due to reason: {"code":"no_candidate_model","message":"exact model unavailable: speech-2.8-hd@minimax-t2: filtered by canonical_field_unsatisfied"}. Evidence: cases/t2.minimax.minimax-t2.speech-2.8-hd.audio.tts.json
- **defect.aicc.t2.minimax.minimax-t2.speech-2.8-turbo.audio.tts** (AICC, assertion_failed): expected the selected exact model satisfies the case routing, capability, resource, and security assertions; observed RPCError: RPC call error: Failed due to reason: {"code":"no_candidate_model","message":"exact model unavailable: speech-2.8-turbo@minimax-t2: filtered by canonical_field_unsatisfied"}. Evidence: cases/t2.minimax.minimax-t2.speech-2.8-turbo.audio.tts.json

## Cleanup

Status: passed

- removed 0 uploaded fixture object(s)
- removed 0 generated output object(s)
- temporary Provider credentials restored after testing (deepseek, kimi, minimax)
