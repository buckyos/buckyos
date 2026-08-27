# AICC acceptance tests

T2 的模型库存基准来自 Runner 直接调用 `provider_capability_baseline.json` 配置的 Provider 官方目录接口。AICC `models.list` 仅作为被测输出参与双向 diff；官方目录抓取失败或为空时不会回退到 AICC inventory。目录请求默认复用本地 TOML 的 Provider 凭据，也可用 `official_catalog_credentials.<provider>.api_token`（或 `AICC_<PROVIDER>_CATALOG_TOKEN`）单独覆盖，凭据不会写入报告；SN 的目录凭据是 SN SSO session token，必须使用独立覆盖。`provider_credentials.apply_to_aicc_settings` 只控制 Provider API token 是否同时临时注入 AICC，不影响目录认证。

这组测试对应 `doc/aicc/aicc_e2e_test_requirements.md`：

- `preflight.ts`：严格校验 23 个 canonical API、静态 case、内置 Provider 基线和 fixture 完整性。
- `mock_provider.ts`：位于 Provider HTTP 边界的确定性 Mock，支持错误、流式、异步和调用记录。
- `run_gateway.ts`：经 Zone Gateway 登录真实 AICC；默认只生成 T2 计划，只有显式允许时才调用真实 Provider。
- `provider_capability_baseline.json`：按 Provider 参数化的版本化能力证据基线。

```bash
cd test/aicc_test
pnpm run acceptance:preflight
pnpm run acceptance:self-test
cp aicc_acceptance.example.toml aicc_acceptance.local.toml
pnpm run acceptance:t1 -- --config aicc_acceptance.local.toml --allow-config-mutation
pnpm run acceptance:gateway -- --config aicc_acceptance.local.toml
```

真实调用必须通过 `allow_real_model_calls = true` 或命令行 `--allow-real-model-calls` 显式开启，并受调用数、成本和 timeout 上限约束。需要安全审计计划时，`--no-real-model-calls` 可强制覆盖 TOML 中的开启值，仍读取真实 inventory、生成完整 skipped/N/A/基线差异与零成本报告。报告会把能力基线不一致、路由/资源/安全断言失败和成功调用后的 usage/trace 归因失败写入结构化 `product_defects`，记录预期、实际结果和证据路径；测试不会修改 AICC/Jarvis 实现。

Runner 会并行执行不同 case/session。`global_concurrency` 控制整轮并发，`provider_concurrency` 和 `provider_min_interval_ms` 是默认 Provider 限制；可通过 `[limits.<provider_driver>]` 单独覆盖。每次 retry 也重新经过同一 Provider 的并发和请求间隔门禁，不会绕过限流。

每轮会生成独立的 `finance.json`、`finance.csv` 和 `finance.md`。账本按 case/attempt 记录 token、request unit、调用前估算和 AICC 返回的 USD cost，并按 Provider、instance、精确模型、API、case 汇总。Provider 未返回费用时会计入“未知费用的估算敞口”，不会记成零成本；并发调用在发出前会先预留预算。

开启真实模型调用后，runner 会先输出 case、最大调用次数和预计成本，再进入 10 秒确认倒计时；输入 `c` 取消，直接回车立即开始，超时视为确认。已经获得人工授权的自动化运行可传 `--yes` 跳过倒计时。`--yes` 只是 runner 的非交互开关，不代表 CodeAgent 已获得运行 T2/T3 的人工授权。

T1 会临时写入带 `run_id` 的 Mock Provider instance，并在 `finally` 中原样恢复整个 `services/aicc/settings`。为避免误改环境，配置文件的 `mock.allow_config_mutation` 与命令行 `--allow-config-mutation` 必须同时开启。AICC 与 Mock 不在同一主机时，`mock.base_url` 必须填写 AICC 服务进程可访问的地址，不能使用 runner 自己的 loopback。

第二租户隔离用例通过 `[auth].other_tenant_session_token` 或 `BUCKYOS_TEST_OTHER_TENANT_SESSION_TOKEN` 参数化，覆盖 task 查询/取消、usage、msg-center 消息、Named Object 和管理方法 RBAC。未配置时这些 case 保留在 manifest 和覆盖报告中并明确记为 `skipped`，不会伪造同租户结果或阻断其他 T1 用例。

T2 会为 OpenAI、Claude、Google Gemini、Fal、MiniMax、OpenRouter 和 SN 生成完整参数化矩阵；是否实际执行由 `--provider`、凭据和真实调用开关共同决定。可用报告中的 `targeted_retest_command`，或重复传入 `--case <case_id>`，只重跑失败单元。没有账号的 MiniMax 以及明确禁止执行的 OpenRouter/SN 仍保留基线和用例，但不应开启真实调用。

普通 `llm.chat` 用例显式关闭 AICC 默认附加的 `web_search`，避免把基础聊天错误地限定为必须支持联网搜索；联网搜索作为独立 capability 分支验证。Provider 凭据临时写入后，runner 会等待 system-config 与 AICC runtime settings 收敛，再验证 settings 字节恢复、运行时 inventory、Named Data 和消息资源清理。
