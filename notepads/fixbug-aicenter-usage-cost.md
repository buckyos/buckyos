# AICenter Usage 成本与用量修复（#755）

分支 `fix/aicenter-usage-cost-755` 从 `buckyos/main` 的 `6e3d91cbf` 创建；配套 websdk 同名分支从 `origin/main` 的 `b2a7e0f1b` 创建，配套提交 `c2f3e70fb`。

## 原因与修复

SN 缺少静态模型价格，部分实报金额未声明币种。按用户要求，统一采用 OpenRouter 报价，配置 52 个精确映射型号的 USD 价格（全模态/embedding/video 目录，原始字段快照随测试保存），实报成本优先、静态快照后备；仅对已核实的两个 Kimi 型号声明默认 USD，不外推到其他模型。

媒体价格要求的计费单位与适配器记录不一致：Gemini 图片缺生成张数、音乐缺请求单位，fal 音频时长和计算秒未传至结算，OpenAI Responses 图片计数及转写顶层时长未补全。修复计数及异步结果合并，保留实际零值、避免续调用重复累计，增加计算秒和百万像素结算。Omni 缺图片/视频模态费率，补齐并阻止用文本费率替代未知的视频用量。复用 addons 支持 token 与音频秒/请求数的组合计费，实报优先、全量用量校验及冻结语义保持一致。

Usage 仅有记录列表。服务器按同一筛选条件聚合所有分页，返回各计费单位、上报覆盖数和未计价条数；首页和 Usage 页展示 token、多币种金额以及可点击展开的单位明细，列表与近期记录也可展开；多币种默认展示 USD，点击展开实际各币种金额，不换汇。SDK、共享类型、协议 fixtures 和文档同步更新；按用户要求，不修改 Jarvis Agent 的 SKILL.md。没有新增依赖或服务。

详情、报价来源、历史记录恢复工具及限制见 `doc/aicc/usage_costs.md`。历史恢复工具默认只读，仅凭历史证据回填，写入时备份和审计；未对用户部署执行。

## 验证

- `cargo build -p aicc` 首轮通过；补充修改后运行 `cargo check -p aicc --all-targets`，AICC lib 单测：825 通过；异步/续调用合并额外定向回归：9 通过。
- buckyos-api lib：419 通过、3 忽略；workflow、llm_context、agent_tool、opendan all-targets 检查通过；llm_context 用量合并场景通过。
- Desktop：TypeScript、生产构建、相关 ESLint、6 项数据模型测试、17 项 AICenter Playwright 回归通过。
- AICC 离线 acceptance：167 通过；历史恢复 Python 测试：5 通过。
- websdk：18 项 AICC 客户端测试、TypeScript 检查、SDK/CLI/owner extensions 构建通过，生成分发文件同步。

没有运行真实 Zone Gateway T1.5、付费 Provider T2/T3、PostgreSQL 集成测试、完整 workspace 测试或安装部署。仅 `gpt-4o-mini-tts` 和 `gpt-4o-transcribe-diarize` 未在此次 OpenRouter 全模态目录收录而缺静态配置，详见文档；fal 视频放大未上报完整计费百万像素时也保持未知，不虚构价格或用量。线上历史账本未修改。

2026-11-22 补充核验：fal 官方总说明、RealESRGAN 模型比较及其他视频放大 endpoint 的明示公式构成旁证；最终应以同一 request_id 的 Billing Events 的 quantity/output_units/cost_total 和实际输出帧数对照。尚未读取账号账单或执行付费探测，不把其他模型公式套进当前结算。
