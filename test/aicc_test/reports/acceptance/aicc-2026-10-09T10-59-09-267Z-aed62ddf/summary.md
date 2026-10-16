# AICC E2E Acceptance Report

- Run: `aicc-2026-10-09T10-59-09-267Z-aed62ddf`
- Commit: `0c32470f0ebaa0f14a1a262bc2bcb0eea553d8fc`
- Capability baseline: `2026-10-09.1`
- Real model calls: 0/2
- Planned maximum cost: $0.020000
- Known actual / unknown estimated exposure: $0.000000 / $0.000000
- Total exposure / budget: $0.000000 / $0.020000
- Unknown-cost calls: 0; budget exceeded: false
- Results: passed=0, failed=59, provider_restricted=0, skipped=2, not_applicable=0, review=0

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
| t2.preflight.baseline_mismatch.4 | T2 | -/- | - | models.list | failed |
| t2.preflight.baseline_mismatch.5 | T2 | -/- | - | models.list | failed |
| t2.preflight.baseline_mismatch.6 | T2 | -/- | - | models.list | failed |
| t2.preflight.baseline_mismatch.7 | T2 | -/- | - | models.list | failed |
| t2.preflight.baseline_mismatch.8 | T2 | -/- | - | models.list | failed |
| t2.preflight.baseline_mismatch.9 | T2 | -/- | - | models.list | failed |
| t2.preflight.baseline_mismatch.10 | T2 | -/- | - | models.list | failed |
| t2.preflight.baseline_mismatch.11 | T2 | -/- | - | models.list | failed |
| t2.preflight.baseline_mismatch.12 | T2 | -/- | - | models.list | failed |
| t2.preflight.baseline_mismatch.13 | T2 | -/- | - | models.list | failed |
| t2.preflight.baseline_mismatch.14 | T2 | -/- | - | models.list | failed |
| t2.preflight.baseline_mismatch.15 | T2 | -/- | - | models.list | failed |
| t2.preflight.baseline_mismatch.16 | T2 | -/- | - | models.list | failed |
| t2.preflight.baseline_mismatch.17 | T2 | -/- | - | models.list | failed |
| t2.preflight.baseline_mismatch.18 | T2 | -/- | - | models.list | failed |
| t2.preflight.baseline_mismatch.19 | T2 | -/- | - | models.list | failed |
| t2.preflight.baseline_mismatch.20 | T2 | -/- | - | models.list | failed |
| t2.preflight.baseline_mismatch.21 | T2 | -/- | - | models.list | failed |
| t2.preflight.baseline_mismatch.22 | T2 | -/- | - | models.list | failed |
| t2.preflight.baseline_mismatch.23 | T2 | -/- | - | models.list | failed |
| t2.preflight.baseline_mismatch.24 | T2 | -/- | - | models.list | failed |
| t2.preflight.baseline_mismatch.25 | T2 | -/- | - | models.list | failed |
| t2.preflight.baseline_mismatch.26 | T2 | -/- | - | models.list | failed |
| t2.preflight.baseline_mismatch.27 | T2 | -/- | - | models.list | failed |
| t2.preflight.baseline_mismatch.28 | T2 | -/- | - | models.list | failed |
| t2.preflight.baseline_mismatch.29 | T2 | -/- | - | models.list | failed |
| t2.preflight.baseline_mismatch.30 | T2 | -/- | - | models.list | failed |
| t2.preflight.baseline_mismatch.31 | T2 | -/- | - | models.list | failed |
| t2.preflight.baseline_mismatch.32 | T2 | -/- | - | models.list | failed |
| t2.preflight.baseline_mismatch.33 | T2 | -/- | - | models.list | failed |
| t2.preflight.baseline_mismatch.34 | T2 | -/- | - | models.list | failed |
| t2.preflight.baseline_mismatch.35 | T2 | -/- | - | models.list | failed |
| t2.preflight.baseline_mismatch.36 | T2 | -/- | - | models.list | failed |
| t2.preflight.baseline_mismatch.37 | T2 | -/- | - | models.list | failed |
| t2.preflight.baseline_mismatch.38 | T2 | -/- | - | models.list | failed |
| t2.preflight.baseline_mismatch.39 | T2 | -/- | - | models.list | failed |
| t2.preflight.baseline_mismatch.40 | T2 | -/- | - | models.list | failed |
| t2.preflight.baseline_mismatch.41 | T2 | -/- | - | models.list | failed |
| t2.preflight.baseline_mismatch.42 | T2 | -/- | - | models.list | failed |
| t2.preflight.baseline_mismatch.43 | T2 | -/- | - | models.list | failed |
| t2.preflight.baseline_mismatch.44 | T2 | -/- | - | models.list | failed |
| t2.preflight.baseline_mismatch.45 | T2 | -/- | - | models.list | failed |
| t2.preflight.baseline_mismatch.46 | T2 | -/- | - | models.list | failed |
| t2.preflight.baseline_mismatch.47 | T2 | -/- | - | models.list | failed |
| t2.preflight.baseline_mismatch.48 | T2 | -/- | - | models.list | failed |
| t2.preflight.baseline_mismatch.49 | T2 | -/- | - | models.list | failed |
| t2.preflight.baseline_mismatch.50 | T2 | -/- | - | models.list | failed |
| t2.preflight.baseline_mismatch.51 | T2 | -/- | - | models.list | failed |
| t2.preflight.baseline_mismatch.52 | T2 | -/- | - | models.list | failed |
| t2.preflight.baseline_mismatch.53 | T2 | -/- | - | models.list | failed |
| t2.preflight.baseline_mismatch.54 | T2 | -/- | - | models.list | failed |
| t2.preflight.baseline_mismatch.55 | T2 | -/- | - | models.list | failed |
| t2.preflight.baseline_mismatch.56 | T2 | -/- | - | models.list | failed |
| t2.preflight.baseline_mismatch.57 | T2 | -/- | - | models.list | failed |
| t2.preflight.baseline_mismatch.58 | T2 | -/- | - | models.list | failed |
| t2.preflight.baseline_mismatch.59 | T2 | -/- | - | models.list | failed |
| t2.qwen.qwen-t2.qwen3.7-plus.llm | T2 | qwen/qwen-t2 | qwen3.7-plus@qwen-t2 | llm | skipped |
| t2.qwen.qwen-t2.qwen3.8-flash.llm | T2 | qwen/qwen-t2 | qwen3.8-flash@qwen-t2 | llm | skipped |

## Physical model coverage

| Provider | Inventory model | Physical model | Decision | Reason | Retained representative |
|---|---|---|---|---|---|
| qwen/qwen-t2 | qwen3.7-plus | qwen3.7-plus | included | physical model | - |
| qwen/qwen-t2 | qwen3.8-flash | qwen3.8-flash | included | physical model | - |
| qwen/qwen-t2 | deepseek-v4-flash-0731 | deepseek-v4-flash-0731 | included | physical model | - |
| qwen/qwen-t2 | deepseek-v4-flash | deepseek-v4-flash | included | physical model | - |
| qwen/qwen-t2 | deepseek-v4-pro-0813 | deepseek-v4-pro-0813 | included | physical model | - |
| qwen/qwen-t2 | deepseek-v4-pro | deepseek-v4-pro | included | physical model | - |
| qwen/qwen-t2 | deepseek-v4.1-flash | deepseek-v4.1-flash | included | physical model | - |
| qwen/qwen-t2 | glm-4.7 | glm-4.7 | included | physical model | - |
| qwen/qwen-t2 | glm-5.1 | glm-5.1 | included | physical model | - |
| qwen/qwen-t2 | glm-5.2 | glm-5.2 | included | physical model | - |
| qwen/qwen-t2 | glm-5.3 | glm-5.3 | included | physical model | - |
| qwen/qwen-t2 | glm-5 | glm-5 | included | physical model | - |
| qwen/qwen-t2 | kimi-k2.6 | kimi-k2.6 | included | physical model | - |
| qwen/qwen-t2 | kimi-k2.7-code | kimi-k2.7-code | included | physical model | - |
| qwen/qwen-t2 | kimi-k3 | kimi-k3 | included | physical model | - |
| qwen/qwen-t2 | kimi/kimi-k2.6 | kimi/kimi-k2.6 | included | physical model | - |
| qwen/qwen-t2 | kimi/kimi-k2.7-code-highspeed | kimi/kimi-k2.7-code-highspeed | included | physical model | - |
| qwen/qwen-t2 | kimi/kimi-k2.7-code | kimi/kimi-k2.7-code | included | physical model | - |
| qwen/qwen-t2 | kimi/kimi-k2.8-preview | kimi/kimi-k2.8-preview | included | physical model | - |
| qwen/qwen-t2 | kimi/kimi-k3 | kimi/kimi-k3 | included | physical model | - |
| qwen/qwen-t2 | MiniMax-M2.1 | MiniMax-M2.1 | included | physical model | - |
| qwen/qwen-t2 | MiniMax-M2.5 | MiniMax-M2.5 | included | physical model | - |
| qwen/qwen-t2 | MiniMax/MiniMax-M2.1 | MiniMax/MiniMax-M2.1 | included | physical model | - |
| qwen/qwen-t2 | MiniMax/MiniMax-M2.5 | MiniMax/MiniMax-M2.5 | included | physical model | - |
| qwen/qwen-t2 | MiniMax/MiniMax-M2.7 | MiniMax/MiniMax-M2.7 | included | physical model | - |
| qwen/qwen-t2 | MiniMax/MiniMax-M3 | MiniMax/MiniMax-M3 | included | physical model | - |
| qwen/qwen-t2 | qwen3-max-2025-09-23 | qwen3-max-2025-09-23 | included | physical model | - |
| qwen/qwen-t2 | qwen3-max-2026-01-23 | qwen3-max-2026-01-23 | included | physical model | - |
| qwen/qwen-t2 | qwen3-max | qwen3-max | included | physical model | - |
| qwen/qwen-t2 | qwen3.5-122b-a10b | qwen3.5-122b-a10b | included | physical model | - |
| qwen/qwen-t2 | qwen3.5-35b-a3b | qwen3.5-35b-a3b | included | physical model | - |
| qwen/qwen-t2 | qwen3.5-397b-a17b | qwen3.5-397b-a17b | included | physical model | - |
| qwen/qwen-t2 | qwen3.5-flash-2026-02-23 | qwen3.5-flash-2026-02-23 | included | physical model | - |
| qwen/qwen-t2 | qwen3.5-flash | qwen3.5-flash | included | physical model | - |
| qwen/qwen-t2 | qwen3.5-plus-2026-02-15 | qwen3.5-plus-2026-02-15 | included | physical model | - |
| qwen/qwen-t2 | qwen3.5-plus-2026-04-20 | qwen3.5-plus-2026-04-20 | included | physical model | - |
| qwen/qwen-t2 | qwen3.5-plus | qwen3.5-plus | included | physical model | - |
| qwen/qwen-t2 | qwen3.6-35b-a3b | qwen3.6-35b-a3b | included | physical model | - |
| qwen/qwen-t2 | qwen3.6-flash-2026-04-16 | qwen3.6-flash-2026-04-16 | included | physical model | - |
| qwen/qwen-t2 | qwen3.6-flash | qwen3.6-flash | included | physical model | - |
| qwen/qwen-t2 | qwen3.6-max-preview | qwen3.6-max-preview | included | physical model | - |
| qwen/qwen-t2 | qwen3.6-plus-2026-04-02 | qwen3.6-plus-2026-04-02 | included | physical model | - |
| qwen/qwen-t2 | qwen3.6-plus | qwen3.6-plus | included | physical model | - |
| qwen/qwen-t2 | qwen3.7-flash-2026-07-15 | qwen3.7-flash-2026-07-15 | included | physical model | - |
| qwen/qwen-t2 | qwen3.7-flash | qwen3.7-flash | included | physical model | - |
| qwen/qwen-t2 | qwen3.7-max-2026-05-17 | qwen3.7-max-2026-05-17 | included | physical model | - |
| qwen/qwen-t2 | qwen3.7-max-2026-05-20 | qwen3.7-max-2026-05-20 | included | physical model | - |
| qwen/qwen-t2 | qwen3.7-max-2026-06-08 | qwen3.7-max-2026-06-08 | included | physical model | - |
| qwen/qwen-t2 | qwen3.7-max | qwen3.7-max | included | physical model | - |
| qwen/qwen-t2 | qwen3.7-plus-2026-05-26 | qwen3.7-plus-2026-05-26 | included | physical model | - |
| qwen/qwen-t2 | qwen3.7-plus | qwen3.7-plus | included | physical model | - |
| qwen/qwen-t2 | qwen3.8-27b | qwen3.8-27b | included | physical model | - |
| qwen/qwen-t2 | qwen3.8-flash | qwen3.8-flash | included | physical model | - |
| qwen/qwen-t2 | qwen3.8-max-0902 | qwen3.8-max-0902 | included | physical model | - |
| qwen/qwen-t2 | qwen3.8-omni-flash | qwen3.8-omni-flash | included | physical model | - |
| qwen/qwen-t2 | vanchin/deepseek-v4-pro-0813 | vanchin/deepseek-v4-pro-0813 | included | physical model | - |
| qwen/qwen-t2 | vanchin/deepseek-v4-pro | vanchin/deepseek-v4-pro | included | physical model | - |
| qwen/qwen-t2 | vanchin/deepseek-v4.1-flash | vanchin/deepseek-v4.1-flash | included | physical model | - |
| qwen/qwen-t2 | ZHIPU/GLM-5.1 | ZHIPU/GLM-5.1 | included | physical model | - |
| qwen/qwen-t2 | ZHIPU/GLM-5.2 | ZHIPU/GLM-5.2 | included | physical model | - |
| qwen/qwen-t2 | ZHIPU/GLM-5.3-Flash | ZHIPU/GLM-5.3-Flash | included | physical model | - |
| qwen/qwen-t2 | ZHIPU/GLM-5.3 | ZHIPU/GLM-5.3 | included | physical model | - |
| qwen/qwen-t2 | ZHIPU/GLM-5 | ZHIPU/GLM-5 | included | physical model | - |

## Confirmed product defects

- **defect.aicc.t2.preflight.baseline_mismatch.1** (AICC, baseline_mismatch): expected official Provider capability, AICC inventory declaration, and adapter support are bidirectionally consistent; observed aicc_advertised_but_official_missing qwen/deepseek-v4-flash-0731. Evidence: cases/t2.preflight.baseline_mismatch.1.json, effective_baseline.json
- **defect.aicc.t2.preflight.baseline_mismatch.2** (AICC, baseline_mismatch): expected official Provider capability, AICC inventory declaration, and adapter support are bidirectionally consistent; observed aicc_advertised_but_official_missing qwen/deepseek-v4-flash. Evidence: cases/t2.preflight.baseline_mismatch.2.json, effective_baseline.json
- **defect.aicc.t2.preflight.baseline_mismatch.3** (AICC, baseline_mismatch): expected official Provider capability, AICC inventory declaration, and adapter support are bidirectionally consistent; observed aicc_advertised_but_official_missing qwen/deepseek-v4-pro-0813. Evidence: cases/t2.preflight.baseline_mismatch.3.json, effective_baseline.json
- **defect.aicc.t2.preflight.baseline_mismatch.4** (AICC, baseline_mismatch): expected official Provider capability, AICC inventory declaration, and adapter support are bidirectionally consistent; observed aicc_advertised_but_official_missing qwen/deepseek-v4-pro. Evidence: cases/t2.preflight.baseline_mismatch.4.json, effective_baseline.json
- **defect.aicc.t2.preflight.baseline_mismatch.5** (AICC, baseline_mismatch): expected official Provider capability, AICC inventory declaration, and adapter support are bidirectionally consistent; observed aicc_advertised_but_official_missing qwen/deepseek-v4.1-flash. Evidence: cases/t2.preflight.baseline_mismatch.5.json, effective_baseline.json
- **defect.aicc.t2.preflight.baseline_mismatch.6** (AICC, baseline_mismatch): expected official Provider capability, AICC inventory declaration, and adapter support are bidirectionally consistent; observed aicc_advertised_but_official_missing qwen/glm-4.7. Evidence: cases/t2.preflight.baseline_mismatch.6.json, effective_baseline.json
- **defect.aicc.t2.preflight.baseline_mismatch.7** (AICC, baseline_mismatch): expected official Provider capability, AICC inventory declaration, and adapter support are bidirectionally consistent; observed aicc_advertised_but_official_missing qwen/glm-5.1. Evidence: cases/t2.preflight.baseline_mismatch.7.json, effective_baseline.json
- **defect.aicc.t2.preflight.baseline_mismatch.8** (AICC, baseline_mismatch): expected official Provider capability, AICC inventory declaration, and adapter support are bidirectionally consistent; observed aicc_advertised_but_official_missing qwen/glm-5.2. Evidence: cases/t2.preflight.baseline_mismatch.8.json, effective_baseline.json
- **defect.aicc.t2.preflight.baseline_mismatch.9** (AICC, baseline_mismatch): expected official Provider capability, AICC inventory declaration, and adapter support are bidirectionally consistent; observed aicc_advertised_but_official_missing qwen/glm-5.3. Evidence: cases/t2.preflight.baseline_mismatch.9.json, effective_baseline.json
- **defect.aicc.t2.preflight.baseline_mismatch.10** (AICC, baseline_mismatch): expected official Provider capability, AICC inventory declaration, and adapter support are bidirectionally consistent; observed aicc_advertised_but_official_missing qwen/glm-5. Evidence: cases/t2.preflight.baseline_mismatch.10.json, effective_baseline.json
- **defect.aicc.t2.preflight.baseline_mismatch.11** (AICC, baseline_mismatch): expected official Provider capability, AICC inventory declaration, and adapter support are bidirectionally consistent; observed aicc_advertised_but_official_missing qwen/kimi-k2.6. Evidence: cases/t2.preflight.baseline_mismatch.11.json, effective_baseline.json
- **defect.aicc.t2.preflight.baseline_mismatch.12** (AICC, baseline_mismatch): expected official Provider capability, AICC inventory declaration, and adapter support are bidirectionally consistent; observed aicc_advertised_but_official_missing qwen/kimi-k2.7-code. Evidence: cases/t2.preflight.baseline_mismatch.12.json, effective_baseline.json
- **defect.aicc.t2.preflight.baseline_mismatch.13** (AICC, baseline_mismatch): expected official Provider capability, AICC inventory declaration, and adapter support are bidirectionally consistent; observed aicc_advertised_but_official_missing qwen/kimi-k3. Evidence: cases/t2.preflight.baseline_mismatch.13.json, effective_baseline.json
- **defect.aicc.t2.preflight.baseline_mismatch.14** (AICC, baseline_mismatch): expected official Provider capability, AICC inventory declaration, and adapter support are bidirectionally consistent; observed aicc_advertised_but_official_missing qwen/kimi/kimi-k2.6. Evidence: cases/t2.preflight.baseline_mismatch.14.json, effective_baseline.json
- **defect.aicc.t2.preflight.baseline_mismatch.15** (AICC, baseline_mismatch): expected official Provider capability, AICC inventory declaration, and adapter support are bidirectionally consistent; observed aicc_advertised_but_official_missing qwen/kimi/kimi-k2.7-code-highspeed. Evidence: cases/t2.preflight.baseline_mismatch.15.json, effective_baseline.json
- **defect.aicc.t2.preflight.baseline_mismatch.16** (AICC, baseline_mismatch): expected official Provider capability, AICC inventory declaration, and adapter support are bidirectionally consistent; observed aicc_advertised_but_official_missing qwen/kimi/kimi-k2.7-code. Evidence: cases/t2.preflight.baseline_mismatch.16.json, effective_baseline.json
- **defect.aicc.t2.preflight.baseline_mismatch.17** (AICC, baseline_mismatch): expected official Provider capability, AICC inventory declaration, and adapter support are bidirectionally consistent; observed aicc_advertised_but_official_missing qwen/kimi/kimi-k2.8-preview. Evidence: cases/t2.preflight.baseline_mismatch.17.json, effective_baseline.json
- **defect.aicc.t2.preflight.baseline_mismatch.18** (AICC, baseline_mismatch): expected official Provider capability, AICC inventory declaration, and adapter support are bidirectionally consistent; observed aicc_advertised_but_official_missing qwen/kimi/kimi-k3. Evidence: cases/t2.preflight.baseline_mismatch.18.json, effective_baseline.json
- **defect.aicc.t2.preflight.baseline_mismatch.19** (AICC, baseline_mismatch): expected official Provider capability, AICC inventory declaration, and adapter support are bidirectionally consistent; observed aicc_advertised_but_official_missing qwen/MiniMax-M2.1. Evidence: cases/t2.preflight.baseline_mismatch.19.json, effective_baseline.json
- **defect.aicc.t2.preflight.baseline_mismatch.20** (AICC, baseline_mismatch): expected official Provider capability, AICC inventory declaration, and adapter support are bidirectionally consistent; observed aicc_advertised_but_official_missing qwen/MiniMax-M2.5. Evidence: cases/t2.preflight.baseline_mismatch.20.json, effective_baseline.json
- **defect.aicc.t2.preflight.baseline_mismatch.21** (AICC, baseline_mismatch): expected official Provider capability, AICC inventory declaration, and adapter support are bidirectionally consistent; observed aicc_advertised_but_official_missing qwen/MiniMax/MiniMax-M2.1. Evidence: cases/t2.preflight.baseline_mismatch.21.json, effective_baseline.json
- **defect.aicc.t2.preflight.baseline_mismatch.22** (AICC, baseline_mismatch): expected official Provider capability, AICC inventory declaration, and adapter support are bidirectionally consistent; observed aicc_advertised_but_official_missing qwen/MiniMax/MiniMax-M2.5. Evidence: cases/t2.preflight.baseline_mismatch.22.json, effective_baseline.json
- **defect.aicc.t2.preflight.baseline_mismatch.23** (AICC, baseline_mismatch): expected official Provider capability, AICC inventory declaration, and adapter support are bidirectionally consistent; observed aicc_advertised_but_official_missing qwen/MiniMax/MiniMax-M2.7. Evidence: cases/t2.preflight.baseline_mismatch.23.json, effective_baseline.json
- **defect.aicc.t2.preflight.baseline_mismatch.24** (AICC, baseline_mismatch): expected official Provider capability, AICC inventory declaration, and adapter support are bidirectionally consistent; observed aicc_advertised_but_official_missing qwen/MiniMax/MiniMax-M3. Evidence: cases/t2.preflight.baseline_mismatch.24.json, effective_baseline.json
- **defect.aicc.t2.preflight.baseline_mismatch.25** (AICC, baseline_mismatch): expected official Provider capability, AICC inventory declaration, and adapter support are bidirectionally consistent; observed aicc_advertised_but_official_missing qwen/qwen3-max-2025-09-23. Evidence: cases/t2.preflight.baseline_mismatch.25.json, effective_baseline.json
- **defect.aicc.t2.preflight.baseline_mismatch.26** (AICC, baseline_mismatch): expected official Provider capability, AICC inventory declaration, and adapter support are bidirectionally consistent; observed aicc_advertised_but_official_missing qwen/qwen3-max-2026-01-23. Evidence: cases/t2.preflight.baseline_mismatch.26.json, effective_baseline.json
- **defect.aicc.t2.preflight.baseline_mismatch.27** (AICC, baseline_mismatch): expected official Provider capability, AICC inventory declaration, and adapter support are bidirectionally consistent; observed aicc_advertised_but_official_missing qwen/qwen3-max. Evidence: cases/t2.preflight.baseline_mismatch.27.json, effective_baseline.json
- **defect.aicc.t2.preflight.baseline_mismatch.28** (AICC, baseline_mismatch): expected official Provider capability, AICC inventory declaration, and adapter support are bidirectionally consistent; observed aicc_advertised_but_official_missing qwen/qwen3.5-122b-a10b. Evidence: cases/t2.preflight.baseline_mismatch.28.json, effective_baseline.json
- **defect.aicc.t2.preflight.baseline_mismatch.29** (AICC, baseline_mismatch): expected official Provider capability, AICC inventory declaration, and adapter support are bidirectionally consistent; observed aicc_advertised_but_official_missing qwen/qwen3.5-35b-a3b. Evidence: cases/t2.preflight.baseline_mismatch.29.json, effective_baseline.json
- **defect.aicc.t2.preflight.baseline_mismatch.30** (AICC, baseline_mismatch): expected official Provider capability, AICC inventory declaration, and adapter support are bidirectionally consistent; observed aicc_advertised_but_official_missing qwen/qwen3.5-397b-a17b. Evidence: cases/t2.preflight.baseline_mismatch.30.json, effective_baseline.json
- **defect.aicc.t2.preflight.baseline_mismatch.31** (AICC, baseline_mismatch): expected official Provider capability, AICC inventory declaration, and adapter support are bidirectionally consistent; observed aicc_advertised_but_official_missing qwen/qwen3.5-flash-2026-02-23. Evidence: cases/t2.preflight.baseline_mismatch.31.json, effective_baseline.json
- **defect.aicc.t2.preflight.baseline_mismatch.32** (AICC, baseline_mismatch): expected official Provider capability, AICC inventory declaration, and adapter support are bidirectionally consistent; observed aicc_advertised_but_official_missing qwen/qwen3.5-flash. Evidence: cases/t2.preflight.baseline_mismatch.32.json, effective_baseline.json
- **defect.aicc.t2.preflight.baseline_mismatch.33** (AICC, baseline_mismatch): expected official Provider capability, AICC inventory declaration, and adapter support are bidirectionally consistent; observed aicc_advertised_but_official_missing qwen/qwen3.5-plus-2026-02-15. Evidence: cases/t2.preflight.baseline_mismatch.33.json, effective_baseline.json
- **defect.aicc.t2.preflight.baseline_mismatch.34** (AICC, baseline_mismatch): expected official Provider capability, AICC inventory declaration, and adapter support are bidirectionally consistent; observed aicc_advertised_but_official_missing qwen/qwen3.5-plus-2026-04-20. Evidence: cases/t2.preflight.baseline_mismatch.34.json, effective_baseline.json
- **defect.aicc.t2.preflight.baseline_mismatch.35** (AICC, baseline_mismatch): expected official Provider capability, AICC inventory declaration, and adapter support are bidirectionally consistent; observed aicc_advertised_but_official_missing qwen/qwen3.5-plus. Evidence: cases/t2.preflight.baseline_mismatch.35.json, effective_baseline.json
- **defect.aicc.t2.preflight.baseline_mismatch.36** (AICC, baseline_mismatch): expected official Provider capability, AICC inventory declaration, and adapter support are bidirectionally consistent; observed aicc_advertised_but_official_missing qwen/qwen3.6-35b-a3b. Evidence: cases/t2.preflight.baseline_mismatch.36.json, effective_baseline.json
- **defect.aicc.t2.preflight.baseline_mismatch.37** (AICC, baseline_mismatch): expected official Provider capability, AICC inventory declaration, and adapter support are bidirectionally consistent; observed aicc_advertised_but_official_missing qwen/qwen3.6-flash-2026-04-16. Evidence: cases/t2.preflight.baseline_mismatch.37.json, effective_baseline.json
- **defect.aicc.t2.preflight.baseline_mismatch.38** (AICC, baseline_mismatch): expected official Provider capability, AICC inventory declaration, and adapter support are bidirectionally consistent; observed aicc_advertised_but_official_missing qwen/qwen3.6-flash. Evidence: cases/t2.preflight.baseline_mismatch.38.json, effective_baseline.json
- **defect.aicc.t2.preflight.baseline_mismatch.39** (AICC, baseline_mismatch): expected official Provider capability, AICC inventory declaration, and adapter support are bidirectionally consistent; observed aicc_advertised_but_official_missing qwen/qwen3.6-max-preview. Evidence: cases/t2.preflight.baseline_mismatch.39.json, effective_baseline.json
- **defect.aicc.t2.preflight.baseline_mismatch.40** (AICC, baseline_mismatch): expected official Provider capability, AICC inventory declaration, and adapter support are bidirectionally consistent; observed aicc_advertised_but_official_missing qwen/qwen3.6-plus-2026-04-02. Evidence: cases/t2.preflight.baseline_mismatch.40.json, effective_baseline.json
- **defect.aicc.t2.preflight.baseline_mismatch.41** (AICC, baseline_mismatch): expected official Provider capability, AICC inventory declaration, and adapter support are bidirectionally consistent; observed aicc_advertised_but_official_missing qwen/qwen3.6-plus. Evidence: cases/t2.preflight.baseline_mismatch.41.json, effective_baseline.json
- **defect.aicc.t2.preflight.baseline_mismatch.42** (AICC, baseline_mismatch): expected official Provider capability, AICC inventory declaration, and adapter support are bidirectionally consistent; observed aicc_advertised_but_official_missing qwen/qwen3.7-flash-2026-07-15. Evidence: cases/t2.preflight.baseline_mismatch.42.json, effective_baseline.json
- **defect.aicc.t2.preflight.baseline_mismatch.43** (AICC, baseline_mismatch): expected official Provider capability, AICC inventory declaration, and adapter support are bidirectionally consistent; observed aicc_advertised_but_official_missing qwen/qwen3.7-flash. Evidence: cases/t2.preflight.baseline_mismatch.43.json, effective_baseline.json
- **defect.aicc.t2.preflight.baseline_mismatch.44** (AICC, baseline_mismatch): expected official Provider capability, AICC inventory declaration, and adapter support are bidirectionally consistent; observed aicc_advertised_but_official_missing qwen/qwen3.7-max-2026-05-17. Evidence: cases/t2.preflight.baseline_mismatch.44.json, effective_baseline.json
- **defect.aicc.t2.preflight.baseline_mismatch.45** (AICC, baseline_mismatch): expected official Provider capability, AICC inventory declaration, and adapter support are bidirectionally consistent; observed aicc_advertised_but_official_missing qwen/qwen3.7-max-2026-05-20. Evidence: cases/t2.preflight.baseline_mismatch.45.json, effective_baseline.json
- **defect.aicc.t2.preflight.baseline_mismatch.46** (AICC, baseline_mismatch): expected official Provider capability, AICC inventory declaration, and adapter support are bidirectionally consistent; observed aicc_advertised_but_official_missing qwen/qwen3.7-max-2026-06-08. Evidence: cases/t2.preflight.baseline_mismatch.46.json, effective_baseline.json
- **defect.aicc.t2.preflight.baseline_mismatch.47** (AICC, baseline_mismatch): expected official Provider capability, AICC inventory declaration, and adapter support are bidirectionally consistent; observed aicc_advertised_but_official_missing qwen/qwen3.7-max. Evidence: cases/t2.preflight.baseline_mismatch.47.json, effective_baseline.json
- **defect.aicc.t2.preflight.baseline_mismatch.48** (AICC, baseline_mismatch): expected official Provider capability, AICC inventory declaration, and adapter support are bidirectionally consistent; observed aicc_advertised_but_official_missing qwen/qwen3.7-plus-2026-05-26. Evidence: cases/t2.preflight.baseline_mismatch.48.json, effective_baseline.json
- **defect.aicc.t2.preflight.baseline_mismatch.49** (AICC, baseline_mismatch): expected official Provider capability, AICC inventory declaration, and adapter support are bidirectionally consistent; observed aicc_advertised_but_official_missing qwen/qwen3.8-27b. Evidence: cases/t2.preflight.baseline_mismatch.49.json, effective_baseline.json
- **defect.aicc.t2.preflight.baseline_mismatch.50** (AICC, baseline_mismatch): expected official Provider capability, AICC inventory declaration, and adapter support are bidirectionally consistent; observed aicc_advertised_but_official_missing qwen/qwen3.8-max-0902. Evidence: cases/t2.preflight.baseline_mismatch.50.json, effective_baseline.json
- **defect.aicc.t2.preflight.baseline_mismatch.51** (AICC, baseline_mismatch): expected official Provider capability, AICC inventory declaration, and adapter support are bidirectionally consistent; observed aicc_advertised_but_official_missing qwen/qwen3.8-omni-flash. Evidence: cases/t2.preflight.baseline_mismatch.51.json, effective_baseline.json
- **defect.aicc.t2.preflight.baseline_mismatch.52** (AICC, baseline_mismatch): expected official Provider capability, AICC inventory declaration, and adapter support are bidirectionally consistent; observed aicc_advertised_but_official_missing qwen/vanchin/deepseek-v4-pro-0813. Evidence: cases/t2.preflight.baseline_mismatch.52.json, effective_baseline.json
- **defect.aicc.t2.preflight.baseline_mismatch.53** (AICC, baseline_mismatch): expected official Provider capability, AICC inventory declaration, and adapter support are bidirectionally consistent; observed aicc_advertised_but_official_missing qwen/vanchin/deepseek-v4-pro. Evidence: cases/t2.preflight.baseline_mismatch.53.json, effective_baseline.json
- **defect.aicc.t2.preflight.baseline_mismatch.54** (AICC, baseline_mismatch): expected official Provider capability, AICC inventory declaration, and adapter support are bidirectionally consistent; observed aicc_advertised_but_official_missing qwen/vanchin/deepseek-v4.1-flash. Evidence: cases/t2.preflight.baseline_mismatch.54.json, effective_baseline.json
- **defect.aicc.t2.preflight.baseline_mismatch.55** (AICC, baseline_mismatch): expected official Provider capability, AICC inventory declaration, and adapter support are bidirectionally consistent; observed aicc_advertised_but_official_missing qwen/ZHIPU/GLM-5.1. Evidence: cases/t2.preflight.baseline_mismatch.55.json, effective_baseline.json
- **defect.aicc.t2.preflight.baseline_mismatch.56** (AICC, baseline_mismatch): expected official Provider capability, AICC inventory declaration, and adapter support are bidirectionally consistent; observed aicc_advertised_but_official_missing qwen/ZHIPU/GLM-5.2. Evidence: cases/t2.preflight.baseline_mismatch.56.json, effective_baseline.json
- **defect.aicc.t2.preflight.baseline_mismatch.57** (AICC, baseline_mismatch): expected official Provider capability, AICC inventory declaration, and adapter support are bidirectionally consistent; observed aicc_advertised_but_official_missing qwen/ZHIPU/GLM-5.3-Flash. Evidence: cases/t2.preflight.baseline_mismatch.57.json, effective_baseline.json
- **defect.aicc.t2.preflight.baseline_mismatch.58** (AICC, baseline_mismatch): expected official Provider capability, AICC inventory declaration, and adapter support are bidirectionally consistent; observed aicc_advertised_but_official_missing qwen/ZHIPU/GLM-5.3. Evidence: cases/t2.preflight.baseline_mismatch.58.json, effective_baseline.json
- **defect.aicc.t2.preflight.baseline_mismatch.59** (AICC, baseline_mismatch): expected official Provider capability, AICC inventory declaration, and adapter support are bidirectionally consistent; observed aicc_advertised_but_official_missing qwen/ZHIPU/GLM-5. Evidence: cases/t2.preflight.baseline_mismatch.59.json, effective_baseline.json

## Cleanup

Status: passed

- removed 0 uploaded fixture object(s)
- removed 0 generated output object(s)
- temporary Provider credentials restored after testing (qwen)
