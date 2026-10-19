# PIKG 历史参考与合并记录

> 历史参考；这里不是当前协议、命令或执行计划。重复的原 TODO 已从 notepads 删除，保留其设计理由、未收敛议题和 Git 原文入口，避免继续维护多份规范。

## 当前权威文档

当前格式、命令、安装协议、职责、配置和构建的所有权见 [PIKG 总览](../README.md)。阅读本目录不能推导某个能力已实现或仍未实现，必须回到对应权威文档和源码。

## 原文与归并去向

| 原文件 | 现行内容的归属 | 历史摘要 |
| --- | --- | --- |
| `notepads/app-installer-base-interface-todo.md` | [安装协议](../../App%20安装协议.md) / [生命周期职责](../../app_service/BuckyOS%20App基础设施职责边界.md) | [installer-and-preinstall.md](installer-and-preinstall.md) |
| `notepads/app-installer-v0.5-implementation-todo.md` | [安装协议](../../App%20安装协议.md) / [生命周期职责](../../app_service/BuckyOS%20App基础设施职责边界.md) | [installer-and-preinstall.md](installer-and-preinstall.md) |
| `notepads/preinstall-app-pikg-control-panel-todo.md` | [安装协议](../../App%20安装协议.md) / [生命周期职责](../../app_service/BuckyOS%20App基础设施职责边界.md) | [installer-and-preinstall.md](installer-and-preinstall.md) |
| `notepads/buckyos-sdk-tool-distribution-todo.md` | [SDK 分发](https://github.com/buckyos/buckyos-websdk/blob/main/doc/distribution.md) / [Module 集成](../../CI/module_build_type_and_format.md) | [tool-and-build-migration.md](tool-and-build-migration.md) |
| `notepads/service_config_tips + permissions 实用配置指南.md` | [App 配置](../../sdk/app-package-config.md) / [生命周期职责](../../app_service/BuckyOS%20App基础设施职责边界.md) | [app-config-and-publication.md](app-config-and-publication.md) |
| `doc/opendan/Agent元能力/Agent Skill.md` | [OpenDAN 指南](../../sdk/OpenDAN_Agent_Dev_Guide.md) / [PIKG CLI](https://github.com/buckyos/buckyos-websdk/blob/main/doc/modules/pikg.md) | [agent-skills-design.md](agent-skills-design.md) |

旧生命周期文档与 Module 设计文档保留原路径，正文已改写为当前职责/集成规则；其中迁移、签名发布和计划验收内容分别归入上表对应摘要。旧 Installer README 的日期记录独立存为 [2026-08 DV 记录](installer-dv-2026-08.md)。

## 保留方式

原稿全文通过各摘要的 Git commit 链接追溯；当前树只保留必要历史结论，不再复制旧命令、完整 schema、重复 checklist 或已迁移目录。历史测试数字和环境结果不向当前版本继承。
