# AICC E2E Acceptance Report

- Run: `aicc-2026-10-14T11-10-39-661Z-90019e27`
- Commit: `e742bc1cf5be6b716ef849e624d5b381a495adb6`
- Capability baseline: `2026-10-13.1`
- Real model calls: 0/83
- Planned maximum cost: $0.830000
- Known actual / unknown estimated exposure: $0.000000 / $0.000000
- Total exposure / budget: $0.000000 / $1.000000
- Unknown-cost calls: 0; budget exceeded: false
- Results: passed=0, failed=19, provider_restricted=0, skipped=81, not_applicable=0, review=0

## Targeted retest

Run after fixing the reported defect; repeat `--case` to select additional cases:

```bash
pnpm run acceptance:gateway -- --config aicc_acceptance.local.toml --allow-credential-mutation --timeout-ms 900000 --no-real-model-calls
```

## Cases

| Case | Layer | Provider | Model | API | Status |
|---|---|---|---|---|---|
| t2.preflight.baseline_mismatch.1 | T2 | -/- | - | models.list | failed |
| t2.preflight.baseline_mismatch.2 | T2 | -/- | - | models.list | failed |
| t2.preflight.baseline_mismatch.3 | T2 | -/- | - | models.list | failed |
| t2.preflight.baseline_mismatch.4 | T2 | -/- | - | models.list | failed |
| t2.preflight.baseline_mismatch.5 | T2 | -/- | - | models.list | failed |
| t2.preflight.baseline_mismatch.6 | T2 | -/- | - | models.list | failed |
| t2.preflight.baseline_mismatch.7 | T2 | -/- | - | models.list | failed |
| t2.preflight.baseline_mismatch.8 | T2 | -/- | - | models.list | failed |
| t2.preflight.baseline_mismatch.9 | T2 | -/- | - | models.list | failed |
| t2.preflight.baseline_mismatch.10 | T2 | -/- | - | models.list | failed |
| t2.preflight.baseline_mismatch.11 | T2 | -/- | - | models.list | failed |
| t2.preflight.baseline_mismatch.12 | T2 | -/- | - | models.list | failed |
| t2.preflight.baseline_mismatch.13 | T2 | -/- | - | models.list | failed |
| t2.preflight.baseline_mismatch.14 | T2 | -/- | - | models.list | failed |
| t2.preflight.baseline_mismatch.15 | T2 | -/- | - | models.list | failed |
| t2.preflight.baseline_mismatch.16 | T2 | -/- | - | models.list | failed |
| t2.preflight.baseline_mismatch.17 | T2 | -/- | - | models.list | failed |
| t2.preflight.baseline_mismatch.18 | T2 | -/- | - | models.list | failed |
| t2.preflight.baseline_mismatch.19 | T2 | -/- | - | models.list | failed |
| t2.google-gemini.google-gemini-t2.gemini-2.5-flash-image.image.img2img | T2 | google-gemini/google-gemini-t2 | gemini-2.5-flash-image@google-gemini-t2 | image.img2img | skipped |
| t2.google-gemini.google-gemini-t2.gemini-2.5-flash-image.image.txt2img | T2 | google-gemini/google-gemini-t2 | gemini-2.5-flash-image@google-gemini-t2 | image.txt2img | skipped |
| t2.google-gemini.google-gemini-t2.gemini-2.5-flash-preview-tts.audio.tts | T2 | google-gemini/google-gemini-t2 | gemini-2.5-flash-preview-tts@google-gemini-t2 | audio.tts | skipped |
| t2.google-gemini.google-gemini-t2.gemini-2.5-flash.audio.asr | T2 | google-gemini/google-gemini-t2 | gemini-2.5-flash@google-gemini-t2 | audio.asr | skipped |
| t2.google-gemini.google-gemini-t2.gemini-2.5-flash.llm | T2 | google-gemini/google-gemini-t2 | gemini-2.5-flash@google-gemini-t2 | llm | skipped |
| t2.google-gemini.google-gemini-t2.gemini-2.5-flash.vision.caption | T2 | google-gemini/google-gemini-t2 | gemini-2.5-flash@google-gemini-t2 | vision.caption | skipped |
| t2.google-gemini.google-gemini-t2.gemini-2.5-flash.vision.detect | T2 | google-gemini/google-gemini-t2 | gemini-2.5-flash@google-gemini-t2 | vision.detect | skipped |
| t2.google-gemini.google-gemini-t2.gemini-2.5-flash.vision.ocr | T2 | google-gemini/google-gemini-t2 | gemini-2.5-flash@google-gemini-t2 | vision.ocr | skipped |
| t2.google-gemini.google-gemini-t2.gemini-2.5-flash.vision.segment | T2 | google-gemini/google-gemini-t2 | gemini-2.5-flash@google-gemini-t2 | vision.segment | skipped |
| t2.google-gemini.google-gemini-t2.gemini-2.5-pro-preview-tts.audio.tts | T2 | google-gemini/google-gemini-t2 | gemini-2.5-pro-preview-tts@google-gemini-t2 | audio.tts | skipped |
| t2.google-gemini.google-gemini-t2.gemini-3-flash-preview.audio.asr | T2 | google-gemini/google-gemini-t2 | gemini-3-flash-preview@google-gemini-t2 | audio.asr | skipped |
| t2.google-gemini.google-gemini-t2.gemini-3-flash-preview.llm | T2 | google-gemini/google-gemini-t2 | gemini-3-flash-preview@google-gemini-t2 | llm | skipped |
| t2.google-gemini.google-gemini-t2.gemini-3-flash-preview.vision.caption | T2 | google-gemini/google-gemini-t2 | gemini-3-flash-preview@google-gemini-t2 | vision.caption | skipped |
| t2.google-gemini.google-gemini-t2.gemini-3-flash-preview.vision.detect | T2 | google-gemini/google-gemini-t2 | gemini-3-flash-preview@google-gemini-t2 | vision.detect | skipped |
| t2.google-gemini.google-gemini-t2.gemini-3-flash-preview.vision.ocr | T2 | google-gemini/google-gemini-t2 | gemini-3-flash-preview@google-gemini-t2 | vision.ocr | skipped |
| t2.google-gemini.google-gemini-t2.gemini-3-flash-preview.vision.segment | T2 | google-gemini/google-gemini-t2 | gemini-3-flash-preview@google-gemini-t2 | vision.segment | skipped |
| t2.google-gemini.google-gemini-t2.gemini-3-pro-image.image.img2img | T2 | google-gemini/google-gemini-t2 | gemini-3-pro-image@google-gemini-t2 | image.img2img | skipped |
| t2.google-gemini.google-gemini-t2.gemini-3-pro-image.image.txt2img | T2 | google-gemini/google-gemini-t2 | gemini-3-pro-image@google-gemini-t2 | image.txt2img | skipped |
| t2.google-gemini.google-gemini-t2.gemini-3.1-flash-image.image.img2img | T2 | google-gemini/google-gemini-t2 | gemini-3.1-flash-image@google-gemini-t2 | image.img2img | skipped |
| t2.google-gemini.google-gemini-t2.gemini-3.1-flash-image.image.txt2img | T2 | google-gemini/google-gemini-t2 | gemini-3.1-flash-image@google-gemini-t2 | image.txt2img | skipped |
| t2.google-gemini.google-gemini-t2.gemini-3.1-flash-lite-image.image.img2img | T2 | google-gemini/google-gemini-t2 | gemini-3.1-flash-lite-image@google-gemini-t2 | image.img2img | skipped |
| t2.google-gemini.google-gemini-t2.gemini-3.1-flash-lite-image.image.txt2img | T2 | google-gemini/google-gemini-t2 | gemini-3.1-flash-lite-image@google-gemini-t2 | image.txt2img | skipped |
| t2.google-gemini.google-gemini-t2.gemini-3.1-flash-lite.audio.asr | T2 | google-gemini/google-gemini-t2 | gemini-3.1-flash-lite@google-gemini-t2 | audio.asr | skipped |
| t2.google-gemini.google-gemini-t2.gemini-3.1-flash-lite.llm | T2 | google-gemini/google-gemini-t2 | gemini-3.1-flash-lite@google-gemini-t2 | llm | skipped |
| t2.google-gemini.google-gemini-t2.gemini-3.1-flash-lite.vision.caption | T2 | google-gemini/google-gemini-t2 | gemini-3.1-flash-lite@google-gemini-t2 | vision.caption | skipped |
| t2.google-gemini.google-gemini-t2.gemini-3.1-flash-lite.vision.detect | T2 | google-gemini/google-gemini-t2 | gemini-3.1-flash-lite@google-gemini-t2 | vision.detect | skipped |
| t2.google-gemini.google-gemini-t2.gemini-3.1-flash-lite.vision.ocr | T2 | google-gemini/google-gemini-t2 | gemini-3.1-flash-lite@google-gemini-t2 | vision.ocr | skipped |
| t2.google-gemini.google-gemini-t2.gemini-3.1-flash-lite.vision.segment | T2 | google-gemini/google-gemini-t2 | gemini-3.1-flash-lite@google-gemini-t2 | vision.segment | skipped |
| t2.google-gemini.google-gemini-t2.gemini-3.1-flash-tts-preview.audio.tts | T2 | google-gemini/google-gemini-t2 | gemini-3.1-flash-tts-preview@google-gemini-t2 | audio.tts | skipped |
| t2.google-gemini.google-gemini-t2.gemini-3.1-pro-preview.audio.asr | T2 | google-gemini/google-gemini-t2 | gemini-3.1-pro-preview@google-gemini-t2 | audio.asr | skipped |
| t2.google-gemini.google-gemini-t2.gemini-3.1-pro-preview.llm | T2 | google-gemini/google-gemini-t2 | gemini-3.1-pro-preview@google-gemini-t2 | llm | skipped |
| t2.google-gemini.google-gemini-t2.gemini-3.1-pro-preview.vision.caption | T2 | google-gemini/google-gemini-t2 | gemini-3.1-pro-preview@google-gemini-t2 | vision.caption | skipped |
| t2.google-gemini.google-gemini-t2.gemini-3.1-pro-preview.vision.detect | T2 | google-gemini/google-gemini-t2 | gemini-3.1-pro-preview@google-gemini-t2 | vision.detect | skipped |
| t2.google-gemini.google-gemini-t2.gemini-3.1-pro-preview.vision.ocr | T2 | google-gemini/google-gemini-t2 | gemini-3.1-pro-preview@google-gemini-t2 | vision.ocr | skipped |
| t2.google-gemini.google-gemini-t2.gemini-3.1-pro-preview.vision.segment | T2 | google-gemini/google-gemini-t2 | gemini-3.1-pro-preview@google-gemini-t2 | vision.segment | skipped |
| t2.google-gemini.google-gemini-t2.gemini-3.5-flash-lite.audio.asr | T2 | google-gemini/google-gemini-t2 | gemini-3.5-flash-lite@google-gemini-t2 | audio.asr | skipped |
| t2.google-gemini.google-gemini-t2.gemini-3.5-flash-lite.llm | T2 | google-gemini/google-gemini-t2 | gemini-3.5-flash-lite@google-gemini-t2 | llm | skipped |
| t2.google-gemini.google-gemini-t2.gemini-3.5-flash-lite.vision.caption | T2 | google-gemini/google-gemini-t2 | gemini-3.5-flash-lite@google-gemini-t2 | vision.caption | skipped |
| t2.google-gemini.google-gemini-t2.gemini-3.5-flash-lite.vision.detect | T2 | google-gemini/google-gemini-t2 | gemini-3.5-flash-lite@google-gemini-t2 | vision.detect | skipped |
| t2.google-gemini.google-gemini-t2.gemini-3.5-flash-lite.vision.ocr | T2 | google-gemini/google-gemini-t2 | gemini-3.5-flash-lite@google-gemini-t2 | vision.ocr | skipped |
| t2.google-gemini.google-gemini-t2.gemini-3.5-flash-lite.vision.segment | T2 | google-gemini/google-gemini-t2 | gemini-3.5-flash-lite@google-gemini-t2 | vision.segment | skipped |
| t2.google-gemini.google-gemini-t2.gemini-3.5-flash.audio.asr | T2 | google-gemini/google-gemini-t2 | gemini-3.5-flash@google-gemini-t2 | audio.asr | skipped |
| t2.google-gemini.google-gemini-t2.gemini-3.5-flash.llm | T2 | google-gemini/google-gemini-t2 | gemini-3.5-flash@google-gemini-t2 | llm | skipped |
| t2.google-gemini.google-gemini-t2.gemini-3.5-flash.vision.caption | T2 | google-gemini/google-gemini-t2 | gemini-3.5-flash@google-gemini-t2 | vision.caption | skipped |
| t2.google-gemini.google-gemini-t2.gemini-3.5-flash.vision.detect | T2 | google-gemini/google-gemini-t2 | gemini-3.5-flash@google-gemini-t2 | vision.detect | skipped |
| t2.google-gemini.google-gemini-t2.gemini-3.5-flash.vision.ocr | T2 | google-gemini/google-gemini-t2 | gemini-3.5-flash@google-gemini-t2 | vision.ocr | skipped |
| t2.google-gemini.google-gemini-t2.gemini-3.5-flash.vision.segment | T2 | google-gemini/google-gemini-t2 | gemini-3.5-flash@google-gemini-t2 | vision.segment | skipped |
| t2.google-gemini.google-gemini-t2.gemini-3.5-transcribe.audio.asr | T2 | google-gemini/google-gemini-t2 | gemini-3.5-transcribe@google-gemini-t2 | audio.asr | skipped |
| t2.google-gemini.google-gemini-t2.gemini-3.6-flash.audio.asr | T2 | google-gemini/google-gemini-t2 | gemini-3.6-flash@google-gemini-t2 | audio.asr | skipped |
| t2.google-gemini.google-gemini-t2.gemini-3.6-flash.llm | T2 | google-gemini/google-gemini-t2 | gemini-3.6-flash@google-gemini-t2 | llm | skipped |
| t2.google-gemini.google-gemini-t2.gemini-3.6-flash.vision.caption | T2 | google-gemini/google-gemini-t2 | gemini-3.6-flash@google-gemini-t2 | vision.caption | skipped |
| t2.google-gemini.google-gemini-t2.gemini-3.6-flash.vision.detect | T2 | google-gemini/google-gemini-t2 | gemini-3.6-flash@google-gemini-t2 | vision.detect | skipped |
| t2.google-gemini.google-gemini-t2.gemini-3.6-flash.vision.ocr | T2 | google-gemini/google-gemini-t2 | gemini-3.6-flash@google-gemini-t2 | vision.ocr | skipped |
| t2.google-gemini.google-gemini-t2.gemini-3.6-flash.vision.segment | T2 | google-gemini/google-gemini-t2 | gemini-3.6-flash@google-gemini-t2 | vision.segment | skipped |
| t2.google-gemini.google-gemini-t2.gemini-3.7-flash.audio.asr | T2 | google-gemini/google-gemini-t2 | gemini-3.7-flash@google-gemini-t2 | audio.asr | skipped |
| t2.google-gemini.google-gemini-t2.gemini-3.7-flash.llm | T2 | google-gemini/google-gemini-t2 | gemini-3.7-flash@google-gemini-t2 | llm | skipped |
| t2.google-gemini.google-gemini-t2.gemini-3.7-flash.vision.caption | T2 | google-gemini/google-gemini-t2 | gemini-3.7-flash@google-gemini-t2 | vision.caption | skipped |
| t2.google-gemini.google-gemini-t2.gemini-3.7-flash.vision.detect | T2 | google-gemini/google-gemini-t2 | gemini-3.7-flash@google-gemini-t2 | vision.detect | skipped |
| t2.google-gemini.google-gemini-t2.gemini-3.7-flash.vision.ocr | T2 | google-gemini/google-gemini-t2 | gemini-3.7-flash@google-gemini-t2 | vision.ocr | skipped |
| t2.google-gemini.google-gemini-t2.gemini-3.7-flash.vision.segment | T2 | google-gemini/google-gemini-t2 | gemini-3.7-flash@google-gemini-t2 | vision.segment | skipped |
| t2.google-gemini.google-gemini-t2.gemini-3.8-flash.audio.asr | T2 | google-gemini/google-gemini-t2 | gemini-3.8-flash@google-gemini-t2 | audio.asr | skipped |
| t2.google-gemini.google-gemini-t2.gemini-3.8-flash.llm | T2 | google-gemini/google-gemini-t2 | gemini-3.8-flash@google-gemini-t2 | llm | skipped |
| t2.google-gemini.google-gemini-t2.gemini-3.8-flash.vision.caption | T2 | google-gemini/google-gemini-t2 | gemini-3.8-flash@google-gemini-t2 | vision.caption | skipped |
| t2.google-gemini.google-gemini-t2.gemini-3.8-flash.vision.detect | T2 | google-gemini/google-gemini-t2 | gemini-3.8-flash@google-gemini-t2 | vision.detect | skipped |
| t2.google-gemini.google-gemini-t2.gemini-3.8-flash.vision.ocr | T2 | google-gemini/google-gemini-t2 | gemini-3.8-flash@google-gemini-t2 | vision.ocr | skipped |
| t2.google-gemini.google-gemini-t2.gemini-3.8-flash.vision.segment | T2 | google-gemini/google-gemini-t2 | gemini-3.8-flash@google-gemini-t2 | vision.segment | skipped |
| t2.google-gemini.google-gemini-t2.gemini-embedding-001.embedding.text | T2 | google-gemini/google-gemini-t2 | gemini-embedding-001@google-gemini-t2 | embedding.text | skipped |
| t2.google-gemini.google-gemini-t2.gemini-embedding-2.embedding.multimodal | T2 | google-gemini/google-gemini-t2 | gemini-embedding-2@google-gemini-t2 | embedding.multimodal | skipped |
| t2.google-gemini.google-gemini-t2.gemini-embedding-2.embedding.text | T2 | google-gemini/google-gemini-t2 | gemini-embedding-2@google-gemini-t2 | embedding.text | skipped |
| t2.google-gemini.google-gemini-t2.gemini-omni-1.1-flash.video.extend | T2 | google-gemini/google-gemini-t2 | gemini-omni-1.1-flash@google-gemini-t2 | video.extend | skipped |
| t2.google-gemini.google-gemini-t2.gemini-omni-1.1-flash.video.img2video | T2 | google-gemini/google-gemini-t2 | gemini-omni-1.1-flash@google-gemini-t2 | video.img2video | skipped |
| t2.google-gemini.google-gemini-t2.gemini-omni-1.1-flash.video.txt2video | T2 | google-gemini/google-gemini-t2 | gemini-omni-1.1-flash@google-gemini-t2 | video.txt2video | skipped |
| t2.google-gemini.google-gemini-t2.gemini-omni-1.1-flash.video.video2video | T2 | google-gemini/google-gemini-t2 | gemini-omni-1.1-flash@google-gemini-t2 | video.video2video | skipped |
| t2.google-gemini.google-gemini-t2.lyria-3-clip-preview.audio.music | T2 | google-gemini/google-gemini-t2 | lyria-3-clip-preview@google-gemini-t2 | audio.music | skipped |
| t2.google-gemini.google-gemini-t2.lyria-3-pro-preview.audio.music | T2 | google-gemini/google-gemini-t2 | lyria-3-pro-preview@google-gemini-t2 | audio.music | skipped |
| t2.google-gemini.google-gemini-t2.lyria-3.5.audio.music | T2 | google-gemini/google-gemini-t2 | lyria-3.5@google-gemini-t2 | audio.music | skipped |
| t2.google-gemini.google-gemini-t2.veo-3.1-generate-preview.video.extend | T2 | google-gemini/google-gemini-t2 | veo-3.1-generate-preview@google-gemini-t2 | video.extend | skipped |
| t2.google-gemini.google-gemini-t2.veo-3.1-generate-preview.video.img2video | T2 | google-gemini/google-gemini-t2 | veo-3.1-generate-preview@google-gemini-t2 | video.img2video | skipped |
| t2.google-gemini.google-gemini-t2.veo-3.1-generate-preview.video.txt2video | T2 | google-gemini/google-gemini-t2 | veo-3.1-generate-preview@google-gemini-t2 | video.txt2video | skipped |
| t2.google-gemini.google-gemini-t2.veo-3.1-lite-generate-preview.video.img2video | T2 | google-gemini/google-gemini-t2 | veo-3.1-lite-generate-preview@google-gemini-t2 | video.img2video | skipped |
| t2.google-gemini.google-gemini-t2.veo-3.1-lite-generate-preview.video.txt2video | T2 | google-gemini/google-gemini-t2 | veo-3.1-lite-generate-preview@google-gemini-t2 | video.txt2video | skipped |

## Physical model coverage

| Provider | Inventory model | Physical model | Decision | Reason | Retained representative |
|---|---|---|---|---|---|
| google-gemini/google-gemini-t2 | antigravity-preview-05-2026 | antigravity-preview-05-2026 | filtered | unsupported_canonical_protocol | - |
| google-gemini/google-gemini-t2 | antigravity-preview-09-2026 | antigravity-preview-09-2026 | included | physical model | - |
| google-gemini/google-gemini-t2 | antigravity-preview-latest | antigravity-preview-latest | filtered | logical_alias | - |
| google-gemini/google-gemini-t2 | aqa | aqa | filtered | unsupported_canonical_protocol | - |
| google-gemini/google-gemini-t2 | deep-research-max-preview-04-2026 | deep-research-max-preview-04-2026 | filtered | unsupported_canonical_protocol | - |
| google-gemini/google-gemini-t2 | deep-research-preview-04-2026 | deep-research-preview-04-2026 | filtered | unsupported_canonical_protocol | - |
| google-gemini/google-gemini-t2 | deep-research-pro-preview-12-2025 | deep-research-pro-preview-12-2025 | filtered | unsupported_canonical_protocol | - |
| google-gemini/google-gemini-t2 | gemini-2.5-computer-use-preview-10-2025 | gemini-2.5-computer-use-preview-10-2025 | included | physical model | - |
| google-gemini/google-gemini-t2 | gemini-2.5-flash-image | gemini-2.5-flash-image | included | physical model | - |
| google-gemini/google-gemini-t2 | gemini-2.5-flash-lite | gemini-2.5-flash-lite | included | physical model | - |
| google-gemini/google-gemini-t2 | gemini-2.5-flash-native-audio-latest | gemini-2.5-flash-native-audio-latest | filtered | logical_alias | - |
| google-gemini/google-gemini-t2 | gemini-2.5-flash-native-audio-preview-09-2025 | gemini-2.5-flash-native-audio-preview-09-2025 | filtered | unsupported_canonical_protocol | - |
| google-gemini/google-gemini-t2 | gemini-2.5-flash-native-audio-preview-12-2025 | gemini-2.5-flash-native-audio-preview-12-2025 | filtered | unsupported_canonical_protocol | - |
| google-gemini/google-gemini-t2 | gemini-2.5-flash-preview-tts | gemini-2.5-flash-preview-tts | included | physical model | - |
| google-gemini/google-gemini-t2 | gemini-2.5-flash | gemini-2.5-flash | included | physical model | - |
| google-gemini/google-gemini-t2 | gemini-2.5-pro-preview-tts | gemini-2.5-pro-preview-tts | included | physical model | - |
| google-gemini/google-gemini-t2 | gemini-2.5-pro | gemini-2.5-pro | included | physical model | - |
| google-gemini/google-gemini-t2 | gemini-3-flash-preview | gemini-3-flash-preview | included | physical model | - |
| google-gemini/google-gemini-t2 | gemini-3-pro-image-preview | gemini-3-pro-image-preview | filtered | deprecated_or_retiring | - |
| google-gemini/google-gemini-t2 | gemini-3-pro-image | gemini-3-pro-image | included | physical model | - |
| google-gemini/google-gemini-t2 | gemini-3.1-flash-image-preview | gemini-3.1-flash-image-preview | filtered | deprecated_or_retiring | - |
| google-gemini/google-gemini-t2 | gemini-3.1-flash-image | gemini-3.1-flash-image | included | physical model | - |
| google-gemini/google-gemini-t2 | gemini-3.1-flash-lite-image | gemini-3.1-flash-lite-image | included | physical model | - |
| google-gemini/google-gemini-t2 | gemini-3.1-flash-lite-preview | gemini-3.1-flash-lite-preview | included | physical model | - |
| google-gemini/google-gemini-t2 | gemini-3.1-flash-lite | gemini-3.1-flash-lite | included | physical model | - |
| google-gemini/google-gemini-t2 | gemini-3.1-flash-live-preview | gemini-3.1-flash-live-preview | filtered | unsupported_canonical_protocol | - |
| google-gemini/google-gemini-t2 | gemini-3.1-flash-tts-preview | gemini-3.1-flash-tts-preview | included | physical model | - |
| google-gemini/google-gemini-t2 | gemini-3.1-pro-preview-customtools | gemini-3.1-pro-preview-customtools | included | physical model | - |
| google-gemini/google-gemini-t2 | gemini-3.1-pro-preview | gemini-3.1-pro-preview | included | physical model | - |
| google-gemini/google-gemini-t2 | gemini-3.5-flash-lite | gemini-3.5-flash-lite | included | physical model | - |
| google-gemini/google-gemini-t2 | gemini-3.5-flash | gemini-3.5-flash | included | physical model | - |
| google-gemini/google-gemini-t2 | gemini-3.5-live-translate-preview | gemini-3.5-live-translate-preview | filtered | unsupported_canonical_protocol | - |
| google-gemini/google-gemini-t2 | gemini-3.5-transcribe-live | gemini-3.5-transcribe-live | filtered | unsupported_canonical_protocol | - |
| google-gemini/google-gemini-t2 | gemini-3.5-transcribe | gemini-3.5-transcribe | included | physical model | - |
| google-gemini/google-gemini-t2 | gemini-3.6-flash | gemini-3.6-flash | included | physical model | - |
| google-gemini/google-gemini-t2 | gemini-3.7-flash | gemini-3.7-flash | included | physical model | - |
| google-gemini/google-gemini-t2 | gemini-3.8-flash-lite-tts | gemini-3.8-flash-lite-tts | included | physical model | - |
| google-gemini/google-gemini-t2 | gemini-3.8-flash-tts | gemini-3.8-flash-tts | included | physical model | - |
| google-gemini/google-gemini-t2 | gemini-3.8-flash | gemini-3.8-flash | included | physical model | - |
| google-gemini/google-gemini-t2 | gemini-3.8-live-extended-thinking | gemini-3.8-live-extended-thinking | included | physical model | - |
| google-gemini/google-gemini-t2 | gemini-3.8-live | gemini-3.8-live | included | physical model | - |
| google-gemini/google-gemini-t2 | gemini-4-argon | gemini-4-argon | included | physical model | - |
| google-gemini/google-gemini-t2 | gemini-embedding-001 | gemini-embedding-001 | included | physical model | - |
| google-gemini/google-gemini-t2 | gemini-embedding-2-preview | gemini-embedding-2-preview | filtered | deprecated_or_retiring | - |
| google-gemini/google-gemini-t2 | gemini-embedding-2 | gemini-embedding-2 | included | physical model | - |
| google-gemini/google-gemini-t2 | gemini-flash-latest | gemini-flash-latest | filtered | logical_alias | - |
| google-gemini/google-gemini-t2 | gemini-flash-lite-latest | gemini-flash-lite-latest | filtered | logical_alias | - |
| google-gemini/google-gemini-t2 | gemini-nano-banana-2.1 | gemini-nano-banana-2.1 | included | physical model | - |
| google-gemini/google-gemini-t2 | gemini-omni-1.1-flash | gemini-omni-1.1-flash | included | physical model | - |
| google-gemini/google-gemini-t2 | gemini-omni-flash-preview | gemini-omni-flash-preview | included | physical model | - |
| google-gemini/google-gemini-t2 | gemini-pro-latest | gemini-pro-latest | filtered | logical_alias | - |
| google-gemini/google-gemini-t2 | gemini-robotics-er-2-preview | gemini-robotics-er-2-preview | included | physical model | - |
| google-gemini/google-gemini-t2 | gemini-robotics-er-2-streaming-preview | gemini-robotics-er-2-streaming-preview | filtered | not_physical_model | - |
| google-gemini/google-gemini-t2 | gemma-4-26b-a4b-it | gemma-4-26b-a4b-it | included | physical model | - |
| google-gemini/google-gemini-t2 | gemma-4-31b-it | gemma-4-31b-it | included | physical model | - |
| google-gemini/google-gemini-t2 | lyria-3-clip-preview | lyria-3-clip-preview | included | physical model | - |
| google-gemini/google-gemini-t2 | lyria-3-pro-preview | lyria-3-pro-preview | included | physical model | - |
| google-gemini/google-gemini-t2 | lyria-3.5 | lyria-3.5 | included | physical model | - |
| google-gemini/google-gemini-t2 | lyria-realtime-exp | lyria-realtime-exp | included | physical model | - |
| google-gemini/google-gemini-t2 | nano-banana-pro-preview | nano-banana-pro-preview | filtered | logical_alias | - |
| google-gemini/google-gemini-t2 | veo-3.1-fast-generate-preview | veo-3.1-fast-generate-preview | included | physical model | - |
| google-gemini/google-gemini-t2 | veo-3.1-generate-preview | veo-3.1-generate-preview | included | physical model | - |
| google-gemini/google-gemini-t2 | veo-3.1-lite-generate-preview | veo-3.1-lite-generate-preview | included | physical model | - |
| google-gemini/google-gemini-t2 | gemini-2.5-flash-image | gemini-2.5-flash-image | included | physical model | - |
| google-gemini/google-gemini-t2 | gemini-2.5-flash-preview-tts | gemini-2.5-flash-preview-tts | included | physical model | - |
| google-gemini/google-gemini-t2 | gemini-2.5-flash | gemini-2.5-flash | included | physical model | - |
| google-gemini/google-gemini-t2 | gemini-2.5-pro-preview-tts | gemini-2.5-pro-preview-tts | included | physical model | - |
| google-gemini/google-gemini-t2 | gemini-3-flash-preview | gemini-3-flash-preview | included | physical model | - |
| google-gemini/google-gemini-t2 | gemini-3-pro-image | gemini-3-pro-image | included | physical model | - |
| google-gemini/google-gemini-t2 | gemini-3.1-flash-image | gemini-3.1-flash-image | included | physical model | - |
| google-gemini/google-gemini-t2 | gemini-3.1-flash-lite-image | gemini-3.1-flash-lite-image | included | physical model | - |
| google-gemini/google-gemini-t2 | gemini-3.1-flash-lite | gemini-3.1-flash-lite | included | physical model | - |
| google-gemini/google-gemini-t2 | gemini-3.1-flash-tts-preview | gemini-3.1-flash-tts-preview | included | physical model | - |
| google-gemini/google-gemini-t2 | gemini-3.1-pro-preview | gemini-3.1-pro-preview | included | physical model | - |
| google-gemini/google-gemini-t2 | gemini-3.5-flash-lite | gemini-3.5-flash-lite | included | physical model | - |
| google-gemini/google-gemini-t2 | gemini-3.5-flash | gemini-3.5-flash | included | physical model | - |
| google-gemini/google-gemini-t2 | gemini-3.5-transcribe | gemini-3.5-transcribe | included | physical model | - |
| google-gemini/google-gemini-t2 | gemini-3.6-flash | gemini-3.6-flash | included | physical model | - |
| google-gemini/google-gemini-t2 | gemini-3.7-flash | gemini-3.7-flash | included | physical model | - |
| google-gemini/google-gemini-t2 | gemini-3.8-flash | gemini-3.8-flash | included | physical model | - |
| google-gemini/google-gemini-t2 | gemini-embedding-001 | gemini-embedding-001 | included | physical model | - |
| google-gemini/google-gemini-t2 | gemini-embedding-2-preview | gemini-embedding-2-preview | filtered | deprecated_or_retiring | - |
| google-gemini/google-gemini-t2 | gemini-embedding-2 | gemini-embedding-2 | included | physical model | - |
| google-gemini/google-gemini-t2 | gemini-omni-1.1-flash | gemini-omni-1.1-flash | included | physical model | - |
| google-gemini/google-gemini-t2 | lyria-3-clip-preview | lyria-3-clip-preview | included | physical model | - |
| google-gemini/google-gemini-t2 | lyria-3-pro-preview | lyria-3-pro-preview | included | physical model | - |
| google-gemini/google-gemini-t2 | lyria-3.5 | lyria-3.5 | included | physical model | - |
| google-gemini/google-gemini-t2 | veo-3.1-generate-preview | veo-3.1-generate-preview | included | physical model | - |
| google-gemini/google-gemini-t2 | veo-3.1-lite-generate-preview | veo-3.1-lite-generate-preview | included | physical model | - |

## Confirmed product defects

- **defect.aicc.t2.preflight.baseline_mismatch.1** (AICC, baseline_mismatch): expected official Provider capability, AICC inventory declaration, and adapter support are bidirectionally consistent; observed missing official capability rule for google-gemini/antigravity-preview-09-2026. Evidence: cases/t2.preflight.baseline_mismatch.1.json, effective_baseline.json
- **defect.aicc.t2.preflight.baseline_mismatch.2** (AICC, baseline_mismatch): expected official Provider capability, AICC inventory declaration, and adapter support are bidirectionally consistent; observed official_supported_but_aicc_missing google-gemini/gemini-2.5-computer-use-preview-10-2025. Evidence: cases/t2.preflight.baseline_mismatch.2.json, effective_baseline.json
- **defect.aicc.t2.preflight.baseline_mismatch.3** (AICC, baseline_mismatch): expected official Provider capability, AICC inventory declaration, and adapter support are bidirectionally consistent; observed official_supported_but_aicc_missing google-gemini/gemini-2.5-flash-lite. Evidence: cases/t2.preflight.baseline_mismatch.3.json, effective_baseline.json
- **defect.aicc.t2.preflight.baseline_mismatch.4** (AICC, baseline_mismatch): expected official Provider capability, AICC inventory declaration, and adapter support are bidirectionally consistent; observed official_supported_but_aicc_missing google-gemini/gemini-2.5-pro. Evidence: cases/t2.preflight.baseline_mismatch.4.json, effective_baseline.json
- **defect.aicc.t2.preflight.baseline_mismatch.5** (AICC, baseline_mismatch): expected official Provider capability, AICC inventory declaration, and adapter support are bidirectionally consistent; observed official_supported_but_aicc_missing google-gemini/gemini-3.1-flash-lite-preview. Evidence: cases/t2.preflight.baseline_mismatch.5.json, effective_baseline.json
- **defect.aicc.t2.preflight.baseline_mismatch.6** (AICC, baseline_mismatch): expected official Provider capability, AICC inventory declaration, and adapter support are bidirectionally consistent; observed official_supported_but_aicc_missing google-gemini/gemini-3.1-pro-preview-customtools. Evidence: cases/t2.preflight.baseline_mismatch.6.json, effective_baseline.json
- **defect.aicc.t2.preflight.baseline_mismatch.7** (AICC, baseline_mismatch): expected official Provider capability, AICC inventory declaration, and adapter support are bidirectionally consistent; observed official_supported_but_aicc_missing google-gemini/gemini-3.8-flash-lite-tts. Evidence: cases/t2.preflight.baseline_mismatch.7.json, effective_baseline.json
- **defect.aicc.t2.preflight.baseline_mismatch.8** (AICC, baseline_mismatch): expected official Provider capability, AICC inventory declaration, and adapter support are bidirectionally consistent; observed official_supported_but_aicc_missing google-gemini/gemini-3.8-flash-tts. Evidence: cases/t2.preflight.baseline_mismatch.8.json, effective_baseline.json
- **defect.aicc.t2.preflight.baseline_mismatch.9** (AICC, baseline_mismatch): expected official Provider capability, AICC inventory declaration, and adapter support are bidirectionally consistent; observed official_supported_but_aicc_missing google-gemini/gemini-3.8-live-extended-thinking. Evidence: cases/t2.preflight.baseline_mismatch.9.json, effective_baseline.json
- **defect.aicc.t2.preflight.baseline_mismatch.10** (AICC, baseline_mismatch): expected official Provider capability, AICC inventory declaration, and adapter support are bidirectionally consistent; observed official_supported_but_aicc_missing google-gemini/gemini-3.8-live. Evidence: cases/t2.preflight.baseline_mismatch.10.json, effective_baseline.json
- **defect.aicc.t2.preflight.baseline_mismatch.11** (AICC, baseline_mismatch): expected official Provider capability, AICC inventory declaration, and adapter support are bidirectionally consistent; observed official_supported_but_aicc_missing google-gemini/gemini-4-argon. Evidence: cases/t2.preflight.baseline_mismatch.11.json, effective_baseline.json
- **defect.aicc.t2.preflight.baseline_mismatch.12** (AICC, baseline_mismatch): expected official Provider capability, AICC inventory declaration, and adapter support are bidirectionally consistent; observed official_supported_but_aicc_missing google-gemini/gemini-nano-banana-2.1. Evidence: cases/t2.preflight.baseline_mismatch.12.json, effective_baseline.json
- **defect.aicc.t2.preflight.baseline_mismatch.13** (AICC, baseline_mismatch): expected official Provider capability, AICC inventory declaration, and adapter support are bidirectionally consistent; observed official_supported_but_aicc_missing google-gemini/gemini-omni-flash-preview. Evidence: cases/t2.preflight.baseline_mismatch.13.json, effective_baseline.json
- **defect.aicc.t2.preflight.baseline_mismatch.14** (AICC, baseline_mismatch): expected official Provider capability, AICC inventory declaration, and adapter support are bidirectionally consistent; observed official_supported_but_aicc_missing google-gemini/gemini-robotics-er-2-preview. Evidence: cases/t2.preflight.baseline_mismatch.14.json, effective_baseline.json
- **defect.aicc.t2.preflight.baseline_mismatch.15** (AICC, baseline_mismatch): expected official Provider capability, AICC inventory declaration, and adapter support are bidirectionally consistent; observed official_supported_but_aicc_missing google-gemini/gemma-4-26b-a4b-it. Evidence: cases/t2.preflight.baseline_mismatch.15.json, effective_baseline.json
- **defect.aicc.t2.preflight.baseline_mismatch.16** (AICC, baseline_mismatch): expected official Provider capability, AICC inventory declaration, and adapter support are bidirectionally consistent; observed official_supported_but_aicc_missing google-gemini/gemma-4-31b-it. Evidence: cases/t2.preflight.baseline_mismatch.16.json, effective_baseline.json
- **defect.aicc.t2.preflight.baseline_mismatch.17** (AICC, baseline_mismatch): expected official Provider capability, AICC inventory declaration, and adapter support are bidirectionally consistent; observed official_supported_but_aicc_missing google-gemini/lyria-realtime-exp. Evidence: cases/t2.preflight.baseline_mismatch.17.json, effective_baseline.json
- **defect.aicc.t2.preflight.baseline_mismatch.18** (AICC, baseline_mismatch): expected official Provider capability, AICC inventory declaration, and adapter support are bidirectionally consistent; observed official_supported_but_aicc_missing google-gemini/veo-3.1-fast-generate-preview. Evidence: cases/t2.preflight.baseline_mismatch.18.json, effective_baseline.json
- **defect.aicc.t2.preflight.baseline_mismatch.19** (AICC, baseline_mismatch): expected official Provider capability, AICC inventory declaration, and adapter support are bidirectionally consistent; observed official_not_supported_but_aicc_advertised veo-3.1-lite-generate-preview@google-gemini-t2 video.extend. Evidence: cases/t2.preflight.baseline_mismatch.19.json, effective_baseline.json

## Cleanup

Status: passed

- removed 0 uploaded fixture object(s)
- removed 0 generated output object(s)
- temporary Provider credentials restored after testing (google-gemini)
