# AICC E2E Acceptance Report

- Run: `aicc-2026-10-30T08-48-26-328Z-e63f03c2`
- Commit: `d40291a9a9085d1cec7ccb37d519d9115baf534c`
- Capability baseline: `2026-10-30.1`
- Real model calls: 0/44
- Planned maximum cost: $0.880000
- Known actual / unknown estimated exposure: $0.000000 / $0.000000
- Total exposure / budget: $0.000000 / $10.000000
- Unknown-cost calls: 0; budget exceeded: false
- Results: passed=0, failed=0, provider_restricted=0, skipped=42, not_applicable=0, review=0

## Targeted retest

Run after fixing the reported defect; repeat `--case` to select additional cases:

```bash
pnpm run acceptance:gateway -- --config aicc_acceptance.sn-global-open-20261030.local.toml --allow-credential-mutation --timeout-ms 900000 --no-real-model-calls
```

## Cases

| Case | Layer | Provider | Model | API | Status |
|---|---|---|---|---|---|
| t2.sn-ai-provider.sn-global-open-20261030-t2.deepseek-v4-pro-0813.llm | T2 | sn-ai-provider/sn-global-open-20261030-t2 | deepseek-v4-pro-0813@sn-global-open-20261030-t2 | llm | skipped |
| t2.sn-ai-provider.sn-global-open-20261030-t2.deepseek-v4.1-flash-1017.llm | T2 | sn-ai-provider/sn-global-open-20261030-t2 | deepseek-v4.1-flash-1017@sn-global-open-20261030-t2 | llm | skipped |
| t2.sn-ai-provider.sn-global-open-20261030-t2.deepseek-v4.1-flash-1017.vision.caption | T2 | sn-ai-provider/sn-global-open-20261030-t2 | deepseek-v4.1-flash-1017@sn-global-open-20261030-t2 | vision.caption | skipped |
| t2.sn-ai-provider.sn-global-open-20261030-t2.deepseek-v4.1-flash-1017.vision.ocr | T2 | sn-ai-provider/sn-global-open-20261030-t2 | deepseek-v4.1-flash-1017@sn-global-open-20261030-t2 | vision.ocr | skipped |
| t2.sn-ai-provider.sn-global-open-20261030-t2.glm-5.3-flash.llm | T2 | sn-ai-provider/sn-global-open-20261030-t2 | glm-5.3-flash@sn-global-open-20261030-t2 | llm | skipped |
| t2.sn-ai-provider.sn-global-open-20261030-t2.glm-5.3-flash.vision.caption | T2 | sn-ai-provider/sn-global-open-20261030-t2 | glm-5.3-flash@sn-global-open-20261030-t2 | vision.caption | skipped |
| t2.sn-ai-provider.sn-global-open-20261030-t2.glm-5.3-flash.vision.ocr | T2 | sn-ai-provider/sn-global-open-20261030-t2 | glm-5.3-flash@sn-global-open-20261030-t2 | vision.ocr | skipped |
| t2.sn-ai-provider.sn-global-open-20261030-t2.glm-5.3.llm | T2 | sn-ai-provider/sn-global-open-20261030-t2 | glm-5.3@sn-global-open-20261030-t2 | llm | skipped |
| t2.sn-ai-provider.sn-global-open-20261030-t2.glm-5.llm | T2 | sn-ai-provider/sn-global-open-20261030-t2 | glm-5@sn-global-open-20261030-t2 | llm | skipped |
| t2.sn-ai-provider.sn-global-open-20261030-t2.kimi-k3.1-pro.llm | T2 | sn-ai-provider/sn-global-open-20261030-t2 | kimi-k3.1-pro@sn-global-open-20261030-t2 | llm | skipped |
| t2.sn-ai-provider.sn-global-open-20261030-t2.kimi-k3.1-pro.vision.caption | T2 | sn-ai-provider/sn-global-open-20261030-t2 | kimi-k3.1-pro@sn-global-open-20261030-t2 | vision.caption | skipped |
| t2.sn-ai-provider.sn-global-open-20261030-t2.kimi-k3.1-pro.vision.ocr | T2 | sn-ai-provider/sn-global-open-20261030-t2 | kimi-k3.1-pro@sn-global-open-20261030-t2 | vision.ocr | skipped |
| t2.sn-ai-provider.sn-global-open-20261030-t2.kimi-k3.llm | T2 | sn-ai-provider/sn-global-open-20261030-t2 | kimi-k3@sn-global-open-20261030-t2 | llm | skipped |
| t2.sn-ai-provider.sn-global-open-20261030-t2.kimi-k3.vision.caption | T2 | sn-ai-provider/sn-global-open-20261030-t2 | kimi-k3@sn-global-open-20261030-t2 | vision.caption | skipped |
| t2.sn-ai-provider.sn-global-open-20261030-t2.kimi-k3.vision.ocr | T2 | sn-ai-provider/sn-global-open-20261030-t2 | kimi-k3@sn-global-open-20261030-t2 | vision.ocr | skipped |
| t2.sn-ai-provider.sn-global-open-20261030-t2.minimax-m3.1-flash.llm | T2 | sn-ai-provider/sn-global-open-20261030-t2 | minimax-m3.1-flash@sn-global-open-20261030-t2 | llm | skipped |
| t2.sn-ai-provider.sn-global-open-20261030-t2.minimax-m3.1-flash.vision.caption | T2 | sn-ai-provider/sn-global-open-20261030-t2 | minimax-m3.1-flash@sn-global-open-20261030-t2 | vision.caption | skipped |
| t2.sn-ai-provider.sn-global-open-20261030-t2.minimax-m3.1-flash.vision.ocr | T2 | sn-ai-provider/sn-global-open-20261030-t2 | minimax-m3.1-flash@sn-global-open-20261030-t2 | vision.ocr | skipped |
| t2.sn-ai-provider.sn-global-open-20261030-t2.qwen-image-3-pro.image.img2img | T2 | sn-ai-provider/sn-global-open-20261030-t2 | qwen-image-3-pro@sn-global-open-20261030-t2 | image.img2img | skipped |
| t2.sn-ai-provider.sn-global-open-20261030-t2.qwen-image-3-pro.image.txt2img | T2 | sn-ai-provider/sn-global-open-20261030-t2 | qwen-image-3-pro@sn-global-open-20261030-t2 | image.txt2img | skipped |
| t2.sn-ai-provider.sn-global-open-20261030-t2.qwen3.6-plus.llm | T2 | sn-ai-provider/sn-global-open-20261030-t2 | qwen3.6-plus@sn-global-open-20261030-t2 | llm | skipped |
| t2.sn-ai-provider.sn-global-open-20261030-t2.qwen3.6-plus.vision.caption | T2 | sn-ai-provider/sn-global-open-20261030-t2 | qwen3.6-plus@sn-global-open-20261030-t2 | vision.caption | skipped |
| t2.sn-ai-provider.sn-global-open-20261030-t2.qwen3.7-plus.llm | T2 | sn-ai-provider/sn-global-open-20261030-t2 | qwen3.7-plus@sn-global-open-20261030-t2 | llm | skipped |
| t2.sn-ai-provider.sn-global-open-20261030-t2.qwen3.7-plus.vision.caption | T2 | sn-ai-provider/sn-global-open-20261030-t2 | qwen3.7-plus@sn-global-open-20261030-t2 | vision.caption | skipped |
| t2.sn-ai-provider.sn-global-open-20261030-t2.qwen3.7-plus.vision.ocr | T2 | sn-ai-provider/sn-global-open-20261030-t2 | qwen3.7-plus@sn-global-open-20261030-t2 | vision.ocr | skipped |
| t2.sn-ai-provider.sn-global-open-20261030-t2.qwen3.8-2.4t-a95b.llm | T2 | sn-ai-provider/sn-global-open-20261030-t2 | qwen3.8-2.4t-a95b@sn-global-open-20261030-t2 | llm | skipped |
| t2.sn-ai-provider.sn-global-open-20261030-t2.qwen3.8-27b.llm | T2 | sn-ai-provider/sn-global-open-20261030-t2 | qwen3.8-27b@sn-global-open-20261030-t2 | llm | skipped |
| t2.sn-ai-provider.sn-global-open-20261030-t2.qwen3.8-flash.llm | T2 | sn-ai-provider/sn-global-open-20261030-t2 | qwen3.8-flash@sn-global-open-20261030-t2 | llm | skipped |
| t2.sn-ai-provider.sn-global-open-20261030-t2.qwen3.8-flash.vision.caption | T2 | sn-ai-provider/sn-global-open-20261030-t2 | qwen3.8-flash@sn-global-open-20261030-t2 | vision.caption | skipped |
| t2.sn-ai-provider.sn-global-open-20261030-t2.qwen3.8-flash.vision.ocr | T2 | sn-ai-provider/sn-global-open-20261030-t2 | qwen3.8-flash@sn-global-open-20261030-t2 | vision.ocr | skipped |
| t2.sn-ai-provider.sn-global-open-20261030-t2.qwen3.8-max-0902.llm | T2 | sn-ai-provider/sn-global-open-20261030-t2 | qwen3.8-max-0902@sn-global-open-20261030-t2 | llm | skipped |
| t2.sn-ai-provider.sn-global-open-20261030-t2.qwen3.8-max-0902.vision.caption | T2 | sn-ai-provider/sn-global-open-20261030-t2 | qwen3.8-max-0902@sn-global-open-20261030-t2 | vision.caption | skipped |
| t2.sn-ai-provider.sn-global-open-20261030-t2.qwen3.8-omni-flash.llm | T2 | sn-ai-provider/sn-global-open-20261030-t2 | qwen3.8-omni-flash@sn-global-open-20261030-t2 | llm | skipped |
| t2.sn-ai-provider.sn-global-open-20261030-t2.qwen3.8-omni-flash.vision.caption | T2 | sn-ai-provider/sn-global-open-20261030-t2 | qwen3.8-omni-flash@sn-global-open-20261030-t2 | vision.caption | skipped |
| t2.sn-ai-provider.sn-global-open-20261030-t2.ui-tars-1.5-7b.llm | T2 | sn-ai-provider/sn-global-open-20261030-t2 | ui-tars-1.5-7b@sn-global-open-20261030-t2 | llm | skipped |
| t2.sn-ai-provider.sn-global-open-20261030-t2.ui-tars-1.5-7b.vision.caption | T2 | sn-ai-provider/sn-global-open-20261030-t2 | ui-tars-1.5-7b@sn-global-open-20261030-t2 | vision.caption | skipped |
| t2.sn-ai-provider.sn-global-open-20261030-t2.ui-tars-1.5-7b.vision.ocr | T2 | sn-ai-provider/sn-global-open-20261030-t2 | ui-tars-1.5-7b@sn-global-open-20261030-t2 | vision.ocr | skipped |
| t2.sn-ai-provider.sn-global-open-20261030-t2.voxtral-small-24b-2507.audio.asr | T2 | sn-ai-provider/sn-global-open-20261030-t2 | voxtral-small-24b-2507@sn-global-open-20261030-t2 | audio.asr | skipped |
| t2.sn-ai-provider.sn-global-open-20261030-t2.voxtral-small-24b-2507.llm | T2 | sn-ai-provider/sn-global-open-20261030-t2 | voxtral-small-24b-2507@sn-global-open-20261030-t2 | llm | skipped |
| t2.sn-ai-provider.sn-global-open-20261030-t2.wan-3.0.video.edit | T2 | sn-ai-provider/sn-global-open-20261030-t2 | wan-3.0@sn-global-open-20261030-t2 | video.edit | skipped |
| t2.sn-ai-provider.sn-global-open-20261030-t2.wan-3.0.video.extend | T2 | sn-ai-provider/sn-global-open-20261030-t2 | wan-3.0@sn-global-open-20261030-t2 | video.extend | skipped |
| t2.sn-ai-provider.sn-global-open-20261030-t2.wan-3.0.video.generate | T2 | sn-ai-provider/sn-global-open-20261030-t2 | wan-3.0@sn-global-open-20261030-t2 | video.generate | skipped |

## Physical model coverage

| Provider | Inventory model | Physical model | Decision | Reason | Retained representative |
|---|---|---|---|---|---|
| sn-ai-provider/sn-global-open-20261030-t2 | deepseek-v4-pro-0813 | deepseek-v4-pro-0813 | included | physical model | - |
| sn-ai-provider/sn-global-open-20261030-t2 | deepseek-v4.1-flash-1017 | deepseek-v4.1-flash-1017 | included | physical model | - |
| sn-ai-provider/sn-global-open-20261030-t2 | glm-5.3-flash | glm-5.3-flash | included | physical model | - |
| sn-ai-provider/sn-global-open-20261030-t2 | glm-5.3-flashx | glm-5.3-flash | filtered | duplicate_physical_model | glm-5.3-flash@sn-global-open-20261030-t2 |
| sn-ai-provider/sn-global-open-20261030-t2 | glm-5.3-prime | glm-5.3 | filtered | duplicate_physical_model | glm-5.3@sn-global-open-20261030-t2 |
| sn-ai-provider/sn-global-open-20261030-t2 | glm-5.3 | glm-5.3 | included | physical model | - |
| sn-ai-provider/sn-global-open-20261030-t2 | glm-5 | glm-5 | included | physical model | - |
| sn-ai-provider/sn-global-open-20261030-t2 | kimi-k3.1-pro | kimi-k3.1-pro | included | physical model | - |
| sn-ai-provider/sn-global-open-20261030-t2 | kimi-k3 | kimi-k3 | included | physical model | - |
| sn-ai-provider/sn-global-open-20261030-t2 | minimax-m3.1-flash | minimax-m3.1-flash | included | physical model | - |
| sn-ai-provider/sn-global-open-20261030-t2 | qwen-image-3-pro | qwen-image-3-pro | included | physical model | - |
| sn-ai-provider/sn-global-open-20261030-t2 | qwen3.6-plus | qwen3.6-plus | included | physical model | - |
| sn-ai-provider/sn-global-open-20261030-t2 | qwen3.7-plus | qwen3.7-plus | included | physical model | - |
| sn-ai-provider/sn-global-open-20261030-t2 | qwen3.8-2.4t-a95b | qwen3.8-2.4t-a95b | included | physical model | - |
| sn-ai-provider/sn-global-open-20261030-t2 | qwen3.8-27b | qwen3.8-27b | included | physical model | - |
| sn-ai-provider/sn-global-open-20261030-t2 | qwen3.8-flash | qwen3.8-flash | included | physical model | - |
| sn-ai-provider/sn-global-open-20261030-t2 | qwen3.8-max-0902 | qwen3.8-max-0902 | included | physical model | - |
| sn-ai-provider/sn-global-open-20261030-t2 | qwen3.8-max-prime | qwen3.8-max-0902 | filtered | duplicate_physical_model | qwen3.8-max-0902@sn-global-open-20261030-t2 |
| sn-ai-provider/sn-global-open-20261030-t2 | qwen3.8-omni-flash | qwen3.8-omni-flash | included | physical model | - |
| sn-ai-provider/sn-global-open-20261030-t2 | ui-tars-1.5-7b | ui-tars-1.5-7b | included | physical model | - |
| sn-ai-provider/sn-global-open-20261030-t2 | voxtral-small-24b-2507 | voxtral-small-24b-2507 | included | physical model | - |
| sn-ai-provider/sn-global-open-20261030-t2 | wan-3.0 | wan-3.0 | included | physical model | - |
| sn-ai-provider/sn-global-open-20261030-t2 | deepseek-v4-pro-0813 | deepseek-v4-pro-0813 | included | physical model | - |
| sn-ai-provider/sn-global-open-20261030-t2 | deepseek-v4.1-flash-1017 | deepseek-v4.1-flash-1017 | included | physical model | - |
| sn-ai-provider/sn-global-open-20261030-t2 | glm-5.3-flash | glm-5.3-flash | included | physical model | - |
| sn-ai-provider/sn-global-open-20261030-t2 | glm-5.3-flashx | glm-5.3-flash | filtered | duplicate_physical_model | glm-5.3-flash@sn-global-open-20261030-t2 |
| sn-ai-provider/sn-global-open-20261030-t2 | glm-5.3-prime | glm-5.3 | filtered | duplicate_physical_model | glm-5.3@sn-global-open-20261030-t2 |
| sn-ai-provider/sn-global-open-20261030-t2 | glm-5.3 | glm-5.3 | included | physical model | - |
| sn-ai-provider/sn-global-open-20261030-t2 | glm-5 | glm-5 | included | physical model | - |
| sn-ai-provider/sn-global-open-20261030-t2 | kimi-k3.1-pro | kimi-k3.1-pro | included | physical model | - |
| sn-ai-provider/sn-global-open-20261030-t2 | kimi-k3 | kimi-k3 | included | physical model | - |
| sn-ai-provider/sn-global-open-20261030-t2 | minimax-m3.1-flash | minimax-m3.1-flash | included | physical model | - |
| sn-ai-provider/sn-global-open-20261030-t2 | qwen-image-3-pro | qwen-image-3-pro | included | physical model | - |
| sn-ai-provider/sn-global-open-20261030-t2 | qwen3.6-plus | qwen3.6-plus | included | physical model | - |
| sn-ai-provider/sn-global-open-20261030-t2 | qwen3.7-plus | qwen3.7-plus | included | physical model | - |
| sn-ai-provider/sn-global-open-20261030-t2 | qwen3.8-2.4t-a95b | qwen3.8-2.4t-a95b | included | physical model | - |
| sn-ai-provider/sn-global-open-20261030-t2 | qwen3.8-27b | qwen3.8-27b | included | physical model | - |
| sn-ai-provider/sn-global-open-20261030-t2 | qwen3.8-flash | qwen3.8-flash | included | physical model | - |
| sn-ai-provider/sn-global-open-20261030-t2 | qwen3.8-max-0902 | qwen3.8-max-0902 | included | physical model | - |
| sn-ai-provider/sn-global-open-20261030-t2 | qwen3.8-max-prime | qwen3.8-max-0902 | filtered | duplicate_physical_model | qwen3.8-max-0902@sn-global-open-20261030-t2 |
| sn-ai-provider/sn-global-open-20261030-t2 | qwen3.8-omni-flash | qwen3.8-omni-flash | included | physical model | - |
| sn-ai-provider/sn-global-open-20261030-t2 | ui-tars-1.5-7b | ui-tars-1.5-7b | included | physical model | - |
| sn-ai-provider/sn-global-open-20261030-t2 | voxtral-small-24b-2507 | voxtral-small-24b-2507 | included | physical model | - |
| sn-ai-provider/sn-global-open-20261030-t2 | wan-3.0 | wan-3.0 | included | physical model | - |

## Confirmed product defects

None.

## Cleanup

Status: passed

- removed 0 uploaded fixture object(s)
- removed 0 generated output object(s)
- temporary Provider credentials restored after testing (sn-ai-provider)
