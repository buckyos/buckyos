# AICC E2E Acceptance Report

- Run: `aicc-2026-10-23T07-51-08-265Z-08d47181`
- Commit: `53b4431e2d46ec731598ed0343d4a6596bee8d15`
- Capability baseline: `2026-10-23.1`
- Real model calls: 4/4
- Planned maximum cost: $0.040000
- Known actual / unknown estimated exposure: $0.000000 / $0.040000
- Total exposure / budget: $0.040000 / $30.000000
- Unknown-cost calls: 4; budget exceeded: false
- Results: passed=0, failed=0, provider_restricted=0, skipped=0, not_applicable=0, review=4

## Targeted retest

Run after fixing the reported defect; repeat `--case` to select additional cases:

```bash
pnpm run acceptance:gateway -- --config aicc_acceptance.native.local.toml --allow-credential-mutation --timeout-ms 900000 --no-real-model-calls
```

## Cases

| Case | Layer | Provider | Model | API | Status |
|---|---|---|---|---|---|
| t2.minimax.native-minimax-t2.minimax-h3-max.video.img2video | T2 | minimax/native-minimax-t2 | MiniMax-H3-Max@native-minimax-t2 | video.img2video | review |
| t2.minimax.native-minimax-t2.minimax-h3-max.video.txt2video | T2 | minimax/native-minimax-t2 | MiniMax-H3-Max@native-minimax-t2 | video.txt2video | review |
| t2.minimax.native-minimax-t2.minimax-h3.video.img2video | T2 | minimax/native-minimax-t2 | MiniMax-H3@native-minimax-t2 | video.img2video | review |
| t2.minimax.native-minimax-t2.minimax-h3.video.txt2video | T2 | minimax/native-minimax-t2 | MiniMax-H3@native-minimax-t2 | video.txt2video | review |

## Physical model coverage

| Provider | Inventory model | Physical model | Decision | Reason | Retained representative |
|---|---|---|---|---|---|
| minimax/native-minimax-t2 | MiniMax-H3-Max | MiniMax-H3-Max | included | physical model | - |
| minimax/native-minimax-t2 | MiniMax-H3 | MiniMax-H3 | included | physical model | - |
| minimax/native-minimax-t2 | MiniMax-H3-Max | MiniMax-H3-Max | included | physical model | - |
| minimax/native-minimax-t2 | MiniMax-H3 | MiniMax-H3 | included | physical model | - |

## Confirmed product defects

None.

## Cleanup

Status: passed

- removed 0 uploaded fixture object(s)
- removed 0 generated output object(s)
- temporary Provider credentials restored after testing (minimax)
