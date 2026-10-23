# AICC E2E Acceptance Report

- Run: `aicc-2026-10-23T08-46-59-378Z-65d86963`
- Commit: `53b4431e2d46ec731598ed0343d4a6596bee8d15`
- Capability baseline: `2026-10-23.1`
- Real model calls: 3/5
- Planned maximum cost: $0.050000
- Known actual / unknown estimated exposure: $0.000000 / $0.030000
- Total exposure / budget: $0.030000 / $30.000000
- Unknown-cost calls: 3; budget exceeded: false
- Results: passed=0, failed=4, provider_restricted=0, skipped=0, not_applicable=0, review=0

## Targeted retest

Run after fixing the reported defect; repeat `--case` to select additional cases:

```bash
pnpm run acceptance:gateway -- --config aicc_acceptance.sn-recheck-wan.local.toml --allow-credential-mutation --timeout-ms 900000 --provider sn-ai-provider --case t2.sn-ai-provider.sn-recheck-wan-t2.wan-3.0.video.extend --case t2.sn-ai-provider.sn-recheck-wan-t2.wan-3.0.video.img2video --case t2.sn-ai-provider.sn-recheck-wan-t2.wan-3.0.video.txt2video --case t2.sn-ai-provider.sn-recheck-wan-t2.wan-3.0.video.video2video
```

## Cases

| Case | Layer | Provider | Model | API | Status |
|---|---|---|---|---|---|
| t2.sn-ai-provider.sn-recheck-wan-t2.wan-3.0.video.extend | T2 | sn-ai-provider/sn-recheck-wan-t2 | wan-3.0@sn-recheck-wan-t2 | video.extend | failed |
| t2.sn-ai-provider.sn-recheck-wan-t2.wan-3.0.video.img2video | T2 | sn-ai-provider/sn-recheck-wan-t2 | wan-3.0@sn-recheck-wan-t2 | video.img2video | failed |
| t2.sn-ai-provider.sn-recheck-wan-t2.wan-3.0.video.txt2video | T2 | sn-ai-provider/sn-recheck-wan-t2 | wan-3.0@sn-recheck-wan-t2 | video.txt2video | failed |
| t2.sn-ai-provider.sn-recheck-wan-t2.wan-3.0.video.video2video | T2 | sn-ai-provider/sn-recheck-wan-t2 | wan-3.0@sn-recheck-wan-t2 | video.video2video | failed |

## Physical model coverage

| Provider | Inventory model | Physical model | Decision | Reason | Retained representative |
|---|---|---|---|---|---|
| sn-ai-provider/sn-recheck-wan-t2 | wan-3.0 | wan-3.0 | included | physical model | - |
| sn-ai-provider/sn-recheck-wan-t2 | wan-3.0 | wan-3.0 | included | physical model | - |

## Confirmed product defects

None.

## Cleanup

Status: passed

- removed 0 uploaded fixture object(s)
- removed 0 generated output object(s)
- temporary Provider credentials restored after testing (sn-ai-provider)
