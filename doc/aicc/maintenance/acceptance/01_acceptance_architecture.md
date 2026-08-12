# AICC 验收架构、执行入口与任务拆分

定义 L1/L2/L3/L4 架构、统一执行脚本、CI 与手工边界、里程碑、分层用例入口、执行命令和实现任务拆分。

本文档是拆分后的自包含验收任务文档。实现或评审本任务时，以本文档和 README 中列出的依赖文档为准。

## 1. 测试分层

| 层级 | 入口 | 位置 | 模型 | 执行方式 | 目标 |
|---|---|---|---|---|---|
| L1 白盒单测 | AICC 内部模块 | `src/frame/aicc/tests`；如需测试非 `pub` 程序块，可嵌入对应实现文件 | Rust Mock Provider | `cargo test -p aicc` | 精细覆盖路由、调度、协议转换、任务、usage log、异常分支 |
| L2 AiccClient 黑盒 | `AiccClient` | `src/kernel/buckyos-api/tests/aicc_client_test.rs` | In-process Mock AICC server | `cargo test -p buckyos-api --test aicc_client_test` | 验证 SDK client 的 request/response、错误、任务接口语义 |
| L3 本地 kRPC 黑盒 | `/kapi/aicc` | `test/aicc_test` | TypeScript Mock Provider | 启动本机 BuckyOS + AICC 后运行 TS 用例 | 验证真实服务进程、配置重载、kRPC、task-manager、资源链路 |
| L4 Gateway 验收 | gateway 远程访问 | `test/aicc_test` | 真实模型 | `buckyos-devkit` 临时 group + 自动 runner | 验证真实部署链路；覆盖 `api_type × method × 标准逻辑目录路径 × Provider × model` 的笛卡尔积；每个用例同时验证逻辑模型路由和精确物理模型调用 |

分层原则：

- L1/L2/L3 使用 Mock 模型，必须确定性执行，适合 CI 和开发阶段反复运行。
- L4 使用真实模型，受网络、模型状态、额度和内容不确定性影响，不要求 100% 通过，但必须报告失败原因。
- Mock 阶段不得访问真实模型，不得依赖外网，不得依赖真实 API key。
- 真实模型验收只验证协议事实、任务状态、artifact、usage、trace 和错误分类，不断言自然语言结果全文。
- L4 的被测 BuckyOS 环境由 runner 通过 `buckyos-devkit` 构造为临时 group；执行脚本的宿主机只作为客户端经 gateway 访问，不在被测 group 内直接调用本地服务。
- L4 runner 必须在结束时清理新构造的 group 环境；清理失败作为 warning 写入报告，不能掩盖原始测试失败。

## 2. 统一执行脚本

当前 `test/aicc_test` 已有 Deno smoke、`models.list`、fal provider 测试和 `aicc_remote_runner.ts`。后续仍建议在此基础上收敛出统一 acceptance runner；不要再假设该目录是空白起点。

现有入口：

```text
test/aicc_test/aicc_smoke.ts
test/aicc_test/test_list_models.ts
test/aicc_test/test_fal.ts
test/aicc_test/aicc_remote_runner.ts
```

规划中的统一入口可命名为：

```text
test/aicc_test/run_acceptance.{ts|py}
```

执行顺序：

1. `cargo test -p aicc`
2. `cargo test -p buckyos-api --test aicc_client_test`
3. 检查本机 BuckyOS / AICC 状态。
4. 启动或连接 TS Mock Provider（当前 smoke/remote runner 还未提供统一 Mock Provider 管理接口）。
5. 写入 Mock settings，调用 `service.reload_settings`。
6. 调用 `models.list` 验证 Mock Provider 生效。
7. 执行本地 kRPC TS 用例。
8. 如 TOML 配置启用 gateway 且 `managed_by_devkit=true`，则通过 `buckyos-devkit` 创建临时 group；当前已有 `aicc_remote_runner.ts` 主要面向既有 gateway。
9. 从宿主机经 gateway 登录临时 group，并写入真实 Provider settings。
10. 调用 `models.list` 并读取最终生效逻辑目录，生成 `api_type × method × logical_path × Provider × model` 矩阵。
11. 执行真实模型 workflow；失败 case 最多累计 3 次 attempt。
12. 生成报告。
13. 清理本次 runner 创建的临时 group。

报告输出：

```text
test/aicc_test/reports/acceptance/<run_id>/
  summary.md
  summary.json
  cargo_aicc.log
  cargo_buckyos_api.log
  local_krpc/
  gateway/
    matrix.json
    attempts/
  artifacts/
  cleanup.log
```

## 3. CI 与手工验收边界

| 范围 | 执行环境 | 是否阻塞合入 |
|---|---|---|
| L1 `cargo test -p aicc` | CI / 本地 | 是 |
| L2 `cargo test -p buckyos-api --test aicc_client_test` | CI / 本地 | 是 |
| L3 本地 kRPC + Mock Provider | CI 或 nightly；本地可手工 | P0 阶段应阻塞 |
| L4 gateway + 真实模型 | nightly / 手工 | 不阻塞普通合入，阻塞发布验收 |

真实模型 key 缺失时，L4 用例必须 `skipped`，不能算失败。

## 4. 验收里程碑

验收落地建议拆成三个里程碑，避免一次性实现全部用例导致范围过大。

| 里程碑 | 范围 | 完成标准 |
|---|---|---|
| M0 | L1 白盒单测 + L2 AiccClient 黑盒测试 | Rust Mock Provider 可用；`cargo test -p aicc` 和 `cargo test -p buckyos-api --test aicc_client_test` 的 P0 用例 100% 通过 |
| M1 | L3 本地 kRPC + TypeScript Mock Provider | 本机 BuckyOS + AICC 启动后，可通过 TS Mock Provider 或现有 smoke 扩展完成配置重载、`models.list`、各类 method、task、usage、trace 和异常路径测试 |
| M2 | L4 gateway + 真实模型验收 | runner 能连接既有 gateway；发布强覆盖阶段再要求能用 `buckyos-devkit` 启动临时 group，经 gateway 执行 `api_type × method × logical_path × Provider × model` 矩阵 workflow，报告可区分 passed / failed / skipped / not_applicable / partial；真实模型失败原因和重试 attempt 可追踪 |

里程碑边界：

- M0 只要求进程内确定性测试，不启动完整 BuckyOS。
- M1 要求真实 AICC 服务进程和本地 kRPC 链路可用，但仍不访问真实模型。
- M2 允许访问真实模型，主要用于发布前验收和远程部署验证；只有 runner 本次创建了临时 group 时，M2 完成后才必须清理该 group。

## 5. 分层用例清单

本节把前文功能域拆成更接近实现任务的用例清单。用例 ID 可在实现时继续细化，但应保持前缀稳定。

### 5.1 L1 白盒单测

| 用例族 | 优先级 | 覆盖点 |
|---|---|---|
| `l1_routing_exact_model_*` | P0 | 精确模型解析、Provider instance 校验、API type 校验、默认不 fallback |
| `l1_routing_logical_tree_*` | P0 | 逻辑目录展开、items target、目录软链接、候选去重 |
| `l1_routing_fallback_*` | P0 | `strict`、`parent`、`target_exact`、`target_logical`、`disabled` |
| `l1_routing_loop_*` | P0 | fallback loop、logical tree loop、最大 fallback depth |
| `l1_scheduler_weight_*` | P0 | item weight、exact model weight、weight 0 硬过滤、同权重 profile 评分 |
| `l1_scheduler_profile_*` | P0 | `cost_first`、`latency_first`、`quality_first`、`balanced`、`local_first`、`strict_local` |
| `l1_request_overlay_*` | P0 | overlay 合并、逻辑目录覆盖、policy locked、互不污染 |
| `l1_provider_protocol_openai_*` | P0 | OpenAI request/response 转换、tool call、JSON schema、SSE 聚合 |
| `l1_provider_protocol_claude_*` | P0 | Claude content block、tool use、vision block、stop reason、usage |
| `l1_provider_protocol_gemini_*` | P0 | Gemini parts、function call、safety block、operation 状态 |
| `l1_provider_protocol_fal_*` | P1 | fal submit/poll、artifact URL、operation timeout |
| `l1_resource_ref_*` | P0 | `url`、`base64`、`named_object`、FileObject meta 推导 |
| `l1_task_lifecycle_*` | P0 | immediate、async running、final succeeded、failed、cancel |
| `l1_usage_log_*` | P0 | 成功写 usage、幂等去重、缺 usage 报错、查询聚合 |
| `l1_method_api_type_canonical_*` | P0 | `method` 与 `api_type` 边界、`llm` vs `llm.chat`、非正式 api_type 拒绝或降级诊断 |
| `l1_control_method_*` | P0 | cancel、reload、models list、usage/quota/provider 查询的 schema 和权限边界 |
| `l1_security_*` | P0 | `local_only`、`proxy_unknown`、locked policy、trace 脱敏 |
| `l1_concurrency_*` | P1 | session patch 并发、幂等并发、异步任务并发完成 |

### 5.2 L2 AiccClient 黑盒测试

| 用例族 | 优先级 | 覆盖点 |
|---|---|---|
| `l2_client_llm_chat_success` | P0 | AiccClient 构造标准 `llm.chat` 请求并解析成功响应 |
| `l2_client_exact_model_no_fallback` | P0 | 精确模型不可用时透传可判断错误 |
| `l2_client_idempotency_*` | P0 | running / succeeded / failed / conflict 语义 |
| `l2_client_async_task_*` | P0 | running response、event_ref、最终 task 查询 |
| `l2_client_cancel_*` | P0 | cancel 成功、unknown task、forbidden |
| `l2_client_control_method_*` | P0 | reload、models list、usage/quota/provider 查询响应解析和错误映射 |
| `l2_client_resource_ref_*` | P1 | client 侧 `ResourceRef` JSON tag 和反序列化 |
| `l2_client_error_mapping_*` | P0 | kRPC error 与 AICC task failed error 的边界 |

### 5.3 L3 本地 kRPC 黑盒测试

| 用例族 | 优先级 | 覆盖点 |
|---|---|---|
| `l3_settings_reload_mock_*` | P0 | system_config 写入 Mock settings、reload、models.list |
| `l3_provider_admin_*` | P0 | provider.validate/add/delete/refresh_models 的 system_config 写入、reload 和回滚语义 |
| `l3_models_list_*` | P0 | `models.list` / `service.models.list` inventory、逻辑目录、health、legacy aliases 脱敏诊断 |
| `l3_quota_query_*` | P1 | `quota.query` 按 tenant、capability、method 返回预算状态和拒绝路径 |
| `l3_krpc_llm_chat_*` | P0 | 纯文本、多模态 content part、tool call、JSON schema |
| `l3_krpc_resource_*` | P0 | `url`、`base64`、`named_object` 输入和 artifact 输出 |
| `l3_krpc_stream_*` | P0 | Mock streaming chunks、task data progress、final summary |
| `l3_krpc_async_*` | P0 | image/audio/video 类异步 task 状态闭环 |
| `l3_krpc_usage_*` | P0 | usage event 写入和查询 |
| `l3_krpc_failover_*` | P0 | Provider timeout / 5xx / quota exhausted 后 failover |
| `l3_krpc_security_*` | P0 | local_only、跨用户访问拒绝、脱敏扫描 |
| `l3_krpc_legacy_*` | P1 | legacy alias、旧字段兼容或迁移提示 |

### 5.4 L4 Gateway 真实模型验收

| 用例族 | 优先级 | 覆盖点 |
|---|---|---|
| `l4_gateway_openai_<model>_complex_workflow` | P2 | OpenAI 每个支持模型的文本、JSON schema、tool call、usage、trace |
| `l4_gateway_claude_<model>_complex_workflow` | P2 | Claude 每个支持模型的多模态或 vision、tool use、usage、trace |
| `l4_gateway_gemini_<model>_complex_workflow` | P2 | Google Gemini 每个支持模型的多模态、safety / function call / operation 语义 |
| `l4_gateway_openrouter_<model>_complex_workflow` | P2 | OpenRouter 每个支持模型的 OpenAI-compatible 协议兼容、usage、trace |
| `l4_gateway_fal_<model>_media_workflow` | P2 | fal 每个支持模型的 image/video/audio 工具型异步任务和 artifact |
| `l4_gateway_sn_ai_provider_<model>_complex_workflow` | P2 | SN AI Provider 每个支持模型的无普通 API key 链路、usage、trace、provider 归因 |
| `l4_gateway_models_list` | P2 | 真实环境 inventory、逻辑目录和 Provider health 可诊断 |

L4 用例 ID 中的 `<model>` 必须使用稳定可读的 slug，由精确模型名归一化得到；报告中必须保留原始精确模型名。

## 6. 执行命令约定

推荐统一 runner 最终屏蔽底层命令，但文档仍保留基础命令，便于开发者单独定位问题。以下分为“当前可用命令”和“规划统一命令”。

### 6.1 Rust 单测

在仓库根目录执行：

```bash
cargo test -p aicc
cargo test -p buckyos-api --test aicc_client_test
```

如果按 AGENTS.md 建议在 `src` 目录运行，也可使用 workspace 相对命令；最终 runner 应固定工作目录，避免不同目录下路径解析不一致。

### 6.2 本地 kRPC Mock 验收

当前可用流程：

```bash
uv run src/check.py
cd test/aicc_test
pnpm install
pnpm test
pnpm run test:models
pnpm run test:fal
```

当前命令含义：

1. `pnpm test` 执行 `aicc_smoke.ts`，输出 `reports/aicc_smoke/<run_id>`。
2. `pnpm run test:models` 调用 `models.list`，打印 Provider inventory、legacy aliases 和逻辑目录树。
3. `pnpm run test:fal` 执行 fal provider 的 `image.upscale` / `image.bg_remove` / `video.upscale` 用例；未配置 fal 时按 skipped 处理。
4. 这些命令当前连接已启动的 BuckyOS / AICC，不负责自动启动 TS Mock Provider 或写入 Mock settings；统一 L3 runner 需要补齐该管理闭环。

规划统一命令：

```bash
cd test/aicc_test
pnpm run acceptance:local
```

`acceptance:local` 应完成：

1. 启动或连接 TS Mock Provider。
2. 写入 Mock settings。
3. 调用 `service.reload_settings`。
4. 调用 `models.list` 验证配置生效。
5. 运行 L3 用例。
6. 输出 `reports/acceptance/<run_id>`。

### 6.3 Gateway 真实模型验收

当前可用 remote runner：

```bash
cd test/aicc_test
pnpm install
pnpm run remote -- --config ./aicc_remote_runner.toml
```

规划统一 gateway 命令：

```bash
cd test/aicc_test
pnpm run acceptance:gateway -- --config ./aicc_acceptance.toml
```

真实模型验收必须显式传入配置文件；不应从开发者环境变量中隐式读取 key 后直接发起调用，避免误触发费用。

推荐最终提供一个全量自动化入口，用于发布前一次性执行 L1/L2/L3/L4 并输出报告：

```bash
cd test/aicc_test
pnpm run acceptance:all -- \
  --openai-key "<openai-api-key>" \
  --fal-key "<fal-api-key>" \
  --gemini-key "<google-gemini-api-key>" \
  --claude-key "<claude-api-key>"
```

`acceptance:all` 的职责：

1. 固定从仓库根目录或 `src` 目录解析路径，避免工作目录差异。
2. 执行 L1/L2 Rust 单测。
3. 执行 L3 本地 Mock 验收。
4. 使用 `buckyos-devkit` 创建并启动 L4 临时 group，通过 gateway 访问该 group。
5. 将传入的 4 个 key 写入临时 group 的 AICC settings；`sn-ai-provider` 不需要普通 API key。
6. 对 `openrouter`，runner 优先读取配置文件或临时 group settings 中的 `openrouter` key；如果发布验收要求强覆盖但缺 key，应在 preflight 阶段失败。普通开发验收可将 openrouter 矩阵标记为 `skipped`。
7. 动态读取 `models.list` 和最终生效逻辑目录，生成 `api_type × method × logical_path × Provider × model` 矩阵。
8. 每个 planned 矩阵用例执行逻辑模型段与精确物理模型段验证，失败后最多额外执行 2 次。
9. 输出 `summary.md`、`summary.json` 和脱敏后的 attempt 明细。
10. 清理 runner 新建的临时 group。

为了让参数尽可能少，`sn-ai-provider` 不设置普通 API key；OpenRouter key 不作为默认必填命令行参数，但发布验收若要求 OpenRouter 强覆盖，必须通过 `--openrouter-key` 或配置文件提供。

## 7. 实现任务拆分建议

建议按以下任务顺序实现，减少互相阻塞。

| 顺序 | 任务 | 主要文件 |
|---:|---|---|
| 1 | 固化 `summary.json` schema 和报告目录结构 | `test/aicc_test` |
| 2 | 增加 fixture manifest 和最小 fixture | `test/aicc_test/fixtures` |
| 3 | 实现 TS Mock Provider 管理接口和 OpenAI-like 最小接口 | `test/aicc_test` |
| 4 | 实现 L3 runner preflight、settings reload、models.list | `test/aicc_test` |
| 5 | 增加 L3 `llm.chat`、resource、usage、trace P0 用例 | `test/aicc_test` |
| 6 | 增加 L2 `aicc_client_test.rs` | `src/kernel/buckyos-api/tests` |
| 7 | 对齐 Rust Mock Provider 与 TS Mock scenario 契约 | `src/frame/aicc/tests`、`test/aicc_test` |
| 8 | 增加 Provider-specific protocol P0 用例 | `src/frame/aicc/tests` |
| 9 | 增加 gateway TOML、`buckyos-devkit` 临时 group 管理和真实模型 workflow | `test/aicc_test` |
| 10 | 增加五维矩阵生成、失败 case 三次 attempt 和 attempt 报告 | `test/aicc_test` |
| 11 | 接入脱敏扫描和发布验收报告 | `test/aicc_test` |

每个任务完成后至少应能回答：

- 增加了哪些 case id。
- 覆盖了哪些需求追踪项。
- 如何单独运行。
- Mock 与真实模型是否都会触发。
- 报告中如何定位失败。

## 8. 评审清单

新增或修改 AICC 验收用例时，评审应检查：

1. case id 是否符合命名规范。
2. 是否标明 layer、priority、method、provider、scenario。
3. 是否可以稳定复现。
4. 是否避免真实模型默认调用。
5. 是否有明确断言，而不是只检查“不报错”。
6. 是否覆盖成功和至少一个失败路径。
7. 是否检查 usage、trace、task 或 artifact 中与该用例相关的关键字段。
8. 是否避免记录密钥、token、原始 prompt 和原始文件内容。
9. 是否在失败时输出足够诊断信息。
10. 是否更新需求追踪矩阵或 manifest。
11. 是否需要同步更新 `doc/aicc` 其它协议文档。
12. 是否会引入新的依赖；如需要，应先单独确认。

