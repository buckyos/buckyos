# AICC E2E Acceptance Report

- Run: `t15-20261023084435-3956049`
- Commit: `53b4431e2d46ec731598ed0343d4a6596bee8d15`
- Capability baseline: `official-provider-protocols-2026-10-23.2`
- Real model calls: 0/0
- Planned maximum cost: $0.000000
- Known actual / unknown estimated exposure: $0.000000 / $0.000000
- Total exposure / budget: $0.000000 / $0.000000
- Unknown-cost calls: 0; budget exceeded: false
- Results: passed=3, failed=1, provider_restricted=0, skipped=0, not_applicable=0, review=0
- Manifest coverage: 3/3 (100.00%); passed=3, failed=0

## Targeted retest

Run after fixing the reported defect; repeat `--case` to select additional cases:

```bash
AICC_T15_ALLOW_CONFIG_MUTATION=true pnpm run acceptance:t1.5 -- --gateway-url "http://127.0.0.1:3180" --mock-base-url "http://127.0.0.1:18081" --mock-control-url "http://127.0.0.1:18081" --allow-config-mutation
```

## Cases

| Case | Layer | Provider | Model | API | Status |
|---|---|---|---|---|---|
| t1.5.sn-ai-provider.sn.openai-videos.v1.video.txt2video.error.upstream_json_required | T1.5 | sn-ai-provider/t15-20261023084435-3956049-sn-ai-provider | wan-3.0@t15-20261023084435-3956049-sn-ai-provider | video.txt2video | passed |
| t1.5.sn-ai-provider.sn.openai-video-edits.v1.video.video2video.error.upstream_json_required | T1.5 | sn-ai-provider/t15-20261023084435-3956049-sn-ai-provider | wan-3.0@t15-20261023084435-3956049-sn-ai-provider | video.video2video | passed |
| t1.5.sn-ai-provider.sn.openai-video-extensions.v1.video.extend.error.upstream_json_required | T1.5 | sn-ai-provider/t15-20261023084435-3956049-sn-ai-provider | wan-3.0@t15-20261023084435-3956049-sn-ai-provider | video.extend | passed |
| t1.5.runner | T1.5 | -/- | - | provider.add/models.list | failed |

## Confirmed product defects

- **defect.aicc.t1.5.runner** (AICC, provider_protocol_failed): expected AICC maps the official Provider protocol fixture through its real Adapter; observed captured_requests=0; custom:google-gemini:setup: Error: unknown or out-of-scope --case: t1.5.sn-ai-provider.sn.openai-videos.v1.video.img2video.error.upstream_json_required. Evidence: cases/t1.5.runner.json

## Cleanup

Status: passed

- temporary Provider instances were removed and the Mock selection was reset
