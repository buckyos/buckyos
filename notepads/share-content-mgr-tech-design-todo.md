# ShareContentMgr 技术设计 TODO：分享链接、Public 投影与管理中心（PRD 第一阶段）

> 创建日期：2026-10-18
> 版本：v0.1（待技术决策）
> 状态：§4 的决策项由用户在开工前拍板；拍板后按 §5 切片实施，实施记录追加到 §9。
> 依据：[ShareContentMgr PRD v0.1](../product/control_panel/ShareContentMgr_PRD.md)、[分享/发布/上线产品设计](<../doc/control_panel/BuckyOS 的分享_发布_上线 功能的产品设计.md>) §17–§18、[讨论记录](<../doc/control_panel/BuckyOS 内容分享，发布，上线功能讨论记录.md>)、[旧 ShareContentMgr 设计](../doc/control_panel/ShareContentMgr.md)（只借用修订号、分页与统计思路），以及 2026-10-18 对 buckyos / cyfs-ndn / cyfs-gateway / buckyos-websdk 四个仓库的源码核对。
> 第 2 节“现状”只以源码为准。PRD 里要求但代码没有的能力，一律标明“不存在”，不按文档推断。

## 1. 目标与范围

PRD 第一阶段要求交付“普通分享的完整生命周期”和“受支持网站或服务的上线衔接”。本设计把第一阶段拆成可独立验收的五块，并为第二阶段（独立内容发布、长期入口、收录者）预留数据模型，但不实现。

本轮做：

1. **ShareContentMgr 服务**：control-panel 内的新模块，维护操作、结果、内容版本、保留、授权、审计记录；提供 `share.*` kRPC。
2. **分享链接闭环**：从文件/目录生成不可变 NDN 快照，建立保留（pin），签发持链接者或密码访问的分享链接；访客经 control-panel 的分享提供服务读预览、下载；到期和停止由服务端执行。
3. **Public 投影**：观察 `data/srv/publish/`（gateway 已把 `public.<zone>` 指向它）并在管理中心展示；首期不做 NDN 对象化。
4. **通过应用分享**：首期只接 MessageHub 一个内置目标，走快照 → obj_id → MessageHub 附件引用 → 回传。
5. **前端**：系统分享面板 `sysdlg/share`、File Browser 入口、“分享与发布”管理中心、访客页面。上线衔接只做入口路由到既有 `sysdlg/app_installer` 与实例投影。

不做（PRD 第二、三阶段或明确排除）：独立内容发布与长期入口切版、Public 的 NDN 对象化与“保存为独立发布”、第三方分享目标注册、Static Web 本地打包 PIKG、收录者、BNS、交易、访问统计的真实采集管线（只留表）。

## 2. 现状（2026-10-18 核对）

### 2.1 control-panel 的 handler 约定

- 分发：`src/frame/control_panel/src/main.rs:853-1069` 的 `impl RPCHandler for ControlPanelServer` 先取 token（`:859`）、调 `authenticate_rpc_request` 得到 `principal`（`:863`），再按点分方法名一个大 `match`（`"user.*"`、`"agent.*"`、`"apps.*"`、`"download.create"`、`"zone.*"`、`"ui.*"`），未知方法返回 `UnknownMethod`（`:1066`）。
- 管理器不是独立 struct：每个 `*_mgr.rs` 对 `ControlPanelServer` 写 `impl` 块，`main.rs:1-29` 用 `mod` 引入。签名范式：`pub(crate) async fn handle_xxx(&self, req: RPCRequest, principal: Option<&RpcAuthPrincipal>) -> Result<RPCResponse, RPCErrors>`（如 `download_mgr.rs:55`）。
- 身份：`RpcAuthPrincipal { username, owner_user_id, authenticated_app_id, user_type, owner_did, is_user_session, is_control_panel_session }`（`main.rs:106-114`），由 `sys_auth_backend.rs:1151-1243` 从 verify-hub token 构造。`authenticated_app_id` 形如 `app:<instance>` 或 `system:control-panel`。
- Agent 代表用户：Agent runtime 用 App token 调用，`sub` 是 owner、`target` 是 `app:<agent_id>@<owner>`，control-panel 看到的是 `username = owner`、`authenticated_app_id = "app:<agent_id>@<owner>"`（`agent_mgr.rs:786-820`）。纯 Agent DID token 走不通 control-panel 的用户加载。没有 `on_behalf` 一类的委托字段。
- 授权：control-panel 对自己的方法**不走 RBAC**，各 handler 手工判 `require_self_or_admin` 等（`user_mgr.rs:48-86`）。`runtime.enforce` 只有 msg_center 和 aicc 在用。
- 存储：没有任何 control-panel 管理器用 SQLite；用户文件在 `data/home/{user}/…`，服务数据按 [path_usage.md](../doc/path_usage.md) `:85-87` 放 `data/var/<service>`（卸载删）或 `data/srv/<service>`（卸载保留）。
- HTTP：`runner.add_http_server("/api", …)`（`main.rs:1218`）等路径由 control-panel 自己处理；`/sso_*`、`/ndm/*` 在 gateway 硬编码转发到 control-panel（见 §2.5）。

### 2.2 现有 ShareContentMgr 桩：结论是废弃重写

- `src/frame/control_panel/src/share_content_mgr.rs`（1103 行）实现的是 buckyos-api 的 `ContentMgrHandler` trait（`content_mgr_client.rs:714`，service id `publish-content-mgr`，裸方法名 `publish/resolve/...`，只有 `RPCContext` 没有用户），与 control-panel 的 `(req, principal)` 范式不一致，未在 `main.rs` 接线。
- 默认 DB 路径硬编码 `/opt/buckyos/data/share_content_mgr.sqlite3`（`:13`），不认 `$BUCKYOS_ROOT`。
- 表以 `name` 为主键（`published_items`），PRD §5.2 明确反对以名称作为结果身份。
- buckyos-websdk 没有对应客户端，仓库内没有调用者，没有存量数据。因此 §19.2 的“清点迁移”结论是：无需迁移，删除桩与 `content_mgr_client.rs`，可复用的只有修订号、分页上限与 `access_logs/access_stats` 两张统计表的形状。

### 2.3 named store 与 NDN 对象（cyfs-ndn）

- 入口：`get_buckyos_api_runtime()?.get_named_store()`（`buckyos-api/src/runtime.rs:292-311`）返回 `NamedDataMgr`，配置来自 `$BUCKYOS_ROOT/storage/named_store.json`。它是**同一节点上所有服务共享的本地存储**，不跨 OOD 复制。
- 对象与 chunk：`put_object/get_object/open_reader/open_chunk_reader(chunk_id, offset)`（`named_store/src/ndm.rs:456-1236`）。`put_chunk` 单 chunk 上限 32 MiB（`CHUNK_DEFAULT_SIZE`），Range 只支持起点偏移，终点由调用方截断。
- **保留有现成 API**：`pin(obj_id, owner, PinScope::{Recursive|Skeleton|Lease}, ttl)`、`unpin`、`unpin_owner`（`ndm.rs:1279-1310`）。新 put 的对象默认是 GC 候选（eviction_class 0）；目前没有自动 GC 循环，只有 `forced_gc_until`。所以快照必须 pin，否则一旦打开 GC 就会丢。
- 没有磁盘占用/对象计数的统计 API（`current_layout()` 只回显配置）。PRD §15.2 的“真实占用”首期只能用快照累计大小近似。
- 文件 → 对象：复用 `ndn_toolkit::cacl_file_object(store, path, template, use_chunklist, CheckMode::ByFullHash, StoreMode::StoreInNamedMgr, cb)`（`ndn-toolkit/src/tools.rs:156`）与 `cacl_dir_object`（`:375`）。最佳参考实现 `src/frame/opendan/src/objects.rs:21-62`，但它对 ≥12 KiB 文件用 `StoreMode::LocalFile` 链接原文件，**不满足不可变**，分享必须强制 `StoreInNamedMgr`。
- `FileObject { name, size, content, meta(flatten) }`，meta 顶层平铺会与基础字段撞名；`DirObject` 内嵌子 FileObject JSON（`dirobj.rs:172`）。obj_id 是 JCS 规范化后的 sha256。

### 2.4 nfs_server

- 不依赖 ndn-lib。filedb 的 `full_hash` 是整文件 sha256，`stat` 以 `obj_id: "sha256:<hex>"` 返回（`namespace.rs:483`），**不是** NDN ChunkId/FileObject id，快照必须重新计算。
- ACL principal 只有 `public`（只读）、`zone_users`、`user:<id>`（`acl.rs:24-27`）；匿名请求 = `Actor::Anonymous`（`auth.rs:63-76`）。没有“Public 文件夹”概念，根目录从 `data/` 自动发现（含 `home`、`srv`）。
- `GET /nfs/v1/read/{node_id}` 对所有文件都给出 read URL（`namespace.rs:560-577`），Desktop 把它当 `publicUrl`，实际只有带 public ACL 的路径才能匿名读。
- Rust 侧没有 nfs 读客户端；`buckyos-api/src/nfs_copy.rs` 只有 token 校验与任务 schema。nfs-server 的 REST（`/nfs/v1/stat|resolve|list|read`）可以用调用者的 token 直接 HTTP 调用。

### 2.5 gateway 路由

- 规则手写在 `src/rootfs/etc/boot_gateway.yaml`：`get_app_info_from_req`（`:138`）按 host 映射 `APP_INFO`，根域是 `_`；`get_service_info_from_req`（`:192`）硬编码 `/sso_*`、`/ndm/*` → control-panel，`/nfs/v1/*` → nfs-server，`/home/*` → homestation，其余 `/kapi/<service>`。
- **服务路径不做登录检查**（`permit_check` 只查 `block_services`），`check_oauth` 只对 App host 生效。服务自己鉴权。
- `public.<zone>` → `dir` 服务 `../data/srv/publish/`（`:76-79`、`:116-118`），与 `path_usage.md:20,118` 的“zone 级别的分享数据”一致。这就是 PRD 所说的 Public 目录的现成落点。
- scheduler 把 `_`、`www`、`sys`、`homestation` 四个 host key 都映射到 control-panel（`scheduler/src/system_config_agent.rs:1306-1324`），TLS 配置含 `*.<zone>` 通配（`:1371`）。新增一个 `share` host key 可以同法加入。
- cyfs-gateway 的 `cyfs-dir` 服务（`cyfs-gateway-lib/src/server/cyfs_dir_server.rs`）有 process-chain 前置钩子可 `reject`，但 buckyos 的 gateway 配置**没有实例化它**；内层 `NdnDirServer` 无鉴权、无 Content-Type/文件名、Range 只认起点。`named_inbox_cache_server` 是入站 PUT 收件箱，与读缓存无关。

### 2.6 已有的“带授权的对象读取”先例

- msg_center：`GET /kapi/msg-center/objects/{obj_id}[/content]`（`msg_center/src/object_access.rs`），Bearer 或 `?token=`，按消息 refs 判权，整段缓冲 64 MB 上限，无 Range。
- homestation：`/home/<user>/objects/{id}[/content]`（`homestation/src/http.rs:288-331`），per-object grants，“知道 ObjId 不授予任何权限”，单 chunk。
- 两者都说明：分享提供服务在 control-panel 内自己读 named store 并鉴权，是本仓库的现行做法。

### 2.7 Desktop

- 免登录路由：`src/frame/desktop/src/publicRoutes.ts` 列出 `/login`、`/userprofile`、`/workspace/<id>`、`/homestation/<user>`；`main.tsx:64-80` 对这些路径不跳登录。
- AI Workspace 访客令牌：浏览器自己生成 `gt_` + 26 位随机（`aiworkspace/api/transport.ts:36-45`），服务端 sha256 后做 `guest:<id>` principal（`aiworkspace/store/src/workspace.rs:50-57`）。**访客页与控制台同源**，共享 localStorage 与 host-only 的 `buckyos_session_token` cookie（`sys_auth_backend.rs:707-715`），只有服务端授权隔离。
- File Browser：菜单已有 “Share…” 项，但命令硬连到 `unsupported`（`filebrowser/menu/commands.ts:14`）；“Copy public URL” 只在 `publicUrl` 存在时显示；权限对话框是真实的 `get_acl/set_acl`。
- System Dialog：只有 `sysdlg/app_installer`（`desktop/src/sysdlg/index.tsx`），`useSystemDialog().open(path, params)` 返回 Promise；独立 URL `/sysdlg/app_installer` 对外部调用方**没有结果回传通道**。
- 新应用注册三处：`app/registry.tsx`、`app/backend-apps.ts` 的 `DESKTOP_BUILTIN_APP_IDS`、`mock/data.ts` 的目录项。kRPC 经 `api/rpc.ts` 的 `callRpc`，共享 API 模块带 mock 切换（`api/user_mgr.ts:5-12`）。

### 2.8 MessageHub / msg_center

- 附件是 `MsgObject.content.refs[] = { role, target: {type:"data_obj", obj_id, uri_hint?}, label? }`（`ndn-lib/src/msgobj.rs:199-245`）。接收方读附件走 §2.6 的 msg-center 对象路由，**授权依据是消息引用了该对象**，不需要 NFS 权限。
- Desktop 上传走 `/ndm/v1/uploads` + `put_object`（`messagehub/api/upload.ts`）；`ForwardDialog.tsx` 的 `store.forward()` 可按 obj_id 转发已存在的 data_obj 引用而不重新上传。
- 没有“分享意图”深链；`MessageHubRoute.tsx:44-52` 只认 `entityId/sessionId/messageId/notify/ownerDid/mode`；窗口启动有 `messageHubLaunchSchema`（`messagehub/launch.ts:4-12`）可扩展。
- 系统服务代用户发消息：非 User principal 可通过 owner_session 身份检查（`owner_session.rs:479-501`），再由 RBAC 决定；control-panel 属 `system` 组，有 `obj://*`。但首期不需要代发（见 §3.10）。

### 2.9 TaskManager

- schema id 形如 `"<domain>.<verb>/v1"`（`buckyos-api/src/task_mgr.rs:1018-1052`）；创建用 `create_delegated_task(CreateDelegatedTaskReq{ name, schema_id, input, creator: ActorRef::new(user_id, app_id), storage_domain, idempotency_key, … })`；download_mgr 在 `TASK_ERR_SCHEMA_NOT_FOUND` 时自注册 schema（`download_mgr.rs:119-129`）。
- 幂等：客户端传 `idempotency_key`，重放用 `list_tasks{creator_user_id, creator_app_id, idempotency_key, schema_id}` 查（`app_installer.rs:2606-2627`）。
- 进度与完成：`report_progress / runner_start / runner_wait(TaskWaitReason) / runner_complete / runner_fail`（`app_install_engine.rs:1715-1830`）。

### 2.10 结论

| PRD 要求 | 现状 | 本设计的处理 |
| --- | --- | --- |
| 内置分享提供服务 | 不存在；gateway 无 NDN 鉴权读路径 | control-panel 内新增 `/s/*` HTTP 处理器（D1） |
| 快照不可变 + 保留 | `cacl_file_object` + `pin` 都有，但没人串起来 | §3.4 |
| 读取路径授权覆盖 chunk/Range | NdnDirServer 无鉴权 | 不暴露裸对象 URL，只暴露分享路径（§3.5） |
| Public 目录 | `data/srv/publish/` 已被 gateway 匿名服务 | 观察投影（§3.9） |
| 结果身份、修订、审计 | 桩以 name 为主键 | 新 schema（§3.3） |
| 应用分享回传 | 无任何契约 | MessageHub 单目标 + `share.app.report`（§3.10） |
| 系统分享面板 | 只有 app_installer 一个 sysdlg | `sysdlg/share`（§3.12） |

## 3. 设计

### 3.1 分层与接线

```text
Desktop                 sysdlg/share ──┐  分享与发布(管理中心)  File Browser 入口   MessageHub(附件引用+回传)
                                       │          │                   │                  │
                                       ▼          ▼                   ▼                  ▼
control-panel  kRPC  share.capabilities / plan / submit / operation.get / result.* / app.* / public.*
               HTTP  /s/<token>[/meta|/unlock|/d[/<inner>]]        ← 访客（匿名或密码）
               内部  ShareContentMgr{ sqlite, snapshotter, serving, sweeper, public_observer }
                        │ cacl_file_object / pin / open_reader          │ create_delegated_task
                        ▼                                               ▼
               named store（本节点共享）                              task-manager（share.snapshot/v1）
gateway        boot_gateway.yaml 新增 /s/* → control-panel（或 share.<zone> host，D3）
```

ShareContentMgr 是 control-panel 的一个模块（`src/frame/control_panel/src/share/` 目录，拆 `mod.rs / db.rs / snapshot.rs / serving.rs / rpc.rs / public_observer.rs / sweeper.rs`），不是独立进程。原因：它需要 principal、named store、TaskManager 客户端和 HTTP runner，这些 control-panel 全有；PRD §3.1 也只要求“一个权威写入方”。

### 3.2 身份与权限

- `owner_user_id = principal.username`；`actor_app_id = principal.authenticated_app_id`；Agent 操作天然带 `app:<agent_id>@<owner>`，记录在操作与审计里，满足 PRD §6.1 第 2 条“分别记录”。
- 普通用户只能读写 `owner_user_id == principal.username` 的结果；`Admin/Root` 可跨用户，但审计写 `source = "admin"`。沿用 `user_mgr.rs` 的 `require_self_or_admin` 风格，不引入 RBAC 表。
- 源读取权限：快照前用调用者的 token 调 nfs-server `POST /nfs/v1/stat`（REST，`server.rs:39-51`）确认 `Level::Read`，再按 cyfs 路径映射到磁盘路径读取（D6）。这样 ACL 只有 nfs-server 一个权威。
- 应用回传（`share.app.report`）只接受 `authenticated_app_id` 等于该结果 `target_app_id` 的调用；owner 不可被回传更改。
- `/s/*` 访客请求不读任何 session cookie，只认分享 token 与密码解锁后签发的 `share_access`（§3.5）。

### 3.3 数据模型

SQLite，路径 `$BUCKYOS_ROOT/data/srv/control-panel/share_content.sqlite3`（D5），WAL，外键开。所有 id 为 `ulid` 或 `uuid v4` 字串，`revision` 为管理修订号。

```sql
CREATE TABLE share_operations (
  operation_id     TEXT PRIMARY KEY,
  intent           TEXT NOT NULL,            -- create_link | update_link | stop_link | app_handoff
  owner_user_id    TEXT NOT NULL,
  actor_user_id    TEXT NOT NULL,
  actor_app_id     TEXT NOT NULL,
  input_json       TEXT NOT NULL,            -- 输入快照（source、policy、title）
  idempotency_key  TEXT NOT NULL,
  task_id          TEXT,                     -- 有异步快照时才有
  result_id        TEXT,
  state            TEXT NOT NULL,            -- preparing | running | completed | failed | canceled
  error_json       TEXT,
  created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL,
  UNIQUE (owner_user_id, actor_app_id, idempotency_key)
);

CREATE TABLE content_versions (
  obj_id           TEXT PRIMARY KEY,         -- cyfile:... 或 cydir:...
  obj_type         TEXT NOT NULL,
  root_name        TEXT NOT NULL,
  mime             TEXT,
  total_size       INTEGER NOT NULL,
  item_count       INTEGER NOT NULL,
  source_json      TEXT NOT NULL,            -- {cyfs_path, mtime, size, excluded:[...]}，私有，不进对象 meta
  prepared_by      TEXT NOT NULL,
  prepared_at      INTEGER NOT NULL
);

CREATE TABLE share_results (
  result_id        TEXT PRIMARY KEY,
  kind             TEXT NOT NULL,            -- link | app_delivery   （public_entry 不入此表，见 public_observations）
  owner_user_id    TEXT NOT NULL,
  created_by_app_id TEXT NOT NULL,
  title            TEXT NOT NULL,
  source_json      TEXT NOT NULL,            -- 创建时的来源引用，仅展示用
  obj_id           TEXT NOT NULL REFERENCES content_versions(obj_id),
  status           TEXT NOT NULL,            -- active | stopping | stopped | expired | failed
  status_reason    TEXT,
  hidden           INTEGER NOT NULL DEFAULT 0,
  revision         INTEGER NOT NULL DEFAULT 1,
  created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL, stopped_at INTEGER
);

CREATE TABLE share_links (                   -- kind = link 的策略与凭据
  result_id        TEXT PRIMARY KEY REFERENCES share_results(result_id) ON DELETE CASCADE,
  token_hash       TEXT NOT NULL UNIQUE,     -- sha256(token)，token 不落库
  access           TEXT NOT NULL,            -- holder | password
  password_hash    TEXT,                     -- pbkdf2-sha256 PHC 字串
  expires_at       INTEGER,                  -- NULL = 不自动到期
  allow_download   INTEGER NOT NULL DEFAULT 1,
  access_count     INTEGER NOT NULL DEFAULT 0,
  last_access_at   INTEGER
);

CREATE TABLE retentions (
  result_id        TEXT NOT NULL REFERENCES share_results(result_id) ON DELETE CASCADE,
  obj_id           TEXT NOT NULL,
  pin_owner        TEXT NOT NULL,            -- "share:<result_id>"
  pinned_at        INTEGER NOT NULL,
  released_at      INTEGER,
  PRIMARY KEY (result_id, obj_id)
);

CREATE TABLE app_deliveries (                -- kind = app_delivery 的投影
  result_id        TEXT PRIMARY KEY REFERENCES share_results(result_id) ON DELETE CASCADE,
  target_app_id    TEXT NOT NULL,            -- 首期固定 "messagehub"
  handoff_at       INTEGER NOT NULL,
  delivery_state   TEXT NOT NULL,            -- handed_off | confirmed | failed | unknown
  business_ref_json TEXT,                    -- {session_id, message_id}
  last_confirmed_at INTEGER,
  error            TEXT
);

CREATE TABLE share_audit (
  audit_id         INTEGER PRIMARY KEY AUTOINCREMENT,
  result_id        TEXT NOT NULL,
  action           TEXT NOT NULL,            -- create | update_policy | stop | expire | hide | app_report
  revision_before  INTEGER, revision_after INTEGER,
  actor_user_id    TEXT NOT NULL, actor_app_id TEXT NOT NULL,
  source           TEXT NOT NULL,            -- gui | cli | agent | system | admin
  reason           TEXT,
  outcome          TEXT NOT NULL,            -- ok | conflict | failed
  at               INTEGER NOT NULL
);

CREATE TABLE public_observations (          -- data/srv/publish 的观察投影
  rel_path         TEXT PRIMARY KEY,
  entry_kind       TEXT NOT NULL,            -- file | dir | symlink
  link_target      TEXT, link_ok INTEGER,
  size INTEGER, mtime INTEGER,
  first_seen_at INTEGER NOT NULL, last_seen_at INTEGER NOT NULL, missing_since INTEGER
);

CREATE TABLE share_access_log (             -- 形状沿用旧设计，首期只写不聚合
  log_id INTEGER PRIMARY KEY AUTOINCREMENT,
  result_id TEXT NOT NULL, at INTEGER NOT NULL,
  status INTEGER NOT NULL, bytes INTEGER NOT NULL DEFAULT 0,
  ip_hash TEXT, user_agent TEXT
);
```

与 PRD §5.2 的对应：操作记录 = `share_operations`；结果记录 = `share_results`；内容版本 = `content_versions`；授权与保留 = `share_links` + `retentions`；外部投影 = `app_deliveries`；审计 = `share_audit`。“发布关系”“名称绑定”两张表第二阶段再加，`share_results.kind` 预留 `publication`。

### 3.4 内容快照（snapshotter）

1. 输入 `source = {kind:'cyfs-path', path}`（复用 websdk `TransferableContentRef`，`buckyos-websdk/src/content.ts:4-6`）。首期只接受 `cyfs://<zone>/home/<user>/...`、`/srv/library/...`、`/srv/publish/...` 三类前缀；`object-id` 输入第二阶段。
2. 用调用者 token 调 nfs-server `stat`，确认可读并取 size/mtime。目录递归 `list`。
3. 磁盘映射：`cyfs://<zone>/home/u/x` → `$BUCKYOS_ROOT/data/home/u/x`，`/srv/...` → `data/srv/...`。拒绝路径中出现 `..`；符号链接按真实目标是否仍在允许前缀内判定。
4. 目录排除默认规则：`.git/`、`node_modules/`、`.DS_Store`、`*.key`、`*.pem`、`.env*`，plan 阶段返回排除清单让用户看见（PRD §10.1）。
5. 调 `ndn_toolkit::cacl_file_object(..., StoreMode::StoreInNamedMgr)` / `cacl_dir_object`。`FileObject` 的 meta 只放 `mime` 与 `name`，不放来源路径与设备信息。
6. 快照完成后 `pin(root_obj_id, "share:<result_id>", PinScope::Recursive, None)`，写 `retentions`，再写 `content_versions`。
7. 一致性：快照后再次 `stat`，若 size/mtime 已变则标记 `source_changed=true` 返回给前端（PRD §8.1 “源已变，继续旧快照还是重做”），不静默换内容。
8. 同步/异步：总大小 ≤ 阈值（D7，建议 64 MiB）在 RPC 内同步完成；否则创建 `share.snapshot/v1` 任务，`input = {operation_id, source, exclusions}`，`result = {obj_id, total_size, item_count}`，进度用 `report_progress`。任务由 control-panel 自己的 runner 执行（同 download_mgr 的做法）。
9. 失败：删除本次新建且无其他引用的 retention（unpin），已被其他结果引用的对象不动（PRD §14.2 补偿原则）。

### 3.5 分享提供服务（serving）

URL（D2、D3）：

```text
https://<zone>/s/<token>              预览页（HTML，服务端渲染，无 SDK）
https://<zone>/s/<token>/meta         JSON：title, kind, size, items[], expires_at, requires_password, allow_download
https://<zone>/s/<token>/unlock       POST {password} → 200 + Set-Cookie share_access=<jwt>; Path=/s/<token>; HttpOnly; Secure; SameSite=Strict
https://<zone>/s/<token>/d            单文件下载/内联预览
https://<zone>/s/<token>/d/<inner>    目录分享内的文件
```

- `token`：128 bit 随机，base32 小写 26 字符；库里只存 sha256。`result_id` 不出现在公开 URL 中。
- 每个请求先查 `share_links` 联 `share_results`：`status = active` 且（`expires_at IS NULL OR expires_at > now`），否则 404（不区分不存在与已停止，避免枚举）。
- `access = password` 时要求 `share_access` cookie 或 `Authorization: Bearer <jwt>`；JWT 由 control-panel 用自身 key 签，claims `{sub:"share", rid, exp ≤ min(expires_at, now+12h), pwd_rev}`。改密码时 `pwd_rev++`，旧 JWT 立即失效（T05）。解锁接口按 token 做速率限制（每分钟 10 次）。
- 下载：`open_reader(obj_id, inner_path)` 取 FileObject 后用 `open_chunk_reader / open_chunklist_reader(offset)` 流式输出，解析 `Range: bytes=a-b` 并自行截断到 b；响应 `Content-Type`（meta.mime 或按扩展名）、`Content-Disposition`（`inline` 用于图片/视频/音频/PDF/文本，其余 `attachment`）、`Accept-Ranges: bytes`、`Cache-Control: private, no-store`、`X-Content-Type-Options: nosniff`、`Content-Security-Policy: sandbox; default-src 'none'`（防止被分享的 HTML 在本域执行）。
- 目录：预览页列出 DirObject 子项，子目录可下钻；不提供 zip 打包（第二阶段）。
- 写 `share_access_log` 与 `access_count/last_access_at`（异步、失败不影响响应）。
- gateway：D3=A 时在 `boot_gateway.yaml` 的 `get_service_info_from_req` 加 `elif match $REQ.path "/s/*" then global SERVICE_ID="control-panel"`；D3=B 时在 `system_config_agent.rs:1306-1324` 加 `share` host key，control-panel 按 `Host`/`X-Forwarded-Host` 判断只服务 `/s/*`。两种都要重建 rootfs 与镜像才能在 DV 生效。

### 3.6 kRPC 接口（control-panel，前缀 `share.`）

所有方法要求已认证 principal；错误用现有 `RPCErrors`，业务错误放 `RPCResult::Failed`，code 见下表。

| 方法 | 参数 | 返回 |
| --- | --- | --- |
| `share.capabilities` | `{source?}` | `{actions:["create_link","app_handoff"], access:["holder","password"], expiry_presets:[86400,604800,2592000,null], limits:{max_total_size, sync_threshold}, blocked:[{action, reason}], app_targets:[{app_id, label, accepts:[...]}]}` |
| `share.plan` | `{intent, source, policy, title?}` | `{plan_fingerprint, estimate:{total_size, item_count, excluded:[...]}, warnings:[...], requires_task, existing:[{result_id, access, expires_at}], source_public:{in_publish_dir, public_acl}}`。无副作用。 |
| `share.submit` | `{intent:"create_link", plan_fingerprint, source, policy:{access, password?, expires_at?, allow_download?}, title?, idempotency_key}` | `{operation_id, state, result_id?, task_id?, url?}`。同 key 重放返回原操作；同 key 不同输入拒绝 `IDEMPOTENCY_MISMATCH`。 |
| `share.operation.get` | `{operation_id}` | 操作全貌 + 关联任务 phase/outcome 映射后的 `state` |
| `share.result.get` | `{result_id}` | 结果、链接（含完整 URL，仅 owner/admin）、版本、保留、审计摘要 |
| `share.result.list` | `{kind?, status?, source_path?, obj_id?, q?, include_hidden?, limit, cursor}` | `{items:[...], next_cursor}`。只返回主体可见条目。 |
| `share.result.update` | `{result_id, revision, changes:{title?, expires_at?, password?: string\|null, allow_download?}}` | `{revision}`；修订不符返回 `REVISION_CONFLICT` 并附当前记录 |
| `share.result.stop` | `{result_id, revision, reason?}` | `{status:"stopped"}`；unpin 失败时 `status:"stopping"` + 后台重试 |
| `share.result.hide` | `{result_id, hidden}` | 仅视图 |
| `share.app.handoff` | `{source, target_app_id, idempotency_key}` | 快照 + 创建 `app_delivery` 结果 → `{result_id, obj_id, launch:{appId:"messagehub", payload:{share:{resultId, objId, name, size, mime}}}}` |
| `share.app.report` | `{result_id, delivery_state, business_ref?, error?}` | 校验调用 app 与 target 一致、按时间戳拒绝旧事实覆盖 → `{accepted}` |
| `share.public.list` | `{refresh?}` | `{entries:[...], last_scan_at, scan_ok}` |
| `share.stats.get` | `{result_id}` | `{access_count, last_access_at}`（首期只这两项） |

业务错误码：`SOURCE_NOT_READABLE`、`SOURCE_NOT_ALLOWED`、`SOURCE_CHANGED`、`TOO_LARGE`、`IDEMPOTENCY_MISMATCH`、`REVISION_CONFLICT`、`NOT_OWNER`、`RESULT_NOT_ACTIVE`、`TARGET_MISMATCH`、`STALE_REPORT`。

### 3.7 TaskManager 映射

- schema `share.snapshot/v1`，`user_creatable=false`，`default_storage_domain=User`，publisher `control-panel`；首次使用时自注册。
- `creator = ActorRef::new(principal.username, principal.authenticated_app_id)`，`idempotency_key` 直接沿用 `share.submit` 的 key。
- 状态映射到 PRD §14.1：`Promised/Accepted/Running → preparing/running`，`Waiting → running(等待资源)`，`Terminal+Succeeded → completed`，`Failed → failed`，`Canceled → canceled`。`operation.state` 是业务态，`task_id` 另给。
- 取消：快照任务可取消；已完成的 pin 与 result 不因取消回滚，走 `stop`。

### 3.8 生命周期与一致性

- `revision` 只在 `update/stop/hide` 时 +1；改密码或期限不产生新 `obj_id`。
- sweeper：tokio 定时（60 s）把 `expires_at <= now` 的 `active` 改为 `expired`，unpin 写 `retentions.released_at`，审计 `expire/system`。`/s/*` 仍每请求判一次时间，不依赖 sweeper 及时。
- 停止：`active → stopping`（先写状态，访客立即 404）→ unpin 成功 → `stopped`；unpin 失败保留 `stopping`，sweeper 重试。
- 重新分享已停止内容：PRD §8.3 要求默认新建结果；若 `content_versions` 仍存在且 `is_object_stored` 为真，可复用 obj_id 省去重新计算，但仍是新 `result_id`、新 token。
- 进程重启：`share_operations.state in (preparing, running)` 且有 `task_id` 的，启动时查任务状态补写；无 `task_id` 的同步操作直接标 `failed(interrupted)`。

### 3.9 Public 投影（public_observer）

- Public = `$BUCKYOS_ROOT/data/srv/publish/`（D4）。control-panel 定时（默认 5 min）加上手动 `refresh` 扫描该目录，写 `public_observations`：首次发现时间、kind、symlink 目标与是否越界（目标必须解析后仍在 `data/srv/publish/` 或 `data/srv/library/` 内，否则 `link_ok=0` 并在 UI 标红）、消失时间。
- 公开 URL 形态由 gateway 决定：`https://public.<zone>/<rel_path>`。管理中心只展示“路径可访问”状态，不宣称 NDN 对象就绪（PRD §11、T09）。
- 删除/改名委托 File Browser（nfs-server `srv` 根下的 `publish/`），ShareContentMgr 不写文件。
- `share.plan` 的 `source_public` 用两条证据回答 T11：源路径是否位于 publish 目录；nfs-server `get_acl` 是否含 `public`。两者都只检查当前主体有权看到的信息。
- 不提供密码/到期开关（PRD §11）。

### 3.10 通过应用分享：首期只接 MessageHub（D8）

1. 面板选“发送到 MessageHub” → `share.app.handoff` 做快照（或复用已有版本）并创建 `app_delivery` 结果，`delivery_state=handed_off`。
2. Desktop 用 `desktopUIStore.openAppWindow('messagehub', {launch:{requestId, payload:{share:{...}}}})` 打开 MessageHub；`messageHubLaunchSchema` 扩一个 `share` 字段。MessageHub 复用 `ForwardMessageForm` 的选人逻辑，把 `{type:'data_obj', obj_id}` 作为 `refs` 发送，不重新上传。
3. 发送成功后 MessageHub 前端调 `share.app.report {result_id, delivery_state:'confirmed', business_ref:{session_id, message_id}}`；用户关闭未发送则 `report {delivery_state:'canceled'}`。无回传则管理中心显示“结果未确认 + 交接时间”（T06）。
4. 接收方读附件走 msg-center 的对象路由，权限来自消息引用；分享的 pin 由 `share:<result_id>` 持有，用户“停止”这条 app_delivery 只释放保留，不撤回消息（PRD §9.2 “本地取消不能抹去已发生的外发”），UI 文案照此写。
5. 第三方目标注册（AppDoc 字段、回传验证）第二阶段。

### 3.11 上线衔接（D9）

- 面板对 PIKG / AppDID / URL 输入给“在我的 BuckyOS 上线”，直接 `useSystemDialog().open('sysdlg/app_installer', {identifier})`。
- HTML 目录输入显示“受管理站点上线尚未交付”，并给两条已支持的替代：复制到 Public（说明匿名公开）或创建分享链接。不静默复制到 Public（PRD §4）。
- 管理中心“网站与服务”页只读 `apps.list` / app service 状态与 `StaticWebDeploymentEvidence`，不新建记录；管理动作跳转 App Service。

### 3.12 前端

- **`sysdlg/share`**：参数 `{ source: TransferableContentRef, title?: string, preferred?: 'link'|'app' }`，返回 `{action:'created', resultId, url?} | {action:'handed_off', resultId} | {action:'close'}`。首屏：摘要 + 已有有效结果（可复制，显示范围/期限）+ 三个动作。链接配置卡：范围（持链接者/密码）、期限（1 天/7 天/30 天/不自动到期/自定义，显示时区）、固定快照说明。进度卡：同步直接等待，异步显示任务进度并允许“后台继续”。独立路由 `/sysdlg/share?source=...` 与 installer 同样无回传通道，先只做 SPA 内调用。
- **File Browser**：`menu/commands.ts` 的 `share` 改为打开上面的对话框；“Copy public URL” 保留但改名“复制读取地址”并只在源路径有 public ACL 或在 publish 目录时标“公开”。
- **管理中心**：新建 builtin app `share-center`（显示名“分享与发布”），tab：全部 / 分享链接 / 已发布内容（= Public 投影）/ 网站与服务 / 通过应用发出。列：名称、类型、访问范围、状态、更新时间；筛选按 PRD §13。详情抽屉：版本 obj_id（折叠显示，复制完整）、链接、期限、密码开关、保留、审计。动作命名照 PRD §13 表。注册三处见 §2.7；mock-first，`VITE_SHARE_CENTER_USE_MOCK`；i18n en+zh 全量。
- **访客页**：control-panel 服务端渲染的最小 HTML（内联 CSS/JS，不加载 websdk，不读 cookie 以外的任何会话），显示标题、大小、到期、密码框、预览区（img/video/audio/pdf/text 原生渲染，其他给下载按钮）。HTML 类型文件只提供下载，不内联渲染。
- 旧 `product/control_panel/share_content.md` 的两行内容并入 `BuckyOS Sys Dlg.md` 新增的 `sysdlg/share` 小节。

### 3.13 websdk 与 CLI

- `buckyos-websdk/src/share_client.ts`：`ShareClient` 包装 `share.*`，`runtime.getShareClient()`；CLI 加 `buckyos share create <cyfs-path> [--expires 7d|none] [--password] [--title]`、`share list`、`share get`、`share stop`、`share update`。Agent 经 websdk 使用（已有原则：Agent 工具优先 TS + websdk）。

### 3.14 安全清单与已知风险

- token 128 bit、只存哈希；密码 pbkdf2-sha256（`pbkdf2` 已在 Cargo.lock）≥ 600k 轮；解锁限速；404 不区分状态；日志经 `redaction.rs` 过滤 token 与密码。
- 分享内容响应带 `CSP: sandbox`，HTML 只下载不渲染。
- 已知风险（按“先能力后安全”原则记录，不在首期加机制）：D3 选 A 时访客页与控制台同源，依赖访客页不加载 SDK 来避免凭证使用，T35 只能部分满足；核对中发现 `/ndm/*` 与 `/ndm/proxy/v1` 由 gateway 直接转发且处理器内没有会话检查，与本设计无关，单独开 issue。

## 4. 决策项（开工前拍板）

| # | 问题 | 选项 | 建议 |
| --- | --- | --- | --- |
| D1 | 分享提供服务宿主 | A) control-panel 内 `/s/*` HTTP 处理器；B) cyfs-gateway `cyfs-dir` + process-chain 回调 control-panel 鉴权 | **A**。B 需要 NdnDirServer 补 Content-Type/文件名/Range 终点，且 buckyos 尚未配置 cyfs-dir；作为第二阶段迁移方向 |
| D2 | 链接与凭据形态 | token 放路径 `/s/<token>`；密码解锁签 JWT 放 cookie（Path 限定）还是 query | **路径 token + cookie**。query 会进日志与 Referer |
| D3 | 访客页宿主 | A) 根域 `/s/*`；B) 独立 host `share.<zone>`（scheduler 加 host key，通配证书已有） | **B**，cookie 与 localStorage 天然隔离，T35 可完整通过；若 control-panel 拿不到可靠的 Host 头则回退 A 并记录已知风险 |
| D4 | Public 的定义 | A) 固定目录 `data/srv/publish/`；B) 任意带 public ACL 的 nfs 路径 | **A** 作为“Public 目录”；B 的路径在 `share.plan` 里作为“已公开证据”使用，不做投影 |
| D5 | 存储位置与旧代码 | DB 放 `data/srv/control-panel/`；删除 `share_content_mgr.rs` 桩与 `content_mgr_client.rs` | **按此执行**。无调用者、无数据 |
| D6 | 源读取权限校验 | A) 用调用者 token 调 nfs-server REST `stat`；B) control-panel 只允许 owner home 子树 | **A**，ACL 单一权威；B 作为 nfs-server 不可用时的拒绝策略，不做放行 |
| D7 | 同步/异步阈值 | 32 / 64 / 128 MiB | **64 MiB**（两个 chunk） |
| D8 | 应用分享首期目标 | 仅 MessageHub；或加 HomeStation | **仅 MessageHub** |
| D9 | 上线衔接范围 | 仅路由到 app_installer + 实例投影 | **按此**，HTML 目录标“未交付” |
| D10 | 密码哈希 | pbkdf2（已有）/ argon2（新依赖） | **pbkdf2** |
| D11 | 访客页实现 | control-panel 服务端模板 / Desktop 第二个 vite 入口 | **服务端模板**，无构建依赖、无 SDK |
| D12 | 管理中心承载 | 独立 builtin app `share-center` / 并入 Settings | **独立 app**，PRD §13 要求系统级入口“分享与发布” |

## 5. 实施切片

### M0：契约与清理（评审通过后开始写代码）

- 删除 `share_content_mgr.rs` 桩与 `buckyos-api/src/content_mgr_client.rs` 及 `lib.rs` 导出；全仓 grep 确认无引用。
- 新建 `src/frame/control_panel/src/share/` 骨架、DDL（§3.3）、`share.capabilities` 与 `share.result.list` 空实现接进 `main.rs` 分发。
- `boot_gateway.yaml` / `system_config_agent.rs` 按 D3 加路由；`BuckyOS Sys Dlg.md` 加 `sysdlg/share` 小节；本文档 §3.6 定稿为接口契约。
- 验证：`cargo test -p control_panel`；本地 kRPC 调 `share.capabilities` 返回能力清单。

### M1：分享链接后端

- snapshotter（§3.4）、serving（§3.5）、`submit/operation.get/result.get/update/stop`、sweeper、`share.snapshot/v1` 任务。
- 单测：token 哈希与 404 不可区分；密码 JWT `pwd_rev` 失效；Range 截断；目录排除；源变化检测；幂等重放与 mismatch；修订冲突；到期 sweeper；unpin 失败重试。
- 集成：用临时 named store 配置跑“文件 → 快照 → pin → HTTP 读 → stop → 404”。

### M2：系统分享面板 + File Browser 入口

- `sysdlg/share`、`api/share.ts`（含 mock）、File Browser `share` 命令接通、i18n。
- Playwright：T01（打开关闭无副作用）、T02（默认七天、改不自动到期类型不变）、创建后复制链接 ≤ 3 次主要操作。

### M3：管理中心 + Public 投影

- `share-center` app、`public_observer`、`share.public.list`、`share.result.hide`。
- 验证：CLI 直接在 `data/srv/publish/` 增删文件与越界软链接，刷新后视图一致、越界标红（T08、T10）。

### M4：MessageHub 目标 + websdk/CLI

- `share.app.handoff/report`、MessageHub launch `share` 字段与发送后回传、`ShareClient`、CLI。
- 验证：交接后不发送 → “结果未确认”；发送后 → “应用已完成”并能点开消息；停止不撤回消息（T06）。

### M5：DV 验收

- 重建 rootfs 与 aios/control-panel 镜像（gateway 规则在 rootfs 里），`start.py --all` 新装；按 §6 用例逐条留证据目录。

## 6. 验收（对应 PRD §18）

| 编号 | 本设计覆盖 | 说明 |
| --- | --- | --- |
| T01–T05 | 完整 | T05 依赖 `pwd_rev` 与每请求状态检查 |
| T06 | 完整 | 仅 MessageHub |
| T07 | 部分 | 分享已上线实例地址 = 普通文本/链接走 MessageHub，不做权限分析 |
| T08–T10 | T08、T10 完整；T09 以“只报告路径可访问、不报 NDN 就绪”满足 | Public 无对象化 |
| T11 | 完整 | `source_public` 两条证据 |
| T12–T15 | 入口路由与投影层面满足；安装/运行/下线本身由 App Service 既有验收覆盖 | HTML 目录标未交付 |
| T16 | 部分 | 只清理失败快照的 unpin 与任务临时数据；无全局空间统计 API |
| T17 | 完整 | GUI/CLI/Agent 同一 `share.*`，审计记真实 actor_app_id |
| T18 | 完整 | result 列表按 owner 过滤；token 不可枚举；回传校验 target app |
| T19 | 完整 | 幂等、重启恢复、`STALE_REPORT` |
| T35 | D3=B 完整；D3=A 部分 | 见 §3.14 |
| T20–T34 | 不覆盖 | 第二、三阶段 |

## 7. 对 PRD 的回写建议

- §3.1 “指定的内置分享提供服务” → 明确为 control-panel 的分享提供服务（D1）。
- §4 第一阶段 Public 列去掉“准备状态”，移到第二阶段“完整对象化能力”。
- §3.2 基线段补一句：旧桩与 `content_mgr_client` 已删除，无存量数据，§19.2 不触发迁移。
- §6.2 “指定用户或好友”：My Network 的 contact 能力已具备，但分享链接的身份校验依赖访客登录，首期不开放，列入第二阶段。
- §19.1 表“NDN 数据授权”一行的阻塞范围改为“对象直链（第二阶段）”，首期受控分享不经 NDN 直链。

## 8. 参考入口

- control-panel 分发与身份：`src/frame/control_panel/src/main.rs:106-114, 853-1069`；`sys_auth_backend.rs:1151-1243`
- 任务创建范式：`src/frame/control_panel/src/download_mgr.rs:94-131`；`app_installer.rs:2606-2627`
- 文件 → 对象：`src/frame/opendan/src/objects.rs:21-62`；`cyfs-ndn/src/ndn-toolkit/src/tools.rs:156, 375`
- pin/unpin：`cyfs-ndn/src/named_store/src/ndm.rs:1279-1310`；`gc_types.rs:9`
- 授权对象读取先例：`src/frame/msg_center/src/object_access.rs`；`src/frame/homestation/src/http.rs:288-331`
- gateway 路由：`src/rootfs/etc/boot_gateway.yaml:76-79, 116-126, 138-232`；`src/kernel/scheduler/src/system_config_agent.rs:1306-1324, 1371`
- nfs-server ACL 与 REST：`src/frame/nfs_server/src/acl.rs:24-27`；`auth.rs:63-76`；`server.rs:39-51`
- Desktop：`src/frame/desktop/src/sysdlg/index.tsx`；`publicRoutes.ts`；`app/filebrowser/menu/commands.ts:14`；`app/messagehub/launch.ts`；`app/registry.tsx`；`app/backend-apps.ts`
- websdk：`buckyos-websdk/src/content.ts:4-6`；`repo_client.ts:157-190`（客户端写法）

## 9. 实施记录

（开工后追加：落点、与 §3 不同的决定、验证命令与结果、DV 证据、未做项。）
