# AICC E2E Acceptance Report

- Run: `aicc-2026-10-14T11-35-43-776Z-4f156a50`
- Commit: `e742bc1cf5be6b716ef849e624d5b381a495adb6`
- Capability baseline: `2026-10-14.1`
- Real model calls: 0/124
- Planned maximum cost: $1.240000
- Known actual / unknown estimated exposure: $0.000000 / $0.000000
- Total exposure / budget: $0.000000 / $1.000000
- Unknown-cost calls: 0; budget exceeded: false
- Results: passed=0, failed=3, provider_restricted=0, skipped=121, not_applicable=0, review=0

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
| t2.google-gemini.google-gemini-t2.gemini-2.5-flash-image.image.img2img | T2 | google-gemini/google-gemini-t2 | gemini-2.5-flash-image@google-gemini-t2 | image.img2img | skipped |
| t2.google-gemini.google-gemini-t2.gemini-2.5-flash-image.image.txt2img | T2 | google-gemini/google-gemini-t2 | gemini-2.5-flash-image@google-gemini-t2 | image.txt2img | skipped |
| t2.google-gemini.google-gemini-t2.gemini-2.5-flash-lite.audio.asr | T2 | google-gemini/google-gemini-t2 | gemini-2.5-flash-lite@google-gemini-t2 | audio.asr | skipped |
| t2.google-gemini.google-gemini-t2.gemini-2.5-flash-lite.llm | T2 | google-gemini/google-gemini-t2 | gemini-2.5-flash-lite@google-gemini-t2 | llm | skipped |
| t2.google-gemini.google-gemini-t2.gemini-2.5-flash-lite.vision.caption | T2 | google-gemini/google-gemini-t2 | gemini-2.5-flash-lite@google-gemini-t2 | vision.caption | skipped |
| t2.google-gemini.google-gemini-t2.gemini-2.5-flash-lite.vision.detect | T2 | google-gemini/google-gemini-t2 | gemini-2.5-flash-lite@google-gemini-t2 | vision.detect | skipped |
| t2.google-gemini.google-gemini-t2.gemini-2.5-flash-lite.vision.ocr | T2 | google-gemini/google-gemini-t2 | gemini-2.5-flash-lite@google-gemini-t2 | vision.ocr | skipped |
| t2.google-gemini.google-gemini-t2.gemini-2.5-flash-lite.vision.segment | T2 | google-gemini/google-gemini-t2 | gemini-2.5-flash-lite@google-gemini-t2 | vision.segment | skipped |
| t2.google-gemini.google-gemini-t2.gemini-2.5-flash-preview-tts.audio.tts | T2 | google-gemini/google-gemini-t2 | gemini-2.5-flash-preview-tts@google-gemini-t2 | audio.tts | skipped |
| t2.google-gemini.google-gemini-t2.gemini-2.5-flash.audio.asr | T2 | google-gemini/google-gemini-t2 | gemini-2.5-flash@google-gemini-t2 | audio.asr | skipped |
| t2.google-gemini.google-gemini-t2.gemini-2.5-flash.llm | T2 | google-gemini/google-gemini-t2 | gemini-2.5-flash@google-gemini-t2 | llm | skipped |
| t2.google-gemini.google-gemini-t2.gemini-2.5-flash.vision.caption | T2 | google-gemini/google-gemini-t2 | gemini-2.5-flash@google-gemini-t2 | vision.caption | skipped |
| t2.google-gemini.google-gemini-t2.gemini-2.5-flash.vision.detect | T2 | google-gemini/google-gemini-t2 | gemini-2.5-flash@google-gemini-t2 | vision.detect | skipped |
| t2.google-gemini.google-gemini-t2.gemini-2.5-flash.vision.ocr | T2 | google-gemini/google-gemini-t2 | gemini-2.5-flash@google-gemini-t2 | vision.ocr | skipped |
| t2.google-gemini.google-gemini-t2.gemini-2.5-flash.vision.segment | T2 | google-gemini/google-gemini-t2 | gemini-2.5-flash@google-gemini-t2 | vision.segment | skipped |
| t2.google-gemini.google-gemini-t2.gemini-2.5-pro-preview-tts.audio.tts | T2 | google-gemini/google-gemini-t2 | gemini-2.5-pro-preview-tts@google-gemini-t2 | audio.tts | skipped |
| t2.google-gemini.google-gemini-t2.gemini-2.5-pro.audio.asr | T2 | google-gemini/google-gemini-t2 | gemini-2.5-pro@google-gemini-t2 | audio.asr | skipped |
| t2.google-gemini.google-gemini-t2.gemini-2.5-pro.llm | T2 | google-gemini/google-gemini-t2 | gemini-2.5-pro@google-gemini-t2 | llm | skipped |
| t2.google-gemini.google-gemini-t2.gemini-2.5-pro.vision.caption | T2 | google-gemini/google-gemini-t2 | gemini-2.5-pro@google-gemini-t2 | vision.caption | skipped |
| t2.google-gemini.google-gemini-t2.gemini-2.5-pro.vision.detect | T2 | google-gemini/google-gemini-t2 | gemini-2.5-pro@google-gemini-t2 | vision.detect | skipped |
| t2.google-gemini.google-gemini-t2.gemini-2.5-pro.vision.ocr | T2 | google-gemini/google-gemini-t2 | gemini-2.5-pro@google-gemini-t2 | vision.ocr | skipped |
| t2.google-gemini.google-gemini-t2.gemini-2.5-pro.vision.segment | T2 | google-gemini/google-gemini-t2 | gemini-2.5-pro@google-gemini-t2 | vision.segment | skipped |
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
| t2.google-gemini.google-gemini-t2.gemini-3.1-pro-preview-customtools.llm | T2 | google-gemini/google-gemini-t2 | gemini-3.1-pro-preview-customtools@google-gemini-t2 | llm | skipped |
| t2.google-gemini.google-gemini-t2.gemini-3.1-pro-preview-customtools.vision.caption | T2 | google-gemini/google-gemini-t2 | gemini-3.1-pro-preview-customtools@google-gemini-t2 | vision.caption | skipped |
| t2.google-gemini.google-gemini-t2.gemini-3.1-pro-preview-customtools.vision.detect | T2 | google-gemini/google-gemini-t2 | gemini-3.1-pro-preview-customtools@google-gemini-t2 | vision.detect | skipped |
| t2.google-gemini.google-gemini-t2.gemini-3.1-pro-preview-customtools.vision.ocr | T2 | google-gemini/google-gemini-t2 | gemini-3.1-pro-preview-customtools@google-gemini-t2 | vision.ocr | skipped |
| t2.google-gemini.google-gemini-t2.gemini-3.1-pro-preview-customtools.vision.segment | T2 | google-gemini/google-gemini-t2 | gemini-3.1-pro-preview-customtools@google-gemini-t2 | vision.segment | skipped |
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
| t2.google-gemini.google-gemini-t2.gemini-3.8-flash-lite-tts.audio.tts | T2 | google-gemini/google-gemini-t2 | gemini-3.8-flash-lite-tts@google-gemini-t2 | audio.tts | skipped |
| t2.google-gemini.google-gemini-t2.gemini-3.8-flash-tts.audio.tts | T2 | google-gemini/google-gemini-t2 | gemini-3.8-flash-tts@google-gemini-t2 | audio.tts | skipped |
| t2.google-gemini.google-gemini-t2.gemini-3.8-flash.audio.asr | T2 | google-gemini/google-gemini-t2 | gemini-3.8-flash@google-gemini-t2 | audio.asr | skipped |
| t2.google-gemini.google-gemini-t2.gemini-3.8-flash.llm | T2 | google-gemini/google-gemini-t2 | gemini-3.8-flash@google-gemini-t2 | llm | skipped |
| t2.google-gemini.google-gemini-t2.gemini-3.8-flash.vision.caption | T2 | google-gemini/google-gemini-t2 | gemini-3.8-flash@google-gemini-t2 | vision.caption | skipped |
| t2.google-gemini.google-gemini-t2.gemini-3.8-flash.vision.detect | T2 | google-gemini/google-gemini-t2 | gemini-3.8-flash@google-gemini-t2 | vision.detect | skipped |
| t2.google-gemini.google-gemini-t2.gemini-3.8-flash.vision.ocr | T2 | google-gemini/google-gemini-t2 | gemini-3.8-flash@google-gemini-t2 | vision.ocr | skipped |
| t2.google-gemini.google-gemini-t2.gemini-3.8-flash.vision.segment | T2 | google-gemini/google-gemini-t2 | gemini-3.8-flash@google-gemini-t2 | vision.segment | skipped |
| t2.google-gemini.google-gemini-t2.gemini-4-argon.llm | T2 | google-gemini/google-gemini-t2 | gemini-4-argon@google-gemini-t2 | llm | skipped |
| t2.google-gemini.google-gemini-t2.gemini-4-argon.vision.caption | T2 | google-gemini/google-gemini-t2 | gemini-4-argon@google-gemini-t2 | vision.caption | skipped |
| t2.google-gemini.google-gemini-t2.gemini-4-argon.vision.detect | T2 | google-gemini/google-gemini-t2 | gemini-4-argon@google-gemini-t2 | vision.detect | skipped |
| t2.google-gemini.google-gemini-t2.gemini-4-argon.vision.ocr | T2 | google-gemini/google-gemini-t2 | gemini-4-argon@google-gemini-t2 | vision.ocr | skipped |
| t2.google-gemini.google-gemini-t2.gemini-4-argon.vision.segment | T2 | google-gemini/google-gemini-t2 | gemini-4-argon@google-gemini-t2 | vision.segment | skipped |
| t2.google-gemini.google-gemini-t2.gemini-embedding-001.embedding.text | T2 | google-gemini/google-gemini-t2 | gemini-embedding-001@google-gemini-t2 | embedding.text | skipped |
| t2.google-gemini.google-gemini-t2.gemini-embedding-2.embedding.multimodal | T2 | google-gemini/google-gemini-t2 | gemini-embedding-2@google-gemini-t2 | embedding.multimodal | skipped |
| t2.google-gemini.google-gemini-t2.gemini-embedding-2.embedding.text | T2 | google-gemini/google-gemini-t2 | gemini-embedding-2@google-gemini-t2 | embedding.text | skipped |
| t2.google-gemini.google-gemini-t2.gemini-nano-banana-2.1.image.img2img | T2 | google-gemini/google-gemini-t2 | gemini-nano-banana-2.1@google-gemini-t2 | image.img2img | skipped |
| t2.google-gemini.google-gemini-t2.gemini-nano-banana-2.1.image.txt2img | T2 | google-gemini/google-gemini-t2 | gemini-nano-banana-2.1@google-gemini-t2 | image.txt2img | skipped |
| t2.google-gemini.google-gemini-t2.gemini-omni-1.1-flash.video.extend | T2 | google-gemini/google-gemini-t2 | gemini-omni-1.1-flash@google-gemini-t2 | video.extend | skipped |
| t2.google-gemini.google-gemini-t2.gemini-omni-1.1-flash.video.img2video | T2 | google-gemini/google-gemini-t2 | gemini-omni-1.1-flash@google-gemini-t2 | video.img2video | skipped |
| t2.google-gemini.google-gemini-t2.gemini-omni-1.1-flash.video.txt2video | T2 | google-gemini/google-gemini-t2 | gemini-omni-1.1-flash@google-gemini-t2 | video.txt2video | skipped |
| t2.google-gemini.google-gemini-t2.gemini-omni-1.1-flash.video.video2video | T2 | google-gemini/google-gemini-t2 | gemini-omni-1.1-flash@google-gemini-t2 | video.video2video | skipped |
| t2.google-gemini.google-gemini-t2.gemini-robotics-er-2-preview.llm | T2 | google-gemini/google-gemini-t2 | gemini-robotics-er-2-preview@google-gemini-t2 | llm | skipped |
| t2.google-gemini.google-gemini-t2.gemini-robotics-er-2-preview.vision.caption | T2 | google-gemini/google-gemini-t2 | gemini-robotics-er-2-preview@google-gemini-t2 | vision.caption | skipped |
| t2.google-gemini.google-gemini-t2.gemini-robotics-er-2-preview.vision.detect | T2 | google-gemini/google-gemini-t2 | gemini-robotics-er-2-preview@google-gemini-t2 | vision.detect | skipped |
| t2.google-gemini.google-gemini-t2.gemini-robotics-er-2-preview.vision.ocr | T2 | google-gemini/google-gemini-t2 | gemini-robotics-er-2-preview@google-gemini-t2 | vision.ocr | skipped |
| t2.google-gemini.google-gemini-t2.gemini-robotics-er-2-preview.vision.segment | T2 | google-gemini/google-gemini-t2 | gemini-robotics-er-2-preview@google-gemini-t2 | vision.segment | skipped |
| t2.google-gemini.google-gemini-t2.gemma-4-26b-a4b-it.llm | T2 | google-gemini/google-gemini-t2 | gemma-4-26b-a4b-it@google-gemini-t2 | llm | skipped |
| t2.google-gemini.google-gemini-t2.gemma-4-26b-a4b-it.vision.caption | T2 | google-gemini/google-gemini-t2 | gemma-4-26b-a4b-it@google-gemini-t2 | vision.caption | skipped |
| t2.google-gemini.google-gemini-t2.gemma-4-26b-a4b-it.vision.ocr | T2 | google-gemini/google-gemini-t2 | gemma-4-26b-a4b-it@google-gemini-t2 | vision.ocr | skipped |
| t2.google-gemini.google-gemini-t2.gemma-4-31b-it.llm | T2 | google-gemini/google-gemini-t2 | gemma-4-31b-it@google-gemini-t2 | llm | skipped |
| t2.google-gemini.google-gemini-t2.gemma-4-31b-it.vision.caption | T2 | google-gemini/google-gemini-t2 | gemma-4-31b-it@google-gemini-t2 | vision.caption | skipped |
| t2.google-gemini.google-gemini-t2.gemma-4-31b-it.vision.ocr | T2 | google-gemini/google-gemini-t2 | gemma-4-31b-it@google-gemini-t2 | vision.ocr | skipped |
| t2.google-gemini.google-gemini-t2.lyria-3-clip-preview.audio.music | T2 | google-gemini/google-gemini-t2 | lyria-3-clip-preview@google-gemini-t2 | audio.music | skipped |
| t2.google-gemini.google-gemini-t2.lyria-3-pro-preview.audio.music | T2 | google-gemini/google-gemini-t2 | lyria-3-pro-preview@google-gemini-t2 | audio.music | skipped |
| t2.google-gemini.google-gemini-t2.lyria-3.5.audio.music | T2 | google-gemini/google-gemini-t2 | lyria-3.5@google-gemini-t2 | audio.music | skipped |
| t2.google-gemini.google-gemini-t2.veo-3.1-fast-generate-preview.video.extend | T2 | google-gemini/google-gemini-t2 | veo-3.1-fast-generate-preview@google-gemini-t2 | video.extend | skipped |
| t2.google-gemini.google-gemini-t2.veo-3.1-fast-generate-preview.video.img2video | T2 | google-gemini/google-gemini-t2 | veo-3.1-fast-generate-preview@google-gemini-t2 | video.img2video | skipped |
| t2.google-gemini.google-gemini-t2.veo-3.1-fast-generate-preview.video.txt2video | T2 | google-gemini/google-gemini-t2 | veo-3.1-fast-generate-preview@google-gemini-t2 | video.txt2video | skipped |
| t2.google-gemini.google-gemini-t2.veo-3.1-generate-preview.video.extend | T2 | google-gemini/google-gemini-t2 | veo-3.1-generate-preview@google-gemini-t2 | video.extend | skipped |
| t2.google-gemini.google-gemini-t2.veo-3.1-generate-preview.video.img2video | T2 | google-gemini/google-gemini-t2 | veo-3.1-generate-preview@google-gemini-t2 | video.img2video | skipped |
| t2.google-gemini.google-gemini-t2.veo-3.1-generate-preview.video.txt2video | T2 | google-gemini/google-gemini-t2 | veo-3.1-generate-preview@google-gemini-t2 | video.txt2video | skipped |
| t2.google-gemini.google-gemini-t2.veo-3.1-lite-generate-preview.video.img2video | T2 | google-gemini/google-gemini-t2 | veo-3.1-lite-generate-preview@google-gemini-t2 | video.img2video | skipped |
| t2.google-gemini.google-gemini-t2.veo-3.1-lite-generate-preview.video.txt2video | T2 | google-gemini/google-gemini-t2 | veo-3.1-lite-generate-preview@google-gemini-t2 | video.txt2video | skipped |

## Physical model coverage

| Provider | Inventory model | Physical model | Decision | Reason | Retained representative |
|---|---|---|---|---|---|
| google-gemini/google-gemini-t2 | antigravity-preview-05-2026 | antigravity-preview-05-2026 | filtered | unsupported_canonical_protocol | - |
| google-gemini/google-gemini-t2 | antigravity-preview-09-2026 | antigravity-preview-09-2026 | filtered | unsupported_canonical_protocol | - |
| google-gemini/google-gemini-t2 | antigravity-preview-latest | antigravity-preview-latest | filtered | logical_alias | - |
| google-gemini/google-gemini-t2 | aqa | aqa | filtered | unsupported_canonical_protocol | - |
| google-gemini/google-gemini-t2 | deep-research-max-preview-04-2026 | deep-research-max-preview-04-2026 | filtered | unsupported_canonical_protocol | - |
| google-gemini/google-gemini-t2 | deep-research-preview-04-2026 | deep-research-preview-04-2026 | filtered | unsupported_canonical_protocol | - |
| google-gemini/google-gemini-t2 | deep-research-pro-preview-12-2025 | deep-research-pro-preview-12-2025 | filtered | unsupported_canonical_protocol | - |
| google-gemini/google-gemini-t2 | gemini-2.5-computer-use-preview-10-2025 | gemini-2.5-computer-use-preview-10-2025 | filtered | deprecated_or_retiring | - |
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
| google-gemini/google-gemini-t2 | gemini-3.1-flash-lite-preview | gemini-3.1-flash-lite-preview | filtered | deprecated_or_retiring | - |
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
| google-gemini/google-gemini-t2 | gemini-3.8-live-extended-thinking | gemini-3.8-live-extended-thinking | filtered | unsupported_canonical_protocol | - |
| google-gemini/google-gemini-t2 | gemini-3.8-live | gemini-3.8-live | filtered | unsupported_canonical_protocol | - |
| google-gemini/google-gemini-t2 | gemini-4-argon | gemini-4-argon | included | physical model | - |
| google-gemini/google-gemini-t2 | gemini-embedding-001 | gemini-embedding-001 | included | physical model | - |
| google-gemini/google-gemini-t2 | gemini-embedding-2-preview | gemini-embedding-2-preview | filtered | deprecated_or_retiring | - |
| google-gemini/google-gemini-t2 | gemini-embedding-2 | gemini-embedding-2 | included | physical model | - |
| google-gemini/google-gemini-t2 | gemini-flash-latest | gemini-flash-latest | filtered | logical_alias | - |
| google-gemini/google-gemini-t2 | gemini-flash-lite-latest | gemini-flash-lite-latest | filtered | logical_alias | - |
| google-gemini/google-gemini-t2 | gemini-nano-banana-2.1 | gemini-nano-banana-2.1 | included | physical model | - |
| google-gemini/google-gemini-t2 | gemini-omni-1.1-flash | gemini-omni-1.1-flash | included | physical model | - |
| google-gemini/google-gemini-t2 | gemini-omni-flash-preview | gemini-omni-flash-preview | filtered | deprecated_or_retiring | - |
| google-gemini/google-gemini-t2 | gemini-pro-latest | gemini-pro-latest | filtered | logical_alias | - |
| google-gemini/google-gemini-t2 | gemini-robotics-er-2-preview | gemini-robotics-er-2-preview | included | physical model | - |
| google-gemini/google-gemini-t2 | gemini-robotics-er-2-streaming-preview | gemini-robotics-er-2-streaming-preview | filtered | not_physical_model | - |
| google-gemini/google-gemini-t2 | gemma-4-26b-a4b-it | gemma-4-26b-a4b-it | included | physical model | - |
| google-gemini/google-gemini-t2 | gemma-4-31b-it | gemma-4-31b-it | included | physical model | - |
| google-gemini/google-gemini-t2 | lyria-3-clip-preview | lyria-3-clip-preview | included | physical model | - |
| google-gemini/google-gemini-t2 | lyria-3-pro-preview | lyria-3-pro-preview | included | physical model | - |
| google-gemini/google-gemini-t2 | lyria-3.5 | lyria-3.5 | included | physical model | - |
| google-gemini/google-gemini-t2 | lyria-realtime-exp | lyria-realtime-exp | filtered | unsupported_canonical_protocol | - |
| google-gemini/google-gemini-t2 | nano-banana-pro-preview | nano-banana-pro-preview | filtered | logical_alias | - |
| google-gemini/google-gemini-t2 | veo-3.1-fast-generate-preview | veo-3.1-fast-generate-preview | included | physical model | - |
| google-gemini/google-gemini-t2 | veo-3.1-generate-preview | veo-3.1-generate-preview | included | physical model | - |
| google-gemini/google-gemini-t2 | veo-3.1-lite-generate-preview | veo-3.1-lite-generate-preview | included | physical model | - |
| google-gemini/google-gemini-t2 | gemini-2.5-flash-image | gemini-2.5-flash-image | included | physical model | - |
| google-gemini/google-gemini-t2 | gemini-2.5-flash-lite | gemini-2.5-flash-lite | included | physical model | - |
| google-gemini/google-gemini-t2 | gemini-2.5-flash-preview-tts | gemini-2.5-flash-preview-tts | included | physical model | - |
| google-gemini/google-gemini-t2 | gemini-2.5-flash | gemini-2.5-flash | included | physical model | - |
| google-gemini/google-gemini-t2 | gemini-2.5-pro-preview-tts | gemini-2.5-pro-preview-tts | included | physical model | - |
| google-gemini/google-gemini-t2 | gemini-2.5-pro | gemini-2.5-pro | included | physical model | - |
| google-gemini/google-gemini-t2 | gemini-3-flash-preview | gemini-3-flash-preview | included | physical model | - |
| google-gemini/google-gemini-t2 | gemini-3-pro-image | gemini-3-pro-image | included | physical model | - |
| google-gemini/google-gemini-t2 | gemini-3.1-flash-image | gemini-3.1-flash-image | included | physical model | - |
| google-gemini/google-gemini-t2 | gemini-3.1-flash-lite-image | gemini-3.1-flash-lite-image | included | physical model | - |
| google-gemini/google-gemini-t2 | gemini-3.1-flash-lite | gemini-3.1-flash-lite | included | physical model | - |
| google-gemini/google-gemini-t2 | gemini-3.1-flash-tts-preview | gemini-3.1-flash-tts-preview | included | physical model | - |
| google-gemini/google-gemini-t2 | gemini-3.1-pro-preview-customtools | gemini-3.1-pro-preview-customtools | included | physical model | - |
| google-gemini/google-gemini-t2 | gemini-3.1-pro-preview | gemini-3.1-pro-preview | included | physical model | - |
| google-gemini/google-gemini-t2 | gemini-3.5-flash-lite | gemini-3.5-flash-lite | included | physical model | - |
| google-gemini/google-gemini-t2 | gemini-3.5-flash | gemini-3.5-flash | included | physical model | - |
| google-gemini/google-gemini-t2 | gemini-3.5-transcribe | gemini-3.5-transcribe | included | physical model | - |
| google-gemini/google-gemini-t2 | gemini-3.6-flash | gemini-3.6-flash | included | physical model | - |
| google-gemini/google-gemini-t2 | gemini-3.7-flash | gemini-3.7-flash | included | physical model | - |
| google-gemini/google-gemini-t2 | gemini-3.8-flash-lite-tts | gemini-3.8-flash-lite-tts | included | physical model | - |
| google-gemini/google-gemini-t2 | gemini-3.8-flash-tts | gemini-3.8-flash-tts | included | physical model | - |
| google-gemini/google-gemini-t2 | gemini-3.8-flash | gemini-3.8-flash | included | physical model | - |
| google-gemini/google-gemini-t2 | gemini-4-argon | gemini-4-argon | included | physical model | - |
| google-gemini/google-gemini-t2 | gemini-embedding-001 | gemini-embedding-001 | included | physical model | - |
| google-gemini/google-gemini-t2 | gemini-embedding-2-preview | gemini-embedding-2-preview | filtered | deprecated_or_retiring | - |
| google-gemini/google-gemini-t2 | gemini-embedding-2 | gemini-embedding-2 | included | physical model | - |
| google-gemini/google-gemini-t2 | gemini-nano-banana-2.1 | gemini-nano-banana-2.1 | included | physical model | - |
| google-gemini/google-gemini-t2 | gemini-omni-1.1-flash | gemini-omni-1.1-flash | included | physical model | - |
| google-gemini/google-gemini-t2 | gemini-robotics-er-2-preview | gemini-robotics-er-2-preview | included | physical model | - |
| google-gemini/google-gemini-t2 | gemma-4-26b-a4b-it | gemma-4-26b-a4b-it | included | physical model | - |
| google-gemini/google-gemini-t2 | gemma-4-31b-it | gemma-4-31b-it | included | physical model | - |
| google-gemini/google-gemini-t2 | lyria-3-clip-preview | lyria-3-clip-preview | included | physical model | - |
| google-gemini/google-gemini-t2 | lyria-3-pro-preview | lyria-3-pro-preview | included | physical model | - |
| google-gemini/google-gemini-t2 | lyria-3.5 | lyria-3.5 | included | physical model | - |
| google-gemini/google-gemini-t2 | veo-3.1-fast-generate-preview | veo-3.1-fast-generate-preview | included | physical model | - |
| google-gemini/google-gemini-t2 | veo-3.1-generate-preview | veo-3.1-generate-preview | included | physical model | - |
| google-gemini/google-gemini-t2 | veo-3.1-lite-generate-preview | veo-3.1-lite-generate-preview | included | physical model | - |

## Confirmed product defects

- **defect.aicc.t2.preflight.baseline_mismatch.1** (AICC, baseline_mismatch): expected official Provider capability, AICC inventory declaration, and adapter support are bidirectionally consistent; observed official_supported_but_aicc_missing gemini-3.1-pro-preview-customtools@google-gemini-t2 audio.asr. Evidence: cases/t2.preflight.baseline_mismatch.1.json, effective_baseline.json
- **defect.aicc.t2.preflight.baseline_mismatch.2** (AICC, baseline_mismatch): expected official Provider capability, AICC inventory declaration, and adapter support are bidirectionally consistent; observed official_supported_but_aicc_missing gemini-4-argon@google-gemini-t2 audio.asr. Evidence: cases/t2.preflight.baseline_mismatch.2.json, effective_baseline.json
- **defect.aicc.t2.preflight.baseline_mismatch.3** (AICC, baseline_mismatch): expected official Provider capability, AICC inventory declaration, and adapter support are bidirectionally consistent; observed official_not_supported_but_aicc_advertised veo-3.1-fast-generate-preview@google-gemini-t2 video.video2video. Evidence: cases/t2.preflight.baseline_mismatch.3.json, effective_baseline.json

## Cleanup

Status: passed

- removed 0 uploaded fixture object(s)
- removed 0 generated output object(s)
- temporary Provider credentials restored after testing (google-gemini)
