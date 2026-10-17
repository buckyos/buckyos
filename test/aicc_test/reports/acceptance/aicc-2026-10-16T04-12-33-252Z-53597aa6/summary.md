# AICC E2E Acceptance Report

- Run: `aicc-2026-10-16T04-12-33-252Z-53597aa6`
- Commit: `e897c75d6ec5aaa78f72325a27fca221bbd9409e`
- Capability baseline: `2026-10-15.1`
- Real model calls: 15/15
- Planned maximum cost: $0.150000
- Known actual / unknown estimated exposure: $0.055083 / $0.110000
- Total exposure / budget: $0.165083 / $0.500000
- Unknown-cost calls: 11; budget exceeded: false
- Results: passed=0, failed=11, provider_restricted=0, skipped=0, not_applicable=0, review=4

## Targeted retest

Run after fixing the reported defect; repeat `--case` to select additional cases:

```bash
pnpm run acceptance:gateway -- --config aicc_acceptance.local.toml --allow-credential-mutation --timeout-ms 900000 --provider openrouter --case t2.openrouter.openrouter-t2.google-gemini-2.5-flash-image.llm --case t2.openrouter.openrouter-t2.google-gemini-2.5-flash-image.vision.caption --case t2.openrouter.openrouter-t2.google-gemini-2.5-flash-image.vision.ocr --case t2.openrouter.openrouter-t2.google-gemini-nano-banana-2.1.image.img2img --case t2.openrouter.openrouter-t2.google-gemini-nano-banana-2.1.image.txt2img --case t2.openrouter.openrouter-t2.google-gemini-nano-banana-2.1.llm --case t2.openrouter.openrouter-t2.google-gemini-nano-banana-2.1.vision.caption --case t2.openrouter.openrouter-t2.google-gemini-nano-banana-2.1.vision.ocr --case t2.openrouter.openrouter-t2.openai-gpt-5.4-pro.vision.caption --case t2.openrouter.openrouter-t2.openai-gpt-5.4-pro.vision.ocr --case t2.openrouter.openrouter-t2.openai-gpt-5.5-pro.vision.ocr
```

## Cases

| Case | Layer | Provider | Model | API | Status |
|---|---|---|---|---|---|
| t2.openrouter.openrouter-t2.google-gemini-2.5-flash-image.llm | T2 | openrouter/openrouter-t2 | google/gemini-2.5-flash-image@openrouter-t2 | llm | failed |
| t2.openrouter.openrouter-t2.google-gemini-2.5-flash-image.vision.caption | T2 | openrouter/openrouter-t2 | google/gemini-2.5-flash-image@openrouter-t2 | vision.caption | failed |
| t2.openrouter.openrouter-t2.google-gemini-2.5-flash-image.vision.ocr | T2 | openrouter/openrouter-t2 | google/gemini-2.5-flash-image@openrouter-t2 | vision.ocr | failed |
| t2.openrouter.openrouter-t2.google-gemini-3.1-pro-preview-customtools.vision.ocr | T2 | openrouter/openrouter-t2 | google/gemini-3.1-pro-preview-customtools@openrouter-t2 | vision.ocr | review |
| t2.openrouter.openrouter-t2.google-gemini-nano-banana-2.1.image.img2img | T2 | openrouter/openrouter-t2 | google/gemini-nano-banana-2.1@openrouter-t2 | image.img2img | failed |
| t2.openrouter.openrouter-t2.google-gemini-nano-banana-2.1.image.txt2img | T2 | openrouter/openrouter-t2 | google/gemini-nano-banana-2.1@openrouter-t2 | image.txt2img | failed |
| t2.openrouter.openrouter-t2.google-gemini-nano-banana-2.1.llm | T2 | openrouter/openrouter-t2 | google/gemini-nano-banana-2.1@openrouter-t2 | llm | failed |
| t2.openrouter.openrouter-t2.google-gemini-nano-banana-2.1.vision.caption | T2 | openrouter/openrouter-t2 | google/gemini-nano-banana-2.1@openrouter-t2 | vision.caption | failed |
| t2.openrouter.openrouter-t2.google-gemini-nano-banana-2.1.vision.ocr | T2 | openrouter/openrouter-t2 | google/gemini-nano-banana-2.1@openrouter-t2 | vision.ocr | failed |
| t2.openrouter.openrouter-t2.openai-gpt-5.4-pro.vision.caption | T2 | openrouter/openrouter-t2 | openai/gpt-5.4-pro@openrouter-t2 | vision.caption | failed |
| t2.openrouter.openrouter-t2.openai-gpt-5.4-pro.vision.ocr | T2 | openrouter/openrouter-t2 | openai/gpt-5.4-pro@openrouter-t2 | vision.ocr | failed |
| t2.openrouter.openrouter-t2.openai-gpt-5.5-pro.vision.ocr | T2 | openrouter/openrouter-t2 | openai/gpt-5.5-pro@openrouter-t2 | vision.ocr | failed |
| t2.openrouter.openrouter-t2.qwen-qwen3.7-flash.vision.caption | T2 | openrouter/openrouter-t2 | qwen/qwen3.7-flash@openrouter-t2 | vision.caption | review |
| t2.openrouter.openrouter-t2.qwen-qwen3.7-plus.vision.ocr | T2 | openrouter/openrouter-t2 | qwen/qwen3.7-plus@openrouter-t2 | vision.ocr | review |
| t2.openrouter.openrouter-t2.z-ai-glm-4.6.llm | T2 | openrouter/openrouter-t2 | z-ai/glm-4.6@openrouter-t2 | llm | review |

## Physical model coverage

| Provider | Inventory model | Physical model | Decision | Reason | Retained representative |
|---|---|---|---|---|---|
| openrouter/openrouter-t2 | google/gemini-2.5-flash-image | google/gemini-2.5-flash-image | included | physical model | - |
| openrouter/openrouter-t2 | google/gemini-3.1-pro-preview-customtools | google/gemini-3.1-pro-preview-customtools | included | physical model | - |
| openrouter/openrouter-t2 | google/gemini-nano-banana-2.1 | google/gemini-nano-banana-2.1 | included | physical model | - |
| openrouter/openrouter-t2 | openai/gpt-5.4-pro | openai/gpt-5.4-pro | included | physical model | - |
| openrouter/openrouter-t2 | openai/gpt-5.5-pro | openai/gpt-5.5-pro | included | physical model | - |
| openrouter/openrouter-t2 | qwen/qwen3.7-flash | qwen/qwen3.7-flash | included | physical model | - |
| openrouter/openrouter-t2 | qwen/qwen3.7-plus | qwen/qwen3.7-plus | included | physical model | - |
| openrouter/openrouter-t2 | z-ai/glm-4.6 | z-ai/glm-4.6 | included | physical model | - |
| openrouter/openrouter-t2 | google/gemini-2.5-flash-image | google/gemini-2.5-flash-image | included | physical model | - |
| openrouter/openrouter-t2 | google/gemini-3.1-pro-preview-customtools | google/gemini-3.1-pro-preview-customtools | included | physical model | - |
| openrouter/openrouter-t2 | google/gemini-nano-banana-2.1 | google/gemini-nano-banana-2.1 | included | physical model | - |
| openrouter/openrouter-t2 | openai/gpt-5.4-pro | openai/gpt-5.4-pro | included | physical model | - |
| openrouter/openrouter-t2 | openai/gpt-5.5-pro | openai/gpt-5.5-pro | included | physical model | - |
| openrouter/openrouter-t2 | qwen/qwen3.7-flash | qwen/qwen3.7-flash | included | physical model | - |
| openrouter/openrouter-t2 | qwen/qwen3.7-plus | qwen/qwen3.7-plus | included | physical model | - |
| openrouter/openrouter-t2 | z-ai/glm-4.6 | z-ai/glm-4.6 | included | physical model | - |

## Confirmed product defects

- **defect.aicc.t2.openrouter.openrouter-t2.google-gemini-nano-banana-2.1.image.txt2img** (AICC, assertion_failed): expected the selected exact model satisfies the case routing, capability, resource, and security assertions; observed RPCError: RPC call error: Failed due to reason: {"code":"no_candidate_model","message":"exact model unavailable: google/gemini-nano-banana-2.1@openrouter-t2: filtered by circuit_open"}. Evidence: cases/t2.openrouter.openrouter-t2.google-gemini-nano-banana-2.1.image.txt2img.json
- **defect.aicc.t2.openrouter.openrouter-t2.google-gemini-nano-banana-2.1.llm** (AICC, assertion_failed): expected the selected exact model satisfies the case routing, capability, resource, and security assertions; observed RPCError: RPC call error: Failed due to reason: {"code":"no_candidate_model","message":"exact model unavailable: google/gemini-nano-banana-2.1@openrouter-t2: filtered by circuit_open"}. Evidence: cases/t2.openrouter.openrouter-t2.google-gemini-nano-banana-2.1.llm.json
- **defect.aicc.t2.openrouter.openrouter-t2.google-gemini-nano-banana-2.1.vision.caption** (AICC, assertion_failed): expected the selected exact model satisfies the case routing, capability, resource, and security assertions; observed RPCError: RPC call error: Failed due to reason: {"code":"no_candidate_model","message":"exact model unavailable: google/gemini-nano-banana-2.1@openrouter-t2: filtered by circuit_open"}. Evidence: cases/t2.openrouter.openrouter-t2.google-gemini-nano-banana-2.1.vision.caption.json
- **defect.aicc.t2.openrouter.openrouter-t2.google-gemini-nano-banana-2.1.vision.ocr** (AICC, assertion_failed): expected the selected exact model satisfies the case routing, capability, resource, and security assertions; observed RPCError: RPC call error: Failed due to reason: {"code":"no_candidate_model","message":"exact model unavailable: google/gemini-nano-banana-2.1@openrouter-t2: filtered by circuit_open"}. Evidence: cases/t2.openrouter.openrouter-t2.google-gemini-nano-banana-2.1.vision.ocr.json

## Cleanup

Status: passed

- removed 0 uploaded fixture object(s)
- removed 0 generated output object(s)
- temporary Provider credentials restored after testing (openrouter)
