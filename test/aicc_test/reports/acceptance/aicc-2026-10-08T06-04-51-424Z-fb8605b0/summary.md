# AICC E2E Acceptance Report

- Run: `aicc-2026-10-08T06-04-51-424Z-fb8605b0`
- Commit: `7612d141be9938f5d1c4a89b41f20ae80459c1c1`
- Capability baseline: `2026-10-08.1`
- Real model calls: 4/4
- Planned maximum cost: $0.040000
- Known actual / unknown estimated exposure: $0.000000 / $0.040000
- Total exposure / budget: $0.040000 / $0.060000
- Unknown-cost calls: 4; budget exceeded: false
- Results: passed=1, failed=0, provider_restricted=0, skipped=0, not_applicable=0, review=1

## Targeted retest

Run after fixing the reported defect; repeat `--case` to select additional cases:

```bash
pnpm run acceptance:gateway -- --config aicc_acceptance.local.toml --allow-credential-mutation --timeout-ms 900000 --no-real-model-calls
```

## Cases

| Case | Layer | Provider | Model | API | Status |
|---|---|---|---|---|---|
| t2.minimax.minimax-t2.minimax-m3.vision.caption | T2 | minimax/minimax-t2 | MiniMax-M3@minimax-t2 | vision.caption | passed |
| t2.minimax.minimax-t2.minimax-m3.vision.ocr | T2 | minimax/minimax-t2 | MiniMax-M3@minimax-t2 | vision.ocr | review |

## Physical model coverage

| Provider | Inventory model | Physical model | Decision | Reason | Retained representative |
|---|---|---|---|---|---|
| minimax/minimax-t2 | MiniMax-M3 | MiniMax-M3 | included | physical model | - |
| minimax/minimax-t2 | MiniMax-M3 | MiniMax-M3 | included | physical model | - |

## Confirmed product defects

None.

## Cleanup

Status: passed

- removed 0 uploaded fixture object(s)
- removed 0 generated output object(s)
- temporary Provider credentials restored after testing (minimax)
