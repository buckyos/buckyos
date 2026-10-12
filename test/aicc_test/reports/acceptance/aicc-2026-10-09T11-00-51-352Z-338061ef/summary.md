# AICC E2E Acceptance Report

- Run: `aicc-2026-10-09T11-00-51-352Z-338061ef`
- Commit: `0c32470f0ebaa0f14a1a262bc2bcb0eea553d8fc`
- Capability baseline: `2026-10-09.1`
- Real model calls: 1/1
- Planned maximum cost: $0.010000
- Known actual / unknown estimated exposure: $0.000000 / $0.010000
- Total exposure / budget: $0.010000 / $0.010000
- Unknown-cost calls: 1; budget exceeded: false
- Results: passed=1, failed=2, provider_restricted=0, skipped=0, not_applicable=0, review=0

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
| t2.qwen.qwen-t2.qwen3.8-omni-flash.llm | T2 | qwen/qwen-t2 | qwen3.8-omni-flash@qwen-t2 | llm | passed |

## Physical model coverage

| Provider | Inventory model | Physical model | Decision | Reason | Retained representative |
|---|---|---|---|---|---|
| qwen/qwen-t2 | qwen3.8-omni-flash | qwen3.8-omni-flash | included | physical model | - |
| qwen/qwen-t2 | qwen3.8-omni-flash | qwen3.8-omni-flash | included | physical model | - |

## Confirmed product defects

- **defect.aicc.t2.preflight.baseline_mismatch.1** (AICC, baseline_mismatch): expected official Provider capability, AICC inventory declaration, and adapter support are bidirectionally consistent; observed official_not_supported_but_aicc_advertised qwen3.8-omni-flash@qwen-t2 vision.caption. Evidence: cases/t2.preflight.baseline_mismatch.1.json, effective_baseline.json
- **defect.aicc.t2.preflight.baseline_mismatch.2** (AICC, baseline_mismatch): expected official Provider capability, AICC inventory declaration, and adapter support are bidirectionally consistent; observed official_not_supported_but_aicc_advertised qwen3.8-omni-flash@qwen-t2 vision.ocr. Evidence: cases/t2.preflight.baseline_mismatch.2.json, effective_baseline.json

## Cleanup

Status: passed

- removed 0 uploaded fixture object(s)
- removed 0 generated output object(s)
- temporary Provider credentials restored after testing (qwen)
