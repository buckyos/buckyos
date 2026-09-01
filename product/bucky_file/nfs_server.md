# nfs_server 技术需求文档(v1,无 fs_meta 阶段)

> 文档状态:Draft
> 日期:2026-09-01
> 代码位置:`buckyos/src/frame/nfs_server`(frame 服务,形态对齐 `smb_service`)
> 上游协议:[NFSP v0](../../../cyfs-ndn/doc/NamedFileSystem_Protocol_v0.md)
> 产品需求:[filebrowser_PRD.md](./filebrowser_PRD.md)
> 记法:**【待决策】** = 需要 owner 拍板;**【上游】** = 需要 cyfs-ndn 侧配合修改。

---

## 1. 定位与总体策略

### 1.1 一句话定位

**nfs_server 是 NFSP v0 的一个"降级但诚实"的服务端实现**:在没有 fs_meta 的环境里,
以服务器本地文件系统为实体树真相源、以 filedb 为虚拟 Node / Binding 真相源,
对 WebUI(File Browser)提供统一的 Node / Entry / Container 协议服务;
后续通过替换实体树实现平滑过渡到 cyfs-ndn 的 DFS(fs_meta)。

```
阶段一:WebUI → nfs_server(本地FS实体树 + filedb虚拟Binding/View/Collection + NamedStore)
阶段二:WebUI → nfs_server(fs_meta实体树 + filedb虚拟Node + named_store) ← 客户端零改动
```

### 1.2 为什么这条路成立

NFSP 的六条不变式(Ops_v3 I1–I6)里,I3/I4/I5/I6 依赖 fs_meta 作为强一致真相源。
但在"网页访问服务器上的文件系统"这个产品形态下,**所有协议客户端的写入都必经
nfs_server 进程**,唯一的旁路写入者只剩服务器本机进程——这与传统 NFS/Samba 服务器
面对的局面完全相同,是有成熟解法的工程问题,而不是不可能的强一致模拟。

### 1.3 平滑过渡的三条依赖

协议里三样东西专门支撑这种分期,v1 必须正确使用:

1. **`hello` feature 协商**(NFSP §4.4 / G7):v1 只广告实现了的 feature;
   阶段二 feature 集合变大,老客户端无感。
2. **结构化三态错误**(NFSP §8):客户端从第一天就必须处理 `NEED_PULL` / `REFERRAL` /
   `STALE`,即使 v1 服务端永远不返回前两者。
3. **opaque `revision` 是唯一正确性来源、watch 事件有损 + `resync`**(NFSP §5.6 / D11):
   v1 的目录计数器、View/Collection generation 与 watch 都统一在这个语义框架内降级。

### 1.4 目标与非目标

**目标**

- G1:支撑 filebrowser PRD 的 MVP 功能(浏览/上传/下载/搜索/分享/Topic 只读/Collection)。
- G2:客户端(WebUI)只面向 NFSP 协议编程,阶段二零改动。
- G3:服务端内部以 trait 隔离命名空间实现,阶段二只替换底层。
- G4:所有降级语义显式、诚实——宁可返回错误/stale,绝不静默给出错误内容。
- G5:Dir/View/Collection 统一返回 Container + Entry;协议从 v1 起允许 DFS Entry 引用
  View/Collection,即使 `reference-binding` feature 分期开放。

**非目标**

- 不实现 fs_meta,不做 DirObject Base/Upper 内容 overlay;但必须实现"本地 FS native entries +
  filedb virtual bindings"的最小 sidecar merge,否则未来无法在实体目录中引用 View/Collection。
- 不做跨 Zone、不做设备视图 referral(可先硬编码只读)。
- 不做 FUSE / 本地挂载。
- 不追求 POSIX 兼容(继承 NFSP §1.2)。

---

## 2. 架构

### 2.1 分层与 trait 边界

```
┌─────────────────────────────────────────────────┐
│ NFSP handlers(resolve/stat/list/bind/commit/...)│ ← 一次写对,阶段二不动
│ Node/Entry/Binding / want / cursor / cap         │
├────────────── trait NamespacePlane ──────────────┤
│ UnifiedNamespace                                 │
│  ├─ NativeTree: v1 PassthroughFs → 阶段二 fs_meta│
│  └─ VirtualPlane: filedb View/Collection/Binding │ ← 两阶段共用
│     + Reconciler(v1 对账循环)                    │
├─────────────────────────────────────────────────┤
│  NamedStore(link 模式为主)                      │ ← 两阶段共用
└─────────────────────────────────────────────────┘
```

**trait 边界放在统一 Node/Entry 原语层**,其中 NativeTree 对齐 fs_meta 的 dentry/inode 原语
(Ops_v3 §0.4),VirtualPlane 提供同构的 Container 原语:
resolve_locator / stat_node / list_entries / create_binding / unlink_entry /
replace_target(CAS) / lease 表操作。
这样 handler 层沉淀的全部工作——错误码、掩码、游标、watch 管道、上传编排、
cap 校验——原样平移到阶段二。

`list_entries` 必须对 Dir / View / Collection / Group 返回同一信封。NativeTree 与
VirtualPlane 只在 NamespacePlane 内部按 Node kind 路由,WebUI 不得感知实现分层。

**Reconciler 是 v1 唯一的一次性组件**(阶段二删除),必须隔离成独立模块,
不允许其它模块依赖它的内部状态。

### 2.2 部署形态

- frame 服务,依赖 `buckyos-kit` / `buckyos-api`,由 node_daemon 拉起,对齐 `smb_service`。
- 导出根目录(export roots)由服务配置声明:哪些本地目录以哪个逻辑路径暴露。
  v1 至少支持多个 root 映射到 `dfs://` 命名空间的一级子目录。
- 鉴权:本 Zone 已登录用户经 buckyos SSO 换取 session(NFSP §7.1 第一行);
  分享链接走 `grant` 签发的 bearer cap。`uid/gid` 不进协议。

---

## 3. 数据模型

### 3.1 真相源划分(v1 的核心决定)

| 数据 | 真相源 | 说明 |
|---|---|---|
| 实体 DFS 树(native Entry、名字、大小、mtime) | **本地文件系统** | nfs_server 无状态直通,不镜像 |
| 虚拟 Node / Binding(View、Collection、Group、DFS reference Entry) | **filedb** | 与 native Entry 在服务端 list 时合并 |
| 锚点及其派生物(meta/grant) | **filedb** | 见 §3.2 |
| chunk 内容 | 本地文件(link 模式)/ NamedStore(Store 模式) | 见 §5 |
| Dir revision / lease / seq 重放窗口 | **内存** | 重启即失效,靠协议的 resync/STALE 语义恢复 |
| View/Collection revision | **filedb** | 持久 generation,对外编码为 opaque revision |

可见命名空间不是把 View/Collection 伪装成本地目录,而是:

```
VisibleEntries(dir) = NativeFsEntries(dir) + VirtualBindings(dir)
```

native 父子边继续满足严格树;reference 边形成可成环导航图。递归操作默认不跟随 reference。

### 3.2 filedb:最小持久集

设计哲学:**filedb 只装本地文件系统表达不了的东西**。其中不可约的核心包括
稳定 Node 身份、虚拟 Binding、View/Collection 结构。View/Collection 成员只保存 Ref;
锚点不稳,Collection 必然烂掉(成员死链,违反 PRD 11.5"路径始终可信")。

**持久表(丢失即数据丢失):**

| 表 | 内容 |
|---|---|
| `entities` | 全局 `node_id`、gen、kind、owner、created_at;虚拟 Node 必有行,实体文件惰性建行 |
| `anchors` | `node_id` ↔ 原生文件ID(ino/FileID) ↔ 最后已知路径 ↔ size/mtime ↔ qcid ↔ full_hash(可空) |
| `namespace_bindings` | `entry_id`、parent Node、name、target Ref、binding_type、state;表示 DFS 目录中的 virtual reference |
| `views` | View Node、title/origin/generation/stale/query 等 |
| `view_base` | View 自动发现成员(target Ref + provenance),对应 NFSP §3.5 Base Layer |
| `view_patch` | add/remove/pin,对应 Upper Layer;两表合并规则同 I2 |
| `collections` | Collection Node、title/description/generation/owner |
| `collection_nodes` | `entry_id`、parent Group、node_type(ref/group)、target Ref/name、manual_order |
| `meta_records` | MetaRecord,anchor 列为 `live:<node_id>` 或 `obj:<obj_id>` |
| `grants` | cap 签发记录(id、subtree、ops、expiry、revoked) |

**缓存表(可全量重建,允许丢):** hash 缓存、搜索索引(名字索引起步)、缩略图映射。

**惰性锚定原则:** 只有当实体文件/目录第一次进入 View/Collection、被写 meta、被分享、
或作为 virtual binding 的 parent/target 时才分配稳定 node_id 建 anchor 行。纯浏览零 DB 写入。
DB 规模 ∝ 参与高级功能的 Node 数,而非文件总数。

Collection 和 namespace binding 的目标列只能保存 Ref,不得保存 path 作为身份。页面返回时服务端
批量解析 target Ref 补 `canonical_path`;rename 后自然跟随。失联保存 Entry 并返回 stale,
权限不足则保留最小占位但不得泄漏目标属性。

**schema 约束:** entities.node_id 是 NFSP LiveRef 的稳定 id。实体 file/dir 的 anchors 必须可映射到
fs_meta inode;阶段二迁移时保留 node_id(或建立一一映射而不改变对外 LiveRef)。
View/Collection/Group 继续由 VirtualPlane 管理,namespace_bindings/meta/grants 原样跟随。
这是"平滑过渡"的实际含义:已发出的 Ref、Collection 成员、Meta 锚点和分享链接跨切换存活。

### 3.3 Ref 设计(NFSP 纪律:locator 只用于首次解析,后续 op 只认 Ref)

两级 LiveRef:

- **未锚定实体 Node**:LiveRef.node_id 是带签名的 opaque handle,内部编码
  (export_root_id, 相对路径, 原生文件ID),gen 表达句柄代际。
  解析时校验原生ID仍与路径匹配,失配返回 `STALE`,客户端重新 resolve。
  这就是经典 NFS filehandle 方案。
- **已锚定或虚拟 Node**:`LiveRef{node_id,gen}`,由 entities + anchors/View/Collection 表解析。
  View/Collection/Group 从创建起就有稳定 LiveRef。

**红线:WebUI 和服务端都不得在任何持久引用位置(收藏、View、Collection、Meta、分享)
存路径作为身份,只能存 Ref。** path/`view://`/`collection://` 只是 locator 或展示 location。
这是阶段二客户端零改动的前提。

### 3.4 Container revision 语义(相对 I3 的显式偏离)

- Dir 在内存中惰性维护计数器;协议写入同步递增;Reconciler 发现旁路变更后递增。
- View/Collection/Group 使用 filedb generation。handler 对外统一编码为 opaque `revision`。
- **偏离:Dir revision 不跨进程生命周期单调。** 客户端必须把所有 revision 当**相等性令牌**使用
  (CAS 与缓存失效判断),不得跨 kind 或按数值比大小。此约定写入客户端开发文档。
- 重启后所有 watch 连接收到 `resync`,客户端全量重拉——协议 D11 已定义此路径。

### 3.5 Entry / Binding 与 sidecar merge

- native Entry 来自本地 FS;entry_ref 可由 parent handle + name + 原生文件ID 派生。
- View 成员是 `binding:derived`;Collection 文件成员是 `binding:reference`,自有 Group 是
  `binding:member`;DFS 中引用 View/Collection/文件也是 `binding:reference`。
- `entry_ref` 与 `target.ref` 必须分离。同一 target 可在 Collection 中出现多次;
  unlink reference 只删 Entry,不删 target。
- `list(DirRef)` 合并 native Entry 与 `namespace_bindings`;绑定创建时拒绝同名。服务器本机旁路
  产生同名 native 项时,native 可见项保持不变,virtual binding 标 conflict 并通过 `conflicts[]`
  + `container_changed/resync` 暴露,绝不静默覆盖或改绑。
- reference 图允许成环。递归操作默认不跟随;显式跟随必须有 `max_hops` 并按 Ref 去重。
- 权限 = 可见 Binding 与可访问 target 的交集。无 target 权限时只返回不可用占位,
  不返回名称以外的目标信息、canonical_path、attrs 或成员数量。

### 3.6 锚点维护(Reconciler 的核心职责)

变更检测阶梯(与 NamedStore link 模式共用同一套判据,见 §5):

```
(size, mtime) 未变 → 视为未变(0 IO)
(size, mtime) 变、qcid 未变 → 仅 touch(8KB IO)
qcid 变 → 重算完整 hash,产生新 obj_id;旧 obj 上的 meta 走谱系迁移(commit 时刻
         同时知道 node_id 与新旧 obj_id,可显式记录 lineage)
```

两个破坏源及对策:

1. **rename/move**(原生ID不变、路径变):watcher 事件驱动更新 anchors.path。
   平台机制:NTFS 用 USN Journal(可补读离线期间变更);Linux 用 inotify,
   有条件时用 fanotify `FAN_REPORT_FID`(5.17+);macOS 用 FSEvents + 文件ID反查。
2. **编辑器覆盖保存**(路径不变、原生ID变):启发式重绑——同目录同名出现新原生ID,
   anchor 换绑到新ID(该场景的语义本意就是"同一文件被编辑")。

两者都失败(离线期间既改名又编辑)→ anchor 标 **stale**,不猜。
View/Collection/namespace binding 成员显示"失联"并提供重新定位或 unlink 入口。
**诚实的 stale 优于静默的错链。**

---

## 4. 协议实现范围与降级契约

### 4.1 v1 方法集(NFSP §12.2 MVP 子集的再裁剪)

| 组 | 方法 | v1 状态 |
|---|---|---|
| 会话 | `hello` / `bye` | 完整 |
| 解析 | `resolve` / `stat` / `list` / `batch` | 完整;`list` 接受 Dir/View/Collection/Group Ref,统一信封 |
| 写入 | `mkdir` / `move` / `delete` / `open_write` / `commit_file` | 完整(revision CAS + 内存租约) |
| 引用绑定 | `bind_ref` / `unlink` | 报文与 schema 从 v1 固定;首版可不广告 `reference-binding`,但 list 必须能返回 Binding Entry |
| 上传 | `probe` + tus 续传 + `commit_file` | 完整(见 §5.3) |
| 元数据 | `get_meta` / `set_meta`(user ns) | 完整 |
| 视图 | `open_view` + 通用 `list(view_ref)` | 只读;`view_patch` 延后 |
| 集合 | `create_collection` / `open_collection` / `collection_patch` + 通用 `list` | 完整;只管理引用/Group,不是上传目标 |
| 搜索 | `search` | v1 只做 `name` mode,但**响应结构完整**(带 `match_source`/`sources[]`),后续加 mode 不改结构 |
| 分享 | `grant` / `revoke` | 完整(bearer cap) |
| 派生 | `repr`(thumb256/thumb1024) | 完整(产物进 NamedStore) |
| 通知 | `watch`(SSE) | 完整;统一 `container_changed(ref,revision)` + 有损 `resync` |

**首版不实现且不在 hello 广告:** `get_tree`、`reference-binding` 写操作、`publish_dir` / Frozen、
`referral`、`get_policy` 多维策略、跨 Zone、`search.semantic`。即使首版不允许用户创建 DFS Binding,
Node/Entry/list 报文也不得退回 Folder-only 形态。

### 4.2 降级契约(必须写进客户端开发文档)

| 语义 | 忠实 / 降级 | 说明 |
|---|---|---|
| 信封 / seq exactly-once | 忠实 | 重放窗口在内存;重启后 `SEQ_OUT_OF_WINDOW`,客户端重新 resolve |
| revision CAS(协议客户端之间) | 忠实 | Container 写入全部串行经过 nfs_server |
| 租约 I4(协议客户端之间) | 忠实 | 多标签页/多设备单写保护是真的 |
| 租约(对服务器本机进程) | **劝告性** | 旁路编辑在 `commit_file` 时显式冲突(size/mtime/qcid 复检),不静默覆盖 |
| Dir revision 跨重启单调(I3) | **降级** | opaque 相等性令牌;重启 resync;View/Collection generation 持久化 |
| `NEED_PULL`(I6) | 服务端不产生 | v1 全部内容在本地;客户端仍须实现三态处理 |
| Frozen 承诺 | **不提供** | 不广告 `frozen-subtree`;绝不返回背后是可变本地文件的 Frozen |
| meta 锚定 | 忠实(按协议) | 可变 Node 锚 `live:`;committed 内容锚 `obj:` |
| reference 删除语义 | 忠实 | `unlink(entry_ref)` 只解除 Binding;`delete` 才销毁 native target |
| reference 权限 | 忠实 | Binding 可见性与 target 权限取交集,不得借引用提权 |
| Cache-Control immutable | 仅对 Store 模式对象 | link 模式内容不发 immutable 头 |

### 4.3 错误模型

完整实现 NFSP §8 错误码表。v1 额外强调:

- `STALE`(LiveRef/EntryRef/动态 Group Ref 失效)→ 客户端从可信 locator 或父 Container 重新
  resolve/list。NFSP §8 已纳入该错误。
- `NAMESPACE_CONFLICT`(native 与 virtual binding 同名)→ list 返回 `conflicts[]`,客户端展示修复入口;
  服务端不得猜测、覆盖或自动改绑。
- link 模式 chunk 读取时 qcid 失配 → 数据面返回 5xx + 结构化 `VERIFY_FAILED`,
  同时触发 Reconciler 对该锚点重新走检测阶梯;不返回撕裂内容。

---

## 5. NamedStore 集成(link 模式为主)

### 5.1 存储策略

依据 [link to local.md](../../../cyfs-ndn/doc/link%20to%20local.md) 与
`named_store` 现有实现(`add_chunk_by_link_to_local_file` + `verify_local_link`):

- **commit 默认零拷贝**:算完整 hash + qcid,以 link 模式登记 chunk;
  每次打开 reader 前 `verify_local_link` 重校 qcid,失配响亮失败(检测而非防护,
  失败模式正确)。
- **Link → Store 单向提升**,触发时刻 = **对外承诺的生命周期超过本地文件时**:
  - `grant` 分享一个文件/子树;
  - 生成 pinned URL;
  - (未来)`publish_dir` / 广告 Frozen。
  本地自用的 commit 保持 link;坏了会 `VerifyError`,由 Reconciler 重绑重算。
- **小于 12KB 的文件必须走 Store 模式**(`calc_quick_hash` 对
  `MIN_QCID_FILE_SIZE` 以下报错),commit 路径显式分支;小文件拷贝成本可忽略。
- 缩略图(`repr` 产物)一律 Store 模式(它们本来就是新生成的字节)。

### 5.2 已知边界(接受并记录)

1. qcid 采样盲区:等长且不触及采样窗口的修改检不出。**不导出**数据库文件、
   VM 镜像等原地写文件所在目录,或对此类目录禁用 link 模式(配置项)。
2. verify 与流式读之间存在小窗口 TOCTOU:纯浏览器数据面兜不住撕裂读。
   已知限制,走 mtree proof 的客户端(阶段二)可端到端兜底。

### 5.3 上传路径

按 NFSP §5.3:客户端算 hash → `probe`(批量查缺,命中即秒传)→ tus 续传 →
`commit_file`(parent Container revision CAS 原子绑定)。上传中的文件不进命名空间(D4),
占位条目由 WebUI 本地渲染。上传落盘后即为普通本地文件,立即可被 link 模式登记。

**【待决策】** v1 是否复用 `/ndm/v1/uploads` 的 tus 实现,还是 nfs_server 内嵌
一个最小 tus endpoint。倾向后者(减少 v1 对 ndm gateway 的部署依赖),
但 URL 与报文格式保持与 NDM 协议一致,阶段二可切换。

### 5.4 【上游】cyfs-ndn 侧需要的三个修改

1. **qcid 算法不一致(疑似 bug)**:`calc_quick_hash`(hasher.rs:208)只采样
   头4KB+中点4KB 两片;`calc_quick_hash_by_buffer` 要求头/中/尾三片;
   `MIN_QCID_FILE_SIZE = 4096*3` 表明设计意图是三片。需统一为三片
   (补尾片同时收窄最常见的尾部修改盲区:MP4 moov、ID3 tag 等)。
2. **`verify_local_link` 增加 (size, mtime) 快速通道**:`ChunkLocalInfo` 存
   size/mtime,未变则跳过 qcid 重算,省高频小对象每次 GET 的 8KB IO。
3. `probe` 依赖的 `objects/lookup` 批量化(NFSP §12.1 P1 项)。

---

## 6. Watch 通知面

- `GET /nfs/v1/watch` SSE,事件集按 NFSP §5.6:`container_changed` / `meta_changed` /
  `lease_recall` / `policy_changed` / `resync`。
- 事件来源:协议写入(同步发)+ Reconciler(去抖后发,建议 200–500ms 窗口)。
- `container_changed` 携带 Node Ref、kind、opaque revision、reason;Dir/View/Collection/Group 同形。
- `hint` 尽力而为;**正确性来源永远是 revision**(客户端收到事件后按 Ref 重拉 list)。
- 大批量旁路变更(如服务器上跑构建)→ 直接发 `resync`,不逐条发事件。
- 断线重连:v1 不做事件缓冲补发,重连一律先收 `resync`(D11 允许)。

---

## 7. 里程碑

| 阶段 | 内容 | 验收 |
|---|---|---|
| M1 | 只读:hello/resolve/stat/list/batch + 数据面 GET(Range)+ 无状态游标 | WebUI 能浏览、下载,首屏 1 次 batch 往返(NFSP §9.1) |
| M2 | 写入:mkdir/move/delete/open_write/commit_file + 内存 rev/lease + watch | 多标签页写入互斥、变更互见 |
| M3 | filedb:anchors + 惰性锚定 + Reconciler(watcher+扫描+重绑启发式) | 旁路 rename/编辑后 Ref 与 meta 不丢;失联标 stale |
| M4 | NamedStore:probe/秒传/link 模式 commit/repr 缩略图 | 重复上传秒传;缩略图可被浏览器缓存 |
| M5 | meta/search(name)/open_view(只读)/grant 分享 | PRD MVP 功能闭环 |
| M6 | 阶段二准备:NamespacePlane trait 冻结,fs_meta 实现开工;anchors → inode 迁移工具 | 迁移演练:bucky_id 保留,客户端不改 |

## 8. 阶段二切换脚本(现在就约束设计)

1. 停写(维护窗口)→ Reconciler 最后一轮对账;
2. anchors 灌入 fs_meta(**bucky_id 保留为 inode_id**),未 hash 文件补算,
   字节按需 Link→Store;view/meta/grants 原样跟随;
3. NamespacePlane 换实现,`hello` feature 集合扩大;
4. 内存态(handle/rev/lease/seq)全部作废:客户端经 `STALE`/`resync` 重新 resolve
   ——这正是它们 v1 起就必须实现的路径,切换日无新代码。

## 9. 待决策清单

| # | 决策点 | 建议 |
|---|---|---|
| N1 | tus endpoint 内嵌 vs 依赖 ndm gateway | 内嵌最小实现,报文对齐 NDM 协议 |
| N2 | export roots 配置格式与热更新 | 静态配置起步,热更新延后 |
| N3 | 原地写文件目录(数据库等)的导出策略 | 提供目录级 `no_link` 配置,默认导出但禁 link |
| N4 | `STALE` 错误码进 NFSP 协议 | 需要,提交上游【上游】 |
| N5 | search 索引落 filedb 缓存表 vs 独立引擎 | v1 落 filedb(名字索引),语义检索阶段二再议 |
| N6 | Windows 服务器支持是否进 v1 | 【待决策】影响 Reconciler 的 watcher 选型(USN vs inotify) |
