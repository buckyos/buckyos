# AICC Remote Test Report

- Time: 2026-04-16T03:39:56.892Z
- Gateway: http://192.168.100.136
- kRPC Host: http://192.168.100.136
- AiccClient Host: http://192.168.100.136
- Model Alias: llm.plan.default
- Providers: sn-openai
- Overall: <span style="color:#16a34a;">&#x2714;</span> PASSED
- Summary: total=22, passed=20, partial=0, failed=0, skipped=2

## Case Results

| Status | Provider | Layer | Case | Duration(ms) | Detail |
|---|---|---|---|---:|---|
| <span style="color:#16a34a;">&#x2714;</span> | sn-openai | kRPC Direct | krpc_direct_01_complete_minimal_llm_success | 4319 | task_id=aicc-1776310710502-1, status=succeeded |
| <span style="color:#16a34a;">&#x2714;</span> | sn-openai | kRPC Direct | krpc_direct_02_complete_with_sys_seq_token_trace_success | 5503 | task_id=aicc-1776310714849-2 |
| <span style="color:#16a34a;">&#x2714;</span> | sn-openai | kRPC Direct | krpc_direct_03_complete_invalid_sys_shape_returns_bad_request | 1084 | invalid payload rejected as expected |
| <span style="color:#16a34a;">&#x2714;</span> | sn-openai | kRPC Direct | krpc_direct_04_cancel_cross_tenant_rejected | 5429 | cross-tenant cancel rejected with accepted=false |
| <span style="color:#16a34a;">&#x2714;</span> | sn-openai | kRPC Direct | krpc_direct_05_cancel_same_tenant_accepted_or_graceful_false | 4881 | task_id=aicc-1776310726830-4, accepted=false |
| <span style="color:#16a34a;">&#x2714;</span> | sn-openai | TS SDK | gateway_01_complete_minimal_llm_success | 4102 | task_id=aicc-1776310731743-5, status=succeeded |
| <span style="color:#16a34a;">&#x2714;</span> | sn-openai | TS SDK | gateway_02_complete_with_sys_seq_token_trace_success | 4414 | task_id=aicc-1776310735892-6 |
| <span style="color:#16a34a;">&#x2714;</span> | sn-openai | TS SDK | gateway_03_complete_without_token_with_trace_uses_null_placeholder | 2839 | task_id=aicc-1776310740218-7 |
| <span style="color:#16a34a;">&#x2714;</span> | sn-openai | TS SDK | gateway_04_complete_invalid_sys_shape_returns_bad_request | 1084 | invalid payload rejected as expected |
| <span style="color:#16a34a;">&#x2714;</span> | sn-openai | TS SDK | gateway_05_cancel_cross_tenant_rejected | 5196 | cross-tenant cancel rejected with accepted=false |
| <span style="color:#16a34a;">&#x2714;</span> | sn-openai | TS SDK | gateway_06_cancel_same_tenant_accepted_or_graceful_false | 5351 | task_id=aicc-1776310749366-9, accepted=false |
| <span style="color:#16a34a;">&#x2714;</span> | sn-openai | TS SDK | cfg_01_sys_config_get_aicc_settings_success | 1148 | key=services/aicc/test_settings/sn-openai/cfg_01_1776310751514 |
| <span style="color:#16a34a;">&#x2714;</span> | sn-openai | TS SDK | cfg_02_sys_config_set_full_value_effective | 1157 | key=services/aicc/test_settings/sn-openai/cfg_02_1776310752642 |
| <span style="color:#16a34a;">&#x2714;</span> | sn-openai | TS SDK | cfg_03_sys_config_set_by_json_path_partial_update_effective | 1157 | key=services/aicc/test_settings/sn-openai/cfg_03_1776310753799 |
| <span style="color:#16a34a;">&#x2714;</span> | sn-openai | TS SDK | cfg_04_sys_config_write_without_permission_rejected | 1069 | write rejected without permission |
| <span style="color:#16a34a;">&#x2714;</span> | sn-openai | TS SDK | cfg_05_sys_config_value_not_json_string_rejected | 1106 | plain string accepted by target |
| <span style="color:#16a34a;">&#x2714;</span> | sn-openai | AiccClient (Rust) | aicc_client_complete_success | 7234 | task_id=aicc-1776310762412-10 |
| <span style="color:#16a34a;">&#x2714;</span> | sn-openai | AiccClient (Rust) | aicc_client_cancel_same_tenant | 13056 | accepted=false |
| <span style="color:#16a34a;">&#x2714;</span> | sn-openai | TS SDK | ts_sdk_complete_success | 5221 | task_id=aicc-1776310780692-12, status=succeeded |
| <span style="color:#6b7280;">&#x23ED;</span> | sn-openai | TS SDK | workflow_remote_01_gateway_complex_scenario_protocol_mix | 1094 | openai-only workflow case |
| <span style="color:#16a34a;">&#x2714;</span> | sn-openai | TS SDK | workflow_remote_02_sn_openai_complex_scenario_protocol_mix | 13146 | workflow protocol mix ok |
| <span style="color:#6b7280;">&#x23ED;</span> | sn-openai | TS SDK | workflow_remote_03_gemini_complex_scenario_protocol_mix | 1084 | gemini-only workflow case |

## Strategy Mapping

- 覆盖链路：`/kapi/aicc`（complete/cancel） + `/kapi/system_config`（set/get） + `service.reload_settings`
- 调用层次：`kRPC Direct`、`AiccClient (Rust)`、`TS SDK`
- 用例顺序：按 provider 串行，每个用例执行前重置 AICC 配置
