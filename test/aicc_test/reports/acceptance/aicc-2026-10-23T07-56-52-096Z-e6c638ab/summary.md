# AICC E2E Acceptance Report

- Run: `aicc-2026-10-23T07-56-52-096Z-e6c638ab`
- Commit: `53b4431e2d46ec731598ed0343d4a6596bee8d15`
- Capability baseline: `2026-10-23.1`
- Real model calls: 12/20
- Planned maximum cost: $0.200000
- Known actual / unknown estimated exposure: $0.000000 / $0.120000
- Total exposure / budget: $0.120000 / $30.000000
- Unknown-cost calls: 12; budget exceeded: false
- Results: passed=0, failed=0, provider_restricted=16, skipped=0, not_applicable=0, review=0

## Targeted retest

Run after fixing the reported defect; repeat `--case` to select additional cases:

```bash
pnpm run acceptance:gateway -- --config aicc_acceptance.native.local.toml --allow-credential-mutation --timeout-ms 900000 --no-real-model-calls
```

## Cases

| Case | Layer | Provider | Model | API | Status |
|---|---|---|---|---|---|
| t2.doubao-agent-plan.native-doubao-agent-plan-t2.doubao-seedance-2.0-fast.video.extend | T2 | doubao-agent-plan/native-doubao-agent-plan-t2 | doubao-seedance-2.0-fast@native-doubao-agent-plan-t2 | video.extend | provider_restricted |
| t2.doubao-agent-plan.native-doubao-agent-plan-t2.doubao-seedance-2.0-fast.video.img2video | T2 | doubao-agent-plan/native-doubao-agent-plan-t2 | doubao-seedance-2.0-fast@native-doubao-agent-plan-t2 | video.img2video | provider_restricted |
| t2.doubao-agent-plan.native-doubao-agent-plan-t2.doubao-seedance-2.0-fast.video.txt2video | T2 | doubao-agent-plan/native-doubao-agent-plan-t2 | doubao-seedance-2.0-fast@native-doubao-agent-plan-t2 | video.txt2video | provider_restricted |
| t2.doubao-agent-plan.native-doubao-agent-plan-t2.doubao-seedance-2.0-fast.video.video2video | T2 | doubao-agent-plan/native-doubao-agent-plan-t2 | doubao-seedance-2.0-fast@native-doubao-agent-plan-t2 | video.video2video | provider_restricted |
| t2.doubao-agent-plan.native-doubao-agent-plan-t2.doubao-seedance-2.0-mini.video.extend | T2 | doubao-agent-plan/native-doubao-agent-plan-t2 | doubao-seedance-2.0-mini@native-doubao-agent-plan-t2 | video.extend | provider_restricted |
| t2.doubao-agent-plan.native-doubao-agent-plan-t2.doubao-seedance-2.0-mini.video.img2video | T2 | doubao-agent-plan/native-doubao-agent-plan-t2 | doubao-seedance-2.0-mini@native-doubao-agent-plan-t2 | video.img2video | provider_restricted |
| t2.doubao-agent-plan.native-doubao-agent-plan-t2.doubao-seedance-2.0-mini.video.txt2video | T2 | doubao-agent-plan/native-doubao-agent-plan-t2 | doubao-seedance-2.0-mini@native-doubao-agent-plan-t2 | video.txt2video | provider_restricted |
| t2.doubao-agent-plan.native-doubao-agent-plan-t2.doubao-seedance-2.0-mini.video.video2video | T2 | doubao-agent-plan/native-doubao-agent-plan-t2 | doubao-seedance-2.0-mini@native-doubao-agent-plan-t2 | video.video2video | provider_restricted |
| t2.doubao-agent-plan.native-doubao-agent-plan-t2.doubao-seedance-2.0.video.extend | T2 | doubao-agent-plan/native-doubao-agent-plan-t2 | doubao-seedance-2.0@native-doubao-agent-plan-t2 | video.extend | provider_restricted |
| t2.doubao-agent-plan.native-doubao-agent-plan-t2.doubao-seedance-2.0.video.img2video | T2 | doubao-agent-plan/native-doubao-agent-plan-t2 | doubao-seedance-2.0@native-doubao-agent-plan-t2 | video.img2video | provider_restricted |
| t2.doubao-agent-plan.native-doubao-agent-plan-t2.doubao-seedance-2.0.video.txt2video | T2 | doubao-agent-plan/native-doubao-agent-plan-t2 | doubao-seedance-2.0@native-doubao-agent-plan-t2 | video.txt2video | provider_restricted |
| t2.doubao-agent-plan.native-doubao-agent-plan-t2.doubao-seedance-2.0.video.video2video | T2 | doubao-agent-plan/native-doubao-agent-plan-t2 | doubao-seedance-2.0@native-doubao-agent-plan-t2 | video.video2video | provider_restricted |
| t2.doubao-agent-plan.native-doubao-agent-plan-t2.doubao-seedance-2.5.video.extend | T2 | doubao-agent-plan/native-doubao-agent-plan-t2 | doubao-seedance-2.5@native-doubao-agent-plan-t2 | video.extend | provider_restricted |
| t2.doubao-agent-plan.native-doubao-agent-plan-t2.doubao-seedance-2.5.video.img2video | T2 | doubao-agent-plan/native-doubao-agent-plan-t2 | doubao-seedance-2.5@native-doubao-agent-plan-t2 | video.img2video | provider_restricted |
| t2.doubao-agent-plan.native-doubao-agent-plan-t2.doubao-seedance-2.5.video.txt2video | T2 | doubao-agent-plan/native-doubao-agent-plan-t2 | doubao-seedance-2.5@native-doubao-agent-plan-t2 | video.txt2video | provider_restricted |
| t2.doubao-agent-plan.native-doubao-agent-plan-t2.doubao-seedance-2.5.video.video2video | T2 | doubao-agent-plan/native-doubao-agent-plan-t2 | doubao-seedance-2.5@native-doubao-agent-plan-t2 | video.video2video | provider_restricted |

## Physical model coverage

| Provider | Inventory model | Physical model | Decision | Reason | Retained representative |
|---|---|---|---|---|---|
| doubao-agent-plan/native-doubao-agent-plan-t2 | doubao-seedance-2.0-fast | doubao-seedance-2.0-fast | included | physical model | - |
| doubao-agent-plan/native-doubao-agent-plan-t2 | doubao-seedance-2.0-mini | doubao-seedance-2.0-mini | included | physical model | - |
| doubao-agent-plan/native-doubao-agent-plan-t2 | doubao-seedance-2.0 | doubao-seedance-2.0 | included | physical model | - |
| doubao-agent-plan/native-doubao-agent-plan-t2 | doubao-seedance-2.5 | doubao-seedance-2.5 | included | physical model | - |
| doubao-agent-plan/native-doubao-agent-plan-t2 | doubao-seedance-2.0-fast | doubao-seedance-2.0-fast | included | physical model | - |
| doubao-agent-plan/native-doubao-agent-plan-t2 | doubao-seedance-2.0-mini | doubao-seedance-2.0-mini | included | physical model | - |
| doubao-agent-plan/native-doubao-agent-plan-t2 | doubao-seedance-2.0 | doubao-seedance-2.0 | included | physical model | - |
| doubao-agent-plan/native-doubao-agent-plan-t2 | doubao-seedance-2.5 | doubao-seedance-2.5 | included | physical model | - |

## Confirmed product defects

None.

## Cleanup

Status: passed

- removed 0 uploaded fixture object(s)
- removed 0 generated output object(s)
- temporary Provider credentials restored after testing (doubao-agent-plan)
