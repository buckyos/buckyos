# AICC E2E Acceptance Report

- Run: `aicc-2026-10-08T14-04-29-790Z-ded511de`
- Commit: `7612d141be9938f5d1c4a89b41f20ae80459c1c1`
- Capability baseline: `2026-10-08.3`
- Real model calls: 1/1
- Planned maximum cost: $0.010000
- Known actual / unknown estimated exposure: $0.000253 / $0.000000
- Total exposure / budget: $0.000253 / $0.010000
- Unknown-cost calls: 0; budget exceeded: false
- Results: passed=0, failed=0, provider_restricted=0, skipped=0, not_applicable=0, review=1

## Targeted retest

Run after fixing the reported defect; repeat `--case` to select additional cases:

```bash
pnpm run acceptance:gateway -- --config aicc_acceptance.local.toml --allow-credential-mutation --timeout-ms 900000 --no-real-model-calls
```

## Cases

| Case | Layer | Provider | Model | API | Status |
|---|---|---|---|---|---|
| t2.deepseek.deepseek-t2.deepseek-flash.vision.caption | T2 | deepseek/deepseek-t2 | deepseek-flash@deepseek-t2 | vision.caption | review |

## Physical model coverage

| Provider | Inventory model | Physical model | Decision | Reason | Retained representative |
|---|---|---|---|---|---|
| deepseek/deepseek-t2 | deepseek-flash | deepseek-flash | included | physical model | - |
| deepseek/deepseek-t2 | deepseek-flash | deepseek-flash | included | physical model | - |

## Confirmed product defects

None.

## Cleanup

Status: passed

- removed 0 uploaded fixture object(s)
- removed 0 generated output object(s)
- temporary Provider credentials restored after testing (deepseek)
