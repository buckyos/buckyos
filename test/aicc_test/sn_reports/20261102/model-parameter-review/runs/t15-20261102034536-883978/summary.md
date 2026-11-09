# AICC E2E Acceptance Report

- Run: `t15-20261102034536-883978`
- Commit: `7c9472fda19935f8070a63943361063b8ad2e376`
- Capability baseline: `2026-11-02.1`
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

- t1.5.openrouter.openrouter.responses.v1.llm.model.moonshotai-kimi-k3.1-pro.success
- t1.5.openrouter.openrouter.responses.v1.llm.model.x-ai-grok-4.7.success
- t1.5.openrouter.openrouter.responses.v1.llm.model.z-ai-glm-5.3-flash.success
- t1.5.openrouter.openrouter.responses.v1.llm.model.z-ai-glm-5.3-flashx.success
- t1.5.openrouter.openrouter.responses.v1.vision.caption.model.moonshotai-kimi-k3.1-pro.success
- t1.5.openrouter.openrouter.responses.v1.vision.caption.model.x-ai-grok-4.7.success
- t1.5.openrouter.openrouter.responses.v1.vision.caption.model.z-ai-glm-5.3-flash.success
- t1.5.openrouter.openrouter.responses.v1.vision.caption.model.z-ai-glm-5.3-flashx.success
- t1.5.openrouter.openrouter.responses.v1.vision.ocr.model.moonshotai-kimi-k3.1-pro.success
- t1.5.openrouter.openrouter.responses.v1.vision.ocr.model.x-ai-grok-4.7.success
- t1.5.openrouter.openrouter.responses.v1.vision.ocr.model.z-ai-glm-5.3-flash.success
- t1.5.openrouter.openrouter.responses.v1.vision.ocr.model.z-ai-glm-5.3-flashx.success

## Confirmed product defects

- **defect.aicc.t1.5.runner** (AICC, provider_protocol_failed): expected AICC maps the official Provider protocol fixture through its real Adapter; observed captured_requests=0; provider:openrouter:inventory: Error: openrouter is missing official metadata variant openai/gpt-5.6:reasoning-none. Evidence: cases/t1.5.runner.json

## Cleanup

Status: passed

- temporary Provider instances were removed and the Mock selection was reset
