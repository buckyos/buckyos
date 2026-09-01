# AICC 文档目录

本目录是 AICC 模块文档的统一归档位置。

## 根目录文档

根目录文档用于描述 AICC 的需求、设计目标、协议契约、路由规则、Provider 方案、schema 和验收目标。后续开发需要判断设计约束或工程目标时，应优先阅读这些根目录文档。

主要入口：

- `aicc_requirements.md`：产品与功能需求。
- `AICC.md`：服务级设计总览。
- `aicc_api设计.md`：对外 API 设计。
- `aicc_router.md`：模型路由设计。
- `aicc 逻辑模型目录.md`：逻辑模型目录设计。
- `aicc_provider_plan.md`：Provider 实现方案。
- `aicc-models-mgr.md`：模型管理与路由概念设计。
- `aicc-models-todo.md`：模型管理历史实现记录，不作为 Beta 2.2 目标规范。
- `driver_metadata_update_protocol.md`：NDN 目标序列、Provider 已应用序列与 AICC 全局库存收敛契约。
- `driver_metadata_update_storage.md`：当前 metadata 文件、目标/已应用序列和 Provider inventory 的持久边界。
- `provider_profile_schema.md`：Provider Profile、Protocol Adapter、Provider Rules、Model Driver 和 Pricing 的目标边界与 schema。
- `match_rule.md`：Model Driver、Provider Rules、请求/价格条件及发布 track 共用的统一匹配语义，采用字符串优先、多维对象按需展开的配置形式。
- `internal_module_architecture.md`：AICC 重构后的内部模块职责、依赖方向、协议代际复用、运行时快照和生命周期边界。
- `provider_architecture_durable_data_schema.md`：Issue #579 新 Provider 架构的持久数据边界，定义三类 catalog、Provider Instance 外部真相源和实例级 inventory LKGS。
- `provider_ui_backend_mapping.md`：Provider catalog、Instance、inventory、trace 的前后端字段映射、状态和性能边界。
- `aicc-upgrade-todo.md`：新 API 与模型体系历史实现记录。
- `aicc改进.md`：AICC 改进方案记录。
- `aicc_log1.html`：AICC 设计讨论和历史记录。

## 维护参考文档

`maintenance/` 存放操作指南、实现备忘、当前代码总结、TODO 和历史日志。这些文档来自需求、方案和具体实现代码的整理，适合用于理解和维护当前实现。

`maintenance/` 下的文档不是设计目标，也不应单独作为后续开发的约束来源。如果维护参考与根目录需求/设计文档或当前代码冲突，以根目录文档和当前代码为准，并按需更新维护参考。
