# AICC E2E Acceptance Report

- Run: `t15-20261016163905-499397`
- Commit: `6d025991be1a359de271a110191c273a17c0fcfe`
- Capability baseline: `official-provider-protocols-2026-10-15.1`
- Real model calls: 0/0
- Planned maximum cost: $0.000000
- Known actual / unknown estimated exposure: $0.000000 / $0.000000
- Total exposure / budget: $0.000000 / $0.000000
- Unknown-cost calls: 0; budget exceeded: false
- Results: passed=0, failed=6, provider_restricted=0, skipped=0, not_applicable=0, review=0
- Manifest coverage: 6/6 (100.00%); passed=0, failed=6

## Targeted retest

Run after fixing the reported defect; repeat `--case` to select additional cases:

```bash
AICC_T15_ALLOW_CONFIG_MUTATION=true pnpm run acceptance:t1.5 -- --gateway-url "http://127.0.0.1:3180" --mock-base-url "http://127.0.0.1:18080" --mock-control-url "http://127.0.0.1:18080" --allow-config-mutation --case t1.5.google-gemini.gemini.interactions.v1beta.llm.model.gemini-4-argon.success --case t1.5.google-gemini.gemini.interactions.v1beta.vision.ocr.model.gemini-4-argon.success --case t1.5.google-gemini.gemini.interactions.v1beta.vision.caption.model.gemini-4-argon.success --case t1.5.google-gemini.gemini.interactions.v1beta.vision.detect.model.gemini-4-argon.success --case t1.5.google-gemini.gemini.interactions.v1beta.vision.segment.model.gemini-4-argon.success --case t1.5.google-gemini.gemini.interactions.v1beta.audio.asr.model.gemini-4-argon.success
```

## Cases

| Case | Layer | Provider | Model | API | Status |
|---|---|---|---|---|---|
| t1.5.google-gemini.gemini.interactions.v1beta.llm.model.gemini-4-argon.success | T1.5 | google-gemini/t15-20261016163905-499397-google-gemini | gemini-4-argon@t15-20261016163905-499397-google-gemini | llm | failed |
| t1.5.google-gemini.gemini.interactions.v1beta.vision.ocr.model.gemini-4-argon.success | T1.5 | google-gemini/t15-20261016163905-499397-google-gemini | gemini-4-argon@t15-20261016163905-499397-google-gemini | vision.ocr | failed |
| t1.5.google-gemini.gemini.interactions.v1beta.vision.caption.model.gemini-4-argon.success | T1.5 | google-gemini/t15-20261016163905-499397-google-gemini | gemini-4-argon@t15-20261016163905-499397-google-gemini | vision.caption | failed |
| t1.5.google-gemini.gemini.interactions.v1beta.vision.detect.model.gemini-4-argon.success | T1.5 | google-gemini/t15-20261016163905-499397-google-gemini | gemini-4-argon@t15-20261016163905-499397-google-gemini | vision.detect | failed |
| t1.5.google-gemini.gemini.interactions.v1beta.vision.segment.model.gemini-4-argon.success | T1.5 | google-gemini/t15-20261016163905-499397-google-gemini | gemini-4-argon@t15-20261016163905-499397-google-gemini | vision.segment | failed |
| t1.5.google-gemini.gemini.interactions.v1beta.audio.asr.model.gemini-4-argon.success | T1.5 | google-gemini/t15-20261016163905-499397-google-gemini | gemini-4-argon@t15-20261016163905-499397-google-gemini | audio.asr | failed |

## Confirmed product defects

- **defect.aicc.t1.5.google-gemini.gemini.interactions.v1beta.llm.model.gemini-4-argon.success** (AICC, provider_protocol_failed): expected AICC maps the official Provider protocol fixture through its real Adapter; observed contract=gemini.interactions.v1beta; scenario=success; captured_requests=1; RPCError: RPC call error: Failed due to reason: {"code":"internal_error","message":"set_running cannot persist a terminal execution state"}. Evidence: cases/t1.5.google-gemini.gemini.interactions.v1beta.llm.model.gemini-4-argon.success.json
- **defect.aicc.t1.5.google-gemini.gemini.interactions.v1beta.vision.ocr.model.gemini-4-argon.success** (AICC, provider_protocol_failed): expected AICC maps the official Provider protocol fixture through its real Adapter; observed contract=gemini.interactions.v1beta; scenario=success; captured_requests=1; RPCError: RPC call error: Failed due to reason: {"code":"internal_error","message":"set_running cannot persist a terminal execution state"}. Evidence: cases/t1.5.google-gemini.gemini.interactions.v1beta.vision.ocr.model.gemini-4-argon.success.json
- **defect.aicc.t1.5.google-gemini.gemini.interactions.v1beta.vision.caption.model.gemini-4-argon.success** (AICC, provider_protocol_failed): expected AICC maps the official Provider protocol fixture through its real Adapter; observed contract=gemini.interactions.v1beta; scenario=success; captured_requests=1; RPCError: RPC call error: Failed due to reason: {"code":"internal_error","message":"set_running cannot persist a terminal execution state"}. Evidence: cases/t1.5.google-gemini.gemini.interactions.v1beta.vision.caption.model.gemini-4-argon.success.json
- **defect.aicc.t1.5.google-gemini.gemini.interactions.v1beta.vision.detect.model.gemini-4-argon.success** (AICC, provider_protocol_failed): expected AICC maps the official Provider protocol fixture through its real Adapter; observed contract=gemini.interactions.v1beta; scenario=success; captured_requests=1; RPCError: RPC call error: Failed due to reason: {"code":"internal_error","message":"set_running cannot persist a terminal execution state"}. Evidence: cases/t1.5.google-gemini.gemini.interactions.v1beta.vision.detect.model.gemini-4-argon.success.json
- **defect.aicc.t1.5.google-gemini.gemini.interactions.v1beta.vision.segment.model.gemini-4-argon.success** (AICC, provider_protocol_failed): expected AICC maps the official Provider protocol fixture through its real Adapter; observed contract=gemini.interactions.v1beta; scenario=success; captured_requests=1; RPCError: RPC call error: Failed due to reason: {"code":"internal_error","message":"set_running cannot persist a terminal execution state"}. Evidence: cases/t1.5.google-gemini.gemini.interactions.v1beta.vision.segment.model.gemini-4-argon.success.json
- **defect.aicc.t1.5.google-gemini.gemini.interactions.v1beta.audio.asr.model.gemini-4-argon.success** (AICC, provider_protocol_failed): expected AICC maps the official Provider protocol fixture through its real Adapter; observed contract=gemini.interactions.v1beta; scenario=success; captured_requests=1; RPCError: RPC call error: Failed due to reason: {"code":"internal_error","message":"set_running cannot persist a terminal execution state"}. Evidence: cases/t1.5.google-gemini.gemini.interactions.v1beta.audio.asr.model.gemini-4-argon.success.json

## Cleanup

Status: passed

- temporary Provider instances were removed and the Mock selection was reset
