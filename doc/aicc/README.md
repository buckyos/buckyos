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
- `provider_protocol_and_model_catalog.md`：Provider API 协议、模型目录模板与实例配置的解耦设计。
- `aicc-models-mgr.md`：模型管理与路由概念设计。
- `aicc-models-todo.md`：模型管理后续设计任务。
- `driver_metadata_update_protocol.md`：provider-driver metadata 的 NDN 增量更新、验证、LKGS 与生效协议。
- `driver_metadata_update_storage.md`：更新水位、不可变对象和 activation 的持久化兼容格式。
- `aicc-upgrade-todo.md`：新 API 与模型体系升级设计任务。
- `aicc改进.md`：AICC 改进方案记录。
- `provider-driver-cloud-update-design.md`：provider-driver metadata 云更新详细设计。
- `aicc_log1.html`：AICC 设计讨论和历史记录。

## 维护参考文档

`maintenance/` 存放操作指南、实现备忘、当前代码总结、TODO 和历史日志。这些文档来自需求、方案和具体实现代码的整理，适合用于理解和维护当前实现。

`maintenance/` 下的文档不是设计目标，也不应单独作为后续开发的约束来源。如果维护参考与根目录需求/设计文档或当前代码冲突，以根目录文档和当前代码为准，并按需更新维护参考。
