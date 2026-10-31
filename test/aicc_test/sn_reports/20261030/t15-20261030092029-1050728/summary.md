# AICC E2E Acceptance Report

- Run: `t15-20261030092029-1050728`
- Commit: `d40291a9a9085d1cec7ccb37d519d9115baf534c`
- Capability baseline: `2026-10-30.1`
- Real model calls: 0/0
- Planned maximum cost: $0.000000
- Known actual / unknown estimated exposure: $0.000000 / $0.000000
- Total exposure / budget: $0.000000 / $0.000000
- Unknown-cost calls: 0; budget exceeded: false
- Results: passed=16, failed=0, provider_restricted=0, skipped=0, not_applicable=0, review=0
- Manifest coverage: 16/16 (100.00%); passed=16, failed=0

## Cases

| Case | Layer | Provider | Model | API | Status |
|---|---|---|---|---|---|
| t1.5.sn-ai-provider.sn.openai-videos.v1.video.generate.success | T1.5 | sn-ai-provider/t15-20261030092029-1050728-sn-ai-provider | wan-3.0@t15-20261030092029-1050728-sn-ai-provider | video.generate | passed |
| t1.5.sn-ai-provider.sn.openai-videos.v1.video.generate.task-result-artifact | T1.5 | sn-ai-provider/t15-20261030092029-1050728-sn-ai-provider | wan-3.0@t15-20261030092029-1050728-sn-ai-provider | video.generate | passed |
| t1.5.sn-ai-provider.sn.openai-videos.v1.video.generate.video-keyframe | T1.5 | sn-ai-provider/t15-20261030092029-1050728-sn-ai-provider | wan-3.0@t15-20261030092029-1050728-sn-ai-provider | video.generate | passed |
| t1.5.sn-ai-provider.sn.openai-videos.v1.video.generate.async | T1.5 | sn-ai-provider/t15-20261030092029-1050728-sn-ai-provider | wan-3.0@t15-20261030092029-1050728-sn-ai-provider | video.generate | passed |
| t1.5.sn-ai-provider.sn.openai-videos.v1.video.generate.async-failed | T1.5 | sn-ai-provider/t15-20261030092029-1050728-sn-ai-provider | wan-3.0@t15-20261030092029-1050728-sn-ai-provider | video.generate | passed |
| t1.5.sn-ai-provider.sn.openai-videos.v1.video.generate.async_poll_timeout | T1.5 | sn-ai-provider/t15-20261030092029-1050728-sn-ai-provider | wan-3.0@t15-20261030092029-1050728-sn-ai-provider | video.generate | passed |
| t1.5.sn-ai-provider.sn.openai-videos.v1.video.generate.async_artifact_unavailable | T1.5 | sn-ai-provider/t15-20261030092029-1050728-sn-ai-provider | wan-3.0@t15-20261030092029-1050728-sn-ai-provider | video.generate | passed |
| t1.5.sn-ai-provider.sn.openai-videos.v1.video.generate.error.invalid_argument | T1.5 | sn-ai-provider/t15-20261030092029-1050728-sn-ai-provider | wan-3.0@t15-20261030092029-1050728-sn-ai-provider | video.generate | passed |
| t1.5.sn-ai-provider.sn.openai-videos.v1.video.generate.error.authentication | T1.5 | sn-ai-provider/t15-20261030092029-1050728-sn-ai-provider | wan-3.0@t15-20261030092029-1050728-sn-ai-provider | video.generate | passed |
| t1.5.sn-ai-provider.sn.openai-videos.v1.video.generate.error.insufficient_balance | T1.5 | sn-ai-provider/t15-20261030092029-1050728-sn-ai-provider | wan-3.0@t15-20261030092029-1050728-sn-ai-provider | video.generate | passed |
| t1.5.sn-ai-provider.sn.openai-videos.v1.video.generate.error.unavailable | T1.5 | sn-ai-provider/t15-20261030092029-1050728-sn-ai-provider | wan-3.0@t15-20261030092029-1050728-sn-ai-provider | video.generate | passed |
| t1.5.sn-ai-provider.sn.openai-videos.v1.video.generate.error.timeout | T1.5 | sn-ai-provider/t15-20261030092029-1050728-sn-ai-provider | wan-3.0@t15-20261030092029-1050728-sn-ai-provider | video.generate | passed |
| t1.5.sn-ai-provider.sn.openai-videos.v1.video.generate.error.upstream_json_required | T1.5 | sn-ai-provider/t15-20261030092029-1050728-sn-ai-provider | wan-3.0@t15-20261030092029-1050728-sn-ai-provider | video.generate | passed |
| t1.5.sn-ai-provider.sn.openai-videos.v1.video.generate.response.malformed_response | T1.5 | sn-ai-provider/t15-20261030092029-1050728-sn-ai-provider | wan-3.0@t15-20261030092029-1050728-sn-ai-provider | video.generate | passed |
| t1.5.sn-ai-provider.sn.openai-videos.v1.video.generate.response.wrong_content_type | T1.5 | sn-ai-provider/t15-20261030092029-1050728-sn-ai-provider | wan-3.0@t15-20261030092029-1050728-sn-ai-provider | video.generate | passed |
| t1.5.sn-ai-provider.sn.openai-videos.v1.video.generate.response.missing_required_response_field | T1.5 | sn-ai-provider/t15-20261030092029-1050728-sn-ai-provider | wan-3.0@t15-20261030092029-1050728-sn-ai-provider | video.generate | passed |

## Confirmed product defects

None.

## Cleanup

Status: passed

- temporary Provider instances were removed and the Mock selection was reset
