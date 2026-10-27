# HomeStation — Durable Data Schema

> **日期**：2026-10-26（Named Bucket 迁移 P1-3 / P2-2 / P3-2）
> **服务**：`homestation`（`src/frame/homestation`），协议与 kRPC 见《HomeStation 协议与实现》；桶的模型见 `doc/arch/应用对象桶 Bucket 设计.md`，迁移规则见 `notepads/bucket-migration-todo.md` §1、§3.2。
> **兼容**：开发期，**不兼容旧数据**：旧的 `<data>/users/<user>/homestation.db`、`<data>/zone.db`、单机 `<data>/chunks/` 与 `homestation:<entry>` pin 不再读取，DV 用 `uv run src/start.py --all` 全新安装。

## 1. Overview

HomeStation 一个服务进程服务 Zone 的全部用户。它持久化三类东西：

- **结构化记录**（每个用户的发表条目、Head、投递、来源、候选与待读、评论与评价、设置，以及 Zone 汇总列表）——一个 RDB Instance `homestation-main`，多用户靠 `owner` 列隔离（裁决 Q1）。
- **对象本体与字节**（用户发表的 Feed Object 各版本、附件文件、别人动态的预取副本、上传与抓取的中间产物）——Zone named store，保留经用户的应用对象桶 `user/homestation/<user>/<name>` 表达。
- 别的都可以丢（见 §2）。

## 2. Data Classification

| 数据 | 分类 | 去处 |
| --- | --- | --- |
| 用户的条目、Head 历史、变化序列、读取授权、发布任务 | 持久 | RDB `published`、`heads`、`head_history`、`stream_changes`、`object_grants`、`publish_tasks` |
| 已获取 / 已签发的命名对象（JWT 形式、签名者、是否仅本地） | 持久（本地已验对象缓存） | RDB `objects`、`feed_index`；本人发表的版本另绑定在 `entries` 桶 |
| 投递与接收 | 持久 | RDB `outbox`、`inbox_receipts` |
| 来源、意图、关注者、候选、待读、个人状态、评论来源、评价、约定、收录索引、设置 | 持久 | RDB 对应表（§4.1） |
| Zone 汇总列表 | 持久 | RDB `zone_feed`、`zone_changes`（Zone 自己的行，无 `owner`） |
| 本人发表版本的附件（图片、音视频、文件、目录、长文正文、分享的网页快照） | 持久，用户创作 | `published` 桶（documents） |
| 本人发表的 Feed Object 各版本 | 持久，用户创作 | `entries` 桶（documents） |
| 别人动态的预取：Feed Object 与附件 | 描述持久，内容可再拉 | `following-entries` / `following` 桶（ambient） |
| 浏览器上传、长文正文、网页快照（发表前） | 处理中间产物，7 天 | `scratch` 桶（temp） |
| 封面、缩略图等派生物 | 可丢弃 | `cache` 桶（声明，当前没有写入方） |
| `behavior_events` | 可丢弃（默认 7 天清理） | RDB |
| 进程内缓存（联系人、用户表、变化计数） | 可丢弃 | 内存 |

## 3. Storage Strategy

### 3.1 结构化数据：RDB Instance `homestation-main`

- scheduler `add_homestation()` 在 `services/homestation/spec` 的 `spec_config.rdb_instances` 登记 `homestation-main`（`buckyos_api::homestation_default_rdb_instance_config()`：sqlite，`user_data` 分区，schema 见 `buckyos_api::HOMESTATION_RDB_SCHEMA_SQLITE`）。服务启动时 `get_rdb_instance("homestation", None, "homestation-main")` 取连接串（`sqlite:$BUCKYOS_ROOT/data/homestation/homestation-main.db?mode=rwc`），按它打开（rusqlite，一个连接，进程内串行）。单机模式用 `<data-dir>/homestation-main.db`。
- **一个实例服务全部用户**：每张用户表的第一列是 `owner`（Zone 用户名，即 `/home/<user>` 的路径段），主键与唯一约束都以 `owner` 开头；每个 Station 拿到的数据库句柄 `Db::for_owner(<user>)` 让每条语句按 `owner=?` 读写。用户数据的搬迁、清除与隔离走导入 / 导出（"user data mgr"），不靠按用户分库。
- `zone_feed`、`zone_changes` 是 Zone 的行，没有 `owner`；`meta` 只有 `schema_version`。
- RDB 只存 ObjId 与业务 id，不存本机路径；桶 key 由业务 id 推出（§3.2），不另存。

### 3.2 对象与字节：named store + 桶

`spec_config.buckets` 由 `buckyos_api::homestation_bucket_decls()` 给出（scheduler 与单机 nfs-server 的 `--bucket-specs` 共用；`homestation --bucket-specs` 打印后者），`spec_config.storage_quota_mib = 4096`（约束 `scratch`）。全部是 `scope: user`，即 `user/homestation/<user>/<name>`，nfs-server 首次访问时为该用户懒建；服务以 `Session::Service` 调用，只能动 `appid = homestation` 的桶；绑定的持有证明是 frame 对 `obj://ndm/objects` 的读权。

| 桶 | kind | data_policy | key | 谁写（同一函数里先 put 后绑定） | 谁解绑 |
| --- | --- | --- | --- | --- | --- |
| `published` | data | documents，`versioning: false` | `<namespace>/<entry key>/<version ObjId>/<n>`（`n` 是该版本 `content_parts` 中第 n 个 FileObject / DirObject：wraps、cover、media 的顺序） | `publish_version` → `bind_version`：在写 RDB 之前绑定（设计 §15.1） | 撤回（Head → Withdrawn）后 `sync_entry_bindings` 解绑该条目全部 key；写 RDB 失败时同一函数撤回刚绑的 key |
| `entries` | object | documents，`versioning: false` | `<namespace>/<entry key>/<version ObjId>`；值是该版本对象的规范 JSON（ObjId 由它算出；签名 JWT 留在 RDB `objects.jwt`，投递用） | 同上（`put_bound_object`：put_object + 绑定） | 同上；再次激活（如再次点赞指回旧版本）时重新绑定 |
| `following-entries` | object | ambient（`origin` 必填） | `<feed ObjId>`；被它包裹的内层动态 `<feed ObjId>/<inner ObjId>` | `prepare_resources`（入列、打开候选、重试资源时）对**别人**的动态；本人的动态不进 | `release_following`：离开待读窗口、候选到期、取消收藏 / 稍后再看，且不再在待读、待选或收藏中 |
| `following` | data | ambient（`origin` 必填） | `<feed ObjId>/<file ObjId>` | `prepare_file`（预取描述对象与 chunk 后）、`fetch_content_on_demand`（所有者按需取内容后） | 同上（按 `<feed ObjId>` 前缀） |
| `scratch` | data | temp，`default_ttl_days: 7` | `uploads/<id>/<文件名>`（`object.upload_target` 分配，浏览器 finalize 时 `auto_bind`）、`articles/<id>/<slug>.md`、`snapshots/<capture ObjId>.html` | 浏览器上传（NDM finalize）、`store_file`（长文正文、网页快照） | 到期自动解绑；发表时内容另绑进 `published`，不依赖 scratch |
| `cache` | data | cache，`quota_mib: 1024`（软） | — | 当前没有写入方（为封面 / 缩略图等派生物预留） | 软配额 LRU |

- `origin`：`author` = 动态的发表者 DID，`source` = 动态的入口 `cyfs://<zone>/home/<user>/<ns>/@/<key>`（没有入口时用发表者 DID），`collected_at` = 预取时刻。
- ambient 只锁描述对象（Skeleton），预取的 chunk 是 class 0 缓存；内容被逐出后，所有者读取走按需再取（`fetch_content_on_demand`），其他读者得到 404（D20 “有则用、无则 404”）。
- 发表时附件一律按 ObjId `bucket.put` 进 `published`（frame 持有证明），不从 `scratch` copy：附件也可能来自 Agent 的 `artifacts` 桶（OpenDAN 自动分享，`object.register` 不带 key），两种来源走同一条路；用户对附件的持有在 `object.register` 时已验证（带 key：上传在用户自己的 `scratch` 中绑定的就是这个 ObjId；不带 key：调用者自己的会话能经 NDM 读到它）。
- named store 写入点与对应绑定：`Storage::put_bound_file`（chunk + FileObject → 绑定）、`Storage::put_bound_object`（对象 → 绑定）、`prepare_file` / `fetch_content_on_demand`（`put_fetched_chunk` / `put_fetched_object` → `following` 绑定）。接收与拉取到的对象（含 FileObject 描述）只进 RDB `objects`，不写 named store。

### 3.3 文件系统

- 服务模式：只有 rdb_mgr 给出的 sqlite 文件（`$BUCKYOS_ROOT/data/homestation/homestation-main.db`，由 rdb_mgr 创建分区目录）。
- 单机模式：`<data-dir>/homestation-main.db`（开发与测试用；named store 与桶由配置的 nfs-server 提供）。

## 4. Schema Definitions

### 4.1 RDB 表

完整列定义是 `buckyos_api::HOMESTATION_RDB_SCHEMA_SQLITE`（`src/kernel/buckyos-api/src/homestation_client.rs`），以下给出主键与用途。时间均为 Unix ms（`iat` 为秒）。

| 表 | 主键 / 唯一 | 内容 |
| --- | --- | --- |
| `meta` | `key` | `schema_version` |
| `settings` | `(owner, key)` | 用户设置 JSON（`user_settings`）、评论认领标记 `claimed:<obj>`、收录游标 `collector_cursor:<did>`、`stream_compacted_through` |
| `import_metadata` | `(owner, obj_id)` | 附件的展示元数据（name、mime、width、height、duration_ms） |
| `objects` | `(owner, obj_id)` | 已验对象：`obj_type`、规范 JSON `body`、`jwt`、`signer`、`publisher`、`verified`、`local_only`、`origin`（self / fetch / upload / spider / named_store / push…） |
| `feed_index` | `(owner, obj_id)` | Feed Object 可检索事实：publisher、kind、comment_type、content_type、entry、entry_valid、target、wraps、iat、category、text、tags |
| `heads` | `(owner, entry)` | 每个入口最高 seq 的 Head、冲突、受限、档位 |
| `entry_versions` | `(owner, entry, obj_id)` | 入口的已知版本 |
| `published` | `(owner, entry)` | 本人条目：key、namespace（桶 key 前缀 `<namespace>/<key>` 由此得出）、kind、audience、category、target |
| `head_history` | `(owner, entry, seq)` | 本人入口的 Head 历史 |
| `stream_changes` | `cursor`（全库自增） | 本人发表流的变化序列；按 `(owner, cursor)` 读 |
| `object_grants` | `(owner, obj_id, entry)` | 读取授权：对象可给能读该入口的人 |
| `publish_tasks` | `(owner, key)` | 发布任务（按意图 key 幂等） |
| `outbox` | `id`（全库自增），唯一 `(owner, obj_id, recipient)` | 投递任务与结果 |
| `inbox_receipts` | `(owner, obj_id)` | 已接收 ObjId 与准入 |
| `sources` | `(owner, id)`，唯一 `(owner, did)`、`(owner, url)` | 来源与采集进度 |
| `intents` | `(owner, id)` | 订阅意图 |
| `followers` | `(owner, did)` | 收到的关注 |
| `candidates` | `(owner, obj_id)` | 候选（保留期 14 天，入列的保留） |
| `admission_history` | `(owner, obj_id)` | 首次入列与补看阅读 |
| `reading` | `(owner, obj_id)` | 待读列表（窗口默认 300） |
| `view_cursors` | `(owner, view_key)` | 视图游标 |
| `personal` | `(owner, obj_id)` | 点赞 / 转发 / 收藏 / 稍后再看 / 点踩 |
| `comment_sources`、`author_list`、`tracked`、`participants` | 均以 `owner` 开头 | 评论来源、作者列表、追踪的讨论、参与者 |
| `view_snapshots` | `id`（全库自增） | 评论视图快照 |
| `eval_records`、`eval_tasks`、`tag_overrides` | `(owner, …)` | 评价记录、任务、修正 |
| `eval_changes` | `cursor`（全库自增） | 评价变化流 |
| `behavior_events` | `id`（全库自增） | 临时行为事件（可丢弃） |
| `agreements`、`consumption_proofs`、`capture_shares`、`collector_index` | `(owner, …)` | 消费约定与证明、抓取分享关系、收录索引 |
| `zone_feed` | `entry` | Zone 汇总列表（`user`、publisher、iat、listed） |
| `zone_changes` | `cursor` | 汇总列表变化序列 |

### 4.2 对象

| 对象 | 类型 | 命名 | 格式 |
| --- | --- | --- | --- |
| Feed Object | `cyfeed` | ObjId = 规范 JSON 的哈希 | 线格式见《协议与实现》§2.2；桶里存规范 JSON，签名形式在 RDB |
| 关注声明等其他条目版本 | `cyfollow` 等 | 同上 | 同上 |
| 附件、长文正文、网页快照 | `cyfile` / `cydir`（+ ChunkList、chunk） | ndn-lib 标准 | FileObject `meta.mime` 及展示元数据 |

## 5. Schema Version

`meta.schema_version = 3`（`buckyos_api::HOMESTATION_RDB_SCHEMA_VERSION`，也是 rdb 实例配置的 `version`）。3 = 单实例多用户（`owner` 列）、去掉 `retentions` / `retention_issues`。版本不符时服务拒绝启动；格式变化直接升版本。

## 6. Upgrade Compatibility Strategy

- **No-compat（pre-release）**：全部表与桶。开发期变化直接升版本、全新安装，不写迁移（`AGENTS.md`、"DV 可全新安装"）。
- 桶：`kind` / `data_policy` / `scope` 创建后不可改；换档要声明新桶并 `bucket.copy`。
- 发布后：RDB 表 additive-only；桶 key 方案冻结（RDB 与桶靠它对账）。

## 7. Extensibility Rules

- 冻结：`owner` 列及其在主键中的位置；`published.namespace` / `key`（桶 key 前缀）；桶名与 key 方案；ObjId 的计算方式。
- 可扩展：各表新增可空列；`settings` 的 JSON；`import_metadata.metadata` 的 JSON；Binding 的 `metadata` / `tags`。

## 8. Query Patterns

- 每条语句都以 `owner = ?` 起头匹配主键或 `(owner, …)` 索引；变化序列按 `(owner, cursor)`；待读按 `(owner, order_key)`；候选按 `(owner, selection, arrived_at)`；投递按 `(owner, state, next_at)`。
- 一个条目的桶绑定按前缀 `<namespace>/<key>/` 列举（`bucket.list_objects`），预取副本按 `<feed ObjId>` 前缀列举；都是该用户桶内的前缀范围，不扫全桶。
- 已知较重：`fetch_content_on_demand` 用 `instr(body, ?)` 在本用户的 `objects` 中找引用该文件的动态（LIMIT 8）。

## 9. Validation Checklist（本服务）

- [x] 结构化数据在 RDB 实例 `homestation-main`，经 rdb_mgr 登记；不按用户分库。
- [x] 每个 named store 写入都在同一函数里绑定到桶（§3.2 最后一条）；没有 `homestation:<entry>` 这类自管 pin。
- [x] 没有自建内容寻址目录（单机的 `chunks/` 已删除，单机与测试用独立 nfs-server）。
- [x] RDB、system config、RPC 结果不存本机路径。
- [ ] Files 投影（D11：`published` 投影到 `Apps/HomeStation/published/`）由 nfs-server 实现，未做（TODO P3-4）。
