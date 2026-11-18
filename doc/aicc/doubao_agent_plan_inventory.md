# Doubao Agent Plan 模型清单

2026-11-16 对齐官方当前清单。来源：

- [套餐概览](https://ark.volcengine.com/region:cn-beijing/docs/agent-plan-personal-plan-overview.md)
- [企业版套餐概览](https://ark.volcengine.com/region:cn-beijing/docs/agent-plan-enterprise-plan-overview.md)
- [模型上线公告](https://ark.volcengine.com/region:cn-beijing/docs/agent-plan-personal-model-release.md)
- [模型下线公告](https://ark.volcengine.com/region:cn-beijing/docs/agent-plan-personal-model-deprecation-announcement.md)
- [语音接入](https://ark.volcengine.com/region:cn-beijing/docs/agent-plan-personal-voice-model.md)

当前实现以 `src/frame/aicc/driver_metadata/providers/doubao-agent-plan.provider.json` 的 `static_inventory_models` 为准；AI Center 通过 AICC 获取库存，不维护另一份前端模型清单。Model Driver 的新条目由 `build.rs` 自动嵌入，构建时无需修改安装映射或生成副本。

本轮新增 `kimi-k3.1-pro`、`deepseek-v4.1-flash-economy`、`doubao-seedream-5.0-flash`，补齐对应推理档位和图片操作。经济版 DeepSeek 与标准版权重及能力相同，沿用相同的模型能力与推理档位。Kimi K3.1 Pro 复用已有 Model Driver。

移除已下线的 `doubao-seed-2.0-lite`、`doubao-seed-2.1-turbo`、`kimi-k2.7-code`、`minimax-m3`、`doubao-seedream-5.0-lite`，仅调整 `static_inventory_models`。保留这些模型原有的操作映射、能力及推理档位配置，方便官方恢复上架后直接加回清单。其他渠道的同名模型不受影响。

`kimi-k3` 仍列于当前官方清单，保留其调用能力。它自 2026-11-11 起停止向新用户提供服务，计划于 2026-11-25 14:00（北京时间）下线；当前新增接入应选择 `kimi-k3.1-pro`。

Agent Plan 的 `doubao-seed-asr-2.0` 仅提供 WebSocket 流式识别，资源 ID 为 `volc.seedasr.sauc.duration`。现有 ASR 适配器实现的是 HTTP AUC/Flash 协议，不能据模型同名复用，因此本轮不将它加入可调用库存。视频模型仍受用户套餐档位限制，静态清单表示渠道支持范围，不保证每个账号可用。

模型清单对齐只调整内置 metadata 与测试，不改变 RPC、SDK、CLI、共享类型、存储或第三方依赖。已检查 `aigc_image`、`aigc_audio`、`aigc_video`、`xllm`、`buckyos_admin` 手册：它们通过实时模型查询选择模型，没有引用本轮移除的 ID。按用户要求不修改 Jarvis 提示词或手册；模型限制由 AICC 配置表达，记录在本工程文档中。

回归入口（在 `src/` 下运行）：`cargo test -p aicc --lib -- --test-threads=1`。库存用例检查完整静态列表、已下线模型不在静态列表中、新增语言模型能力及长度；媒体挂载用例检查新增图片模型的逻辑路由。

初始验证：AICC 单测 794 项通过；metadata JSON 解析、官方表格清单比较及 `git diff --check` 通过。本轮清单采用上述官方公开文档核验。后续 T1.5/T2 验证记录如下。


T2 首轮暴露 Seedream Flash 单图请求携带 `sequential_image_generation=disabled` 被渠道拒绝。[官方图片 API](https://ark.volcengine.com/region:cn-beijing/docs/image-generation-api.md) 明确禁止该模型携带组图参数。原厂 Model Driver 通过 `/n` 的 `omit` converter 与缺省 `omit` fallback 表达单图参数映射，避免在图片编辑请求没有 `n` 时误拒绝。标准 Doubao Provider 同步将 `doubao-seedream-5-0-flash-260915` 指向独立 Flash Model Driver；Agent Plan 保留无版本后缀 ID。图片参数修复未修改公共产品组件或私有协议实现。后续标准渠道目录问题另见下文。

T1.5 的官方 wire fixture 对两个渠道的 Flash 模型均禁止 `sequential_image_generation` 和 `sequential_image_generation_options`；文生图请求显式包含 `n=1`，覆盖真实调用暴露的条件。新增负向 Mock 单测与跨两个渠道的完整 lowering 单测，验证字段不会发给上游。`acceptance:self-test` 共 163 项通过，定向 Rust lowering 和 provider revision 单测通过。

模型参数依据补充：[Kimi K3.1 Pro 原厂接入指南](https://platform.kimi.com/docs/guide/kimi-k3-1-pro-quickstart)、[DeepSeek 原厂参数与定价](https://api-docs.deepseek.com/quick_start/pricing/)、[Ark DeepSeek V4.1 Flash 经济版](https://docs.volcengine.com/docs/ark/deepseek-v4-1-flash?lang=zh)、[Ark 推理档位](https://ark.volcengine.com/region:cn-beijing/docs/deep-thinking.md)。两款语言模型的上下文/输出上限和推理档位与现有原厂配置一致，无需改变 Kimi Model Driver 或扩展其他语言渠道的真实调用。

本轮通过 Zone Gateway 临时加载源码 metadata，T1.5 连接高保真 Mock，T2 连接明确选定的真实 Provider。每轮恢复 metadata 原始字节，runner 同时恢复 AICC settings、库存与测试资源。临时加载 metadata 本身无需重启；目录解析修复构建后由用户完成了一次服务重启，未改动 NDN/Gateway 启动配置。新 AICC 二进制已包含源码内置 metadata；本地或云端更高优先级覆盖仍按原规则生效。


标准 Doubao 渠道 T2 在推理前发现库存缺失：官方数据面 `GET /api/v3/models` 使用 `text_to_image` / `image_to_image` 等 snake_case `task_type`，原专用 parser 使用了控制面 FoundationModelTag.TaskTypes 的枚举名称。修复只调整 `discover_volcengine_ark_models` 的私有目录解析；控制面枚举不再用于数据面。独立官方目录快照见 `test/aicc_test/acceptance/fixtures/doubao-models.json`，Mock 与独立能力基线同步更新。T1.5 根据已有 `T15_PROVIDER_DISCOVERY_CONTRACTS` 选择 machine API / catalog-only；标准渠道不再通过注入 discovery 跳过目录解析。公共 lowering 文件仅增加回归单测，公共产品逻辑没有改动。


2026-11-16 已完成的参数修复回归：

| 范围 | T1.5 | T2 |
| --- | --- | --- |
| Agent Plan 新增 3 个模型 | 新进程上 56/56 通过 | 8 个基础单元正常完成；2 项自动通过，6 项人工复核通过 |
| 标准 Doubao Flash（同一原厂模型） | 修复后真实发现路径 4/4 通过 | 2 个基础单元正常完成，视觉复核通过 |

Agent Plan 最终 T2 实际发出 9 次调用（8 单元加一次参考图前置生成），无自动重试；首轮失败诊断另发出 8 次调用。标准渠道另有 3 次调用，合计 20 次真实推理调用。所有轮次均未获服务端定价，不能按免费或 0 元解释；累计未知费用估算敞口为 2.00 USD。当前通过的语义 rubric 仅要求 OCR 包含 `4827`，完整字串仍有像素字体的字形误识别，不作为逐字精确识别通过。

本地证据目录统一位于 `test/aicc_test/reports/acceptance/agent-plan-new-20261116/`。参数修复后 T1.5：`t15-20261116032225-465139`（Agent Plan）、`t15-20261116032252-465698`（标准渠道旧发现准备路径）；Agent Plan 最终 T2：`aicc-2026-11-16T03-23-16-033Z-22cb4f72`。该 T2 的 `summary.md` 追加了人工复核结论，机器原始 2 passed / 6 review 状态保持不变；逐项复核和原始任务证据只保存在被忽略的本地明细中。


标准渠道目录修复：`cargo test -p aicc --lib volcengine_ark_discovery -- --test-threads=1` 与 `cargo build -p aicc` 已通过。新二进制已原子安装至 `/opt/buckyos/bin/aicc/aicc`，旧文件备份为 `/opt/buckyos/bin/aicc/aicc.before-agent-plan-20261116`；新 SHA256 为 `73a1369d8fdf546a0a4ecf210ece75bd79b70fd360c5dc9585f7ff2bf22afe48`。部署时 `sudo -n true` 返回 `sudo: a password is required`；用户随后按约定完成重启，新 AICC PID 为 480257。网关旧连接的端口占用导致短暂启动失败，约一分钟后自动恢复，未修改启动配置。修复后标准渠道 T1.5 通过真实目录发现路径，4/4 通过（`t15-20261116033612-482495`）；标准渠道 T2 随后通过两个基础图片单元（含视觉复核），运行 ID 为 `aicc-2026-11-16T03-37-05-215Z-d72865d4`。未执行完整 workspace 测试或系统全量构建。


最终新进程回归的 Agent Plan T1.5 为 `t15-20261116033636-483042`，56/56 通过；标准渠道 T1.5 为 `t15-20261116033612-482495`，4/4 通过。标准渠道 T2 报告位于 `test/aicc_test/reports/acceptance/agent-plan-flash-standard-20261116/aicc-2026-11-16T03-37-05-215Z-d72865d4/`。最终共 10 个基础 T2 单元满足当前 rubric，机器报告保留 2 passed / 8 review，8 项均已完成视觉或文本复核。两渠道 metadata、AICC settings、临时凭据、库存和测试资源清理检查通过；NDN 下载无需改配置或逻辑。`deno check acceptance/run_t15_gateway.ts acceptance/t15_mock_provider.ts`、preflight、自测 163 项及 `git diff --check` 均通过。


后续 PR #740 review 已发现 `/n` omit 不能保留多图请求语义。当前修正方案改为原厂 `max_images: 1`、公共 lowering 显式拒绝超限、豆包私有 codec 在单图时不发组图字段；原先的 omit 方案仅为历史调试记录。当前实现、公共组件修改必要性和本轮验证范围见 [PR #740 review 修复](pr740_review_fixes.md)。
