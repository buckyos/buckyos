# PIKG 文档总览与权威边界

> 当前入口。PIKG 文档按字段/职责确定唯一维护位置；旧清单已合并清理，完整处理记录见 [历史参考](history/README.md)。本页提供导航，不再重复格式、schema、安装步骤或发布状态机。

## 1. 当前权威文档

| 领域 | 唯一维护文档 | 不在其他文档重复维护 |
| --- | --- | --- |
| CLI、开发态输入与快照 | [WebSDK PIKG CLI](https://github.com/buckyos/buckyos-websdk/blob/main/doc/modules/pikg.md) | 六个命令、app.json/pikg.json、dapp_dist、构建/清理 |
| 包内容格式与 reader 层次 | [PIKG 格式](format.md) | ZIP/ZIP64、PACKAGE_META/content index、payload identity、验签限制 |
| 普通 App 安装协议 | [App 安装协议](../App%20安装协议.md) | AppDID/AppDoc、Object ID、InstallPlan、提交/恢复与投影 |
| App CLI 行为 | [WebSDK App 模块](https://github.com/buckyos/buckyos-websdk/blob/main/doc/modules/app.md) | 来源、首装、本地/Catalog 升级、Repo candidate |
| 跨组件职责与预装 | [App 生命周期](../app_service/BuckyOS%20App基础设施职责边界.md) | CLI/Repo/Installer/Scheduler/node-daemon/Agent 管理边界 |
| 应用配置声明 | [App 包配置](../sdk/app-package-config.md) | permissions、service_config_tips 的当前形状与消费边界 |
| 系统构建集成 | [Module type/format](../CI/module_build_type_and_format.md) | builder/formatter、PIKG rootfs 落位 |
| SDK/Tool 分发 | [WebSDK distribution](https://github.com/buckyos/buckyos-websdk/blob/main/doc/distribution.md) | npm 与系统入口、runtime、产物完整性和更新归属 |
| Agent skill 生命周期 | [OpenDAN 开发指南](../sdk/OpenDAN_Agent_Dev_Guide.md) | cmd_skills/goal_skills、安装接口、Session 冻结 |

“当前权威”表示该领域的文档维护位置，不能覆盖实现证据。若规范与代码冲突，应在该文档明确差异并修正，不能从历史方案补出一项尚未交付的能力。尤其 `info` 内容通过、Owner 签名、BNS 当前发布和实际运行成功是不同证据。

## 2. 教程与样例

- 开发普通 App：[快速入门](../sdk/app-dev-quickstart.md)，结合 [Installer 样例](../../test/app_installer_test/pikg_samples/README.md)。
- 开发 Agent：[Agent 手册](../../src/apps/jarvis_agent/agent/cmd_skills/opendan_agent_dev.md) 与 [Jarvis](../../src/apps/jarvis_agent/README.md)。
- 独立 skill 包：[最小样例](../../test/skills_pikg_sample/README.md)。
- 运行安装测试：[Installer DV](../../test/app_installer_test/README.md)。带日期旧结果已移出当前步骤。
- 系统与企业部署：[企业部署指南](../sdk/buckyos-deploy.md)。行业项目专用配置与操作说明留在所属业务仓库。

教程、样例与 Agent 命令手册只维护各自场景，不复制完整 schema；变更 PIKG 行为时同步检查 `buckyos_app_dev/buckyos_dev_release/opendan_agent_dev`。

## 3. 仅供历史或设计参考

- [历史摘要与原稿映射](history/README.md)：旧 Installer/预装/Tool 清单、权限配置目标稿、Skills Mgr 研究和旧 DV 结果。
- `notepads/`：任务记录，可含已完成与未完成项，不是当前协议；不能因旧 checkbox 未勾选就判断实现状态。
- `product/`、讨论稿及未交付设计：说明产品意图，不作为已存在命令/字段的依据。
- 架构与 UI 的周边文档：各自负责相邻领域；PIKG 字段发生冲突时回到上表指定文档。

原 TODO 全文通过历史摘要的 Git 原文链接保留，当前树不保留重复规范或失效执行清单。历史文档中出现的旧错误码、目录、RPC 和测试数字不向当前版本继承。

## 4. 相关资料目录

下列目录收录受版本控制的相关文本和本次新增文件。这里只做用途分类，文件提到 PIKG 不代表它是 PIKG 规范。

### 协议、集成与关联指南

- [doc/App 安装协议.md](../../doc/App%20%E5%AE%89%E8%A3%85%E5%8D%8F%E8%AE%AE.md)
- [doc/CI/module_build_type_and_format.md](../../doc/CI/module_build_type_and_format.md)
- [doc/app_service/BuckyOS App基础设施职责边界.md](../../doc/app_service/BuckyOS%20App%E5%9F%BA%E7%A1%80%E8%AE%BE%E6%96%BD%E8%81%8C%E8%B4%A3%E8%BE%B9%E7%95%8C.md)
- [doc/arch/10_user_lifecycle_and_permissions.md](../../doc/arch/10_user_lifecycle_and_permissions.md)
- [doc/arch/beta2.2-data-compatibility.md](../../doc/arch/beta2.2-data-compatibility.md)
- [doc/arch/system_config_reference.md](../../doc/arch/system_config_reference.md)
- [doc/control_panel/Control_Panel_Service.md](../../doc/control_panel/Control_Panel_Service.md)
- [doc/opendan/Agent元能力/基于Agent元能力的Agent Runtime设计.md](../../doc/opendan/Agent%E5%85%83%E8%83%BD%E5%8A%9B/%E5%9F%BA%E4%BA%8EAgent%E5%85%83%E8%83%BD%E5%8A%9B%E7%9A%84Agent%20Runtime%E8%AE%BE%E8%AE%A1.md)
- [doc/pikg/format.md](../../doc/pikg/format.md)
- [doc/sdk/OpenDAN_Agent_Dev_Guide.md](../../doc/sdk/OpenDAN_Agent_Dev_Guide.md)
- [doc/sdk/app-dev-quickstart.md](../../doc/sdk/app-dev-quickstart.md)
- [doc/sdk/app-package-config.md](../../doc/sdk/app-package-config.md)
- [doc/sdk/buckyos-deploy.md](../../doc/sdk/buckyos-deploy.md)
- [doc/sdk/context ext.md](../../doc/sdk/context%20ext.md)

### 应用、Agent 手册与样例

- [src/apps/jarvis_agent/README.md](../../src/apps/jarvis_agent/README.md)
- [src/apps/jarvis_agent/agent/cmd_skills/buckyos_app_dev.md](../../src/apps/jarvis_agent/agent/cmd_skills/buckyos_app_dev.md)
- [src/apps/jarvis_agent/agent/cmd_skills/buckyos_dev_release.md](../../src/apps/jarvis_agent/agent/cmd_skills/buckyos_dev_release.md)
- [src/apps/jarvis_agent/agent/cmd_skills/opendan_agent_dev.md](../../src/apps/jarvis_agent/agent/cmd_skills/opendan_agent_dev.md)
- [src/apps/jarvis_agent/agent/cmd_skills/ui_core.md](../../src/apps/jarvis_agent/agent/cmd_skills/ui_core.md)
- [src/apps/sys_test/README.md](../../src/apps/sys_test/README.md)
- [src/apps/text_editor/readme.md](../../src/apps/text_editor/readme.md)
- [src/frame/opendan/README.md](../../src/frame/opendan/README.md)
- [test/app_installer_test/README.md](../../test/app_installer_test/README.md)
- [test/app_installer_test/pikg_samples/README.md](../../test/app_installer_test/pikg_samples/README.md)
- [test/app_installer_test/pikg_samples/agent/agent/dist/prompts/self.md](../../test/app_installer_test/pikg_samples/agent/agent/dist/prompts/self.md)
- [test/app_installer_test/pikg_samples/content-open-handler/README.md](../../test/app_installer_test/pikg_samples/content-open-handler/README.md)
- [test/skills_pikg_sample/README.md](../../test/skills_pikg_sample/README.md)
- [test/skills_pikg_sample/skills/cmd_skills/hello.md](../../test/skills_pikg_sample/skills/cmd_skills/hello.md)

### 产品、交互与讨论设计

- [doc/opendan/Agent配置改进.md](../../doc/opendan/Agent%E9%85%8D%E7%BD%AE%E6%94%B9%E8%BF%9B.md)
- [doc/control_panel/BuckyOS 内容分享，发布，上线功能讨论记录.md](../../doc/control_panel/BuckyOS%20%E5%86%85%E5%AE%B9%E5%88%86%E4%BA%AB%EF%BC%8C%E5%8F%91%E5%B8%83%EF%BC%8C%E4%B8%8A%E7%BA%BF%E5%8A%9F%E8%83%BD%E8%AE%A8%E8%AE%BA%E8%AE%B0%E5%BD%95.md)
- [doc/control_panel/BuckyOS 的分享_发布_上线 功能的产品设计.md](../../doc/control_panel/BuckyOS%20%E7%9A%84%E5%88%86%E4%BA%AB_%E5%8F%91%E5%B8%83_%E4%B8%8A%E7%BA%BF%20%E5%8A%9F%E8%83%BD%E7%9A%84%E4%BA%A7%E5%93%81%E8%AE%BE%E8%AE%A1.md)
- [product/BuckyOS Sys Dlg.md](../../product/BuckyOS%20Sys%20Dlg.md)
- [product/app_service/BuckyOS_App_Service_PRD.md](../../product/app_service/BuckyOS_App_Service_PRD.md)
- [product/control_panel/ShareContentMgr_PRD.md](../../product/control_panel/ShareContentMgr_PRD.md)
- [product/users_and_agents/BuckyOS Agent的初始化流程PRD.md](../../product/users_and_agents/BuckyOS%20Agent%E7%9A%84%E5%88%9D%E5%A7%8B%E5%8C%96%E6%B5%81%E7%A8%8BPRD.md)
- [src/frame/desktop/src/app/app-service/UI_DATAMODEL.md](../../src/frame/desktop/src/app/app-service/UI_DATAMODEL.md)

### 任务记录与旧工作背景

- [notepads/app-id-simplification-todo.md](../../notepads/app-id-simplification-todo.md)
- [notepads/app-service-beta2.2-prototype-todo.md](../../notepads/app-service-beta2.2-prototype-todo.md)
- [notepads/buckyos-tool-app-control-panel-todo.md](../../notepads/buckyos-tool-app-control-panel-todo.md)
- [notepads/opendan-builtin-cmd-skills-todo.md](../../notepads/opendan-builtin-cmd-skills-todo.md)
- [notepads/opendan-cmd-skill-todo.md](../../notepads/opendan-cmd-skill-todo.md)
- [notepads/opendan-goal-session-todo.md](../../notepads/opendan-goal-session-todo.md)
- [notepads/share-content-mgr-tech-design-todo.md](../../notepads/share-content-mgr-tech-design-todo.md)

### 历史参考

- [doc/pikg/history/README.md](../../doc/pikg/history/README.md)
- [doc/pikg/history/agent-skills-design.md](../../doc/pikg/history/agent-skills-design.md)
- [doc/pikg/history/app-config-and-publication.md](../../doc/pikg/history/app-config-and-publication.md)
- [doc/pikg/history/installer-and-preinstall.md](../../doc/pikg/history/installer-and-preinstall.md)
- [doc/pikg/history/installer-dv-2026-08.md](../../doc/pikg/history/installer-dv-2026-08.md)
- [doc/pikg/history/tool-and-build-migration.md](../../doc/pikg/history/tool-and-build-migration.md)

### 仓库入口与协作规则

- [.impeccable.md](../../.impeccable.md)
- [AGENTS.md](../../AGENTS.md)
- [README.md](../../README.md)
- [README_zhCN.md](../../README_zhCN.md)

### WebSDK

- [README.md](https://github.com/buckyos/buckyos-websdk/blob/main/README.md)
- [cli/README.md](https://github.com/buckyos/buckyos-websdk/blob/main/cli/README.md)
- [doc/buckycli-coverage.md](https://github.com/buckyos/buckyos-websdk/blob/main/doc/buckycli-coverage.md)
- [doc/buckyos tool PRD.md](https://github.com/buckyos/buckyos-websdk/blob/main/doc/buckyos%20tool%20PRD.md)
- [doc/distribution.md](https://github.com/buckyos/buckyos-websdk/blob/main/doc/distribution.md)
- [doc/modules/README.md](https://github.com/buckyos/buckyos-websdk/blob/main/doc/modules/README.md)
- [doc/modules/app.md](https://github.com/buckyos/buckyos-websdk/blob/main/doc/modules/app.md)
- [doc/modules/pikg.md](https://github.com/buckyos/buckyos-websdk/blob/main/doc/modules/pikg.md)
- [doc/modules/provision.md](https://github.com/buckyos/buckyos-websdk/blob/main/doc/modules/provision.md)

检索也覆盖 buckyos-base、cyfs-gateway、sn-business 的受管 Markdown/reStructuredText/txt，未发现直接提及 PIKG 的资料；通用 Package/NDN/BNS 资料仍由所属模块维护。
