# AICC CI、发布门槛与性能组件

定义 CI 与手工边界、阶段性门槛、发布验收、性能并发最低要求、文档联动和评审清单。

本文档是拆分后的自包含验收任务文档。实现或评审本任务时，以本文档和 README 中列出的依赖文档为准。

## 1. CI 与手工验收边界

| 范围 | 执行环境 | 是否阻塞合入 |
|---|---|---|
| L1 `cargo test -p aicc` | CI / 本地 | 是 |
| L2 `cargo test -p buckyos-api --test aicc_client_test` | CI / 本地 | 是 |
| L3 本地 kRPC + Mock Provider | CI 或 nightly；本地可手工 | P0 阶段应阻塞 |
| L4 gateway + 真实模型 | nightly / 手工 | 不阻塞普通合入，阻塞发布验收 |

真实模型 key 缺失时，L4 用例必须 `skipped`，不能算失败。

## 2. 阶段性验收门槛

### 2.1 Mock 阶段

- L1/L2/L3 P0 必须 100% 通过。
- 不允许访问真实模型。
- 测试执行环境必须可重复，失败必须可复现。
- usage、task、trace、resource、routing 的核心断言必须稳定。
- 报告必须能定位失败原因。

### 2.2 Gateway 阶段

- gateway 链路、鉴权、配置读取、报告生成必须稳定。
- 被测环境必须由 `buckyos-devkit` 临时 group 启动，宿主机 runner 作为客户端通过 gateway 访问。
- 发布强覆盖 Provider 矩阵必须包含 `openai`、`fal`、`google-gemini`、`claude`、`openrouter`、`sn-ai-provider`；普通开发验收可因缺 key skipped，但必须在报告中明确。
- 用例覆盖必须按 `api_type × method × 标准逻辑目录路径 × Provider × model` 的笛卡尔积生成；每个 planned 用例必须覆盖逻辑模型路由和精确物理模型调用。
- 每个失败用例必须额外重试 2 次，累计最多 3 次；任意一次成功则最终判定为成功。
- 真实模型内容不要求固定，但协议、任务状态、错误分类、trace、usage 记录必须可判定。
- 真实模型失败不能吞掉原因，必须进入报告。
- 测试完成后必须清理临时 group；如果清理失败，报告记录 `cleanup_failed` warning。

## 3. 发布验收标准

发布前建议满足以下硬指标：

1. P0 Mock 用例 100% 通过。
2. `cargo test -p aicc` 通过。
3. `cargo test -p buckyos-api --test aicc_client_test` 通过。
4. 本地 kRPC Mock 验收能完成 `reload_settings -> models.list -> route -> provider call -> task / usage / trace` 闭环。
5. gateway runner 能读取 TOML 配置并生成 `summary.md` 和 `summary.json`。
6. gateway runner 能通过 `buckyos-devkit` 启动临时 group，并从宿主机经 gateway 完成访问。
7. 已配置真实凭据的 Provider 必须覆盖其全部可用模型；`sn-ai-provider` 必须覆盖 API Key 和动态登录两种模式；缺少所选模式凭据的 Provider 在普通开发验收中标记为 `skipped`，发布强覆盖验收中应 preflight 失败。
8. 报告、trace、task data、日志摘要中不得出现 API key、session token、原始 prompt 全文和原始文件内容。
9. 真实模型调用次数、attempt 次数和成本在报告中可见。
10. 所有 failed / partial 用例都有明确失败原因、错误码或 Provider 摘要。
11. runner 创建的临时 group 已清理，或报告中明确记录保留原因和清理命令。

### 3.1 新模型维护更新验收

当验收目标来自 `maintenance/aicc_maintenance_roles.md` 中的新模型、新 Provider、新逻辑目录挂载、metadata、运营策略或 routing 维护动作时，除满足常规发布标准外，还必须执行本节闭环。

维护更新类型：

| 类型 | 交付物 | 必验内容 |
|---|---|---|
| 已有 Provider 新增协议兼容模型 | 模型事实 metadata、运营策略、必要的 routing_config | `models.list` 出现新 exact model；`api_types`、`capabilities`、上下文长度、`logical_mounts` 正确；成本、健康度、权重和 fallback 策略生效 |
| 新增 OpenAI-compatible Provider Instance | Provider Profile、协议族、`endpoint`、授权、discovery 策略 | 用户无需选择 API 代际；接入测试优先新接口并按序测试已注册历史接口；resolved Adapter 固化后 inventory 身份链正确；运行时不得重新探测或跨代际降级 |
| 新增非兼容 Provider adapter 或新 API type | 版本包、adapter、schema、metadata 基线、默认路由策略 | 新 adapter 的协议转换、错误映射、streaming / task 语义、usage、fallback 和 helper / typed inference 链路通过相关用例 |
| 仅更新运营策略 | 策略配置、成本 / quota / health / 权重 / 熔断 / 灰度规则 | 不改变模型事实；route trace 显示策略命中；回滚策略后路由恢复；不需要回滚 metadata |
| 随版本内置缓存更新 | 版本包内 builtin metadata / 默认策略 | 新安装或无云端更新环境中仍能识别发布时已知模型，并生成可用默认路由 |
| 云端 metadata 更新 | 严格递增的 manifest `revision_seq`、客户端兼容范围与分组目标；NDN 当前文件和 `metadata_target_seq`；Provider `metadata_applied_seq` | 新旧客户端获得各自兼容版本；非法发布被拒绝；新文件就绪前 target seq 不推进；Provider 真正完成库存刷新后才推进自己的 applied seq；两个触发点均收敛全部落后 Provider |
| 人工运行时覆盖 | `$BUCKYOS_ROOT/etc/aicc/driver_metadata/local/<driver>.json` 或 `system-config/<driver>.json` | `reload_settings` 后生效；优先级高于 NDN 当前云端 metadata；损坏配置被拒绝且不破坏可用基线 |

统一验收顺序：

1. 准备更新说明，列出 provider、model、api type、逻辑目录、模型事实变更、运营策略变更、routing 变更、是否需要 adapter 发版，以及影响的旧用例族。
2. 新增或更新命名可检索的相关用例，并在 manifest tags 中标明更新类型、provider、model、api type 和逻辑目录。
3. 在测试环境发布云端配置、运行时覆盖文件或版本包，触发 `reload_settings`。
4. 先执行本次新增用例和受影响旧用例，覆盖 inventory、metadata 解析、exact model、logical model、fallback、成本估算、禁用策略和错误返回。
5. 相关用例通过后执行 AICC 全量用例，确认旧 Provider、旧模型和旧路由策略未回归。
6. 发布环境上线后重复相关用例，再执行发布环境全量用例；发布环境的授权、网络、Provider 实际状态和报告摘要必须可诊断。
7. 如本次支持回滚，至少执行一次目标回滚用例：模型事实错误时回滚 metadata / override；路由错误时优先回滚运营策略或 routing_config；回滚后重新 `reload_settings`，确认 `models.list`、route trace 和关键调用恢复预期。

角色边界：

- BuckyOS 项目方更新公共协议、默认模型事实基线、默认运营策略基线、默认逻辑目录和随版本缓存时，必须同时提交或更新对应 L1/L3/L4 用例。
- 商业服务商跟随 BuckyOS 更新或维护自有 Provider 网关、模型事实包、运营策略包和产品默认 routing_config 时，必须保留服务商维度的用例 tags，报告中能按服务商 Provider / model 聚合。
- 产品用户通过 system_config、local metadata override 或 `session_overlay` 做临时接入时，验收只要求配置生效、可回滚和安全边界正确；不要求修改公共基线。
- 模型服务商主动提供 BuckyOS metadata / inventory / cost / quota / health 信息目前按畅想处理；如接入试点，应作为服务商或第三方 Provider 包验收，不作为 P0 默认要求。

## 4. 性能与并发最低要求

性能和并发不作为第一阶段的主要目标，但需要设置最低验收线，避免破坏基础可用性。

| 项目 | 最低要求 | 主要层级 |
|---|---|---|
| 路由解析耗时 | Mock 环境下单次普通路由不应成为主要耗时瓶颈；建议记录 p50 / p95，不先设置硬阈值 | L1/L3 |
| 并发 request overlay 路由 | 多个请求携带不同 `session_overlay` 时互不污染，无共享 session 状态 | L1 |
| 幂等重试 | 并发重复提交同一 `idempotency_key` 不得重复执行 Provider，不得重复写 usage | L1/L3 |
| usage 写入 | 多个异步任务并发完成时，usage event 不串任务、不重复、不丢失 | L1/L3 |
| artifact 输出 | 并发生成 artifact 时 ObjectId、meta、task result 不串 | L3 |
| failover | 多候选并发失败时，trace 能区分每次 attempt，不污染其它 request | L1/L3 |
| task 状态 | 多个异步任务并发运行和完成时，状态、event_ref、最终 result 对应正确 | L1/L3 |

并发测试建议：

1. 多个 request 携带不同 overlay 并发 route resolve。
2. 多个 request 携带相同 overlay 并发 route resolve。
3. 同一 idempotency key 并发提交相同 request。
4. 同一 idempotency key 并发提交不同 request。
5. 多个异步 Mock video/audio/image task 并发完成。
6. Provider 先失败后 failover 的请求与普通成功请求并发执行。

## 5. 文档联动要求

后续实现测试或修改 AICC 协议时，需要同步检查：

- `doc/aicc/aicc_api设计.md`
- `doc/aicc/aicc_router.md`
- `doc/aicc/aicc 逻辑模型目录.md`
- `doc/aicc/maintenance/krpc_aicc_calling_guide.md`
- `doc/aicc/maintenance/update_aicc_settings_via_system_config.md`
- `doc/aicc/aicc_provider_plan.md`
- `doc/aicc/aicc_usage_log_db_requirements.md`
- `doc/aicc/maintenance/aicc_maintenance_roles.md`
- `src/kernel/buckyos-api/src/aicc_client.rs`
- `src/frame/aicc/src`
- `test/aicc_test`

触发文档联动的变更包括：

1. 新增或改名 method。
2. 修改 request / response schema。
3. 修改 `ResourceRef` JSON 表达。
4. 修改 Provider settings 字段。
5. 修改 exact model 命名规则。
6. 修改 fallback、session config、policy 字段。
7. 修改 usage log schema。
8. 修改 task data / event 中 AICC 字段。
9. 修改 metadata、运营策略、NDN 当前文件/目标 seq、Provider applied seq、provider settings、routing_config 或回滚流程。

## 6. 评审清单

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
