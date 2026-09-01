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
以服务器本地文件系统为命名空间真相源,对 WebUI(File Browser)提供 NFSP 协议服务;
后续通过替换底层实现平滑过渡到 cyfs-ndn 的 DFS(fs_meta)。

```
阶段一:WebUI → nfs_server(本地FS + filedb + NamedStore link模式)
阶段二:WebUI → nfs_server(fs_meta + named_store)          ← 客户端零改动
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
3. **`rev` 是唯一正确性来源、watch 事件有损 + `resync`**(NFSP §5.6 / D11):
   v1 的 rev 与 watch 天然可以在这个语义框架内降级。

### 1.4 目标与非目标

**目标**

- G1:支撑 filebrowser PRD 的 MVP 功能(浏览/上传/下载/搜索/分享/Topic 只读)。
- G2:客户端(WebUI)只面向 NFSP 协议编程,阶段二零改动。
- G3:服务端内部以 trait 隔离命名空间实现,阶段二只替换底层。
- G4:所有降级语义显式、诚实——宁可返回错误/stale,绝不静默给出错误内容。

**非目标**

- 不实现 fs_meta,不做 Base/Upper overlay(v1 所有目录都是"纯 upper")。
- 不做跨 Zone、不做设备视图 referral(可先硬编码只读)。
- 不做 FUSE / 本地挂载。
- 不追求 POSIX 兼容(继承 NFSP §1.2)。

---

## 2. 架构

### 2.1 分层与 trait 边界

```
┌─────────────────────────────────────────────────┐
│  NFSP handlers(hello/resolve/list/commit/...)  │ ← 一次写对,阶段二不动
│  错误码映射 / want 掩码 / batch 游标 / cap 校验   │
├───────────────── trait NamespacePlane ──────────┤
│  v1: PassthroughFs 实现                          │ ← 阶段二整体替换为 fs_meta
│   = 本地FS直通 + filedb + Reconciler(对账循环)   │
├─────────────────────────────────────────────────┤
│  NamedStore(link 模式为主)                      │ ← 两阶段共用
└─────────────────────────────────────────────────┘
```

**trait 边界放在 fs_meta 的 dentry/inode 原语层**(Ops_v3 §0.4 定义的那组操作):
resolve / list / create_dentry / replace_target(CAS) / lease 表操作。
这样 handler 层沉淀的全部工作——错误码、掩码、游标、watch 管道、上传编排、
cap 校验——原样平移到阶段二。

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
| 命名空间(树、名字、大小、mtime) | **本地文件系统** | nfs_server 无状态直通,不镜像 |
| 锚点及其派生物(view/meta/grant) | **filedb** | 见 §3.2 |
| chunk 内容 | 本地文件(link 模式)/ NamedStore(Store 模式) | 见 §5 |
| rev / lease / seq 重放窗口 | **内存** | 重启即失效,靠协议的 resync/STALE 语义恢复 |

### 3.2 filedb:最小持久集

设计哲学:**filedb 只装本地文件系统表达不了的东西**。但其中不可约的核心不是
view/collection 本身,而是**文件身份锚点**——view 只是锚点表上的派生物;
锚点不稳,collection 必然烂掉(成员死链,违反 PRD 11.5"路径始终可信")。

**持久表(丢失即数据丢失):**

| 表 | 内容 |
|---|---|
| `anchors` | `bucky_id`(自增或uuid)↔ 原生文件ID(ino/FileID)↔ 最后已知路径 ↔ size/mtime ↔ qcid ↔ full_hash(可空) |
| `view_base` | view 自动发现成员(anchor 引用 + provenance),对应 NFSP §3.5 Base Layer |
| `view_patch` | add/remove/pin,对应 Upper Layer;两表合并规则同 I2 |
| `meta_records` | MetaRecord,anchor 列为 `inode:<bucky_id>` 或 `obj:<obj_id>` |
| `grants` | cap 签发记录(id、subtree、ops、expiry、revoked) |

**缓存表(可全量重建,允许丢):** hash 缓存、搜索索引(名字索引起步)、缩略图映射。

**惰性锚定原则:** 只有当文件第一次进入 view、被写 meta、或被分享时才分配 bucky_id
建 anchor 行。纯浏览零 DB 写入。DB 规模 ∝ 参与高级功能的文件数,而非文件总数。

**schema 约束:** anchors 表结构必须是 fs_meta inode 表的可映射子集——
阶段二迁移 = 把 anchors 灌成 fs_meta 的 inode 且 **bucky_id 保留为 inode_id**,
view/meta/grants 原样跟随。这是"平滑过渡"的实际含义:已发出的 Ref、meta 锚点、
分享链接跨切换存活。

### 3.3 Ref 设计(NFSP 纪律:所有 op 只认 Ref 不认路径)

两级身份:

- **未锚定文件**:opaque handle,内部编码(export_root_id, 相对路径, 原生文件ID)。
  解析时校验原生ID仍与路径匹配,失配返回 `STALE`,客户端重新 resolve。
  这就是经典 NFS filehandle 方案。
- **已锚定文件**:`InodeRef{inode_id: bucky_id, gen}`,由 anchors 表解析。

**红线:WebUI 不得在任何持久位置(收藏、view、meta、分享)存路径,只能存 Ref。**
这是阶段二客户端零改动的前提。

### 3.4 rev 语义(相对 I3 的显式偏离)

- 按目录在内存中惰性维护计数器;经协议的写入同步递增;Reconciler 发现的旁路变更事后递增。
- **偏离:rev 不跨进程生命周期单调。** 客户端必须把 rev 当**相等性令牌**使用
  (CAS 与缓存失效判断),不得当序号比大小。此约定写入客户端开发文档。
- 重启后所有 watch 连接收到 `resync`,客户端全量重拉——协议 D11 已定义此路径。

### 3.5 锚点维护(Reconciler 的核心职责)

变更检测阶梯(与 NamedStore link 模式共用同一套判据,见 §5):

```
(size, mtime) 未变 → 视为未变(0 IO)
(size, mtime) 变、qcid 未变 → 仅 touch(8KB IO)
qcid 变 → 重算完整 hash,产生新 obj_id;旧 obj 上的 meta 走谱系迁移(commit 时刻
         同时知道 bucky_id 与新旧 obj_id,可显式记录 lineage)
```

两个破坏源及对策:

1. **rename/move**(原生ID不变、路径变):watcher 事件驱动更新 anchors.path。
   平台机制:NTFS 用 USN Journal(可补读离线期间变更);Linux 用 inotify,
   有条件时用 fanotify `FAN_REPORT_FID`(5.17+);macOS 用 FSEvents + 文件ID反查。
2. **编辑器覆盖保存**(路径不变、原生ID变):启发式重绑——同目录同名出现新原生ID,
   anchor 换绑到新ID(该场景的语义本意就是"同一文件被编辑")。

两者都失败(离线期间既改名又编辑)→ anchor 标 **stale**,不猜。
View 成员显示"失联"并提供重新定位入口(协议 View 对象已有 `stale` 字段)。
**诚实的 stale 优于静默的错链。**

---

## 4. 协议实现范围与降级契约

### 4.1 v1 方法集(NFSP §12.2 MVP 子集的再裁剪)

| 组 | 方法 | v1 状态 |
|---|---|---|
| 会话 | `hello` / `bye` | 完整 |
| 解析 | `resolve` / `stat` / `list` / `batch` | 完整(无状态游标、want 掩码、稳定字节序排序) |
| 写入 | `mkdir` / `move` / `delete` / `open_write` / `commit_file` | 完整(rev CAS + 内存租约) |
| 上传 | `probe` + tus 续传 + `commit_file` | 完整(见 §5.3) |
| 元数据 | `get_meta` / `set_meta`(user ns) | 完整 |
| 视图 | `open_view` / `view_page` | 只读;`view_patch` 延后 |
| 搜索 | `search` | v1 只做 `name` mode,但**响应结构完整**(带 `match_source`/`sources[]`),后续加 mode 不改结构 |
| 分享 | `grant` / `revoke` | 完整(bearer cap) |
| 派生 | `repr`(thumb256/thumb1024) | 完整(产物进 NamedStore) |
| 通知 | `watch`(SSE) | 完整(有损 + `resync`) |

**不实现且不在 hello 广告:** `get_tree`、`link_obj`、`publish_dir` / Frozen、
`referral`、`get_policy` 多维策略、跨 Zone、`search.semantic`。

### 4.2 降级契约(必须写进客户端开发文档)

| 语义 | 忠实 / 降级 | 说明 |
|---|---|---|
| 信封 / seq exactly-once | 忠实 | 重放窗口在内存;重启后 `SEQ_OUT_OF_WINDOW`,客户端重新 resolve |
| rev CAS(协议客户端之间) | 忠实 | 写入全部串行经过 nfs_server |
| 租约 I4(协议客户端之间) | 忠实 | 多标签页/多设备单写保护是真的 |
| 租约(对服务器本机进程) | **劝告性** | 旁路编辑在 `commit_file` 时显式冲突(size/mtime/qcid 复检),不静默覆盖 |
| rev 跨重启单调(I3) | **降级** | 相等性令牌;重启 resync |
| `NEED_PULL`(I6) | 服务端不产生 | v1 全部内容在本地;客户端仍须实现三态处理 |
| Frozen 承诺 | **不提供** | 不广告 `frozen-subtree`;绝不返回背后是可变本地文件的 Frozen |
| meta 锚定 | 忠实(按协议) | 可变文件锚 `inode:`(§3.4 本就允许),committed 内容锚 `obj:` |
| Cache-Control immutable | 仅对 Store 模式对象 | link 模式内容不发 immutable 头 |

### 4.3 错误模型

完整实现 NFSP §8 错误码表。v1 额外强调:

- `STALE`(handle 失效)→ 客户端重新 resolve。**【上游】** NFSP §8 无此码,需补入协议
  (NFSv4 有对应物,阶段二 fs_meta 同样需要它表达 inode 回收)。
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
`commit_file`(rev CAS 原子绑定)。上传中的文件不进命名空间(D4),
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

- `GET /nfs/v1/watch` SSE,事件集按 NFSP §5.6:`dir_changed` / `meta_changed` /
  `view_updated` / `lease_recall` / `resync`。
- 事件来源:协议写入(同步发)+ Reconciler(去抖后发,建议 200–500ms 窗口)。
- `hint` 尽力而为;**正确性来源永远是 rev**(客户端收到 dir_changed 后重拉 list)。
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
