# AICC E2E Acceptance Report

- Run: `aicc-2026-10-08T05-44-41-155Z-e37ceeba`
- Commit: `7612d141be9938f5d1c4a89b41f20ae80459c1c1`
- Capability baseline: `2026-10-08.1`
- Real model calls: 16/16
- Planned maximum cost: $0.160000
- Known actual / unknown estimated exposure: $0.001842 / $0.090000
- Total exposure / budget: $0.091842 / $0.250000
- Unknown-cost calls: 9; budget exceeded: false
- Results: passed=10, failed=1, provider_restricted=0, skipped=0, not_applicable=0, review=5

## Targeted retest

Run after fixing the reported defect; repeat `--case` to select additional cases:

```bash
pnpm run acceptance:gateway -- --config aicc_acceptance.local.toml --allow-credential-mutation --timeout-ms 900000 --provider kimi --case t2.kimi.kimi-t2.kimi-k2.7-code.vision.ocr
```

## Cases

| Case | Layer | Provider | Model | API | Status |
|---|---|---|---|---|---|
| t2.kimi.kimi-t2.kimi-k2.6.llm | T2 | kimi/kimi-t2 | kimi-k2.6@kimi-t2 | llm | passed |
| t2.kimi.kimi-t2.kimi-k2.6.vision.caption | T2 | kimi/kimi-t2 | kimi-k2.6@kimi-t2 | vision.caption | review |
| t2.kimi.kimi-t2.kimi-k2.6.vision.ocr | T2 | kimi/kimi-t2 | kimi-k2.6@kimi-t2 | vision.ocr | review |
| t2.kimi.kimi-t2.kimi-k2.7-code.llm | T2 | kimi/kimi-t2 | kimi-k2.7-code@kimi-t2 | llm | passed |
| t2.kimi.kimi-t2.kimi-k2.7-code.vision.caption | T2 | kimi/kimi-t2 | kimi-k2.7-code@kimi-t2 | vision.caption | review |
| t2.kimi.kimi-t2.kimi-k2.7-code.vision.ocr | T2 | kimi/kimi-t2 | kimi-k2.7-code@kimi-t2 | vision.ocr | failed |
| t2.minimax.minimax-t2.minimax-m2.1-highspeed.llm | T2 | minimax/minimax-t2 | MiniMax-M2.1-highspeed@minimax-t2 | llm | passed |
| t2.minimax.minimax-t2.minimax-m2.1.llm | T2 | minimax/minimax-t2 | MiniMax-M2.1@minimax-t2 | llm | passed |
| t2.minimax.minimax-t2.minimax-m2.5-highspeed.llm | T2 | minimax/minimax-t2 | MiniMax-M2.5-highspeed@minimax-t2 | llm | passed |
| t2.minimax.minimax-t2.minimax-m2.5.llm | T2 | minimax/minimax-t2 | MiniMax-M2.5@minimax-t2 | llm | passed |
| t2.minimax.minimax-t2.minimax-m2.7-highspeed.llm | T2 | minimax/minimax-t2 | MiniMax-M2.7-highspeed@minimax-t2 | llm | passed |
| t2.minimax.minimax-t2.minimax-m2.7.llm | T2 | minimax/minimax-t2 | MiniMax-M2.7@minimax-t2 | llm | passed |
| t2.minimax.minimax-t2.minimax-m2.llm | T2 | minimax/minimax-t2 | MiniMax-M2@minimax-t2 | llm | passed |
| t2.minimax.minimax-t2.minimax-m3.llm | T2 | minimax/minimax-t2 | MiniMax-M3@minimax-t2 | llm | passed |
| t2.minimax.minimax-t2.minimax-m3.vision.caption | T2 | minimax/minimax-t2 | MiniMax-M3@minimax-t2 | vision.caption | review |
| t2.minimax.minimax-t2.minimax-m3.vision.ocr | T2 | minimax/minimax-t2 | MiniMax-M3@minimax-t2 | vision.ocr | review |

## Physical model coverage

| Provider | Inventory model | Physical model | Decision | Reason | Retained representative |
|---|---|---|---|---|---|
| kimi/kimi-t2 | kimi-k2.6 | kimi-k2.6 | included | physical model | - |
| kimi/kimi-t2 | kimi-k2.7-code | kimi-k2.7-code | included | physical model | - |
| minimax/minimax-t2 | MiniMax-M2.1-highspeed | MiniMax-M2.1-highspeed | included | physical model | - |
| minimax/minimax-t2 | MiniMax-M2.1 | MiniMax-M2.1 | included | physical model | - |
| minimax/minimax-t2 | MiniMax-M2.5-highspeed | MiniMax-M2.5-highspeed | included | physical model | - |
| minimax/minimax-t2 | MiniMax-M2.5 | MiniMax-M2.5 | included | physical model | - |
| minimax/minimax-t2 | MiniMax-M2.7-highspeed | MiniMax-M2.7-highspeed | included | physical model | - |
| minimax/minimax-t2 | MiniMax-M2.7 | MiniMax-M2.7 | included | physical model | - |
| minimax/minimax-t2 | MiniMax-M2 | MiniMax-M2 | included | physical model | - |
| minimax/minimax-t2 | MiniMax-M3 | MiniMax-M3 | included | physical model | - |
| kimi/kimi-t2 | kimi-k2.6 | kimi-k2.6 | included | physical model | - |
| kimi/kimi-t2 | kimi-k2.7-code | kimi-k2.7-code | included | physical model | - |
| minimax/minimax-t2 | MiniMax-M2.1-highspeed | MiniMax-M2.1-highspeed | included | physical model | - |
| minimax/minimax-t2 | MiniMax-M2.1 | MiniMax-M2.1 | included | physical model | - |
| minimax/minimax-t2 | MiniMax-M2.5-highspeed | MiniMax-M2.5-highspeed | included | physical model | - |
| minimax/minimax-t2 | MiniMax-M2.5 | MiniMax-M2.5 | included | physical model | - |
| minimax/minimax-t2 | MiniMax-M2.7-highspeed | MiniMax-M2.7-highspeed | included | physical model | - |
| minimax/minimax-t2 | MiniMax-M2.7 | MiniMax-M2.7 | included | physical model | - |
| minimax/minimax-t2 | MiniMax-M2 | MiniMax-M2 | included | physical model | - |
| minimax/minimax-t2 | MiniMax-M3 | MiniMax-M3 | included | physical model | - |

## Confirmed product defects

None.

## Cleanup

Status: passed

- removed 0 uploaded fixture object(s)
- removed 0 generated output object(s)
- temporary Provider credentials restored after testing (kimi, minimax)
