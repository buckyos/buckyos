# AICC E2E Acceptance Report

- Run: `aicc-2026-10-21T11-15-39-265Z-105ab5fa`
- Commit: `53b4431e2d46ec731598ed0343d4a6596bee8d15`
- Capability baseline: `2026-10-21.2`
- Real model calls: 22/23
- Planned maximum cost: $0.230000
- Known actual / unknown estimated exposure: $0.000000 / $0.220000
- Total exposure / budget: $0.220000 / $30.000000
- Unknown-cost calls: 22; budget exceeded: false
- Results: passed=9, failed=4, provider_restricted=0, skipped=0, not_applicable=0, review=9

## Targeted retest

Run after fixing the reported defect; repeat `--case` to select additional cases:

```bash
pnpm run acceptance:gateway -- --config aicc_acceptance.sn.local.toml --allow-credential-mutation --timeout-ms 900000 --provider sn-ai-provider --case t2.sn-ai-provider.sn-issues-45-46-t2.qwen-image-3-pro.image.img2img --case t2.sn-ai-provider.sn-issues-45-46-t2.qwen-image-3-pro.image.txt2img --case t2.sn-ai-provider.sn-issues-45-46-t2.wan-3.0.video.img2video --case t2.sn-ai-provider.sn-issues-45-46-t2.wan-3.0.video.txt2video
```

## Cases

| Case | Layer | Provider | Model | API | Status |
|---|---|---|---|---|---|
| t2.sn-ai-provider.sn-issues-45-46-t2.glm-5.3.llm | T2 | sn-ai-provider/sn-issues-45-46-t2 | glm-5.3@sn-issues-45-46-t2 | llm | passed |
| t2.sn-ai-provider.sn-issues-45-46-t2.glm-5.llm | T2 | sn-ai-provider/sn-issues-45-46-t2 | glm-5@sn-issues-45-46-t2 | llm | passed |
| t2.sn-ai-provider.sn-issues-45-46-t2.kimi-k3.llm | T2 | sn-ai-provider/sn-issues-45-46-t2 | kimi-k3@sn-issues-45-46-t2 | llm | passed |
| t2.sn-ai-provider.sn-issues-45-46-t2.kimi-k3.vision.caption | T2 | sn-ai-provider/sn-issues-45-46-t2 | kimi-k3@sn-issues-45-46-t2 | vision.caption | review |
| t2.sn-ai-provider.sn-issues-45-46-t2.kimi-k3.vision.ocr | T2 | sn-ai-provider/sn-issues-45-46-t2 | kimi-k3@sn-issues-45-46-t2 | vision.ocr | review |
| t2.sn-ai-provider.sn-issues-45-46-t2.qwen-image-3-pro.image.img2img | T2 | sn-ai-provider/sn-issues-45-46-t2 | qwen-image-3-pro@sn-issues-45-46-t2 | image.img2img | failed |
| t2.sn-ai-provider.sn-issues-45-46-t2.qwen-image-3-pro.image.txt2img | T2 | sn-ai-provider/sn-issues-45-46-t2 | qwen-image-3-pro@sn-issues-45-46-t2 | image.txt2img | failed |
| t2.sn-ai-provider.sn-issues-45-46-t2.qwen3-embedding-8b.embedding.text | T2 | sn-ai-provider/sn-issues-45-46-t2 | qwen3-embedding-8b@sn-issues-45-46-t2 | embedding.text | passed |
| t2.sn-ai-provider.sn-issues-45-46-t2.qwen3.6-plus.llm | T2 | sn-ai-provider/sn-issues-45-46-t2 | qwen3.6-plus@sn-issues-45-46-t2 | llm | passed |
| t2.sn-ai-provider.sn-issues-45-46-t2.qwen3.6-plus.vision.caption | T2 | sn-ai-provider/sn-issues-45-46-t2 | qwen3.6-plus@sn-issues-45-46-t2 | vision.caption | review |
| t2.sn-ai-provider.sn-issues-45-46-t2.qwen3.7-plus.llm | T2 | sn-ai-provider/sn-issues-45-46-t2 | qwen3.7-plus@sn-issues-45-46-t2 | llm | passed |
| t2.sn-ai-provider.sn-issues-45-46-t2.qwen3.7-plus.vision.caption | T2 | sn-ai-provider/sn-issues-45-46-t2 | qwen3.7-plus@sn-issues-45-46-t2 | vision.caption | review |
| t2.sn-ai-provider.sn-issues-45-46-t2.qwen3.7-plus.vision.ocr | T2 | sn-ai-provider/sn-issues-45-46-t2 | qwen3.7-plus@sn-issues-45-46-t2 | vision.ocr | review |
| t2.sn-ai-provider.sn-issues-45-46-t2.qwen3.8-max-0902.llm | T2 | sn-ai-provider/sn-issues-45-46-t2 | qwen3.8-max-0902@sn-issues-45-46-t2 | llm | passed |
| t2.sn-ai-provider.sn-issues-45-46-t2.qwen3.8-max-0902.vision.caption | T2 | sn-ai-provider/sn-issues-45-46-t2 | qwen3.8-max-0902@sn-issues-45-46-t2 | vision.caption | review |
| t2.sn-ai-provider.sn-issues-45-46-t2.ui-tars-1.5-7b.llm | T2 | sn-ai-provider/sn-issues-45-46-t2 | ui-tars-1.5-7b@sn-issues-45-46-t2 | llm | passed |
| t2.sn-ai-provider.sn-issues-45-46-t2.ui-tars-1.5-7b.vision.caption | T2 | sn-ai-provider/sn-issues-45-46-t2 | ui-tars-1.5-7b@sn-issues-45-46-t2 | vision.caption | review |
| t2.sn-ai-provider.sn-issues-45-46-t2.ui-tars-1.5-7b.vision.ocr | T2 | sn-ai-provider/sn-issues-45-46-t2 | ui-tars-1.5-7b@sn-issues-45-46-t2 | vision.ocr | review |
| t2.sn-ai-provider.sn-issues-45-46-t2.voxtral-small-24b-2507.audio.asr | T2 | sn-ai-provider/sn-issues-45-46-t2 | voxtral-small-24b-2507@sn-issues-45-46-t2 | audio.asr | review |
| t2.sn-ai-provider.sn-issues-45-46-t2.voxtral-small-24b-2507.llm | T2 | sn-ai-provider/sn-issues-45-46-t2 | voxtral-small-24b-2507@sn-issues-45-46-t2 | llm | passed |
| t2.sn-ai-provider.sn-issues-45-46-t2.wan-3.0.video.img2video | T2 | sn-ai-provider/sn-issues-45-46-t2 | wan-3.0@sn-issues-45-46-t2 | video.img2video | failed |
| t2.sn-ai-provider.sn-issues-45-46-t2.wan-3.0.video.txt2video | T2 | sn-ai-provider/sn-issues-45-46-t2 | wan-3.0@sn-issues-45-46-t2 | video.txt2video | failed |

## Physical model coverage

| Provider | Inventory model | Physical model | Decision | Reason | Retained representative |
|---|---|---|---|---|---|
| sn-ai-provider/sn-issues-45-46-t2 | glm-5.3 | glm-5.3 | included | physical model | - |
| sn-ai-provider/sn-issues-45-46-t2 | glm-5 | glm-5 | included | physical model | - |
| sn-ai-provider/sn-issues-45-46-t2 | kimi-k3 | kimi-k3 | included | physical model | - |
| sn-ai-provider/sn-issues-45-46-t2 | qwen-image-3-pro | qwen-image-3-pro | included | physical model | - |
| sn-ai-provider/sn-issues-45-46-t2 | qwen3-embedding-8b | qwen3-embedding-8b | included | physical model | - |
| sn-ai-provider/sn-issues-45-46-t2 | qwen3.6-plus | qwen3.6-plus | included | physical model | - |
| sn-ai-provider/sn-issues-45-46-t2 | qwen3.7-plus | qwen3.7-plus | included | physical model | - |
| sn-ai-provider/sn-issues-45-46-t2 | qwen3.8-max-0902 | qwen3.8-max-0902 | included | physical model | - |
| sn-ai-provider/sn-issues-45-46-t2 | ui-tars-1.5-7b | ui-tars-1.5-7b | included | physical model | - |
| sn-ai-provider/sn-issues-45-46-t2 | voxtral-small-24b-2507 | voxtral-small-24b-2507 | included | physical model | - |
| sn-ai-provider/sn-issues-45-46-t2 | wan-3.0 | wan-3.0 | included | physical model | - |
| sn-ai-provider/sn-issues-45-46-t2 | glm-5.3 | glm-5.3 | included | physical model | - |
| sn-ai-provider/sn-issues-45-46-t2 | glm-5 | glm-5 | included | physical model | - |
| sn-ai-provider/sn-issues-45-46-t2 | kimi-k3 | kimi-k3 | included | physical model | - |
| sn-ai-provider/sn-issues-45-46-t2 | qwen-image-3-pro | qwen-image-3-pro | included | physical model | - |
| sn-ai-provider/sn-issues-45-46-t2 | qwen3-embedding-8b | qwen3-embedding-8b | included | physical model | - |
| sn-ai-provider/sn-issues-45-46-t2 | qwen3.6-plus | qwen3.6-plus | included | physical model | - |
| sn-ai-provider/sn-issues-45-46-t2 | qwen3.7-plus | qwen3.7-plus | included | physical model | - |
| sn-ai-provider/sn-issues-45-46-t2 | qwen3.8-max-0902 | qwen3.8-max-0902 | included | physical model | - |
| sn-ai-provider/sn-issues-45-46-t2 | ui-tars-1.5-7b | ui-tars-1.5-7b | included | physical model | - |
| sn-ai-provider/sn-issues-45-46-t2 | voxtral-small-24b-2507 | voxtral-small-24b-2507 | included | physical model | - |
| sn-ai-provider/sn-issues-45-46-t2 | wan-3.0 | wan-3.0 | included | physical model | - |

## Confirmed product defects

- **defect.aicc.t2.sn-ai-provider.sn-issues-45-46-t2.qwen-image-3-pro.image.img2img** (AICC, assertion_failed): expected the selected exact model satisfies the case routing, capability, resource, and security assertions; observed RPCError: RPC call error: 500. Evidence: cases/t2.sn-ai-provider.sn-issues-45-46-t2.qwen-image-3-pro.image.img2img.json
- **defect.aicc.t2.sn-ai-provider.sn-issues-45-46-t2.qwen-image-3-pro.image.txt2img** (AICC, assertion_failed): expected the selected exact model satisfies the case routing, capability, resource, and security assertions; observed RPCError: RPC call error: 500. Evidence: cases/t2.sn-ai-provider.sn-issues-45-46-t2.qwen-image-3-pro.image.txt2img.json

## Cleanup

Status: passed

- removed 0 uploaded fixture object(s)
- removed 0 generated output object(s)
- temporary Provider credentials restored after testing (sn-ai-provider)
