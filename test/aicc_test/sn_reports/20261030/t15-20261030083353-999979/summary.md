# AICC E2E Acceptance Report

- Run: `t15-20261030083353-999979`
- Commit: `d40291a9a9085d1cec7ccb37d519d9115baf534c`
- Capability baseline: `2026-10-30.1`
- Real model calls: 0/0
- Planned maximum cost: $0.000000
- Known actual / unknown estimated exposure: $0.000000 / $0.000000
- Total exposure / budget: $0.000000 / $0.000000
- Unknown-cost calls: 0; budget exceeded: false
- Results: passed=0, failed=1, provider_restricted=0, skipped=0, not_applicable=0, review=0
- Manifest coverage: 0/6 (0.00%); passed=0, failed=0

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

- t1.5.glm.glm.chat-completions.v4.llm.model.glm-5.3-flash.success
- t1.5.glm.glm.chat-completions.v4.llm.model.glm-5.3-flashx.success
- t1.5.glm.glm.chat-completions.v4.vision.caption.model.glm-5.3-flash.success
- t1.5.glm.glm.chat-completions.v4.vision.caption.model.glm-5.3-flashx.success
- t1.5.glm.glm.chat-completions.v4.vision.ocr.model.glm-5.3-flash.success
- t1.5.glm.glm.chat-completions.v4.vision.ocr.model.glm-5.3-flashx.success

## Confirmed product defects

- **defect.aicc.t1.5.runner** (AICC, provider_protocol_failed): expected AICC maps the official Provider protocol fixture through its real Adapter; observed captured_requests=0; provider:glm:inventory: Error: glm is missing official metadata variant glm-5.3-flashx:reasoning-thinking. Evidence: cases/t1.5.runner.json

## Cleanup

Status: passed

- temporary Provider instances were removed and the Mock selection was reset
