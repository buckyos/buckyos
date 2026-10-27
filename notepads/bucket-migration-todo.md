# 系统服务与默认应用改用 Named Bucket 的改造 TODO

日期：2026-10-26

状态：§7 六项裁决已由用户定案（2026-10-26），可开工。本文是后续 code agent 的工作指南：先读 §1 的硬规则，再按 §4 的分期领任务；每个任务的"完成"以 §6 的验收为准。

路径均相对 `buckyos/`。依据文档：

- 架构原则：`doc/arch/BuckyOS 架构设计原则.md`（简单可靠、没有潜规则、任何写入都要有对应的删除机会）
- 方向：`doc/arch/buckyos 数据落盘架构方向.md`（§2 从目录转向资源抽象、§5 RDB 存引用 namedstore 存本体、§6 原生应用优先 Named Data、§7 保留文件系统语义的场景、§8 资源访问 URL 化）
- 设计定稿与实施状态：`doc/arch/应用对象桶 Bucket 设计.md`（v0.6；§20 实施状态：M0–M4 已实现，含 S3 面）
- 宿主与接口：`src/frame/nfs_server/README.md`「应用对象桶」节；Rust `src/kernel/buckyos-api/src/bucket_client.rs`；websdk `src/bucket_client.ts`、`cli/modules/bucket.ts`；Agent 手册 `src/apps/jarvis_agent/agent/cmd_skills/buckyos_app_dev.md`
- 目录约定：`doc/path_usage.md`（改造完成后要回写，见 §4 P4）

## 0. 目标与边界

**目标**：把系统服务和默认应用的"业务数据"从"进程自己在数据目录里写文件"改成"结构化数据进 RDB Instance、对象本体进 named store、保留意图经应用对象桶表达"。改造完成后，**除非任务书特殊要求，新开发的服务和应用不再使用文件系统保存业务数据**；§1.2 的规则进入 harness 检查清单。

**不在范围内**（继续用文件系统，理由见 §3 各表的"保留 FS"行）：

- 内核层：System Config、`security/`、kmsg 的 sled 队列（`local/kmsg`）、kevent 共享内存。
- 用户明确以文件夹组织的数据：`/home/<user>/Desktop`、`Downloads`、Files 的用户目录（方向文档 §7.2）。
- Agent 自己的电脑：AgentRoot 的 state/memory/notebook/skills、Session 工作目录及 `runs/`（方向文档 §7、session-runtime 解耦决策）。
- 传统应用兼容的数据目录（方向文档 §7.1）。

**不做兼容**：按 `AGENTS.md` 与"DV 可全新安装"的既定规则，删除旧保留表、旧目录、旧 pin owner 时不写迁移代码，DV 用 `uv run src/start.py --all` 重装。遇到必须留旧数据的情况在任务结果里写明，由用户决定。

## 1. 给 code agent 的硬规则

### 1.1 数据归类四问（每个要落盘的数据项都先回答）

| 问 | 答案 | 去处 |
| --- | --- | --- |
| 是结构化业务记录、要按字段查询吗 | 是 | `buckyos-api::rdb_mgr` 的 RDB Instance；表里存 ObjId 或 `(bucket_id, key, version)`，不存本机路径 |
| 是一个不可变的对象本体（消息、动态、包、便签、快照）吗 | 是 | named store `put_object` + **`object` 桶绑定** |
| 是非结构化字节（上传文件、图片、产物、导入目录）吗 | 是 | named store（上传链路 / `put_file`）+ **`data` 桶绑定** |
| 必须有文件系统语义吗（用户文件夹、可执行、被外部程序按路径打开、Agent 工作目录） | 是 | 文件系统，且在任务结果里写明理由 |

三条推论：

- **写进 named store 的每一处都必须回答"谁 pin"**。答案只能是"某个桶"或"某个短期 Lease（处理期间持有，≤24h）"。不允许新增"只 put 不 pin"的代码；§3 列出的既有只 put 不 pin 的写入方是本 TODO 的改造对象。
- **ObjId 不带保留、不带读权**（设计 §3.4）。需要保留就绑定到桶，需要让别人读就授权桶/前缀或签令牌；把 ObjId 单独存进 RDB 或传给别人都不建立保留。
- **系统服务默认只有一个 RDB Instance**，多用户靠 `owner` 列隔离，不按用户建库；用户数据的搬迁、清除、隔离靠导入 / 导出接口（"user data mgr"），不靠目录或文件（用户裁决 Q1）。

### 1.2 新开发的服务 / 应用默认禁止

- 在 `get_data_folder()` / `get_buckyos_service_data_dir()` 下自建目录保存业务数据；自建内容寻址存储（`objects/`、`chunks/` 之类）。
- 不经 `rdb_mgr` 按路径打开 SQLite。
- 在 RDB、system-config、RPC 返回值里保存本机路径作为数据引用（`"path": "/opt/buckyos/..."`）。
- 为了调用某个工具先把对象落成临时文件再按路径传（方向文档 §8.1）；工具的数据访问层应接受 `obj://` / ObjId。
- 各服务自己实现一套"ObjId → 浏览器 URL + 鉴权"的下载端点（见 §1.4）。

允许：`/tmp/buckyos/<svc>`（进程重启后可丢）、Session 工作目录、用户 home 目录（用户文件夹工作流）、`$BUCKYOS_ROOT/etc` 与 `security/`、cache 分区里可重建的派生物。

### 1.3 怎么声明桶（"声明了才有"，D7）

- **系统服务**：在 `src/kernel/scheduler/src/system_config_builder.rs` 对应的 `add_<service>()` 里向 `spec_config.buckets` 插入 `BucketDecl`，与 `spec_config.rdb_instances` 并列。nfs-server 按 `services/<appid>/spec` 懒建（`src/frame/nfs_server/src/bucket/spec.rs`）。服务可以同时声明 `scope: zone`（服务级归属，`zone/<svc>/_/<name>`）和 `scope: user`（按用户隔离，`user/<svc>/<user>/<name>`，首次访问时为该用户懒建）。RBAC 默认策略已放行 `frame` 对 `obj://bucket/user/{frame}/*` 与 `obj://bucket/zone/{frame}/*` 的全部动作（`rbac_config.rs:217-226`）。
- **应用**：`dapp_meta/app.json` 的 `install_config.buckets`，范本 `src/apps/jarvis_agent/dapp_meta/app.json:29-35`。
- 每个桶必须显式 `kind`（`data` / `object`）与 `data_policy`（`documents` / `app_data` / `collected` / `ambient` / `temp` / `cache`，语义见设计 §4.5）。声明了 `app_data` 或 `temp` 桶必须同时声明 `storage_quota_mib`（D17）。`kind` / `data_policy` / `scope` 创建后不可改。
- 选档口诀：用户创作或明确要求保存的 → `documents`；App 替用户生成、重建昂贵 → `app_data`；别人创作、我认真收藏 → `collected`（必带 `origin`）；别人创作、最好能有 → `ambient`；处理中间产物 → `temp`（必填 TTL ≤30 天）；可重建派生物 → `cache`（不 pin）。
- 桶对 zone 其他用户默认无权限（v0.5）。服务代用户读（如 msg-center 按会话成员关系）或签单 key 只读令牌（`bucket.create_access_token`）。

### 1.4 读取一律 URL 化

- 浏览器：`GET /bucket/v1/<bucket_id>/<key>`，同源带 SSO cookie 即可放进 `<img src>`；跨主体用 `bucket.create_access_token` 的 `?token=`。websdk `BucketClient.objectUrl(bucketId, key, {token})`。
- Agent / CLI：`obj://bucket/<bucket_id>/<key>`；`buckyos bucket ls|stat|put|get|rm|link`。
- 服务间：Rust `BucketClient.get/head` 拿 Binding 的 ObjId 后经 named store 读。
- `ambient` 桶要先 `bucket.head` 看 `availability`，不要直接放 `<img>`。

### 1.5 写入链路

- 浏览器上传：`ndm.startUpload(..., { bind: { bucketId, key, ... } })` 或 `ensureUploaded`，finalize 时 `auto_bind`，结果带 `binding`。**不要**再 `importFiles` 之后单独调 `bucket.put`（两段之间崩溃会漏绑定）。
- 小对象（≤32 MiB）：`PUT /bucket/v1/...` 或 websdk `putBytes`、CLI `bucket put`。
- Rust 服务：`BucketClient.put_file(runtime, bucket, key, path, content_type)`（`StoreInNamedMgr`，成功即持久）；已有 ObjId 用 `put(PutRequest{key, obj_id, conditions})`。绑定要求持有证明（D6）：本主体 put 过、或对已绑定它的桶有 read、或有 `obj://ndm/objects` 读权（frame 服务满足第三条）。
- 对象桶：`put_object` 后 `bucket.put(key, obj_id)`；`object` 桶的子项级联目前只认 Dir / File / ChunkList（设计 §3.3 现状），**消息的附件要单独绑进 `data` 桶**，不能指望绑消息对象就保住附件。

### 1.6 其它必须遵守

- 写代码前先把该服务的持久数据格式文档（`doc/<module>/*Durable Data Schema*.md`，无则新建）按 `harness/SKILLS/design-durable-data-schema` 更新"Storage Strategy"：哪些表、哪些桶、key 方案、data_policy、谁 pin。修改持久格式触发 `harness/trigger_rules.md` §3.2 的检查。
- 不加"旧目录还在就读旧目录"的回退分支。
- 改动了用户可见的 CLI / API / 路径时，按 `AGENTS.md`「cmd_skills 联动维护」更新手册（`buckyos_app_dev`、`opendan_agent_dev`、`buckyos_admin`）。
- 日志：每次绑定 / 解绑写一行 info，含 bucket_id、key、obj_id、version。

## 2. 现状总览（2026-10-26 盘点）

named store 的 GC 宿主在 nfs-server（`src/frame/nfs_server/src/bucket/tasks.rs`），驱逐开关 `services/nfs-server/settings.named_store_gc_evict` 默认 **false**，因为下面这些写入方只 put 不 pin，开了会删它们的内容。**本 TODO 的终点就是把这个开关默认打开。**

| 只 put 不 pin 的写入方 | 位置 | 本 TODO 的处置 |
| --- | --- | --- |
| msg-center 消息对象 `store_message` | `src/frame/msg_center/src/msg_center.rs:946-976` | P1-2 object 桶 |
| HomeStation 预取的远端附件、spider 快照 | `src/frame/homestation/src/resources.rs:100-140,225-240`、`spider.rs:329-346` | P2-2 ambient / collected 桶 |
| control-panel 包正文与 NDN 下载的 App | `app_install_driver.rs:1403-1430`、`ndn_download.rs` | P2-3 |
| repo-service 包内容（`pin`/`unpin` 只改 DB 状态） | `src/frame/repo_service/src/service.rs:371-421` | P2-4 |
| AICC 生成物 `write_artifact` | `src/frame/aicc/src/resource/mod.rs:744-781` | P2-1 |
| OpenDAN 回复附件 ≥12 KiB 的 LocalLink | `src/frame/opendan/src/objects.rs:14-16,74-78` | P1-4 |

既有自建保留机制（将被桶替换）：

| 机制 | pin owner | 落点 |
| --- | --- | --- |
| msg-center `attachment_retention.rs`（三张表 + 60s reconcile） | `msg-center:<msg_id>` | P1-2 |
| HomeStation `retention.rs`（per-user sqlite 两张表 + 600s reconcile） | `homestation:<entry>` | P1-3 |
| AIWorkspace `FsObjectStore`（自己的 `objects/` `chunks/` 目录 + `retentions` 表 + mark-sweep GC，导入资产复制字节、不 pin） | 无 | P1-1 |
| nfs-server ShareMgr `share_retentions` | `share:<share_id>` | P3-1 |

短期 Lease（处理期间持有，**保留不动**）：`ndm-hold:<user>|<app>`（24h）、`nfs-import:<uuid>`（1h）、`control-panel:pikg-import:<uuid>`（24h）、`aicc:input:*`（24h 续租）、tus 上传会话。

## 3. 逐服务清单与目标形态

表中 bucket_id 用设计 §4.2 的格式；`_` 表示 zone 归属。key 方案是建议，实施时可调但要写进该服务的持久数据格式文档。

### 3.1 msg-center（`src/frame/msg_center`）

| 数据 | 现状 | 目标 |
| --- | --- | --- |
| 附件（FileObject） | NDM + `msg-center:<msg_id>` pin + 三张保留表 | `zone/msg-center/_/attachments`，`data` / `app_data`，key `<session>/<msg_id>/<n>`；不授权任何用户；私聊永不释放 = 不写 lifecycle |
| 消息对象 MsgObject | NDM `put_object`，不 pin | `zone/msg-center/_/messages`，`object` / `app_data`，key `<msg_id>`；群撤回 / 清除 = `bucket.delete` 后 `remove_object` |
| 附件读取 | `GET /kapi/msg-center/objects/{obj_id}[/content]` 代读 | **保留代读入口**（Q5 已定）：权限在会话成员关系；内部改为先 `bucket.get` 校验该附件确属此消息，再经 named store 读。单 key 只读令牌只用于外部分享 |
| 收藏附件到用户空间 | 无 | MessageHub "收藏" = `bucket.copy` 到 `user/<messagehub appid>/<user>/collected`（`collected`，`origin` 由发送者 DID + msg id 填） |
| TG 会话文件 `data/var/msg-center/tg_sessions` | grammers SQLite | **保留 FS**（第三方库的凭据文件，按路径打开） |

### 3.2 HomeStation（`src/frame/homestation`）

| 数据 | 现状 | 目标 |
| --- | --- | --- |
| 已发表内容的附件 | NDM + `homestation:<entry>` pin + per-user sqlite 保留表 | `user/homestation/<user>/published`，`data` / `documents`（用户创作、永久、备份、Files 投影 `Apps/HomeStation/published/`），key `<entry>/<version>/<n>`；撤回 = 解绑 |
| Feed Object 本体 | 只在 per-user sqlite `objects` 表 | `user/homestation/<user>/entries`，`object` / `documents`，key `<entry>/<version>`；sqlite 只留索引与 ObjId |
| 别人 Zone 的动态与附件（订阅、预取） | NDM `put_chunk`，不 pin | `user/homestation/<user>/following`，`data`+`object` 各一，`ambient`（必带 `origin`）；v1 "有则用、无则 404" |
| spider 抓取的网页快照 | NDM，不 pin | 发表前是处理中间产物 → `temp`（7 天）；发表时 `bucket.copy` 到 `published` |
| 封面、缩略图等派生物 | 混在上面 | `cache` 桶 |
| per-user `homestation.db`、`zone.db` | rusqlite 按路径 | **不是桶的事，但违反 §1.2**：合并为一个 `rdb_mgr` 实例 `homestation-main`（表加 `owner` 列，zone feed 也在同库），用户数据的搬迁靠导入 / 导出（Q1 已定） |

### 3.3 AIWorkspace（`src/frame/aiworkspace`）— 设计 D9 指定的首个迁移对象

| 数据 | 现状 | 目标 |
| --- | --- | --- |
| 资产字节（`FsObjectStore` 的 cyfile） | 自建 `objects/` `chunks/`，复制字节 | `user/aiworkspace/<user>/assets`，`data` / `app_data`，key `<workspace_id>/<asset_id>`；`doc.sqlite.assets` 只存 ObjId 与 (key, version) |
| 快照、内容根、程序对象（jobj） | `FsObjectStore` | `user/aiworkspace/<user>/objects`，`object` / `app_data`，key `<workspace_id>/<kind>/<id>` |
| `asset.import_object`（从 NDM 导入） | 读整个 FileObject 再复制进 FsObjectStore | 不复制：`bucket.put(key, obj_id)`（持有证明走"对已绑定它的桶有 read"或 frame 读权） |
| 上传 staging `staging/<up_id>.part` | FS | 改走 `ndm.startUpload({bind})` 直达 `assets` 桶；服务端 `upload/<id>` 端点与 `import-receipts/*.json` 删除 |
| `.bcanvas` 导出、replica sqlite | FS，RPC 返回本机路径 | `user/aiworkspace/<user>/exports`，`data` / `temp`（7 天）；RPC 返回 `(bucket_id, key)`，前端用 `objectUrl` 下载 |
| 许愿格 run 的 `work/`、`output/`、`llm_map` 缓存 | FS（Deno 工作目录） | **保留 FS**（执行器工作目录）；但 run 结果文件在 apply 时绑进 `assets`，run 详情 JSON 存 (key, version) 而不是绝对路径 |
| `workspaces/<id>/doc.sqlite`、`local.sqlite`、show 克隆、trash | rusqlite 按路径 | 本 TODO 不动。Q2 已定：另起 TODO 处理，出发点是"单个 Workspace 在抽象上基本等同于单个 App"，应按 App 的方式拿自己的 RDB Instance 与桶，而不是在 aiworkspace 服务目录下按路径开库 |
| 自带 mark-sweep GC、`retentions` 表 | 存在 | 删除 |

### 3.4 OpenDAN / Agent（`src/frame/opendan`、`src/frame/lib_opendan`、`src/tools/buckyos-agent`）

| 数据 | 现状 | 目标 |
| --- | --- | --- |
| 报告产物 | `put_file` + 绑进 `user/<agent>/<owner>/artifacts`（`app_data`）；≥12 KiB 用 `LocalFile` 链接 Session 目录里的稳定副本 | `artifacts` 桶改为 **`documents`** 档（Q3 已定：归用户、卸载 Agent 不删、必备份、Files 投影到 `Apps/<Agent>/artifacts/`、计用户配额、默认 versioning）；一律 `StoreInNamedMgr`（成功后 Session 目录副本可删）；绑定失败不再只 warn（按 §1.5 "成功即持久"，失败要进 report 的错误） |
| 回复 `<attachment path>` | `put_file` 工作目录活文件，大文件 LocalLink | 同上，先入库再 compose_reply；`RefItem::DataObj` 同时带 `obj://bucket/...` |
| AIGC 输出、`materialize_resource` | buckyos-agent 写本地文件，`uploadNamed` 未实现 | 实现 `uploadNamed`：输出直接 `bucket put`（或 `startUpload({bind})`）到 `artifacts`，命令返回 `obj://bucket/...`；`materialize_resource` 保留为"工具需要路径时的显式降级" |
| 入站附件 | ObjId，`attachment_path` hook 无人实现 | 保持 ObjId；`read cyfile:` 已可读；不补 hook |
| AgentRoot state/memory/notebook/skills、Session 工作目录、`runs/`、`usage.jsonl` | FS | **保留 FS**（Agent 的电脑） |
| `llm_understand_media` / `image_fit` 临时文件 | OS temp | 保留（处理期间，用完即删） |
| Jarvis 模板声明 | 只有 `artifacts`（app_data） | `artifacts` 改 `documents`（去掉 `quota_mib`，不能再按 App 配额限制）；增加 `scratch`（`temp`，7 天）给中间产物，`storage_quota_mib` 只约束 `scratch`。不另设 `library` 桶 |

### 3.5 control-panel（`src/frame/control_panel`）

| 数据 | 现状 | 目标 |
| --- | --- | --- |
| 壁纸 | `data/home/<user>/.local/share/control-panel/wallpapers/<sha>.<ext>`，自写 `/api/desktop/wallpaper/<asset>` 端点 | `user/control-panel/<user>/wallpapers`，`data` / `app_data`，key `<sha256[:32]>.<ext>`；appearance 存 key；前端 `objectUrl` 直接 `<img src>`；删除自写端点与 prune 逻辑（保留 3 张改为 App 逻辑 `bucket.delete`） |
| Agent 模板 PIKG `data/srv/control-panel/agent_templates/<owner>/<app>/<digest>.pikg` | FS，KV 记录存相对路径 | `zone/control-panel/_/agent_templates`，`data` / `app_data`，key `<owner>/<app_id>/<digest>.pikg`；记录存 (key, version) |
| 已安装包正文、NDN 下载的 App | NDM，不 pin | `zone/control-panel/_/packages`，`data` / `app_data`，key `<app_id>/<version>/<digest>`；卸载 = 解绑 |
| PIKG staging（从 NDM 复制成文件再解析） | `cache/control_panel/pikg_staging` | 安装驱动直接从 named store reader 读；不再 `finalize_named_object` 落文件。staging 目录只留 `incoming/`（外部投放）|
| App 发布的临时 tar、诊断 zip、日志 zip | OS temp / cache | 保留（处理期间） |
| 用户 / Agent / Zone 头像 | system-config data URL（≤128 KiB） | 不动 |
| `Desktop/*.lnk`、`*.widget`、`Downloads/` | 用户 home | **保留 FS**（用户文件夹语义） |
| App 卸载删数据 `app_installer.rs:946-995` | 按 manifest 删目录 | 补：按 data_policy 处置该 App 的桶（`app_data` / `temp` / `cache` 删，`documents` / `collected` 转孤儿桶） |

### 3.6 AICC（`src/frame/aicc`）

| 数据 | 现状 | 目标 |
| --- | --- | --- |
| 生成物（图、音、视频、embedding） | NDM，不 pin，靠调用方绑定 | AICC 自己声明 `user/aicc/<user>/outputs`，`data` / `temp`（7 天，Q6 已定）：`write_artifact` 成功即绑定，结果里带 `obj://bucket/...`；调用方要长期保留就 `bucket.copy` 到自己的桶。这样 Desktop AI Center 等不经 OpenDAN 的消费者也不会丢 |
| 输入 Lease `aicc:input:*` | 24h 续租 | 保留 |
| `data/aicc/driver_metadata/cloud` | FS 缓存 | 保留（cache 分区，可重建）或改 `cache` 桶，二选一按改动最小 |

### 3.7 repo-service（`src/frame/repo_service`）

| 数据 | 现状 | 目标 |
| --- | --- | --- |
| 包内容 | NDM，`pin`/`unpin` 只改 DB | `zone/repo-service/_/packages`，`data` / `app_data`，key `<pkg_name>/<version>`；DB 的 `REPO_STATUS_PINNED` 与桶绑定同事务 |
| `pending_announces/*.json` | FS 队列 | RDB `repo-service-main` 新表 |
| `ready_check_store`（无 runtime 时的私有 store） | dev 回退 | 删除或保留为 `--dev` 专用，不进生产路径 |

### 3.8 nfs-server（`src/frame/nfs_server`）

| 数据 | 现状 | 目标 |
| --- | --- | --- |
| ShareMgr 快照与 `share:<share_id>` 保留 | filedb 表 + pin | `zone/nfs-server/_/shares`，`data` / `app_data`，key `<share_id>`；`share_retentions` 删除，sweeper 删除（P3-1） |
| tus 上传 `.fsdb/ndm_uploads/*.part`、`nfs-import` 物化 | FS 临时 | 保留（处理期间） |
| `filedb.sqlite`、`bucket-index` 按路径打开 | 经 rdb_mgr 登记但 rusqlite 打开 | 不动（设计 §14.1 已定） |

### 3.9 默认 Web 应用（`src/frame/desktop`、`src/apps`）

| 数据 | 现状 | 目标 |
| --- | --- | --- |
| MessageHub / HomeStation / AI Center / AIWorkspace 上传 | 各自 `ndm.importFiles` → `ensureUploaded`，无 `bind` | 统一走 `startUpload({ bind })`，目标桶由各服务按 §3.1–3.3 提供 |
| ObjId → 预览 URL | msg-center blob URL、HomeStation `access=<token>`、Preview 组件 POST 读 Blob，三套 | Preview 组件 / 公共层新增 `obj://bucket/` source：`objectUrl` 直取；只有 msg-center 代读保留 |
| Text Editor 的 `buffers/`、`recovery/`（NFSP 写用户 app data 目录） | FS | `user/text-editor.../<user>/buffers`、`recovery`（`data` / `app_data`，`putBytes`）；作为"默认 Web 应用不用文件系统"的样板（Q4 已定）。用户打开的文件本身仍是 Files 里的文件 |
| sys_test | 只演示 NDM 导入 + commitObject | 增加"上传直达桶 + objectUrl 读取"的用例，作为 DV 冒烟 |

## 4. 分期任务

每个任务：先改持久数据格式文档 → 改 spec 声明 → 改代码 → 删旧机制 → 单测 → DV（§6）。任务编号用于任务结果与提交说明。

### P0 公共补缺（先做，否则各迁移项都要各自绕）

- [x] **P0-1 websdk 上传统一**：`importFiles` / `pickupAndImport` 的 `autoStartUpload` 支持透传 `bind`；增加 `BucketId` 构造辅助（现在调用方手拼 `user/<appid>/<owner>/<name>` 字符串）；`ensureUploaded` 返回值直接带 `binding`。Desktop 四处上传改用。
- [x] **P0-2 Desktop 公共 ObjId 读取**：`src/frame/desktop/src/components/preview` 支持 `obj://bucket/<id>/<key>` 来源（`objectUrl` + 可选 token）；提供公共 `bucketObjectUrl()`；`doc/key url.md` 登记 `obj://bucket`。
- [x] **P0-3 Rust `BucketClient` 补便利方法**：`put_reader`（流式，不落盘）、`put_json_object`（object 桶：`put_object` + 绑定一步）、`copy` 的 `origin` 参数；`put_file` 增加 `conditions` 参数。服务侧不再出现"先 put_object 再想起绑定"的两步代码。
- [x] **P0-4 cyfs-ndn 并行项**：`parse_obj_refs` 接入 `MsgObject` 等标准对象子项遍历（D1 前提）；chunk 字节回收（`gc_round` 扫 `chunk_items`）。本仓用 `[patch]` 指同级 `cyfs-ndn` 联调，发布先推 cyfs-ndn 再 `cargo update`。没做完不阻塞 P1，但 P4-5 开驱逐前必须完成。
- [x] **P0-5 App 卸载与孤儿桶**：control-panel 卸载流程按 data_policy 处置桶（§3.5 末行）；存储空间页列出每 App 的桶用量（`bucket.usage_report`）。

### P1 首批迁移（已经是 NDM + pin 的，只换保留机制）

- [x] **P1-1 AIWorkspace**（§3.3）：删 `store/src/objects.rs` 的 `FsObjectStore`、`service.rs` 的 GC 与 `collect_roots`、`schema.rs` 的 `retentions`；声明 `assets` / `objects` / `exports` 三桶 + `storage_quota_mib`；`import_object` 改绑定；上传改 `bind`；导出 RPC 返回 (bucket_id, key)。e2e 基线按 `aiworkspace-kernel` 记录的 161/170 核对。
- [x] **P1-2 msg-center**（§3.1）：声明 `attachments` / `messages` 两个 zone 桶；`store_message` 改 `put_json_object`；收消息时附件绑定与消息绑定在同一处（§1.5：各自绑定）；删 `attachment_retention.rs` 与三张表；代读入口改经 `bucket.get` 校验。Durable Data Schema 文档 `doc/message_hub/Message Center Durable Data Schema.md` 同步。
- [x] **P1-3 HomeStation**（§3.2 前四行）：声明 `published` / `entries` / `following` / `cache` / `scratch`；删 `retention.rs` 与两张表；`register_object` 改绑定；`NdmChunkStore` 的 pin/unpin 删除；`FsChunkStore`（standalone）删除，standalone 用 `--bucket-specs` 的 nfs-server。sqlite → RDB 另列 P3-2。
- [x] **P1-4 OpenDAN 产物**（§3.4 前三行）：`objects.rs` 去掉 LocalLink 分支；绑定失败进错误；`outbound.rs` 的回复附件先入库；buckyos-agent 实现 `uploadNamed` 并让 AIGC 命令默认直传桶；Jarvis 模板 `artifacts` 改 `documents` 并增加 `scratch`（temp 7 天）桶；更新 `opendan_agent_dev.md`、`aigc_*.md`。

### P2 给"只 put 不 pin"的写入方补归属

- [x] **P2-1 AICC outputs 桶**（§3.6）：`write_artifact` 成功即绑定 `user/aicc/<user>/outputs`（temp 7 天）；`StoredFile` 增加 `bucket_ref`；OpenDAN 收到后 `bucket.copy` 到 `artifacts`，不再重复 put。
- [x] **P2-2 HomeStation 订阅与抓取**（§3.2 后三行）：预取改 `ambient` 桶绑定（带 `origin`）；spider 快照改 `temp`，发表时 copy。
- [x] **P2-3 control-panel**（§3.5）：壁纸、Agent 模板、包正文三项；PIKG staging 改直读 named store。Desktop 壁纸前端改 `objectUrl`，删 `/api/desktop/wallpaper` 端点。
- [x] **P2-4 repo-service**（§3.7）：包绑定、announce 队列进 RDB。

### P3 收尾与剩余文件系统项

- [x] **P3-1 ShareMgr 保留迁到桶**（§3.8）；`bucket.snapshot` 已复用其快照构造，迁后 ShareMgr 不再自己管 pin。
- [x] **P3-2 HomeStation sqlite → 单个 RDB Instance `homestation-main`**（owner 列；`system_config_builder.rs::add_homestation` 登记；standalone 模式用本地 sqlite 路径的 rdb 配置）。
- [x] **P3-3 Text Editor 缓冲区与恢复区改桶**（`app_data`，`putBytes`；app.json 声明两桶 + `storage_quota_mib`）。
- [ ] **P3-6 另起 TODO：AIWorkspace 的 Workspace 按 App 对待**（Q2）：每个 Workspace 自己的 RDB Instance 与桶，而不是 aiworkspace 服务目录下的 `doc.sqlite` / `local.sqlite`；涉及 rdb_mgr 动态实例、Loro 持久化后端、show 克隆与 trash 的语义，不在本 TODO 内实施。
- [x] **P3-4 D11 Files 投影**：`documents` / `collected` 桶只读投影到 `cyfs:///home/<user>/Apps/...`、`Collected/...`（设计 §4.5.4，§20 未做项）。HomeStation `published` 迁到 documents 后用户在 Files 里应能看到。
- [x] **P3-5 D18 令牌格式统一**（2026-10-26 用户修订 D18）：不并表；`bkt_`、`gt_` 访客令牌与群 48h 链接只共用令牌格式与校验代码（buckyos-base `name_lib::bearer_token`），各自仍在原服务签发与撤销（设计 §4.4.5）。

### P4 规则落地与开关

- [x] **P4-1 文档回写**：`doc/path_usage.md`「系统服务视角 / 服务常用的目录」改为"业务数据进 RDB / 桶，目录只剩 tmp 与 cache"，并把本 TODO §1 的归类四问写进去；方向文档 §10.1 的"资源清单"用 §3 填上；设计文档 §20 更新。
- [x] **P4-2 harness**：`harness/SKILLS/design-durable-data-schema` 的 Storage Strategy 增加"Object / Unstructured → 桶（kind、data_policy、key 方案、谁 pin）"小节；`harness/checklists/System Service Delivery Checklist.md` 增加检查项："无 `std::fs` / `tokio::fs` 业务数据写入；每个 named store 写入有对应桶绑定；RPC 不返回本机路径"；`harness/trigger_rules.md` §3.2 触发条件加"新增文件系统数据目录"。
- [x] **P4-3 cmd_skills**：`buckyos_app_dev.md` 的存储节改为"默认桶、特殊要求才用目录"；`opendan_agent_dev.md` 同步 P1-4；`buckyos_admin.md` 增加 `buckyos bucket` 诊断与 `named_store_gc_evict`。
- [x] **P4-4 beta2.2 兼容清单**：`doc/arch/beta2.2-data-compatibility.md` F08 / F14 / F15 / F11 按新形态改写"内容"与"兼容策略"。
- [ ] **P4-5 打开驱逐**（2026-10-26：默认值已改 `true`，写入方已逐个核对；DV 验证待做）：确认 `named_store` 里不再有 §2 表中的无主对象（§6 验收脚本），`NfsServerSettings.named_store_gc_evict` 默认改 `true`；在 DV 上验证解绑 → 收敛 → `owned_bytes` 下降。
- [ ] **P4-6 清理旧 pin owner**（2026-10-26：代码中常量已删；全新安装上的 `list_pins_by_owner` 待 DV 验证）：`list_pins_by_owner("msg-center:")`、`"homestation:"`、`"share:"` 在全新安装上必须为空；代码里这些 owner 常量删除。

## 5. 实施顺序与并行

```
P0-1 ─┬─ P1-1 AIWorkspace ─┐
P0-2 ─┤                    │
P0-3 ─┼─ P1-2 msg-center ──┼─ P2-* ─ P3-* ─ P4-1..4 ─ P4-5 ─ P4-6
      ├─ P1-3 HomeStation ─┤
      └─ P1-4 OpenDAN ─────┘
P0-4 cyfs-ndn（并行，P4-5 前完成）
P0-5 卸载/孤儿桶（并行）
```

P1 四项互不依赖，可以分给四个 agent 并行；每个 agent 一个服务，不跨服务改动。P2-1 依赖 P1-4（OpenDAN 改 copy）。

## 6. 验收

每个任务至少：

1. `cargo test -p <package> -- --test-threads=1` 与受影响的 websdk / desktop 测试通过。
2. 该服务源码里 `grep -n "std::fs\|tokio::fs\|create_dir_all\|get_data_folder\|get_buckyos_service_data_dir"` 的剩余命中逐条对应 §3 的"保留 FS"行或处理期间临时文件，写进任务结果。
3. 该服务不再有 named store 写入没有对应绑定：`grep -n "put_object\|put_chunk\|cacl_file_object"` 的每一处在同一函数或同一事务里能看到 `bucket` 绑定或短期 Lease。
4. DV（`uv run src/start.py --all` 后）：
   - `buckyos bucket ls` 看到该服务声明的桶；`buckyos bucket stat <bucket_id>` 的 key 数与业务记录数一致；
   - 业务流程走一遍（发消息带附件 / 发动态 / 导入资产 / Agent 交付产物），浏览器用 `/bucket/v1/<id>/<key>` 能读；
   - 删除业务记录后 `bucket stat` 的 key 数下降；`list_pins_by_owner("bucket:<id>")` 与索引一致（`bucket.verify_references`）；
   - 旧目录（`objects/`、`wallpapers/`、`agent_templates/`…）在全新安装上不再被创建。
5. P4-5 之后：DV 上 `named_store_gc_evict = true`，重启，业务数据无丢失；解绑一批对象后 `gc_round` 使 `owned_bytes` 下降。

证据目录沿用既往做法：`/root/.claude/jobs/<job>/evidence/<task>/`，任务结果里给出路径。

## 7. 裁决（用户，2026-10-26）

| # | 问题 | 裁决 | 落点 |
| --- | --- | --- | --- |
| Q1 | HomeStation per-user sqlite 改 RDB：一库多用户还是每用户一库 | **系统服务都倾向一个 instance**；用户数据的搬迁 / 隔离通过导入导出实现（"user data mgr"） | §1.1 第三条推论、§3.2、P3-2 |
| Q2 | AIWorkspace 每工作区 `doc.sqlite` / `local.sqlite` 是否本轮改 | **本轮不改，单独起 TODO**：单个 Workspace 在抽象上基本等同单个 App | §3.3、P3-6 |
| Q3 | Agent 产物档位 | **进 `documents`**：`artifacts` 桶改 documents，不另设 `library`；中间产物用 `scratch`（temp） | §3.4、P1-4 |
| Q4 | Text Editor 缓冲区与恢复区是否改桶 | **改**（`app_data`） | §3.9、P3-3 |
| Q5 | msg-center 附件读取方式 | **保留代读端点**；令牌留给外部分享 | §3.1、P1-2 |
| Q6 | AICC outputs 桶 TTL 与 scope | **`user` scope，7 天** | §3.6、P2-1 |

## 8. 坑与注意

- `bucket-index` 是 nfs-server 单进程持有的 SQLite；所有绑定都经 kRPC `bucket.*`，服务不要试图直接打开它。
- 持有证明（D6）：服务代用户 `put_object` 的对象，绑定主体是服务自己（frame 读权满足）；但 Web 前端上传的对象要在 finalize 时 `auto_bind`，事后由服务再绑需要服务对 `obj://ndm/objects` 有读权（frame 有，App 没有）。
- `object` 桶绑 `cymsg` 不保护其附件（设计 §3.3 现状），附件必须另绑 `data` 桶；P0-4 做完也保持两边各绑，归属更清楚。
- `cache` 桶不 pin：刚写入就可能被逐出；需要保底窗口的用 `temp`。
- zone 桶（`_` 归属）对 zone 用户默认无权限，用户桶（`user/<svc>/<user>/`）frame 服务全权、该用户全权；给别的用户 / App 读要 `bucket.grant`。
- RBAC 策略占位符名必须等于 `p.sub`（`{users}` / `{app}` / `{frame}` / `{admin}`），别在桶策略里发明新占位符。
- `storage_quota_mib` 对声明了 `app_data` / `temp` 的 App 必填，pikg 校验会拒绝缺失；系统服务 spec 同样要填，否则 `sync_app` 警告。
- 事件走 kevent `/bucket/<bucket_id>`，尽力投递，不要把业务一致性建在事件上。
- 本仓联调 cyfs-ndn 改动：写一个 cargo 配置片段 `[patch."https://github.com/buckyos/cyfs-ndn.git"]` 把 `ndn-lib` / `named_store` / `bucket_store` / `s3_gateway` / `ndn-toolkit` / `package-lib` 指向同级 `../cyfs-ndn/src/<crate>`，用 `cargo --config <该文件> test ...`；发布先推 cyfs-ndn main，再 `cargo update -p bucket_store -p s3_gateway -p named_store`。`src/Cargo.lock` 被 gitignore，patch 前备份、验证后还原。
- 旧 DV 数据不兼容：直接 `start.py --all`，不写回退。
