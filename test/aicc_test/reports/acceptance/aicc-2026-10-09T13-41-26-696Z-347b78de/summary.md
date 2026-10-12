# AICC E2E Acceptance Report

- Run: `aicc-2026-10-09T13-41-26-696Z-347b78de`
- Commit: `0c32470f0ebaa0f14a1a262bc2bcb0eea553d8fc`
- Capability baseline: `2026-10-09.6`
- Real model calls: 15/15
- Planned maximum cost: $0.150000
- Known actual / unknown estimated exposure: $0.000000 / $0.150000
- Total exposure / budget: $0.150000 / $1.000000
- Unknown-cost calls: 15; budget exceeded: false
- Results: passed=12, failed=11, provider_restricted=0, skipped=0, not_applicable=0, review=0

## Targeted retest

Run after fixing the reported defect; repeat `--case` to select additional cases:

```bash
pnpm run acceptance:gateway -- --config aicc_acceptance.local.toml --allow-credential-mutation --timeout-ms 900000 --provider qwen --case t2.qwen.qwen-t2.glm-5.1.llm --case t2.qwen.qwen-t2.glm-5.3.llm --case t2.qwen.qwen-t2.minimax-m2.5.llm
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
| t2.qwen.qwen-t2.deepseek-v4-flash-0731.llm | T2 | qwen/qwen-t2 | deepseek-v4-flash-0731@qwen-t2 | llm | passed |
| t2.qwen.qwen-t2.deepseek-v4-flash.llm | T2 | qwen/qwen-t2 | deepseek-v4-flash@qwen-t2 | llm | passed |
| t2.qwen.qwen-t2.deepseek-v4-pro-0813.llm | T2 | qwen/qwen-t2 | deepseek-v4-pro-0813@qwen-t2 | llm | passed |
| t2.qwen.qwen-t2.deepseek-v4-pro.llm | T2 | qwen/qwen-t2 | deepseek-v4-pro@qwen-t2 | llm | passed |
| t2.qwen.qwen-t2.deepseek-v4.1-flash.llm | T2 | qwen/qwen-t2 | deepseek-v4.1-flash@qwen-t2 | llm | passed |
| t2.qwen.qwen-t2.glm-4.7.llm | T2 | qwen/qwen-t2 | glm-4.7@qwen-t2 | llm | passed |
| t2.qwen.qwen-t2.glm-5.1.llm | T2 | qwen/qwen-t2 | glm-5.1@qwen-t2 | llm | failed |
| t2.qwen.qwen-t2.glm-5.2.llm | T2 | qwen/qwen-t2 | glm-5.2@qwen-t2 | llm | passed |
| t2.qwen.qwen-t2.glm-5.3.llm | T2 | qwen/qwen-t2 | glm-5.3@qwen-t2 | llm | failed |
| t2.qwen.qwen-t2.glm-5.llm | T2 | qwen/qwen-t2 | glm-5@qwen-t2 | llm | passed |
| t2.qwen.qwen-t2.kimi-k2.6.llm | T2 | qwen/qwen-t2 | kimi-k2.6@qwen-t2 | llm | passed |
| t2.qwen.qwen-t2.kimi-k2.7-code.llm | T2 | qwen/qwen-t2 | kimi-k2.7-code@qwen-t2 | llm | passed |
| t2.qwen.qwen-t2.kimi-k3.llm | T2 | qwen/qwen-t2 | kimi-k3@qwen-t2 | llm | passed |
| t2.qwen.qwen-t2.minimax-m2.1.llm | T2 | qwen/qwen-t2 | MiniMax-M2.1@qwen-t2 | llm | passed |
| t2.qwen.qwen-t2.minimax-m2.5.llm | T2 | qwen/qwen-t2 | MiniMax-M2.5@qwen-t2 | llm | failed |

## Physical model coverage

| Provider | Inventory model | Physical model | Decision | Reason | Retained representative |
|---|---|---|---|---|---|
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
| qwen/qwen-t2 | MiniMax-M2.1 | MiniMax-M2.1 | included | physical model | - |
| qwen/qwen-t2 | MiniMax-M2.5 | MiniMax-M2.5 | included | physical model | - |
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
| qwen/qwen-t2 | MiniMax-M2.1 | MiniMax-M2.1 | included | physical model | - |
| qwen/qwen-t2 | MiniMax-M2.5 | MiniMax-M2.5 | included | physical model | - |

## Confirmed product defects

- **defect.aicc.t2.preflight.baseline_mismatch.1** (AICC, baseline_mismatch): expected official Provider capability, AICC inventory declaration, and adapter support are bidirectionally consistent; observed official_not_supported_but_aicc_advertised deepseek-v4.1-flash@qwen-t2 vision.caption. Evidence: cases/t2.preflight.baseline_mismatch.1.json, effective_baseline.json
- **defect.aicc.t2.preflight.baseline_mismatch.2** (AICC, baseline_mismatch): expected official Provider capability, AICC inventory declaration, and adapter support are bidirectionally consistent; observed official_not_supported_but_aicc_advertised deepseek-v4.1-flash@qwen-t2 vision.ocr. Evidence: cases/t2.preflight.baseline_mismatch.2.json, effective_baseline.json
- **defect.aicc.t2.preflight.baseline_mismatch.3** (AICC, baseline_mismatch): expected official Provider capability, AICC inventory declaration, and adapter support are bidirectionally consistent; observed official_not_supported_but_aicc_advertised kimi-k2.6@qwen-t2 vision.caption. Evidence: cases/t2.preflight.baseline_mismatch.3.json, effective_baseline.json
- **defect.aicc.t2.preflight.baseline_mismatch.4** (AICC, baseline_mismatch): expected official Provider capability, AICC inventory declaration, and adapter support are bidirectionally consistent; observed official_not_supported_but_aicc_advertised kimi-k2.6@qwen-t2 vision.ocr. Evidence: cases/t2.preflight.baseline_mismatch.4.json, effective_baseline.json
- **defect.aicc.t2.preflight.baseline_mismatch.5** (AICC, baseline_mismatch): expected official Provider capability, AICC inventory declaration, and adapter support are bidirectionally consistent; observed official_not_supported_but_aicc_advertised kimi-k2.7-code@qwen-t2 vision.caption. Evidence: cases/t2.preflight.baseline_mismatch.5.json, effective_baseline.json
- **defect.aicc.t2.preflight.baseline_mismatch.6** (AICC, baseline_mismatch): expected official Provider capability, AICC inventory declaration, and adapter support are bidirectionally consistent; observed official_not_supported_but_aicc_advertised kimi-k2.7-code@qwen-t2 vision.ocr. Evidence: cases/t2.preflight.baseline_mismatch.6.json, effective_baseline.json
- **defect.aicc.t2.preflight.baseline_mismatch.7** (AICC, baseline_mismatch): expected official Provider capability, AICC inventory declaration, and adapter support are bidirectionally consistent; observed official_not_supported_but_aicc_advertised kimi-k3@qwen-t2 vision.caption. Evidence: cases/t2.preflight.baseline_mismatch.7.json, effective_baseline.json
- **defect.aicc.t2.preflight.baseline_mismatch.8** (AICC, baseline_mismatch): expected official Provider capability, AICC inventory declaration, and adapter support are bidirectionally consistent; observed official_not_supported_but_aicc_advertised kimi-k3@qwen-t2 vision.ocr. Evidence: cases/t2.preflight.baseline_mismatch.8.json, effective_baseline.json
- **defect.aicc.t2.qwen.qwen-t2.glm-5.1.llm** (AICC, assertion_failed): expected the selected exact model satisfies the case routing, capability, resource, and security assertions; observed Error: LLM output omitted the requested acceptance marker. Evidence: cases/t2.qwen.qwen-t2.glm-5.1.llm.json
- **defect.aicc.t2.qwen.qwen-t2.glm-5.3.llm** (AICC, assertion_failed): expected the selected exact model satisfies the case routing, capability, resource, and security assertions; observed Error: LLM output omitted the requested acceptance marker. Evidence: cases/t2.qwen.qwen-t2.glm-5.3.llm.json
- **defect.aicc.t2.qwen.qwen-t2.minimax-m2.5.llm** (AICC, assertion_failed): expected the selected exact model satisfies the case routing, capability, resource, and security assertions; observed Error: LLM output omitted the requested acceptance marker. Evidence: cases/t2.qwen.qwen-t2.minimax-m2.5.llm.json

## Cleanup

Status: passed

- removed 0 uploaded fixture object(s)
- removed 0 generated output object(s)
- temporary Provider credentials restored after testing (qwen)
