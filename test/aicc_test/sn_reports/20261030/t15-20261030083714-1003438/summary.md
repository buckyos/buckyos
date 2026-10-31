# AICC E2E Acceptance Report

- Run: `t15-20261030083714-1003438`
- Commit: `d40291a9a9085d1cec7ccb37d519d9115baf534c`
- Capability baseline: `2026-10-30.1`
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

- **defect.aicc.t1.5.runner** (AICC, provider_protocol_failed): expected AICC maps the official Provider protocol fixture through its real Adapter; observed captured_requests=0; provider:openrouter:add: Error: openrouter add failed: RPCError: RPC call error: Failed due to reason: provider validation failed; settings were not changed; validation={"auth_valid":true,"balance_available":false,"base_url_reachable":true,"error_details":[{"kind":"models","message":"provider validation failed at Inventory: inventory build failed: provider instance \"t15-20261030083714-1003438-openrouter\" model \"moonshotai/kimi-k3.1-pro\": inventory build failed: adapter \"openrouter-responses\" has multiple default operations [\"chat.completions.create\", \"responses.create\"] for api_type `llm`"}],"errors":["provider validation failed at Inventory: inventory build failed: provider instance \"t15-20261030083714-1003438-openrouter\" model \"moonshotai/kimi-k3.1-pro\": inventory build failed: adapter \"openrouter-responses\" has multiple default operations [\"chat.completions.create\", \"responses.create\"] for api_type `llm`"],"filtered_capabilities":[],"models_discovered":[],"models_recognized":[],"resolved_protocol_adapter_id":"openrouter-responses","routing_candidates":{},"unavailable_presets":[],"unmatched_models":[],"warnings":[]}. Evidence: cases/t1.5.runner.json

## Cleanup

Status: passed

- temporary Provider instances were removed and the Mock selection was reset
