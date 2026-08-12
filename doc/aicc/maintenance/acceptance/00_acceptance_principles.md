# AICC 验收原则与设计依据

定义 AICC 验收测试的总体目标、设计依据、协议约束、优先级、确定性要求、风险处理和第一阶段边界。

本文档是拆分后的自包含验收任务文档。实现或评审本任务时，以本文档和 README 中列出的依赖文档为准。

## 1. 目标

本文基于 `doc/aicc` 目录下的 AICC 设计文档，制定 AICC 模块的分层测试验收方案。目标是在不依赖真实模型、不产生不可控费用的前提下，先用确定性的 Mock 模型覆盖协议、路由、任务、资源、配置、异常和安全语义；再通过 gateway 访问由 `buckyos-devkit` 临时启动的 group 环境，使用真实模型完成端到端验收，并在验收结束后清理该临时环境。

验收方案需要覆盖：

1. AICC 文档中说明的各类能力、控制面和运行时行为。
2. 主流模型接口协议，包括 OpenAI、Claude、Google Gemini、OpenAI-compatible / OpenRouter、fal、SN AI Provider 等 Provider 的请求参数、响应格式、错误格式和 streaming 差异。
3. 文本、结构化数据、图片、音频、视频等输入输出格式。
4. 非结构化数据的 `url`、`base64`、`named_object`、artifact 输出策略。
5. AICC 的异步任务与进度观察语义，以及 Provider-native streaming 到 task data / final summary 的适配。
6. 路由、Provider、协议、任务、权限、预算、资源、配置等异常路径。
7. 低成本、确定性 Mock 模型前期测试，以及真实模型的 gateway 验收。
8. gateway 发布强覆盖验收必须覆盖 `openai`、`fal`、`google-gemini`、`claude`、`openrouter`、`sn-ai-provider` 六类 Provider；其中 `sn-ai-provider` 不需要普通 API key。普通开发验收可按缺失 key 将真实 Provider 用例标记为 skipped。
9. gateway 真实模型验收必须按 `api_type × method × 标准逻辑目录路径 × Provider × model` 的笛卡尔积生成用例；`api_type` 以代码中 canonical `ApiType` 枚举为准，标准逻辑目录路径以当前生效 `LocalLogicalTreeConfig.logical_definitions`、`SessionConfig.logical_tree` 全部可寻址节点和 `models.list` 暴露的逻辑目录为准，Provider 与 model 以实际 inventory 中可观察到的可用模型为准。
10. 每个矩阵用例必须同时覆盖逻辑模型和实际物理模型两种调用方式：先用逻辑目录路径执行 `route.resolve` 并断言选中的 `selected_exact_model`，再用该精确模型名执行 typed inference 或 legacy method，报告中必须保留 requested logical path 与 exact model 的对应关系。
11. 新模型、新 Provider、新逻辑目录挂载、metadata / 运营策略 / routing 更新的维护验收闭环，覆盖测试环境相关用例、测试环境全量用例、发布环境相关用例、发布环境全量用例，以及必要的回滚验证。

## 2. 设计依据

主要依据：

- `doc/aicc/aicc_api设计.md`
- `doc/aicc/aicc_router.md`
- `doc/aicc/aicc 逻辑模型目录.md`
- `doc/aicc/aicc_requirements.md`
- `doc/aicc/aicc_provider_plan.md`
- `doc/aicc/maintenance/krpc_aicc_calling_guide.md`
- `doc/aicc/maintenance/update_aicc_settings_via_system_config.md`
- `doc/aicc/aicc_usage_log_db_requirements.md`
- `doc/aicc/maintenance/how_to_add_provider.md`
- `doc/aicc/driver_metadata_schema.md`
- `doc/aicc/maintenance/aicc_maintenance_roles.md`

`doc/aicc/maintenance/` 下的文档只作为维护场景和当前实现参考；验收目标必须回到根目录需求、设计、协议和 schema 文档确认，不能由维护参考单独派生新的设计约束。

关键协议约束：

- kRPC `method` 是 schema discriminator，例如 `llm.chat`、`image.txt2img`、`audio.asr`。
- 正式 request body 放在 `payload.input_json`，`payload.resources` 只用于资源复用或旧调用方兼容。
- `ResourceRef` JSON tag 使用 `url`、`base64`、`named_object`。
- AICC 不暴露独立 streaming 协议；长任务、进度、Provider streaming 中间态统一通过 task-manager event / task data 观察。
- AI method response 只有 `succeeded`、`running`、`failed` 三类状态；失败细节写入 task event / task data。
- 精确模型名格式为 `<provider_model_id>[:variant]@<provider_instance_name>`；variant 由 driver metadata 展开并 lower 为 provider base model + `provider_options`。
- 精确模型默认不 fallback，除非显式开启。
- fallback 不得跨 API namespace。
- `local_only`、隐私、预算、能力、上下文长度、Provider health 是硬过滤条件。
- 使用量记录只在 Provider 成功完成且存在 usage 时写入；缺 usage 的成功结果应视为 provider protocol error。可选 `finance_snapshot_json` 是非权威快照，缺失不得影响成功。

## 3. 用例优先级

| 优先级 | 含义 | 验收要求 |
|---|---|---|
| P0 | 核心协议、路由、任务、安全和 Mock Provider 行为 | L1/L2/L3 必须 100% 通过，阻塞合入 |
| P1 | 完整功能覆盖，包括所有 method、Provider 协议细节、usage 查询、配置闭环 | 可阶段性 pending，但必须在报告中标注缺口 |
| P2 | 真实模型、性能、兼容性、边界增强和长期稳定性 | gateway / nightly / 手工验收，不阻塞普通开发合入 |

## 4. 测试环境确定性要求

Mock 阶段必须保证执行环境确定：

1. Mock Provider 固定端口或由 runner 分配端口后写入 settings。
2. Mock Provider 启动成功后必须有 health check。
3. 每次运行使用独立 `run_id` 和独立报告目录。
4. fixture 数据固定 digest；runner 在开始时校验 digest。
5. Mock scenario 不依赖系统时间，除非用例明确测试 TTL / timeout。
6. timeout 用例应使用虚拟时钟或短固定延迟，避免 CI 偶发失败。
7. 并发用例必须设置最大等待时间，并在失败报告中输出未完成任务列表。
8. 所有 Mock usage、cost、latency 都应由配置或用例明确指定。
9. 测试结束后清理 Mock settings 或恢复测试前 settings，避免影响开发环境。

## 5. 风险与处理策略

| 风险 | 影响 | 处理策略 |
|---|---|---|
| 真实模型输出不稳定 | gateway 用例误报 | 只断言协议事实，不断言自然语言全文 |
| 真实模型费用失控 | 成本风险 | `allow_real_model_calls=false` 默认值、按五维矩阵生成前先输出 planned / skipped / not_applicable 数量和预计调用次数；失败用例最多 3 次 attempt；报告统计真实调用次数和估算成本 |
| L4 临时环境残留 | 占用本机资源、污染后续测试 | group 名带 `run_id`，默认 `cleanup_on_exit=true`；只清理 runner 创建的 group；清理失败写 warning |
| VM 多节点构造慢 | 发布验收耗时长 | 先构造空白 VM，再 clone 出所需节点；只在需要 gateway / SN / 多节点路径时扩展节点数 |
| Mock 与真实 Provider 差异过大 | Mock 通过但真实失败 | Mock 按 Provider 原生协议构造请求/响应，不只 mock AICC 内部 trait |
| Streaming 语义混乱 | UI 或 task 状态不一致 | AICC 协议只验最终 summary；中间态只验 task data / event |
| 使用量重复写入 | 账单和统计错误 | 幂等并发测试、usage 唯一约束测试 |
| 配置 reload 破坏旧状态 | 服务不可用 | 非法 settings reload 失败后必须保留上一版配置 |
| trace 泄露敏感信息 | 安全风险 | 脱敏扫描作为 P0 |
| 并发测试偶发失败 | CI 不稳定 | 固定 Mock 行为、短 timeout、失败输出足够诊断信息 |

## 6. 第一阶段明确不做

第一阶段目标是建立确定、可执行、可报告的验收体系。以下内容明确不做，避免范围失控：

1. 不压测真实模型。
2. 不把真实模型用例放入普通 CI 必跑。
3. 不比较真实模型自然语言质量。
4. 不断言真实模型自然语言全文一致。
5. 不实现复杂账单、发票、余额或对账逻辑。
6. 不把 Provider 原始完整响应写入报告。
7. 不把原始 prompt、原始文件内容、API key、session token 写入报告或普通日志。
8. 不要求所有需要 API key 的真实模型 Provider 在无 key 环境下通过；`sn-ai-provider` 例外，它本身不需要普通 API key。
9. 不要求 `agent.computer_use` 在第一阶段接入真实桌面或浏览器环境。
10. 不要求所有视频、音乐等高成本能力在 Mock 阶段之外真实执行。
11. 不引入新的通用测试框架或依赖，除非先单独确认。
12. 不在验收 runner 中自动修改生产环境真实 Provider 配置，除非配置文件显式允许。

