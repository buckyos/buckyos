# AICC acceptance tests

本目录实现 T1/T1.5/T2 的验收基础设施。分层边界如下：

- T1 只验证路由、调度、fallback、任务与系统行为，使用通用确定性 Mock，不把 Provider 专属 wire 字段作为通过证据。
- T1.5 经 Zone Gateway 调用真实 AICC typed method 和 Protocol Adapter，再连接按 Provider 官方资料独立实现的高保真 Mock；它验证请求、正常响应、streaming、异步状态和官方错误，不验证推理语义。
- T2 对每个获准 Provider instance 的全部 active 基础物理模型生成 `ProviderInstance × model × API-Type` 矩阵，每个单元只运行一个最小正确性请求。metadata variant、输入排列、资源表示、文档格式和错误注入不生成 T2 单元。

T2 的模型库存基准来自 Runner 直接调用 `provider_capability_baseline.json` 配置的 Provider 官方目录接口。Runner 先抓取官方目录，再确定本轮每个 Provider instance，对每个选中 instance 调用 `provider.refresh_models` 直到首次成功，之后读取刷新后的 AICC `models.list` 参与双向 diff。不同 Provider 并行刷新；单个 Provider 串行有限重试并指数退避，间隔不小于该 Provider 的 `min_interval_ms`，首次成功后本轮不再主动刷新。最大尝试次数和基础间隔由 `runner.inventory_refresh_max_attempts`、`runner.inventory_refresh_retry_delay_ms` 配置；最终失败会在执行 case 前终止。官方目录抓取失败或为空时不会回退到 AICC inventory。目录请求默认复用本地 TOML 的 Provider 凭据，也可用 `official_catalog_credentials.<provider>.api_token`（或 `AICC_<PROVIDER>_CATALOG_TOKEN`）单独覆盖，凭据不会写入报告；SN 的目录凭据是 SN SSO session token，必须使用独立覆盖。`provider_credentials.apply_to_aicc_settings` 只控制 Provider API token 是否同时临时注入 AICC，不影响目录认证。

这组测试对应 `doc/aicc/aicc_e2e_test_requirements.md`：

- `preflight.ts`：从规范文档校验 23 个 canonical API，并检查静态 T1 case、T1.5 官方协议契约、Provider 能力基线和 fixture 完整性；不读取 AICC 实现代码或实现 metadata。
- `mock_provider.ts`：T1 使用的通用确定性 Mock。
- `mock_provider_contract.ts`：T1 Mock 的版本化 scenario 与管理接口契约；Mock 实现直接消费该契约，preflight 检查其完整性。
- `provider_protocol_contracts.json`：T1.5 独立协议契约、官方证据 revision、测试用 Provider Profile/模型映射、请求字段类型、正常响应、异步 lifecycle 和 Provider 专属错误 fixture。Runner 不按模型名或厂商名选择协议分支。
- `t15_mock_provider.ts`：T1.5 高保真 Mock；按所选官方契约严格校验 method、path、认证、content type、必需字段、字段类型和未知字段，并独立记录 submit、poll、result、cancel wire，然后返回对应 Provider 的正常、stream、异步或错误响应。
- `run_t15_gateway.ts`：临时注册目标 Provider instance，用精确模型固定 adapter，经 Gateway 执行 T1.5 并审计 Mock capture；每个运行时可独立调用的 variant 形成独立协议单元。
- `cloud_update_fixture_service.ts`：启动独立 `cyfs-gateway` `cyfs-dir` NDN 服务，以 process chain 将协议路径绑定到 Named Object，并为 T1/T1.5 发布 index、manifest、catalog 与 tombstone。
- `run_gateway.ts`：经 Zone Gateway 登录真实 AICC；默认只生成 T2 计划，只有显式允许时才调用真实 Provider。
- `provider_capability_baseline.json`：按 Provider 参数化的版本化能力证据基线。

WP-01 至 WP-17 的模块单测入口与验收职责保持如下映射；T1/T1.5 只覆盖跨模块集成，不替代这些入口：

| 工作包 | 模块单测入口 | 覆盖边界 |
|---|---|---|
| WP-01 | `cd src && cargo test -p buckyos-api`；`cargo test -p buckyos-api --test aicc_client_test` | Rust canonical DTO、Client/Handler dispatch、序列化与稳定错误映射 |
| WP-01TS | WebSDK 上游的 canonical AICC 测试；本仓通过 `src/apps/sys_test/package.json` 与本目录 lockfile 固定 `092009c...` | TypeScript method/DTO/export 与 Rust contract 对齐；本仓不复制 WebSDK 源码 |
| WP-02 | `cd src && cargo test -p aicc matching` | MatchRule 编译、字段约束、组合与脱敏 trace |
| WP-03 | `cd src && cargo test -p aicc catalog` | catalog 加载、编译、revision、冲突和 snapshot |
| WP-04 | `cd src && cargo test -p aicc model` | model registry、logical directory、variant 与能力交集 |
| WP-05 | `cd src && cargo test -p aicc protocol` | adapter registry、transport、codec limit 与错误边界 |
| WP-06 | `cd src && cargo test -p aicc protocol` | OpenAI Responses、Claude Messages、Gemini Interactions、OpenAI Chat codec |
| WP-07 | `cd src && cargo test -p aicc provider` | Provider core、discovery、inventory、LKGS 与 refresh/stop |
| WP-08 | `cd src && cargo test -p aicc provider::builtin` | 11 家内置 Provider、SN、dialect 和 metadata 装配 |
| WP-09 | `cd src && cargo test -p aicc admission`；`cargo test -p aicc routing` | quota、budget、privacy、trust 与 fail-closed policy |
| WP-10 | `cd src && cargo test -p aicc routing`；`cargo test -p aicc scheduler` | deterministic routing、并发/间隔、公平性、fallback 与 trace |
| WP-11 | `cd src && cargo test -p aicc call` | operation 选择、variant lowering、set/remove 和资源要求 |
| WP-12 | `cd src && cargo test -p aicc execution` | immediate/stream/native task、cancel、幂等、恢复、竞态与 usage completion |
| WP-13 | `cd src && cargo test -p aicc resource` | ResourceRef、权限、MIME、大小、压缩包安全、上传与 artifact |
| WP-14 | `cd src && cargo test -p aicc storage`；`cargo test -p aicc usage` | 原子存储、去重、retention、usage/finance、trace 关联与脱敏 |
| WP-15 | `cd src && cargo test -p aicc runtime`；`cargo test -p aicc metadata` | RuntimeSnapshot 原子发布、settings/metadata seq 收敛和失败保留 |
| WP-16 | `cd src && cargo test -p aicc service`；`cargo test -p buckyos-api` | service 装配、管理 API、RBAC、reload/CAS 与 Client 映射 |
| WP-17A | `cd src/frame/desktop && pnpm run check && pnpm run test:e2e` | AI Center DataModel、Provider Wizard、财务展示与交互 |
| WP-17B | `cd src && cargo test -p workflow` | Workflow typed AICC adapter、任务进度与错误传播 |
| WP-17C | `cd src && cargo test -p llm_context -p agent_tool -p opendan`；`cd src/tools/buckyos-agent && deno test --allow-read lib/aicc_test.ts` | OpenDAN/Jarvis/CLI canonical typed/helper 调用与 TaskMgr 轮询 |
| WP-17D | `cd src && cargo test -p scheduler -p buckyos-api -p aicc` | rootfs/dev settings、首装/重装、locked credential、RBAC 与 reload |

能力基线 schema v3 分别记录 `provider_driver`（公共 RPC 与报告分组）、
`provider_profile_id`、`protocol_adapter_ids` 和 `model_driver_ids`。preflight 会独立校验
canonical api_type 值域、typed method 值域及其显式关联，并检查能力基线与 T1.5
协议契约中的 Profile/Adapter 身份一致；这些身份不会回退成 settings 中的
`provider_driver`。

T1.5 的 `official_variant_rules` 独立记录官方模型、variant 与预期下发参数。
运行时 metadata 展开的每个 variant 都是独立协议单元；缺少官方期望、缺失或多出
variant、没有对应 API contract、实际 wire 参数不一致都会使测试失败。

```bash
cd test/aicc_test
pnpm run acceptance:preflight
deno check acceptance/*.ts
pnpm run acceptance:self-test
cp aicc_acceptance.example.toml aicc_acceptance.local.toml
pnpm run acceptance:t1 -- --config aicc_acceptance.local.toml --allow-config-mutation
pnpm run acceptance:t1.5-mock-provider -- --port 18081
AICC_T15_ALLOW_CONFIG_MUTATION=true pnpm run acceptance:t1.5 -- \
  --gateway-url https://test.buckyos.io \
  --mock-base-url http://aicc-reachable-host:18081 \
  --mock-control-url http://127.0.0.1:18081 \
  --provider openai \
  --allow-config-mutation
pnpm run acceptance:gateway -- --config aicc_acceptance.local.toml
```

T1.5 可以用 `--start-local-mock` 启动本机 Mock；只有 AICC 服务也能访问 runner loopback 时才可将其作为 Provider endpoint。配置变更需要环境变量 `AICC_T15_ALLOW_CONFIG_MUTATION=true` 与命令行 `--allow-config-mutation` 同时授权。Runner 创建带 `run_id` 的临时 Provider instance，并在正常结束或异常退出时调用 `provider.delete`，等待运行时 inventory 中该实例消失，再重置 Mock。它顺序执行单元，固定全局和 Provider 并发为 1，并用 `--provider-min-interval-ms` 控制同 Provider 请求间隔。按 Provider 回归使用 `--provider <driver>`；目标重测可以重复传 `--case <case_id>`，未知或超出 Provider 范围的 case 会使执行失败。

T1 的 `t1.config.cloud_update_dynamic_catalog` 通过 NDN 连续发布两个完整 cloud revision：修改已有 Model Driver 的逻辑挂载、删除并恢复已有模型项，并动态增加 Provider Rules 和 Known Provider 文件；断言 cache 原子提交、全局 metadata sequence 收敛、动态 Provider 路由和真实 Mock 访问。T1.5 的 `t1.5.openai.openai.responses.v1.llm.cloud-update` 覆盖云端 Provider Rules 更新，并从真实 Adapter 的捕获请求确认 `service_tier` 已生效。两者都在结束时发布更高 revision 的 tombstone、禁用更新并恢复原始 system-config。

NDN fixture 默认使用 `/opt/buckyos`；临时 devtest root 可通过 `AICC_NDN_GATEWAY_BINARY`、`AICC_NDN_NAMED_STORE_CONFIG`、`AICC_NDN_GATEWAY_CONTROL_URL`、`AICC_NDN_SYSTEM_ROOT` 和 `AICC_CLOUD_CACHE_ROOT` 覆盖。T1 也可在 TOML `[fixtures]` 中配置对应的小写字段。fixture 使用当前 cyfs-gateway 文档定义的 `cyfs-dir` process chain 语义路径绑定，因此运行环境的 gateway binary 必须包含该契约。

T1.5 与其它层共用 `schema_version=1` 的 acceptance report，输出到 `<report-dir>/<run-id>/`，包含 `summary.json`、Markdown 摘要、逐 case evidence 和零费用 finance 文件；即使初始化失败，也会记录 runner failure、未执行 manifest 单元和 cleanup 结果。

T1.5 契约的 endpoint、header、body、response、stream event、异步状态和错误形态只允许依据 Provider 官方 API 文档、官方 schema/SDK 协议定义和官方错误文档更新。AICC 设计文档只用于确定 typed method、adapter/operation 边界、稳定错误码和 Provider instance 配置，不用于生成 Provider wire 期望。官方资料不明确的协议点不得从 AICC 实现、metadata、日志或旧 Mock 猜测。成功用例同时检查 Provider 响应被映射成对应 canonical typed 输出、usage 和异步 operation 归因；错误用例检查 `provider_start_failed`、Provider 原始错误码摘要和 canonical `retriable`。

真实调用必须通过 `allow_real_model_calls = true` 或命令行 `--allow-real-model-calls` 显式开启，并受调用数、成本和 timeout 上限约束。需要安全审计计划时，`--no-real-model-calls` 可强制覆盖 TOML 中的开启值，仍读取真实 inventory、生成完整 skipped/N/A/基线差异与零成本报告。Provider 返回 `request not allowed` 时记录为 `provider_restricted` 和 `platform_limitation`，不计入 passed、failed 或 skipped。报告会把能力基线不一致、路由/资源/安全断言失败和成功调用后的 usage/trace 归因失败写入结构化 `product_defects`，记录预期、实际结果和证据路径；测试不会修改 AICC/Jarvis 实现。

Runner 会并行执行不同 case/session。`global_concurrency` 控制整轮并发，`provider_concurrency` 和 `provider_min_interval_ms` 是默认 Provider 限制；可通过 `[limits.<provider_driver>]` 单独覆盖。每次 retry 也重新经过同一 Provider 的并发和请求间隔门禁，不会绕过限流。

每轮会生成独立的 `finance.json`、`finance.csv` 和 `finance.md`。账本按 case/attempt 记录 token、request unit、调用前估算和 AICC 返回的 USD cost，并按 Provider、instance、精确模型、API、case 汇总。Provider 未返回费用时会计入“未知费用的估算敞口”，不会记成零成本；并发调用在发出前会先预留预算。

开启真实模型调用后，runner 会先输出 case、最大调用次数和预计成本，再进入 10 秒确认倒计时；输入 `c` 取消，直接回车立即开始，超时视为确认。已经获得人工授权的自动化运行可传 `--yes` 跳过倒计时。`--yes` 只是 runner 的非交互开关，不代表 CodeAgent 已获得运行 T2/T3 的人工授权。

T1 会临时写入带 `run_id` 的 Mock Provider instance，并在 `finally` 中原样恢复整个 `services/aicc/settings`。为避免误改环境，配置文件的 `mock.allow_config_mutation` 与命令行 `--allow-config-mutation` 必须同时开启。AICC 与 Mock 不在同一主机时，`mock.base_url` 必须填写 AICC 服务进程可访问的地址，不能使用 runner 自己的 loopback。

第二租户隔离用例通过 `[auth].other_tenant_session_token` 或 `BUCKYOS_TEST_OTHER_TENANT_SESSION_TOKEN` 参数化，覆盖 task 查询/取消、usage、msg-center 消息、Named Object 和管理方法 RBAC。未配置时这些 case 保留在 manifest 和覆盖报告中并明确记为 `skipped`，不会伪造同租户结果或阻断其他 T1 用例。

T2 会为 OpenAI、Claude、Google Gemini、Fal、MiniMax、OpenRouter、Kimi、GLM、DeepSeek、Doubao、Qwen 和 SN 生成全部 active 基础物理模型的最小 `ProviderInstance × model × API-Type` 矩阵；是否实际执行由 `--provider`、凭据和真实调用开关共同决定。可用报告中的 `targeted_retest_command`，或重复传入 `--case <case_id>`，只重跑失败单元。没有账号或明确禁止执行的 Provider 仍保留基线和用例，但不应开启真实调用。

普通 `chat.completions.create` 用例显式关闭 AICC 默认附加的 `web_search`，避免把基础聊天错误地限定为必须支持联网搜索；联网搜索作为独立 capability 分支验证。Provider 凭据临时写入后，runner 会等待 system-config 与 AICC runtime settings 收敛，再验证 settings 字节恢复、运行时 inventory、Named Data 和消息资源清理。
