# BuckyOS Beta2.2 发布数据格式与升级兼容清单

## 1. 范围与结论

Beta2.2 应作为后续覆盖升级的第一份正式数据基线。凡升级后仍要读取、恢复执行、验证签名或通过原有 ID/URL 访问的数据，都需要格式文档；后续版本必须直接兼容，或提供从这份基线出发的转换器。此要求不代表 Beta2.2 必须兼容此前所有开发版本。

兼容范围包括目录布局、数据库物理结构、记录内的 JSON/TOML/二进制结构，以及字段语义、身份、权限、对象引用、消费游标和事务恢复规则。保留文件而新程序打不开，不能算升级成功。

核对基线：2026-10-09 的本地 `buckyos/main`，HEAD `c0a35bbc`，工程版本 `0.7.0`。Beta2.2 是本次发布名称，最终 release commit/tag 尚需在发布时登记。工作区存在未提交改动，因此这里是发布盘点，不是已验证的发布产物清单。关联源码为同级 `buckyos-base`、`cyfs-ndn`、`cyfs-gateway`、`buckyos-devkit`，并补查了随安装包分发的 BuckyOSApp 与 CLI。

建议发布前优先补齐：**身份与钱包、system-config、应用注册与安装事务、NDN 对象存储、消息与任务、Agent 数据、AI Workspace、文件元数据**。已有协议或 DDL 可以复用，但必须明确哪一版是发布契约，以及升级时如何处理。

## 2. 数据分类

| 分类 | 判断标准 | 后续覆盖升级要求 |
| --- | --- | --- |
| 必须保留 | 用户输入、身份密钥、唯一业务事实、不可重取的内容、已对外发布的引用 | 兼容读取或无损迁移；不得以重建空库代替 |
| 必须恢复或明确终止 | 已接受但未完成的操作、消息投递、队列游标、幂等记录、Agent 工具调用 | 保持不丢失、不重复产生副作用；允许有文档和验收的终止/重试语义 |
| 可重建 | 能从保留的权威数据完整恢复，且重建不改变身份、权限、稳定引用或已接受操作结果 | 可更换格式，但要记录来源、重建顺序和失败处理 |
| 可丢弃 | 临时文件、纯缓存、可重新获取的派生物 | 不承诺旧格式；清理范围必须具体到数据项 |

**不能用目录名判断分类。** 当前 `local/system_config` 保存 Zone 主数据，`local/kmsg` 保存消息队列；AI Workspace 的 `local.sqlite` 保存授权和用户状态。它们均不能因含有 `local` 就在覆盖升级时清空。反过来，落在数据库中的 typing、心跳、搜索索引也不必永久兼容。

保留周期也需区分：覆盖升级保留，不等于卸载、用户主动重置、TTL 到期后仍永久保留。升级不能绕过原有保留策略，更不能自行缩短它。

## 3. 存储位置与安装行为

以下以 `$ROOT = $BUCKYOS_ROOT` 表示安装根目录。自定义根目录、环境变量、RDB backend/connection 可以改变实际位置，格式文档必须同时列出解析规则。

| 数据位置 | 当前用途与核对入口 | 需要冻结的边界 |
| --- | --- | --- |
| `etc/`、`security/`、`local/identity/` | 本机身份、网关配置、公有身份材料与私钥 | 三者共同组成身份集，不能只备份 `etc/` |
| `local/system_config/` | system-config 的 Sled 主库 | 整库与内部 revision 元数据；不是缓存 |
| `local/kmsg/` | kmsg 的 Sled 队列库 | 队列、未消费消息、订阅与游标 |
| `data/<service>/` | runtime `get_data_folder()`、RDB `user_data` 的默认服务位置 | 实际存在的持久数据区；不能只扫描 `data/srv` |
| `data/<user>/<app>/` | runtime/RDB 的 App 数据路径 | 与挂载式 App home 数据路径不同；变更需迁移 |
| `data/home/<user>/.local/share/<app>/` | 容器应用永久数据；其 `agents/<agent_id>/` 是 AgentRootFS | AppId、AgentId 与目录的绑定不能随版本重新分配 |
| `data/srv/`、`data/home/` | 服务持久目录、共享库、发布目录、用户文件 | 文件内容、权限、链接、扩展元数据与引用 |
| `storage/ndn/<mgr_id>/` 等对象存储根 | Named Store、对象与 chunks；实际 store 可由配置指定 | 存储布局、ObjectId/ChunkId、引用与 GC 根 |
| `data/.fsdb/` | nfs-server 的 `filedb.sqlite` 和 staging | 数据库中有用户元数据和稳定 ID，不能整体重扫替代 |
| `$ROOT` 之外 | BuckyOSApp 钱包、CLI profile/identity、用户外部挂载目录 | 也属于分发组件的升级契约；不能只备份安装目录 |

路径依据：[path_usage](../path_usage.md)、[runtime](../../src/kernel/buckyos-api/src/runtime.rs)、[RDB 路径解析](../../src/kernel/buckyos-api/src/rdb_mgr.rs)、[基础目录 helper](../../../buckyos-base/src/buckyos-kit/src/path.rs)。

安装器行为需要独立文档化。当前 [bucky_project.yaml](../../src/bucky_project.yaml) 配合 [devkit install.py](../../../buckyos-devkit/src/install.py) 的行为是：

- `modules` 每次更新覆盖；目录型 module 在本地安装时先删除目标目录再复制。因此 module 内不能混入唯一用户数据。
- `install_app_data` 在目标路径已存在时跳过整个路径，并不递归补齐其中新增的默认文件。`update_app` 对已有安装主要更新 modules；新增配置不能假定会自动由 `data_paths` 注入。
- BuckyOS 的 `etc/scheduler/`、`etc/boot_gateway.yaml` 属于覆盖更新项；网关仓库的 `etc/cyfs_gateway/` 也是 module。用户可编辑配置与产品模板的归属必须逐文件明确。
- `clean_paths` 包含 `local/`、`security/`、`etc/`、`data/var/`、`data/cache/`。`reinstall_app` 会先 clean；不能复用为承诺保留数据的覆盖升级。
- 新版本服务 spec、RDB 声明、默认 settings 需要对既有 system-config 做增量处理。替换 boot seed 并不能自动更新已经初始化的 Zone。

## 4. 必须文档化的格式清单

下列编号可作为发布任务拆分单位。“已有文档”仅表示有可复用材料，不表示已经完成发布冻结或具备转换器。

### F01 本机身份、启动与密钥材料

**内容：** `etc/node_identity.json`、`etc/start_config.json`、`etc/zone_document.jwt`；`local/identity/<encoded-identity>/` 下的 `did.json`、`device_doc.jwt`、`device_mini_doc.jwt` 等；`security/` 下的私钥、keyref、signer descriptor。

**文档必须包括：** 节点/Zone/Owner/Device DID 关系、路径编码、JWK/PEM/JWT 格式、key usage、引用解析、文档签发和验签规则、文件缺失的处理、权限以及身份恢复所需的完整文件集。启动种子与运行中的权威配置必须区分。

**兼容策略：** 保持身份和密钥不变；允许变更容器格式与路径，但需原子迁移并验证原身份仍可签名、验签及启动。签名文档不能直接改字段后保留旧签名；没有签发权时必须保留原文及对应验证能力。

依据：[device_identity.rs](../../src/kernel/buckyos-api/src/device_identity.rs)、[激活持久化](../../src/kernel/node_daemon/src/active_server.rs)、[Identity Manager](../../../buckyos-base/src/name-client/src/identity_mgr.rs)。已有说明：[启动与激活](02_boot_and_activation.md)、[身份与 RBAC](07_identity_and_rbac.md)、[身份与证书管理](../../../buckyos-base/doc/did-identity-certificate-manager.md)。需补成完整磁盘格式和恢复契约。

### F02 system-config 存储与 key 空间

**内容：** `local/system_config` 中的 Sled 数据；key/value 编码、路径规范化、`__meta/revision/<key>`、删除后 revision 的语义、CAS/事务和逻辑导出格式。

**文档必须包括：** 物理数据库与逻辑 KV 的两层版本；每个 key/prefix 的值类型、唯一写入者、权威/派生属性、敏感等级；升级或换 backend 时如何保持 revision、原子性和未完成操作的一致性。

**兼容策略：** 可继续用旧物理格式，也可通过旧引擎读取并转换到新库。不能只导出公开业务 key 后就声称保留了全部存储语义；revision 若重置，需要同步定义失效旧 CAS 请求和重启消费者的方案。

依据：[SledStore](../../src/kernel/sys_config_service/src/sled_provider.rs)。已有 [system-config Key Reference](system_config_reference.md)，但还需存储格式、格式版本识别、导出/导入与恢复说明。

### F03 账号、权限、用户设置与服务设置

**内容：** `boot/config`；`users/*/{settings,profile,doc,key}`；`devices/*/doc`；`agents/*/{doc,key,settings}`；`security/verify-hub/key`；`services/*/settings`；用户邀请、应用可访问策略、SMB 设置、桌面 session 状态等。`resolver/cache/*` 当前还承载解析 override 与 revoked/tombstoned/missing 等强负状态，不能仅因名字含 cache 就清除控制面记录。

**文档必须包括：** 密码哈希算法、编码和登录校验语义；账号类型/状态、角色与资源池；Provider/外部通道凭据；用户设置的缺省值与优先级；邀请状态、过期时间；权限主体、动作和资源路径的解释规则。

**兼容策略：** 账号仍可登录，权限不扩大，用户修改不被新默认值覆盖。密码验证格式升级应保留旧验证能力或定义认证后重新编码路径，不能凭旧哈希“转换”出不存在的明文。按 key 分类：`system/rbac/policy` 的调度器动态部分、SMB 已应用快照等允许从权威输入重建，但显式用户授权和设置必须保留。

已有：[Key Reference](system_config_reference.md)、[用户生命周期](10_user_lifecycle_and_permissions.md)、[RBAC 设计](<verify-hub/BuckyOS RBAC权限设计.md>)。需按服务 settings 分别补 schema，不能以“任意 JSON”结束。

### F04 应用与 Agent 注册、部署规格和安装事务

**内容：** `system/app_registry`；App/Agent `spec`、`settings`、`install_record`；`services/*/spec` 中的服务与 RDB 声明；`system/install_settings`；`system/scheduler/install_plan_executions/*`；TaskManager 中的安装 input/progress/result；部署 identity/generation。

**文档必须包括：** AppId/AppInstanceId/AgentId 的构造与相互引用、AppName/AppHostName/AppIndex 分配、revision/generation、AppDoc 与包摘要、安装阶段、claim/commit point、幂等 fingerprint、失败后的恢复与取消。也要规定旧服务 spec 如何得到新版本新增字段和 DDL。

**兼容策略：** 以整体事务迁移 registry、spec、record 和 task 引用。不得重新分配稳定 ID、host name 或端口索引；不得把已经 commit 的安装重放成另一实例。升级前未完成安装必须恢复或按明确规则收敛到终态。

依据：[app_schema.rs](../../src/kernel/buckyos-api/src/app_schema.rs)、[app_install.rs](../../src/kernel/buckyos-api/src/app_install.rs)、[install_plan_executor.rs](../../src/kernel/scheduler/src/install_plan_executor.rs)。已有 [App 安装协议](<../App 安装协议.md>) 和 [Key Reference](system_config_reference.md)。`NodeConfig`、service info、gateway 派生配置可重算；`system/app_registry` 是分配真相，不能归入可丢弃调度缓存。scheduler snapshot 的重建安全性需单独验证。

### F05 网关配置、证书与外部通道身份

**内容：** `etc/user_gateway.yaml`、`etc/post_gateway.yaml`、`etc/cyfs_gateway.yaml`、node gateway 参数、`etc/machine.json` 中的本机配置；TLS 证书与私钥引用；ACME 账户；SN/DNS 配置和证书管理状态；消息外部通道账户/session。

**文档必须包括：** YAML/JSON 的 schema、include/overlay/优先级、用户与生成字段边界；证书域名与 key usage；ACME account 格式与续期所需身份；通道认证和恢复游标。msg-center 的 Telegram session 目录由 `get_buckyos_service_data_dir(...)/tg_sessions` 构造，不能仅因位于运行数据目录就当作普通缓存。

**兼容策略：** 保留用户路由、证书账户和认证材料；派生 gateway view 可重新生成。证书可以依照续期规则更换，但不能把“重新申请应该成功”作为覆盖安装时删除唯一账户材料的理由。

依据：[网关架构](06_network_and_gateways.md)、[网关配置规范](../../../cyfs-gateway/doc/skills/cyfs-gateway-config-spec/references/config-reference.md)、[cert_mgr.rs](../../../cyfs-gateway/src/components/cyfs-acme/src/cert_mgr.rs)、[msg-center 入口](../../src/frame/msg_center/src/main.rs)。

### F06 NDN 对象、chunks、引用关系与 GC 状态

**内容：** Named Store 的 `named_store.json`、`named_store.db`、`chunks/`；数据库 objects、chunk_items、incoming_refs、edge_outbox、pins、fs_anchors；上层持久化的 File/Dir/Msg/Package 等对象。

**文档必须包括：** ObjectId/ChunkId 算法与编码、对象类型与规范化/哈希输入、chunk 布局、外部文件引用、分片/差分结构、pins 与引用图、GC 根、跨 store outbox。各业务对象 schema 必须另有定义，不能只记录底层“存储 JSON 字符串”。

**兼容策略：** 存储布局可以迁移，已发布 ObjectId/ChunkId 必须继续解析到原内容。不能重新序列化旧内容后把新摘要冒充旧 ID；若新旧对象并存，要保留旧解析与引用关系。迁移期间应避免 GC 把尚未转换的引用当成无主对象回收。

依据：[named_store.rs](../../../cyfs-ndn/src/named_store/src/named_store.rs)、[store_db.rs](../../../cyfs-ndn/src/named_store/src/store_db.rs)。已有 [CYFS 标准对象](<../../../cyfs-ndn/doc/CYFS Protocol/CYFS 标准对象.md>)、[Named Store GC](../../../cyfs-ndn/doc/GC/named_store_gc.md)。需补发布版磁盘布局、数据库版本和跨业务引用验收。

### F07 软件包、离线包与 Repo 发布数据

**内容：** `.pikg`、Package Meta、AppDoc/ServiceDoc/AgentDoc、包签名和验证材料；PackageEnv 的 `pkg.cfg.json`、`pkgs/meta_index.db`、`pkgs/env.lock`；repo-service 的 objects/proofs/receipts；本机应用清单 `bin/applist.json`。后者在当前安装配置中属于保留数据，不能把整个 `bin/` 一概理解为可替换程序。

**文档必须包括：** 包封装、manifest、依赖及版本选择、平台选择、包名和 DID/ObjectId 的映射、签名验证、部署产物目录、导入/导出。区分可重新下载的索引/解包目录与唯一的本地发布包、证明和收据。

**兼容策略：** 已安装旧应用、已保存的离线包和旧发布内容仍能识别；本地发布事实不可当作远端索引缓存清空。格式转换必须保持文档签名与内容寻址语义。

依据：[交付与包系统](05_delivery_pkg_system.md)、[Repo v2](<../repo_service/Repo v2.md>)、[repo_client.rs](../../src/kernel/buckyos-api/src/repo_client.rs)、[package-lib](../../../cyfs-ndn/src/package-lib/src/env.rs)。

### F08 Message Center 业务数据

**内容：** RDB `msg-center-main`，默认 `data/msg-center/msg-center-main.db`；mailbox_records、delivery_records、msg_refs、contacts/contact_metadata、group_subscribers、msg_idempotency、msg_tunnel_cursors、持久 UI session 设置；named store 消息正文和附件。

**文档必须包括：** 邮箱/会话/联系人 key、消息对象版本、owner 和成员范围、投递状态、幂等 key 作用域与 retention、外部通道 cursor、对象引用生命周期。桌面标题/草稿等与 typing/在线状态分开处理。

**兼容策略：** 数据库与对象存储联合保留；恢复后不漏投、不因 cursor/幂等信息丢失而重新发送已完成消息。消息正文可解析不等于已保留邮箱、已读或投递状态。

已有 [Message Center Durable Data Schema](<../message_hub/Message Center Durable Data Schema.md>)；当前仍声明 Beta2.2 no-compat，需明确该规则仅适用于本次发布前。实际表定义见 [msg_center_client.rs](../../src/kernel/buckyos-api/src/msg_center_client.rs)，建库入口见 [msg_box_db.rs](../../src/frame/msg_center/src/msg_box_db.rs)。

### F09 kmsg 持久队列

**内容：** `local/kmsg`；Sled trees `queues`、`queue_meta`、`messages`、`subs`、`queue_subs`、`meta`。

**文档必须包括：** 队列 URN、message key 的 `queue_urn + NUL + big-endian index` 编码、消息 envelope/body、队列头尾计数、订阅 ID、cursor、retention、ack 和删除语义。

**兼容策略：** 保留消息 index 和已消费边界，处理与 Agent input receipt 的一致性；不能重新编号后直接沿用旧 cursor。`queue_subs` 当前启动时从 `subs` 重建，是可重建索引，其他内容不能据此一起删除。

已有 [kmsg](kmsg.md)，需补磁盘格式与版本；实现见 [sled_msg_queue.rs](../../src/kernel/kmsg/src/sled_msg_queue.rs)、[msg_queue.rs](../../src/kernel/buckyos-api/src/msg_queue.rs)。

### F10 Task Manager、派发与 Workflow 计划任务

**内容：** Task Core 的 User/System 两个分区，默认分别在 `data/task-manager/task-mgr-main.db`、`local/task-manager/task-mgr-main.db`；同服务 Local 分区的 `task-dispatcher-main.db`；task/schema/note/event/ACL；派发、attempt、runner、route、cursor；业务 input/progress/result。

**文档必须包括：** 数据库物理版本与每种 task schema 版本；状态机、owner、storage_domain、父子任务和权限、幂等、重试关系、执行权/租约。定时任务还包括 timezone、触发规则、next_fire、fire_key 和补偿策略。

**兼容策略：** 不丢用户任务、计划、note 和结果；不重复执行已完成的外部副作用。System/dispatcher 即使不纳入用户备份，也要明确覆盖升级时的恢复或终止规则。User/System 两库版本必须协调。

已有 [Task Manager 2.0](<../task_mgr/task-mgr 2.0.md>)、[Task Data Schema](<../task_mgr/task data schema.md>)、[派发中心](../task_mgr/task_dispatch_center.md)；需补物理布局与业务 task 类型的联合升级矩阵。实现见 [task_store.rs](../../src/kernel/task_manager/src/task_store.rs)、[task_mgr.rs](../../src/kernel/buckyos-api/src/task_mgr.rs)、[task_dispatcher.rs](../../src/kernel/buckyos-api/src/task_dispatcher.rs)。

**当前能力边界：** Workflow 的 schedule 权威定义/触发记录保存在 Task DB，内存 ScheduleStore 可 hydrate；但普通 Definition/Run/Amendment 等私有状态以及当前对象 store 仍使用内存，不能声称保留 Task DB 即可恢复整个运行中的 Workflow。发布需明确支持的恢复范围。依据：[workflow 入口](../../src/kernel/workflow/src/main.rs)、[state.rs](../../src/kernel/workflow/src/state.rs)、[scheduled_task_manager.rs](../../src/kernel/workflow/src/scheduled_task_manager.rs)。

### F11 AICC 配置、执行、用量与产物归属

**内容：** `services/aicc/settings` 与 Control Panel Provider 设置；RDB `aicc-usage-log`（默认 `data/aicc/aicc-usage-log.db`）中的执行、用量/费用、路由 trace、artifact tenant/owner、外部 Provider 产物 ID、URL source、audit、inventory LKGS；metadata catalog 与防回退序列。

**文档必须包括：** Provider identity/凭据引用、方法与模型选择、金额/计量单位、`*_json` 权威 payload 与查询投影的关系、外部异步任务 handle、归属与可访问性、metadata schema/version/revision_seq。

**兼容策略：** 历史用量及费用不因单位或投影变化被误读；正在执行的 Provider 任务不得按空库重发；产物归属不可从 URL/对象内容猜测。可重建的 inventory/search 等缓存与防回退水位分开处理。

已有 [Runtime Durable Data Schema](../aicc/aicc_runtime_durable_data_schema.md)、[Provider Durable Data Schema](../aicc/provider_architecture_durable_data_schema.md)。当前 runtime 文档写初始 schema 1 和清库策略，而 [storage/mod.rs](../../src/frame/aicc/src/storage/mod.rs) 的全局 schema 已为 2，存在版本检查及迁移代码，必须在发布前对齐。

### F12 OpenDAN Agent 与 Session

**内容：** AgentRootFS 中配置、行为/提示词用户修改、session registry、perception/cursor、artifact head/version、会话目录的 `session_config.json`、`state.json`、`worklog.jsonl`、`runs/` 和 snapshot、输入 receipt、outbox、task/tool 调用关联、用户 workspace 文件。

**文档必须包括：** AgentId 与 AppInstance 的绑定；session 目录与 registry 关系；schema major；JSONL 记录边界；snapshot 与 worklog 提交位置；输入 ack；工具副作用结果；跨 Session/Artifact 引用和恢复提交顺序。状态锁/心跳可重新建立，但不能丢失其保护下的已提交业务事实。

**兼容策略：** 至少支持读取旧历史，并且对承诺可继续的会话支持恢复或迁移。当前代码对旧主版本会阻止推进，文档规定旧 Session 在显式迁移前只读；这是保护机制，不等于后续版本已支持无缝升级。

已有 [Agent Session 协议与 JSON Schema/fixtures](../opendan/protocol/README.md)，可作为最完整的基线材料之一。需冻结所有相关格式的组合版本，尤其是 Session、Input、RunRecord 与 LLM snapshot，不能只冻结 `state.json`。依据：[AgentLayout](../../src/frame/lib_opendan/src/state/fs_client.rs)、[session/runs.rs](../../src/frame/lib_opendan/src/session/runs.rs)。

### F13 Agent Memory、Notebook 与 Attention Signal

**内容：** `memory/` 的 meta、occasions JSONL 及 archive、canonical 内容文件、状态与图；`notebook/notebook.sqlite`；attention signal 数据、检查点与 extraction windows。

**文档必须包括：** 哪份是重放真相、是否包含完整正文、删除/tombstone 语义、条目/对象/关系/证据 ID、笔记与阅读水位、事件序号、抽取覆盖范围和人工修改。

**兼容策略：** Memory 的派生 SQLite/索引可以在保留并验证完整重建来源后重建；Notebook 条目、关系、备注不能当成索引删除。Attention Signal 需逐表说明可否重建，避免在原始感知历史已被清理时声称可以重新抽取。

已有 [Agent Memory v2](<../opendan/Agent元能力/Agent Memory v2.md>)、[Agent Notebook](<../opendan/Agent元能力/Agent Notebook.md>)、[Attention Signal](<../opendan/Agent元能力/Agent Memory Attention Singal Mgr.md>)。实现：[agent_memory.rs](../../src/frame/agent_tool/src/agent_memory.rs)、[agent_notebook.rs](../../src/frame/agent_tool/src/agent_notebook.rs)、[agent_attention_signal.rs](../../src/frame/agent_tool/src/agent_attention_signal.rs)。

### F14 AI Workspace 与便携文档包

**内容：** 默认 `data/aiworkspace/workspaces/<workspace_id>/{doc.sqlite,local.sqlite}`、共享 `objects/`、trash 与有效 staging；`.bcanvas` 导出包。doc 库有 entities/tree/refs/table/richtext/commit/assets/snapshot；local 库有 grants/user_state/runs/locks 等。

**文档必须包括：** storage schema、每种 entity/type/payload schema、稳定 ID、树与引用、revision/seq/tombstone、富文本 engine/version/encoding 与二进制 update、commit/undo/幂等、对象内容与包 manifest。导出格式版本应与内部存储版本区分。

**兼容策略：** 文档、附件、授权、个人状态和已接受 run 要分别迁移；`local.sqlite` 不随便携导出，不代表可在原部署升级时丢弃。保留原 entity/object ID 及外部引用。旧 `.bcanvas` 应能导入或通过转换器读取。

依据：[schema.rs](../../src/frame/aiworkspace/store/src/schema.rs)、[workspace.rs](../../src/frame/aiworkspace/store/src/workspace.rs)、[export.rs](../../src/frame/aiworkspace/store/src/export.rs)。已有 [核心设计](<../workspace/BuckyOS AI Workspace 实现的核心设计.md>) 等多份阶段文档，需收敛发布存储契约。当前打开文档要求 storage schema 精确匹配，尚不能据此推断具备跨版本迁移能力。

### F15 用户文件与 nfs-server 元数据

**内容：** `data/home`、`data/srv` 等实际文件与链接；`data/.fsdb/filedb.sqlite` 中 entities、anchors、namespace_bindings、views/view_patch、collections、meta_records、grants 等；`cyfs://`、anchor、node_id、object 引用。

**文档必须包括：** namespace/挂载映射、路径规范化、稳定锚点、实体/引用/视图/集合 ID、删除与改名、用户元数据、capability/撤销状态、外部路径链接及跨平台权限处理。

**兼容策略：** 扫描文件可重建部分物理属性，但不能重建用户集合、视图修改、授权或所有旧 anchor。明确哪些表是扫描投影、哪些是权威数据，并验证旧链接和对象引用继续可用。普通用户文件按字节保留；其第三方应用内部格式由该应用负责，OS 负责路径和挂载契约。

依据：[config.rs](../../src/frame/nfs_server/src/config.rs)、[filedb.rs](../../src/frame/nfs_server/src/filedb.rs)、[路径规范](../path_usage.md)。需补面向发布的 FileDB 持久 schema 与迁移规范。

### F16 BuckyOSApp 钱包、CLI 与客户端持久状态

**内容：** BuckyOSApp `wallet.store` 中的 vault、加密 seed、派生钱包、OwnerDocument、SN 绑定；应用 config；CLI config/profile/identity；浏览器持久化中尚未同步到服务端的用户状态。

**文档必须包括：** vault 版本、KDF/加密算法和参数、salt/nonce/cipher 编码、种子与地址派生规则、身份目录引用、配置根目录和平台差异。登录 session/token 可以要求重新登录；钱包和未同步用户内容不能按认证缓存删除。

**兼容策略：** 旧钱包必须能解密并派生出相同身份；改变算法时保留旧解密器并迁移，禁止通过重新生成钱包解决。CLI 旧 profile 仍能选择原 Zone 和 identity。

依据：[wallet store](../../../BuckyOSApp/src-tauri/src/did/store.rs)、[wallet crypto](../../../BuckyOSApp/src-tauri/src/did/crypto.rs)、[CLI config](../../../buckyos-websdk/cli/core/config.ts)。这些数据不一定在 `$ROOT` 下，桌面发布包应纳入同一次验收。

### 按实际交付组件补充的项目

- `klog/slog`：仓库包含 Raft 日志、state store、snapshot、分段日志及索引实现，但当前主安装 module 清单未包含相应 daemon。若发布包启用这些服务，应追加 term/vote/membership/log entry/state-machine snapshot 和记录序列化的版本契约；不能把它们当普通文本日志处理。
- DCFS/fs_meta/fs_buffer：当前单机路径与未来分布式文件系统不能混为一谈；交付并启用时，应补 inode/dentry、overlay、journal、chunk 映射和存储拓扑格式。
- `share_content_mgr.rs` 含 published_items/item_revisions/access_logs/access_stats 和默认 `data/share_content_mgr.sqlite3`，本次未在 control-panel 入口找到启用引用。若发布产物接入，应追加分享 ID/URL、发布版本、权限与撤回状态；不能仅因源码里有 DDL 就声称已上线。
- 其他预装/第三方 App：OS 要保留挂载的数据目录与 App 绑定；若随 OS 同时升级 App，还要取得该 App 自己的 schema 与转换方案。前端原型、本地 mock 不自动视为已经上线的服务存储。

## 5. 已确认的版本与文档缺口

软件版本 `0.7.0`、文档协议版本、数据库 schema version 和记录 schema version 是不同维度，不能相互替代。

| 格式 | 当前源码/文档版本标识 | 发布前需要完成 |
| --- | --- | --- |
| NodeIdentity | `buckyos.node_identity.v2` | 身份全套文件布局、旧格式识别与升级入口 |
| AppRegistry / AgentSpec / NodeExecutionSpec | 各为 `1` | 联合不变量、稳定 ID、拒绝未知字段时的演进规则 |
| App 安装持久记录 | `APP_INSTALL_SCHEMA_VERSION = 4`；预装 seed `1` | Task/SystemConfig 的事务及跨版本恢复 |
| Task Core / Dispatcher | RDB `8` / `4` | 两分区实库版本、payload schema、执行恢复 |
| Message Center | RDB spec `12` | 消除发布后 no-compat 歧义；验证实库而不只看 spec |
| Repo | RDB spec `1` | 发布事实与可重建索引的边界 |
| AICC | runtime storage `2`；inventory row `1` | runtime 文档仍写 `1`，对齐；区分 RDB spec 与服务内部 schema |
| OpenDAN | session config/state `/5`，input `/3`，LLM snapshot `4` | 结合 RunRecord、binding、summary 等形成完整版本矩阵与 fixture |
| Agent Memory / Notebook / Attention | `2.10` / `0.4` / `0.1` | 定义读写兼容范围及日志/索引/正文重建边界 |
| AI Workspace | `STORAGE_SCHEMA_VERSION = 2` | 当前精确版本检查之外的旧库/旧包读取或迁移路径 |
| BuckyOSApp vault / CLI profile | `2` / `1` | 加密与派生算法、旧钱包和 profile 样本 |
| system-config、kmsg、Named Store、FileDB | 上述存储入口未见统一的全库格式版本门禁 | 建立可靠格式识别；无版本库也必须能判定是 Beta2.2 还是未知/损坏数据 |

`CREATE TABLE IF NOT EXISTS`、类型上有 serde default、spec 里有 version，都不能单独证明具备迁移能力。必须核实旧物理表、旧枚举和旧 payload 在新实现下的实际行为。

## 6. 后续升级与格式转换约定

### 最小兼容承诺

每项格式明确选择：直接读取旧格式、增加可缺省字段、迁移到新格式、从权威源重建，或在升级前将未完成操作安全收敛到终态。发布后的唯一业务数据不能继续使用“清空后重建”的开发期策略。

“兼容旧数据”要求新版处理旧版数据；不自动要求旧二进制能读取新格式。若升级允许回滚，应明确通过反向迁移，还是恢复升级前完整快照实现。

### 转换器的最低要求

1. 在新服务写入前判定源格式、目标格式和支持的版本范围；不能把未知/损坏的库认成空库。
2. 明确需要停止哪些写入者，备份数据库、WAL/相关文件、对象和密钥引用的一致性集合。
3. 输出可重试的迁移阶段和完成标记；失败不得继续在部分转换的数据上启动服务，也不能偷偷清库。
4. 先完成内容及引用校验，再提交目标版本；大数据可分批，但中间状态必须可识别和恢复。
5. 保留跨库 ID、owner、权限、对象摘要、队列序号、幂等结果及已完成副作用；无法恢复的在途动作进入明确的恢复状态。
6. 支持升级中断后重跑。格式已是目标版时不再次执行有副作用的转换。
7. 明确磁盘空间、失败恢复和回滚边界；只回滚二进制不足以恢复已经改变的数据。

可采用安装前统一 preflight 加各服务内部 migrator；无需为了本次盘点立即引入新框架。关键是旧数据首次被新版写入前，有且只有一个明确迁移入口。

### 发布前优先任务

- [ ] 登记最终 Beta2.2 发布源码与构建产物，核实所有实际启用组件，产出按 F01–F16 分类的格式清单。
- [ ] 对 `local/system_config`、`local/kmsg`、身份目录、服务 `data/<service>`、AI Workspace local DB 明确保留规则，核对各平台真实安装包流程。
- [ ] 将现有 no-compat 条款限定到 Beta2.2 发布前；对齐 AICC 文档/代码版本等差异。
- [ ] 为无统一版本标识的存储建立基线识别办法；保存真实写入路径生成的脱敏样本和期望结果。
- [ ] 记录未完成的 Workflow 等恢复能力边界，避免对用户承诺超出现有实现。
- [ ] 下一版本更改任一冻结格式时，把 reader/migrator 与 Beta2.2 样本验收作为发布条件。

## 7. 每份格式文档的最低内容与扩展规则

每个负责模块应补齐以下信息；可引用现有协议、DDL、JSON Schema，避免重复维护两份字段表。

| 项目 | 必须说明 |
| --- | --- |
| 所有权与生命周期 | 谁写谁读；用户事实/运行事实/投影/缓存；升级、卸载、重置、TTL 的区别 |
| 位置与编码 | 默认路径、环境变量、RDB instance/partition、backend、文本/二进制编码、目录命名与文件权限 |
| schema | 表/列/约束/索引；JSON/TOML 字段；枚举取值；null/缺省/空值；时间、金额、大小单位 |
| 身份与关系 | ID 生成、唯一性、外键和跨库关系、内容摘要/签名、URI 的稳定性 |
| 版本 | 版本存放位置；无版本数据识别；读/写范围；未知字段/未知枚举/未知 major 行为 |
| 恢复语义 | 提交点、日志与 snapshot 一致性、幂等/ack/游标/租约、崩溃后状态 |
| 演进 | 可添加字段及默认值；不可改变的字段语义；reader 或转换器入口；失败与回滚 |
| 样本与验证 | Beta2.2 有效/缺省/边界样本、损坏/未知版本样本、脱敏方式、业务层期望结果 |

特别注意 `deny_unknown_fields` 的类型：增加字段并不天然兼容；混跑与回滚必须按实际 reader 能力判断。`extra` 或 `metadata` 扩展字段也不能偷偷改变旧字段的含义。

直接使用 SQLite、Sled 或文件作为核心存储的现状应如实登记。仓库 [持久数据设计规范](../../harness/SKILLS/design-durable-data-schema/SKILL.md) 推荐平台 RDB/object 管理；今后重构到平台能力时也要迁移已有数据，不能把规范当成旧数据无需兼容的理由。

## 8. 业务查询与升级验收

验收应让 Beta2.2 先写出数据，再用未来版本的安装包覆盖升级。只创建“看起来像旧版”的新 fixture，或只证明 SQL 能打开，不足以验证兼容。

| 验收动作/查询 | 必须保持的结果 |
| --- | --- |
| 原账号登录、钱包解密与签名、节点启动 | DID/派生身份不变，旧文档可验签，无需重新激活 |
| 查询用户、服务、Provider、App、Agent 和 registry | 设置保留，身份/绑定/hostname/index 不变，新默认值不覆盖用户值 |
| 查询旧任务与计划，恢复未完成安装和执行 | note/result/ACL 完整，无重复副作用；到点不漏触发或按既定补偿处理 |
| 读取历史消息、投递待发消息、消费 kmsg | 正文/附件可达，cursor/ack/幂等一致，不丢不重 |
| 打开旧 Agent 会话，读取记忆与 Notebook | 历史完整；承诺可续跑的会话恢复，输入与工具结果不重复提交 |
| 打开 Workspace、导入旧 `.bcanvas`、访问文件 anchor/集合 | 文档、富文本、附件、视图、授权和用户状态一致，旧链接继续可用 |
| 按旧 ObjectId/ChunkId 与包 ID 查找内容 | 内容及签名正确，迁移后的 GC 不误删在用对象 |
| 聚合历史 AICC 用量与费用，查询外部执行结果 | 单位/归属/金额和执行状态正确 |
| 迁移中断、磁盘不足、重复升级、未知/损坏版本 | 可诊断且不丢原数据，不启动部分迁移后的系统，不重跑已提交副作用 |
| 回滚到升级前状态、空缓存重建、离线启动 | 按公开支持边界工作；不依赖能随时重新下载唯一数据 |

各模块格式文档还应给出关键查询对应的索引/扫描成本与重建耗时，尤其是消息、任务、Named Store 引用图和 Workspace；格式兼容但启动时无界全库扫描，也可能使覆盖升级不可用。

本次盘点只新增此文档，未修改数据格式、安装逻辑或运行实例，未执行实际覆盖升级。发布验收需针对最终产物和实际 backend/平台完成。
