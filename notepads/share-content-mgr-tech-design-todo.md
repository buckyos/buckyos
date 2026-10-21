# ShareContentMgr 技术设计 TODO：分享链接、Public 投影与“我的内容”（PRD 第一阶段）

> 创建日期：2026-10-18
> 版本：v0.6（2026-10-20，用户确认后续收尾决策 D16–D19，明确允许 breaking change；任务与验收见 §10）。v0.5（2026-10-19 M0–M5 完成：DV 验收通过并修掉 4 个 DV 问题，记录见 §9.7）。v0.4（同日 M0–M4 实施）。v0.3（同日用户确认 D13–D15 按建议执行）。v0.2（同日）按用户对 D1–D12 的回复重写：后台整体放 nfs-server，数据面走 nfs-server 的 cyfs 协议能力；应用分享目标为 MessageHub 与 HomeStation；管理中心是独立 App“我的内容”，要能看到内容的消费情况。v0.1 的 control-panel 方案已作废。
> 状态：M0–M5 已完成并提交（`835b5bd44`）。test.buckyos.io 上 DV 验收通过（§9.7，`tests/dv/share.spec.ts` 4 例 + 手工探针）；与 §3 不同之处见 §9.2。后续 7 项收尾已批准、尚未实施，见 §10；批准设计不代表代码已完成。
> 依据：[ShareContentMgr PRD v0.1](../product/control_panel/ShareContentMgr_PRD.md)、[分享/发布/上线产品设计](<../doc/control_panel/BuckyOS 的分享_发布_上线 功能的产品设计.md>) §17–§18、[nfs_server 技术需求](../product/bucky_file/nfs_server.md) §3.2 / §4.1 / §5 / §11、[NFSP v0](../../cyfs-ndn/doc/NamedFileSystem_Protocol_v0.md) §3.6 / §5.5 / §7.3、[旧 ShareContentMgr 设计](../doc/control_panel/ShareContentMgr.md)（只借用统计表形状），以及 2026-10-18～19 对 buckyos / cyfs-ndn / cyfs-gateway / buckyos-websdk 的源码核对。
> 第 2 节保留实施前的源码核对基线，第 9 节保留 M0–M5 实施与验证记录，不能把其中历史状态重复算作待办。后续目标以 §4.3、§10 为准；当前实现仍须核对源码。

## 1. 目标与范围

PRD 第一阶段要求交付“普通分享的完整生命周期”和“受支持网站或服务的上线衔接”。本设计把 ShareContentMgr 实现为 nfs-server 的一个模块：nfs-server 本来就是 Zone 的文件服务器，按 NFSP 设计承担 cyfs 数据面（cap token、`/s/{token}` 分享链接、`access_urls`、Link → Store 提升），分享是它已规划但未完成的那一块。

M0–M5 交付范围：

1. **nfs-server 分享模块**：快照（文件/目录 → NDN 对象，Store 模式，pin 保留）、分享记录、cap 凭据、审计、访问统计；以 NFSP 扩展方法 `share.*` 提供管理接口。
2. **分享数据面**：`https://share.<zone>/s/{token}` 访客页与下载，持链接者或密码访问，服务端执行到期与停止。
3. **Public 投影**：`data/srv/publish/`（gateway 已把 `public.<zone>` 指向它）在“我的内容”里可见；nfs-server 为其中文件返回 `access_urls` 的 `public` 条目。首期不做 NDN 对象化。
4. **通过应用分享**：MessageHub（发消息附件）与 HomeStation（发动态），都以快照 obj_id 交付，回传业务结果。
5. **前端**：系统分享面板 `sysdlg/share`、File Browser 入口、独立 App“我的内容”（含消费情况）、访客页。上线衔接只做入口路由到既有 `sysdlg/app_installer` 与实例投影。

不做（PRD 第二、三阶段或明确排除）：独立内容发布与长期入口切版、Public 对象化与“保存为独立发布”、第三方分享目标注册、Static Web 本地打包 PIKG、收录者、BNS、交易与收入（“我的内容”预留列，不实现）、live 子树 cap 的数据面放行（§3.5）。

2026-10-20 批准的后续范围：两份文档回写、真实 Agent DV、unpin 失败重试测试、应用附件保留、旧回归套件、NDM 网关迁移，共 7 项（§10）。本版本允许 breaking change，相关服务、SDK、前端与测试按同一版本同步修改，不承担旧版本兼容义务。

## 2. 实施前现状（2026-10-18～19 核对，历史基线）

### 2.1 nfs-server

- 进程与路由：axum，`src/frame/nfs_server/src/server.rs:39-51` 的 `build_router` 挂 `/nfs/v1/watch`、`/nfs/v1/read/{node_id}`、`/nfs/v1/uploads/{fb}`、`POST /nfs/v1/{method}`；`main.rs:226` 起服务。服务模式端口 4110，`KernelService + SelectorType::Single`，gateway 把根级 `/nfs/v1/*` 原样转发（`boot_gateway.yaml:209-212`）。
- 信封：`control_handler`（`server.rs:57-133`）先 `authenticate`，除 `hello` 外都要求 hello 会话；`WRITE_METHODS`（`server.rs:22-37`）里的方法要求 `seq`，按会话做 exactly-once 重放（内存窗口，重启即失效）。`dispatch`（`server.rs:240-275`）是方法名大 `match`。
- 身份：`Actor::{Admin(Option<user>), User(user), Anonymous}`（`acl.rs:52-90`）。`auth.rs:78-101`：User token → zone owner 为 `Admin`、其他为 `User`；System/Device → `Admin(None)`；**App/Agent token 直接拒绝**。
- ACL：路径 ACL，principal 只有 `public`（只读）、`zone_users`、`user:<id>`（`acl.rs:24-27`）；`resolve_for(actor, locator, Level, op)` 统一判权。首次启动种子 `srv/library`（zone_users 写）与 `srv/publish`（public 读）。
- **分享已有半成品**：`grant`/`revoke`（`mutate.rs:833-901`）签发 32 字节随机 token，库里只存 sha256；`grants` 表（`filedb.rs:241-251`）有 `cap_id, token_hash, subtree, ops_json, audience, expires_at, max_uses, revoked, created_at`。subtree 是**活路径**（`cyfs:///...`、`view://`、`collection://`），要求 `Manage` 权限。`grant_by_token_hash`（`filedb.rs:1427`）已写但**无人调用**：数据面不认 cap（`nfs_server.md` §2.2 原话“数据面尚不放行”）。websdk `nfsp.ts:702-720` 已有 `grant/revoke`。
- **NDN 未接入**：Cargo.toml 无 `ndn-lib / named_store / ndn-toolkit`；`nfs_server.md` §5 的 NamedStore 集成（M4：link 模式 commit、Link → Store 提升、repr 缩略图）未实现。filedb 的 `full_hash` 是整文件 sha256，以 `obj_id: "sha256:<hex>"` 返回（`namespace.rs:483`），不是 NDN ChunkId/FileObject id。
- `access_urls`（`namespace.rs:560-577`）目前只返回 `fs` 与 `read` 两种；NFSP §3.6 规划的 `public`、`pinned`、`signed` 都未实现。Desktop 把 `read` 当 `publicUrl` 用（`filebrowser/data/nfsp/mapping.ts:107-117`），只有带 public ACL 的路径才真能匿名读。
- filedb 无 schema 版本，`SCHEMA` 全是 `CREATE TABLE IF NOT EXISTS`（`filedb.rs:271-282`），加新表无需迁移框架。
- TaskManager：复制功能已用 `create_delegated_task`，schema `nfs.copy/v1`（`buckyos-api/src/nfs_copy.rs:5`，`copy.rs:216-567`），是本模块任务的直接范本。
- 根目录发现：`config_from_data_root`（`config.rs`）把 `$BUCKYOS_ROOT/data` 一级子目录作为 export root（`home`、`srv`…），filedb 在 `data/.fsdb/filedb.sqlite`。nfs-server 进程能直接读这些磁盘路径。

### 2.2 named store 与 NDN 对象（cyfs-ndn）

- `get_buckyos_api_runtime()?.get_named_store()`（`buckyos-api/src/runtime.rs:292-311`）返回 `NamedDataMgr`，配置 `$BUCKYOS_ROOT/storage/named_store.json`，**同节点所有服务共享**，不跨 OOD 复制。workspace 已声明 `ndn-lib / named_store / ndn-toolkit`（`src/Cargo.toml:60-62`，均 `branch = "main"`）。
- 读：`open_reader(obj_id, inner_path)`、`open_chunk_reader(chunk_id, offset)`、`open_chunklist_reader`（`named_store/src/ndm.rs:879-1071`），Range 只支持起点，终点由调用方截断。单 chunk 上限 32 MiB。
- 保留：`pin(obj_id, owner, PinScope::{Recursive|Skeleton|Lease}, ttl)`、`unpin`、`unpin_owner`（`ndm.rs:1279-1310`）。新 put 的对象默认是 GC 候选；目前无自动 GC 循环，只有 `forced_gc_until`。无磁盘占用统计 API。
- 文件 → 对象：`ndn_toolkit::cacl_file_object(store, path, template, use_chunklist, CheckMode::ByFullHash, StoreMode, cb)`（`ndn-toolkit/src/tools.rs:156`），>32 MiB 才用 chunklist；`cacl_dir_object`（`:375`）递归。参考实现 `src/frame/opendan/src/objects.rs:21-62`，但它对 ≥12 KiB 文件用 `StoreMode::LocalFile`（链接原文件），**不满足不可变**；分享必须 `StoreInNamedMgr`，这也正是 `nfs_server.md` §5.1“grant 时 Link → Store 单向提升”的规定。
- `FileObject { name, size, content, meta(flatten) }`，meta 顶层平铺；`DirObject` 内嵌子 FileObject。obj_id 为 JCS 规范化后 sha256。

### 2.3 gateway 与主机名

- `boot_gateway.yaml`：`get_app_info_from_req`（`:138`）按 host 查 `APP_INFO`，条目带 `service_id` 时直接设 `SERVICE_ID` 走 `forward_to_service`；服务路径不做登录检查。转发是对原 HTTP 请求的代理，`REQ.host` 在链里可用，后端能否拿到原始 `Host` 需 M0 实测（§3.5）。
- `public.<zone>` → dir 服务 `../data/srv/publish/`（`:76-79`、`:116-118`）。
- scheduler 把 `_`、`www`、`sys`、`homestation` 映射到 control-panel（`system_config_agent.rs:1210`、`:1306-1324`）；保留主机名在两处硬编码（`install_plan_executor.rs:178-186`、`system_config_agent.rs:1660-1666`）。TLS 含 `*.<zone>` 通配（`:1371`）。新增 `share` host 要改这三处。

### 2.4 MessageHub / msg-center

- 附件 = `MsgObject.content.refs[] = {role, target:{type:"data_obj", obj_id, uri_hint?}, label?}`（`ndn-lib/src/msgobj.rs:199-245`）。接收方经 `GET /kapi/msg-center/objects/{obj_id}[/content]` 读，**授权依据是消息引用了该对象**，从同节点 named store 读，整段缓冲 64 MB 上限（`msg_center/src/object_access.rs`）。
- Desktop：`ForwardDialog.tsx` 的 `store.forward()` 按 obj_id 转发已有 data_obj 而不重新上传；启动参数 `messageHubLaunchSchema`（`messagehub/launch.ts:4-12`）可扩展；无“分享意图”深链。

### 2.5 HomeStation

- 发动态：kRPC `publish.create {key, input: PublishInput}`（`homestation/src/api.rs:151-156`），`PublishInput { text, attachments[], link?, article?, audience, category?, tags, zone_feed }`（`publish.rs:633-650`）。`Attachment { id, kind, name, status, object: Option<obj_id> }`。
- 校验（`publish.rs:652-686`）：附件 ≤ 9（`MAX_MEDIA_PARTS`），`status=="uploaded"` 且有 `object`，视频/音频各 ≤ 1；只有 `image/video/audio` 三类会进动态内容（`:848-878`），其他 kind 被忽略。`link` 需 http(s) URL、标题 ≤ 120。
- **可直接引用 named store 里的对象**：`ensure_file` → `objects::load_local`（`objects.rs:368-397`），本库没有时从 named store 取 `cyfile` 并校验 id 后入库。所以 nfs-server 快照出的单 chunk FileObject 可直接作为附件。
- 限制：上传上限 32 MiB（`http.rs:24`）；对象读取路由 `/home/<user>/objects/{id}/content` 只支持单 chunk（记忆中的“未做：chunk list 大文件”）。
- Desktop HomeStation App 无启动参数，composer 在 `app/homestation/publish/PublishComposer.tsx`。

### 2.6 control-panel 旧桩（待删除）

- `src/frame/control_panel/src/share_content_mgr.rs` 实现 buckyos-api 的 `ContentMgrHandler`（`content_mgr_client.rs:714`，service id `publish-content-mgr`），以 `name` 为主键，DB 硬编码 `/opt/buckyos/data/...`，未在 control-panel `main.rs` 接线，websdk 无客户端，仓库无调用者，无存量数据。删除即可，§19.2 不触发迁移。

### 2.7 Desktop

- 免登录路由 `publicRoutes.ts`；AI Workspace 访客页与控制台同源、共享 localStorage 与 host-only 的 `buckyos_session_token` cookie（`sys_auth_backend.rs:707-715`），只靠服务端授权隔离。
- File Browser 菜单有 “Share…” 但命令硬连 `unsupported`（`filebrowser/menu/commands.ts:14`）；权限对话框是真实 `get_acl/set_acl`。
- System Dialog 只有 `sysdlg/app_installer`（`desktop/src/sysdlg/index.tsx`）；独立 URL 无结果回传通道。
- 新 App 注册三处：`app/registry.tsx`、`app/backend-apps.ts` 的 `DESKTOP_BUILTIN_APP_IDS`、`mock/data.ts` 目录项。NFSP 调用经 websdk `NfspClient`（Desktop 吃 websdk dist，改 src 后要重建 dist）。

### 2.8 结论

| PRD 要求 | 现状 | 本设计 |
| --- | --- | --- |
| 内置分享提供服务 | nfs-server 有 cap 签发，数据面不放行 | nfs-server 加 `/s/{token}` 数据面（§3.5） |
| 快照不可变 + 保留 | nfs-server 未接 NDN；`cacl_file_object` + `pin` 现成 | nfs-server 接 named store，分享时 Store 模式快照并 pin（§3.4） |
| 读取路径授权覆盖 chunk/Range | NDN 直链无鉴权 | 不暴露 obj_id 直链，只经 `/s/*`（§3.5） |
| Public 目录 | `data/srv/publish/` 已被 gateway 匿名服务 | nfs-server 投影 + `access_urls.public`（§3.9） |
| 结果身份、修订、审计 | `grants` 只有凭据字段 | filedb 新增 `share_*` 表，凭据仍用 `grants`（§3.3） |
| 应用分享回传 | 无 | MessageHub + HomeStation，`share.app.*`（§3.10） |
| 消费情况 | 无采集 | `/s/*` 访问日志 + 小时聚合（§3.13） |

## 3. 设计

### 3.1 分层与接线

```text
Desktop   sysdlg/share   我的内容(my-content)   File Browser   MessageHub / HomeStation(发送后回传)
              └──────────────┴───────── websdk NfspClient ───────┴──────────────┘
                                         │ POST /nfs/v1/share.*（NFSP 信封）
gateway   <zone>/nfs/v1/* ───────────────┤
          share.<zone>/s/{token}… ───────┤（APP_INFO["share"] → nfs-server）
                                         ▼
nfs-server  share/ { mod, db, snapshot, serving, page, stats, public, sweeper, rpc }
              │ resolve_for(actor, Read)   │ cacl_file_object / pin / open_reader   │ create_delegated_task
              ▼                            ▼                                        ▼
            本地 FS + ACL（同进程）       named store（本节点共享）                 task-manager（nfs.share_snapshot/v1）
```

- 代码落点：`src/frame/nfs_server/src/share/` 子模块；`server.rs` 的 `dispatch` 增加 `share.*` 分支，`build_router` 增加 `/s/{token}` 系列路由；`state.rs` 的 `AppState` 增加 `share: ShareMgr`（持 named store 句柄、页面签名 key、统计缓冲）。
- 管理接口用 NFSP 信封而不是另起 kRPC：与 File Browser 同一会话、同一鉴权、同一 websdk 客户端，`hello` 的 `features` 增加 `share`。`share.*` 中的写方法加入 `WRITE_METHODS`，享受 seq 重放；跨页面重试另用显式 `idempotency_key`（§3.6）。
- 既有 `grant/revoke` 保持为 NFSP 低层原语（活子树 cap），不进“我的内容”。`share.create` 内部复用 `grants` 表签发凭据，subtree 写 `obj:<root_obj_id>`，与活子树 cap 区分。
- 源读取在进程内判权：`resolve_for(actor, locator, Level::Read, "read")`，与 File Browser 同一条代码路径（D6 的“ACL 单一权威”在同进程内成立，无需 REST 自调用）。

### 3.2 身份与权限

- `owner_user_id = actor.user_id()`；`Admin(None)`（System/Device token）创建分享时必须显式带 `on_behalf_of` 用户并记审计 `source=system`。
- 普通用户只能读写自己的分享；`Admin(Some(_))` 可跨用户管理，审计 `source=admin`，但不因此获得源内容读取权（列表不返回他人分享的 token/URL）。
- 创建分享需要源路径 `Read`（PRD：读取 ≠ 外发，但首期不再单列“外发”权限，按“先能力后安全”记为已知风险）；停止/修改需要是分享 owner 或 Admin。
- App/Agent（D13）：`share.*` 方法接受 App token，按 `sub` 视为该用户、`app_id` 记入操作与审计（`actor_app_id`）；文件平面方法仍拒绝 App/Agent。实现上给 `Actor` 增加 `Delegated { user, app }` 变体，ACL 判定在 `share.*` 内按 `User(user)` 执行，其余 dispatch 分支直接拒绝该变体。
- 回传（`share.app.report`）要求调用者是该分享 owner 本人（首期 MessageHub、HomeStation 都在 Desktop 前端以用户会话回传）；owner 不可被回传更改。
- 访客请求不读任何 session cookie，只认 token 与密码解锁后的 `share_access` cookie。

### 3.3 数据模型（filedb 新增表）

全部写进 `data/.fsdb/filedb.sqlite`，追加到 `filedb.rs` 的 `SCHEMA`。凭据沿用 `grants`（`cap_id`、`token_hash`、`expires_at`、`revoked`），其余事实放新表。

```sql
CREATE TABLE IF NOT EXISTS share_operations(
  operation_id TEXT PRIMARY KEY,
  intent TEXT NOT NULL,                 -- create_link | app_handoff | update | stop
  owner_user_id TEXT NOT NULL,
  actor_user_id TEXT NOT NULL,
  actor_app_id TEXT,                    -- Delegated 调用时的 app
  input_json TEXT NOT NULL,
  idempotency_key TEXT NOT NULL,
  task_id TEXT,
  share_id TEXT,
  state TEXT NOT NULL,                  -- preparing | running | completed | failed | canceled
  error_json TEXT,
  created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL,
  UNIQUE(owner_user_id, idempotency_key)
);

CREATE TABLE IF NOT EXISTS share_versions(
  obj_id TEXT PRIMARY KEY,              -- cyfile:… / cydir:…
  obj_type TEXT NOT NULL,
  root_name TEXT NOT NULL,
  mime TEXT,
  total_size INTEGER NOT NULL,
  item_count INTEGER NOT NULL,
  source_path TEXT NOT NULL,            -- 快照时的 cyfs:/// 路径，私有，不进对象 meta
  source_mtime INTEGER, source_size INTEGER,
  excluded_json TEXT,
  prepared_at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS shares(
  share_id TEXT PRIMARY KEY,
  kind TEXT NOT NULL,                   -- link | app_delivery
  owner_user_id TEXT NOT NULL,
  created_by_app TEXT,
  title TEXT NOT NULL,
  obj_id TEXT NOT NULL REFERENCES share_versions(obj_id),
  cap_id TEXT,                          -- kind=link 时指向 grants.cap_id
  related_share_id TEXT,                -- HomeStation 链接卡片模式下，投递与链接分享互指（D14）
  status TEXT NOT NULL,                 -- active | stopping | stopped | expired | failed
  status_reason TEXT,
  hidden INTEGER NOT NULL DEFAULT 0,
  revision INTEGER NOT NULL DEFAULT 1,
  created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL, stopped_at INTEGER
);
CREATE INDEX IF NOT EXISTS idx_shares_owner ON shares(owner_user_id, status);

CREATE TABLE IF NOT EXISTS share_link_policy(
  share_id TEXT PRIMARY KEY REFERENCES shares(share_id) ON DELETE CASCADE,
  access TEXT NOT NULL,                 -- holder | password
  password_hash TEXT,                   -- pbkdf2-sha256 PHC
  password_rev INTEGER NOT NULL DEFAULT 0,
  allow_download INTEGER NOT NULL DEFAULT 1
);

CREATE TABLE IF NOT EXISTS share_retentions(
  share_id TEXT NOT NULL REFERENCES shares(share_id) ON DELETE CASCADE,
  obj_id TEXT NOT NULL,
  pin_owner TEXT NOT NULL,              -- "share:<share_id>"
  pinned_at INTEGER NOT NULL, released_at INTEGER,
  PRIMARY KEY(share_id, obj_id)
);

CREATE TABLE IF NOT EXISTS share_deliveries(
  share_id TEXT PRIMARY KEY REFERENCES shares(share_id) ON DELETE CASCADE,
  target_app TEXT NOT NULL,             -- messagehub | homestation
  handoff_at INTEGER NOT NULL,
  delivery_state TEXT NOT NULL,         -- handed_off | confirmed | canceled | failed | unknown
  business_ref_json TEXT,               -- messagehub {session_id,message_id} / homestation {entry,obj_id}
  reported_at INTEGER, error TEXT
);

CREATE TABLE IF NOT EXISTS share_audit(
  audit_id INTEGER PRIMARY KEY AUTOINCREMENT,
  share_id TEXT NOT NULL, action TEXT NOT NULL,   -- create | update | stop | expire | hide | report
  revision_before INTEGER, revision_after INTEGER,
  actor_user_id TEXT NOT NULL, actor_app_id TEXT,
  source TEXT NOT NULL,                 -- gui | cli | agent | system | admin
  reason TEXT, outcome TEXT NOT NULL, at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS share_access_log(
  log_id INTEGER PRIMARY KEY AUTOINCREMENT,
  share_id TEXT NOT NULL, at INTEGER NOT NULL,
  kind TEXT NOT NULL,                   -- page | meta | download | unlock_fail
  status INTEGER NOT NULL, bytes INTEGER NOT NULL DEFAULT 0,
  visitor TEXT,                         -- sha256(ip + UA + 日期 + 服务盐)[..16]，按天去重用
  inner_path TEXT
);
CREATE INDEX IF NOT EXISTS idx_access_share_at ON share_access_log(share_id, at);

CREATE TABLE IF NOT EXISTS share_access_stats(
  share_id TEXT NOT NULL, bucket INTEGER NOT NULL,   -- 小时桶起点（秒）
  views INTEGER NOT NULL DEFAULT 0, downloads INTEGER NOT NULL DEFAULT 0,
  bytes INTEGER NOT NULL DEFAULT 0, visitors INTEGER NOT NULL DEFAULT 0,
  PRIMARY KEY(share_id, bucket)
);

CREATE TABLE IF NOT EXISTS public_observations(
  rel_path TEXT PRIMARY KEY,            -- 相对 srv/publish
  entry_kind TEXT NOT NULL,             -- file | dir | symlink
  link_target TEXT, link_ok INTEGER,
  size INTEGER, mtime INTEGER,
  first_seen_at INTEGER NOT NULL, last_seen_at INTEGER NOT NULL, missing_since INTEGER
);
```

与 PRD §5.2 的对应：操作记录 = `share_operations`；结果记录 = `shares`；内容版本 = `share_versions`；授权 = `grants` + `share_link_policy`；保留 = `share_retentions`；外部投影 = `share_deliveries`；审计 = `share_audit`。发布关系与名称绑定第二阶段再加，`shares.kind` 预留 `publication`。

### 3.4 内容快照

1. 输入 `source`：NFSP locator 或 Ref（与 File Browser 已持有的一致），首期只接受 native 文件/目录；View/Collection 第二阶段。
2. `resolve_for(actor, locator, Level::Read)` 得到 `Node::Native{root, rel}`，`join_root` 得磁盘路径。符号链接按真实路径重新判权（与 `nfs-server-acl-impl` 的“读文件链接按真实路径授权”一致）。
3. 目录排除默认规则：`.git/`、`node_modules/`、`.DS_Store`、`*.key`、`*.pem`、`.env*`；`share.plan` 返回排除清单。
4. `cacl_file_object(..., StoreMode::StoreInNamedMgr)` / `cacl_dir_object`。FileObject meta 只放 `mime`；不放来源路径、设备、EXIF。
5. `pin(root_obj_id, "share:<share_id>", PinScope::Recursive, None)`，写 `share_retentions`、`share_versions`。
6. 快照后再 `stat` 一次；size/mtime 变了则返回 `source_changed=true`，由前端让用户选“用已形成的快照”或“重做”（PRD §8.1）。
7. 同步/异步：总大小 ≤ 64 MiB（D7）在请求内完成；否则建 `nfs.share_snapshot/v1` 任务（照 `nfs.copy/v1`：`user_creatable=false`，creator = 用户 + app，首次使用自注册 schema），`report_progress` 报字节进度，结果 `{obj_id, total_size, item_count}`。
8. 失败只回收本次新建且无其他分享引用的 pin；已被引用的对象不动。
9. 相同 obj_id 的 `share_versions` 已存在时直接复用（不重算），但每个分享自己 pin 一次。

### 3.5 分享数据面

```text
https://share.<zone>/s/{token}              访客页（服务端模板 HTML，无 SDK）
https://share.<zone>/s/{token}/meta         JSON：title, kind, size, items[], expires_at, requires_password, allow_download
https://share.<zone>/s/{token}/unlock       POST {password} → Set-Cookie share_access=…; Path=/s/{token}; HttpOnly; Secure; SameSite=Strict
https://share.<zone>/s/{token}/d            单文件下载或内联预览
https://share.<zone>/s/{token}/d/{inner}    目录分享内的文件
```

- 与 NFSP §7.3“cap in path”与 §3.6 `access_urls.signed` 一致：分享成功后，源文件的 `access_urls` 增加 `{kind:"signed", url, expires_at, cap_id, revocable:true}`（只对分享 owner 返回）。
- token：沿用 `op_grant` 的 32 字节 URL-safe base64；库存 sha256。share_id 不进公开 URL。
- 每请求：`grant_by_token_hash` → `revoked=0` 且未过期 → 联 `shares.status=active`；任一不满足返回 404（不区分不存在/停止/到期）。subtree 不是 `obj:` 前缀（活子树 cap）的 token 首期也返回 404。
- 密码：`share_access` cookie 是 nfs-server 自签 HMAC 令牌 `{share_id, exp ≤ min(expires_at, now+12h), password_rev}`，key 每进程随机或持久化到 filedb（重启后要求重输密码可接受，选随机）。改密码 `password_rev++`，旧 cookie 立即失效（T05）。`unlock` 每 token 每分钟 10 次限速。
- 下载：`open_reader(obj_id, inner_path)` 拿 FileObject，`open_chunk_reader/open_chunklist_reader(offset)` 流式输出，自行处理 `Range: bytes=a-b` 的终点；响应 `Content-Type`（meta.mime 或 `fsutil::content_type_for`）、`Content-Disposition`（图片/视频/音频/PDF/纯文本 inline，其余 attachment）、`Accept-Ranges`、`Cache-Control: private, no-store`、`X-Content-Type-Options: nosniff`、`Content-Security-Policy: sandbox; default-src 'none'`。HTML 只下载不渲染。
- 目录：访客页列 DirObject 子项可下钻；不打包 zip（第二阶段）。
- Host 约束：nfs-server 只在 `share.<zone>` 上服务 `/s/*`，在该 host 上拒绝 `/nfs/v1/*`。判断依据是请求 `Host`；M0 实测 gateway 是否保留原始 Host，若不保留，在 `boot_gateway.yaml` 的 forward 前 `map-add REQ X-Forwarded-Host $REQ.host`（`:113-115` 已有设 `X-Forwarded-Proto` 的先例）。
- 路由：scheduler 三处加 `share`（§2.3）：`APP_INFO["share"] = Service{service_id:"nfs-server"}`，两份保留主机名加 `share`。
- 访问日志异步写 `share_access_log`，失败不影响响应（§3.13）。

### 3.6 NFSP 扩展方法 `share.*`

全部走 `POST /nfs/v1/share.<op>`，信封同其他方法，需 hello 会话；写方法要 `seq`。

| 方法 | 写 | 参数 | 返回 |
| --- | --- | --- | --- |
| `share.capabilities` | | `{source?}` | `{actions:["create_link","app_handoff"], access:["holder","password"], expiry_presets:[86400,604800,2592000,null], limits:{sync_threshold, max_total_size}, targets:[{app:"messagehub"},{app:"homestation", accepts:["image","video","audio"], max_item_size}], blocked:[{action,reason}]}` |
| `share.plan` | | `{intent, source, policy?, target?}` | `{plan_fingerprint, estimate:{total_size,item_count,excluded[]}, warnings[], requires_task, existing:[{share_id,access,expires_at}], source_public:{in_publish_dir, public_acl}, target_fit?}`；无副作用 |
| `share.create` | ✓ | `{plan_fingerprint, source, policy:{access, password?, expires_at?, allow_download?}, title?, idempotency_key}` | `{operation_id, state, share_id?, task_id?, url?, source_changed?}`；同 key 重放返回原操作，同 key 不同输入 `IDEMPOTENCY_MISMATCH` |
| `share.operation` | | `{operation_id}` | 操作全貌；任务态按 §3.7 映射 |
| `share.get` | | `{share_id}` | 分享、策略、版本、保留、投递、审计摘要、统计摘要；`url` 只给 owner |
| `share.list` | | `{kind?, status?, source?, obj_id?, q?, include_hidden?, limit, cursor}` | `{items[], next_cursor}`；只含主体可见条目 |
| `share.update` | ✓ | `{share_id, revision, changes:{title?, expires_at?, password?:string\|null, allow_download?}}` | `{revision}`；不符返回 `REVISION_CONFLICT` + 当前记录 |
| `share.stop` | ✓ | `{share_id, revision, reason?}` | `{status}`；先置 `stopping`（访客即 404），unpin 成功后 `stopped` |
| `share.hide` | ✓ | `{share_id, hidden}` | 仅视图 |
| `share.app.handoff` | ✓ | `{source, target_app, idempotency_key}` | 快照 + 建 `app_delivery` → `{share_id, obj_id, name, size, mime, mode:"attachment"\|"link", link_share_id?, url?, launch:{appId, payload}}`；`mode=link` 只出现在 HomeStation 不适配附件时（D14） |
| `share.app.report` | ✓ | `{share_id, delivery_state, business_ref?, error?, observed_at}` | 比较 `observed_at` 拒绝旧事实（`STALE_REPORT`）→ `{accepted}` |
| `share.public.list` | | `{refresh?}` | `{entries[], last_scan_at, scan_ok}` |
| `share.stats` | | `{share_id?, from, to, granularity:"hour"\|"day"}` | 单个分享或本人全部分享的 views/downloads/bytes/visitors 序列与总计 |

错误：沿用 NFSP 错误信封，`code` 用现有 `ErrorCode`，业务原因放 `reason`：`source_not_readable`、`source_changed`、`too_large`、`idempotency_mismatch`、`revision_conflict`、`not_owner`、`share_not_active`、`target_unsupported`、`stale_report`。

### 3.7 TaskManager 映射

- schema `nfs.share_snapshot/v1`（常量放 `buckyos-api/src/nfs_copy.rs` 旁，新文件 `nfs_share.rs`），publisher `nfs-server`，`default_storage_domain = User`。
- `Promised/Accepted/Running/Waiting → running`，`Terminal+Succeeded → completed`，`Failed → failed`，`Canceled → canceled`；`operation.state` 是业务态，`task_id` 另给。
- 取消只停快照；已生成的 pin 与分享不回滚，走 `share.stop`。
- 重启恢复：`share_operations.state in (preparing, running)` 有 `task_id` 的查任务补写；无任务的同步操作标 `failed(interrupted)`。

### 3.8 生命周期

- `revision` 只在 update/stop/hide 时 +1；改密码或期限不产生新 obj_id。
- sweeper（60 s）：`expires_at <= now` 的 active → `expired`，unpin，审计 `expire/system`；`stopping` 重试 unpin。数据面每请求也判时间，不依赖 sweeper。
- 重新分享已停止的内容：默认新分享、新 token；`share_versions` 仍在且 `is_object_stored` 为真时复用 obj_id。

### 3.9 Public 投影

- Public = `data/srv/publish/`（D4）。nfs-server 的 reconciler 本来就扫描 export root，在其扫描周期里顺带对 `srv/publish` 子树更新 `public_observations`（首次发现、消失、symlink 目标是否越界：解析后必须仍在 `srv/publish` 或 `srv/library` 内）。`share.public.list {refresh:true}` 立即扫一次。
- `access_urls`：路径位于 `srv/publish` 下的文件增加 `{kind:"public", url:"https://public.<zone>/<rel>"}`，File Browser 的“复制公开地址”改用这一条；`read` 条目不再被当成公开地址。
- 只报告“路径可访问”，不宣称 NDN 对象就绪（PRD §11、T09）。删改由 File Browser 完成。
- `share.plan.source_public`：源在 `srv/publish` 下，或 ACL 含 `public` 读（T11）。

### 3.10 通过应用分享

**MessageHub（发消息附件）**

1. 面板选 MessageHub → `share.app.handoff`，`delivery_state=handed_off`。
2. Desktop `openAppWindow('messagehub', {launch:{requestId, payload:{share:{shareId, objId, name, size, mime}}}})`；`messageHubLaunchSchema` 加 `share`，复用 `ForwardMessageForm` 选人，以 `{type:'data_obj', obj_id}` 写 refs 发送，不重新上传。
3. 发送成功 → `share.app.report {confirmed, business_ref:{session_id, message_id}}`；关闭未发 → `canceled`；无回传显示“结果未确认 + 交接时间”（T06）。
4. 接收方经 msg-center 对象路由读取，授权来自消息引用。停止这条投递只释放 nfs-server 的保留，不撤回消息；msg-center 独立保留有效消息所引用的附件及其依赖（D17，待 §10 R05 实施）。

**HomeStation（发动态）**

1. `share.capabilities/plan` 按 HomeStation 的约束给出 `target_fit`：图片/视频/音频、单个 ≤ 32 MiB（单 chunk）、总数 ≤ 9、视频/音频各 ≤ 1。适配时 `mode=attachment`，否则 `mode=link`（D14），面板在提交前明示“将创建一条不自动到期的公开链接”。
2. `share.app.handoff {target_app:"homestation"}` 快照（≤ 32 MiB 必然是单 chunk FileObject，HomeStation 的 `load_local` 能从 named store 拿到）。`mode=link` 时在同一操作里再建一条 `kind=link` 的分享（`access=holder`、`expires_at=NULL`），返回其 `url` 与 `link_share_id`；两条分享在“我的内容”里互相关联。
3. Desktop 打开 HomeStation 并进入 composer：HomeStation App 增加启动参数 `{compose:{attachments?:[{kind, name, object, status:"uploaded"}], link?:{url, title, summary?}, text?}}`，`PublishComposer` 以预填附件或链接卡片打开；用户选可见范围、写文字后由 HomeStation 自己调 `publish.create`。链接卡片标题取分享标题并截到 120 字。
4. `publish.create` 返回后前端回传 `share.app.report {confirmed, business_ref:{entry, obj_id}}`。动态的可见性与撤回归 HomeStation 管，“我的内容”里给“在 HomeStation 查看”入口。
5. 停止这条投递只释放 nfs-server 的 pin；HomeStation 的 `object_grants` 决定可读性，附件及 chunk 还须由 HomeStation 独立保留（D17，待 §10 R05 实施），不能把对象元数据入库当作内容已保留。文案说明“撤回动态请在 HomeStation 操作”。`mode=link` 时停止关联的链接分享会让动态里的链接失效，停止确认框要写明这一点。

两个目标都要求 nfs-server、msg-center、HomeStation 与 named store 在同一节点；单 OOD 成立，多 OOD 部署记为已知限制。

### 3.11 上线衔接（D9）

- PIKG / AppDID / URL 输入：面板给“在我的 BuckyOS 上线”，`useSystemDialog().open('sysdlg/app_installer', {identifier})`。
- HTML 目录：显示“受管理站点上线尚未交付”，给两条替代：复制到 Public（说明匿名公开）或创建分享链接。不静默复制到 Public。
- “我的内容”的“网站与服务”页只读既有 app service 状态与 `StaticWebDeploymentEvidence`，管理动作跳 App Service。

### 3.12 前端

- **`sysdlg/share`**：参数 `{ source: NfspLocator | NfspRef, title?: string, preferred?: 'link'|'messagehub'|'homestation' }`，返回 `{action:'created', shareId, url} | {action:'handed_off', shareId, app} | {action:'close'}`。首屏：摘要 + 已有有效分享（可复制，显示范围/期限）+ 动作（创建链接 / 发到 MessageHub / 发到 HomeStation / 上线）。链接卡：范围（持链接者/密码）、期限（1 天/7 天/30 天/不自动到期/自定义，显示时区）、固定快照说明。进度卡：同步直接等，异步显示任务进度可“后台继续”。只做 SPA 内调用。
- **File Browser**：`share` 命令改为打开 `sysdlg/share`；“复制公开地址”只在 `access_urls` 有 `public` 时出现。
- **“我的内容”**（app id `my-content`，显示名 zh“我的内容” / en“My Content”，D12）：
  - 概览：本期总访问、下载、流量、访客数，按天折线；活跃分享数、即将到期数。
  - 列表 tab：全部 / 分享链接 / 应用发出（MessageHub、HomeStation）/ Public / 网站与服务。列：名称、类型、访问范围、状态、访问量、更新时间；筛选按 PRD §13。
  - 详情抽屉：快照 obj_id（折叠，复制完整）、链接、期限、密码开关、保留、审计、该分享的消费序列（小时/天）。
  - 预留“收入”列与“购买”统计位，第三阶段接交易（首期不显示）。
  - 注册三处见 §2.7；mock-first（`VITE_MY_CONTENT_USE_MOCK`）；i18n en + zh 全量；浅色深色都截图核对（Desktop 主题约定）。
- **访客页**：nfs-server 内嵌模板（`include_str!` 的 HTML + 内联 CSS/JS），显示标题、大小、到期、密码框、预览区（img/video/audio/pdf/text 原生渲染，其余给下载）。不加载 websdk，不读站点 cookie。
- `product/control_panel/share_content.md` 的两行并入 `BuckyOS Sys Dlg.md` 新增的 `sysdlg/share` 小节。

### 3.13 消费统计

- 采集点只有 `/s/*` 数据面：page / meta / download / unlock_fail 四类事件进内存缓冲，每 5 s 或 200 条批量写 `share_access_log`，同时累加 `share_access_stats` 小时桶；`visitor` 用按天加盐哈希去重，不存原始 IP。
- 日志保留 90 天（sweeper 删旧），小时桶永久保留。
- 不采集：Public（gateway dir 服务直出，无日志钩子）；MessageHub 接收方读取（msg-center 路由）；HomeStation 的阅读与互动（HomeStation 自有）。“我的内容”对这些只显示“统计不可用”，不编造。HomeStation 互动数后续可通过其 `published.list` 拉取，列入第二阶段。

### 3.14 websdk 与 CLI

- `buckyos-websdk/src/nfsp.ts`：`NfspClient` 增加 `shareCapabilities / sharePlan / shareCreate / shareOperation / shareGet / shareList / shareUpdate / shareStop / shareHide / shareAppHandoff / shareAppReport / sharePublicList / shareStats`；改完重建 dist。
- CLI（websdk `cli/modules`）：`buckyos share create <cyfs-path> [--expires 7d|none] [--password] [--title]`、`share list`、`share get`、`share stop`、`share update`、`share stats`。
- cmd_skills：CLI 是用户可见的新命令，按 AGENTS.md 联动规则检查 `buckyos_admin` 手册，增加“分享文件给人”的条目，或新建 `share` 手册（实施时判断，提交说明写依据）。

### 3.15 安全清单与已知风险

- token 256 bit、只存哈希；密码 pbkdf2-sha256（`pbkdf2` 已在 Cargo.lock）≥ 600k 轮；解锁限速；404 不区分状态；日志不打 token 与密码。
- 分享响应带 `CSP: sandbox`，HTML 只下载不渲染；访客页在 `share.<zone>`，与控制台不同源（T35）。
- 已知风险（按“先能力后安全”记录）：Agent 经 Delegated 身份可外发其 owner 可读的文件（D13）；读取权限即可外发，无单列“外发”授权；多 OOD 时快照所在节点与 msg-center/HomeStation 不同会读不到。
- M0–M5 核对发现：`/ndm/*`、`/ndm/proxy/v1` 由 gateway 直转且 control-panel 处理器内无会话检查。2026-10-20 已批准纳入后续独立迁移切片（D18–D19、§10 R07），不再仅记录为另开 issue。

## 4. 决策

### 4.1 已定（2026-10-19，用户回复原话保留）

| # | 问题 | 决定 | 落实 |
| --- | --- | --- | --- |
| D1 | 分享提供服务宿主 | “cyfs-dir 设计上是方便 static 快速部署的，按设计 buckyos 的 nfs-server 是完整支持 cyfs 协议的文件服务器，可以在这里处理 ndn 的相关逻辑” | 数据面放 nfs-server（§3.5），不用 cyfs-gateway 的 cyfs-dir |
| D2 | 链接与凭据形态 | “都行吧，一般 cookie 好点” | 路径 token + Path 限定的 HttpOnly cookie |
| D3 | 访客页宿主 | “好” | `share.<zone>` → nfs-server（§3.5） |
| D4 | Public 的定义 | “A，这个还是保持现有熟悉的语义” | 固定目录 `data/srv/publish/`（§3.9） |
| D5 | 存储位置与旧代码 | “我倾向于整个后台移动到 nfs-server” | 全部后台进 nfs-server，表进 filedb（§3.3）；control-panel 旧桩与 `content_mgr_client.rs` 删除 |
| D6 | 源读取权限校验 | “A)” | nfs-server 同进程 `resolve_for(Read)`，ACL 单一权威 |
| D7 | 同步/异步阈值 | “好” | 64 MiB |
| D8 | 应用分享首期目标 | “MessageHub 和 HomeStation，HomeStation 就是发动态” | §3.10 两个目标 |
| D9 | 上线衔接范围 | “好” | 只路由到 app_installer + 投影 |
| D10 | 密码哈希 | “好” | pbkdf2 |
| D11 | 访客页实现 | “好” | 服务端模板，现由 nfs-server 内嵌 |
| D12 | 管理中心承载 | “独立 app 吧，可以叫‘我的内容’之类的名字，毕竟规划上支持通过内容赚钱，要有一个地方看到内容的消费情况” | `my-content`“我的内容”，含消费统计（§3.12、§3.13），预留收入位 |

### 4.2 已定（2026-10-19 第二轮，用户：“这 3 个问题都按你的建议来”）

| # | 问题 | 决定 | 落实 |
| --- | --- | --- | --- |
| D13 | nfs-server 是否接受 App/Agent token 调 `share.*` | 接受，按 token `sub` 视为该用户并记录 app；文件平面仍拒绝 | 新增 `Actor::Delegated { user, app }`（§3.2）；外发风险记入 §3.15 |
| D14 | HomeStation 不支持的内容（PDF 等非媒体，或单个 > 32 MiB）怎么发动态 | 自动建一条不自动到期、持链接者的分享链接，以 `link` 卡片发动态 | `share.app.handoff` 的 `mode=link`（§3.6、§3.10），`shares.related_share_id` 关联两条分享（§3.3） |
| D15 | 给 nfs-server 引入 `ndn-lib`、`named_store`、`ndn-toolkit` 依赖 | 同意引入 | 三项用 workspace 声明（`branch = "main"`）；standalone 模式加 `--named-store <dir>` 生成单桶配置供测试（M0） |

### 4.3 已定（2026-10-20，后续收尾与 breaking change）

用户确认：“这个版本是breaking change,所以可以没有负担的完成修改。你的建议我觉得都OK，补入文档吧”。以下为已批准、待实施的目标；涉及既有设计的调整以本节为准。

| # | 决策 | 已确认的执行范围 |
| --- | --- | --- |
| D16 | 本轮范围与兼容原则 | 完成 §10 的 7 项收尾，NDM 单独作为实施切片；接口、数据结构和调用方式可直接调整，联动升级服务、SDK、前端及测试，不增加兼容层、双栈、旧服务转发或双写。 |
| D17 | 应用附件保留 | MessageHub/HomeStation 接收附件后承担独立保留责任；分享停止不撤回已发送消息或已发布动态，也不释放应用的保留。 |
| D18 | NDM 宿主与协议 | nfs-server 承接 Zone 上传与 NDM proxy 网关，control-panel 移除对应初始化和挂载；继续使用同节点 NamedStore，目标路由为 `/ndm/v1/*` 与 `/ndm/proxy/v1/*`。 |
| D19 | NDM 授权边界 | 用户上传按主体隔离；proxy 按已验证身份与明确授权区分普通对象操作、读取及管理操作，保留受授权 AppClient/AppService/System/Device 的业务能力。 |

**D16：范围与实现自由度**

- PRD 与 NFSP v0 回写已获确认，作为交付项执行，不再等待单独确认。涉及 `buckyos`、`buckyos-websdk`、`cyfs-ndn`、`cyfs-gateway` 的实现、契约、测试、生成来源与分发产物按实际影响联动。
- 保持现有阶段边界：不扩展 ZIP、View/Collection 分享、HomeStation 互动统计、独立发布、长期入口、交易收入及完整网站上线；不在本轮补全 T07、T12–T16 超出既定范围的能力或 T20–T34。收入预留位未实现的状态如实记录，不扩展成交易功能。
- 多 OOD 跨节点对象复制/读取继续延期；本轮部署保持 nfs-server、msg-center、HomeStation 与目标 NamedStore 同节点。不新设独立“外发权限”，沿用现有身份、路径权限与 Agent 权限约束。
- 数据库结构、pin owner 编码、适配层、重试间隔及测试夹具由 code agent 按现有组件自行确定。breaking change 不要求实现旧版数据库格式迁移；保留的当前有效业务记录按 D17 补齐对象保留。

**D17：附件接管与释放语义**

- MessageHub/HomeStation 后端在确认接收成功前，为附件根对象及其依赖建立独立、持久的保留；不依赖前端 `share.app.report` 才建立 pin，也不把读取授权或对象元数据入库视为保留完成。
- 应用保留随有效业务引用存续，不以分享的到期时间为 TTL。停止分享、释放 `share:<share_id>` 的 pin、隐藏列表记录，都不能释放应用仍需要的对象。
- 应用的有效业务引用消失后才释放对应保留；同一对象被其他消息、动态或分享引用时仍应存续。单个用户隐藏/删除本地视图不代表所有有效引用消失，按各应用实际业务生命周期判断。
- 接收、持久化、pin/unpin 的失败与重启需要可恢复、可重试，重复接收应幂等，不能对外确认成功后留下无保留的附件。若环境保留了当前有效消息或动态，按当前可读取格式扫描并补齐对象保留；不要求兼容旧版存储格式。对象或依赖已缺失时记录未修复项与证据，不能仅凭 pin 记录创建成功就宣称补齐，也不通过旧版本兼容迁移重建历史内容。
- `mode=link` 仅交付链接，不将链接目标转为应用独立附件；关联分享停止或到期后，链接仍应失效。

**D18：迁移契约与调用方联动**

- `/ndm/v1/*` 与 `/ndm/proxy/v1/*` 作为目标协议入口，不代表必须兼容旧客户端。保留内容寻址的对象 ID 语义；请求字段、认证方式、SDK 接口与内部结构可按目标设计直接修改，消费方同轮更新。
- nfs-server 承接对应处理器、上传暂存及清理责任，网关路由转向 nfs-server；完成后删除 control-panel 的 NDM 初始化、字段及挂载。部署配置和 scheduler 派生路由按实际需要同步。
- 联动检查 SDK 的上传、续传与 proxy 客户端，Desktop MessageHub 附件上传，系统 App 与应用包上传/安装测试。服务端新增鉴权时同步 SDK 凭据传递，不能只改服务端。
- `app.publish` 仍是应用包发布，`TrustedShare` 仍是安装信任策略，不按旧分享管理器清理；它们的 NDM 消费链路如受影响，随迁移适配。

**D19：授权矩阵与上传归属**

复用现有 token 验证与 RBAC/服务授权机制，实施时列明主体、操作、资源范围的实际映射；没有可验证授权依据的请求默认拒绝，不另建独立授权体系。

| 主体/操作 | 目标约束 |
| --- | --- |
| 普通用户上传 | 创建时绑定已验证主体；查询、HEAD/PATCH 续传、完成/登记 FileObject、取消均校验归属，仅持上传 ID 不足以操作。登记对象必须校验与本次获授权上传的对应关系。 |
| 对象读取 | 已登录或知道 obj_id 不构成全库读取权；继续通过消息、动态、分享等业务授权读取，proxy 对象读取须有明确授权。 |
| AppClient/AppService/System/Device | 依据真实验证的 token 身份与授予的 NDM 权限放行相应操作；不能仅按 runtime 标签、请求自报 app_id 或 token 类别一概授权，也不能一刀切禁止既有受授权 App 消费能力。 |
| App/Agent 上传或对象操作 | 仅在获得相应 NDM 权限且满足既有主体约束时开放；允许 `share.*` 不自动授予 NDM 权限。 |
| 删除、pin/unpin、GC 等管理操作 | 与普通上传/对象操作分别授权；应用的保留操作仅作用于其受授权的业务范围，不能释放其他主体的保留。 |
| 匿名或分享链接 token | 不获得上传、proxy 或 NDM 管理权限；匿名访客仅经既定分享数据面与对应业务公开接口访问。 |

上传失败、取消、过期后的暂存清理由 nfs-server 负责；只能清理该上传持有的临时资源或保留，不能删除其他业务仍引用的内容。清理时机与重试参数由实现确定，并提供可验证的完成条件。

## 5. 实施切片

### M0：契约、清理与路由

- 删除 `src/frame/control_panel/src/share_content_mgr.rs`、`src/kernel/buckyos-api/src/content_mgr_client.rs` 及 `lib.rs` 导出；全仓 grep 确认无引用。
- nfs-server 加三项 ndn 依赖（D15）；`AppState` 接 named store（服务模式 `get_named_store()`，standalone `--named-store`）。
- `share/` 骨架、§3.3 DDL、`share.capabilities` 与空 `share.list` 接入 `dispatch`，`hello.features` 加 `share`。
- scheduler 三处加 `share` host（§2.3）；实测 gateway 转发后 nfs-server 收到的 `Host`，必要时补 `X-Forwarded-Host`。
- `BuckyOS Sys Dlg.md` 加 `sysdlg/share`；本文 §3.6 定稿为接口契约；`nfs_server.md` §2.2 / §4.1 / §5.1 回写（§7）。
- 验证：`cargo test -p nfs_server`、`cargo test -p scheduler`、`cargo test -p buckyos-api`；standalone 起 nfs-server 调 `share.capabilities`。

### M1：分享链接后端

- 快照（§3.4）、数据面与访客页（§3.5）、`share.plan/create/operation/get/list/update/stop/hide`、sweeper、`nfs.share_snapshot/v1`、访问统计（§3.13）、`access_urls.signed`。
- 单测（`tests/share.rs`，临时 named store + 测试密钥，照 `tests/acl.rs`）：无读权限拒绝；token 404 不可区分；密码 cookie 随 `password_rev` 失效；Range 截断；目录排除；源变化检测；幂等重放与 mismatch；修订冲突；到期 sweeper 与 unpin；`stopping` 重试；统计小时桶与按天去重；活子树 cap 的 token 在 `/s/*` 返回 404。

### M2：系统分享面板 + File Browser

- websdk `NfspClient` share 方法并重建 dist；Desktop `sysdlg/share`（含 mock）；File Browser `share` 命令接通；i18n。
- Playwright：T01（打开关闭无副作用）、T02（默认七天，改不自动到期类型不变）、打开面板到复制链接 ≤ 3 次主要操作。

### M3：“我的内容” + Public

- `my-content` App（概览、列表、详情、统计图）；`public_observations` 与 `access_urls.public`；`share.public.list`、`share.stats`。
- 验证：CLI 直接在 `data/srv/publish/` 增删文件与越界软链接，刷新后一致、越界标红（T08、T10）；mock 统计数据渲染，浅色深色截图。

### M4：MessageHub、HomeStation 目标 + CLI

- `share.app.handoff/report`；MessageHub launch `share` 字段与发送后回传；HomeStation App 启动参数 `compose`、composer 预填、`publish.create` 后回传；D14 的链接卡片路径。
- websdk CLI `buckyos share …`；cmd_skills 手册检查与更新。
- 验证：MessageHub 交接后不发 → “结果未确认”；发送后 → “应用已完成”，可点到消息；停止不撤回消息（T06）。HomeStation 发图片动态成功并在门户可见；PDF 走链接卡片。

### M5：DV 验收

- 构建 `nfs_server`、`scheduler`、`desktop`，`start.py --all` 新装（scheduler 的 host 变更要新 spec），按 §6 逐条留证据目录。部署可能需要用户执行。

## 6. 验收（对应 PRD §18）

| 编号 | 覆盖 | 说明 |
| --- | --- | --- |
| T01–T05 | 完整 | T05 依赖 `password_rev`、每请求状态判断 |
| T06 | 完整 | MessageHub 与 HomeStation 各一条 |
| T07 | 部分 | 已上线实例地址按文本/链接分享，不做权限分析 |
| T08、T10 | 完整 | |
| T09 | 以“只报告路径可访问、不报 NDN 就绪”满足 | Public 无对象化 |
| T11 | 完整 | `source_public` |
| T12–T15 | 入口路由与投影层面满足 | 安装/运行/下线由 App Service 既有验收覆盖；HTML 目录标未交付 |
| T16 | 部分 | 只回收失败快照的 pin 与任务临时数据；无全局空间统计 API |
| T17 | 完整 | GUI / CLI / Agent 都走 `share.*`，审计记真实 `actor_app_id`（D13） |
| T18 | 完整 | 列表按 owner；token 不可枚举；回传校验 owner |
| T19 | 完整 | 幂等键、重启恢复、`stale_report` |
| T35 | 完整 | `share.<zone>` 独立源，访客页不加载 SDK |
| T20–T34 | 不覆盖 | 第二、三阶段 |
| 新增 | 消费统计 | 访问、下载、流量、访客在“我的内容”可见，与访问日志一致；不可统计的渠道显示“不可用” |

## 7. 对其他文档的回写建议

- **ShareContentMgr PRD**：§3.1 “指定的内置分享提供服务”改为 nfs-server；§3.2 基线改为“nfs-server 已有 grant/cap，数据面未放行；control-panel 旧桩已删除，无存量数据”；§4 Public 列去掉“准备状态”；§13 系统入口名改为“我的内容”并加“消费情况”；§6.2 指定用户/好友首期不开放；§19.1 “NDN 数据授权”阻塞范围改为“对象直链（第二阶段）”。
- **nfs_server.md**：§2.2 “数据面尚不放行”改为“`share.*` 快照分享经 `share.<zone>/s/{token}` 放行，活子树 cap 仍不放行”；§4.1 方法表加 `share.*`；§5.1 注明“grant 时 Link → Store 提升”首期以分享快照实现，commit 路径的 link 模式仍未做。
- **NFSP v0**：§3.6 `signed` 条目的 URL 主机为 `share.<zone>`；§5.5 cap token 记一句“BuckyOS 实现以快照 obj 为目标”。

## 8. 参考入口

- nfs-server：`src/frame/nfs_server/src/server.rs:22-133, 240-275`（信封、写方法、dispatch）；`auth.rs:78-101`（身份）；`acl.rs:24-90`；`mutate.rs:833-901`（grant/revoke）；`filedb.rs:241-251, 1386-1440`（grants）；`namespace.rs:560-577`（access_urls）；`copy.rs:216-567`（TaskManager 范本）；`config.rs`（data root）
- named store：`cyfs-ndn/src/named_store/src/ndm.rs:879-1071, 1279-1310`；`ndn-toolkit/src/tools.rs:156, 375`；参考 `src/frame/opendan/src/objects.rs:21-62`
- gateway / scheduler：`src/rootfs/etc/boot_gateway.yaml:76-79, 113-126, 138-232, 269-299`；`src/kernel/scheduler/src/system_config_agent.rs:1204-1216, 1306-1324, 1660-1666`；`install_plan_executor.rs:178-186`
- MessageHub：`src/frame/msg_center/src/object_access.rs`；`src/frame/desktop/src/app/messagehub/{launch.ts, ForwardDialog.tsx}`
- HomeStation：`src/frame/homestation/src/{api.rs:151-176, publish.rs:604-686, 811-878, objects.rs:368-397, http.rs:24}`；`src/frame/desktop/src/app/homestation/publish/PublishComposer.tsx`
- Desktop：`src/frame/desktop/src/sysdlg/index.tsx`；`app/filebrowser/menu/commands.ts:14`；`app/registry.tsx`；`app/backend-apps.ts`
- websdk：`buckyos-websdk/src/nfsp.ts:702-720`

## 9. 实施记录

### 9.1 落点（2026-10-19 实施记录，已随 `835b5bd44` 提交）

- **nfs-server**（`src/frame/nfs_server/`）：`src/share/` —— `mod.rs`（ShareMgr：named store 句柄、token/密码/解锁
  cookie、统计缓冲、sweeper）、`db.rs`（§3.3 表 + 查询；shares 冗余 `source_path` 便于按源查已有链接）、
  `snapshot.rs`（规划与快照、读回）、`rpc.rs`（`share.*`）、`serving.rs` + `page.html`（数据面与访客页、host guard）、
  `public.rs`（Public 观察）。改动：`acl.rs`（`Actor::Delegated`）、`auth.rs`（App/Agent token → Delegated，Agent 经
  `users/*/agents/<id>/spec` 找 owner）、`server.rs`（`/s/*` 路由、`share.*` 分派与写方法 seq、Delegated 只许 `share.*`）、
  `namespace.rs`（`access_urls` 增 `public`/`signed`）、`config.rs`（`NamedStoreSource`、`ShareConfig`）、`main.rs`
  （`--named-store/--share-base-url/--public-base-url`；服务模式取 zone host 配 `https://share.<zone>`、`https://public.<zone>`，
  启动 sweeper 与操作恢复）、`types.rs`（hello 广告 `share`）、`filedb.rs`（执行 share DDL）。新依赖：`ndn-lib`、
  `named_store`（D15）、`pbkdf2`（D10）、`hmac`（解锁 cookie；两者原已在 Cargo.lock）、`name-lib`（workspace）。
- **buckyos-api**：`nfs_share.rs`（`nfs.share_snapshot/v1`，加入内置 schema）；删除 `content_mgr_client.rs`。
  control-panel 删除 `share_content_mgr.rs`。
- **scheduler**：`system_config_agent.rs`（nfs-server 有选择器时 `APP_INFO["share"]`，保留名加 `share`）、
  `install_plan_executor.rs`（两处保留名）。**gateway**：`rootfs/etc/boot_gateway.yaml` 覆盖写 `X-Forwarded-Host`。
- **websdk**：`src/nfsp.ts` share 类型与 `share*` 方法；CLI `cli/modules/share.ts`（`buckyos share create|list|get|update|stop|stats`，
  直接走 NFSP 信封）+ `cli/tests/share_test.ts`；dist 已重建。
- **Desktop**：`api/shares.ts`（服务接缝，NFSP / mock，`?shareData=`、`VITE_MY_CONTENT_USE_MOCK`）、`api/shares_mock.ts`；
  `sysdlg/ShareDialog.tsx`、`shareFormat.ts`、`shareTargets.ts`，`sysdlg/index.tsx` 注册 `sysdlg/share`；`app/my-content/`
  （概览、列表、详情、Public、图表，已注册三处 + 图标）；File Browser `share` 命令接通、“复制公开地址”改用
  `access_urls.public`；MessageHub `launch.share` + `ShareIntake.tsx`（复用转发选人）；HomeStation `launch.compose` +
  composer 预填与回传；`i18n/shares.ts`（en/zh）；e2e `share.spec.ts`（9 例）、`share.nfsp.spec.ts`（真实后端，opt-in）。
- **cmd_skills**：新建 `src/apps/jarvis_agent/agent/cmd_skills/share.md`。依据：分享是给人交付内容的用户动作，不属于
  `buckyos_admin`（Zone 管理）范畴，且该手册已到 140 行上限；新手册自带 front matter，`agent-session skill list` 自动发现。
- **文档**：`product/BuckyOS Sys Dlg.md` 写 `sysdlg/share`（`product/control_panel/share_content.md` 并入后删除，Albums PRD
  与 text_editor readme 的链接已改）；`product/bucky_file/nfs_server.md` §2.2/§4.1/§5.1/§11 回写并新增 §12；nfs_server README；
  `doc/arch/beta2.2-data-compatibility.md`、`doc/control_panel/Control_Panel_Service.md` 去掉旧桩描述。

### 9.2 与 §3 不同的决定

1. **token 可再现**：`token = base64url(HMAC(secret, "share-token", cap_id))`，secret 存 filedb kv `share_token_secret`，
   `grants` 仍只存 sha256。否则 owner 无法再次复制已建链接（`share.get.url`、面板“已分享的链接”）。
2. **快照不用 `ndn_toolkit::cacl_file_object`**：它在 Store 模式下每个 chunk 都把整个文件读进内存（`read_chunk_bytes`），
   大文件 O(n²) 读放大。改为自写：按 32 MiB 段读一次、同一缓冲哈希并 `put_chunk`，≤ 32 MiB 单 chunk、更大用 chunk list；
   目录自己遍历（逐项 ACL、默认排除、不跟随符号链接，DirObject 的 put 方式照 `cacl_dir_object`）。因此未引入 `ndn-toolkit`。
3. **MessageHub 附件上限 32 MiB**：msg-center 对象路由只读单 chunk 文件；文件夹或超过 32 MiB 时 MessageHub 也走 D14 的
   不过期链接（`mode=link`，消息正文为标题 + 链接）。
4. **任务镜像是尽力而为**：`share_operations` 是业务记录；有 bearer 且 TaskManager 可达时才建 `nfs.share_snapshot/v1`，
   并照 `nfs.copy/v1` 用 `StorageDomain::System`（§3.7 写的是 User）。没有 TaskManager 时后台任务照常执行。
   新增 `share.operation {cancel:true}` 供面板取消。
5. **回传 canceled/failed 时释放**：投递分享（及其关联链接分享）置 stopped 并 unpin，避免每次关掉 composer 都留下 pin。
6. **host guard**：后端按 `X-Forwarded-Host`（网关覆盖写）/`Host` 判断，`share.<zone>` 上只放行 `/s/*`。核对发现网关 tcp
   转发会复制原请求头，但 HTTP/2 请求只有 `:authority`，所以加了 `X-Forwarded-Host`。`/s/*` 不判 host（只有 share host 会路由到它）。
7. **Public 扫描**放在 sweeper（每 5 分钟）与 `share.public.list {refresh}`，不挂 reconciler（其循环可被关闭）。
8. **Delegated 身份**：`Actor::Delegated{user, app, admin}`，路径 ACL 对它恒为 none，`share.*` 内换成用户的文件身份；
   Agent token 的 owner 通过扫描 `users/*/agents/<agent_id>/spec` 确定并缓存。独立模式未鉴权时调用者记为用户 `local`。
9. **其余**：`share.plan` 增 `site.has_index_html`（上线入口）；`share.list.source` 也接受路径字符串；访客页由服务端注入
   标题与 boot JSON、内容由页面取 `meta` 渲染；PDF 响应不带 `sandbox`（Chrome 拒绝在沙箱文档显示 PDF），其余仍 `CSP: sandbox`；
   FileObject meta 用 `mime`（HomeStation 约定，msg-center 读 `mime_type` 时按扩展名兜底）；设置密码即切到密码访问、
   删除密码即回到持链接者；“我的内容”不加默认桌面快捷方式（全部应用 + 面板入口）。
10. **DV 后的修正**（§9.7）：System/Device 调用者（节点上的 CLI）不带 `on_behalf_of` 时代表 Zone owner；
    下载只计附件响应（页面里的图片/文本预览不算）；统计的“即将到期”窗口为 3 天（`summary.expiring_soon`/
    `expiring_within`，原 7 天窗口会把每条默认 7 天链接都算进去）；日桶是 UTC 日，图表按 UTC 标注；访客解锁的
    PBKDF2 校验放到 `spawn_blocking`。

### 9.3 验证（均在本机执行）

| 命令 | 结果 |
| --- | --- |
| `cargo test -p nfs_server`（`src/`，`CARGO_INCREMENTAL=0`） | 52 单元 + 8 acl + 19 integration + 10 share 全过 |
| `cargo test -p scheduler` | 67 + 21 全过 |
| `cargo test -p buckyos-api` | 239/240；`runtime::tests::app_service_uses_node_gateway_for_verify_hub_before_zone_bootstrap` 并行时因环境变量竞争失败，单跑通过，与本改动无关 |
| `cargo test -p task_manager builtin` | 内置 schema 播种通过 |
| `cargo test -p libopendan --test xagent_cli skills_are`、`--test xagent_lib work_sessions_take` | 通过 |
| websdk `pnpm run check:cli`、`pnpm run test:cli`、`pnpm run build` | 通过（109 例，含 share 3 例） |
| Desktop `tsc -p tsconfig.app.json`、`pnpm lint` | 0 错误，8 个既有 warning（均不在本次文件） |
| Desktop `playwright test tests/e2e/pages/share.spec.ts` | 9 例全过（T01、T02、≤3 次操作、应用目标判定、我的内容改密/停止、T06 HomeStation/MessageHub 三例、浅深色截图） |
| Desktop 相关既有 spec（filebrowser×3、messagehub×2、homestation、desktop、desktop-folder） | 105 过；3 例失败均为既有基线（SN Router、FB-02 NFSP batches、codeassistant wheel scrolling） |
| 真实后端：独立 `nfs_server --named-store … --share-base-url …` + `VITE_NFS_PROXY` + `SHARE_NFSP_E2E=1 playwright test share.nfsp.spec.ts` | 通过：Files 建密码链接 → 访客页解锁看到快照内容 → 我的内容统计出现访问 → 停止后链接 404；另用 curl 与探针截图核对目录访客页（浅/深/手机宽度） |
| `uv run buckyos-build.py -s nfs_server scheduler task_manager desktop jarvis_agent` | exit 0（musl release + desktop build + SDK/CLI 分发包） |

`tests/share.rs` 覆盖：无读权限拒绝、未知/停止/到期 token 同为 404、密码 cookie 随改密失效与解锁限速、Range（含跨 chunk 边界）、
目录排除与路径穿越、源变化（plan_stale）、幂等重放与 mismatch、修订冲突、到期 sweeper 与 unpin、统计小时桶与按天去重访客、
活子树 cap 在 `/s/*` 返回 404、App 身份只能用 `share.*` 并记审计、HomeStation 附件/链接两种交接与回传新旧判断、
MessageHub 文件/文件夹、HTML 只下载、Public 观察与越界软链接、`access_urls.public/signed`、share host 上 `/nfs/v1` 404。

### 9.4 未验证 / 已知风险

- **Agent 身份未在 DV 实测**：Agent 容器内 docker exec 拿不到 Agent token（只有 session shell 有），App/Agent token →
  `Delegated` 只有 `tests/share.rs` 的 App token 用例覆盖；Agent owner 查找（扫描 `users/*/agents/<id>/spec`）未在真实 Zone 跑过。
- `stopping` 状态重试 unpin 只有 sweeper 代码路径，没有单测；多 OOD 时快照所在节点与 msg-center/HomeStation 不同会读不到（§3.10 已列）。
- 停止应用投递会释放 nfs-server 的 pin：MessageHub 消息、HomeStation 动态引用的 chunk 之后成为 GC 候选，除非这些服务
  自己 pin（当时没有自动 GC 循环，DV 上停止后 alice 仍能读到附件）。2026-10-20 已决定由接收应用独立保留，见 D17；实现与 GC 验证仍待 §10 R05 完成。
- 已知风险照 §3.15：读取权限即可外发；Agent 可外发 owner 能读的文件。

### 9.5 未做

- §7 对 ShareContentMgr PRD 与 NFSP v0（cyfs-ndn 仓库）的回写尚未完成；2026-10-20 已获用户确认，纳入 §10 R01–R02。
- 文件夹打包下载（zip）、HomeStation 互动数拉取、收入列（数据结构未预留展示）、T07/T12–T15 超出入口路由的部分。上述继续延期，另见 D16；T16 的全局空间统计也不在本轮范围。

### 9.6 DV 部署（2026-10-19 历史步骤，已由 §9.7 完成）

产物已在 `src/rootfs/`。scheduler 改动只影响派生的 gateway_info，不需要 `--all`：

```bash
cd buckyos/src
uv run start.py            # 停止、更新安装文件（含 bin/nfs-server、scheduler、task-manager、control-panel/web 与 etc/boot_gateway.yaml）、启动
uv run check.py
```

之后验收：`https://share.<zone>/s/<token>` 可打开，`https://<zone>/nfs/v1/hello` 正常而 `https://share.<zone>/nfs/v1/hello` 为 404；
Desktop Files 右键“分享…”建链接，“我的内容”看到访问统计；容器内 Agent 执行 `buckyos share create /home/<owner>/…`。

### 9.7 DV 验收（2026-10-19，test.buckyos.io，用户部署后）

环境：用户 `uv run start.py` 部署后，nfs-server 二进制与构建产物一致，`/opt/buckyos/etc/boot_gateway.yaml` 含
`X-Forwarded-Host`，scheduler 派生的 `node_gateway_info.json` 有 `app_info.share = {service_id: nfs-server, selector: ood1:4110}`。
主体用普通用户 bob（dev 配置用户），收件方 alice。证据：`/home/lzc/project/share_dv_runs/20261019/`（截图 + 探针脚本）。

| 项 | 结果 |
| --- | --- |
| 网关与主机 | `https://test.buckyos.io/nfs/v1/hello` 200（广告 `share`），`https://share.test.buckyos.io/nfs/v1/hello` 404；share 主机走 `*.test.buckyos.io` 通配证书（DV 测试 CA，与主域同一信任） |
| 链接（T01–T05） | Files → 分享… → 密码链接，URL 为 `https://share.test.buckyos.io/s/<43 字符>`；访客页 CSP 生效；无 cookie `/d` 401；错误密码提示；解锁后看到快照内容并可下载；改到期为“不自动到期”、停止后页面与 `/d` 均 404；审计 create/update/stop |
| 目录 | 排除 `.env`、`node_modules/`；`meta` 列出 `sub`、`a.txt`；`/d/sub/b.txt` 可读，`/d/.env` 404 |
| 大文件后台 | 70 MiB：`share.create` 返回 running，进度卡出现，约 1 s 完成；TaskManager 有 `nfs.share_snapshot/v1`（creator bob，Succeeded）；跨 32 MiB chunk 边界的 Range 字节正确 |
| 到期 | 20 s 到期链接：到期后每请求即 404，sweeper 置 `expired` 并释放 pin，审计 `expire/system` |
| 权限 | bob 分享 `/home/devtest` → `PERMISSION_DENIED source_not_readable`；随机 token 404 |
| HomeStation（T06） | 照片交接 → composer 预填附件 → 发布成功（entry `cyfs://test.buckyos.io/home/bob/feed/@/p-…`）→ 回传 confirmed；HomeStation `/home/bob/objects/<obj>/content` 匿名 200，字节与源文件一致（HomeStation `load_local` 从 named store 取到快照） |
| MessageHub（T06） | PDF 交接 → 转发选人 → 发给 alice（`msg.post_send` delivered）→ 回传 confirmed；alice 用 Bearer 读 `/kapi/msg-center/objects/<obj>/content` 200，内容正确；匿名 401 |
| 我的内容 | 概览统计、列表、两条投递“应用已完成”、改到期、停止确认文案、Public 页列出 `readme` 且 `https://public.test.buckyos.io/readme` 200；`access_urls` 对 `srv/publish` 文件有 `public`，对自己的有效链接有 `signed` |
| CLI | 用户会话（容器同款注入方式）：create/list/get/update/stop/stats 通过，审计 `source: cli`；设备身份见下 |
| 重启 | 重启 nfs-server 后，重启前建的链接仍可访问（token 密钥持久） |
| 回归 | `test/nfs_server_test` 的旧登录辅助在本机缺 AppClient 私钥，未运行；文件平面由 Files UI 浏览、上传（spec 用 NFSP 上传夹具）、删除覆盖 |

DV 发现并已修复（修复后用备份方式替换 `/opt/buckyos/bin/nfs-server/nfs_server` 与 `bin/control-panel/web`，备份
`*.bak-sharedv-20261019`，并复验通过）：

1. 节点上的 `buckyos --yes share …`（设备身份）被拒：“system callers must pass on_behalf_of”。改为代表 Zone owner；复验以
   devtest 身份建/停分享成功。新增 `tests/share.rs` 设备 token 用例。
2. 访客页的图片/文本预览被算作下载。改为只计附件响应；复验：打开页面 + 预览 + 点下载 = views 1 / downloads 1 / visitors 1。
3. “7 天内到期”把每条默认 7 天链接都算进去。改为 3 天窗口，字段 `expiring_soon` + `expiring_within`（websdk 类型、Desktop、mock 同步）。
4. 概览图日桶为 UTC 日，却按本地时区标注，美西显示成前一天。改为 UTC 标注。
另：`tests/share.rs` 的解锁限速用例在 debug 构建下 12 次串行 PBKDF2 跨分钟而不稳定，改为并发；访客解锁的 PBKDF2 放到 `spawn_blocking`。

新增可重跑的 DV 用例 `src/frame/desktop/tests/dv/share.spec.ts`（4 例，自建并清理夹具目录）：
`BUCKYOS_TEST_ZONE_HOST=test.buckyos.io npx playwright test --config=playwright.dv.config.ts tests/dv/share.spec.ts`，2026-10-19 4/4 通过。

清理：探针与用例建的分享均已停止（filedb 中 0 条 active、0 个未释放 pin），手工夹具 `/opt/buckyos/data/home/bob/share-dv` 已删除；
HomeStation 上 bob 的测试动态（3 条）与发给 alice 的测试消息保留。

## 10. 后续收尾 TODO（2026-10-20 已批准，尚未实施）

以下 7 项是后续执行清单；M0–M5 的完成记录不替代本节验收。每项完成后勾选并追加实际验证命令、结果与证据位置，不把历史 DV 结果算作新实现的验证。

- [ ] **R01：ShareContentMgr PRD 回写。** 按 §7 更新服务归属、已交付基线、Public、管理中心和阶段边界，并同步 D17 的应用附件保留语义；删除与当前目标矛盾的描述。
- [ ] **R02：NFSP v0 回写。** 在 `cyfs-ndn/doc/NamedFileSystem_Protocol_v0.md` 更新 signed URL 主机、快照目标与 BuckyOS 实现范围；NDM 协议变化同步对应 SDK/网关文档，不把 NFSP 路径授权等同于原始对象全库权限。
- [ ] **R03：真实 Agent 身份 DV。** 在真实 Zone 的 Agent 会话中调用 `buckyos share`，验证 Agent owner 解析、源路径权限、审计主体及有效权限限制；用户会话、设备身份、伪造或测试 token 不能替代。凭据按现有 runtime 注入方式获取，证据不包含 token。
- [ ] **R04：unpin 失败重试。** 注入存储释放失败，确认分享进入 `stopping` 后访客访问立即失效；恢复存储后 sweeper 可重试至释放完成，重复执行/重启不误释放其他引用。覆盖失败路径，不能以正常到期测试替代。
- [ ] **R05：应用附件独立保留。** 按 D17 实现两应用的接收、引用存续、释放、失败恢复及当前有效记录的保留补齐。隔离测试存储中实际执行 GC：停止分享并释放其 pin 后，已发送消息/已发布动态的附件及依赖仍可读；移除最后有效业务引用后可回收；共享引用不受误删；链接模式按链接状态失效。
- [ ] **R06：旧回归套件。** 修复或适配 `test/nfs_server_test` 的登录辅助并运行真实套件；记录命令和结果，不以 Files UI 覆盖替代。适配目标版本接口即可，不为旧登录方式增加兼容分支。
- [ ] **R07：NDM 独立迁移。** 按 D18–D19 完成宿主、路由、授权、SDK 和消费方联动；删除 control-panel 对应实现。验证用户上传/续传/对象登记、失败取消与过期清理、跨用户会话拒绝、匿名拒绝、无授权对象读取拒绝、受授权 AppClient/AppService 操作与管理权限隔离；消息附件上传、应用包上传/安装、相关 SDK/proxy 与系统 App 测试通过。

交付时同时满足：

- 受影响服务及 SDK/Desktop 的相关测试与构建通过；真实 Zone 中验证迁移后的网关路由、MessageHub/HomeStation 交接、Agent 分享和应用包上传/安装。
- R05 的 GC 验证使用隔离数据，不能用“未执行 GC 时仍可读取”证明保留正确；无须在本轮上线自动 GC 调度。
- 接口变化同步共享类型、前后端、SDK/CLI、生成来源与分发产物。按仓库规则检查 `share`、`buckyos_admin`、`buckyos_dev_release`、`buckyos_app_dev` 及实际受影响的 cmd_skills，修改或写明无需修改的依据。
- 本节记录实际完成状态、证据和未验证项；没有验证的场景不能标记通过。后续阶段功能与已接受限制继续明确列出，不扩写为本轮已交付能力。

