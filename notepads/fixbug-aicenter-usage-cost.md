# AICenter Usage 成本与用量修复（#755）

分支 `fix/aicenter-usage-cost-755` 从 `buckyos/main` 的 `6e3d91cbf` 创建；配套 websdk 同名分支从 `origin/main` 的 `b2a7e0f1b` 创建，配套提交 `c2f3e70fb`。

## 原因与修复

SN 缺少静态模型价格，部分实报金额未声明币种。新增 51 个可核实原厂 USD 价格，实报成本优先、静态快照后备；仅对已核实的两个 Kimi 型号声明默认 USD，不外推到其他模型。

媒体价格要求的计费单位与适配器记录不一致：Gemini 图片缺生成张数、音乐缺请求单位，fal 音频时长和计算秒未传至结算，OpenAI Responses 图片计数及转写顶层时长未补全。修复计数及异步结果合并，保留实际零值、避免续调用重复累计，增加计算秒和百万像素结算。Omni 缺图片/视频模态费率，补齐并阻止用文本费率替代未知的视频用量。

Usage 仅有记录列表。服务器按同一筛选条件聚合所有分页，返回各计费单位、上报覆盖数和未计价条数；前端按首页消费卡样式展示 token、多币种金额以及可点击展开的单位明细。SDK、共享类型、协议 fixtures、文档及四份内置管理/媒体手册同步更新。没有新增依赖或服务。

详情、报价来源、历史记录恢复工具及限制见 `doc/aicc/usage_costs.md`。历史恢复工具默认只读，仅凭历史证据回填，写入时备份和审计；未对用户部署执行。

## 验证

- `cargo build -p aicc` 通过；AICC lib 单测：821 通过；异步/续调用合并额外定向回归：9 通过。
- buckyos-api lib：419 通过、3 忽略；workflow、llm_context、agent_tool、opendan all-targets 检查通过；llm_context 用量合并场景通过。
- Desktop：TypeScript、生产构建、相关 ESLint、6 项数据模型测试、16 项 AICenter Playwright 回归通过。
- AICC 离线 acceptance：167 通过；历史恢复 Python 测试：4 通过。
- websdk：18 项 AICC 客户端测试、TypeScript 检查、SDK/CLI/owner extensions 构建通过，生成分发文件同步。

没有运行真实 Zone Gateway T1.5、付费 Provider T2/T3、PostgreSQL 集成测试、完整 workspace 测试或安装部署。三个没有可靠原厂 USD 报价的 SN 型号继续明确显示未知；fal 视频放大未上报完整计费百万像素时也保持未知，不虚构价格或用量。线上历史账本未修改。
