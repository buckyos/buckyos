# AICC E2E Acceptance Report

- Run: `t15-20261010043220-2666006`
- Commit: `b6ed2453535ad05256d4809fe33d72f33d2585b7`
- Capability baseline: `official-provider-protocols-2026-10-05.1`
- Real model calls: 0/0
- Planned maximum cost: $0.000000
- Known actual / unknown estimated exposure: $0.000000 / $0.000000
- Total exposure / budget: $0.000000 / $0.000000
- Unknown-cost calls: 0; budget exceeded: false
- Results: passed=0, failed=1, provider_restricted=0, skipped=0, not_applicable=0, review=0
- Manifest coverage: 1/1 (100.00%); passed=0, failed=1

## Targeted retest

Run after fixing the reported defect; repeat `--case` to select additional cases:

```bash
AICC_T15_ALLOW_CONFIG_MUTATION=true pnpm run acceptance:t1.5 -- --gateway-url "http://127.0.0.1:3180" --mock-base-url "http://127.0.0.1:18081" --mock-control-url "http://127.0.0.1:18081" --allow-config-mutation --case t1.5.doubao-agent-plan.doubao-agent-plan.video-tasks.plan-v3.video.txt2video.ta[REDACTED]
```

## Cases

| Case | Layer | Provider | Model | API | Status |
|---|---|---|---|---|---|
| t1.5.doubao-agent-plan.doubao-agent-plan.video-tasks.plan-v3.video.txt2video.ta[REDACTED] | T1.5 | doubao-agent-plan/t15-20261010043220-2666006-doubao-agent-plan | doubao-seedance-2.0@t15-20261010043220-2666006-doubao-agent-plan | video.txt2video | failed |

## Confirmed product defects

- **defect.aicc.t1.5.doubao-agent-plan.doubao-agent-plan.video-tasks.plan-v3.video.txt2video.ta[REDACTED]** (AICC, provider_protocol_failed): expected AICC maps the official Provider protocol fixture through its real Adapter; observed contract=doubao-agent-plan.video-tasks.plan-v3; scenario=success; captured_requests=3; Error: AICC artifact download failed with HTTP 503. Evidence: cases/t1.5.doubao-agent-plan.doubao-agent-plan.video-tasks.plan-v3.video.txt2video.ta[REDACTED].json

## Cleanup

Status: passed

- temporary Provider instances were removed and the Mock selection was reset
