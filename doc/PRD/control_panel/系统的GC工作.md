# 系统的GC工作

> Migration note:
> - Canonical docs now live under `doc/control_panel/`.
> - Storage lifecycle intent is being merged into `doc/control_panel/SPEC.context.md` and `doc/control_panel/CONTEXT.context.md`.
> - This file is retained as historical PRD input during migration.

这是一个早期需求便签，现已降级为 historical stub。

## Historical Intent

- 系统数据应被统一分类，而不是散落成不可见的磁盘占用。
- 用户应能看见不同类别数据的大小与来源。
- 存储管理器应同时支持自动清理和手动清理。

## Canonical Destination

- 生命周期与可见性规则：`doc/control_panel/SPEC.context.md`
- 约束、边界与迁移说明：`doc/control_panel/CONTEXT.context.md`

后续如果存储生命周期进入正式规格，应在 canonical 文档中展开为数据分类、可见性、保留策略、自动回收、手动回收几个子章节，而不是继续在这里扩写。
