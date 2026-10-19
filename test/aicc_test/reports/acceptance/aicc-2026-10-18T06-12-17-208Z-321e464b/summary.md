# AICC E2E Acceptance Report

- Run: `aicc-2026-10-18T06-12-17-208Z-321e464b`
- Commit: `1e8f93081e1a1b9561e77a78e3f7cb67f9922330`
- Capability baseline: `2026-10-18.1`
- Real model calls: 0/21
- Planned maximum cost: $0.210000
- Known actual / unknown estimated exposure: $0.000000 / $0.000000
- Total exposure / budget: $0.000000 / $5.000000
- Unknown-cost calls: 0; budget exceeded: false
- Results: passed=0, failed=0, provider_restricted=0, skipped=21, not_applicable=0, review=0

## Targeted retest

Run after fixing the reported defect; repeat `--case` to select additional cases:

```bash
pnpm run acceptance:gateway -- --config aicc_acceptance.local.toml --allow-credential-mutation --timeout-ms 900000 --no-real-model-calls
```

## Cases

| Case | Layer | Provider | Model | API | Status |
|---|---|---|---|---|---|
| t2.sn-ai-provider.sn-ai-provider-default.glm-5.3.llm | T2 | sn-ai-provider/sn-ai-provider-default | glm-5.3@sn-ai-provider-default | llm | skipped |
| t2.sn-ai-provider.sn-ai-provider-default.glm-5.llm | T2 | sn-ai-provider/sn-ai-provider-default | glm-5@sn-ai-provider-default | llm | skipped |
| t2.sn-ai-provider.sn-ai-provider-default.kimi-k3.llm | T2 | sn-ai-provider/sn-ai-provider-default | kimi-k3@sn-ai-provider-default | llm | skipped |
| t2.sn-ai-provider.sn-ai-provider-default.kimi-k3.vision.caption | T2 | sn-ai-provider/sn-ai-provider-default | kimi-k3@sn-ai-provider-default | vision.caption | skipped |
| t2.sn-ai-provider.sn-ai-provider-default.kimi-k3.vision.ocr | T2 | sn-ai-provider/sn-ai-provider-default | kimi-k3@sn-ai-provider-default | vision.ocr | skipped |
| t2.sn-ai-provider.sn-ai-provider-default.qwen-image-3-pro.image.img2img | T2 | sn-ai-provider/sn-ai-provider-default | qwen-image-3-pro@sn-ai-provider-default | image.img2img | skipped |
| t2.sn-ai-provider.sn-ai-provider-default.qwen-image-3-pro.image.txt2img | T2 | sn-ai-provider/sn-ai-provider-default | qwen-image-3-pro@sn-ai-provider-default | image.txt2img | skipped |
| t2.sn-ai-provider.sn-ai-provider-default.qwen3-embedding-8b.embedding.text | T2 | sn-ai-provider/sn-ai-provider-default | qwen3-embedding-8b@sn-ai-provider-default | embedding.text | skipped |
| t2.sn-ai-provider.sn-ai-provider-default.qwen3.6-plus.llm | T2 | sn-ai-provider/sn-ai-provider-default | qwen3.6-plus@sn-ai-provider-default | llm | skipped |
| t2.sn-ai-provider.sn-ai-provider-default.qwen3.6-plus.vision.caption | T2 | sn-ai-provider/sn-ai-provider-default | qwen3.6-plus@sn-ai-provider-default | vision.caption | skipped |
| t2.sn-ai-provider.sn-ai-provider-default.qwen3.6-plus.vision.ocr | T2 | sn-ai-provider/sn-ai-provider-default | qwen3.6-plus@sn-ai-provider-default | vision.ocr | skipped |
| t2.sn-ai-provider.sn-ai-provider-default.qwen3.7-plus.llm | T2 | sn-ai-provider/sn-ai-provider-default | qwen3.7-plus@sn-ai-provider-default | llm | skipped |
| t2.sn-ai-provider.sn-ai-provider-default.qwen3.7-plus.vision.caption | T2 | sn-ai-provider/sn-ai-provider-default | qwen3.7-plus@sn-ai-provider-default | vision.caption | skipped |
| t2.sn-ai-provider.sn-ai-provider-default.qwen3.7-plus.vision.ocr | T2 | sn-ai-provider/sn-ai-provider-default | qwen3.7-plus@sn-ai-provider-default | vision.ocr | skipped |
| t2.sn-ai-provider.sn-ai-provider-default.qwen3.8-max-0902.llm | T2 | sn-ai-provider/sn-ai-provider-default | qwen3.8-max-0902@sn-ai-provider-default | llm | skipped |
| t2.sn-ai-provider.sn-ai-provider-default.qwen3.8-max-0902.vision.caption | T2 | sn-ai-provider/sn-ai-provider-default | qwen3.8-max-0902@sn-ai-provider-default | vision.caption | skipped |
| t2.sn-ai-provider.sn-ai-provider-default.qwen3.8-max-0902.vision.ocr | T2 | sn-ai-provider/sn-ai-provider-default | qwen3.8-max-0902@sn-ai-provider-default | vision.ocr | skipped |
| t2.sn-ai-provider.sn-ai-provider-default.ui-tars-1.5-7b.llm | T2 | sn-ai-provider/sn-ai-provider-default | ui-tars-1.5-7b@sn-ai-provider-default | llm | skipped |
| t2.sn-ai-provider.sn-ai-provider-default.ui-tars-1.5-7b.vision.caption | T2 | sn-ai-provider/sn-ai-provider-default | ui-tars-1.5-7b@sn-ai-provider-default | vision.caption | skipped |
| t2.sn-ai-provider.sn-ai-provider-default.ui-tars-1.5-7b.vision.ocr | T2 | sn-ai-provider/sn-ai-provider-default | ui-tars-1.5-7b@sn-ai-provider-default | vision.ocr | skipped |
| t2.sn-ai-provider.sn-ai-provider-default.voxtral-small-24b-2507.llm | T2 | sn-ai-provider/sn-ai-provider-default | voxtral-small-24b-2507@sn-ai-provider-default | llm | skipped |

## Physical model coverage

| Provider | Inventory model | Physical model | Decision | Reason | Retained representative |
|---|---|---|---|---|---|
| sn-ai-provider/sn-ai-provider-default | glm-5.3 | glm-5.3 | included | physical model | - |
| sn-ai-provider/sn-ai-provider-default | glm-5 | glm-5 | included | physical model | - |
| sn-ai-provider/sn-ai-provider-default | kimi-k3 | kimi-k3 | included | physical model | - |
| sn-ai-provider/sn-ai-provider-default | qwen-image-3-pro | qwen-image-3-pro | included | physical model | - |
| sn-ai-provider/sn-ai-provider-default | qwen3-embedding-8b | qwen3-embedding-8b | included | physical model | - |
| sn-ai-provider/sn-ai-provider-default | qwen3.6-plus | qwen3.6-plus | included | physical model | - |
| sn-ai-provider/sn-ai-provider-default | qwen3.7-plus | qwen3.7-plus | included | physical model | - |
| sn-ai-provider/sn-ai-provider-default | qwen3.8-max-0902 | qwen3.8-max-0902 | included | physical model | - |
| sn-ai-provider/sn-ai-provider-default | ui-tars-1.5-7b | ui-tars-1.5-7b | included | physical model | - |
| sn-ai-provider/sn-ai-provider-default | voxtral-small-24b-2507 | voxtral-small-24b-2507 | included | physical model | - |
| sn-ai-provider/sn-ai-provider-default | wan-3.0 | wan-3.0 | filtered | unsupported_canonical_protocol | - |
| sn-ai-provider/sn-ai-provider-default | glm-5.3 | glm-5.3 | included | physical model | - |
| sn-ai-provider/sn-ai-provider-default | glm-5 | glm-5 | included | physical model | - |
| sn-ai-provider/sn-ai-provider-default | kimi-k3 | kimi-k3 | included | physical model | - |
| sn-ai-provider/sn-ai-provider-default | qwen-image-3-pro | qwen-image-3-pro | included | physical model | - |
| sn-ai-provider/sn-ai-provider-default | qwen3-embedding-8b | qwen3-embedding-8b | included | physical model | - |
| sn-ai-provider/sn-ai-provider-default | qwen3.6-plus | qwen3.6-plus | included | physical model | - |
| sn-ai-provider/sn-ai-provider-default | qwen3.7-plus | qwen3.7-plus | included | physical model | - |
| sn-ai-provider/sn-ai-provider-default | qwen3.8-max-0902 | qwen3.8-max-0902 | included | physical model | - |
| sn-ai-provider/sn-ai-provider-default | ui-tars-1.5-7b | ui-tars-1.5-7b | included | physical model | - |
| sn-ai-provider/sn-ai-provider-default | voxtral-small-24b-2507 | voxtral-small-24b-2507 | included | physical model | - |

## Confirmed product defects

None.

## Cleanup

Status: passed

- removed 0 uploaded fixture object(s)
- removed 0 generated output object(s)
