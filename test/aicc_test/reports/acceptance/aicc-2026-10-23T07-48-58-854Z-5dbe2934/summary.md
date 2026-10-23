# AICC E2E Acceptance Report

- Run: `aicc-2026-10-23T07-48-58-854Z-5dbe2934`
- Commit: `53b4431e2d46ec731598ed0343d4a6596bee8d15`
- Capability baseline: `2026-10-23.1`
- Real model calls: 6/9
- Planned maximum cost: $0.090000
- Known actual / unknown estimated exposure: $0.000000 / $0.060000
- Total exposure / budget: $0.060000 / $30.000000
- Unknown-cost calls: 6; budget exceeded: false
- Results: passed=0, failed=7, provider_restricted=0, skipped=0, not_applicable=0, review=0

## Targeted retest

Run after fixing the reported defect; repeat `--case` to select additional cases:

```bash
pnpm run acceptance:gateway -- --config aicc_acceptance.sn.local.toml --allow-credential-mutation --timeout-ms 900000 --provider sn-ai-provider --case t2.sn-ai-provider.sn-issues-45-46-t2.qwen-image-3-pro.image.img2img --case t2.sn-ai-provider.sn-issues-45-46-t2.qwen-image-3-pro.image.txt2img --case t2.sn-ai-provider.sn-issues-45-46-t2.qwen3.7-plus.vision.ocr --case t2.sn-ai-provider.sn-issues-45-46-t2.wan-3.0.video.extend --case t2.sn-ai-provider.sn-issues-45-46-t2.wan-3.0.video.img2video --case t2.sn-ai-provider.sn-issues-45-46-t2.wan-3.0.video.txt2video --case t2.sn-ai-provider.sn-issues-45-46-t2.wan-3.0.video.video2video
```

## Cases

| Case | Layer | Provider | Model | API | Status |
|---|---|---|---|---|---|
| t2.sn-ai-provider.sn-issues-45-46-t2.qwen-image-3-pro.image.img2img | T2 | sn-ai-provider/sn-issues-45-46-t2 | qwen-image-3-pro@sn-issues-45-46-t2 | image.img2img | failed |
| t2.sn-ai-provider.sn-issues-45-46-t2.qwen-image-3-pro.image.txt2img | T2 | sn-ai-provider/sn-issues-45-46-t2 | qwen-image-3-pro@sn-issues-45-46-t2 | image.txt2img | failed |
| t2.sn-ai-provider.sn-issues-45-46-t2.qwen3.7-plus.vision.ocr | T2 | sn-ai-provider/sn-issues-45-46-t2 | qwen3.7-plus@sn-issues-45-46-t2 | vision.ocr | failed |
| t2.sn-ai-provider.sn-issues-45-46-t2.wan-3.0.video.extend | T2 | sn-ai-provider/sn-issues-45-46-t2 | wan-3.0@sn-issues-45-46-t2 | video.extend | failed |
| t2.sn-ai-provider.sn-issues-45-46-t2.wan-3.0.video.img2video | T2 | sn-ai-provider/sn-issues-45-46-t2 | wan-3.0@sn-issues-45-46-t2 | video.img2video | failed |
| t2.sn-ai-provider.sn-issues-45-46-t2.wan-3.0.video.txt2video | T2 | sn-ai-provider/sn-issues-45-46-t2 | wan-3.0@sn-issues-45-46-t2 | video.txt2video | failed |
| t2.sn-ai-provider.sn-issues-45-46-t2.wan-3.0.video.video2video | T2 | sn-ai-provider/sn-issues-45-46-t2 | wan-3.0@sn-issues-45-46-t2 | video.video2video | failed |

## Physical model coverage

| Provider | Inventory model | Physical model | Decision | Reason | Retained representative |
|---|---|---|---|---|---|
| sn-ai-provider/sn-issues-45-46-t2 | qwen-image-3-pro | qwen-image-3-pro | included | physical model | - |
| sn-ai-provider/sn-issues-45-46-t2 | qwen3.7-plus | qwen3.7-plus | included | physical model | - |
| sn-ai-provider/sn-issues-45-46-t2 | wan-3.0 | wan-3.0 | included | physical model | - |
| sn-ai-provider/sn-issues-45-46-t2 | qwen-image-3-pro | qwen-image-3-pro | included | physical model | - |
| sn-ai-provider/sn-issues-45-46-t2 | qwen3.7-plus | qwen3.7-plus | included | physical model | - |
| sn-ai-provider/sn-issues-45-46-t2 | wan-3.0 | wan-3.0 | included | physical model | - |

## Confirmed product defects

- **defect.aicc.t2.sn-ai-provider.sn-issues-45-46-t2.qwen3.7-plus.vision.ocr** (AICC, assertion_failed): expected the selected exact model satisfies the case routing, capability, resource, and security assertions; observed RPCError: RPC call error: Failed due to reason: {"code":"no_candidate_model","message":"exact model unavailable: qwen3.7-plus@sn-issues-45-46-t2: filtered by circuit_open"}. Evidence: cases/t2.sn-ai-provider.sn-issues-45-46-t2.qwen3.7-plus.vision.ocr.json
- **defect.aicc.t2.sn-ai-provider.sn-issues-45-46-t2.wan-3.0.video.extend** (AICC, assertion_failed): expected the selected exact model satisfies the case routing, capability, resource, and security assertions; observed RPCError: RPC call error: Failed due to reason: {"code":"no_candidate_model","message":"exact model unavailable: wan-3.0@sn-issues-45-46-t2: filtered by circuit_open"}. Evidence: cases/t2.sn-ai-provider.sn-issues-45-46-t2.wan-3.0.video.extend.json
- **defect.aicc.t2.sn-ai-provider.sn-issues-45-46-t2.wan-3.0.video.img2video** (AICC, assertion_failed): expected the selected exact model satisfies the case routing, capability, resource, and security assertions; observed RPCError: RPC call error: Failed due to reason: {"code":"no_candidate_model","message":"exact model unavailable: wan-3.0@sn-issues-45-46-t2: filtered by circuit_open"}. Evidence: cases/t2.sn-ai-provider.sn-issues-45-46-t2.wan-3.0.video.img2video.json
- **defect.aicc.t2.sn-ai-provider.sn-issues-45-46-t2.wan-3.0.video.txt2video** (AICC, assertion_failed): expected the selected exact model satisfies the case routing, capability, resource, and security assertions; observed RPCError: RPC call error: Failed due to reason: {"code":"no_candidate_model","message":"exact model unavailable: wan-3.0@sn-issues-45-46-t2: filtered by circuit_open"}. Evidence: cases/t2.sn-ai-provider.sn-issues-45-46-t2.wan-3.0.video.txt2video.json
- **defect.aicc.t2.sn-ai-provider.sn-issues-45-46-t2.wan-3.0.video.video2video** (AICC, assertion_failed): expected the selected exact model satisfies the case routing, capability, resource, and security assertions; observed RPCError: RPC call error: Failed due to reason: {"code":"no_candidate_model","message":"exact model unavailable: wan-3.0@sn-issues-45-46-t2: filtered by circuit_open"}. Evidence: cases/t2.sn-ai-provider.sn-issues-45-46-t2.wan-3.0.video.video2video.json

## Cleanup

Status: passed

- removed 0 uploaded fixture object(s)
- removed 0 generated output object(s)
- temporary Provider credentials restored after testing (sn-ai-provider)
