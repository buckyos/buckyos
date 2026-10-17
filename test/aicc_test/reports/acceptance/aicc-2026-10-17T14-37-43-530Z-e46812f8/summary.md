# AICC E2E Acceptance Report

- Run: `aicc-2026-10-17T14-37-43-530Z-e46812f8`
- Commit: `b06b8044b7bf3d5731e14d855ba33c70a5673e27`
- Capability baseline: `2026-10-14.1`
- Real model calls: 3/3
- Planned maximum cost: $0.030000
- Known actual / unknown estimated exposure: $0.000269 / $0.000000
- Total exposure / budget: $0.000269 / $0.100000
- Unknown-cost calls: 0; budget exceeded: false
- Results: passed=1, failed=13, provider_restricted=0, skipped=0, not_applicable=0, review=2

## Targeted retest

Run after fixing the reported defect; repeat `--case` to select additional cases:

```bash
pnpm run acceptance:gateway -- --config /home/zhangzhen/buckyos/test/aicc_test/aicc_acceptance.local.toml --allow-credential-mutation --timeout-ms 900000 --no-real-model-calls
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
| t2.claude.claude-t2.claude-haiku-5-5.llm | T2 | claude/claude-t2 | claude-haiku-5-5@claude-t2 | llm | passed |
| t2.claude.claude-t2.claude-haiku-5-5.vision.caption | T2 | claude/claude-t2 | claude-haiku-5-5@claude-t2 | vision.caption | review |
| t2.claude.claude-t2.claude-haiku-5-5.vision.ocr | T2 | claude/claude-t2 | claude-haiku-5-5@claude-t2 | vision.ocr | review |

## Physical model coverage

| Provider | Inventory model | Physical model | Decision | Reason | Retained representative |
|---|---|---|---|---|---|
| claude/claude-t2 | claude-haiku-5-5 | claude-haiku-5-5 | included | physical model | - |
| claude/claude-t2 | claude-fable-5-1 | claude-fable-5-1 | included | physical model | - |
| claude/claude-t2 | claude-fable-5 | claude-fable-5 | included | physical model | - |
| claude/claude-t2 | claude-haiku-4-5-20251001 | claude-haiku-4-5-20251001 | included | physical model | - |
| claude/claude-t2 | claude-haiku-5-5 | claude-haiku-5-5 | included | physical model | - |
| claude/claude-t2 | claude-opus-4-5-20251101 | claude-opus-4-5-20251101 | included | physical model | - |
| claude/claude-t2 | claude-opus-4-6 | claude-opus-4-6 | included | physical model | - |
| claude/claude-t2 | claude-opus-4-7 | claude-opus-4-7 | included | physical model | - |
| claude/claude-t2 | claude-opus-4-8 | claude-opus-4-8 | included | physical model | - |
| claude/claude-t2 | claude-opus-5-5 | claude-opus-5-5 | included | physical model | - |
| claude/claude-t2 | claude-opus-5 | claude-opus-5 | included | physical model | - |
| claude/claude-t2 | claude-sonnet-4-5-20250929 | claude-sonnet-4-5-20250929 | included | physical model | - |
| claude/claude-t2 | claude-sonnet-4-6 | claude-sonnet-4-6 | included | physical model | - |
| claude/claude-t2 | claude-sonnet-5-5 | claude-sonnet-5-5 | included | physical model | - |
| claude/claude-t2 | claude-sonnet-5 | claude-sonnet-5 | included | physical model | - |

## Confirmed product defects

- **defect.aicc.t2.preflight.baseline_mismatch.1** (AICC, baseline_mismatch): expected official Provider capability, AICC inventory declaration, and adapter support are bidirectionally consistent; observed aicc_advertised_but_official_missing claude/claude-fable-5-1. Evidence: cases/t2.preflight.baseline_mismatch.1.json, effective_baseline.json
- **defect.aicc.t2.preflight.baseline_mismatch.2** (AICC, baseline_mismatch): expected official Provider capability, AICC inventory declaration, and adapter support are bidirectionally consistent; observed aicc_advertised_but_official_missing claude/claude-fable-5. Evidence: cases/t2.preflight.baseline_mismatch.2.json, effective_baseline.json
- **defect.aicc.t2.preflight.baseline_mismatch.3** (AICC, baseline_mismatch): expected official Provider capability, AICC inventory declaration, and adapter support are bidirectionally consistent; observed aicc_advertised_but_official_missing claude/claude-haiku-4-5-20251001. Evidence: cases/t2.preflight.baseline_mismatch.3.json, effective_baseline.json
- **defect.aicc.t2.preflight.baseline_mismatch.4** (AICC, baseline_mismatch): expected official Provider capability, AICC inventory declaration, and adapter support are bidirectionally consistent; observed aicc_advertised_but_official_missing claude/claude-opus-4-5-20251101. Evidence: cases/t2.preflight.baseline_mismatch.4.json, effective_baseline.json
- **defect.aicc.t2.preflight.baseline_mismatch.5** (AICC, baseline_mismatch): expected official Provider capability, AICC inventory declaration, and adapter support are bidirectionally consistent; observed aicc_advertised_but_official_missing claude/claude-opus-4-6. Evidence: cases/t2.preflight.baseline_mismatch.5.json, effective_baseline.json
- **defect.aicc.t2.preflight.baseline_mismatch.6** (AICC, baseline_mismatch): expected official Provider capability, AICC inventory declaration, and adapter support are bidirectionally consistent; observed aicc_advertised_but_official_missing claude/claude-opus-4-7. Evidence: cases/t2.preflight.baseline_mismatch.6.json, effective_baseline.json
- **defect.aicc.t2.preflight.baseline_mismatch.7** (AICC, baseline_mismatch): expected official Provider capability, AICC inventory declaration, and adapter support are bidirectionally consistent; observed aicc_advertised_but_official_missing claude/claude-opus-4-8. Evidence: cases/t2.preflight.baseline_mismatch.7.json, effective_baseline.json
- **defect.aicc.t2.preflight.baseline_mismatch.8** (AICC, baseline_mismatch): expected official Provider capability, AICC inventory declaration, and adapter support are bidirectionally consistent; observed aicc_advertised_but_official_missing claude/claude-opus-5-5. Evidence: cases/t2.preflight.baseline_mismatch.8.json, effective_baseline.json
- **defect.aicc.t2.preflight.baseline_mismatch.9** (AICC, baseline_mismatch): expected official Provider capability, AICC inventory declaration, and adapter support are bidirectionally consistent; observed aicc_advertised_but_official_missing claude/claude-opus-5. Evidence: cases/t2.preflight.baseline_mismatch.9.json, effective_baseline.json
- **defect.aicc.t2.preflight.baseline_mismatch.10** (AICC, baseline_mismatch): expected official Provider capability, AICC inventory declaration, and adapter support are bidirectionally consistent; observed aicc_advertised_but_official_missing claude/claude-sonnet-4-5-20250929. Evidence: cases/t2.preflight.baseline_mismatch.10.json, effective_baseline.json
- **defect.aicc.t2.preflight.baseline_mismatch.11** (AICC, baseline_mismatch): expected official Provider capability, AICC inventory declaration, and adapter support are bidirectionally consistent; observed aicc_advertised_but_official_missing claude/claude-sonnet-4-6. Evidence: cases/t2.preflight.baseline_mismatch.11.json, effective_baseline.json
- **defect.aicc.t2.preflight.baseline_mismatch.12** (AICC, baseline_mismatch): expected official Provider capability, AICC inventory declaration, and adapter support are bidirectionally consistent; observed aicc_advertised_but_official_missing claude/claude-sonnet-5-5. Evidence: cases/t2.preflight.baseline_mismatch.12.json, effective_baseline.json
- **defect.aicc.t2.preflight.baseline_mismatch.13** (AICC, baseline_mismatch): expected official Provider capability, AICC inventory declaration, and adapter support are bidirectionally consistent; observed aicc_advertised_but_official_missing claude/claude-sonnet-5. Evidence: cases/t2.preflight.baseline_mismatch.13.json, effective_baseline.json

## Cleanup

Status: passed

- removed 0 uploaded fixture object(s)
- removed 0 generated output object(s)
- temporary Provider credentials restored after testing (claude)
