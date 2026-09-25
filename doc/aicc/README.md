# AICC 文档目录

本目录是 AICC 模块文档的统一归档位置。

## Beta 2.2 冻结基线

以下四份文档是当前实现的冻结设计入口。后续开发涉及公共协议、内部边界、模型目录或 Provider 接入时，应先以它们为准；更长的专题文档用于字段细节、背景和验收补充。

- `frozen_trait_protocol.md`：公共 kRPC、内部 trait、Protocol Adapter、错误与任务语义。
- `frozen_model_driver_and_logical_model_fs.md`：Model Driver、逻辑模型虚拟 FS、来源优先级、持久化与 overlay。
- `frozen_provider_implementation.md`：Provider 分解、装配、discovery、inventory、凭据与扩展流程。
- `frozen_user_api.md`：面向应用、Agent、UI 和运维工具的调用接口与使用约束。

2026-09-25 LLM 设计修订：按厂商声明规格，以官方模型 ID、能力及固定思考预设为 metadata 主体，动态生成 `功能 -> 规格 -> 家族:预设 -> 物理 instance`。Review、边界见 [Model Driver 冻结设计 §3.3](frozen_model_driver_and_logical_model_fs.md#33-llm-厂商规格与模型家族目标)，字段和 OpenAI 示例见 [Metadata 目标契约](driver_metadata_schema.md#llm-target-contract-vendor-specifications-and-model-families)。OpenAI builtin JSON 已改为 v2 审阅稿，已归规格的模型省略 `logical_mounts`，思考档位从 `supported_efforts` 派生，不再重复声明 `variants`；parser、参数转换和路由实现仍待 Review 确认后修改。

同日价格边界修订：全部 builtin Model Driver 已移除 `model_pricing`。价格只来自 Provider discovery、响应或 Provider Rules；缺失时保持 unknown，不使用原厂默认价兜底。见 [价格优先级](provider_profile_schema.md#7-价格优先级) 和 [价格事实源](provider_pricing_sources.md)。本轮仅更新配置和文档，运行时价格兜底的删除仍待 Review 后实现。

## 根目录文档

根目录文档用于描述 AICC 的需求、设计目标、协议契约、路由规则、Provider 方案、schema 和验收目标。后续开发需要判断设计约束或工程目标时，应优先阅读这些根目录文档。

主要入口：

- `aicc_requirements.md`：产品与功能需求。
- `AICC.md`：服务级设计总览。
- `aicc_reimplementation_roadmap.md`：Beta 2.2 完全重建的模块 TODO、并行工作流、实施波次和 T1/T1.5/T2/T3 验收路线图。
- `aicc_api设计.md`：对外 API 设计。
- `aicc_router.md`：模型路由设计。
- `aicc_e2e_test_requirements.md`：T1 路由、T1.5 Provider 官方协议契约、T2 线上推理和 T3 消息链路的测试分层与验收要求。
- `aicc 逻辑模型目录.md`：逻辑模型目录设计。
- `aicc_provider_plan.md`：Provider 实现方案。
- `aicc-models-mgr.md`：模型管理与路由概念设计。
- `driver_metadata_update_protocol.md`：NDN 目标序列、Provider 已应用序列与 AICC 全局库存收敛契约。
- `driver_metadata_update_storage.md`：当前 metadata 文件、目标/已应用序列和 Provider inventory 的持久边界。
- `provider_profile_schema.md`：Provider Profile、Protocol Adapter、Provider Rules、Model Driver 和 Pricing 的目标边界与 schema。
- `match_rule.md`：Model Driver、Provider Rules、请求/价格条件及发布 track 共用的统一匹配语义，采用字符串优先、多维对象按需展开的配置形式。
- `driver_metadata_schema.md`：当前 Model Driver v1 字段，以及厂商规格、模型家族与固定预设的 v2 目标契约和 OpenAI 配置示例。
- `internal_module_architecture.md`：AICC 重构后的内部模块职责、依赖方向、协议代际复用、运行时快照和生命周期边界。
- `provider_architecture_durable_data_schema.md`：Issue #579 新 Provider 架构的持久数据边界，定义三类 catalog、Provider Instance 外部真相源和实例级 inventory LKGS。
- `aicc_runtime_durable_data_schema.md`：AICC 运行时持久记录，定义幂等/重启恢复 execution、route trace、session exact-model 历史、artifact 租户归属和 audit 表。
- `provider_ui_backend_mapping.md`：Provider catalog、Instance、inventory、trace 的前后端字段映射、状态和性能边界。
- `aicc改进.md`：AICC 改进方案记录。
- `aicc_log1.html`：AICC 设计讨论和历史记录。

## 维护参考文档

`maintenance/` 存放操作指南、实现备忘、当前代码总结、TODO 和历史日志。这些文档来自需求、方案和具体实现代码的整理，适合用于理解和维护当前实现。

`maintenance/` 下的文档不是设计目标，也不应单独作为后续开发的约束来源。如果维护参考与根目录需求/设计文档或当前代码冲突，以根目录文档和当前代码为准，并按需更新维护参考。

## 历史归档

`archive/` 保存已经结束的实现记录、迁移方案和历史 TODO，不属于当前规范或验收依据。
