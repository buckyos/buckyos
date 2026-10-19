# BuckyOS App 生命周期与基础设施职责边界

> 当前权威：PIKG 开发、交付、安装、预装与运行各层的职责。本文合并旧生命周期稿、Installer 和预装迁移清单中的现行规则；旧方案只在 [历史参考](../pikg/history/README.md) 保留。

具体格式由 [PIKG 格式规范](../pikg/format.md) 维护，身份/AppDoc/提交事务由 [App 安装协议](../App%20安装协议.md) 维护，命令由 [PIKG](https://github.com/buckyos/buckyos-websdk/blob/main/doc/modules/pikg.md) 与 [App 模块](https://github.com/buckyos/buckyos-websdk/blob/main/doc/modules/app.md) 维护。

## 1. 当前可执行主线

```text
App 自有构建 → PIKG build/pack/info → 受控 staging
→ Resolve / Inspect 与批准的 InstallPlan
→ Acquire / Verify / Prepare / Deploy / Activate
→ Scheduler 提交和投影 → node-daemon 收敛
```

普通 App 的公开文件交付物是完整 PIKG。AppDoc、PackageMeta 和 payload 是其内部对象，不要求开发者分别发布；`dapp_dist` 是开发工具的中间快照。应用代码及运行依赖应预先进入 source bundle 或本地 image，CLI 不编译业务源码或下载第三方 package。

## 2. 组件职责与写入边界

| 组件 | 当前责任 | 不应混入的职责 |
| --- | --- | --- |
| PIKG CLI | 开发态元数据、payload、完整包和离线内容验证 | 业务编译、安装、Owner 签名、BNS 发布 |
| Repo / NamedStore | 保存和读取可验证对象与内容；`app.publish` 可生成 Repo candidate | 因内容已存储就宣称 App 已正式发布 |
| Resolver / Installer | 身份与发布证据、受控来源、可信 target、权限配置、不可变计划与可恢复任务 | 改写已批准的内容、签名发布、接受任意服务端路径 |
| Scheduler | 消费最终 InstallPlan，CAS 提交 Registry/AppSpec/InstallRecord，再推导 NodeConfig 等目标状态 | 读取 rootfs PIKG、做用户安装决策、改选发行版本 |
| node-daemon | 按 NodeExecutionSpec 和 exact package 部署、启停与上报 | 回读完整 AppDoc 补字段、按 latest/friendly alias 选择另一个版本 |
| Agent 管理 | 模板注册、创建/更新 Agent 的构造 App、维护 Agent 身份和 binding | 把模板包 AppDID 直接当作运行中 AgentDID |

Control Panel 负责计划编排和安装任务，最终 Registry/AppSpec/InstallRecord 提交属于 Scheduler 的事务边界；不能沿用旧稿中“Installer 直接写 AppSpec/Registry”的描述。详细持久 key 与取消/恢复规则只在安装协议中维护。

## 3. 来源、计划与升级

客户端取得 PIKG 字节并绑定 digest，再纳入 Installer staging；远端用 FileObject，本机受限通道用随机 incoming file id。服务端重新验证实际字节，客户端文件路径不作为服务端路径解释。

Inspect 根据已验证 AppDoc、解析证据、服务端可信目标和用户选择生成 immutable InstallPlan。批准绑定计划 fingerprint；source/target/配置/内容身份变化必须重新生成计划。Prepare 不再另选一份 AppDoc 或版本。

普通 App 首装要求 `app fetch` 生成的计划与同一来源；本地新版本使用 `app install`，Catalog 检查使用 `app upgrade`。未签名开发包只能在明确选择且被目标授权的 local developer 路径安装，不会因此成为 BNS 当前版本。参数、同版本处理、等待与失败恢复以 App 模块为准。

## 4. 系统预装

`system/install_settings.pre_install_apps` 保存版本化 `pikg_path` 与计划 seed。bootstrap 仅建立系统状态和 seed，不读取包或提前创建普通 App 安装任务、AppSpec 和 Registry allocation。

Control Panel 启动安装 runner 后，`PreInstallReconciler` 等待所需服务，再把 rootfs 包纳入 immutable staging，以 `SystemInternal` 和内部自动确认走标准 Installer。该内部 authority 不扩展成公共免验证入口。

PIKG 缺失或损坏形成后台预装诊断，不阻止 kernel boot。已有 `Deleted` AppSpec 表示用户已移除，reconciler 记录 `user_removed` 并停止该 seed 的预装；不能因系统包升级而自动装回。执行和恢复仍复用 InstallEngine/InstallRunner 与 Scheduler plan 协议。

Jarvis 在 `system/install_settings.agent_templates` 登记为内置 Agent 模板，由 Agent 管理读取并校验。它不再是旧稿中的独立 scheduler bootstrap Agent helper，也不作为普通预装实例直接运行；创建 Agent 时根据模板构造独立 App。

源码：[boot template](../../src/rootfs/etc/scheduler/boot.template.toml)、[pre_install_reconciler.rs](../../src/frame/control_panel/src/pre_install_reconciler.rs)、[agent_mgr.rs](../../src/frame/control_panel/src/agent_mgr.rs)。构建和 rootfs 落位见 [Module 构建规范](../CI/module_build_type_and_format.md)。

## 5. 三种包的消费入口

| 包 | 当前消费方式 |
| --- | --- |
| 普通 static-web/script/docker App | 标准 App Installer，按用户形成独立 AppInstance |
| `app_type=agent` 模板 | 公共 `apps.submit` 注册/更新模板；创建 Agent 时构造 runtime App |
| 独立 `app_type=agent_skills` 包 | TypeScript verifier → `skill install` → Agent `skills.install`，不进入普通 App Installer |

Agent 模板可包含 `agent` 与 `agent_skills`；schema 中存在 `agent_tools` 不表示所有发布路径都支持，当前 `app.publish` 拒绝该子包。skill 的目录与 Session 冻结规则由 [OpenDAN 开发指南](../sdk/OpenDAN_Agent_Dev_Guide.md) 维护。

## 6. 签名与正式发布的现状

`pikg info` 通过只表示内容完整，不证明 Owner 签名或 BNS 当前权威。`app.publish` 的现有行为是读取 Control Panel 节点目录、重新打包并写 Repo candidate，不是发布已测试 PIKG 的正式闭环。

Owner AppDoc 签名、PIKG 托管/下载可用性、BNS 更新是三个授权边界。当前尚无完成三者并回读的公开命令链；旧稿中的 sign/upload/publish 状态机和 `APPDOC.jwt` 签名建议保留在 [历史设计](../pikg/history/app-config-and-publication.md)，不能当作现有产品操作或 wire 规范。

## 7. 验证责任

开发工具验证包内数据；Installer 重验字节、信任、权限和计划；Scheduler 验证提交身份并完成持久事务；节点提供匹配 deployment 的运行证据。离线校验、任务完成、页面/API 确实返回新版本和升级后数据保留分别核对，不能相互替代。

本次文档整理不重放历史 DV。操作前使用实际 Tool schema、当前源码和目标环境核对；验证入口由 [PIKG 总览](../pikg/README.md) 与各模块测试 README 维护。
