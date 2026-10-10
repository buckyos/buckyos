# AICC E2E Acceptance Report

- Run: `aicc-2026-10-08T05-54-06-572Z-2d9bf678`
- Commit: `7612d141be9938f5d1c4a89b41f20ae80459c1c1`
- Capability baseline: `2026-10-08.1`
- Real model calls: 7/8
- Planned maximum cost: $0.080000
- Known actual / unknown estimated exposure: $0.000000 / $0.070000
- Total exposure / budget: $0.070000 / $0.120000
- Unknown-cost calls: 7; budget exceeded: false
- Results: passed=2, failed=2, provider_restricted=0, skipped=1, not_applicable=0, review=0

## Targeted retest

Run after fixing the reported defect; repeat `--case` to select additional cases:

```bash
pnpm run acceptance:gateway -- --config aicc_acceptance.local.toml --allow-credential-mutation --timeout-ms 900000 --provider kimi --case t2.kimi.kimi-t2.kimi-k2.6.vision.ocr --case t2.kimi.kimi-t2.kimi-k2.7-code.vision.ocr
```

## Cases

| Case | Layer | Provider | Model | API | Status |
|---|---|---|---|---|---|
| t2.preflight.provider_no_eligible_models.minimax | T2 | minimax/- | - | model_coverage.filter | skipped |
| t2.kimi.kimi-t2.kimi-k2.6.vision.caption | T2 | kimi/kimi-t2 | kimi-k2.6@kimi-t2 | vision.caption | passed |
| t2.kimi.kimi-t2.kimi-k2.6.vision.ocr | T2 | kimi/kimi-t2 | kimi-k2.6@kimi-t2 | vision.ocr | failed |
| t2.kimi.kimi-t2.kimi-k2.7-code.vision.caption | T2 | kimi/kimi-t2 | kimi-k2.7-code@kimi-t2 | vision.caption | passed |
| t2.kimi.kimi-t2.kimi-k2.7-code.vision.ocr | T2 | kimi/kimi-t2 | kimi-k2.7-code@kimi-t2 | vision.ocr | failed |

## Physical model coverage

| Provider | Inventory model | Physical model | Decision | Reason | Retained representative |
|---|---|---|---|---|---|
| kimi/kimi-t2 | kimi-k2.6 | kimi-k2.6 | included | physical model | - |
| kimi/kimi-t2 | kimi-k2.7-code | kimi-k2.7-code | included | physical model | - |
| kimi/kimi-t2 | kimi-k2.6 | kimi-k2.6 | included | physical model | - |
| kimi/kimi-t2 | kimi-k2.7-code | kimi-k2.7-code | included | physical model | - |

## Confirmed product defects

- **defect.aicc.t2.kimi.kimi-t2.kimi-k2.6.vision.ocr** (AICC, assertion_failed): expected the selected exact model satisfies the case routing, capability, resource, and security assertions; observed RPCError: RPC call error: 500. Evidence: cases/t2.kimi.kimi-t2.kimi-k2.6.vision.ocr.json

## Cleanup

Status: passed

- removed 0 uploaded fixture object(s)
- removed 0 generated output object(s)
- temporary Provider credentials restored after testing (kimi, minimax)
