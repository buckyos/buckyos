# nfs_server

NFSP v0 的服务端实现(v1,无 fs_meta 阶段)。以服务器本地文件系统为实体树真相源、
以 filedb(SQLite)为虚拟 Node / Binding 真相源,向 WebUI(File Browser)提供统一的
Node / Entry / Container 协议服务。

- 需求文档:`buckyos/product/bucky_file/nfs_server.md`
- 协议:`cyfs-ndn/doc/NamedFileSystem_Protocol_v0.md`(NFSP v0)

## 启动形态:BuckyOS 服务 + 独立启动

### BuckyOS 系统服务模式(无参数启动,node_daemon 拉起)

不带 `--data-dir` / `--export` 启动时进入 buckyos 模式:通过 buckyos-api 以
KernelService 身份 login(含心跳),读取 `services/nfs-server/settings`
(`scan_interval_secs` / `debug_api`,缺省有默认值),然后:

- `cyfs:///` 挂载 `$BUCKYOS_ROOT/data/`:每个可见一级子目录(home/、srv/ 等)
  成为一个 export root;`.` 开头目录不导出。导出列表为启动时快照。
- filedb + 上传暂存放在 `$BUCKYOS_ROOT/data/.fsdb/`。
- 监听 `127.0.0.1:4110`(`NFS_SERVER_SERVICE_PORT`)。NFSP 是跨 zone 通用协议,
  网关把 zone 级根路径 `/nfs/v1/*` 原样转发到本服务(boot_gateway.yaml,
  与 /ndn 同级),不使用 `/kapi/<service>` 形态。
- 每个请求验证调用者的 Verify Hub session token（Bearer 或 SSO cookie），再按路径 ACL 授权；
  Zone owner 全权限，用户管理自己的 `home/<user_id>`，其余靠 ACL 记录授予，匿名只到 `public`。
  详见下文“鉴权与权限”及产品文档 §11。
- App/Agent token 仅能访问 `share.*`，不能直接访问文件平面。服务模式下，绑定 Agent 的 runtime App
  和独立 AgentDID 调用还检查 `AgentSpec.permission_mode`；`read_only` 拒绝
  `share.create/update/stop/hide/app.handoff/app.report`，返回 `PERMISSION_DENIED` 与
  `reason: agent_read_only`。读取分享、普通 App 和真实用户会话继续遵循原有授权。

集成点:`buckyos-api/src/nfs_server_client.rs`(常量 + Settings)、
`scheduler/system_config_builder.rs::add_nfs_server()`、
`rbac_config.rs`(`g, system:nfs-server, frame`)、`bucky_project.yaml`、
`rootfs/bin/nfs-server/`。DV smoke test:`test/nfs_server_test/`(走网关);
协议级回归:`test/test_nfs_server/run.sh`(独立启动,46 例)。

### 独立启动模式(保留,便于独立测试)

给出 `--data-dir` + `--export` 即为独立模式,不依赖任何 buckyos 运行时:

```bash
nfs_server --listen 127.0.0.1:3260 \
           --data-dir /var/lib/nfs_server \
           --export home=/srv/home --export media=/srv/media \
           --scan-interval-secs 30 \
           [--debug-api] [--auth-test-config auth.json] [--log-level info]
```

- `--export name=/abs/path`(可重复):每个 export root 成为当前 Zone 的 `cyfs:///` 命名空间一级子目录。
- `--data-dir`:存放 `filedb.sqlite`、`copy.sqlite`、`copy-locks/` 与上传暂存区。复制日志及目标目录中的复制临时文件保留用于恢复；上传暂存区每次启动清空。
- `--scan-interval-secs`:Reconciler 扫描周期,0 = 关闭。
- `--debug-api`:开启 `POST /nfs/v1/debug/{reconcile|create_view}`(仅测试/开发)。
- 不鉴权:独立模式默认所有请求全权限。`--auth-test-config`(须 loopback + `--debug-api`)给出
  `{"public_key": "<ed25519 x>", "zone_owner": "<user_id>"}` 后,按该测试密钥验证 token,
  路径 ACL 规则与服务模式相同；AgentSpec 权限查询仅在服务模式生效。

```bash
cargo test -p nfs_server        # 单元与集成测试，不需要外部服务
```

## 协议覆盖(nfs_server.md §4.1 对照)

| 组 | 方法 | 状态 |
|---|---|---|
| 会话 | `hello` / `bye` | ✅ feature 协商、limits、realms;session 由 hello 返回 |
| 解析 | `resolve` / `stat` / `list` / `batch` | ✅ 统一信封;list 接受 Dir/View/Collection/Group Ref;无状态游标(D10 不重置,返回 `revision_changed`);batch 共享游标 walk/stat/list |
| 写入 | `mkdir` / `move` / `delete` / `open_write` / `commit_file` / `abort_write` | ✅ revision CAS + 内存租约 + 旁路写 commit 复检(size/mtime) |
| 本地复制 | `copy_capabilities` / `copy_submit` / `copy_list` / `copy_get` / `copy_decide` / `copy_cancel` | ✅ 可信用户、TaskMgr 任务、逐项日志、恢复/取消/失败项重试；详见产品文档 §10 |
| 引用绑定 | `bind_ref` / `unlink` | ✅ sidecar merge、同名冲突 `conflicts[]`、unlink 只解引用 |
| 上传 | `probe` + tus 续传 + `commit_file` | ✅ 内嵌最小 tus(N1 采纳内嵌方案);probe/秒传基于 filedb `content_index` 缓存表 |
| 元数据 | `get_meta` / `set_meta`(user ns) | ✅ 锚定 `live:n_<id>` + `obj:sha256:*`;写 meta 触发惰性锚定 |
| 视图 | `open_view` + 通用 `list(view_ref)` | ✅ 只读;base/patch overlay 合并已实现;内容经 debug API / filedb 灌入(v1 无 AI 生成器) |
| 集合 | `create_collection` / `open_collection` / `collection_patch` + 通用 `list` | ✅ add_ref/remove_entry/move_entries/create_group/rename_group;成员只存 Ref,`canonical_path` 读取时批量补全 |
| 搜索 | `search` | ✅ name mode;响应结构完整(`match_source`/`explain`/`sources[]`);先序遍历 + 路径游标 |
| 权限 | `get_acl` / `set_acl` / `list_acls` | ✅ 路径 ACL(实现扩展);继承/关闭继承、内置 owner 规则、owner 可列出全部记录 |
| 分享(活子树) | `grant` / `revoke` | ⚠️ 需 `manage`;签发/撤销/落库完整,**数据面不凭活子树 cap 放行** |
| 分享(快照) | `share.*`(实现扩展) | ✅ 快照进 named store,分享有效期间绑定在 zone 桶 `zone/nfs-server/_/shares`(key = share_id,停止/到期即解绑);`/s/{token}` 访客页/下载/密码/到期;审计、访问统计、Public 投影、MessageHub/HomeStation 交接;详见产品文档 §12 |
| 派生 | `repr` / `get_repr`(实现扩展) | ✅ Preview Pipeline:ffmpeg/ffprobe 有则用、没有就 `unsupported`;产物进 zone 桶 `zone/nfs-server/_/preview-cache`;见下文「预览管线」 |
| 通知 | `watch`(SSE) | ✅ `container_changed`/`meta_changed`/`resync`;连接首事件恒为 `resync`(D11);watch_token 过滤 |
| 数据面 | `GET /nfs/v1/read/{node_id}` | ✅ Range/ETag(弱)/If-None-Match/Content-Disposition;**不发 immutable**(link 语义,§4.2) |

`hello` 广告的 features:`view` `collection` `reference-binding` `watch.sse` `search.name` `acl` `share`,
找到 ffmpeg/ffprobe 时再加 `repr`;并返回 `identity {user_id, role}` 与 `repr`(转换器能力,见「预览管线」)。
不广告(未实现):`frozen-subtree` `search.semantic` `get_tree` `publish_dir`。

## filedb(§3.2 最小持久集)

持久表:`entities` `anchors` `namespace_bindings` `views` `view_groups` `view_base`
`view_patch` `collections` `collection_nodes` `meta_records` `grants` `acls` + `kv`(handle 签名密钥、分享 token 密钥),
分享模块的 `share_*` 与 `public_observations`(`src/share/db.rs`)。
缓存表(可全量重建):`content_index`(hash → 本地路径,支撑 probe/秒传)。

惰性锚定:只有进入 View/Collection、被写 meta、被分享、或作为 binding parent/target 时
才建 `entities`/`anchors` 行;**纯浏览零 DB 写入**(未锚定节点返回带 HMAC 签名的
opaque handle,重启后仍可验证——密钥持久化在 filedb)。

## Reconciler(v1 唯一的一次性组件,`reconciler.rs` 隔离)

v1 用**扫描循环 + 访问时校验**替代平台 watcher(inotify/USN/FSEvents 留待 N6 决策):

- 访问时:同路径新原生 ID → 覆盖保存启发式重绑;路径消失 → stale。
- 扫描时:ino 唯一命中新路径 → rename 跟随(目录 move 级联改写子孙锚点);
  同路径新 ino → 重绑;歧义(硬链接)或消失 → **stale,不猜**。
- 目录指纹 diff → 旁路变更发 `container_changed`;单轮变更 > 50 个目录 → 只发 `resync`。

## 降级契约(照 §4.2,须写进客户端文档)

- revision 是 **opaque 相等性令牌**:Dir revision 编码进程 epoch,重启即换;客户端只做相等比较。
- 重放窗口(`seq`)在内存,重启后按协议走 `SEQ_OUT_OF_WINDOW`/重新 resolve。
- 租约对协议客户端是真互斥;对服务器本机进程是**劝告性**——旁路编辑在 commit 时显式
  `TARGET_MISMATCH{reason:"bypass_modified"}`,绝不静默覆盖。
- watch 有损:断线/重连/缓冲滚动一律 `resync`,不补发。
- `NEED_PULL` 服务端会产生于 `commit_file{hash}` 未命中本地内容时(客户端须上传)。
- symlink:v1 在解析/列举中作为独立 `kind:symlink` 节点返回(不跟随),数据面读取时跟随。
  这是对 D2(默认跟随)的显式偏离,换取 handle 语义精确,阶段二随 fs_meta 统一。

## 错误码

完整实现 NFSP §8 错误码表。实现扩展(§8 之外,新名字 = 合法 schema 扩展):

| code | HTTP | 用途 |
|---|---|---|
| `INVALID_ARGUMENT` | 400 | 报文/参数不合法(含 delete 用于 reference entry) |
| `NOT_EMPTY` | 409 | 非递归 delete 非空目录 |
| `UNSUPPORTED` | 400 | 未知方法 / v1 未实现能力(如 object ref 解析) |
| `INTERNAL` | 500 | 服务器内部错误 |

## 已知边界(接受并记录)

- 上传 PATCH 分片整体读入内存后落盘,客户端应按 4–16MB 分片。
- `move` 不支持跨设备(EXDEV 报错);export root 通常同盘。
- 非 UTF-8 文件名不进协议(列举时跳过)。
- 非 unix 平台原生 ID 降级为 0(handle 校验放宽);Windows 支持随 N6 决策。
- probe 只走 (size,mtime) 快速通道,mtime 变化即诚实报 missing;
  `commit_file{hash}` 侧会做完整 hash 复验兜底。

## 后续接入(不在 v1)

1. 活子树 cap 校验(grant 数据面放行)与 App/Agent 主体的文件平面访问。按请求鉴权、路径 ACL 与快照分享已完成。
2. NamedStore link 模式(`add_chunk_by_link_to_local_file` / qcid 阶梯)替换 `content_index`。
3. `view_patch`、`get_tree`、`publish_dir`/Frozen、referral 设备视图;`repr` 的 PDF/Office 等非音视频管线。
4. NativeTree trait 冻结(M7):第二个实现(fs_meta)落地时从 `namespace.rs` 提取。

## 文件复制真实 E2E

从仓库 `src/` 执行（已有 Rust/Node/pnpm/Chromium 环境）：

```bash
cargo build -p nfs_server
cargo build -p task_manager --example nfs_copy_fixture
FB_NFSP_E2E=1 FB_COPY_AUTO=1 VITE_NFS_PROXY=http://127.0.0.1:3262 \
  pnpm --dir frame/desktop exec playwright test \
  tests/e2e/pages/filebrowser.copy.nfsp.spec.ts --workers=1 --reporter=list
```

`CARGO_TARGET_DIR` 如果有自定义，构建与 E2E 必须使用同一环境值。fixture 自动启动
3262 端口 NFSP、3382 端口真实 TaskManagerService 及其 SQLite RDB 用户/系统分区，
使用独立临时导出目录。root 下以 uid/gid 65534 运行服务，测试真实文件系统权限拒绝。
固定测试密钥仅供该 fixture；NFSP 的 `--copy-test-config FILE` 只接受独立模式、
loopback 监听且显式开启 `--debug-api`。测试仍执行 JWT 签名及标准用户 claims 校验。
`NFS_COPY_TEST_SLOW` / `NFS_COPY_TEST_FAULT=after-commit` 仅在该测试配置下生效。
服务结束后清理本次目录，不接触开发 Zone 的运行服务。

部署时同时更新 Desktop、nfs-server 和 task-manager 的内置 `nfs.copy/v1` schema。
生产模式通过现有 runtime 验证 Verify Hub 用户 token，并用 nfs-server 服务身份代理
该用户创建/控制 Task；hello 的随机 session 不是 Task creator。详细 Input、恢复状态、
文件元数据和平台边界见 [产品协议 §10](../../../product/bucky_file/nfs_server.md#10-本地文件复制扩展fb-04-copy)。

## 鉴权与权限

- `auth.rs`:token 来源 Bearer > `buckyos_session_token` cookie > `<app_id>_token` cookie;无凭证为匿名。
  User 主体按 `system/zone_owner_user_id` 区分 owner;System/Device 主体视同 owner。hello 会话绑定身份,
  并按需创建该用户的 `home/<user_id>`(建账号时没有服务创建它)。
- `acl.rs`:纯路径判定(不访问文件系统,无权与不存在不可区分)。级别 `traverse < read < write < manage`;
  owner 与 `home/<user>` 主人为内置 `manage`;记录 `{inherit, entries[{principal, level}]}` 向下继承,
  principal 为 `user:<id>` / `zone_users` / `public`(只读)。记录随 move 迁移、随 delete 删除。
- `access.rs`:协议边界检查(路径 locator 先判权限再 lstat;非 owner 不经过符号链接目录,读文件链接按
  真实路径授权)、引用目标交集、watch 事件过滤、ACL 方法。新建/移入路径前清理孤儿记录;复制每次运行重验。
- 服务模式首次启动写入 `srv/library`(zone_users 可写)、`srv/publish`(公开只读)两条默认记录。
- 回归:`cargo test -p nfs_server --test acl`(测试密钥签发 token,覆盖可见性、继承、公开、cookie、
  会话绑定、probe/dedup 侧信道、collection、copy、watch、符号链接、单文件写、孤儿记录)。Desktop 真实后端用例见
  `tests/e2e/pages/filebrowser.permissions.nfsp.spec.ts`(`FB_ACL_E2E=1`,文件头有命令)。

## NDM 网关

`/ndm/v1/*`（浏览器/App 上传协议）与 `/ndm/proxy/v1/*`（受信进程代理）由本服务承接（`src/ndm/`），
每个请求都要会话 token，并按 RBAC 资源 `obj://ndm/{uploads,objects,retain,manage}` 授权；tus 上传会话绑定主体、
持久化可续传、支持终止与过期清理；查重、store API 与 proxy 授权后交给 cyfs-ndn 的现成处理器。说明见
`product/bucky_file/nfs_server.md` §13。回归：`cargo test -p nfs_server --test ndm`（真实 RBAC 模型 + 默认策略，
覆盖归属、续传、终止、过期、登记校验、未授权读取与权限分离）。

浏览器使用 WebSDK `ndm.importFiles` / `ndm.importDirectory`，再等待 `ndm.ensureUploaded`；稳定 ObjId 与 `objectStatus: ready` 分开。`POST /ndm/v1/import/finalize` 接收 `{ objects: [{ obj_id, obj_data }], roots: [ObjId], qcid_links?: [{ qcid, content_id, size }] }`，在原上传用户和 canonical App/System target 下按依赖顺序登记 FileObject、ChunkList、DirObject，校验 ID、子对象读取权限及递归完整性。请求体上限 8 MiB、对象和根各最多 10000 个；根只能是 FileObject 或 DirObject。

成功返回 `{ ready: true, roots: [{ obj_id, ready: true }], warnings: [] }`。缺失/无权访问依赖、登记或临时保留失败均不返回 ready，重试相同 manifest 不重复创建内容。QCID 是可选优化：服务端从已完成内容采样验证，失败只写 warnings；索引按上传主体隔离，并保存原 chunk/ChunkList 标识，避免复用改变 FileObject ID。lookup 的 `scope` 为 `app` 或 `global`；只有 `reusable: true` 才允许跳过上传，存在性查询本身不授予读取或登记权限。

就绪根及依赖元数据使用 24 小时、递归展开的临时 pin，覆盖复用内容的强依赖；该保留不能替代消息、发布、异步请求等业务引用。零字节 TUS 在创建时直接完成真实空 chunk 写入，无需 PATCH。上传取消只清理所属 session 的 staging/pin，不删除共享对象。

manifest 可带 `auto_bind: { bucket_id, key, content_type?, metadata?, tags?, expires_at?, origin?, if_none_match?, if_match? }`（只能有一个根）：登记完成后在同一请求里把根绑定进应用对象桶，响应多一个 `binding: { bucket_id, key, obj_id, version, etag, created }`；绑定失败按桶的错误码返回（如 412 `precondition_failed`、507 `quota_exceeded`），24 小时临时保留照旧。

## 应用对象桶

模型与规则在 cyfs-ndn `bucket_store`（设计：`doc/arch/应用对象桶 Bucket 设计.md`），本服务是它的宿主（`src/bucket/`）：

- 索引：`bucket-index` RDB 实例（scheduler 在 `services/nfs-server/spec` 登记，sqlite，`$BUCKYOS_ROOT/data/nfs-server/bucket-index.db`）；独立模式在 `--data-dir/bucket-index.db`。
- 桶只来自声明：`users/<user>/apps/<appid>/spec` 或 `services/<appid>/spec` 的 `spec_config.buckets` 与 `storage_quota_mib`，访问时懒建、每分钟同步声明值，不再声明的桶标记 `orphaned_at` 保留。独立模式用 `--bucket-specs <json>`（`{appid: {buckets, storage_quota_mib}}`）。
- kRPC `bucket.*`：`POST /kapi/nfs-server`（方法表见设计 §12.1，另有 `bucket.set_versioning`）；`params.access_token` 可带 `bkt_` 能力令牌。
- 数据面：`GET | HEAD | PUT | DELETE /bucket/v1/<scope>/<appid>/<owner>/<name>/<key>`（网关 `/bucket/*` 转发到本服务），会话来自 Bearer 或 SSO cookie，能力令牌来自 `?token=` 或 `Bearer bkt_…`；PUT 单请求上限 32 MiB。
- 授权：第一层 RBAC `obj://bucket/<bucket_id>/<key>`（默认策略在 `buckyos-api` `rbac_config.rs`）；Agent（DID 会话）只在 owner 的 user 桶上 list/read，读写模式再加 create/write；服务会话对自己 appid 的桶全权；之后由 lib 判 grant、令牌与档位规则。
- 后台：本服务是 Zone named store 的 GC 宿主，每 30 秒投递级联边、刷 LRU、过期 pin；并驱逐无 pin、无入边的 class 0 对象和 24 小时未访问的无主 chunk（`services/nfs-server/settings.named_store_gc_evict`，默认开；每个写入方都把要留的内容绑进桶或持有 Lease，关掉只用于排查）。桶 reconcile 每 10 分钟（启动一次、每日深度对账与用量重算），生命周期每小时。对象事件发 kevent `/bucket/<bucket_id>`。

- S3 兼容面：`/s3/*`（网关 `/s3/*` 转发）与 `s3.<zone>` 的所有路径（scheduler 的 APP_INFO `s3` 项），路径风格、region `us-east-1`；协议在 cyfs-ndn `s3_gateway`，本服务只回答密钥对应的 (user, app) 与它的 RBAC（`runtime.enforce_principal`，无会话 token）。密钥用 kRPC `bucket.create_s3_key {appid, scope?, expires_at?}` / `bucket.list_s3_keys` / `bucket.revoke_s3_key` 管理，只接受用户本人从系统 UI 来的会话；桶名是 `<appid>-<name>`。

- Files 投影（设计 §4.5.4，`src/bucket/projection.rs`）：用户 `documents` 档的 `data` 桶只读投影到 `cyfs:///home/<user>/Apps/<App 显示名>/<桶名>/<key 按 / 展开>`，`collected` 档到 `Collected/`；本地真实目录同名时真实目录优先。App 显示名取 spec 的 `app_doc.show_name` / `service_doc.show_name`（独立模式取 `--bucket-specs` 的 `show_name`），重名追加 ` (<appid>)`，App 全部桶都是孤儿时目录名加 `（已卸载）`（部分桶孤儿时加在桶目录上）；key `a` 与前缀 `a/` 并存时文件显示为 `a~file`。节点 ref 为 `bk_<base64url(home 下路径)>`、条目 `binding: "projection"`、`flags: ["read_only"]`、没有 `copy_ref`；文件的 `access_urls.read` 是 `/bucket/v1/<bucket_id>/<key>`。可写操作只有 `delete`：删文件 = 解绑该 key，删前缀目录要 `recursive: true`（解绑其下所有 key），桶与 App 目录不能从 Files 删除；权限按 home 的 NFS ACL（本人 Manage）。不提供：投影内新建/写入/移动/复制（复制走 `read` URL 下载）、目录变化的 watch 推送（revision 随桶的 key 数与版本数变化）。

回归：`cargo test -p nfs_server --test bucket --test projection`；lib 单测 `cargo test -p bucket_store -p s3_gateway`（cyfs-ndn）。

## 预览管线(repr)

`src/preview/`:Preview 组件不能直显的内容(Chromium 上的 HEIC/HEIF、HEVC,浏览器播放失败的 MOV 等)
由 nfs_server 进程内 spawn ffmpeg/ffprobe 转成兼容副本,原文件字节、ObjId、下载原文件不变
(`notepads/preview-iphone-media-compatibility-todo.md` D1–D10、Preview PRD §23.6–23.7)。

- **工具**(`tools.rs`):依次查 `$BUCKYOS_ROOT/libexec/buckyos-tool/bin/` → `PATH`(macOS 另查
  `/opt/homebrew/bin`、`/usr/local/bin`),Windows 加 `.exe` 且子进程无控制台;**有则使用,没有就失败**,
  不下载不安装。启动时、每次 `hello` 与 `repr` 前按 (路径, 大小, mtime) 复核(只 stat,变了才重新跑 `-version`);`hello.repr` 给出
  `available`、两者路径与版本、`tonemap`(zscale+tonemap)、`h264_encoder`(libx264)、`heif`(≥7.1 才有 tile grid)、
  `profiles`,缺失时给 `missing` 与 `detail`。HEIC 需要 ffmpeg ≥ 7.1。
- **协议**(`rpc.rs`,websdk `NfspClient.repr/getRepr` 同形):`repr` 的源是 `at`(NFSP 路径/ref,按路径 ACL
  判 read)或 `args.source`(`{bucket:{bucket_id,key,version?,access_token?}}` 按桶权限,`{obj_id}` 按 NDM 读权);
  `args.purpose` = `preview|thumbnail`,`args.runtime` 是能力提示(`video_codecs.hevc` 决定重封装还是转码),
  `retry` + `expected_attempt_id` 是显式重试的 CAS;未知参数拒绝。规划结果 `state:"unsupported"`
  (`reason` = `no-pipeline|no-renderer|not-content`,`detail` 说明);执行失败 `state:"failed"` + `error`。
  `get_repr {work_key}` 只查询,要求本主体一小时内 ensure 过。
- **记录**(filedb `preview_works`):`work_key = sha256(输入 ObjId, nfs.preview/<版本>@ffmpeg-<版本>, params)`;
  本地文件的输入 ObjId 是 `sha256:<内容>`,按 (路径, 大小, mtime) 缓存在 `preview_inputs`;桶/对象源用内容
  chunk/chunklist id。同一 work_key 合并等待,调用方离开不杀进程;失败负缓存 10 分钟,显式 Retry 立即新 attempt;
  重启把 `processing` 记为可重试的 `INTERRUPTED`。
- **档位**(`convert.rs`):`preview-image` JPEG 长边 ≤4096(HEIF tile grid 拼接、`irot`/`imir` 烘进像素、透明→白底);
  `preview-video` H.264 High ≤1080p yuv420p + AAC 128k `+faststart`(H.264,或 Runtime 可播的 HEVC 只 `-c copy`
  重封装;旋转由 autorotate 烘进像素);`preview-audio` AAC/MP4;`thumbnail` JPEG 长边 ≤640(视频取 min(1s,10%) 帧)。
  PQ/HLG 一律 tonemap 到 SDR BT.709 并在 `fidelity` 注明。并发按 CPU 1–2,超时 max(60s, 时长×10)。
- **产物**:FileObject 绑定在 zone 桶 `zone/nfs-server/_/preview-cache`(`app_data`,key = work_key);结果的
  `read_url` 是 `/bucket/v1/...?token=bkt_…`(只读、单 key、6 小时),每次 `repr` 先判源权限再签,命中缓存也一样。
  30 天未被 `repr` 访问的结果过期,总量按 `services/nfs-server/settings.preview_cache_mib`(默认 20480)LRU 收缩;
  绑定内容丢失时下一次 `repr` 重新转换。
- **验证**:`cargo test -p nfs_server --test preview`(真实 HEIC/HEVC 样本在 `tests/fixtures/preview/`,由 libheif /
  x265 编码;没装 ffmpeg 时转换用例跳过,只跑"缺失即 unsupported")。

## 编辑器取消写入

控制面写操作 `abort_write { lease_id }` 使用当前 NFSP `session` 与递增 `seq`。有效租约只能由所属会话取消；成功返回 `{ "aborted": true }`、释放租约并删除暂存上传。不存在或已提交的租约重复取消也成功，便于客户端在 `finally` 清理。跨会话取消返回 `LEASE_CONFLICT`。NFSP 会话绑定发起 hello 的身份，因此租约也只属于该用户。

## Named-object destination commits

`POST /nfs/v1/commit_object` is a sequenced write. Its envelope uses `at` (or `args.parent_ref`) plus `args.name`, `args.obj_id`, and optional `overwrite`, `expected_revision`, `lease_id`. Only FileObject and DirObject are accepted. The gateway validates the request's original Bearer/cookie principal against NDM holdings/read authorization, then extracts from NamedStore locally. Browser bytes are sent only through the SDK's NDM import.

File commits retain the existing path ACL, binding, CAS, active-lease, and bypass-edit guards. Directory imports preserve their root and empty descendants, check destination ACLs and leases, and require an unused destination name. A retry whose extracted bytes/tree exactly match the existing destination succeeds without replacing content; a differing existing directory requires rename or skip. Successful writes invalidate the parent listing. A sequence replay returns the recorded result; after session expiry the same object and destination can be retried by content comparison. Client cancellation stops preparation and transport; an in-flight destination commit can finish and should be reconciled by refreshing or retrying the same object.

Extraction uses a task-owned staging path on the target filesystem. `.nfs_import_*` and `.nfs_tmp_*` are reserved and inaccessible through NFSP. Failures clean only the task's staging and temporary pin. This is not a multi-root directory transaction or an atomic upload batch: earlier successful tasks remain present, and failed tasks are retried individually. File-system readers no longer depend on the uploaded object's temporary lease after extraction. `commit_file` remains the content-edit operation and no longer accepts an `obj_id` alias for a SHA-256 hash.

To verify the browser path against an isolated real NFS instance, build the WebSDK and `nfs_server`, then run `FB_NFSP_AUTO=1 pnpm --dir frame/desktop exec playwright test tests/e2e/pages/filebrowser.nfsp.spec.ts` from `buckyos/src`. The fixture creates temporary exports and NamedStore, checks actual destination bytes and duplicate upload offsets, and cleans up its own processes and data. Set `CARGO_TARGET_DIR` consistently when using a custom Cargo target directory.

The SDK can also exercise the real HTTP protocol without a browser. From the repository root run `NFS_SERVER_BIN=/path/to/target/debug/nfs_server node test/nfs_server_test/sdk_import_http.mjs` after building both the SDK and NFS. `BUCKYOS_SDK_MODULE` optionally selects the SDK's `dist/node.mjs`; by default the sibling `buckyos-websdk` checkout is used. This creates a fresh NamedStore on a random loopback port and verifies zero bytes, 48 KiB, 33 MiB, duplicate content (12 KiB sampling and no data upload), and Unicode/empty/nested directories. The Node runner uses AuthDisabled; the browser fixture uses local test identities. Neither replaces normal-user Zone/SSO/Gateway acceptance.
