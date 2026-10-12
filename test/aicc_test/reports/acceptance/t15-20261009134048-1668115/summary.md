# AICC E2E Acceptance Report

- Run: `t15-20261009134048-1668115`
- Commit: `0c32470f0ebaa0f14a1a262bc2bcb0eea553d8fc`
- Capability baseline: `official-provider-protocols-2026-10-09.2`
- Real model calls: 0/0
- Planned maximum cost: $0.000000
- Known actual / unknown estimated exposure: $0.000000 / $0.000000
- Total exposure / budget: $0.000000 / $0.000000
- Unknown-cost calls: 0; budget exceeded: false
- Results: passed=12, failed=0, provider_restricted=0, skipped=0, not_applicable=0, review=0
- Manifest coverage: 12/12 (100.00%); passed=12, failed=0

## Cases

| Case | Layer | Provider | Model | API | Status |
|---|---|---|---|---|---|
| t1.5.qwen.qwen.chat-completions.compatible-v1.llm.success | T1.5 | qwen/t15-20261009134048-1668115-qwen | deepseek-v4-flash@t15-20261009134048-1668115-qwen | llm | passed |
| t1.5.qwen.qwen.chat-completions.compatible-v1.llm.stream | T1.5 | qwen/t15-20261009134048-1668115-qwen | deepseek-v4-flash@t15-20261009134048-1668115-qwen | llm | passed |
| t1.5.qwen.qwen.chat-completions.compatible-v1.llm.stream-interrupted | T1.5 | qwen/t15-20261009134048-1668115-qwen | deepseek-v4-flash@t15-20261009134048-1668115-qwen | llm | passed |
| t1.5.qwen.qwen.chat-completions.compatible-v1.llm.error.invalid_request | T1.5 | qwen/t15-20261009134048-1668115-qwen | deepseek-v4-flash@t15-20261009134048-1668115-qwen | llm | passed |
| t1.5.qwen.qwen.chat-completions.compatible-v1.llm.error.authentication | T1.5 | qwen/t15-20261009134048-1668115-qwen | deepseek-v4-flash@t15-20261009134048-1668115-qwen | llm | passed |
| t1.5.qwen.qwen.chat-completions.compatible-v1.llm.error.permission | T1.5 | qwen/t15-20261009134048-1668115-qwen | deepseek-v4-flash@t15-20261009134048-1668115-qwen | llm | passed |
| t1.5.qwen.qwen.chat-completions.compatible-v1.llm.error.not_found | T1.5 | qwen/t15-20261009134048-1668115-qwen | deepseek-v4-flash@t15-20261009134048-1668115-qwen | llm | passed |
| t1.5.qwen.qwen.chat-completions.compatible-v1.llm.error.rate_limit | T1.5 | qwen/t15-20261009134048-1668115-qwen-isolation-1 | deepseek-v4-flash@t15-20261009134048-1668115-qwen-isolation-1 | llm | passed |
| t1.5.qwen.qwen.chat-completions.compatible-v1.llm.error.server_error | T1.5 | qwen/t15-20261009134048-1668115-qwen-isolation-1 | deepseek-v4-flash@t15-20261009134048-1668115-qwen-isolation-1 | llm | passed |
| t1.5.qwen.qwen.chat-completions.compatible-v1.llm.response.malformed_response | T1.5 | qwen/t15-20261009134048-1668115-qwen-isolation-1 | deepseek-v4-flash@t15-20261009134048-1668115-qwen-isolation-1 | llm | passed |
| t1.5.qwen.qwen.chat-completions.compatible-v1.llm.response.wrong_content_type | T1.5 | qwen/t15-20261009134048-1668115-qwen-isolation-1 | deepseek-v4-flash@t15-20261009134048-1668115-qwen-isolation-1 | llm | passed |
| t1.5.qwen.qwen.chat-completions.compatible-v1.llm.response.missing_required_response_field | T1.5 | qwen/t15-20261009134048-1668115-qwen-isolation-1 | deepseek-v4-flash@t15-20261009134048-1668115-qwen-isolation-1 | llm | passed |

## Confirmed product defects

None.

## Cleanup

Status: passed

- temporary Provider instances were removed and the Mock selection was reset
