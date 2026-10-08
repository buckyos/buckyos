# AICC E2E Acceptance Report

- Run: `aicc-t1-2026-10-08T04-31-37-310Z-9bea28a0`
- Commit: `7612d141be9938f5d1c4a89b41f20ae80459c1c1`
- Capability baseline: `T1-mock`
- Real model calls: 0/0
- Planned maximum cost: $0.000000
- Known actual / unknown estimated exposure: $0.000000 / $0.000000
- Total exposure / budget: $0.000000 / $0.000000
- Unknown-cost calls: 0; budget exceeded: false
- Results: passed=178, failed=0, provider_restricted=0, skipped=0, not_applicable=0, review=0
- Manifest coverage: 130/130 (100.00%); passed=130, failed=0
- T1 requirement branches: 127/127 (100.00%); passed=127, failed=0, skipped=0
- T1 route exposure: 48/48 passed; failed=0, skipped=0

## Cases

| Case | Layer | Provider | Model | API | Status |
|---|---|---|---|---|---|
| t1.route.exposure.minimax.minimax-m2-.llm | T1 | minimax/dv-minimax-aicc-t1-2026-10-08T04-31-37-310Z-9bea28a0 | - | llm | passed |
| t1.route.exposure.minimax.minimax-m3-.llm | T1 | minimax/dv-minimax-aicc-t1-2026-10-08T04-31-37-310Z-9bea28a0 | - | llm | passed |
| t1.route.exposure.minimax.minimax-m3-.vision.caption | T1 | minimax/dv-minimax-aicc-t1-2026-10-08T04-31-37-310Z-9bea28a0 | - | vision.caption | passed |
| t1.route.exposure.minimax.minimax-m3-.vision.ocr | T1 | minimax/dv-minimax-aicc-t1-2026-10-08T04-31-37-310Z-9bea28a0 | - | vision.ocr | passed |
| t1.route.exposure.claude.claude--.llm | T1 | claude/dv-claude-aicc-t1-2026-10-08T04-31-37-310Z-9bea28a0 | - | llm | passed |
| t1.route.exposure.claude.claude--.vision.caption | T1 | claude/dv-claude-aicc-t1-2026-10-08T04-31-37-310Z-9bea28a0 | - | vision.caption | passed |
| t1.route.exposure.claude.claude--.vision.ocr | T1 | claude/dv-claude-aicc-t1-2026-10-08T04-31-37-310Z-9bea28a0 | - | vision.ocr | passed |
| t1.route.exposure.openrouter.cohere-rerank--.rerank | T1 | openrouter/dv-openrouter-aicc-t1-2026-10-08T04-31-37-310Z-9bea28a0 | - | rerank | passed |
| t1.route.exposure.doubao-speech.doubao-seed-asr-2.0-fast.audio.asr | T1 | doubao-speech/dv-doubao-speech-aicc-t1-2026-10-08T04-31-37-310Z-9bea28a0 | - | audio.asr | passed |
| t1.route.exposure.doubao-speech.doubao-seed-asr-2.0.audio.asr | T1 | doubao-speech/dv-doubao-speech-aicc-t1-2026-10-08T04-31-37-310Z-9bea28a0 | - | audio.asr | passed |
| t1.route.exposure.doubao-speech.doubao-seed-tts-2.0.audio.tts | T1 | doubao-speech/dv-doubao-speech-aicc-t1-2026-10-08T04-31-37-310Z-9bea28a0 | - | audio.tts | passed |
| t1.route.exposure.fal.fal-ai-deepfilternet3.audio.enhance | T1 | fal/dv-fal-aicc-t1-2026-10-08T04-31-37-310Z-9bea28a0 | - | audio.enhance | passed |
| t1.route.exposure.fal.fal-ai-esrgan.image.upscale | T1 | fal/dv-fal-aicc-t1-2026-10-08T04-31-37-310Z-9bea28a0 | - | image.upscale | passed |
| t1.route.exposure.fal.fal-ai-imageutils-rembg.image.bg_remove | T1 | fal/dv-fal-aicc-t1-2026-10-08T04-31-37-310Z-9bea28a0 | - | image.bg_remove | passed |
| t1.route.exposure.fal.fal-ai-video-upscaler.video.upscale | T1 | fal/dv-fal-aicc-t1-2026-10-08T04-31-37-310Z-9bea28a0 | - | video.upscale | passed |
| t1.route.exposure.google-gemini.-tts-.audio.tts | T1 | google-gemini/dv-gemini-aicc-t1-2026-10-08T04-31-37-310Z-9bea28a0 | - | audio.tts | passed |
| t1.route.exposure.google-gemini.gemini--.audio.asr | T1 | google-gemini/dv-gemini-aicc-t1-2026-10-08T04-31-37-310Z-9bea28a0 | - | audio.asr | passed |
| t1.route.exposure.google-gemini.gemini--.llm | T1 | google-gemini/dv-gemini-aicc-t1-2026-10-08T04-31-37-310Z-9bea28a0 | - | llm | passed |
| t1.route.exposure.google-gemini.gemini--.vision.caption | T1 | google-gemini/dv-gemini-aicc-t1-2026-10-08T04-31-37-310Z-9bea28a0 | - | vision.caption | passed |
| t1.route.exposure.google-gemini.gemini--.vision.detect | T1 | google-gemini/dv-gemini-aicc-t1-2026-10-08T04-31-37-310Z-9bea28a0 | - | vision.detect | passed |
| t1.route.exposure.google-gemini.gemini--.vision.ocr | T1 | google-gemini/dv-gemini-aicc-t1-2026-10-08T04-31-37-310Z-9bea28a0 | - | vision.ocr | passed |
| t1.route.exposure.google-gemini.gemini--.vision.segment | T1 | google-gemini/dv-gemini-aicc-t1-2026-10-08T04-31-37-310Z-9bea28a0 | - | vision.segment | passed |
| t1.route.exposure.google-gemini.gemini-embedding-2-.embedding.multimodal | T1 | google-gemini/dv-gemini-aicc-t1-2026-10-08T04-31-37-310Z-9bea28a0 | - | embedding.multimodal | passed |
| t1.route.exposure.google-gemini.gemini-embedding-2-.embedding.text | T1 | google-gemini/dv-gemini-aicc-t1-2026-10-08T04-31-37-310Z-9bea28a0 | - | embedding.text | passed |
| t1.route.exposure.google-gemini.gemini-omni--.video.extend | T1 | google-gemini/dv-gemini-aicc-t1-2026-10-08T04-31-37-310Z-9bea28a0 | - | video.extend | passed |
| t1.route.exposure.google-gemini.gemini-omni--.video.img2video | T1 | google-gemini/dv-gemini-aicc-t1-2026-10-08T04-31-37-310Z-9bea28a0 | - | video.img2video | passed |
| t1.route.exposure.google-gemini.gemini-omni--.video.txt2video | T1 | google-gemini/dv-gemini-aicc-t1-2026-10-08T04-31-37-310Z-9bea28a0 | - | video.txt2video | passed |
| t1.route.exposure.google-gemini.gemini-omni--.video.video2video | T1 | google-gemini/dv-gemini-aicc-t1-2026-10-08T04-31-37-310Z-9bea28a0 | - | video.video2video | passed |
| t1.route.exposure.google-gemini.lyria--.audio.music | T1 | google-gemini/dv-gemini-aicc-t1-2026-10-08T04-31-37-310Z-9bea28a0 | - | audio.music | passed |
| t1.route.exposure.google-gemini.veo-3.1--.video.extend | T1 | google-gemini/dv-gemini-aicc-t1-2026-10-08T04-31-37-310Z-9bea28a0 | - | video.extend | passed |
| t1.route.exposure.google-gemini.veo-3.1--.video.img2video | T1 | google-gemini/dv-gemini-aicc-t1-2026-10-08T04-31-37-310Z-9bea28a0 | - | video.img2video | passed |
| t1.route.exposure.google-gemini.veo-3.1--.video.txt2video | T1 | google-gemini/dv-gemini-aicc-t1-2026-10-08T04-31-37-310Z-9bea28a0 | - | video.txt2video | passed |
| t1.route.exposure.openai.--tts-.audio.tts | T1 | openai/dv-openai-a-aicc-t1-2026-10-08T04-31-37-310Z-9bea28a0 | - | audio.tts | passed |
| t1.route.exposure.openai.gpt-5-.llm | T1 | openai/dv-openai-a-aicc-t1-2026-10-08T04-31-37-310Z-9bea28a0 | - | llm | passed |
| t1.route.exposure.openai.gpt-5-.vision.caption | T1 | openai/dv-openai-a-aicc-t1-2026-10-08T04-31-37-310Z-9bea28a0 | - | vision.caption | passed |
| t1.route.exposure.openai.gpt-5-.vision.ocr | T1 | openai/dv-openai-a-aicc-t1-2026-10-08T04-31-37-310Z-9bea28a0 | - | vision.ocr | passed |
| t1.route.exposure.openai.gpt-5.6-.agent.computer_use | T1 | openai/dv-openai-a-aicc-t1-2026-10-08T04-31-37-310Z-9bea28a0 | - | agent.computer_use | passed |
| t1.route.exposure.openai.gpt-5.6-.image.img2img | T1 | openai/dv-openai-a-aicc-t1-2026-10-08T04-31-37-310Z-9bea28a0 | - | image.img2img | passed |
| t1.route.exposure.openai.gpt-5.6-.image.txt2img | T1 | openai/dv-openai-a-aicc-t1-2026-10-08T04-31-37-310Z-9bea28a0 | - | image.txt2img | passed |
| t1.route.exposure.openai.gpt-5.6-.llm | T1 | openai/dv-openai-a-aicc-t1-2026-10-08T04-31-37-310Z-9bea28a0 | - | llm | passed |
| t1.route.exposure.openai.gpt-5.6-.vision.caption | T1 | openai/dv-openai-a-aicc-t1-2026-10-08T04-31-37-310Z-9bea28a0 | - | vision.caption | passed |
| t1.route.exposure.openai.gpt-5.6-.vision.ocr | T1 | openai/dv-openai-a-aicc-t1-2026-10-08T04-31-37-310Z-9bea28a0 | - | vision.ocr | passed |
| t1.route.exposure.openai.gpt-image--.image.img2img | T1 | openai/dv-openai-a-aicc-t1-2026-10-08T04-31-37-310Z-9bea28a0 | - | image.img2img | passed |
| t1.route.exposure.openai.gpt-image--.image.inpaint | T1 | openai/dv-openai-a-aicc-t1-2026-10-08T04-31-37-310Z-9bea28a0 | - | image.inpaint | passed |
| t1.route.exposure.openai.gpt-image--.image.txt2img | T1 | openai/dv-openai-a-aicc-t1-2026-10-08T04-31-37-310Z-9bea28a0 | - | image.txt2img | passed |
| t1.route.exposure.openai.-transcribe-.audio.asr | T1 | openai/dv-openai-a-aicc-t1-2026-10-08T04-31-37-310Z-9bea28a0 | - | audio.asr | passed |
| t1.route.exposure.openai.text-embedding--.embedding.text | T1 | openai/dv-openai-a-aicc-t1-2026-10-08T04-31-37-310Z-9bea28a0 | - | embedding.text | passed |
| t1.route.exposure.typesafe.jev-1.13.0.decision | T1 | typesafe/dv-typesafe-aicc-t1-2026-10-08T04-31-37-310Z-9bea28a0 | - | decision | passed |
| t1.route.api_type.llm.chat.completions.create | T1 | custom/dv-custom-openai-aicc-t1-2026-10-08T04-31-37-310Z-9bea28a0 | gpt-5.6@dv-custom-openai-aicc-t1-2026-10-08T04-31-37-310Z-9bea28a0 | llm | passed |
| t1.route.api_type.embedding.text.embedding.text | T1 | openai/dv-openai-a-aicc-t1-2026-10-08T04-31-37-310Z-9bea28a0 | text-embedding-3-small@dv-openai-a-aicc-t1-2026-10-08T04-31-37-310Z-9bea28a0 | embedding.text | passed |
| t1.route.api_type.embedding.multimodal.embedding.multimodal | T1 | google-gemini/dv-gemini-aicc-t1-2026-10-08T04-31-37-310Z-9bea28a0 | gemini-embedding-2-preview@dv-gemini-aicc-t1-2026-10-08T04-31-37-310Z-9bea28a0 | embedding.multimodal | passed |
| t1.route.api_type.decision.decision.evaluate | T1 | typesafe/dv-typesafe-aicc-t1-2026-10-08T04-31-37-310Z-9bea28a0 | jev-1.13.0@dv-typesafe-aicc-t1-2026-10-08T04-31-37-310Z-9bea28a0 | decision | passed |
| t1.route.api_type.rerank.rerank | T1 | openrouter/dv-openrouter-aicc-t1-2026-10-08T04-31-37-310Z-9bea28a0 | cohere/rerank-v3.5@dv-openrouter-aicc-t1-2026-10-08T04-31-37-310Z-9bea28a0 | rerank | passed |
| t1.route.api_type.image.txt2img.images.generate | T1 | openai/dv-openai-a-aicc-t1-2026-10-08T04-31-37-310Z-9bea28a0 | gpt-image-2@dv-openai-a-aicc-t1-2026-10-08T04-31-37-310Z-9bea28a0 | image.txt2img | passed |
| t1.route.api_type.image.img2img.image.img2img | T1 | openai/dv-openai-a-aicc-t1-2026-10-08T04-31-37-310Z-9bea28a0 | gpt-image-2@dv-openai-a-aicc-t1-2026-10-08T04-31-37-310Z-9bea28a0 | image.img2img | passed |
| t1.route.api_type.image.inpaint.image.inpaint | T1 | openai/dv-openai-a-aicc-t1-2026-10-08T04-31-37-310Z-9bea28a0 | gpt-image-2@dv-openai-a-aicc-t1-2026-10-08T04-31-37-310Z-9bea28a0 | image.inpaint | passed |
| t1.route.api_type.image.upscale.image.upscale | T1 | fal/dv-fal-aicc-t1-2026-10-08T04-31-37-310Z-9bea28a0 | fal-ai/esrgan@dv-fal-aicc-t1-2026-10-08T04-31-37-310Z-9bea28a0 | image.upscale | passed |
| t1.route.api_type.image.bg_remove.image.bg_remove | T1 | fal/dv-fal-aicc-t1-2026-10-08T04-31-37-310Z-9bea28a0 | fal-ai/imageutils/rembg@dv-fal-aicc-t1-2026-10-08T04-31-37-310Z-9bea28a0 | image.bg_remove | passed |
| t1.route.api_type.vision.ocr.vision.ocr | T1 | minimax/dv-minimax-aicc-t1-2026-10-08T04-31-37-310Z-9bea28a0 | MiniMax-M3@dv-minimax-aicc-t1-2026-10-08T04-31-37-310Z-9bea28a0 | vision.ocr | passed |
| t1.route.api_type.vision.caption.vision.caption | T1 | minimax/dv-minimax-aicc-t1-2026-10-08T04-31-37-310Z-9bea28a0 | MiniMax-M3@dv-minimax-aicc-t1-2026-10-08T04-31-37-310Z-9bea28a0 | vision.caption | passed |
| t1.route.api_type.vision.detect.vision.detect | T1 | google-gemini/dv-gemini-aicc-t1-2026-10-08T04-31-37-310Z-9bea28a0 | gemini-2.5-flash:reasoning-high@dv-gemini-aicc-t1-2026-10-08T04-31-37-310Z-9bea28a0 | vision.detect | passed |
| t1.route.api_type.vision.segment.vision.segment | T1 | google-gemini/dv-gemini-aicc-t1-2026-10-08T04-31-37-310Z-9bea28a0 | gemini-2.5-flash:reasoning-high@dv-gemini-aicc-t1-2026-10-08T04-31-37-310Z-9bea28a0 | vision.segment | passed |
| t1.route.api_type.audio.tts.audio.tts | T1 | openai/dv-openai-a-aicc-t1-2026-10-08T04-31-37-310Z-9bea28a0 | gpt-4o-mini-tts@dv-openai-a-aicc-t1-2026-10-08T04-31-37-310Z-9bea28a0 | audio.tts | passed |
| t1.route.api_type.audio.asr.audio.asr | T1 | openai/dv-openai-a-aicc-t1-2026-10-08T04-31-37-310Z-9bea28a0 | gpt-transcribe@dv-openai-a-aicc-t1-2026-10-08T04-31-37-310Z-9bea28a0 | audio.asr | passed |
| t1.route.api_type.audio.music.audio.music | T1 | google-gemini/dv-gemini-aicc-t1-2026-10-08T04-31-37-310Z-9bea28a0 | lyria-3-clip-preview@dv-gemini-aicc-t1-2026-10-08T04-31-37-310Z-9bea28a0 | audio.music | passed |
| t1.route.api_type.audio.enhance.audio.enhance | T1 | fal/dv-fal-aicc-t1-2026-10-08T04-31-37-310Z-9bea28a0 | fal-ai/deepfilternet3@dv-fal-aicc-t1-2026-10-08T04-31-37-310Z-9bea28a0 | audio.enhance | passed |
| t1.route.api_type.video.txt2video.video.txt2video | T1 | google-gemini/dv-gemini-aicc-t1-2026-10-08T04-31-37-310Z-9bea28a0 | gemini-omni-1.1-flash@dv-gemini-aicc-t1-2026-10-08T04-31-37-310Z-9bea28a0 | video.txt2video | passed |
| t1.route.api_type.video.img2video.video.img2video | T1 | google-gemini/dv-gemini-aicc-t1-2026-10-08T04-31-37-310Z-9bea28a0 | gemini-omni-1.1-flash@dv-gemini-aicc-t1-2026-10-08T04-31-37-310Z-9bea28a0 | video.img2video | passed |
| t1.route.api_type.video.video2video.video.video2video | T1 | google-gemini/dv-gemini-aicc-t1-2026-10-08T04-31-37-310Z-9bea28a0 | gemini-omni-1.1-flash@dv-gemini-aicc-t1-2026-10-08T04-31-37-310Z-9bea28a0 | video.video2video | passed |
| t1.route.api_type.video.extend.video.extend | T1 | google-gemini/dv-gemini-aicc-t1-2026-10-08T04-31-37-310Z-9bea28a0 | gemini-omni-1.1-flash@dv-gemini-aicc-t1-2026-10-08T04-31-37-310Z-9bea28a0 | video.extend | passed |
| t1.route.api_type.video.upscale.video.upscale | T1 | fal/dv-fal-aicc-t1-2026-10-08T04-31-37-310Z-9bea28a0 | fal-ai/video-upscaler@dv-fal-aicc-t1-2026-10-08T04-31-37-310Z-9bea28a0 | video.upscale | passed |
| t1.route.api_type.agent.computer_use.agent.computer_use | T1 | openai/dv-openai-a-aicc-t1-2026-10-08T04-31-37-310Z-9bea28a0 | gpt-5.6:reasoning-high@dv-openai-a-aicc-t1-2026-10-08T04-31-37-310Z-9bea28a0 | agent.computer_use | passed |
| t1.custom.openai | T1 | custom-openai/dv-custom-openai-aicc-t1-2026-10-08T04-31-37-310Z-9bea28a0 | gpt-5.6@dv-custom-openai-aicc-t1-2026-10-08T04-31-37-310Z-9bea28a0 | llm | passed |
| t1.custom.claude | T1 | custom-claude/dv-custom-claude-aicc-t1-2026-10-08T04-31-37-310Z-9bea28a0 | claude-sonnet-5@dv-custom-claude-aicc-t1-2026-10-08T04-31-37-310Z-9bea28a0 | llm | passed |
| t1.custom.gemini | T1 | custom-gemini/dv-custom-gemini-aicc-t1-2026-10-08T04-31-37-310Z-9bea28a0 | gemini-3.5-flash@dv-custom-gemini-aicc-t1-2026-10-08T04-31-37-310Z-9bea28a0 | llm | passed |
| t1.route.logical_model_selects_candidate | T1 | -/dv-openai-a-aicc-t1-2026-10-08T04-31-37-310Z-9bea28a0 | gpt-5.6:reasoning-high@dv-openai-a-aicc-t1-2026-10-08T04-31-37-310Z-9bea28a0 | llm | passed |
| t1.route.provider_allow | T1 | -/dv-openai-a-aicc-t1-2026-10-08T04-31-37-310Z-9bea28a0 | gpt-5.6:reasoning-high@dv-openai-a-aicc-t1-2026-10-08T04-31-37-310Z-9bea28a0 | llm | passed |
| t1.route.provider_deny | T1 | -/dv-openai-b-aicc-t1-2026-10-08T04-31-37-310Z-9bea28a0 | gpt-5.6:reasoning-high@dv-openai-b-aicc-t1-2026-10-08T04-31-37-310Z-9bea28a0 | llm | passed |
| t1.route.missing_provider_instance | T1 | -/- | - | llm | passed |
| t1.route.local_only | T1 | -/- | - | llm | passed |
| t1.route.invalid_exact_model | T1 | -/- | - | llm | passed |
| t1.route.invalid_logical_path | T1 | -/- | - | llm | passed |
| t1.route.fallback_api_type_boundary | T1 | -/- | - | embedding.text | passed |
| t1.route.version_exact_rule | T1 | -/dv-openai-a-aicc-t1-2026-10-08T04-31-37-310Z-9bea28a0 | gpt-image-2@dv-openai-a-aicc-t1-2026-10-08T04-31-37-310Z-9bea28a0 | image.txt2img | passed |
| t1.route.version_pattern_rule | T1 | -/- | - | llm | passed |
| t1.route.version_default_rule | T1 | -/- | - | llm | passed |
| t1.route.legal_missing_model | T1 | -/- | - | llm | passed |
| t1.route.disabled_model | T1 | -/- | - | llm | passed |
| t1.route.unmounted_model | T1 | -/- | - | llm | passed |
| t1.route.corrupt_metadata | T1 | -/- | - | llm | passed |
| t1.route.privacy_boundary | T1 | -/- | - | llm | passed |
| t1.route.budget_filter | T1 | -/- | - | llm | passed |
| t1.route.context_limit_filter | T1 | -/- | - | llm | passed |
| t1.route.output_limit_filter | T1 | -/- | - | llm | passed |
| t1.route.locked_policy_cannot_override | T1 | -/- | - | llm | passed |
| t1.route.missing_metadata_is_conservative | T1 | -/- | - | llm | passed |
| t1.route.auto_mount_admission | T1 | -/- | - | llm | passed |
| t1.route.manual_mount_requires_mapping | T1 | -/- | - | llm | passed |
| t1.route.min_line_admission | T1 | -/- | - | llm | passed |
| t1.route.disable_line_applied | T1 | -/- | - | llm | passed |
| t1.route.global_exact_model_weight | T1 | -/- | - | llm | passed |
| t1.route.logical_exact_model_weight | T1 | -/- | - | llm | passed |
| t1.route.provider_instance_weight | T1 | -/- | - | llm | passed |
| t1.route.system_config_then_request_overlay | T1 | -/- | - | llm | passed |
| t1.route.offline_model | T1 | -/- | - | llm | passed |
| t1.route.health_filter | T1 | -/- | - | llm | passed |
| t1.route.quota_filter | T1 | -/- | - | llm | passed |
| t1.route.strict_no_fallback | T1 | -/- | - | llm | passed |
| t1.route.parent_fallback | T1 | -/- | - | llm | passed |
| t1.route.target_logical_fallback | T1 | -/- | - | llm | passed |
| t1.route.target_exact_fallback | T1 | -/- | - | llm | passed |
| t1.route.exact_default_no_fallback | T1 | -/- | - | llm | passed |
| t1.route.fallback_loop | T1 | -/- | - | llm | passed |
| t1.route.fallback_max_depth | T1 | -/- | - | llm | passed |
| t1.scheduler.profile.cost_first | T1 | -/- | - | llm | passed |
| t1.scheduler.profile.latency_first | T1 | -/- | - | llm | passed |
| t1.scheduler.profile.quality_first | T1 | -/- | - | llm | passed |
| t1.scheduler.profile.balanced | T1 | -/- | - | llm | passed |
| t1.scheduler.profile.local_first | T1 | -/- | - | llm | passed |
| t1.scheduler.profile.strict_local | T1 | -/- | - | llm | passed |
| t1.route.exact_model_hits_instance | T1 | openai/dv-openai-a-aicc-t1-2026-10-08T04-31-37-310Z-9bea28a0 | gpt-5.6@dv-openai-a-aicc-t1-2026-10-08T04-31-37-310Z-9bea28a0 | llm | passed |
| t1.route.metadata_variant_expands_exact_model | T1 | openai/dv-openai-a-aicc-t1-2026-10-08T04-31-37-310Z-9bea28a0 | gpt-5.6:reasoning-high@dv-openai-a-aicc-t1-2026-10-08T04-31-37-310Z-9bea28a0 | llm | passed |
| t1.embedding.large_batch_artifact | T1 | openai/dv-openai-a-aicc-t1-2026-10-08T04-31-37-310Z-9bea28a0 | text-embedding-3-small@dv-openai-a-aicc-t1-2026-10-08T04-31-37-310Z-9bea28a0 | embedding.text | passed |
| t1.embedding.space_mismatch_rejected | T1 | openai/dv-openai-a-aicc-t1-2026-10-08T04-31-37-310Z-9bea28a0 | text-embedding-3-small@dv-openai-a-aicc-t1-2026-10-08T04-31-37-310Z-9bea28a0 | embedding.text | passed |
| t1.history.same_session_reuses_exact_model | T1 | -/- | - | llm | passed |
| t1.history.hard_constraint_overrides.provider_denied | T1 | -/- | - | llm | passed |
| t1.history.hard_constraint_overrides.api_type_changed | T1 | -/- | - | llm | passed |
| t1.history.hard_constraint_overrides.required_capability_changed | T1 | -/- | - | llm | passed |
| t1.history.hard_constraint_overrides.disabled_capability_changed | T1 | -/- | - | llm | passed |
| t1.history.hard_constraint_overrides.instance_unhealthy | T1 | -/- | - | llm | passed |
| t1.history.hard_constraint_overrides.quota_exhausted | T1 | -/- | - | llm | passed |
| t1.history.hard_constraint_overrides.budget_exhausted | T1 | -/- | - | llm | passed |
| t1.history.hard_constraint_overrides.local_only_changed | T1 | -/- | - | llm | passed |
| t1.history.hard_constraint_overrides.context_limit_exceeded | T1 | -/- | - | llm | passed |
| t1.history.hard_constraint_overrides.output_limit_exceeded | T1 | -/- | - | llm | passed |
| t1.history.hard_constraint_overrides.locked_policy_changed | T1 | -/- | - | llm | passed |
| t1.history.sessions_do_not_leak | T1 | -/- | - | llm | passed |
| t1.runtime_boundary.rate_limit_fallback | T1 | openai/dv-openai-a-aicc-t1-2026-10-08T04-31-37-310Z-9bea28a0 | gpt-5.6:reasoning-high@dv-openai-a-aicc-t1-2026-10-08T04-31-37-310Z-9bea28a0 | llm | passed |
| t1.runtime_boundary.server_error_fallback | T1 | openai/dv-openai-a-aicc-t1-2026-10-08T04-31-37-310Z-9bea28a0 | gpt-5.6:reasoning-high@dv-openai-a-aicc-t1-2026-10-08T04-31-37-310Z-9bea28a0 | llm | passed |
| t1.runtime_boundary.connection_failure_fallback | T1 | openai/dv-openai-a-aicc-t1-2026-10-08T04-31-37-310Z-9bea28a0 | gpt-5.6:reasoning-high@dv-openai-a-aicc-t1-2026-10-08T04-31-37-310Z-9bea28a0 | llm | passed |
| t1.runtime_boundary.timeout_fallback | T1 | openai/dv-openai-a-aicc-t1-2026-10-08T04-31-37-310Z-9bea28a0 | gpt-5.6:reasoning-high@dv-openai-a-aicc-t1-2026-10-08T04-31-37-310Z-9bea28a0 | llm | passed |
| t1.runtime_boundary.malformed_response_rejected | T1 | openai/dv-openai-a-aicc-t1-2026-10-08T04-31-37-310Z-9bea28a0 | gpt-5.6:reasoning-high@dv-openai-a-aicc-t1-2026-10-08T04-31-37-310Z-9bea28a0 | llm | passed |
| t1.runtime_boundary.wrong_mime_rejected | T1 | openai/dv-openai-a-aicc-t1-2026-10-08T04-31-37-310Z-9bea28a0 | gpt-5.6:reasoning-high@dv-openai-a-aicc-t1-2026-10-08T04-31-37-310Z-9bea28a0 | llm | passed |
| t1.runtime_boundary.missing_usage_rejected | T1 | openai/dv-openai-a-aicc-t1-2026-10-08T04-31-37-310Z-9bea28a0 | gpt-5.6:reasoning-high@dv-openai-a-aicc-t1-2026-10-08T04-31-37-310Z-9bea28a0 | llm | passed |
| t1.task.immediate_succeeded | T1 | -/- | - | llm | passed |
| t1.task.running_succeeded | T1 | -/- | - | video.txt2video | passed |
| t1.task.running_failed | T1 | -/- | - | video.txt2video | passed |
| t1.task.cancelled | T1 | -/- | - | image.upscale | passed |
| t1.task.unknown | T1 | -/- | - | llm | passed |
| t1.task.idempotency_conflict_different_body | T1 | -/- | - | llm | passed |
| t1.task.concurrent_idempotency | T1 | -/- | - | llm | passed |
| t1.task.concurrent_completion | T1 | -/- | - | llm | passed |
| t1.task.terminal_idempotent | T1 | -/- | - | video.txt2video | passed |
| t1.task.reload_recovery | T1 | -/- | - | video.txt2video | passed |
| t1.task.restart_recovery | T1 | -/- | - | node-daemon.restart | passed |
| t1.usage.success_once | T1 | -/- | - | llm | passed |
| t1.usage.idempotent_no_double_charge | T1 | -/- | - | llm | passed |
| t1.usage.fallback_attempts_attributed | T1 | -/- | - | llm | passed |
| t1.security.no_token | T1 | -/- | - | llm | passed |
| t1.security.invalid_token | T1 | -/- | - | llm | passed |
| t1.security.expired_token | T1 | -/- | - | llm | passed |
| t1.security.cross_tenant | T1 | -/- | - | llm | passed |
| t1.security.cross_tenant_task_cancel | T1 | -/- | - | llm | passed |
| t1.security.cross_tenant_usage | T1 | -/- | - | llm | passed |
| t1.security.cross_tenant_message | T1 | -/- | - | llm | passed |
| t1.security.cross_tenant_object | T1 | -/- | - | embedding.text | passed |
| t1.security.rbac_admin_method | T1 | -/- | - | service.reload_settings | passed |
| t1.config.reload_valid | T1 | -/- | - | service.reload_settings | passed |
| t1.config.reload_invalid_keeps_old | T1 | -/- | - | service.reload_settings | passed |
| t1.config.provider_instance_isolation | T1 | -/- | - | llm | passed |
| t1.config.provider_add_refresh | T1 | -/- | - | service.reload_settings | passed |
| t1.config.provider_validate_rejects_duplicate | T1 | -/- | - | service.reload_settings | passed |
| t1.config.provider_delete_isolation | T1 | -/- | - | service.reload_settings | passed |
| t1.config.provider_update_rollback | T1 | -/- | - | service.reload_settings | passed |
| t1.config.cloud_update_dynamic_catalog | T1 | -/- | - | llm | passed |
| t1.config.restart_consistency | T1 | -/- | - | node-daemon.restart | passed |
| t1.observability.correlation | T1 | -/- | - | llm | passed |
| t1.observability.redaction | T1 | -/- | - | llm | passed |

## T1 requirement branch coverage

| Branch | Planned cases | Executed cases | Status |
|---|---:|---:|---|
| route.exact_model_hits_instance | 1 | 1 | passed |
| route.logical_model_selects_candidate | 1 | 1 | passed |
| route.metadata_variant_expands_exact_model | 1 | 1 | passed |
| route.version_exact_rule | 1 | 1 | passed |
| route.version_pattern_rule | 1 | 1 | passed |
| route.version_default_rule | 1 | 1 | passed |
| route.legal_missing_model | 1 | 1 | passed |
| route.invalid_exact_model | 1 | 1 | passed |
| route.invalid_logical_path | 1 | 1 | passed |
| route.missing_provider_instance | 1 | 1 | passed |
| route.disabled_model | 1 | 1 | passed |
| route.offline_model | 1 | 1 | passed |
| route.unmounted_model | 1 | 1 | passed |
| route.corrupt_metadata | 1 | 1 | passed |
| route.strict_no_fallback | 1 | 1 | passed |
| route.parent_fallback | 1 | 1 | passed |
| route.target_logical_fallback | 1 | 1 | passed |
| route.target_exact_fallback | 1 | 1 | passed |
| route.exact_default_no_fallback | 1 | 1 | passed |
| route.fallback_api_type_boundary | 1 | 1 | passed |
| route.fallback_loop | 1 | 1 | passed |
| route.fallback_max_depth | 1 | 1 | passed |
| route.provider_allow | 1 | 1 | passed |
| route.provider_deny | 1 | 1 | passed |
| route.local_only | 1 | 1 | passed |
| route.privacy_boundary | 1 | 1 | passed |
| route.health_filter | 1 | 1 | passed |
| route.quota_filter | 1 | 1 | passed |
| route.budget_filter | 1 | 1 | passed |
| route.context_limit_filter | 1 | 1 | passed |
| route.output_limit_filter | 1 | 1 | passed |
| route.locked_policy_cannot_override | 1 | 1 | passed |
| route.missing_metadata_is_conservative | 1 | 1 | passed |
| route.min_line_admission | 1 | 1 | passed |
| route.disable_line_applied | 1 | 1 | passed |
| route.auto_mount_admission | 1 | 1 | passed |
| route.manual_mount_requires_mapping | 1 | 1 | passed |
| route.global_exact_model_weight | 1 | 1 | passed |
| route.logical_exact_model_weight | 1 | 1 | passed |
| route.provider_instance_weight | 1 | 1 | passed |
| route.system_config_then_request_overlay | 1 | 1 | passed |
| route.api_type.llm.chat.completions.create | 1 | 1 | passed |
| route.api_type.embedding.text.embedding.text | 1 | 1 | passed |
| route.api_type.embedding.multimodal.embedding.multimodal | 1 | 1 | passed |
| route.api_type.decision.decision.evaluate | 1 | 1 | passed |
| route.api_type.rerank.rerank | 1 | 1 | passed |
| route.api_type.image.txt2img.images.generate | 1 | 1 | passed |
| route.api_type.image.img2img.image.img2img | 1 | 1 | passed |
| route.api_type.image.inpaint.image.inpaint | 1 | 1 | passed |
| route.api_type.image.upscale.image.upscale | 1 | 1 | passed |
| route.api_type.image.bg_remove.image.bg_remove | 1 | 1 | passed |
| route.api_type.vision.ocr.vision.ocr | 1 | 1 | passed |
| route.api_type.vision.caption.vision.caption | 1 | 1 | passed |
| route.api_type.vision.detect.vision.detect | 1 | 1 | passed |
| route.api_type.vision.segment.vision.segment | 1 | 1 | passed |
| route.api_type.audio.tts.audio.tts | 1 | 1 | passed |
| route.api_type.audio.asr.audio.asr | 1 | 1 | passed |
| route.api_type.audio.music.audio.music | 1 | 1 | passed |
| route.api_type.audio.enhance.audio.enhance | 1 | 1 | passed |
| route.api_type.video.txt2video.video.txt2video | 1 | 1 | passed |
| route.api_type.video.img2video.video.img2video | 1 | 1 | passed |
| route.api_type.video.video2video.video.video2video | 1 | 1 | passed |
| route.api_type.video.extend.video.extend | 1 | 1 | passed |
| route.api_type.video.upscale.video.upscale | 1 | 1 | passed |
| route.api_type.agent.computer_use.agent.computer_use | 1 | 1 | passed |
| scheduler.profile.cost_first | 1 | 1 | passed |
| scheduler.profile.latency_first | 1 | 1 | passed |
| scheduler.profile.quality_first | 1 | 1 | passed |
| scheduler.profile.balanced | 1 | 1 | passed |
| scheduler.profile.local_first | 1 | 1 | passed |
| scheduler.profile.strict_local | 1 | 1 | passed |
| history.same_session_reuses_exact_model | 1 | 1 | passed |
| history.hard_constraint_overrides.api_type_changed | 1 | 1 | passed |
| history.hard_constraint_overrides.required_capability_changed | 1 | 1 | passed |
| history.hard_constraint_overrides.disabled_capability_changed | 1 | 1 | passed |
| history.hard_constraint_overrides.provider_denied | 1 | 1 | passed |
| history.hard_constraint_overrides.instance_unhealthy | 1 | 1 | passed |
| history.hard_constraint_overrides.quota_exhausted | 1 | 1 | passed |
| history.hard_constraint_overrides.budget_exhausted | 1 | 1 | passed |
| history.hard_constraint_overrides.local_only_changed | 1 | 1 | passed |
| history.hard_constraint_overrides.context_limit_exceeded | 1 | 1 | passed |
| history.hard_constraint_overrides.output_limit_exceeded | 1 | 1 | passed |
| history.hard_constraint_overrides.locked_policy_changed | 1 | 1 | passed |
| history.sessions_do_not_leak | 1 | 1 | passed |
| runtime_boundary.rate_limit_fallback | 1 | 1 | passed |
| runtime_boundary.server_error_fallback | 1 | 1 | passed |
| runtime_boundary.connection_failure_fallback | 1 | 1 | passed |
| runtime_boundary.timeout_fallback | 1 | 1 | passed |
| runtime_boundary.malformed_response_rejected | 1 | 1 | passed |
| runtime_boundary.wrong_mime_rejected | 1 | 1 | passed |
| runtime_boundary.missing_usage_rejected | 1 | 1 | passed |
| task.immediate_succeeded | 1 | 1 | passed |
| task.running_succeeded | 1 | 1 | passed |
| task.running_failed | 1 | 1 | passed |
| task.cancelled | 1 | 1 | passed |
| task.unknown | 1 | 1 | passed |
| task.idempotency_conflict_different_body | 1 | 1 | passed |
| task.concurrent_idempotency | 1 | 1 | passed |
| task.concurrent_completion | 1 | 1 | passed |
| task.terminal_idempotent | 1 | 1 | passed |
| task.reload_recovery | 1 | 1 | passed |
| task.restart_recovery | 1 | 1 | passed |
| usage.success_once | 1 | 1 | passed |
| usage.idempotent_no_double_charge | 1 | 1 | passed |
| usage.fallback_attempts_attributed | 1 | 1 | passed |
| security.no_token | 1 | 1 | passed |
| security.invalid_token | 1 | 1 | passed |
| security.expired_token | 1 | 1 | passed |
| security.cross_tenant | 1 | 1 | passed |
| security.cross_tenant_task_cancel | 1 | 1 | passed |
| security.cross_tenant_usage | 1 | 1 | passed |
| security.cross_tenant_message | 1 | 1 | passed |
| security.cross_tenant_object | 1 | 1 | passed |
| security.rbac_admin_method | 1 | 1 | passed |
| config.reload_valid | 1 | 1 | passed |
| config.reload_invalid_keeps_old | 1 | 1 | passed |
| config.provider_instance_isolation | 1 | 1 | passed |
| config.provider_add_refresh | 1 | 1 | passed |
| config.provider_validate_rejects_duplicate | 1 | 1 | passed |
| config.provider_delete_isolation | 1 | 1 | passed |
| config.provider_update_rollback | 1 | 1 | passed |
| config.cloud_update_dynamic_catalog | 1 | 1 | passed |
| config.restart_consistency | 1 | 1 | passed |
| observability.correlation | 1 | 1 | passed |
| observability.redaction | 1 | 1 | passed |
| embedding.large_batch_artifact | 1 | 1 | passed |
| embedding.space_mismatch_rejected | 1 | 1 | passed |

## T1 combination coverage

| Combination group | Planned cells | Executed cells | Passed cells | Coverage |
|---|---:|---:|---:|---:|
| route_constraints | 65 | 65 | 65 | 100.00% |
| scheduler_profiles | 6 | 6 | 6 | 100.00% |
| history_constraints | 13 | 13 | 13 | 100.00% |
| canonical_api_routes | 24 | 24 | 24 | 100.00% |
| runtime_boundaries | 7 | 7 | 7 | 100.00% |
| cross_cutting | 34 | 34 | 34 | 100.00% |
| embedding_boundaries | 2 | 2 | 2 | 100.00% |

## Confirmed product defects

None.

## Cleanup

Status: passed

- services/aicc/settings restored byte-for-byte
- mock Provider uses zero real model calls
