# Installer 与预装迁移：历史决策摘要

> 历史参考。合并旧 Installer 基础接口、v0.5 实现和 rootfs 预装三份清单；旧 checklist、伪代码与“当前实现”快照不再维护，也不自动转成新任务。

## 保留的设计背景

- 早期安装入口混用 App 标识、服务端 path、弱 target 和分阶段输入，后来转向受控 PIKG staging、服务端目标证据和冻结 InstallPlan，以避免用户批准内容与实际部署内容不同。
- 早期提出 Target/Params/Plan/Approval 分离，内容准备状态不混入稳定 identity；这些原则已经归入安装协议和职责文档，不在此再写第二套类型定义。
- 预装曾由 Scheduler bootstrap 拼装 AppDoc/InstallPlan，存在假 pkg_objid、空 required_contents 和以编译机平台选择包的问题；迁移理由是复用 Control Panel 的 staging/Installer/TaskManager，避免 boot 与文件安装耦合。
- 早期 Jarvis 的独立 bootstrap helper 是过渡方案；当前为 Agent 模板登记与构造 App，不能按旧清单恢复 bootstrap Agent 实例。

## 不再可用的旧结论

旧稿记录过 `apps.install/install_package`、旧 app_index 序列、AppDoc `id` 字段、用 resolver KV 证据作为 local developer 前置、Prepare 后分步写 spec/record 等阶段实现。它们不是当前 RPC/schema/事务规范。

旧完成记录中的 1872 个测试、特定 macOS 环境、历史 container 限制和 recovery 缺口仅说明当时的验证范围。本次没有重放这些测试，也没有因为旧 checklist 未勾选就判定能力未实现。

## 当前替代资料

- [App 安装协议](../../App%20安装协议.md)：身份、AppDoc、计划和 Scheduler CAS 提交。
- [App 生命周期](../../app_service/BuckyOS%20App基础设施职责边界.md)：各层职责及当前预装。
- [PIKG 格式](../format.md)：原 D1/D2 中仍有效的容器和内容校验。
- [Installer 测试](../../../test/app_installer_test/README.md)：当前测试入口和已知场景差异。

## 原始稿追溯

- [notepads/app-installer-base-interface-todo.md](https://github.com/buckyos/buckyos/blob/6ee9a4fb092d8d7e0fa73293249a69bc27f93d43/notepads/app-installer-base-interface-todo.md)
- [notepads/app-installer-v0.5-implementation-todo.md](https://github.com/buckyos/buckyos/blob/bb1994892c3134e53e025fd8bb17b520524a8b4f/notepads/app-installer-v0.5-implementation-todo.md)
- [notepads/preinstall-app-pikg-control-panel-todo.md](https://github.com/buckyos/buckyos/blob/3ee48781eec1ed5a8941b756a1f8d298e14d0803/notepads/preinstall-app-pikg-control-panel-todo.md)
