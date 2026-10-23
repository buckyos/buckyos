# AICC E2E Acceptance Report

- Run: `t15-20261023060935-3736780`
- Commit: `53b4431e2d46ec731598ed0343d4a6596bee8d15`
- Capability baseline: `official-provider-protocols-2026-10-21.3`
- Real model calls: 0/0
- Planned maximum cost: $0.000000
- Known actual / unknown estimated exposure: $0.000000 / $0.000000
- Total exposure / budget: $0.000000 / $0.000000
- Unknown-cost calls: 0; budget exceeded: false
- Results: passed=2, failed=0, provider_restricted=0, skipped=0, not_applicable=0, review=0
- Manifest coverage: 2/2 (100.00%); passed=2, failed=0

## Cases

| Case | Layer | Provider | Model | API | Status |
|---|---|---|---|---|---|
| t1.5.sn-ai-provider.sn.openai-video-edits.v1.video.video2video.generated-artifact-id.same-provider | T1.5 | sn-ai-provider/t15-20261023060935-3736780-sn-ai-provider | wan-3.0@t15-20261023060935-3736780-sn-ai-provider | video.video2video | passed |
| t1.5.sn-ai-provider.sn.openai-video-extensions.v1.video.extend.generated-artifact-id.same-provider | T1.5 | sn-ai-provider/t15-20261023060935-3736780-sn-ai-provider | wan-3.0@t15-20261023060935-3736780-sn-ai-provider | video.extend | passed |

## Confirmed product defects

None.

## Cleanup

Status: passed

- temporary Provider instances were removed and the Mock selection was reset
