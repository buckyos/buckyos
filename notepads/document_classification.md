# BuckyOS 文档分类整理

基于 [notepads/Harness Engineering.md](/Users/liuzhicong/project/buckyos/notepads/Harness%20Engineering.md) 的约束，当前文档整理应遵循 3 个原则：

1. 先把只给 AI/Agent 使用的规则文档，与给人阅读的正式文档分开。
2. 再把正式文档区分为：工程实现文档、项目介绍/产品规划文档、教程文档。
3. 对同时混合“愿景 + 规格 + 实现现状 + 操作说明”的文档做拆分，避免上游文档误导后续 AI 和贡献者。

当前仓库沿用 `doc/` 与 `notepads/` 两套目录。若按 Harness Engineering 的建议目录映射，可近似理解为：

- `doc/` 对应建议中的 `/docs`
- AI 规则、Prompt、Agent 行为文档未来应逐步收敛到类似 `/harness/rules` 和 `/harness/prompts`

## 1. 传统的、主要给 AI 看的文档

这类文档的特点是：定义 Agent 行为、提示词规则、模块上下文、工作约束、技能规则，主要不是面向普通读者。

### 1.1 仓库级 AI 规则文档

- [AGENTS.md](/Users/liuzhicong/project/buckyos/AGENTS.md)
- [.claude/settings.local.json](/Users/liuzhicong/project/buckyos/.claude/settings.local.json)
- [SKILLS/krpc/SKILL.md](/Users/liuzhicong/project/buckyos/SKILLS/krpc/SKILL.md)
- [src/frame/control_panel/SKILL.md](/Users/liuzhicong/project/buckyos/src/frame/control_panel/SKILL.md)

### 1.2 模块级 AI 上下文文档

这些文档虽然也给人看，但其写法已经明显接近 “AI 可消费的 canonical context/spec”。

- [doc/control_panel/README.context.md](/Users/liuzhicong/project/buckyos/doc/control_panel/README.context.md)
- [doc/control_panel/ARCHITECTURE.context.md](/Users/liuzhicong/project/buckyos/doc/control_panel/ARCHITECTURE.context.md)
- [doc/control_panel/SPEC.context.md](/Users/liuzhicong/project/buckyos/doc/control_panel/SPEC.context.md)
- [doc/control_panel/CONTEXT.context.md](/Users/liuzhicong/project/buckyos/doc/control_panel/CONTEXT.context.md)
- [doc/message_hub/README.context.md](/Users/liuzhicong/project/buckyos/doc/message_hub/README.context.md)
- [doc/message_hub/ARCHITECTURE.context.md](/Users/liuzhicong/project/buckyos/doc/message_hub/ARCHITECTURE.context.md)
- [doc/message_hub/SPEC.context.md](/Users/liuzhicong/project/buckyos/doc/message_hub/SPEC.context.md)
- [doc/message_hub/CONTEXT.context.md](/Users/liuzhicong/project/buckyos/doc/message_hub/CONTEXT.context.md)

### 1.3 `notepads/` 中明显属于 Agent/Harness/Prompt 规则的文档

- [notepads/Harness Engineering.md](/Users/liuzhicong/project/buckyos/notepads/Harness%20Engineering.md)
- [notepads/Agent Behavior.md](/Users/liuzhicong/project/buckyos/notepads/Agent%20Behavior.md)
- [notepads/Agent Enviroment.md](/Users/liuzhicong/project/buckyos/notepads/Agent%20Enviroment.md)
- [notepads/Agent Memory v2.md](/Users/liuzhicong/project/buckyos/notepads/Agent%20Memory%20v2.md)
- [notepads/Agent Message.md](/Users/liuzhicong/project/buckyos/notepads/Agent%20Message.md)
- [notepads/Agent Prompt Compiler.md](/Users/liuzhicong/project/buckyos/notepads/Agent%20Prompt%20Compiler.md)
- [notepads/Agent Session.md](/Users/liuzhicong/project/buckyos/notepads/Agent%20Session.md)
- [notepads/Agent Skill.md](/Users/liuzhicong/project/buckyos/notepads/Agent%20Skill.md)
- [notepads/Agent TodoList.md](/Users/liuzhicong/project/buckyos/notepads/Agent%20TodoList.md)
- [notepads/Agent Worklog.md](/Users/liuzhicong/project/buckyos/notepads/Agent%20Worklog.md)
- [notepads/Agent workspace.md](/Users/liuzhicong/project/buckyos/notepads/Agent%20workspace.md)
- [notepads/Agent 协作.md](/Users/liuzhicong/project/buckyos/notepads/Agent%20%E5%8D%8F%E4%BD%9C.md)
- [notepads/Agent 意图引擎.md](/Users/liuzhicong/project/buckyos/notepads/Agent%20%E6%84%8F%E5%9B%BE%E5%BC%95%E6%93%8E.md)
- [notepads/Agent 持久化.md](/Users/liuzhicong/project/buckyos/notepads/Agent%20%E6%8C%81%E4%B9%85%E5%8C%96.md)
- [notepads/PDCA提示词需求.md](/Users/liuzhicong/project/buckyos/notepads/PDCA%E6%8F%90%E7%A4%BA%E8%AF%8D%E9%9C%80%E6%B1%82.md)
- [notepads/behavior 提示词编写逻辑.md](/Users/liuzhicong/project/buckyos/notepads/behavior%20%E6%8F%90%E7%A4%BA%E8%AF%8D%E7%BC%96%E5%86%99%E9%80%BB%E8%BE%91.md)
- [notepads/check_behavior_review.md](/Users/liuzhicong/project/buckyos/notepads/check_behavior_review.md)
- [notepads/do_behavior_review.md](/Users/liuzhicong/project/buckyos/notepads/do_behavior_review.md)
- [notepads/ref/Agent Thinking.md](/Users/liuzhicong/project/buckyos/notepads/ref/Agent%20Thinking.md)
- [notepads/ref/gpt5.2 jarvis.md](/Users/liuzhicong/project/buckyos/notepads/ref/gpt5.2%20jarvis.md)
- [notepads/ref/hephaestus.md](/Users/liuzhicong/project/buckyos/notepads/ref/hephaestus.md)
- [notepads/ref/oracle.md](/Users/liuzhicong/project/buckyos/notepads/ref/oracle.md)
- [notepads/ref/review.md](/Users/liuzhicong/project/buckyos/notepads/ref/review.md)

### 1.4 建议归档位置

- Agent 规则、行为、协作、工作流文档：建议归到未来的 `harness/rules`
- Prompt 设计、Prompt 审查、PDCA 提示词文档：建议归到未来的 `harness/prompts`
- 代码样例和脚本参考：建议和规则文档分开，归到 `harness/scripts` 或 `notepads/ref`

## 2. 工程实现相关文档

这类文档面向实现、架构、接口、运行机制、测试策略、模块边界。

### 2.1 `doc/` 中的工程实现文档

- [doc/arch/README.md](/Users/liuzhicong/project/buckyos/doc/arch/README.md) 及 `doc/arch/` 下大多数文档
- [doc/aicc/how_to_add_provider.md](/Users/liuzhicong/project/buckyos/doc/aicc/how_to_add_provider.md)
- [doc/aicc/krpc_aicc_calling_guide.md](/Users/liuzhicong/project/buckyos/doc/aicc/krpc_aicc_calling_guide.md)
- [doc/aicc/test_strategy_and_cases.md](/Users/liuzhicong/project/buckyos/doc/aicc/test_strategy_and_cases.md)
- [doc/aicc/update_aicc_settings_via_system_config.md](/Users/liuzhicong/project/buckyos/doc/aicc/update_aicc_settings_via_system_config.md)
- [doc/control_panel/README.context.md](/Users/liuzhicong/project/buckyos/doc/control_panel/README.context.md)
- [doc/control_panel/ARCHITECTURE.context.md](/Users/liuzhicong/project/buckyos/doc/control_panel/ARCHITECTURE.context.md)
- [doc/control_panel/SPEC.context.md](/Users/liuzhicong/project/buckyos/doc/control_panel/SPEC.context.md)
- [doc/control_panel/CONTEXT.context.md](/Users/liuzhicong/project/buckyos/doc/control_panel/CONTEXT.context.md)
- [doc/message_hub/README.context.md](/Users/liuzhicong/project/buckyos/doc/message_hub/README.context.md)
- [doc/message_hub/ARCHITECTURE.context.md](/Users/liuzhicong/project/buckyos/doc/message_hub/ARCHITECTURE.context.md)
- [doc/message_hub/SPEC.context.md](/Users/liuzhicong/project/buckyos/doc/message_hub/SPEC.context.md)
- [doc/message_hub/CONTEXT.context.md](/Users/liuzhicong/project/buckyos/doc/message_hub/CONTEXT.context.md)
- [doc/opendan/Agent RootFS.md](/Users/liuzhicong/project/buckyos/doc/opendan/Agent%20RootFS.md)
- [doc/opendan/Container_Dependencies.md](/Users/liuzhicong/project/buckyos/doc/opendan/Container_Dependencies.md)
- [doc/sdk/buckyos-api-runtime.md](/Users/liuzhicong/project/buckyos/doc/sdk/buckyos-api-runtime.md)
- [doc/key data.md](/Users/liuzhicong/project/buckyos/doc/key%20data.md)
- [doc/key url.md](/Users/liuzhicong/project/buckyos/doc/key%20url.md)
- [doc/path_usage.md](/Users/liuzhicong/project/buckyos/doc/path_usage.md)
- [doc/port_usage.txt](/Users/liuzhicong/project/buckyos/doc/port_usage.txt)
- [doc/rust的一些基础规范.md](/Users/liuzhicong/project/buckyos/doc/rust%E7%9A%84%E4%B8%80%E4%BA%9B%E5%9F%BA%E7%A1%80%E8%A7%84%E8%8C%83.md)
- [doc/system_events.md](/Users/liuzhicong/project/buckyos/doc/system_events.md)
- [doc/test plan.md](/Users/liuzhicong/project/buckyos/doc/test%20plan.md)

### 2.2 `notepads/` 中的工程实现草稿或设计说明

- [notepads/AICC.md](/Users/liuzhicong/project/buckyos/notepads/AICC.md)
- [notepads/BuckyOS App安装流程.md](/Users/liuzhicong/project/buckyos/notepads/BuckyOS%20App%E5%AE%89%E8%A3%85%E6%B5%81%E7%A8%8B.md)
- [notepads/CI 系统整理.md](/Users/liuzhicong/project/buckyos/notepads/CI%20%E7%B3%BB%E7%BB%9F%E6%95%B4%E7%90%86.md)
- [notepads/Contact Mgr.md](/Users/liuzhicong/project/buckyos/notepads/Contact%20Mgr.md)
- [notepads/GitActionv2.md](/Users/liuzhicong/project/buckyos/notepads/GitActionv2.md)
- [notepads/Message Center.md](/Users/liuzhicong/project/buckyos/notepads/Message%20Center.md)
- [notepads/OpenDAN Agent Runtime 设计.md](/Users/liuzhicong/project/buckyos/notepads/OpenDAN%20Agent%20Runtime%20%E8%AE%BE%E8%AE%A1.md)
- [notepads/OpenDAN DevTools.md](/Users/liuzhicong/project/buckyos/notepads/OpenDAN%20DevTools.md)
- [notepads/OpenDAN Loader.md](/Users/liuzhicong/project/buckyos/notepads/OpenDAN%20Loader.md)
- [notepads/Repo v2.md](/Users/liuzhicong/project/buckyos/notepads/Repo%20v2.md)
- [notepads/ShareContentMgr.md](/Users/liuzhicong/project/buckyos/notepads/ShareContentMgr.md)
- [notepads/api-runtime init.md](/Users/liuzhicong/project/buckyos/notepads/api-runtime%20init.md)
- [notepads/app安装协议.md](/Users/liuzhicong/project/buckyos/notepads/app%E5%AE%89%E8%A3%85%E5%8D%8F%E8%AE%AE.md)
- [notepads/internet级别的OLTP.md](/Users/liuzhicong/project/buckyos/notepads/internet%E7%BA%A7%E5%88%AB%E7%9A%84OLTP.md)
- [notepads/opendan_tools.md](/Users/liuzhicong/project/buckyos/notepads/opendan_tools.md)
- [notepads/opendanv2.md](/Users/liuzhicong/project/buckyos/notepads/opendanv2.md)
- [notepads/opendan关键类型.md](/Users/liuzhicong/project/buckyos/notepads/opendan%E5%85%B3%E9%94%AE%E7%B1%BB%E5%9E%8B.md)
- [notepads/publish_app_to_repo_local_dir格式说明.md](/Users/liuzhicong/project/buckyos/notepads/publish_app_to_repo_local_dir%E6%A0%BC%E5%BC%8F%E8%AF%B4%E6%98%8E.md)
- [notepads/todo_cli_design.md](/Users/liuzhicong/project/buckyos/notepads/todo_cli_design.md)
- [notepads/todo_manage_review.md](/Users/liuzhicong/project/buckyos/notepads/todo_manage_review.md)
- [notepads/各种login整理.md](/Users/liuzhicong/project/buckyos/notepads/%E5%90%84%E7%A7%8Dlogin%E6%95%B4%E7%90%86.md)

### 2.3 建议定位

- `doc/arch/`、`doc/control_panel/`、`doc/message_hub/` 更适合作为长期 canonical engineering docs
- `notepads/` 里的工程文档更适合作为设计草稿、迁移输入或历史思考，不适合直接当最终规格

## 3. 项目介绍类文档 / 产品规划文档

这类文档主要回答“BuckyOS 是什么、想解决什么问题、面向哪些用户、产品要往哪里演进”。

### 3.1 `doc/` 中的项目介绍、PRD、规划类文档

- [doc/BuckyOS 完整规划.md](/Users/liuzhicong/project/buckyos/doc/BuckyOS%20%E5%AE%8C%E6%95%B4%E8%A7%84%E5%88%92.md)
- `doc/PRD/` 下大多数 Markdown 文档
- [doc/PRD/control_panel/README.md](/Users/liuzhicong/project/buckyos/doc/PRD/control_panel/README.md)
- [doc/PRD/control_panel/control_panel.md](/Users/liuzhicong/project/buckyos/doc/PRD/control_panel/control_panel.md)
- [doc/PRD/control_panel/SSO.md](/Users/liuzhicong/project/buckyos/doc/PRD/control_panel/SSO.md)
- [doc/PRD/control_panel/app安装UI.md](/Users/liuzhicong/project/buckyos/doc/PRD/control_panel/app%E5%AE%89%E8%A3%85UI.md)
- [doc/PRD/control_panel/系统的GC工作.md](/Users/liuzhicong/project/buckyos/doc/PRD/control_panel/%E7%B3%BB%E7%BB%9F%E7%9A%84GC%E5%B7%A5%E4%BD%9C.md)
- [doc/PRD/store/Content_Store_PRD_MVP.md](/Users/liuzhicong/project/buckyos/doc/PRD/store/Content_Store_PRD_MVP.md)
- [doc/PRD/filebrowser/filebrowser_PRD.md](/Users/liuzhicong/project/buckyos/doc/PRD/filebrowser/filebrowser_PRD.md)
- [doc/PRD/bucky_file/bucky_file_plan.md](/Users/liuzhicong/project/buckyos/doc/PRD/bucky_file/bucky_file_plan.md)
- [doc/PRD/buckycli/buckycli_v2.md](/Users/liuzhicong/project/buckyos/doc/PRD/buckycli/buckycli_v2.md)
- [doc/PRD/zh_CN/1 Active BuckyOS.md](/Users/liuzhicong/project/buckyos/doc/PRD/zh_CN/1%20Active%20BuckyOS.md)
- [doc/PRD/zh_CN/2 Use and Manage BuckyOS.md](/Users/liuzhicong/project/buckyos/doc/PRD/zh_CN/2%20Use%20and%20Manage%20BuckyOS.md)
- [doc/PRD/zh_CN/3 Backup and Restore.md](/Users/liuzhicong/project/buckyos/doc/PRD/zh_CN/3%20Backup%20and%20Restore.md)
- [doc/PRD/zh_CN/4 Create Other Familiy Account.md](/Users/liuzhicong/project/buckyos/doc/PRD/zh_CN/4%20Create%20Other%20Familiy%20Account.md)
- [doc/PRD/zh_CN/5 Share Files To Friends.md](/Users/liuzhicong/project/buckyos/doc/PRD/zh_CN/5%20Share%20Files%20To%20Friends.md)
- [doc/PRD/zh_CN/7 Support for external storage devices.md](/Users/liuzhicong/project/buckyos/doc/PRD/zh_CN/7%20Support%20for%20external%20storage%20devices.md)
- [doc/PRD/zh_CN/8 Manage dApps.md](/Users/liuzhicong/project/buckyos/doc/PRD/zh_CN/8%20Manage%20dApps.md)
- [doc/PRD/zh_CN/9 Active Used Devices.md](/Users/liuzhicong/project/buckyos/doc/PRD/zh_CN/9%20Active%20Used%20Devices.md)
- [doc/PRD/zh_CN/10 SDN.md](/Users/liuzhicong/project/buckyos/doc/PRD/zh_CN/10%20SDN.md)
- [doc/old/0.1(demo)/release.md](/Users/liuzhicong/project/buckyos/doc/old/0.1%28demo%29/release.md)
- [doc/old/0.3 alpha1/plan.md](/Users/liuzhicong/project/buckyos/doc/old/0.3%20alpha1/plan.md)
- [doc/old/0.5 beta1/todo.md](/Users/liuzhicong/project/buckyos/doc/old/0.5%20beta1/todo.md)

### 3.2 `notepads/` 中偏产品规划或路线图的文档

- [notepads/Jarvis Desing.md](/Users/liuzhicong/project/buckyos/notepads/Jarvis%20Desing.md)
- [notepads/OpenDAN Roadmap.md](/Users/liuzhicong/project/buckyos/notepads/OpenDAN%20Roadmap.md)

### 3.3 现状判断

- 当前仓库里“项目介绍/市场”与“PRD/产品规划”是混放的
- 真正面向外部介绍的材料不多，更多是内部产品规划、需求设想和路线图
- `doc/introduce/` 虽然也有介绍性质，但更接近“技术导读书”而不是市场文案

## 4. 教程类文档

这类文档主要回答“怎么做”“怎么上手”“如何搭建/调用/扩展”。

### 4.1 `doc/` 中的教程和操作手册

- [doc/introduce/README.md](/Users/liuzhicong/project/buckyos/doc/introduce/README.md)
- [doc/introduce/Chapter1 DID.md](/Users/liuzhicong/project/buckyos/doc/introduce/Chapter1%20DID.md)
- [doc/introduce/Chapter2 Acess app.md](/Users/liuzhicong/project/buckyos/doc/introduce/Chapter2%20Acess%20app.md)
- [doc/introduce/Chapter3 Start zone.md](/Users/liuzhicong/project/buckyos/doc/introduce/Chapter3%20Start%20zone.md)
- [doc/introduce/Chapter4 Service lifecycle.md](/Users/liuzhicong/project/buckyos/doc/introduce/Chapter4%20Service%20lifecycle.md)
- [doc/sdk/buckyos app.md](/Users/liuzhicong/project/buckyos/doc/sdk/buckyos%20app.md)
- [doc/sdk/create new service.md](/Users/liuzhicong/project/buckyos/doc/sdk/create%20new%20service.md)
- [doc/aicc/how_to_add_provider.md](/Users/liuzhicong/project/buckyos/doc/aicc/how_to_add_provider.md)
- [doc/aicc/krpc_aicc_calling_guide.md](/Users/liuzhicong/project/buckyos/doc/aicc/krpc_aicc_calling_guide.md)
- [doc/aicc/update_aicc_settings_via_system_config.md](/Users/liuzhicong/project/buckyos/doc/aicc/update_aicc_settings_via_system_config.md)
- [doc/arch/OpenDAN_Agent_Dev_Guide.md](/Users/liuzhicong/project/buckyos/doc/arch/OpenDAN_Agent_Dev_Guide.md)
- [doc/arch/sntest环境使用.md](/Users/liuzhicong/project/buckyos/doc/arch/sntest%E7%8E%AF%E5%A2%83%E4%BD%BF%E7%94%A8.md)
- [doc/arch/基于VM的开发环境构造.md](/Users/liuzhicong/project/buckyos/doc/arch/%E5%9F%BA%E4%BA%8EVM%E7%9A%84%E5%BC%80%E5%8F%91%E7%8E%AF%E5%A2%83%E6%9E%84%E9%80%A0.md)

### 4.2 `notepads/` 中的教程或快速手册

- [notepads/OpenDAN自动测试.md](/Users/liuzhicong/project/buckyos/notepads/OpenDAN%E8%87%AA%E5%8A%A8%E6%B5%8B%E8%AF%95.md)
- [notepads/兼容性思考快速手册.md](/Users/liuzhicong/project/buckyos/notepads/%E5%85%BC%E5%AE%B9%E6%80%A7%E6%80%9D%E8%80%83%E5%BF%AB%E9%80%9F%E6%89%8B%E5%86%8C.md)
- [notepads/用任意docker快速创建app.md](/Users/liuzhicong/project/buckyos/notepads/%E7%94%A8%E4%BB%BB%E6%84%8Fdocker%E5%BF%AB%E9%80%9F%E5%88%9B%E5%BB%BAapp.md)

## 5. 需要优先拆分的混合文档

按照 Harness Engineering 的思路，以下文档不适合继续把多种职责混在一起：

- [doc/PRD/control_panel/control_panel.md](/Users/liuzhicong/project/buckyos/doc/PRD/control_panel/control_panel.md)
  - 同时混合了产品目标、信息架构、实现现状、RPC 规划、路线图
  - 其中当前事实应迁到 `doc/control_panel/*.context.md`
  - 产品愿景和未落地规划应保留在 PRD 或 proposal 文档

- [doc/PRD/control_panel/SSO.md](/Users/liuzhicong/project/buckyos/doc/PRD/control_panel/SSO.md)
  - 同时包含契约、流程、历史设想
  - 适合拆成 “当前 SSO 契约” 与 “历史/未来方案”

- `doc/PRD/` 下若干 PRD
  - 同时夹杂“当前实现”“未来目标”“接口猜想”
  - 建议逐步标记为 `Implemented`、`Planned`、`Historical`

- `notepads/` 下的工程设计草稿
  - 目前很多内容仍有价值，但不应作为长期 source of truth
  - 适合在稳定后沉淀到 `doc/arch/`、`doc/<module>/` 或教程目录

## 6. 建议的后续落地方式

### 6.1 目录角色建议

- `notepads/`
  - 设计草稿
  - 讨论记录
  - Agent/Harness 规则草稿
  - 不直接作为最终规范

- `doc/arch/`
  - 全局架构原则
  - 跨模块实现约束

- `doc/<module>/`
  - 模块级 canonical 文档
  - 推荐延续 `README / ARCHITECTURE / SPEC / CONTEXT` 四分法

- `doc/PRD/`
  - 产品规划、立项、历史需求输入
  - 不再直接承载“当前事实”

- `doc/introduce/` 与 `doc/sdk/`
  - 教程、导读、开发者上手资料

### 6.2 若按 Harness Engineering 进一步重构

可以逐步映射为：

- `doc/architecture`：来自现有 `doc/arch/`
- `doc/modules`：来自现有 `doc/control_panel/`、`doc/message_hub/` 等模块文档
- `doc/proposals`：来自现有 `doc/PRD/` 和部分 `notepads/` 产品规划
- `doc/testing`：来自 `doc/test plan.md`、`doc/aicc/test_strategy_and_cases.md`、`notepads/OpenDAN自动测试.md`
- `harness/rules`：来自 `AGENTS.md`、`SKILL.md`、`Agent *.md`、`Harness Engineering.md`
- `harness/prompts`：来自 `PDCA提示词需求.md`、`behavior 提示词编写逻辑.md`、`check_behavior_review.md`、`do_behavior_review.md`

## 7. 简短结论

当前仓库的文档已经自然分成 4 层：

- AI/Harness 规则层
- 工程实现层
- 产品规划/项目介绍层
- 教程层

其中最需要优先做的，不是一次性移动所有文件，而是先明确：

1. 哪些文档是给 AI 的规则输入。
2. 哪些文档是给人看的 canonical engineering docs。
3. 哪些 PRD 只能表达 `Planned/Historical`，不能继续冒充“当前事实”。
