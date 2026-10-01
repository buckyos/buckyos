# Message Center 群 v2 持久数据格式

## 1. 概述

服务：`msg-center`。协议依据：[Self-Host Group v2](../../../../doc/message_hub/Self-Host-Groupv2.md)。本实现保存 host 的群配置、成员、Session、消息顺序、历史可见区间、操作日志和幂等结果，并保存本 Zone 成员从远端 host 获取的投影及同步水位。群规则与业务消息都必须在进程重启后保持一致。

实现入口：[group_store.rs](../src/group_store.rs)、[group_types.rs](../src/group_types.rs)。新表与既有邮箱表使用同一个平台 RDB 实例；生产环境不另开数据库文件。

## 2. 数据分类

| 数据 | 分类 | 保留和重建规则 |
|---|---|---|
| host 群状态、配置 revision 历史、成员与邀请记录、待接受的群主转让、Session 墓碑、可见区间 | 持久 | host 唯一权威，随平台 RDB 备份恢复 |
| MsgObject canonical JSON、验证后的 JWT 原文、撤回标记 | 持久 | 以 ObjectId 管理；撤回清除正文及 JWT，保留消息元数据 |
| 消息序号、Action Log、已读水位、审计、幂等结果 | 持久 | 与状态变更事务提交；不能从发送方时钟重建 |
| 邮箱投影、个人通知、tunnel 投递队列、本地 Session 登记 | 持久 | 复用 Message Center 现有表 |
| Joined Group 同步位置、本地阅读和上报水位、已获取的消息副本 | 持久 | 后续撤权不回收已获取副本；撤回消息按协议清理 |
| Joined Group 公开资料和 Session 列表缓存 | 可丢弃，存于持久行内 | 下次同步重建；Guest 不缓存其它 Session 或群配置 |
| opaque 同步 token | 可丢弃，存于 RDB | 7 天有效；丢失、过期或读者不匹配时返回 `limited=true`，从 inbox 水位补同步 |
| NamedStore 正文删除待办 | 持久 | 防止进程在撤回提交后退出，遗漏对象存储清理 |
| 群 DID 发布待办、已发布代数 | 持久 | 与群状态同事务提交；通过 system-config 事务更新 Zone resolver 的 doc/state 投影 |
| read Hook 允许结果、同步互斥锁、变更提示、进程消息缓存 | 可丢弃 | 内存数据；重启后不影响权威状态 |

数据库实际位置由平台 RDB 管理；开发模式复用现有 Message Center SQLite 开库路径。没有新增以文件路径为业务主键的存储。

## 3. 存储策略

结构化数据通过既有 `MsgBoxDbMgr` 的 SQLx `AnyPool` 存储，参数占位符由 `render_sql` 适配平台 backend。DDL 使用 SQLite、PostgreSQL 共有的 TEXT、BIGINT、主键和 `ON CONFLICT` 语义。

host 每群一行 `GroupState` JSON 聚合。同群写入先在事务中 UPDATE 锁定该行，再读取并修改状态。序号、配置 CAS、幂等结果、正文、GROUP_INBOX、成员投影与投递队列使用同一个事务。创建使用 `group_locks` 的单行锁，原子提交群、初始 Session 和邀请。

MsgObject 以 ObjectId 索引的独立 `group_objects` 行保存，不复制正文到邮箱记录。保存 canonical JSON 和 JWT 原文是为了支持事务内提交、原样返回签名及可靠撤回。附件仍由 NamedStore/CYFS 提供，群表仅存 ObjectId 引用，不内联二进制。

该聚合布局用于第一版单 host 默认实现：读写群状态的成本与该群累计记录数相关，同群操作串行。大量历史或接近 10,000 个 Session 时，需要基准测试后将元数据和日志拆为可索引的行；配置上限不是已经验证的吞吐保证。

## 4. Schema 定义

以下新表都引用 `group_locks.schema_version = 1` 的共享数据库格式版本；JSON 聚合还包含自己的 `schema_version`。

### `group_states`

| 列 | 类型 | 可空 | 默认 | 含义 |
|---|---|---|---|---|
| group_did | TEXT PK | 否 | 无 | 裸群 DID，全局唯一，删除后不复用 |
| state_json | TEXT | 否 | 无 | `GroupState` JSON |

主键支持按群定位和锁定。`state_json` 字段：

| 字段 | 内容 |
|---|---|
| schema_version | 当前为 1 |
| group_did, controller, owner, host, lifecycle, doc_updated_at_ms | 身份、DID 控制者、治理 Owner、host、生命周期和公开文档更新时间 |
| configuration, config_history | 原始配置 JSON；revision → 配置快照，保留未知字段 |
| members | DID → role、state、epoch、entity_kind、invited_by、邀请及到期信息 |
| sessions | 原始 Session ID → 模板、成员策略、规则覆盖、revision、生命周期、创建者和请求 ID |
| participants | Session ID → DID → kind、epoch、参与状态、since_seq、actor |
| intervals | Session ID → DID → 半开 `group_seq` 可见区间 |
| moderation | DID → blocked、muted_until_ms |
| pending_owner_transfer | 待接受的群主转让：member_did、transfer_id、expires_at_ms；接受、取消或过期后清除 |
| invite_links | token → 创建者、过期、次数、审批规则、撤销状态 |
| group_seq, session_seqs, accepted_at_ms | 群计数器、各 Session 消息计数器和 host 接受时间 |
| messages | ObjectId → Session、双序号、接受时间、作者、kind、关系和撤回状态 |
| changes | 有序变更：内部 group_seq、kind、Session、subject、message/change/action、受众 |
| shared_states, member_states | Session 权威共享状态及成员昵称状态，带独立 revision |
| operations | 操作键 → 请求摘要/原请求及结果；消息键为 `message:<obj_id>` |
| read_markers | Session ID → reader DID → last_read_seq |
| rates, audit | 频率窗口及 `group.read_all` 审计 |
| tombstone_readers | 曾参与者 DID 集合，仅用于获取自己的删除通知 |

默认 Session 的内部 map 键为 `""`，邮箱 `session_id` 为 NULL；对外始终使用规范 MailboxAddress。具名 Session 的内部 ID 是解码字符串，对外编码为单个路径段。

删除群清除配置、成员、Session、历史可见区间、阅读和业务状态，只保留身份墓碑、删除通知的最小受众（曾参与者 DID）及删除操作幂等结果。删除 Session 留下不可复用 ID，清除成员、共享/成员状态和 host 邮箱记录。成员已有的本地副本继续保留。

### `group_objects`

| 列 | 类型 | 可空 | 默认 | 含义 |
|---|---|---|---|---|
| obj_id | TEXT PK | 否 | 无 | MsgObject ObjectId |
| group_did | TEXT | 否 | 无 | 归属群 |
| body_json | TEXT | 是 | NULL | canonical MsgObject JSON；NULL 表示正文已撤回/清除 |
| jwt | TEXT | 是 | NULL | 已验证 JWT 原文 |

索引：`idx_group_objects_group(group_did)`。同一 ObjectId 不改写对象身份；NULL 正文墓碑不能被 Joined Group 重试恢复。撤回还删除 `msg_jwt_originals`、取消未发送的投递，并登记 NamedStore 清理待办。

### `group_sync_tokens`

| 列 | 类型 | 可空 | 默认 | 含义 |
|---|---|---|---|---|
| token | TEXT PK | 否 | 无 | 随机 UUID，不包含序号 |
| group_did | TEXT | 否 | 无 | 群 |
| reader_did | TEXT | 否 | 无 | 绑定读者 |
| group_seq | BIGINT | 否 | 无 | 内部扫描位置，不对外暴露 |
| created_at_ms | BIGINT | 否 | 无 | 生成时间及到期判断 |

索引：`idx_group_sync_reader(group_did,reader_did)`。仅三个键一起匹配且未过期时可用。

### `group_create_operations`

| 列 | 类型 | 可空 | 默认 | 含义 |
|---|---|---|---|---|
| actor | TEXT | 否 | 无 | 认证得到的创建者 |
| operation_key | TEXT | 否 | 无 | 创建幂等键 |
| request_json | TEXT | 否 | 无 | 原请求，用于拒绝键复用 |
| result_json | TEXT | 否 | 无 | 已提交结果 |

联合主键 `(actor,operation_key)`；与群创建同事务提交。

### `group_locks`

| 列 | 类型 | 可空 | 默认 | 含义 |
|---|---|---|---|---|
| lock_key | TEXT PK | 否 | 无 | 当前单行 `create` |
| schema_version | BIGINT | 否 | 无 | 当前为 1，共享格式版本 |

### `group_object_purges`

| 列 | 类型 | 可空 | 默认 | 含义 |
|---|---|---|---|---|
| obj_id | TEXT PK | 否 | 无 | 等待从 NamedStore 删除的对象 |
| schema_version | BIGINT | 否 | 无 | 当前为 1 |

清理成功或对象已不存在后删除待办；失败保留并重试。

### `joined_groups`

| 列 | 类型 | 可空 | 默认 | 含义 |
|---|---|---|---|---|
| owner | TEXT | 否 | 无 | 本 Zone 的读者 |
| group_did | TEXT | 否 | 无 | 远端群 |
| state_json | TEXT | 否 | 无 | `JoinedGroupState` JSON |

联合主键 `(owner,group_did)`。JSON 包含 `schema_version=1`、owner_did、group_did、可见 session_cache、doc_cache、changes_token、session_seqs、last_read_seq、reported_read_seq 和 stopped。凭据不存于本表；authorization/upstream 来源于服务 Settings。

### `group_doc_publications`

| 列 | 类型 | 可空 | 默认 | 含义 |
|---|---|---|---|---|
| group_did | TEXT PK | 否 | 无 | 待发布群 DID |
| generation | BIGINT | 否 | 无 | 公开文档改变时对应的 group_seq |
| published_generation | BIGINT | 否 | 0 | 已成功发布的代数 |
| document_status | TEXT | 否 | 无 | active 或 tombstoned |
| document_json | TEXT | 是 | NULL | 公开 GroupDoc；删除为 NULL |

与群状态同事务更新，仅公开 Doc 改变时推进 generation。后台一次 system-config `exec_tx` 更新 `resolver/cache/<转义DID>/group/doc` 与 `/state`，成功后仅确认匹配代数。崩溃、网络失败或服务权限不足时保留待办；重试幂等。system-config 数据是可由该行重建的发布投影，遵循现有 Zone resolver 的版本、控制者和墓碑格式，不是另一份群规则来源。

### 复用表与对象类型

`mailbox_records`、`msg_refs`、`delivery_records`、`msg_jwt_originals`、`owner_sessions` 复用现有 Message Center Schema。host 记录为 GROUP_INBOX；本地投影和成员 Zone 副本为 INBOX，带 `group:<did>` 与 `session_seq:<n>` tag。`owner_sessions.binding_json` 保存 `{authority_did,session_key}`，本地 Session ID 是群的规范 MailboxAddress。

`cymsg` 使用 ndn-lib 的标准 MsgObject 内容寻址和 canonical JSON；JWT 不改变 ObjectId。Action Log 是 `kind=event`、`from=group_did`、`content.machine.intent=buckyos.action_log` 的 MsgObject，与普通消息共享序号。`group` 用公开 GroupDoc 生成 ObjectId，公开内容是配置派生值，不是第二份规则来源。

## 5. Schema 版本

初始版本为 1，存于共享 `group_locks` 行、GroupState、JoinedGroupState、Group Configuration 和删除待办中。后续改变字段含义或表结构必须提升数据库格式版本，并在开库阶段提供显式迁移；配置 schema_version 与数据库版本独立演进。未知配置字段保留；未知 required_features 拒绝应用。

## 6. 升级兼容策略

所有新增群表和群对象使用 **No-compat（beta 2.2，未发布）** 策略；不读取或迁移 v1 群表，也不在启动时清除现存数据库。旧表残留不参与 v2 权威状态。需要清理开发数据时由部署方明确执行。

Joined Group 已获取副本和阅读水位属于用户数据；不能仅为了刷新缓存而清空。资料缓存和过期 token 可以重建。既有邮箱/联系人/tunnel 表的兼容策略沿用原服务，不在本次变更中修改。

## 7. 扩展规则

发布后冻结 DID、ObjectId、规范 Session 键、成员 epoch、双序号、半开可见区间、不可复用墓碑、认证主体和幂等键的语义。新增表列应提供默认值，或通过版本迁移引入。

配置原 JSON、Session `rule_overrides`、display 和 extensions 可以增加字段；未知字段按原样保存，不自动赋予权限。共享/成员状态 patch 仅接受声明的标题、说明、公告、昵称，不能借 extensions 修改规则。MsgObject 和 Action Log 扩展必须遵守 ndn-lib 协议。凭据不进入公开 Doc、变更流和日志。

## 8. 查询模式

| 查询 | 索引/执行方式 | 成本说明 |
|---|---|---|
| 定位群、同群事务锁定 | group_states 主键 | SQL 单行；反序列化整个群聚合 |
| 配置 CAS、幂等结果与消息查找 | 聚合内 map | 加载群后内存查找，提交整行 |
| 可见 Session、历史、变更流、读者回执 | 聚合内过滤及序号排序 | 扫描该群记录，长历史需要拆表优化 |
| Hosted Group 启动登记、list_by_member | group_states 全表扫描 | 每群解析聚合；第一版未做成员倒排索引 |
| 按 ObjectId 读取正文/JWT | group_objects 主键 | 单行 |
| 查找撤回目标的编辑/回应副本 | idx_group_objects_group | Joined Group 侧扫描该群正文 |
| 创建幂等查找 | group_create_operations 联合主键 | 单行 |
| 读取 reader-bound token | group_sync_tokens 主键 | 单行并检查群、读者、期限 |
| token 过期清理 | created_at_ms 条件扫描 | 尚无时间索引，长时间运行后需压测 |
| 查询本地成员的 Joined Group | joined_groups 联合主键前缀 owner | 返回该成员的群 |
| 邮箱分页、投递重试、本地 Session 列表 | 现有 Message Center 索引 | 沿用原实现 |
| NamedStore 清理队列 | group_object_purges 主键 | 每轮最多 100 条，逐项幂等删除 |
| 待发布 DID 文档 | group_doc_publications 条件扫描 | 每轮最多 100 条；已确认的相同文档不重复发布 |
