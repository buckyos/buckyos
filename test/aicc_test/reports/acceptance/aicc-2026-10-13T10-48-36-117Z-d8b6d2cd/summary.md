# AICC E2E Acceptance Report

- Run: `aicc-2026-10-13T10-48-36-117Z-d8b6d2cd`
- Commit: `04f7c6abe791fd2fb95d16ae0cccf441a4746fc9`
- Capability baseline: `2026-10-13.1`
- Real model calls: 0/0
- Planned maximum cost: $0.000000
- Known actual / unknown estimated exposure: $0.000000 / $0.000000
- Total exposure / budget: $0.000000 / $0.200000
- Unknown-cost calls: 0; budget exceeded: false
- Results: passed=0, failed=0, provider_restricted=3, skipped=0, not_applicable=0, review=0

## Targeted retest

Run after fixing the reported defect; repeat `--case` to select additional cases:

```bash
pnpm run acceptance:gateway -- --config aicc_acceptance.local.toml --allow-credential-mutation --timeout-ms 900000 --no-real-model-calls
```

## Cases

| Case | Layer | Provider | Model | API | Status |
|---|---|---|---|---|---|
| t2.minimax.minimax-t2.minimax-m3.1-flash-preview.llm | T2 | minimax/minimax-t2 | MiniMax-M3.1-Flash-Preview@minimax-t2 | llm | provider_restricted |
| t2.minimax.minimax-t2.minimax-m3.1-flash-preview.vision.caption | T2 | minimax/minimax-t2 | MiniMax-M3.1-Flash-Preview@minimax-t2 | vision.caption | provider_restricted |
| t2.minimax.minimax-t2.minimax-m3.1-flash-preview.vision.ocr | T2 | minimax/minimax-t2 | MiniMax-M3.1-Flash-Preview@minimax-t2 | vision.ocr | provider_restricted |

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
