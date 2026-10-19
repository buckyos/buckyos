# AICC E2E Acceptance Report

- Run: `aicc-2026-10-18T06-10-41-691Z-279cc3b4`
- Commit: `1e8f93081e1a1b9561e77a78e3f7cb67f9922330`
- Capability baseline: `2026-10-18.1`
- Real model calls: 0/0
- Planned maximum cost: $0.000000
- Known actual / unknown estimated exposure: $0.000000 / $0.000000
- Total exposure / budget: $0.000000 / $1.000000
- Unknown-cost calls: 0; budget exceeded: false
- Results: passed=0, failed=10, provider_restricted=0, skipped=1, not_applicable=0, review=0

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
| t2.preflight.provider_missing.sn-ai-provider | T2 | sn-ai-provider/- | - | models.list | skipped |

## Physical model coverage

| Provider | Inventory model | Physical model | Decision | Reason | Retained representative |
|---|---|---|---|---|---|
| sn-ai-provider/sn-ai-provider-catalog | glm-5.3 | glm-5.3 | included | physical model | - |
| sn-ai-provider/sn-ai-provider-catalog | glm-5 | glm-5 | included | physical model | - |
| sn-ai-provider/sn-ai-provider-catalog | kimi-k3 | kimi-k3 | included | physical model | - |
| sn-ai-provider/sn-ai-provider-catalog | qwen-image-3-pro | qwen-image-3-pro | included | physical model | - |
| sn-ai-provider/sn-ai-provider-catalog | qwen3-embedding-8b | qwen3-embedding-8b | included | physical model | - |
| sn-ai-provider/sn-ai-provider-catalog | qwen3.6-plus | qwen3.6-plus | included | physical model | - |
| sn-ai-provider/sn-ai-provider-catalog | qwen3.7-plus | qwen3.7-plus | included | physical model | - |
| sn-ai-provider/sn-ai-provider-catalog | qwen3.8-max-0902 | qwen3.8-max-0902 | included | physical model | - |
| sn-ai-provider/sn-ai-provider-catalog | ui-tars-1.5-7b | ui-tars-1.5-7b | included | physical model | - |
| sn-ai-provider/sn-ai-provider-catalog | voxtral-small-24b-2507 | voxtral-small-24b-2507 | included | physical model | - |
| sn-ai-provider/sn-ai-provider-catalog | wan-3.0 | wan-3.0 | filtered | unsupported_canonical_protocol | - |

## Confirmed product defects

- **defect.aicc.t2.preflight.baseline_mismatch.1** (AICC, baseline_mismatch): expected official Provider capability, AICC inventory declaration, and adapter support are bidirectionally consistent; observed official_supported_but_aicc_missing sn-ai-provider/glm-5.3. Evidence: cases/t2.preflight.baseline_mismatch.1.json, effective_baseline.json
- **defect.aicc.t2.preflight.baseline_mismatch.2** (AICC, baseline_mismatch): expected official Provider capability, AICC inventory declaration, and adapter support are bidirectionally consistent; observed official_supported_but_aicc_missing sn-ai-provider/glm-5. Evidence: cases/t2.preflight.baseline_mismatch.2.json, effective_baseline.json
- **defect.aicc.t2.preflight.baseline_mismatch.3** (AICC, baseline_mismatch): expected official Provider capability, AICC inventory declaration, and adapter support are bidirectionally consistent; observed official_supported_but_aicc_missing sn-ai-provider/kimi-k3. Evidence: cases/t2.preflight.baseline_mismatch.3.json, effective_baseline.json
- **defect.aicc.t2.preflight.baseline_mismatch.4** (AICC, baseline_mismatch): expected official Provider capability, AICC inventory declaration, and adapter support are bidirectionally consistent; observed official_supported_but_aicc_missing sn-ai-provider/qwen-image-3-pro. Evidence: cases/t2.preflight.baseline_mismatch.4.json, effective_baseline.json
- **defect.aicc.t2.preflight.baseline_mismatch.5** (AICC, baseline_mismatch): expected official Provider capability, AICC inventory declaration, and adapter support are bidirectionally consistent; observed official_supported_but_aicc_missing sn-ai-provider/qwen3-embedding-8b. Evidence: cases/t2.preflight.baseline_mismatch.5.json, effective_baseline.json
- **defect.aicc.t2.preflight.baseline_mismatch.6** (AICC, baseline_mismatch): expected official Provider capability, AICC inventory declaration, and adapter support are bidirectionally consistent; observed official_supported_but_aicc_missing sn-ai-provider/qwen3.6-plus. Evidence: cases/t2.preflight.baseline_mismatch.6.json, effective_baseline.json
- **defect.aicc.t2.preflight.baseline_mismatch.7** (AICC, baseline_mismatch): expected official Provider capability, AICC inventory declaration, and adapter support are bidirectionally consistent; observed official_supported_but_aicc_missing sn-ai-provider/qwen3.7-plus. Evidence: cases/t2.preflight.baseline_mismatch.7.json, effective_baseline.json
- **defect.aicc.t2.preflight.baseline_mismatch.8** (AICC, baseline_mismatch): expected official Provider capability, AICC inventory declaration, and adapter support are bidirectionally consistent; observed official_supported_but_aicc_missing sn-ai-provider/qwen3.8-max-0902. Evidence: cases/t2.preflight.baseline_mismatch.8.json, effective_baseline.json
- **defect.aicc.t2.preflight.baseline_mismatch.9** (AICC, baseline_mismatch): expected official Provider capability, AICC inventory declaration, and adapter support are bidirectionally consistent; observed official_supported_but_aicc_missing sn-ai-provider/ui-tars-1.5-7b. Evidence: cases/t2.preflight.baseline_mismatch.9.json, effective_baseline.json
- **defect.aicc.t2.preflight.baseline_mismatch.10** (AICC, baseline_mismatch): expected official Provider capability, AICC inventory declaration, and adapter support are bidirectionally consistent; observed official_supported_but_aicc_missing sn-ai-provider/voxtral-small-24b-2507. Evidence: cases/t2.preflight.baseline_mismatch.10.json, effective_baseline.json

## Cleanup

Status: passed

- removed 0 uploaded fixture object(s)
- removed 0 generated output object(s)
