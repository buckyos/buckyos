# AICC E2E Acceptance Report

- Run: `t15-20261030084240-1009446`
- Commit: `d40291a9a9085d1cec7ccb37d519d9115baf534c`
- Capability baseline: `2026-10-30.1`
- Real model calls: 0/0
- Planned maximum cost: $0.000000
- Known actual / unknown estimated exposure: $0.000000 / $0.000000
- Total exposure / budget: $0.000000 / $0.000000
- Unknown-cost calls: 0; budget exceeded: false
- Results: passed=3, failed=0, provider_restricted=0, skipped=0, not_applicable=0, review=0
- Manifest coverage: 3/3 (100.00%); passed=3, failed=0

## Cases

| Case | Layer | Provider | Model | API | Status |
|---|---|---|---|---|---|
| t1.5.qwen.qwen.dashscope.wan3.video_synthesis.v1.video.generate.success | T1.5 | qwen/t15-20261030084240-1009446-qwen | wan3.0-video@t15-20261030084240-1009446-qwen | video.generate | passed |
| t1.5.qwen.qwen.dashscope.wan3.video_synthesis.v1.video.generate.task-result-artifact | T1.5 | qwen/t15-20261030084240-1009446-qwen | wan3.0-video@t15-20261030084240-1009446-qwen | video.generate | passed |
| t1.5.qwen.qwen.dashscope.wan3.video_synthesis.v1.video.generate.async | T1.5 | qwen/t15-20261030084240-1009446-qwen | wan3.0-video@t15-20261030084240-1009446-qwen | video.generate | passed |

## Confirmed product defects

None.

## Cleanup

Status: passed

- temporary Provider instances were removed and the Mock selection was reset
