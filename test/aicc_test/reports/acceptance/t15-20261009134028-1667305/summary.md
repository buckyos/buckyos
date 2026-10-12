# AICC E2E Acceptance Report

- Run: `t15-20261009134028-1667305`
- Commit: `0c32470f0ebaa0f14a1a262bc2bcb0eea553d8fc`
- Capability baseline: `official-provider-protocols-2026-10-09.2`
- Real model calls: 0/0
- Planned maximum cost: $0.000000
- Known actual / unknown estimated exposure: $0.000000 / $0.000000
- Total exposure / budget: $0.000000 / $0.000000
- Unknown-cost calls: 0; budget exceeded: false
- Results: passed=0, failed=1, provider_restricted=0, skipped=0, not_applicable=0, review=0
- Manifest coverage: 0/12 (0.00%); passed=0, failed=0

## Targeted retest

Run after fixing the reported defect; repeat `--case` to select additional cases:

```bash
AICC_T15_ALLOW_CONFIG_MUTATION=true pnpm run acceptance:t1.5 -- --gateway-url "http://127.0.0.1:3180" --mock-base-url "http://127.0.0.1:18081" --mock-control-url "http://127.0.0.1:18081" --allow-config-mutation
```

## Cases

| Case | Layer | Provider | Model | API | Status |
|---|---|---|---|---|---|
| t1.5.runner | T1.5 | -/- | - | provider.add/models.list | failed |

## Unexecuted manifest cases

- t1.5.qwen.qwen.chat-completions.compatible-v1.llm.error.authentication
- t1.5.qwen.qwen.chat-completions.compatible-v1.llm.error.invalid_request
- t1.5.qwen.qwen.chat-completions.compatible-v1.llm.error.not_found
- t1.5.qwen.qwen.chat-completions.compatible-v1.llm.error.permission
- t1.5.qwen.qwen.chat-completions.compatible-v1.llm.error.rate_limit
- t1.5.qwen.qwen.chat-completions.compatible-v1.llm.error.server_error
- t1.5.qwen.qwen.chat-completions.compatible-v1.llm.response.malformed_response
- t1.5.qwen.qwen.chat-completions.compatible-v1.llm.response.missing_required_response_field
- t1.5.qwen.qwen.chat-completions.compatible-v1.llm.response.wrong_content_type
- t1.5.qwen.qwen.chat-completions.compatible-v1.llm.stream
- t1.5.qwen.qwen.chat-completions.compatible-v1.llm.stream-interrupted
- t1.5.qwen.qwen.chat-completions.compatible-v1.llm.success

## Confirmed product defects

- **defect.aicc.t1.5.runner** (AICC, provider_protocol_failed): expected AICC maps the official Provider protocol fixture through its real Adapter; observed captured_requests=0; startup: RPCError: RPC call failed: fetch failed. Evidence: cases/t1.5.runner.json

## Cleanup

Status: passed

- temporary Provider instances were removed and the Mock selection was reset
