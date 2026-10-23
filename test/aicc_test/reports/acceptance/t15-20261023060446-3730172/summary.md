# AICC E2E Acceptance Report

- Run: `t15-20261023060446-3730172`
- Commit: `53b4431e2d46ec731598ed0343d4a6596bee8d15`
- Capability baseline: `official-provider-protocols-2026-10-21.3`
- Real model calls: 0/0
- Planned maximum cost: $0.000000
- Known actual / unknown estimated exposure: $0.000000 / $0.000000
- Total exposure / budget: $0.000000 / $0.000000
- Unknown-cost calls: 0; budget exceeded: false
- Results: passed=0, failed=2, provider_restricted=0, skipped=0, not_applicable=0, review=0
- Manifest coverage: 2/2 (100.00%); passed=0, failed=2

## Targeted retest

Run after fixing the reported defect; repeat `--case` to select additional cases:

```bash
AICC_T15_ALLOW_CONFIG_MUTATION=true pnpm run acceptance:t1.5 -- --gateway-url "http://127.0.0.1:3180" --mock-base-url "http://127.0.0.1:18081" --mock-control-url "http://127.0.0.1:18081" --allow-config-mutation --case t1.5.sn-ai-provider.sn.openai-video-edits.v1.video.video2video.generated-artifact-id.same-provider --case t1.5.sn-ai-provider.sn.openai-video-extensions.v1.video.extend.generated-artifact-id.same-provider
```

## Cases

| Case | Layer | Provider | Model | API | Status |
|---|---|---|---|---|---|
| t1.5.sn-ai-provider.sn.openai-video-edits.v1.video.video2video.generated-artifact-id.same-provider | T1.5 | sn-ai-provider/t15-20261023060446-3730172-sn-ai-provider | wan-3.0@t15-20261023060446-3730172-sn-ai-provider | video.video2video | failed |
| t1.5.sn-ai-provider.sn.openai-video-extensions.v1.video.extend.generated-artifact-id.same-provider | T1.5 | sn-ai-provider/t15-20261023060446-3730172-sn-ai-provider | wan-3.0@t15-20261023060446-3730172-sn-ai-provider | video.extend | failed |

## Confirmed product defects

- **defect.aicc.t1.5.sn-ai-provider.sn.openai-video-edits.v1.video.video2video.generated-artifact-id.same-provider** (AICC, provider_protocol_failed): expected AICC maps the official Provider protocol fixture through its real Adapter; observed contract=sn.openai-video-edits.v1; scenario=success; captured_requests=6; source_provider=sn-ai-provider; source_exact_model=wan-3.0@t15-20261023060446-3730172-sn-ai-provider; same provider/original_provider did not reuse generated content provider artifact id. Evidence: cases/t1.5.sn-ai-provider.sn.openai-video-edits.v1.video.video2video.generated-artifact-id.same-provider.json
- **defect.aicc.t1.5.sn-ai-provider.sn.openai-video-extensions.v1.video.extend.generated-artifact-id.same-provider** (AICC, provider_protocol_failed): expected AICC maps the official Provider protocol fixture through its real Adapter; observed contract=sn.openai-video-extensions.v1; scenario=success; captured_requests=3; source_provider=sn-ai-provider; source_exact_model=wan-3.0@t15-20261023060446-3730172-sn-ai-provider; artifact target mock received no request; same provider/original_provider did not reuse generated content provider artifact id; Error: AICC returned failed: {"error":{"code":"invalid_request","message":"SN video extension requires a source video ID or explicit continuation handle"},"event_ref":"/task_mgr/t-a08ad487426244a9b0cb0129c497104e","route_trace":{"attempts":[],"final_model":"wan-3.0@t15-20261023060446-3730172-sn-ai-provider"},"status":"failed","task_id":"t-a08ad487426244a9b0cb0129c497104e"}. Evidence: cases/t1.5.sn-ai-provider.sn.openai-video-extensions.v1.video.extend.generated-artifact-id.same-provider.json

## Cleanup

Status: passed

- temporary Provider instances were removed and the Mock selection was reset
