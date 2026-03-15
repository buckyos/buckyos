# buckycli v2 需求文档

更新时间: 2026-03-10

## 1. 背景

当前 `control-panel` 已经形成了一个可用但不完整的系统管理入口；当前 `buckycli` 仍更接近开发工具、包工具、配置直连工具的混合体，还不是一个真正的“系统控制 CLI”。

如果目标是“让 buckycli 能完整控制整个系统”，则不能继续在现有命令上零散追加功能，而需要重新定义：

- `buckycli` 的产品定位
- `buckycli` 与 `control-panel` 的职责边界
- `buckycli` 的统一资源模型、命令体系、输出规范、安全模型
- `buckycli` 依赖的后端控制面与稳定契约

本文先基于仓库当前实现做 review，再给出新的 `buckycli` 系统级需求。

## 2. 当前实现 Review

### 2.1 control-panel 当前真实能力

当前 `control-panel` 是一个统一壳层，内部包含 3 个运行面：

- 主控制台 SPA
- `/kapi/control-panel` kRPC 接口
- 内嵌 Files HTTP API `/api/*`

当前已能提供的真实能力：

1. 认证与会话
- `auth.login`
- `auth.refresh`
- `auth.verify`
- `auth.logout`

2. 系统观测
- `system.overview`
- `system.status`
- `system.metrics`
- `system.logs.list/query/tail/download`

3. 网络与网关观测
- `network.overview`
- `zone.overview`
- `gateway.overview`
- `gateway.file.get`

4. 容器观测与基础控制
- `container.overview`
- `container.action`，当前支持 start/stop/restart

5. 应用基础信息
- `apps.list`
- `apps.version.list`

6. 配置访问
- `sys_config.get/set/list/tree`

7. 文件系统能力
- Files/Share 实际上主要走 `/api/resources*`、`/api/share*`、`/api/upload/session*`、`/api/public/*`
- 文件浏览、上传、预览、分享、收藏、最近、回收站已具备较完整 HTTP 面

8. 前端已接上真实数据的页面
- Desktop shell
- Monitor
- Network
- Containers
- Storage
- System Logs
- Files

### 2.2 control-panel 当前缺口

虽然 `control-panel` dispatch 中预留了大量 namespace，但大多数仍是占位：

- `user.*`
- 大多数 `storage.*`
- `share.*` kRPC
- 大多数 `files.*` kRPC
- `backup.*`
- 大多数 `apps.*` 生命周期操作
- 大多数 `network.*` 写操作
- 大多数 `scheduler.*` / `node.*` / `task.*`
- `repo.*`
- `security.*`
- `audit.*`
- `power.*`
- `time.*`
- `vpn.*`
- `proxy.*`
- `cert.*`
- `vm.*`

前端层面也存在“页面已做，控制能力未落地”的情况：

- `UserManagementPage` 目前主要是静态 mock 展示
- `SettingsPage` 中大部分模块仍是展示型信息，不是完整可写控制面
- `DappStorePage` 有展示，但没有真实安装/升级/卸载链路
- `RecentEventsPage` 主要复用 dashboard 数据，不是完整事件中心

结论：`control-panel` 当前更像“以观测为主、少量控制为辅”的统一工作台，而不是完整系统控制台。

### 2.3 buckycli 当前真实能力

当前 `buckycli` 已有功能主要集中在以下几类：

1. 包与仓库相关
- `pack_pkg`
- `install_pkg`
- `load_pkg`
- `pub_pkg`
- `pub_app`
- `pub_index`
- `update_index`
- `set_pkg_meta`

2. 配置直连
- `connect`
- `sys_config --get/--set/--list/--append/--set_file`

3. DID / 签名 / 本地材料生成
- `did`
- `sign`
- `build_did_docs`

4. 应用配置级操作
- `app --create`
- `app --delete`

5. 开发/测试/初始化工具
- `load`
- `create_user_env`
- `create_node_configs`
- `create_sn_configs`
- `register_device_to_sn`
- `register_user_to_sn`

### 2.4 buckycli 当前核心问题

1. 定位错误
- 当前命令集合更偏“开发脚手架 + 仓库工具 + config 直改工具”
- 不是一个清晰的系统运维 CLI

2. 信息架构错误
- 一级命令按历史脚本自然生长，没有统一资源模型
- 同一类行为分散在 `app`、`sys_config`、`connect`、`pub_*` 等入口

3. 系统控制面缺失
- 没有统一的用户、节点、服务、网络、网关、存储、任务、日志、事件、备份、审计控制能力
- 没有“完整控制整机/整 zone”的命令编排

4. 安全模型薄弱
- 很多写操作本质上是直接改 `sys_config`
- 缺少 plan/apply、diff、确认、审计上下文、回滚语义

5. 自动化能力不足
- 缺少标准化 `--json` / `--yaml`
- 缺少稳定 exit code 约定
- 缺少 watch/tail/selector/filter
- 缺少可供脚本稳定依赖的输出格式

6. 现有代码中存在未完成路径
- `src/tools/buckycli/src/ndn.rs` 仍有 `unimplemented!`
- `src/tools/buckycli/src/package_cmd.rs` 关键路径仍有 `unimplemented!`

结论：当前 `buckycli` 不能作为 BuckyOS 的系统控制入口，只能作为若干开发与运维散工具的集合。

## 3. buckycli v2 产品定位

`buckycli v2` 定位为：

> BuckyOS 的标准化系统控制命令行。

它应满足 4 个角色：

1. SSH 运维入口
- 在没有浏览器的环境下完整控制系统

2. 自动化入口
- shell script、CI、运维流水线、远程批量运维均可调用

3. 精确控制入口
- 对 `control-panel` 中可视化能力提供更精确、可组合、可批处理的命令面

4. 事实探针
- 为用户、测试、SRE、开发者提供稳定的一致系统视图

## 4. 产品边界

### 4.1 In Scope

- Zone、Node、System、Service、User、Network、Gateway、Storage、App、Repo、Task、Log、Security、Files 等系统对象的查看与控制
- 登录、上下文管理、权限校验、审计友好的操作执行
- 既支持人类交互，也支持自动化脚本
- 支持本地节点和远程 Zone 的统一访问

### 4.2 Out Of Scope

- 替代第三方应用自己的业务 CLI
- 替代底层开发脚手架和内部测试脚本
- 直接暴露所有内部存储结构作为用户主接口

## 5. 设计原则

1. 以资源模型组织命令，不以历史模块组织命令
- 用户操作的是 `node`、`app`、`network`、`storage`，不是某个内部 Rust module

2. 读写分离，先观测后变更
- 所有写操作都必须能先看到当前状态、计划变更、影响范围

3. 默认安全
- 破坏性操作默认二次确认
- 批量操作必须显式声明目标范围
- 支持 `--dry-run`

4. 对人类友好，对脚本稳定
- 默认输出适合终端阅读
- `--json` / `--yaml` 输出结构稳定

5. 不要求用户理解底层 `sys_config` 路径
- 高层命令必须屏蔽内部 key 细节
- 原始 key 操作只能作为高级能力存在

6. 与 control-panel 共用同一系统语义
- Web 和 CLI 应基于同一资源模型与同一后端契约

## 6. 与 control-panel 的职责分工

### 6.1 control-panel

适合：

- 可视化观察
- 面板式操作
- 文件预览
- 多对象联动浏览
- 首次接入与轻运维

### 6.2 buckycli

适合：

- SSH 场景
- 自动化脚本
- 批量操作
- 精确筛选、导出、审计
- 复杂写操作
- 无头环境运维

### 6.3 一致性要求

- 同一对象命名必须一致
- 同一状态枚举必须一致
- 同一写操作应复用同一后端能力，而不是 web 与 cli 各写一套逻辑

## 7. 命令体系重设计

## 7.1 一级命令

`buckycli` 新的一级命令建议如下：

1. `auth`
- 登录、登出、刷新 token、查看当前身份

2. `context`
- 当前 zone/node/profile 切换

3. `system`
- 系统状态、版本、升级、重启、关机、时间

4. `zone`
- zone 状态、设备列表、配置、激活、汇总信息

5. `node`
- 节点列表、详情、服务、重启、停机、维护态

6. `service`
- kernel/frame/app service 的列表、详情、日志、启停、重载

7. `config`
- 结构化配置查询、编辑、diff、history、apply、raw

8. `user`
- 用户、角色、组、会话、禁用/启用、授权

9. `network`
- 接口、IP、DNS、路由、防火墙、DDNS

10. `gateway`
- 网关模式、路由规则、配置文件、证书、reload

11. `storage`
- 磁盘、卷、容量、健康、SMART、快照、配额

12. `file`
- 浏览、上传、下载、移动、删除、搜索

13. `share`
- 分享创建、删除、查看、密码、过期时间

14. `app`
- 应用列表、安装、升级、卸载、启停、配置、版本

15. `repo`
- 源、同步、检索、发布、安装任务

16. `container`
- 容器、镜像、启停、重启、拉取、删除、inspect

17. `backup`
- 备份任务、目标、执行、恢复

18. `task`
- 异步任务列表、详情、取消、重试、等待

19. `log`
- 日志查询、tail、下载、服务过滤

20. `event`
- 事件、通知、告警、ack

21. `security`
- 2FA、API key、session、证书、审计

22. `dev`
- 保留当前打包、DID、环境生成、测试脚手架能力

原则：

- 运维命令和开发命令必须分层
- 当前 `pack_pkg`、`did`、`create_user_env` 这类命令应迁到 `dev`
- 用户默认接触的是系统控制命令，不是内部制作工具

## 7.2 通用动词规范

所有资源命令统一使用有限动词集：

- `list`
- `get`
- `create`
- `update`
- `delete`
- `enable`
- `disable`
- `start`
- `stop`
- `restart`
- `run`
- `apply`
- `diff`
- `history`
- `logs`
- `tail`
- `watch`
- `export`
- `import`

禁止继续扩散历史风格动词，如：

- `pub_*`
- `connect`
- `load_*`
- `set_*`

这类命名只能作为兼容别名短期保留。

## 7.3 典型命令示例

```bash
buckycli auth login
buckycli context use --zone myzone
buckycli system status
buckycli node list
buckycli node get ood1
buckycli service list --node ood1
buckycli service restart gateway
buckycli config get system/rbac --structured
buckycli config diff --file desired-system.yaml
buckycli config apply --file desired-system.yaml --dry-run
buckycli user list
buckycli user create alice --role admin
buckycli network interface list
buckycli network firewall list
buckycli gateway routes list
buckycli storage disk list
buckycli storage volume list
buckycli file ls /data
buckycli share create /data/photos --expires 7d
buckycli app list
buckycli app install photos --version 1.2.0
buckycli container list
buckycli container restart nginx
buckycli log tail --service control-panel
buckycli task list
buckycli task wait <task-id>
buckycli event list --severity warning
buckycli dev pkg pack ./pkg ./dist
```

## 8. 功能需求

### 8.1 认证与上下文

必须支持：

- 交互式登录
- 非交互式 token 登录
- refresh token 自动续期
- 多 profile 保存
- 多 zone 上下文切换
- 查看当前身份、权限、目标 zone

### 8.2 系统观测

必须支持：

- 系统概览
- CPU / 内存 / 磁盘 / 网络
- 服务状态
- 当前告警
- 日志查询和 tail
- 任务状态查询

### 8.3 系统控制

必须支持：

- 重启
- 关机
- 版本检查
- 升级执行
- 配置应用
- 服务启停 / 重启 / reload

### 8.4 用户与权限

必须支持：

- 用户列表、详情、创建、删除、禁用、启用
- 角色/组绑定
- 会话查看与撤销
- API key / token 管理

### 8.5 网络与网关

必须支持：

- 网卡查看与修改
- DNS 查看与修改
- 网关规则查看与修改
- 防火墙规则查看与修改
- DDNS 状态与配置
- 网关配置文件导出与 diff

### 8.6 存储与文件

必须支持：

- 磁盘、卷、容量、健康状态
- 文件浏览、上传、下载、重命名、删除、复制、移动
- 分享创建、删除、查看、权限、过期时间
- 回收站查看与恢复
- 文件搜索

### 8.7 应用、仓库、容器

必须支持：

- 应用列表、版本、安装、升级、卸载、启停
- 仓库源查看、同步、检索、发布
- 容器列表、inspect、start/stop/restart、镜像拉取与删除

### 8.8 备份、快照、任务、事件

必须支持：

- 备份任务创建、执行、停止、恢复
- 快照创建、删除、恢复
- 异步任务列表、详情、等待、取消、重试
- 事件与通知查询、过滤、ack

## 9. 交互与输出需求

### 9.1 输出格式

所有命令必须支持：

- 默认终端友好格式
- `--json`
- `--yaml`
- `--quiet`

列表型命令必须支持：

- `--wide`
- `--columns`
- `--sort`
- `--filter`
- `--limit`

流式命令必须支持：

- `--watch`
- `--follow`
- `--interval`

### 9.2 Exit Code

必须定义稳定 exit code：

- `0` 成功
- `1` 业务失败
- `2` 参数错误
- `3` 鉴权失败
- `4` 权限不足
- `5` 网络/超时
- `6` 资源不存在
- `7` 资源冲突

### 9.3 错误信息

错误输出必须包含：

- 失败对象
- 失败原因
- 建议下一步

禁止只输出：

- `failed`
- `panic`
- 原始内部栈信息

## 10. 安全与审计需求

### 10.1 写操作安全

所有写操作必须支持以下机制中的至少一部分，破坏性操作必须全部支持：

- `--dry-run`
- 变更 diff
- 二次确认
- `--yes`
- 批量范围提示
- 幂等语义

### 10.2 审计

所有写操作都应产生统一审计信息，至少包含：

- 操作者
- 时间
- zone/node 范围
- 操作对象
- 操作前后摘要
- 命令来源

### 10.3 权限模型

CLI 不应绕过权限系统直接“偷偷改配置”。

要求：

- 正式运维命令通过受控后端接口落地
- `config raw` 是高级逃生口，不是主路径
- 高风险能力默认要求更高权限或显式 `--force`

## 11. 后端契约需求

如果要让 `buckycli` 真正完整控制系统，必须同步要求后端控制面重构。

### 11.1 契约原则

- CLI 与 Web 共享对象模型
- CLI 与 Web 共享状态枚举
- CLI 与 Web 共享写操作后端
- 不允许“Web 走 RPC，CLI 直接写 sys_config”长期并存

### 11.2 后端能力来源

优先级建议：

1. 复用已存在的正式 client
- `VerifyHubClient`
- `SystemConfigClient`
- `RepoClient`
- `ControlPanelClient`

2. 对缺失域补齐正式控制接口
- `node.*`
- `scheduler.*`
- `task.*`
- `repo.*`
- `user.*`
- `storage.*`
- `network.*` 写操作
- `gateway.*` 写操作
- `app.*` 生命周期

3. 把当前“只有页面、没有稳定 API”的能力补成可脚本化契约

### 11.3 资源模型要求

最低统一资源集合：

- Zone
- Node
- Service
- User
- Session
- App
- RepoSource
- Package
- Container
- Volume
- Disk
- Share
- BackupJob
- Snapshot
- Task
- Event
- LogStream
- FirewallRule
- GatewayRoute

## 12. 兼容与迁移需求

### 12.1 命令迁移

当前一级命令迁移建议：

- `sys_config` -> `config raw`
- `app --create/--delete` -> `app create` / `app delete`
- `pub_pkg` / `pub_app` / `pub_index` -> `dev repo publish-*` 或 `repo publish`
- `pack_pkg` / `load_pkg` / `install_pkg` -> `dev pkg *`
- `create_user_env` / `create_node_configs` / `create_sn_configs` -> `dev env *`
- `register_device_to_sn` / `register_user_to_sn` -> `dev sn *`
- `did` / `sign` -> `dev did *`
- `load` -> `dev service load`

### 12.2 兼容策略

- v2 初期保留旧命令别名
- 默认输出 deprecation warning
- 两个小版本后移除旧入口

## 13. 分阶段落地建议

### Phase 1: 把 CLI 变成真正可用的观测与基础控制面

- `auth`
- `context`
- `system status/metrics/logs`
- `node list/get`
- `service list/restart/logs`
- `config get/diff/apply/raw`
- `app list/version/install`
- `container list/restart`
- `task list/get/wait`

### Phase 2: 补齐高频运维面

- `user`
- `network`
- `gateway`
- `share`
- `file`
- `repo`
- `event`
- `security`

### Phase 3: 补齐深水区能力

- `storage` 全面控制
- `backup`
- `snapshot`
- `power`
- `audit`
- `cert`
- `proxy`
- `vpn`

### Phase 4: 清理历史包袱

- 把开发脚手架完整迁到 `dev`
- 删除旧风格一级命令
- 统一所有帮助文本、输出格式、错误码、审计字段

## 14. 验收标准

满足以下条件时，可认为 `buckycli v2` 达到“完整控制系统”的阶段性目标：

1. 在纯 SSH 环境下，不打开浏览器也能完成常规管理任务
- 登录
- 看状态
- 查日志
- 改配置
- 管用户
- 管应用
- 管服务
- 管网络

2. 所有高频写操作都具备：
- 权限校验
- diff 或计划预览
- 稳定错误码
- 审计记录

3. 80% 以上当前 control-panel 已有能力，都有等价 CLI

4. CLI 能覆盖当前 control-panel 尚未覆盖、但系统必须具备的运维动作

## 15. 最终结论

新的 `buckycli` 不应再被定义为“若干内部命令集合”，而应被定义为：

> 面向 Zone / Node / Service / Config / User / Storage / Network / App 的标准系统控制接口。

这意味着：

- `control-panel` 是可视化工作台
- `buckycli` 是自动化与精确控制面
- 二者必须共享同一套资源模型和后端契约
- 现有 `buckycli` 中偏开发、偏打包、偏测试的命令要降级到 `dev` 子树
- 系统控制能力必须从“直接改 sys_config”升级到“正式控制接口 + 审计 + 安全模型”

如果按这个方向推进，`buckycli` 才能真正成为 BuckyOS 的完整系统控制 CLI。
