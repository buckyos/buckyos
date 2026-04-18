# AICC Remote Test Report

- Time: 2026-04-16T04:44:06.550Z
- Gateway: http://192.168.100.164
- kRPC Host: http://192.168.100.164
- AiccClient Host: http://192.168.100.164
- Model Alias: llm.plan.default
- Providers: sn-openai, openai, gemini, claude
- Mode: protocol_mix_only_no_system_config
- Overall: <span style="color:#dc2626;">&#x2717;</span> FAILED
- Summary: total=12, passed=2, partial=0, failed=1, skipped=9

## Case Results

| Status | Provider | Layer | Case | Duration(ms) | Detail |
|---|---|---|---|---:|---|
| <span style="color:#6b7280;">&#x23ED;</span> | sn-openai | TS SDK | workflow_remote_01_gateway_complex_scenario_protocol_mix | 0 | openai-only workflow case |
| <span style="color:#dc2626;">&#x2717;</span> | sn-openai | TS SDK | workflow_remote_02_sn_openai_complex_scenario_protocol_mix | 10953 | SyntaxError: Unexpected end of JSON input |
| <span style="color:#6b7280;">&#x23ED;</span> | sn-openai | TS SDK | workflow_remote_03_gemini_complex_scenario_protocol_mix | 0 | gemini-only workflow case |
| <span style="color:#16a34a;">&#x2714;</span> | openai | TS SDK | workflow_remote_01_gateway_complex_scenario_protocol_mix | 12468 | workflow protocol mix ok |
| <span style="color:#6b7280;">&#x23ED;</span> | openai | TS SDK | workflow_remote_02_sn_openai_complex_scenario_protocol_mix | 0 | sn-openai-only workflow case |
| <span style="color:#6b7280;">&#x23ED;</span> | openai | TS SDK | workflow_remote_03_gemini_complex_scenario_protocol_mix | 0 | gemini-only workflow case |
| <span style="color:#6b7280;">&#x23ED;</span> | gemini | TS SDK | workflow_remote_01_gateway_complex_scenario_protocol_mix | 0 | openai-only workflow case |
| <span style="color:#6b7280;">&#x23ED;</span> | gemini | TS SDK | workflow_remote_02_sn_openai_complex_scenario_protocol_mix | 0 | sn-openai-only workflow case |
| <span style="color:#16a34a;">&#x2714;</span> | gemini | TS SDK | workflow_remote_03_gemini_complex_scenario_protocol_mix | 14802 | workflow protocol mix ok |
| <span style="color:#6b7280;">&#x23ED;</span> | claude | TS SDK | workflow_remote_01_gateway_complex_scenario_protocol_mix | 0 | missing claude api key |
| <span style="color:#6b7280;">&#x23ED;</span> | claude | TS SDK | workflow_remote_02_sn_openai_complex_scenario_protocol_mix | 0 | missing claude api key |
| <span style="color:#6b7280;">&#x23ED;</span> | claude | TS SDK | workflow_remote_03_gemini_complex_scenario_protocol_mix | 0 | missing claude api key |

## Strategy Mapping

- 覆盖链路：`/kapi/aicc`（complete/cancel） + `/kapi/system_config`（set/get） + `service.reload_settings`
- 调用层次：`kRPC Direct`、`AiccClient (Rust)`、`TS SDK`
- 用例顺序：按 provider 串行，每个用例执行前重置 AICC 配置
