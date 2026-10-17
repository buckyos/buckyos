# AICC E2E Acceptance Report

- Run: `aicc-2026-10-14T16-02-17-344Z-fb71e32d`
- Commit: `5f803fc558207eae84a289904e3ee3bfd2342e93`
- Capability baseline: `2026-10-14.1`
- Real model calls: 0/36
- Planned maximum cost: $0.360000
- Known actual / unknown estimated exposure: $0.000000 / $0.000000
- Total exposure / budget: $0.000000 / $1.000000
- Unknown-cost calls: 0; budget exceeded: false
- Results: passed=0, failed=2, provider_restricted=0, skipped=36, not_applicable=0, review=0

## Targeted retest

Run after fixing the reported defect; repeat `--case` to select additional cases:

```bash
pnpm run acceptance:gateway -- --config aicc_acceptance.local.toml --allow-credential-mutation --timeout-ms 600000 --no-real-model-calls
```

## Cases

| Case | Layer | Provider | Model | API | Status |
|---|---|---|---|---|---|
| t2.preflight.baseline_mismatch.1 | T2 | -/- | - | models.list | failed |
| t2.preflight.baseline_mismatch.2 | T2 | -/- | - | models.list | failed |
| t2.claude.claude-t2.claude-fable-5-1.llm | T2 | claude/claude-t2 | claude-fable-5-1@claude-t2 | llm | skipped |
| t2.claude.claude-t2.claude-fable-5-1.vision.caption | T2 | claude/claude-t2 | claude-fable-5-1@claude-t2 | vision.caption | skipped |
| t2.claude.claude-t2.claude-fable-5-1.vision.ocr | T2 | claude/claude-t2 | claude-fable-5-1@claude-t2 | vision.ocr | skipped |
| t2.claude.claude-t2.claude-fable-5.llm | T2 | claude/claude-t2 | claude-fable-5@claude-t2 | llm | skipped |
| t2.claude.claude-t2.claude-fable-5.vision.caption | T2 | claude/claude-t2 | claude-fable-5@claude-t2 | vision.caption | skipped |
| t2.claude.claude-t2.claude-fable-5.vision.ocr | T2 | claude/claude-t2 | claude-fable-5@claude-t2 | vision.ocr | skipped |
| t2.claude.claude-t2.claude-haiku-4-5-20251001.llm | T2 | claude/claude-t2 | claude-haiku-4-5-20251001@claude-t2 | llm | skipped |
| t2.claude.claude-t2.claude-haiku-4-5-20251001.vision.caption | T2 | claude/claude-t2 | claude-haiku-4-5-20251001@claude-t2 | vision.caption | skipped |
| t2.claude.claude-t2.claude-haiku-4-5-20251001.vision.ocr | T2 | claude/claude-t2 | claude-haiku-4-5-20251001@claude-t2 | vision.ocr | skipped |
| t2.claude.claude-t2.claude-opus-4-5-20251101.llm | T2 | claude/claude-t2 | claude-opus-4-5-20251101@claude-t2 | llm | skipped |
| t2.claude.claude-t2.claude-opus-4-5-20251101.vision.caption | T2 | claude/claude-t2 | claude-opus-4-5-20251101@claude-t2 | vision.caption | skipped |
| t2.claude.claude-t2.claude-opus-4-5-20251101.vision.ocr | T2 | claude/claude-t2 | claude-opus-4-5-20251101@claude-t2 | vision.ocr | skipped |
| t2.claude.claude-t2.claude-opus-4-6.llm | T2 | claude/claude-t2 | claude-opus-4-6@claude-t2 | llm | skipped |
| t2.claude.claude-t2.claude-opus-4-6.vision.caption | T2 | claude/claude-t2 | claude-opus-4-6@claude-t2 | vision.caption | skipped |
| t2.claude.claude-t2.claude-opus-4-6.vision.ocr | T2 | claude/claude-t2 | claude-opus-4-6@claude-t2 | vision.ocr | skipped |
| t2.claude.claude-t2.claude-opus-4-7.llm | T2 | claude/claude-t2 | claude-opus-4-7@claude-t2 | llm | skipped |
| t2.claude.claude-t2.claude-opus-4-7.vision.caption | T2 | claude/claude-t2 | claude-opus-4-7@claude-t2 | vision.caption | skipped |
| t2.claude.claude-t2.claude-opus-4-7.vision.ocr | T2 | claude/claude-t2 | claude-opus-4-7@claude-t2 | vision.ocr | skipped |
| t2.claude.claude-t2.claude-opus-4-8.llm | T2 | claude/claude-t2 | claude-opus-4-8@claude-t2 | llm | skipped |
| t2.claude.claude-t2.claude-opus-4-8.vision.caption | T2 | claude/claude-t2 | claude-opus-4-8@claude-t2 | vision.caption | skipped |
| t2.claude.claude-t2.claude-opus-4-8.vision.ocr | T2 | claude/claude-t2 | claude-opus-4-8@claude-t2 | vision.ocr | skipped |
| t2.claude.claude-t2.claude-opus-5-5.llm | T2 | claude/claude-t2 | claude-opus-5-5@claude-t2 | llm | skipped |
| t2.claude.claude-t2.claude-opus-5-5.vision.caption | T2 | claude/claude-t2 | claude-opus-5-5@claude-t2 | vision.caption | skipped |
| t2.claude.claude-t2.claude-opus-5-5.vision.ocr | T2 | claude/claude-t2 | claude-opus-5-5@claude-t2 | vision.ocr | skipped |
| t2.claude.claude-t2.claude-opus-5.llm | T2 | claude/claude-t2 | claude-opus-5@claude-t2 | llm | skipped |
| t2.claude.claude-t2.claude-opus-5.vision.caption | T2 | claude/claude-t2 | claude-opus-5@claude-t2 | vision.caption | skipped |
| t2.claude.claude-t2.claude-opus-5.vision.ocr | T2 | claude/claude-t2 | claude-opus-5@claude-t2 | vision.ocr | skipped |
| t2.claude.claude-t2.claude-sonnet-4-5-20250929.llm | T2 | claude/claude-t2 | claude-sonnet-4-5-20250929@claude-t2 | llm | skipped |
| t2.claude.claude-t2.claude-sonnet-4-5-20250929.vision.caption | T2 | claude/claude-t2 | claude-sonnet-4-5-20250929@claude-t2 | vision.caption | skipped |
| t2.claude.claude-t2.claude-sonnet-4-5-20250929.vision.ocr | T2 | claude/claude-t2 | claude-sonnet-4-5-20250929@claude-t2 | vision.ocr | skipped |
| t2.claude.claude-t2.claude-sonnet-4-6.llm | T2 | claude/claude-t2 | claude-sonnet-4-6@claude-t2 | llm | skipped |
| t2.claude.claude-t2.claude-sonnet-4-6.vision.caption | T2 | claude/claude-t2 | claude-sonnet-4-6@claude-t2 | vision.caption | skipped |
| t2.claude.claude-t2.claude-sonnet-4-6.vision.ocr | T2 | claude/claude-t2 | claude-sonnet-4-6@claude-t2 | vision.ocr | skipped |
| t2.claude.claude-t2.claude-sonnet-5.llm | T2 | claude/claude-t2 | claude-sonnet-5@claude-t2 | llm | skipped |
| t2.claude.claude-t2.claude-sonnet-5.vision.caption | T2 | claude/claude-t2 | claude-sonnet-5@claude-t2 | vision.caption | skipped |
| t2.claude.claude-t2.claude-sonnet-5.vision.ocr | T2 | claude/claude-t2 | claude-sonnet-5@claude-t2 | vision.ocr | skipped |

## Physical model coverage

| Provider | Inventory model | Physical model | Decision | Reason | Retained representative |
|---|---|---|---|---|---|
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
| claude/claude-t2 | claude-fable-5-1 | claude-fable-5-1 | included | physical model | - |
| claude/claude-t2 | claude-fable-5 | claude-fable-5 | included | physical model | - |
| claude/claude-t2 | claude-haiku-4-5-20251001 | claude-haiku-4-5-20251001 | included | physical model | - |
| claude/claude-t2 | claude-opus-4-5-20251101 | claude-opus-4-5-20251101 | included | physical model | - |
| claude/claude-t2 | claude-opus-4-6 | claude-opus-4-6 | included | physical model | - |
| claude/claude-t2 | claude-opus-4-7 | claude-opus-4-7 | included | physical model | - |
| claude/claude-t2 | claude-opus-4-8 | claude-opus-4-8 | included | physical model | - |
| claude/claude-t2 | claude-opus-5-5 | claude-opus-5-5 | included | physical model | - |
| claude/claude-t2 | claude-opus-5 | claude-opus-5 | included | physical model | - |
| claude/claude-t2 | claude-sonnet-4-5-20250929 | claude-sonnet-4-5-20250929 | included | physical model | - |
| claude/claude-t2 | claude-sonnet-4-6 | claude-sonnet-4-6 | included | physical model | - |
| claude/claude-t2 | claude-sonnet-5 | claude-sonnet-5 | included | physical model | - |

## Confirmed product defects

- **defect.aicc.t2.preflight.baseline_mismatch.1** (AICC, baseline_mismatch): expected official Provider capability, AICC inventory declaration, and adapter support are bidirectionally consistent; observed official_supported_but_aicc_missing claude/claude-haiku-5-5. Evidence: cases/t2.preflight.baseline_mismatch.1.json, effective_baseline.json
- **defect.aicc.t2.preflight.baseline_mismatch.2** (AICC, baseline_mismatch): expected official Provider capability, AICC inventory declaration, and adapter support are bidirectionally consistent; observed official_supported_but_aicc_missing claude/claude-sonnet-5-5. Evidence: cases/t2.preflight.baseline_mismatch.2.json, effective_baseline.json

## Cleanup

Status: passed

- removed 0 uploaded fixture object(s)
- removed 0 generated output object(s)
- temporary Provider credentials restored after testing (claude)
