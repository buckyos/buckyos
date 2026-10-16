# AICC E2E Acceptance Report

- Run: `aicc-2026-10-09T15-43-32-579Z-7fa08d7b`
- Commit: `0c32470f0ebaa0f14a1a262bc2bcb0eea553d8fc`
- Capability baseline: `2026-10-09.7`
- Real model calls: 28/28
- Planned maximum cost: $0.280000
- Known actual / unknown estimated exposure: $0.000000 / $0.280000
- Total exposure / budget: $0.280000 / $1.000000
- Unknown-cost calls: 28; budget exceeded: false
- Results: passed=14, failed=0, provider_restricted=0, skipped=0, not_applicable=0, review=0

## Targeted retest

Run after fixing the reported defect; repeat `--case` to select additional cases:

```bash
pnpm run acceptance:gateway -- --config aicc_acceptance.local.toml --allow-credential-mutation --timeout-ms 900000 --no-real-model-calls
```

## Cases

| Case | Layer | Provider | Model | API | Status |
|---|---|---|---|---|---|
| t2.qwen.qwen-t2.kimi-kimi-k2.6.vision.caption | T2 | qwen/qwen-t2 | kimi/kimi-k2.6@qwen-t2 | vision.caption | passed |
| t2.qwen.qwen-t2.kimi-kimi-k2.6.vision.ocr | T2 | qwen/qwen-t2 | kimi/kimi-k2.6@qwen-t2 | vision.ocr | passed |
| t2.qwen.qwen-t2.kimi-kimi-k2.7-code-highspeed.vision.caption | T2 | qwen/qwen-t2 | kimi/kimi-k2.7-code-highspeed@qwen-t2 | vision.caption | passed |
| t2.qwen.qwen-t2.kimi-kimi-k2.7-code-highspeed.vision.ocr | T2 | qwen/qwen-t2 | kimi/kimi-k2.7-code-highspeed@qwen-t2 | vision.ocr | passed |
| t2.qwen.qwen-t2.kimi-kimi-k2.7-code.vision.caption | T2 | qwen/qwen-t2 | kimi/kimi-k2.7-code@qwen-t2 | vision.caption | passed |
| t2.qwen.qwen-t2.kimi-kimi-k2.7-code.vision.ocr | T2 | qwen/qwen-t2 | kimi/kimi-k2.7-code@qwen-t2 | vision.ocr | passed |
| t2.qwen.qwen-t2.kimi-kimi-k3.vision.caption | T2 | qwen/qwen-t2 | kimi/kimi-k3@qwen-t2 | vision.caption | passed |
| t2.qwen.qwen-t2.kimi-kimi-k3.vision.ocr | T2 | qwen/qwen-t2 | kimi/kimi-k3@qwen-t2 | vision.ocr | passed |
| t2.qwen.qwen-t2.minimax-minimax-m3.vision.caption | T2 | qwen/qwen-t2 | MiniMax/MiniMax-M3@qwen-t2 | vision.caption | passed |
| t2.qwen.qwen-t2.minimax-minimax-m3.vision.ocr | T2 | qwen/qwen-t2 | MiniMax/MiniMax-M3@qwen-t2 | vision.ocr | passed |
| t2.qwen.qwen-t2.vanchin-deepseek-v4.1-flash.vision.caption | T2 | qwen/qwen-t2 | vanchin/deepseek-v4.1-flash@qwen-t2 | vision.caption | passed |
| t2.qwen.qwen-t2.vanchin-deepseek-v4.1-flash.vision.ocr | T2 | qwen/qwen-t2 | vanchin/deepseek-v4.1-flash@qwen-t2 | vision.ocr | passed |
| t2.qwen.qwen-t2.zhipu-glm-5.3-flash.vision.caption | T2 | qwen/qwen-t2 | ZHIPU/GLM-5.3-Flash@qwen-t2 | vision.caption | passed |
| t2.qwen.qwen-t2.zhipu-glm-5.3-flash.vision.ocr | T2 | qwen/qwen-t2 | ZHIPU/GLM-5.3-Flash@qwen-t2 | vision.ocr | passed |

## Physical model coverage

| Provider | Inventory model | Physical model | Decision | Reason | Retained representative |
|---|---|---|---|---|---|
| qwen/qwen-t2 | kimi/kimi-k2.6 | kimi/kimi-k2.6 | included | physical model | - |
| qwen/qwen-t2 | kimi/kimi-k2.7-code-highspeed | kimi/kimi-k2.7-code-highspeed | included | physical model | - |
| qwen/qwen-t2 | kimi/kimi-k2.7-code | kimi/kimi-k2.7-code | included | physical model | - |
| qwen/qwen-t2 | kimi/kimi-k3 | kimi/kimi-k3 | included | physical model | - |
| qwen/qwen-t2 | MiniMax/MiniMax-M3 | MiniMax/MiniMax-M3 | included | physical model | - |
| qwen/qwen-t2 | vanchin/deepseek-v4.1-flash | vanchin/deepseek-v4.1-flash | included | physical model | - |
| qwen/qwen-t2 | ZHIPU/GLM-5.3-Flash | ZHIPU/GLM-5.3-Flash | included | physical model | - |
| qwen/qwen-t2 | ZHIPU/GLM-5 | ZHIPU/GLM-5 | included | physical model | - |
| qwen/qwen-t2 | kimi/kimi-k2.6 | kimi/kimi-k2.6 | included | physical model | - |
| qwen/qwen-t2 | kimi/kimi-k2.7-code-highspeed | kimi/kimi-k2.7-code-highspeed | included | physical model | - |
| qwen/qwen-t2 | kimi/kimi-k2.7-code | kimi/kimi-k2.7-code | included | physical model | - |
| qwen/qwen-t2 | kimi/kimi-k3 | kimi/kimi-k3 | included | physical model | - |
| qwen/qwen-t2 | MiniMax/MiniMax-M3 | MiniMax/MiniMax-M3 | included | physical model | - |
| qwen/qwen-t2 | vanchin/deepseek-v4.1-flash | vanchin/deepseek-v4.1-flash | included | physical model | - |
| qwen/qwen-t2 | ZHIPU/GLM-5.3-Flash | ZHIPU/GLM-5.3-Flash | included | physical model | - |
| qwen/qwen-t2 | ZHIPU/GLM-5 | ZHIPU/GLM-5 | included | physical model | - |

## Confirmed product defects

None.

## Cleanup

Status: passed

- removed 0 uploaded fixture object(s)
- removed 0 generated output object(s)
- temporary Provider credentials restored after testing (qwen)
