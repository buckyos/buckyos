# AICC E2E Acceptance Report

- Run: `aicc-2026-10-08T10-53-03-815Z-e9a08ce1`
- Commit: `7612d141be9938f5d1c4a89b41f20ae80459c1c1`
- Capability baseline: `2026-10-08.3`
- Real model calls: 8/8
- Planned maximum cost: $0.080000
- Known actual / unknown estimated exposure: $0.000000 / $0.080000
- Total exposure / budget: $0.080000 / $20.000000
- Unknown-cost calls: 8; budget exceeded: false
- Results: passed=0, failed=0, provider_restricted=6, skipped=0, not_applicable=0, review=2

## Targeted retest

Run after fixing the reported defect; repeat `--case` to select additional cases:

```bash
pnpm run acceptance:gateway -- --config aicc_acceptance.local.toml --allow-credential-mutation --timeout-ms 900000 --no-real-model-calls
```

## Cases

| Case | Layer | Provider | Model | API | Status |
|---|---|---|---|---|---|
| t2.kimi.kimi-t2.kimi-k2.7-code-highspeed.llm | T2 | kimi/kimi-t2 | kimi-k2.7-code-highspeed@kimi-t2 | llm | provider_restricted |
| t2.kimi.kimi-t2.kimi-k2.7-code-highspeed.vision.caption | T2 | kimi/kimi-t2 | kimi-k2.7-code-highspeed@kimi-t2 | vision.caption | provider_restricted |
| t2.kimi.kimi-t2.kimi-k2.7-code-highspeed.vision.ocr | T2 | kimi/kimi-t2 | kimi-k2.7-code-highspeed@kimi-t2 | vision.ocr | provider_restricted |
| t2.kimi.kimi-t2.kimi-k3.llm | T2 | kimi/kimi-t2 | kimi-k3@kimi-t2 | llm | provider_restricted |
| t2.kimi.kimi-t2.kimi-k3.vision.caption | T2 | kimi/kimi-t2 | kimi-k3@kimi-t2 | vision.caption | provider_restricted |
| t2.kimi.kimi-t2.kimi-k3.vision.ocr | T2 | kimi/kimi-t2 | kimi-k3@kimi-t2 | vision.ocr | provider_restricted |
| t2.minimax.minimax-t2.minimax-h3-max.video.img2video | T2 | minimax/minimax-t2 | MiniMax-H3-Max@minimax-t2 | video.img2video | review |
| t2.minimax.minimax-t2.minimax-h3.video.img2video | T2 | minimax/minimax-t2 | MiniMax-H3@minimax-t2 | video.img2video | review |

## Physical model coverage

| Provider | Inventory model | Physical model | Decision | Reason | Retained representative |
|---|---|---|---|---|---|
| kimi/kimi-t2 | kimi-k2.7-code-highspeed | kimi-k2.7-code-highspeed | included | physical model | - |
| kimi/kimi-t2 | kimi-k3 | kimi-k3 | included | physical model | - |
| minimax/minimax-t2 | MiniMax-H3-Max | MiniMax-H3-Max | included | physical model | - |
| minimax/minimax-t2 | MiniMax-H3 | MiniMax-H3 | included | physical model | - |
| kimi/kimi-t2 | kimi-k2.7-code-highspeed | kimi-k2.7-code-highspeed | included | physical model | - |
| kimi/kimi-t2 | kimi-k3 | kimi-k3 | included | physical model | - |
| minimax/minimax-t2 | MiniMax-H3-Max | MiniMax-H3-Max | included | physical model | - |
| minimax/minimax-t2 | MiniMax-H3 | MiniMax-H3 | included | physical model | - |

## Confirmed product defects

None.

## Cleanup

Status: passed

- removed 0 uploaded fixture object(s)
- removed 0 generated output object(s)
- temporary Provider credentials restored after testing (kimi, minimax)
