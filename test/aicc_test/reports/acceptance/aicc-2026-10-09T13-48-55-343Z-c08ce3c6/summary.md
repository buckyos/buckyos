# AICC E2E Acceptance Report

- Run: `aicc-2026-10-09T13-48-55-343Z-c08ce3c6`
- Commit: `0c32470f0ebaa0f14a1a262bc2bcb0eea553d8fc`
- Capability baseline: `2026-10-09.6`
- Real model calls: 17/17
- Planned maximum cost: $0.170000
- Known actual / unknown estimated exposure: $0.000000 / $0.170000
- Total exposure / budget: $0.170000 / $1.000000
- Unknown-cost calls: 17; budget exceeded: false
- Results: passed=0, failed=17, provider_restricted=0, skipped=0, not_applicable=0, review=0

## Targeted retest

Run after fixing the reported defect; repeat `--case` to select additional cases:

```bash
pnpm run acceptance:gateway -- --config aicc_acceptance.local.toml --allow-credential-mutation --timeout-ms 900000 --provider qwen --case t2.qwen.qwen-t2.kimi-kimi-k2.6.llm --case t2.qwen.qwen-t2.kimi-kimi-k2.7-code-highspeed.llm --case t2.qwen.qwen-t2.kimi-kimi-k2.7-code.llm --case t2.qwen.qwen-t2.kimi-kimi-k2.8-preview.llm --case t2.qwen.qwen-t2.kimi-kimi-k3.llm --case t2.qwen.qwen-t2.minimax-minimax-m2.1.llm --case t2.qwen.qwen-t2.minimax-minimax-m2.5.llm --case t2.qwen.qwen-t2.minimax-minimax-m2.7.llm --case t2.qwen.qwen-t2.minimax-minimax-m3.llm --case t2.qwen.qwen-t2.vanchin-deepseek-v4-pro-0813.llm --case t2.qwen.qwen-t2.vanchin-deepseek-v4-pro.llm --case t2.qwen.qwen-t2.vanchin-deepseek-v4.1-flash.llm --case t2.qwen.qwen-t2.zhipu-glm-5.1.llm --case t2.qwen.qwen-t2.zhipu-glm-5.2.llm --case t2.qwen.qwen-t2.zhipu-glm-5.3-flash.llm --case t2.qwen.qwen-t2.zhipu-glm-5.3.llm --case t2.qwen.qwen-t2.zhipu-glm-5.llm
```

## Cases

| Case | Layer | Provider | Model | API | Status |
|---|---|---|---|---|---|
| t2.qwen.qwen-t2.kimi-kimi-k2.6.llm | T2 | qwen/qwen-t2 | kimi/kimi-k2.6@qwen-t2 | llm | failed |
| t2.qwen.qwen-t2.kimi-kimi-k2.7-code-highspeed.llm | T2 | qwen/qwen-t2 | kimi/kimi-k2.7-code-highspeed@qwen-t2 | llm | failed |
| t2.qwen.qwen-t2.kimi-kimi-k2.7-code.llm | T2 | qwen/qwen-t2 | kimi/kimi-k2.7-code@qwen-t2 | llm | failed |
| t2.qwen.qwen-t2.kimi-kimi-k2.8-preview.llm | T2 | qwen/qwen-t2 | kimi/kimi-k2.8-preview@qwen-t2 | llm | failed |
| t2.qwen.qwen-t2.kimi-kimi-k3.llm | T2 | qwen/qwen-t2 | kimi/kimi-k3@qwen-t2 | llm | failed |
| t2.qwen.qwen-t2.minimax-minimax-m2.1.llm | T2 | qwen/qwen-t2 | MiniMax/MiniMax-M2.1@qwen-t2 | llm | failed |
| t2.qwen.qwen-t2.minimax-minimax-m2.5.llm | T2 | qwen/qwen-t2 | MiniMax/MiniMax-M2.5@qwen-t2 | llm | failed |
| t2.qwen.qwen-t2.minimax-minimax-m2.7.llm | T2 | qwen/qwen-t2 | MiniMax/MiniMax-M2.7@qwen-t2 | llm | failed |
| t2.qwen.qwen-t2.minimax-minimax-m3.llm | T2 | qwen/qwen-t2 | MiniMax/MiniMax-M3@qwen-t2 | llm | failed |
| t2.qwen.qwen-t2.vanchin-deepseek-v4-pro-0813.llm | T2 | qwen/qwen-t2 | vanchin/deepseek-v4-pro-0813@qwen-t2 | llm | failed |
| t2.qwen.qwen-t2.vanchin-deepseek-v4-pro.llm | T2 | qwen/qwen-t2 | vanchin/deepseek-v4-pro@qwen-t2 | llm | failed |
| t2.qwen.qwen-t2.vanchin-deepseek-v4.1-flash.llm | T2 | qwen/qwen-t2 | vanchin/deepseek-v4.1-flash@qwen-t2 | llm | failed |
| t2.qwen.qwen-t2.zhipu-glm-5.1.llm | T2 | qwen/qwen-t2 | ZHIPU/GLM-5.1@qwen-t2 | llm | failed |
| t2.qwen.qwen-t2.zhipu-glm-5.2.llm | T2 | qwen/qwen-t2 | ZHIPU/GLM-5.2@qwen-t2 | llm | failed |
| t2.qwen.qwen-t2.zhipu-glm-5.3-flash.llm | T2 | qwen/qwen-t2 | ZHIPU/GLM-5.3-Flash@qwen-t2 | llm | failed |
| t2.qwen.qwen-t2.zhipu-glm-5.3.llm | T2 | qwen/qwen-t2 | ZHIPU/GLM-5.3@qwen-t2 | llm | failed |
| t2.qwen.qwen-t2.zhipu-glm-5.llm | T2 | qwen/qwen-t2 | ZHIPU/GLM-5@qwen-t2 | llm | failed |

## Physical model coverage

| Provider | Inventory model | Physical model | Decision | Reason | Retained representative |
|---|---|---|---|---|---|
| qwen/qwen-t2 | kimi/kimi-k2.6 | kimi/kimi-k2.6 | included | physical model | - |
| qwen/qwen-t2 | kimi/kimi-k2.7-code-highspeed | kimi/kimi-k2.7-code-highspeed | included | physical model | - |
| qwen/qwen-t2 | kimi/kimi-k2.7-code | kimi/kimi-k2.7-code | included | physical model | - |
| qwen/qwen-t2 | kimi/kimi-k2.8-preview | kimi/kimi-k2.8-preview | included | physical model | - |
| qwen/qwen-t2 | kimi/kimi-k3 | kimi/kimi-k3 | included | physical model | - |
| qwen/qwen-t2 | MiniMax/MiniMax-M2.1 | MiniMax/MiniMax-M2.1 | included | physical model | - |
| qwen/qwen-t2 | MiniMax/MiniMax-M2.5 | MiniMax/MiniMax-M2.5 | included | physical model | - |
| qwen/qwen-t2 | MiniMax/MiniMax-M2.7 | MiniMax/MiniMax-M2.7 | included | physical model | - |
| qwen/qwen-t2 | MiniMax/MiniMax-M3 | MiniMax/MiniMax-M3 | included | physical model | - |
| qwen/qwen-t2 | vanchin/deepseek-v4-pro-0813 | vanchin/deepseek-v4-pro-0813 | included | physical model | - |
| qwen/qwen-t2 | vanchin/deepseek-v4-pro | vanchin/deepseek-v4-pro | included | physical model | - |
| qwen/qwen-t2 | vanchin/deepseek-v4.1-flash | vanchin/deepseek-v4.1-flash | included | physical model | - |
| qwen/qwen-t2 | ZHIPU/GLM-5.1 | ZHIPU/GLM-5.1 | included | physical model | - |
| qwen/qwen-t2 | ZHIPU/GLM-5.2 | ZHIPU/GLM-5.2 | included | physical model | - |
| qwen/qwen-t2 | ZHIPU/GLM-5.3-Flash | ZHIPU/GLM-5.3-Flash | included | physical model | - |
| qwen/qwen-t2 | ZHIPU/GLM-5.3 | ZHIPU/GLM-5.3 | included | physical model | - |
| qwen/qwen-t2 | ZHIPU/GLM-5 | ZHIPU/GLM-5 | included | physical model | - |
| qwen/qwen-t2 | kimi/kimi-k2.6 | kimi/kimi-k2.6 | included | physical model | - |
| qwen/qwen-t2 | kimi/kimi-k2.7-code-highspeed | kimi/kimi-k2.7-code-highspeed | included | physical model | - |
| qwen/qwen-t2 | kimi/kimi-k2.7-code | kimi/kimi-k2.7-code | included | physical model | - |
| qwen/qwen-t2 | kimi/kimi-k2.8-preview | kimi/kimi-k2.8-preview | included | physical model | - |
| qwen/qwen-t2 | kimi/kimi-k3 | kimi/kimi-k3 | included | physical model | - |
| qwen/qwen-t2 | MiniMax/MiniMax-M2.1 | MiniMax/MiniMax-M2.1 | included | physical model | - |
| qwen/qwen-t2 | MiniMax/MiniMax-M2.5 | MiniMax/MiniMax-M2.5 | included | physical model | - |
| qwen/qwen-t2 | MiniMax/MiniMax-M2.7 | MiniMax/MiniMax-M2.7 | included | physical model | - |
| qwen/qwen-t2 | MiniMax/MiniMax-M3 | MiniMax/MiniMax-M3 | included | physical model | - |
| qwen/qwen-t2 | vanchin/deepseek-v4-pro-0813 | vanchin/deepseek-v4-pro-0813 | included | physical model | - |
| qwen/qwen-t2 | vanchin/deepseek-v4-pro | vanchin/deepseek-v4-pro | included | physical model | - |
| qwen/qwen-t2 | vanchin/deepseek-v4.1-flash | vanchin/deepseek-v4.1-flash | included | physical model | - |
| qwen/qwen-t2 | ZHIPU/GLM-5.1 | ZHIPU/GLM-5.1 | included | physical model | - |
| qwen/qwen-t2 | ZHIPU/GLM-5.2 | ZHIPU/GLM-5.2 | included | physical model | - |
| qwen/qwen-t2 | ZHIPU/GLM-5.3-Flash | ZHIPU/GLM-5.3-Flash | included | physical model | - |
| qwen/qwen-t2 | ZHIPU/GLM-5.3 | ZHIPU/GLM-5.3 | included | physical model | - |
| qwen/qwen-t2 | ZHIPU/GLM-5 | ZHIPU/GLM-5 | included | physical model | - |

## Confirmed product defects

None.

## Cleanup

Status: passed

- removed 0 uploaded fixture object(s)
- removed 0 generated output object(s)
- temporary Provider credentials restored after testing (qwen)
