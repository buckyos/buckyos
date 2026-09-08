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
- `copy_*` 方法验证实际用户 Bearer session token；其他 NFSP 方法尚未接入完整 SSO/cap 校验。

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
           [--debug-api] [--log-level info]
```

- `--export name=/abs/path`(可重复):每个 export root 成为当前 Zone 的 `cyfs:///` 命名空间一级子目录。
- `--data-dir`:存放 `filedb.sqlite`、`copy.sqlite`、`copy-locks/` 与上传暂存区。复制日志及目标目录中的复制临时文件保留用于恢复；上传暂存区每次启动清空。
- `--scan-interval-secs`:Reconciler 扫描周期,0 = 关闭。
- `--debug-api`:开启 `POST /nfs/v1/debug/{reconcile|create_view}`(仅测试/开发)。

```bash
cargo test -p nfs_server        # 单元与集成测试，不需要外部服务
```

## 协议覆盖(nfs_server.md §4.1 对照)

| 组 | 方法 | 状态 |
|---|---|---|
| 会话 | `hello` / `bye` | ✅ feature 协商、limits、realms;session 由 hello 返回 |
| 解析 | `resolve` / `stat` / `list` / `batch` | ✅ 统一信封;list 接受 Dir/View/Collection/Group Ref;无状态游标(D10 不重置,返回 `revision_changed`);batch 共享游标 walk/stat/list |
| 写入 | `mkdir` / `move` / `delete` / `open_write` / `commit_file` | ✅ revision CAS + 内存租约 + 旁路写 commit 复检(size/mtime) |
| 本地复制 | `copy_capabilities` / `copy_submit` / `copy_list` / `copy_get` / `copy_decide` / `copy_cancel` | ✅ 可信用户、TaskMgr 任务、逐项日志、恢复/取消/失败项重试；详见产品文档 §10 |
| 引用绑定 | `bind_ref` / `unlink` | ✅ sidecar merge、同名冲突 `conflicts[]`、unlink 只解引用 |
| 上传 | `probe` + tus 续传 + `commit_file` | ✅ 内嵌最小 tus(N1 采纳内嵌方案);probe/秒传基于 filedb `content_index` 缓存表 |
| 元数据 | `get_meta` / `set_meta`(user ns) | ✅ 锚定 `live:n_<id>` + `obj:sha256:*`;写 meta 触发惰性锚定 |
| 视图 | `open_view` + 通用 `list(view_ref)` | ✅ 只读;base/patch overlay 合并已实现;内容经 debug API / filedb 灌入(v1 无 AI 生成器) |
| 集合 | `create_collection` / `open_collection` / `collection_patch` + 通用 `list` | ✅ add_ref/remove_entry/move_entries/create_group/rename_group;成员只存 Ref,`canonical_path` 读取时批量补全 |
| 搜索 | `search` | ✅ name mode;响应结构完整(`match_source`/`explain`/`sources[]`);先序遍历 + 路径游标 |
| 分享 | `grant` / `revoke` | ⚠️ 签发/撤销/落库完整;**数据面尚不凭 cap 放行**(鉴权随 buckyos SSO 接入) |
| 派生 | `repr` | ❌ 未实现、不广告(依赖缩略图管线 + NamedStore) |
| 通知 | `watch`(SSE) | ✅ `container_changed`/`meta_changed`/`resync`;连接首事件恒为 `resync`(D11);watch_token 过滤 |
| 数据面 | `GET /nfs/v1/read/{node_id}` | ✅ Range/ETag(弱)/If-None-Match/Content-Disposition;**不发 immutable**(link 语义,§4.2) |

`hello` 广告的 features:`view` `collection` `reference-binding` `watch.sse` `search.name`。
不广告(未实现):`frozen-subtree` `search.semantic` `repr` `get_tree` `publish_dir`。

## filedb(§3.2 最小持久集)

持久表:`entities` `anchors` `namespace_bindings` `views` `view_groups` `view_base`
`view_patch` `collections` `collection_nodes` `meta_records` `grants` + `kv`(handle 签名密钥)。
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

1. SSO 鉴权 + cap 校验(grant 数据面放行)。runtime/login/心跳/调度已接入;完整 NFSP 按请求鉴权未开；复制方法已验证用户。
2. NamedStore link 模式(`add_chunk_by_link_to_local_file` / qcid 阶梯)替换 `content_index`。
3. `repr` 缩略图、`view_patch`、`get_tree`、`publish_dir`/Frozen、referral 设备视图。
4. NativeTree trait 冻结(M7):第二个实现(fs_meta)落地时从 `namespace.rs` 提取。

## 文件复制真实 E2E

从仓库 `src/` 执行（已有 Rust/Node/pnpm/Chromium 环境）：

```bash
cargo build -p nfs_server
cargo build -p task_manager --example nfs_copy_fixture
FB_NFSP_E2E=1 FB_COPY_AUTO=1 VITE_NFS_PROXY=http://127.0.0.1:3262 \
  pnpm --dir frame/desktop exec playwright test \
  tests/e2e/pages/filebrowser.copy.nfsp.spec.ts \
  tests/e2e/pages/filebrowser.nfsp.spec.ts --workers=1 --reporter=list
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
