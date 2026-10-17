# AICC E2E Acceptance Report

- Run: `aicc-2026-10-15T22-05-45-955Z-8721c006`
- Commit: `e897c75d6ec5aaa78f72325a27fca221bbd9409e`
- Capability baseline: `2026-10-15.1`
- Real model calls: 90/88
- Planned maximum cost: $0.880000
- Known actual / unknown estimated exposure: $0.527005 / $0.100000
- Total exposure / budget: $0.627005 / $1.000000
- Unknown-cost calls: 10; budget exceeded: false
- Results: passed=44, failed=5, provider_restricted=0, skipped=0, not_applicable=0, review=39

## Targeted retest

Run after fixing the reported defect; repeat `--case` to select additional cases:

```bash
pnpm run acceptance:gateway -- --config aicc_acceptance.local.toml --allow-credential-mutation --timeout-ms 900000 --provider openrouter --case t2.openrouter.openrouter-t2.google-gemini-2.5-flash-image.vision.caption --case t2.openrouter.openrouter-t2.google-gemini-nano-banana-2.1.image.img2img --case t2.openrouter.openrouter-t2.google-gemini-nano-banana-2.1.vision.caption --case t2.openrouter.openrouter-t2.openai-gpt-5.4-pro.vision.caption --case t2.openrouter.openrouter-t2.qwen-qwen3.7-plus.vision.ocr
```

## Cases

| Case | Layer | Provider | Model | API | Status |
|---|---|---|---|---|---|
| t2.openrouter.openrouter-t2.anthropic-claude-fable-5.1.llm | T2 | openrouter/openrouter-t2 | anthropic/claude-fable-5.1@openrouter-t2 | llm | passed |
| t2.openrouter.openrouter-t2.anthropic-claude-fable-5.llm | T2 | openrouter/openrouter-t2 | anthropic/claude-fable-5@openrouter-t2 | llm | passed |
| t2.openrouter.openrouter-t2.anthropic-claude-haiku-4.5.llm | T2 | openrouter/openrouter-t2 | anthropic/claude-haiku-4.5@openrouter-t2 | llm | passed |
| t2.openrouter.openrouter-t2.anthropic-claude-haiku-5.5.llm | T2 | openrouter/openrouter-t2 | anthropic/claude-haiku-5.5@openrouter-t2 | llm | passed |
| t2.openrouter.openrouter-t2.anthropic-claude-opus-4.5.llm | T2 | openrouter/openrouter-t2 | anthropic/claude-opus-4.5@openrouter-t2 | llm | passed |
| t2.openrouter.openrouter-t2.anthropic-claude-opus-4.6.llm | T2 | openrouter/openrouter-t2 | anthropic/claude-opus-4.6@openrouter-t2 | llm | passed |
| t2.openrouter.openrouter-t2.anthropic-claude-opus-4.7.llm | T2 | openrouter/openrouter-t2 | anthropic/claude-opus-4.7@openrouter-t2 | llm | passed |
| t2.openrouter.openrouter-t2.anthropic-claude-opus-4.8.llm | T2 | openrouter/openrouter-t2 | anthropic/claude-opus-4.8@openrouter-t2 | llm | passed |
| t2.openrouter.openrouter-t2.anthropic-claude-opus-5.5.llm | T2 | openrouter/openrouter-t2 | anthropic/claude-opus-5.5@openrouter-t2 | llm | passed |
| t2.openrouter.openrouter-t2.anthropic-claude-opus-5.llm | T2 | openrouter/openrouter-t2 | anthropic/claude-opus-5@openrouter-t2 | llm | passed |
| t2.openrouter.openrouter-t2.anthropic-claude-sonnet-4.5.llm | T2 | openrouter/openrouter-t2 | anthropic/claude-sonnet-4.5@openrouter-t2 | llm | passed |
| t2.openrouter.openrouter-t2.anthropic-claude-sonnet-4.6.llm | T2 | openrouter/openrouter-t2 | anthropic/claude-sonnet-4.6@openrouter-t2 | llm | passed |
| t2.openrouter.openrouter-t2.anthropic-claude-sonnet-5.5.llm | T2 | openrouter/openrouter-t2 | anthropic/claude-sonnet-5.5@openrouter-t2 | llm | passed |
| t2.openrouter.openrouter-t2.anthropic-claude-sonnet-5.llm | T2 | openrouter/openrouter-t2 | anthropic/claude-sonnet-5@openrouter-t2 | llm | passed |
| t2.openrouter.openrouter-t2.cohere-rerank-v3.5.rerank | T2 | openrouter/openrouter-t2 | cohere/rerank-v3.5@openrouter-t2 | rerank | passed |
| t2.openrouter.openrouter-t2.deepseek-deepseek-v4-flash-vision-exp.vision.ocr | T2 | openrouter/openrouter-t2 | deepseek/deepseek-v4-flash-vision-exp@openrouter-t2 | vision.ocr | review |
| t2.openrouter.openrouter-t2.deepseek-deepseek-v4.1-flash.llm | T2 | openrouter/openrouter-t2 | deepseek/deepseek-v4.1-flash@openrouter-t2 | llm | passed |
| t2.openrouter.openrouter-t2.google-gemini-2.5-flash-image.image.img2img | T2 | openrouter/openrouter-t2 | google/gemini-2.5-flash-image@openrouter-t2 | image.img2img | review |
| t2.openrouter.openrouter-t2.google-gemini-2.5-flash-image.vision.caption | T2 | openrouter/openrouter-t2 | google/gemini-2.5-flash-image@openrouter-t2 | vision.caption | failed |
| t2.openrouter.openrouter-t2.google-gemini-2.5-flash-lite.vision.caption | T2 | openrouter/openrouter-t2 | google/gemini-2.5-flash-lite@openrouter-t2 | vision.caption | review |
| t2.openrouter.openrouter-t2.google-gemini-2.5-flash.vision.caption | T2 | openrouter/openrouter-t2 | google/gemini-2.5-flash@openrouter-t2 | vision.caption | review |
| t2.openrouter.openrouter-t2.google-gemini-2.5-pro.vision.caption | T2 | openrouter/openrouter-t2 | google/gemini-2.5-pro@openrouter-t2 | vision.caption | review |
| t2.openrouter.openrouter-t2.google-gemini-3-flash-preview.vision.caption | T2 | openrouter/openrouter-t2 | google/gemini-3-flash-preview@openrouter-t2 | vision.caption | review |
| t2.openrouter.openrouter-t2.google-gemini-3-pro-image.image.txt2img | T2 | openrouter/openrouter-t2 | google/gemini-3-pro-image@openrouter-t2 | image.txt2img | review |
| t2.openrouter.openrouter-t2.google-gemini-3-pro-image.vision.ocr | T2 | openrouter/openrouter-t2 | google/gemini-3-pro-image@openrouter-t2 | vision.ocr | review |
| t2.openrouter.openrouter-t2.google-gemini-3.1-flash-image.llm | T2 | openrouter/openrouter-t2 | google/gemini-3.1-flash-image@openrouter-t2 | llm | passed |
| t2.openrouter.openrouter-t2.google-gemini-3.1-flash-lite-image.image.img2img | T2 | openrouter/openrouter-t2 | google/gemini-3.1-flash-lite-image@openrouter-t2 | image.img2img | review |
| t2.openrouter.openrouter-t2.google-gemini-3.1-flash-lite-image.vision.caption | T2 | openrouter/openrouter-t2 | google/gemini-3.1-flash-lite-image@openrouter-t2 | vision.caption | review |
| t2.openrouter.openrouter-t2.google-gemini-3.1-flash-lite.vision.caption | T2 | openrouter/openrouter-t2 | google/gemini-3.1-flash-lite@openrouter-t2 | vision.caption | review |
| t2.openrouter.openrouter-t2.google-gemini-3.1-pro-preview-customtools.llm | T2 | openrouter/openrouter-t2 | google/gemini-3.1-pro-preview-customtools@openrouter-t2 | llm | passed |
| t2.openrouter.openrouter-t2.google-gemini-3.1-pro-preview.llm | T2 | openrouter/openrouter-t2 | google/gemini-3.1-pro-preview@openrouter-t2 | llm | passed |
| t2.openrouter.openrouter-t2.google-gemini-3.5-flash-lite.llm | T2 | openrouter/openrouter-t2 | google/gemini-3.5-flash-lite@openrouter-t2 | llm | passed |
| t2.openrouter.openrouter-t2.google-gemini-3.5-flash.llm | T2 | openrouter/openrouter-t2 | google/gemini-3.5-flash@openrouter-t2 | llm | passed |
| t2.openrouter.openrouter-t2.google-gemini-3.5-transcribe.audio.asr | T2 | openrouter/openrouter-t2 | google/gemini-3.5-transcribe@openrouter-t2 | audio.asr | review |
| t2.openrouter.openrouter-t2.google-gemini-3.6-flash.vision.ocr | T2 | openrouter/openrouter-t2 | google/gemini-3.6-flash@openrouter-t2 | vision.ocr | review |
| t2.openrouter.openrouter-t2.google-gemini-3.7-flash.vision.ocr | T2 | openrouter/openrouter-t2 | google/gemini-3.7-flash@openrouter-t2 | vision.ocr | review |
| t2.openrouter.openrouter-t2.google-gemini-3.8-flash.llm | T2 | openrouter/openrouter-t2 | google/gemini-3.8-flash@openrouter-t2 | llm | passed |
| t2.openrouter.openrouter-t2.google-gemini-4-argon.llm | T2 | openrouter/openrouter-t2 | google/gemini-4-argon@openrouter-t2 | llm | passed |
| t2.openrouter.openrouter-t2.google-gemini-embedding-001.embedding.text | T2 | openrouter/openrouter-t2 | google/gemini-embedding-001@openrouter-t2 | embedding.text | passed |
| t2.openrouter.openrouter-t2.google-gemini-nano-banana-2.1.image.img2img | T2 | openrouter/openrouter-t2 | google/gemini-nano-banana-2.1@openrouter-t2 | image.img2img | failed |
| t2.openrouter.openrouter-t2.google-gemini-nano-banana-2.1.vision.caption | T2 | openrouter/openrouter-t2 | google/gemini-nano-banana-2.1@openrouter-t2 | vision.caption | failed |
| t2.openrouter.openrouter-t2.google-gemma-4-26b-a4b-it.vision.caption | T2 | openrouter/openrouter-t2 | google/gemma-4-26b-a4b-it@openrouter-t2 | vision.caption | review |
| t2.openrouter.openrouter-t2.google-gemma-4-31b-it.vision.caption | T2 | openrouter/openrouter-t2 | google/gemma-4-31b-it@openrouter-t2 | vision.caption | review |
| t2.openrouter.openrouter-t2.google-lyria-3-pro-preview.audio.music | T2 | openrouter/openrouter-t2 | google/lyria-3-pro-preview@openrouter-t2 | audio.music | review |
| t2.openrouter.openrouter-t2.minimax-minimax-m2.1.llm | T2 | openrouter/openrouter-t2 | minimax/minimax-m2.1@openrouter-t2 | llm | passed |
| t2.openrouter.openrouter-t2.minimax-minimax-m2.llm | T2 | openrouter/openrouter-t2 | minimax/minimax-m2@openrouter-t2 | llm | passed |
| t2.openrouter.openrouter-t2.minimax-minimax-m3.vision.ocr | T2 | openrouter/openrouter-t2 | minimax/minimax-m3@openrouter-t2 | vision.ocr | review |
| t2.openrouter.openrouter-t2.moonshotai-kimi-k2.6.llm | T2 | openrouter/openrouter-t2 | moonshotai/kimi-k2.6@openrouter-t2 | llm | passed |
| t2.openrouter.openrouter-t2.moonshotai-kimi-k2.7-code.llm | T2 | openrouter/openrouter-t2 | moonshotai/kimi-k2.7-code@openrouter-t2 | llm | passed |
| t2.openrouter.openrouter-t2.moonshotai-kimi-k3.llm | T2 | openrouter/openrouter-t2 | moonshotai/kimi-k3@openrouter-t2 | llm | passed |
| t2.openrouter.openrouter-t2.openai-gpt-4o-mini-transcribe.audio.asr | T2 | openrouter/openrouter-t2 | openai/gpt-4o-mini-transcribe@openrouter-t2 | audio.asr | review |
| t2.openrouter.openrouter-t2.openai-gpt-5.3-codex.vision.caption | T2 | openrouter/openrouter-t2 | openai/gpt-5.3-codex@openrouter-t2 | vision.caption | review |
| t2.openrouter.openrouter-t2.openai-gpt-5.4-mini.vision.caption | T2 | openrouter/openrouter-t2 | openai/gpt-5.4-mini@openrouter-t2 | vision.caption | review |
| t2.openrouter.openrouter-t2.openai-gpt-5.4-nano.vision.caption | T2 | openrouter/openrouter-t2 | openai/gpt-5.4-nano@openrouter-t2 | vision.caption | review |
| t2.openrouter.openrouter-t2.openai-gpt-5.4-pro.vision.caption | T2 | openrouter/openrouter-t2 | openai/gpt-5.4-pro@openrouter-t2 | vision.caption | failed |
| t2.openrouter.openrouter-t2.openai-gpt-5.4.vision.caption | T2 | openrouter/openrouter-t2 | openai/gpt-5.4@openrouter-t2 | vision.caption | review |
| t2.openrouter.openrouter-t2.openai-gpt-5.5-pro.vision.caption | T2 | openrouter/openrouter-t2 | openai/gpt-5.5-pro@openrouter-t2 | vision.caption | review |
| t2.openrouter.openrouter-t2.openai-gpt-5.5.vision.caption | T2 | openrouter/openrouter-t2 | openai/gpt-5.5@openrouter-t2 | vision.caption | review |
| t2.openrouter.openrouter-t2.openai-gpt-5.6-luna.vision.caption | T2 | openrouter/openrouter-t2 | openai/gpt-5.6-luna@openrouter-t2 | vision.caption | review |
| t2.openrouter.openrouter-t2.openai-gpt-5.6-sol.vision.caption | T2 | openrouter/openrouter-t2 | openai/gpt-5.6-sol@openrouter-t2 | vision.caption | review |
| t2.openrouter.openrouter-t2.openai-gpt-5.6-terra.vision.caption | T2 | openrouter/openrouter-t2 | openai/gpt-5.6-terra@openrouter-t2 | vision.caption | review |
| t2.openrouter.openrouter-t2.openai-gpt-6-astra.vision.caption | T2 | openrouter/openrouter-t2 | openai/gpt-6-astra@openrouter-t2 | vision.caption | review |
| t2.openrouter.openrouter-t2.openai-gpt-6-luna.vision.caption | T2 | openrouter/openrouter-t2 | openai/gpt-6-luna@openrouter-t2 | vision.caption | review |
| t2.openrouter.openrouter-t2.openai-gpt-6-sol.vision.caption | T2 | openrouter/openrouter-t2 | openai/gpt-6-sol@openrouter-t2 | vision.caption | review |
| t2.openrouter.openrouter-t2.openai-gpt-image-2.image.txt2img | T2 | openrouter/openrouter-t2 | openai/gpt-image-2@openrouter-t2 | image.txt2img | review |
| t2.openrouter.openrouter-t2.openai-text-embedding-3-small.embedding.text | T2 | openrouter/openrouter-t2 | openai/text-embedding-3-small@openrouter-t2 | embedding.text | passed |
| t2.openrouter.openrouter-t2.qwen-qwen3.5-122b-a10b.llm | T2 | openrouter/openrouter-t2 | qwen/qwen3.5-122b-a10b@openrouter-t2 | llm | passed |
| t2.openrouter.openrouter-t2.qwen-qwen3.5-27b.llm | T2 | openrouter/openrouter-t2 | qwen/qwen3.5-27b@openrouter-t2 | llm | passed |
| t2.openrouter.openrouter-t2.qwen-qwen3.5-35b-a3b.llm | T2 | openrouter/openrouter-t2 | qwen/qwen3.5-35b-a3b@openrouter-t2 | llm | passed |
| t2.openrouter.openrouter-t2.qwen-qwen3.5-397b-a17b.llm | T2 | openrouter/openrouter-t2 | qwen/qwen3.5-397b-a17b@openrouter-t2 | llm | passed |
| t2.openrouter.openrouter-t2.qwen-qwen3.6-35b-a3b.llm | T2 | openrouter/openrouter-t2 | qwen/qwen3.6-35b-a3b@openrouter-t2 | llm | passed |
| t2.openrouter.openrouter-t2.qwen-qwen3.6-flash.llm | T2 | openrouter/openrouter-t2 | qwen/qwen3.6-flash@openrouter-t2 | llm | passed |
| t2.openrouter.openrouter-t2.qwen-qwen3.6-plus.llm | T2 | openrouter/openrouter-t2 | qwen/qwen3.6-plus@openrouter-t2 | llm | passed |
| t2.openrouter.openrouter-t2.qwen-qwen3.7-flash.llm | T2 | openrouter/openrouter-t2 | qwen/qwen3.7-flash@openrouter-t2 | llm | passed |
| t2.openrouter.openrouter-t2.qwen-qwen3.7-max.llm | T2 | openrouter/openrouter-t2 | qwen/qwen3.7-max@openrouter-t2 | llm | passed |
| t2.openrouter.openrouter-t2.qwen-qwen3.7-plus.vision.ocr | T2 | openrouter/openrouter-t2 | qwen/qwen3.7-plus@openrouter-t2 | vision.ocr | failed |
| t2.openrouter.openrouter-t2.qwen-qwen3.8-27b.vision.caption | T2 | openrouter/openrouter-t2 | qwen/qwen3.8-27b@openrouter-t2 | vision.caption | review |
| t2.openrouter.openrouter-t2.qwen-qwen3.8-flash.vision.caption | T2 | openrouter/openrouter-t2 | qwen/qwen3.8-flash@openrouter-t2 | vision.caption | review |
| t2.openrouter.openrouter-t2.qwen-qwen3.8-max-0902.vision.caption | T2 | openrouter/openrouter-t2 | qwen/qwen3.8-max-0902@openrouter-t2 | vision.caption | review |
| t2.openrouter.openrouter-t2.qwen-qwen3.8-omni-flash.vision.caption | T2 | openrouter/openrouter-t2 | qwen/qwen3.8-omni-flash@openrouter-t2 | vision.caption | review |
| t2.openrouter.openrouter-t2.z-ai-glm-4.5-air.llm | T2 | openrouter/openrouter-t2 | z-ai/glm-4.5-air@openrouter-t2 | llm | passed |
| t2.openrouter.openrouter-t2.z-ai-glm-4.5v.vision.caption | T2 | openrouter/openrouter-t2 | z-ai/glm-4.5v@openrouter-t2 | vision.caption | review |
| t2.openrouter.openrouter-t2.z-ai-glm-4.6v.llm | T2 | openrouter/openrouter-t2 | z-ai/glm-4.6v@openrouter-t2 | llm | passed |
| t2.openrouter.openrouter-t2.z-ai-glm-4.7-flash.llm | T2 | openrouter/openrouter-t2 | z-ai/glm-4.7-flash@openrouter-t2 | llm | passed |
| t2.openrouter.openrouter-t2.z-ai-glm-5.1.llm | T2 | openrouter/openrouter-t2 | z-ai/glm-5.1@openrouter-t2 | llm | passed |
| t2.openrouter.openrouter-t2.z-ai-glm-5.3-flash.vision.caption | T2 | openrouter/openrouter-t2 | z-ai/glm-5.3-flash@openrouter-t2 | vision.caption | review |
| t2.openrouter.openrouter-t2.z-ai-glm-5.llm | T2 | openrouter/openrouter-t2 | z-ai/glm-5@openrouter-t2 | llm | passed |
| t2.openrouter.openrouter-t2.z-ai-glm-5v-turbo.vision.ocr | T2 | openrouter/openrouter-t2 | z-ai/glm-5v-turbo@openrouter-t2 | vision.ocr | review |

## Physical model coverage

| Provider | Inventory model | Physical model | Decision | Reason | Retained representative |
|---|---|---|---|---|---|
| openrouter/openrouter-t2 | anthropic/claude-fable-5.1 | anthropic/claude-fable-5.1 | included | physical model | - |
| openrouter/openrouter-t2 | anthropic/claude-fable-5 | anthropic/claude-fable-5 | included | physical model | - |
| openrouter/openrouter-t2 | anthropic/claude-haiku-4.5 | anthropic/claude-haiku-4.5 | included | physical model | - |
| openrouter/openrouter-t2 | anthropic/claude-haiku-5.5 | anthropic/claude-haiku-5.5 | included | physical model | - |
| openrouter/openrouter-t2 | anthropic/claude-opus-4.5 | anthropic/claude-opus-4.5 | included | physical model | - |
| openrouter/openrouter-t2 | anthropic/claude-opus-4.6 | anthropic/claude-opus-4.6 | included | physical model | - |
| openrouter/openrouter-t2 | anthropic/claude-opus-4.7 | anthropic/claude-opus-4.7 | included | physical model | - |
| openrouter/openrouter-t2 | anthropic/claude-opus-4.8 | anthropic/claude-opus-4.8 | included | physical model | - |
| openrouter/openrouter-t2 | anthropic/claude-opus-5.5 | anthropic/claude-opus-5.5 | included | physical model | - |
| openrouter/openrouter-t2 | anthropic/claude-opus-5 | anthropic/claude-opus-5 | included | physical model | - |
| openrouter/openrouter-t2 | anthropic/claude-sonnet-4.5 | anthropic/claude-sonnet-4.5 | included | physical model | - |
| openrouter/openrouter-t2 | anthropic/claude-sonnet-4.6 | anthropic/claude-sonnet-4.6 | included | physical model | - |
| openrouter/openrouter-t2 | anthropic/claude-sonnet-5.5 | anthropic/claude-sonnet-5.5 | included | physical model | - |
| openrouter/openrouter-t2 | anthropic/claude-sonnet-5 | anthropic/claude-sonnet-5 | included | physical model | - |
| openrouter/openrouter-t2 | cohere/rerank-v3.5 | cohere/rerank-v3.5 | included | physical model | - |
| openrouter/openrouter-t2 | deepseek/deepseek-v4-flash-vision-exp | deepseek/deepseek-v4-flash-vision-exp | included | physical model | - |
| openrouter/openrouter-t2 | deepseek/deepseek-v4-flash | deepseek/deepseek-v4-flash | included | physical model | - |
| openrouter/openrouter-t2 | deepseek/deepseek-v4-pro | deepseek/deepseek-v4-pro | included | physical model | - |
| openrouter/openrouter-t2 | deepseek/deepseek-v4.1-flash | deepseek/deepseek-v4.1-flash | included | physical model | - |
| openrouter/openrouter-t2 | google/gemini-2.5-flash-image | google/gemini-2.5-flash-image | included | physical model | - |
| openrouter/openrouter-t2 | google/gemini-2.5-flash-lite | google/gemini-2.5-flash-lite | included | physical model | - |
| openrouter/openrouter-t2 | google/gemini-2.5-flash | google/gemini-2.5-flash | included | physical model | - |
| openrouter/openrouter-t2 | google/gemini-2.5-pro | google/gemini-2.5-pro | included | physical model | - |
| openrouter/openrouter-t2 | google/gemini-3-flash-preview | google/gemini-3-flash-preview | included | physical model | - |
| openrouter/openrouter-t2 | google/gemini-3-pro-image | google/gemini-3-pro-image | included | physical model | - |
| openrouter/openrouter-t2 | google/gemini-3.1-flash-image | google/gemini-3.1-flash-image | included | physical model | - |
| openrouter/openrouter-t2 | google/gemini-3.1-flash-lite-image | google/gemini-3.1-flash-lite-image | included | physical model | - |
| openrouter/openrouter-t2 | google/gemini-3.1-flash-lite | google/gemini-3.1-flash-lite | included | physical model | - |
| openrouter/openrouter-t2 | google/gemini-3.1-flash-tts-preview | google/gemini-3.1-flash-tts-preview | included | physical model | - |
| openrouter/openrouter-t2 | google/gemini-3.1-pro-preview-customtools | google/gemini-3.1-pro-preview-customtools | included | physical model | - |
| openrouter/openrouter-t2 | google/gemini-3.1-pro-preview | google/gemini-3.1-pro-preview | included | physical model | - |
| openrouter/openrouter-t2 | google/gemini-3.5-flash-lite | google/gemini-3.5-flash-lite | included | physical model | - |
| openrouter/openrouter-t2 | google/gemini-3.5-flash | google/gemini-3.5-flash | included | physical model | - |
| openrouter/openrouter-t2 | google/gemini-3.5-transcribe | google/gemini-3.5-transcribe | included | physical model | - |
| openrouter/openrouter-t2 | google/gemini-3.6-flash | google/gemini-3.6-flash | included | physical model | - |
| openrouter/openrouter-t2 | google/gemini-3.7-flash | google/gemini-3.7-flash | included | physical model | - |
| openrouter/openrouter-t2 | google/gemini-3.8-flash-lite-tts | google/gemini-3.8-flash-lite-tts | included | physical model | - |
| openrouter/openrouter-t2 | google/gemini-3.8-flash-tts | google/gemini-3.8-flash-tts | included | physical model | - |
| openrouter/openrouter-t2 | google/gemini-3.8-flash | google/gemini-3.8-flash | included | physical model | - |
| openrouter/openrouter-t2 | google/gemini-4-argon | google/gemini-4-argon | included | physical model | - |
| openrouter/openrouter-t2 | google/gemini-embedding-001 | google/gemini-embedding-001 | included | physical model | - |
| openrouter/openrouter-t2 | google/gemini-embedding-2-preview | google/gemini-embedding-2-preview | included | physical model | - |
| openrouter/openrouter-t2 | google/gemini-embedding-2 | google/gemini-embedding-2 | included | physical model | - |
| openrouter/openrouter-t2 | google/gemini-nano-banana-2.1 | google/gemini-nano-banana-2.1 | included | physical model | - |
| openrouter/openrouter-t2 | google/gemma-4-26b-a4b-it | google/gemma-4-26b-a4b-it | included | physical model | - |
| openrouter/openrouter-t2 | google/gemma-4-31b-it | google/gemma-4-31b-it | included | physical model | - |
| openrouter/openrouter-t2 | google/lyria-3-clip-preview | google/lyria-3-clip-preview | included | physical model | - |
| openrouter/openrouter-t2 | google/lyria-3-pro-preview | google/lyria-3-pro-preview | included | physical model | - |
| openrouter/openrouter-t2 | minimax/hailuo-2.3 | minimax/hailuo-2.3 | included | physical model | - |
| openrouter/openrouter-t2 | minimax/minimax-m2.1 | minimax/minimax-m2.1 | included | physical model | - |
| openrouter/openrouter-t2 | minimax/minimax-m2.5 | minimax/minimax-m2.5 | included | physical model | - |
| openrouter/openrouter-t2 | minimax/minimax-m2.7 | minimax/minimax-m2.7 | included | physical model | - |
| openrouter/openrouter-t2 | minimax/minimax-m2 | minimax/minimax-m2 | included | physical model | - |
| openrouter/openrouter-t2 | minimax/minimax-m3 | minimax/minimax-m3 | included | physical model | - |
| openrouter/openrouter-t2 | minimax/speech-2.8-hd | minimax/speech-2.8-hd | included | physical model | - |
| openrouter/openrouter-t2 | minimax/speech-2.8-turbo | minimax/speech-2.8-turbo | included | physical model | - |
| openrouter/openrouter-t2 | moonshotai/kimi-k2.6 | moonshotai/kimi-k2.6 | included | physical model | - |
| openrouter/openrouter-t2 | moonshotai/kimi-k2.7-code | moonshotai/kimi-k2.7-code | included | physical model | - |
| openrouter/openrouter-t2 | moonshotai/kimi-k3 | moonshotai/kimi-k3 | included | physical model | - |
| openrouter/openrouter-t2 | openai/gpt-4o-mini-transcribe | openai/gpt-4o-mini-transcribe | included | physical model | - |
| openrouter/openrouter-t2 | openai/gpt-4o-transcribe | openai/gpt-4o-transcribe | included | physical model | - |
| openrouter/openrouter-t2 | openai/gpt-5.3-codex | openai/gpt-5.3-codex | included | physical model | - |
| openrouter/openrouter-t2 | openai/gpt-5.4-mini | openai/gpt-5.4-mini | included | physical model | - |
| openrouter/openrouter-t2 | openai/gpt-5.4-nano | openai/gpt-5.4-nano | included | physical model | - |
| openrouter/openrouter-t2 | openai/gpt-5.4-pro | openai/gpt-5.4-pro | included | physical model | - |
| openrouter/openrouter-t2 | openai/gpt-5.4 | openai/gpt-5.4 | included | physical model | - |
| openrouter/openrouter-t2 | openai/gpt-5.5-pro | openai/gpt-5.5-pro | included | physical model | - |
| openrouter/openrouter-t2 | openai/gpt-5.5 | openai/gpt-5.5 | included | physical model | - |
| openrouter/openrouter-t2 | openai/gpt-5.6-luna | openai/gpt-5.6-luna | included | physical model | - |
| openrouter/openrouter-t2 | openai/gpt-5.6-sol | openai/gpt-5.6-sol | included | physical model | - |
| openrouter/openrouter-t2 | openai/gpt-5.6-terra | openai/gpt-5.6-terra | included | physical model | - |
| openrouter/openrouter-t2 | openai/gpt-6-astra | openai/gpt-6-astra | included | physical model | - |
| openrouter/openrouter-t2 | openai/gpt-6-luna | openai/gpt-6-luna | included | physical model | - |
| openrouter/openrouter-t2 | openai/gpt-6-sol | openai/gpt-6-sol | included | physical model | - |
| openrouter/openrouter-t2 | openai/gpt-image-2 | openai/gpt-image-2 | included | physical model | - |
| openrouter/openrouter-t2 | openai/gpt-transcribe | openai/gpt-transcribe | included | physical model | - |
| openrouter/openrouter-t2 | openai/text-embedding-3-large | openai/text-embedding-3-large | included | physical model | - |
| openrouter/openrouter-t2 | openai/text-embedding-3-small | openai/text-embedding-3-small | included | physical model | - |
| openrouter/openrouter-t2 | openai/text-embedding-ada-002 | openai/text-embedding-ada-002 | included | physical model | - |
| openrouter/openrouter-t2 | openai/whisper-1 | openai/whisper-1 | included | physical model | - |
| openrouter/openrouter-t2 | qwen/qwen3.5-122b-a10b | qwen/qwen3.5-122b-a10b | included | physical model | - |
| openrouter/openrouter-t2 | qwen/qwen3.5-27b | qwen/qwen3.5-27b | included | physical model | - |
| openrouter/openrouter-t2 | qwen/qwen3.5-35b-a3b | qwen/qwen3.5-35b-a3b | included | physical model | - |
| openrouter/openrouter-t2 | qwen/qwen3.5-397b-a17b | qwen/qwen3.5-397b-a17b | included | physical model | - |
| openrouter/openrouter-t2 | qwen/qwen3.6-35b-a3b | qwen/qwen3.6-35b-a3b | included | physical model | - |
| openrouter/openrouter-t2 | qwen/qwen3.6-flash | qwen/qwen3.6-flash | included | physical model | - |
| openrouter/openrouter-t2 | qwen/qwen3.6-plus | qwen/qwen3.6-plus | included | physical model | - |
| openrouter/openrouter-t2 | qwen/qwen3.7-flash | qwen/qwen3.7-flash | included | physical model | - |
| openrouter/openrouter-t2 | qwen/qwen3.7-max | qwen/qwen3.7-max | included | physical model | - |
| openrouter/openrouter-t2 | qwen/qwen3.7-plus | qwen/qwen3.7-plus | included | physical model | - |
| openrouter/openrouter-t2 | qwen/qwen3.8-2.4t-a95b | qwen/qwen3.8-2.4t-a95b | included | physical model | - |
| openrouter/openrouter-t2 | qwen/qwen3.8-27b | qwen/qwen3.8-27b | included | physical model | - |
| openrouter/openrouter-t2 | qwen/qwen3.8-flash | qwen/qwen3.8-flash | included | physical model | - |
| openrouter/openrouter-t2 | qwen/qwen3.8-max-0902 | qwen/qwen3.8-max-0902 | included | physical model | - |
| openrouter/openrouter-t2 | qwen/qwen3.8-omni-flash | qwen/qwen3.8-omni-flash | included | physical model | - |
| openrouter/openrouter-t2 | typesafe/jev-1.13 | typesafe/jev-1.13 | included | physical model | - |
| openrouter/openrouter-t2 | z-ai/glm-4.5-air | z-ai/glm-4.5-air | included | physical model | - |
| openrouter/openrouter-t2 | z-ai/glm-4.5 | z-ai/glm-4.5 | included | physical model | - |
| openrouter/openrouter-t2 | z-ai/glm-4.5v | z-ai/glm-4.5v | included | physical model | - |
| openrouter/openrouter-t2 | z-ai/glm-4.6 | z-ai/glm-4.6 | included | physical model | - |
| openrouter/openrouter-t2 | z-ai/glm-4.6v | z-ai/glm-4.6v | included | physical model | - |
| openrouter/openrouter-t2 | z-ai/glm-4.7-flash | z-ai/glm-4.7-flash | included | physical model | - |
| openrouter/openrouter-t2 | z-ai/glm-4.7 | z-ai/glm-4.7 | included | physical model | - |
| openrouter/openrouter-t2 | z-ai/glm-5-turbo | z-ai/glm-5-turbo | included | physical model | - |
| openrouter/openrouter-t2 | z-ai/glm-5.1 | z-ai/glm-5.1 | included | physical model | - |
| openrouter/openrouter-t2 | z-ai/glm-5.2 | z-ai/glm-5.2 | included | physical model | - |
| openrouter/openrouter-t2 | z-ai/glm-5.3-flash | z-ai/glm-5.3-flash | included | physical model | - |
| openrouter/openrouter-t2 | z-ai/glm-5.3 | z-ai/glm-5.3 | included | physical model | - |
| openrouter/openrouter-t2 | z-ai/glm-5 | z-ai/glm-5 | included | physical model | - |
| openrouter/openrouter-t2 | z-ai/glm-5v-turbo | z-ai/glm-5v-turbo | included | physical model | - |
| openrouter/openrouter-t2 | anthropic/claude-fable-5.1 | anthropic/claude-fable-5.1 | included | physical model | - |
| openrouter/openrouter-t2 | anthropic/claude-fable-5 | anthropic/claude-fable-5 | included | physical model | - |
| openrouter/openrouter-t2 | anthropic/claude-haiku-4.5 | anthropic/claude-haiku-4.5 | included | physical model | - |
| openrouter/openrouter-t2 | anthropic/claude-haiku-5.5 | anthropic/claude-haiku-5.5 | included | physical model | - |
| openrouter/openrouter-t2 | anthropic/claude-opus-4.5 | anthropic/claude-opus-4.5 | included | physical model | - |
| openrouter/openrouter-t2 | anthropic/claude-opus-4.6 | anthropic/claude-opus-4.6 | included | physical model | - |
| openrouter/openrouter-t2 | anthropic/claude-opus-4.7 | anthropic/claude-opus-4.7 | included | physical model | - |
| openrouter/openrouter-t2 | anthropic/claude-opus-4.8 | anthropic/claude-opus-4.8 | included | physical model | - |
| openrouter/openrouter-t2 | anthropic/claude-opus-5.5 | anthropic/claude-opus-5.5 | included | physical model | - |
| openrouter/openrouter-t2 | anthropic/claude-opus-5 | anthropic/claude-opus-5 | included | physical model | - |
| openrouter/openrouter-t2 | anthropic/claude-sonnet-4.5 | anthropic/claude-sonnet-4.5 | included | physical model | - |
| openrouter/openrouter-t2 | anthropic/claude-sonnet-4.6 | anthropic/claude-sonnet-4.6 | included | physical model | - |
| openrouter/openrouter-t2 | anthropic/claude-sonnet-5.5 | anthropic/claude-sonnet-5.5 | included | physical model | - |
| openrouter/openrouter-t2 | anthropic/claude-sonnet-5 | anthropic/claude-sonnet-5 | included | physical model | - |
| openrouter/openrouter-t2 | cohere/rerank-v3.5 | cohere/rerank-v3.5 | included | physical model | - |
| openrouter/openrouter-t2 | deepseek/deepseek-v4-flash-vision-exp | deepseek/deepseek-v4-flash-vision-exp | included | physical model | - |
| openrouter/openrouter-t2 | deepseek/deepseek-v4-flash | deepseek/deepseek-v4-flash | included | physical model | - |
| openrouter/openrouter-t2 | deepseek/deepseek-v4-pro | deepseek/deepseek-v4-pro | included | physical model | - |
| openrouter/openrouter-t2 | deepseek/deepseek-v4.1-flash | deepseek/deepseek-v4.1-flash | included | physical model | - |
| openrouter/openrouter-t2 | google/gemini-2.5-flash-image | google/gemini-2.5-flash-image | included | physical model | - |
| openrouter/openrouter-t2 | google/gemini-2.5-flash-lite | google/gemini-2.5-flash-lite | included | physical model | - |
| openrouter/openrouter-t2 | google/gemini-2.5-flash | google/gemini-2.5-flash | included | physical model | - |
| openrouter/openrouter-t2 | google/gemini-2.5-pro | google/gemini-2.5-pro | included | physical model | - |
| openrouter/openrouter-t2 | google/gemini-3-flash-preview | google/gemini-3-flash-preview | included | physical model | - |
| openrouter/openrouter-t2 | google/gemini-3-pro-image | google/gemini-3-pro-image | included | physical model | - |
| openrouter/openrouter-t2 | google/gemini-3.1-flash-image | google/gemini-3.1-flash-image | included | physical model | - |
| openrouter/openrouter-t2 | google/gemini-3.1-flash-lite-image | google/gemini-3.1-flash-lite-image | included | physical model | - |
| openrouter/openrouter-t2 | google/gemini-3.1-flash-lite | google/gemini-3.1-flash-lite | included | physical model | - |
| openrouter/openrouter-t2 | google/gemini-3.1-flash-tts-preview | google/gemini-3.1-flash-tts-preview | included | physical model | - |
| openrouter/openrouter-t2 | google/gemini-3.1-pro-preview-customtools | google/gemini-3.1-pro-preview-customtools | included | physical model | - |
| openrouter/openrouter-t2 | google/gemini-3.1-pro-preview | google/gemini-3.1-pro-preview | included | physical model | - |
| openrouter/openrouter-t2 | google/gemini-3.5-flash-lite | google/gemini-3.5-flash-lite | included | physical model | - |
| openrouter/openrouter-t2 | google/gemini-3.5-flash | google/gemini-3.5-flash | included | physical model | - |
| openrouter/openrouter-t2 | google/gemini-3.5-transcribe | google/gemini-3.5-transcribe | included | physical model | - |
| openrouter/openrouter-t2 | google/gemini-3.6-flash | google/gemini-3.6-flash | included | physical model | - |
| openrouter/openrouter-t2 | google/gemini-3.7-flash | google/gemini-3.7-flash | included | physical model | - |
| openrouter/openrouter-t2 | google/gemini-3.8-flash-lite-tts | google/gemini-3.8-flash-lite-tts | included | physical model | - |
| openrouter/openrouter-t2 | google/gemini-3.8-flash-tts | google/gemini-3.8-flash-tts | included | physical model | - |
| openrouter/openrouter-t2 | google/gemini-3.8-flash | google/gemini-3.8-flash | included | physical model | - |
| openrouter/openrouter-t2 | google/gemini-4-argon | google/gemini-4-argon | included | physical model | - |
| openrouter/openrouter-t2 | google/gemini-embedding-001 | google/gemini-embedding-001 | included | physical model | - |
| openrouter/openrouter-t2 | google/gemini-embedding-2-preview | google/gemini-embedding-2-preview | included | physical model | - |
| openrouter/openrouter-t2 | google/gemini-embedding-2 | google/gemini-embedding-2 | included | physical model | - |
| openrouter/openrouter-t2 | google/gemini-nano-banana-2.1 | google/gemini-nano-banana-2.1 | included | physical model | - |
| openrouter/openrouter-t2 | google/gemma-4-26b-a4b-it | google/gemma-4-26b-a4b-it | included | physical model | - |
| openrouter/openrouter-t2 | google/gemma-4-31b-it | google/gemma-4-31b-it | included | physical model | - |
| openrouter/openrouter-t2 | google/lyria-3-clip-preview | google/lyria-3-clip-preview | included | physical model | - |
| openrouter/openrouter-t2 | google/lyria-3-pro-preview | google/lyria-3-pro-preview | included | physical model | - |
| openrouter/openrouter-t2 | minimax/hailuo-2.3 | minimax/hailuo-2.3 | included | physical model | - |
| openrouter/openrouter-t2 | minimax/minimax-m2.1 | minimax/minimax-m2.1 | included | physical model | - |
| openrouter/openrouter-t2 | minimax/minimax-m2.5 | minimax/minimax-m2.5 | included | physical model | - |
| openrouter/openrouter-t2 | minimax/minimax-m2.7 | minimax/minimax-m2.7 | included | physical model | - |
| openrouter/openrouter-t2 | minimax/minimax-m2 | minimax/minimax-m2 | included | physical model | - |
| openrouter/openrouter-t2 | minimax/minimax-m3 | minimax/minimax-m3 | included | physical model | - |
| openrouter/openrouter-t2 | minimax/speech-2.8-hd | minimax/speech-2.8-hd | included | physical model | - |
| openrouter/openrouter-t2 | minimax/speech-2.8-turbo | minimax/speech-2.8-turbo | included | physical model | - |
| openrouter/openrouter-t2 | moonshotai/kimi-k2.6 | moonshotai/kimi-k2.6 | included | physical model | - |
| openrouter/openrouter-t2 | moonshotai/kimi-k2.7-code | moonshotai/kimi-k2.7-code | included | physical model | - |
| openrouter/openrouter-t2 | moonshotai/kimi-k3 | moonshotai/kimi-k3 | included | physical model | - |
| openrouter/openrouter-t2 | openai/gpt-4o-mini-transcribe | openai/gpt-4o-mini-transcribe | included | physical model | - |
| openrouter/openrouter-t2 | openai/gpt-4o-transcribe | openai/gpt-4o-transcribe | included | physical model | - |
| openrouter/openrouter-t2 | openai/gpt-5.3-codex | openai/gpt-5.3-codex | included | physical model | - |
| openrouter/openrouter-t2 | openai/gpt-5.4-mini | openai/gpt-5.4-mini | included | physical model | - |
| openrouter/openrouter-t2 | openai/gpt-5.4-nano | openai/gpt-5.4-nano | included | physical model | - |
| openrouter/openrouter-t2 | openai/gpt-5.4-pro | openai/gpt-5.4-pro | included | physical model | - |
| openrouter/openrouter-t2 | openai/gpt-5.4 | openai/gpt-5.4 | included | physical model | - |
| openrouter/openrouter-t2 | openai/gpt-5.5-pro | openai/gpt-5.5-pro | included | physical model | - |
| openrouter/openrouter-t2 | openai/gpt-5.5 | openai/gpt-5.5 | included | physical model | - |
| openrouter/openrouter-t2 | openai/gpt-5.6-luna | openai/gpt-5.6-luna | included | physical model | - |
| openrouter/openrouter-t2 | openai/gpt-5.6-sol | openai/gpt-5.6-sol | included | physical model | - |
| openrouter/openrouter-t2 | openai/gpt-5.6-terra | openai/gpt-5.6-terra | included | physical model | - |
| openrouter/openrouter-t2 | openai/gpt-6-astra | openai/gpt-6-astra | included | physical model | - |
| openrouter/openrouter-t2 | openai/gpt-6-luna | openai/gpt-6-luna | included | physical model | - |
| openrouter/openrouter-t2 | openai/gpt-6-sol | openai/gpt-6-sol | included | physical model | - |
| openrouter/openrouter-t2 | openai/gpt-image-2 | openai/gpt-image-2 | included | physical model | - |
| openrouter/openrouter-t2 | openai/gpt-transcribe | openai/gpt-transcribe | included | physical model | - |
| openrouter/openrouter-t2 | openai/text-embedding-3-large | openai/text-embedding-3-large | included | physical model | - |
| openrouter/openrouter-t2 | openai/text-embedding-3-small | openai/text-embedding-3-small | included | physical model | - |
| openrouter/openrouter-t2 | openai/text-embedding-ada-002 | openai/text-embedding-ada-002 | included | physical model | - |
| openrouter/openrouter-t2 | openai/whisper-1 | openai/whisper-1 | included | physical model | - |
| openrouter/openrouter-t2 | qwen/qwen3.5-122b-a10b | qwen/qwen3.5-122b-a10b | included | physical model | - |
| openrouter/openrouter-t2 | qwen/qwen3.5-27b | qwen/qwen3.5-27b | included | physical model | - |
| openrouter/openrouter-t2 | qwen/qwen3.5-35b-a3b | qwen/qwen3.5-35b-a3b | included | physical model | - |
| openrouter/openrouter-t2 | qwen/qwen3.5-397b-a17b | qwen/qwen3.5-397b-a17b | included | physical model | - |
| openrouter/openrouter-t2 | qwen/qwen3.6-35b-a3b | qwen/qwen3.6-35b-a3b | included | physical model | - |
| openrouter/openrouter-t2 | qwen/qwen3.6-flash | qwen/qwen3.6-flash | included | physical model | - |
| openrouter/openrouter-t2 | qwen/qwen3.6-plus | qwen/qwen3.6-plus | included | physical model | - |
| openrouter/openrouter-t2 | qwen/qwen3.7-flash | qwen/qwen3.7-flash | included | physical model | - |
| openrouter/openrouter-t2 | qwen/qwen3.7-max | qwen/qwen3.7-max | included | physical model | - |
| openrouter/openrouter-t2 | qwen/qwen3.7-plus | qwen/qwen3.7-plus | included | physical model | - |
| openrouter/openrouter-t2 | qwen/qwen3.8-2.4t-a95b | qwen/qwen3.8-2.4t-a95b | included | physical model | - |
| openrouter/openrouter-t2 | qwen/qwen3.8-27b | qwen/qwen3.8-27b | included | physical model | - |
| openrouter/openrouter-t2 | qwen/qwen3.8-flash | qwen/qwen3.8-flash | included | physical model | - |
| openrouter/openrouter-t2 | qwen/qwen3.8-max-0902 | qwen/qwen3.8-max-0902 | included | physical model | - |
| openrouter/openrouter-t2 | qwen/qwen3.8-omni-flash | qwen/qwen3.8-omni-flash | included | physical model | - |
| openrouter/openrouter-t2 | typesafe/jev-1.13 | typesafe/jev-1.13 | included | physical model | - |
| openrouter/openrouter-t2 | z-ai/glm-4.5-air | z-ai/glm-4.5-air | included | physical model | - |
| openrouter/openrouter-t2 | z-ai/glm-4.5 | z-ai/glm-4.5 | included | physical model | - |
| openrouter/openrouter-t2 | z-ai/glm-4.5v | z-ai/glm-4.5v | included | physical model | - |
| openrouter/openrouter-t2 | z-ai/glm-4.6 | z-ai/glm-4.6 | included | physical model | - |
| openrouter/openrouter-t2 | z-ai/glm-4.6v | z-ai/glm-4.6v | included | physical model | - |
| openrouter/openrouter-t2 | z-ai/glm-4.7-flash | z-ai/glm-4.7-flash | included | physical model | - |
| openrouter/openrouter-t2 | z-ai/glm-4.7 | z-ai/glm-4.7 | included | physical model | - |
| openrouter/openrouter-t2 | z-ai/glm-5-turbo | z-ai/glm-5-turbo | included | physical model | - |
| openrouter/openrouter-t2 | z-ai/glm-5.1 | z-ai/glm-5.1 | included | physical model | - |
| openrouter/openrouter-t2 | z-ai/glm-5.2 | z-ai/glm-5.2 | included | physical model | - |
| openrouter/openrouter-t2 | z-ai/glm-5.3-flash | z-ai/glm-5.3-flash | included | physical model | - |
| openrouter/openrouter-t2 | z-ai/glm-5.3 | z-ai/glm-5.3 | included | physical model | - |
| openrouter/openrouter-t2 | z-ai/glm-5 | z-ai/glm-5 | included | physical model | - |
| openrouter/openrouter-t2 | z-ai/glm-5v-turbo | z-ai/glm-5v-turbo | included | physical model | - |

## Confirmed product defects

- **defect.aicc.t2.openrouter.openrouter-t2.openai-gpt-5.4-pro.vision.caption** (AICC, assertion_failed): expected the selected exact model satisfies the case routing, capability, resource, and security assertions; observed RPCError: RPC call error: 500. Evidence: cases/t2.openrouter.openrouter-t2.openai-gpt-5.4-pro.vision.caption.json
- **defect.aicc.t2.openrouter.openrouter-t2.qwen-qwen3.7-plus.vision.ocr** (AICC, assertion_failed): expected the selected exact model satisfies the case routing, capability, resource, and security assertions; observed RPCError: RPC call error: 500. Evidence: cases/t2.openrouter.openrouter-t2.qwen-qwen3.7-plus.vision.ocr.json

## Cleanup

Status: passed

- removed 0 uploaded fixture object(s)
- removed 7 generated output object(s)
- temporary Provider credentials restored after testing (openrouter)
