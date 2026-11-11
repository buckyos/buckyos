# nfs_server：路径的对象 ID（`ident.obj_id`）TODO

> 状态：待实施。**优先级 P0**（owner，2026-11-10）：第二步 `snapshot` 卡住了聊天附件和发动态这两个最高频的“从 Zone 选择”入口，见 [sysdlg-select-adoption-todo](sysdlg-select-adoption-todo.md) N1。§2 是 owner 已裁决的方向，§6 有 4 个问题待裁决。现状核实于 2026-11-09（buckyos `b6c57704a`、buckyos-websdk `b12d6c3`、cyfs-ndn `9115234`），下文行号以这些提交为准。
>
> 背景：系统文件选择器（[设计稿](../doc/desktop/BuckyOS%20系统对话框实现方案.md) §4.3）要“尽力返回对象 ID”，来源是 NFSP 的 `ident.obj_id`。核实时发现 nfs_server 放在这个字段里的不是调用方能用的对象 ID。
>
> 目标：普通文件不再从 `ident.obj_id` 返回任何值；另外提供一个按需求对象 ID 的服务端接口，同一个文件第二次求值走缓存。
>
> 协议与设计依据：NFSP v0（`cyfs-ndn/doc/NamedFileSystem_Protocol_v0.md` §3.3、§5.2、§5.3）、[nfs_server 技术需求](../product/bucky_file/nfs_server.md) §3.6、§5、[nfs_server README](../src/frame/nfs_server/README.md)。FileObject 的构造以 `cyfs-ndn/src/ndn-lib` 为准。

## 1. 问题

系统里和“一个文件的内容”有关的 ID 有三种，nfs_server 给的是第一种，调用方要的是第三种：

| ID | 形式 | 由什么算出 | 谁在用 |
| --- | --- | --- | --- |
| 整文件摘要 | `sha256:<hex>` | 文件全部字节的 SHA-256 | nfs_server 自己：上传秒传、元数据锚点、预览缓存 |
| 内容 ID | 不超过 32 MiB 是 `mix256:` 的 chunk，更大是 `clist:` 的 ChunkList | 按 32 MiB 分块后的块哈希 | FileObject 的 `content` 字段，NamedStore 按它存取字节 |
| FileObject ID | `cyfile:<hex>` | 名字、大小、内容 ID、常用 meta 组成的规范 JSON | 安装器 staging、消息附件、分享、`commit_object`、`/ndn/<obj_id>` |

具体有三个缺口：

1. **类型不对。** `stat` / `list` 的 `ident.obj_id` 对普通文件返回 `sha256:<整文件摘要>`。安装器的 `apps.staging.finalize { source_obj_id }` 明确要 FileObject（`control_panel/src/app_installer.rs:2359`），消息附件要 FileObject / DirObject。
2. **NamedStore 里没有它。** nfs_server 的普通文件在导出目录里，不在 NamedStore；这个摘要只登记在 nfs_server 私有的 `content_index` 表。拿它去 `ndm` 读、去 `/ndn/` 取、去绑桶都找不到对象。
3. **覆盖不全。** 只有经 `commit_file` 写入、之后大小和修改时间都没变的文件才有这个值（`mutate.rs:675` 是唯一写入点）。宿主机程序直接写的文件、被改过的文件都没有。

## 2. 已裁决（owner，2026-11-09）

| # | 裁决 | 含义 |
| --- | --- | --- |
| 1 | nfs_server 只有在后端是分布式文件系统（fs_meta）时，才能零成本地返回 FileObject ID。现在的实现做不到，普通文件一律不返回 | 不改名保留、不在 commit 时补登记。`ident.obj_id` 对普通文件直接不输出 |
| 2 | 需要对象 ID 的调用方用 helper 去求（文件选择器的 `ensureObjId`）。对同一个文件第二次求值要有缓存 | 求值是显式的、有成本的操作，由本 TODO 的 `snapshot` 方法承担，缓存在服务端 |

裁决 1 的直接后果：技术需求 §5.1 里“commit 时以 link 模式登记 NamedStore”这条路不在本 TODO 里做。它本质上是在透传文件系统之上补一层对象登记，等后端换成 fs_meta 后由底层给出。

## 3. 现状

下文路径中，nfs 指 `src/frame/nfs_server/src/`。

### 3.1 `obj_id` 现在从哪来

| 位置 | 行为 |
| --- | --- |
| `namespace.rs:509-533`（`stat` / `resolve` 的 `ident` 组） | 普通文件：锚点有 `full_hash` 时输出 `sha256:<full_hash>`。对象桶投影出来的文件：输出绑定里的真实对象 ID |
| `namespace.rs:918-937`（`list` 的条目属性） | 同上，只对已有锚点、inode 仍匹配的普通文件输出 |
| `mutate.rs:675` → `filedb.rs:533 anchor_set_hash` | `commit_file` 完成时写入 `anchors.full_hash`，是唯一写入点。大小或修改时间变了就清空（`filedb.rs:460,465,511,521`） |
| `mutate.rs:637,646` | `commit_file { hash }` 命中不了本地内容时，`NEED_PULL` 错误里带 `obj_id: sha256:<hash>` |
| `mutate.rs:693 op_commit_object` | 从 NamedStore 把一个 FileObject / DirObject 提取成普通文件，返回值里回显调用方传入的 `obj_id` |

### 3.2 谁在读现在这个值

| 调用方 | 用法 | 去掉后的影响 |
| --- | --- | --- |
| Desktop 预览 `desktop/src/components/preview/nfspProvider.ts:194` | `inputObjectId: info.obj_id`，实际只用于信息面板里的“Object ID”一行（`ContentPreview.tsx:1640`）。预览产物的输入版本由服务端自己算（`preview/mod.rs:676`），不依赖客户端传的值 | 普通文件不再显示这一行，功能不变，不用改代码 |
| nfs_server 元数据锚点 `namespace.rs:634-646` | `obj:sha256:<hash>`，内部键，直接读 `anchors.full_hash` | 无 |
| 上传秒传 `probe` / `commit_file { hash }` / `content_index` | 用摘要查本地是否已有内容 | 无 |
| `tests/integration.rs:411` | 断言 `NEED_PULL` 的 `obj_id` 以 `sha256:` 开头 | 随 T1 改 |
| Text Editor | 只用 `etag`、`node_id` | 无 |

也就是说，客户端没有任何地方真正需要这个摘要，去掉它不需要补一个替代字段。

### 3.3 可以直接复用的能力

- **路径到 FileObject / DirObject 的构造已经存在。** 分享模块的快照（`share/snapshot.rs`）：`share_plan_source`（167 行，逐项查 ACL、默认排除、不跟随目录内符号链接）、`store_file`（234 行，32 MiB 分块、`mix256` 块哈希、超过一块用 ChunkList、`FileObject::new` 加 `apply_media_meta`、以 Store 模式写入 NamedStore）、`run_snapshot`（324 行，带进度、可取消、报告 `source_changed`）。
- **FileObject 的构造在 Rust 和 TS 之间已经对齐。** `ndn-lib/src/media_meta.rs` 与 websdk `src/file_meta.ts` 逐函数对应，有共享向量（见 `notepads/websdk-standard-objects-todo.md` 决定 16）。同一个文件由 nfs_server 构造和由浏览器 `ndm.importFiles` 构造，应当得到同一个 `cyfile:` ID。
- **对象的读权限按“用户 + App”的持有关系判定。** `ndm/mod.rs:302 may_read`：调用方自己持有（`ndm_holdings` 表，73 行），或者有通用读权。安装器 staging 会用调用方的 token 校验它能读 `source_obj_id`。
- **分享快照的保留方式。** 有效分享的快照绑定在 Zone 桶 `zone/nfs-server/_/shares`，停止或到期即解绑。
- **按“大小 + 修改时间”判断文件没变的缓存表已有先例。** `preview_inputs`（`preview/mod.rs:75`）、`anchors.full_hash` 都是这个判据。

## 4. 目标状态

1. `ident.obj_id` 只在条目本身就是对象时出现（现在只有对象桶的投影）。普通文件和目录不输出这个字段，任何响应里都不再有 `sha256:` 形式的 `obj_id`。
2. 有一个服务端方法，把调用方有读权的文件或目录变成 FileObject / DirObject 的 ID：对象和内容在本 Zone 的 NamedStore 里读得到，调用方对它有读权，内容不经过浏览器。
3. 对同一个没变过的文件再次调用，不重读文件内容。
4. 后端换成 fs_meta 之后，`ident.obj_id` 由底层直接给出，这个方法退化为“确保对象被保留并授权给调用方”，调用方不用改。

## 5. 待办

两步可以分开交付。第一步让字段说实话；第二步解除文件选择器等调用方的阻塞。

### 5.1 第一步：普通文件不再返回 `obj_id`

| 编号 | 内容 | 位置 |
| --- | --- | --- |
| T1 | `stat` 和 `list` 对普通文件不再输出 `ident.obj_id`。`NEED_PULL` 错误里的摘要改用和请求参数相同的名字 `hash`。对象桶投影文件的 `obj_id` 不动。`anchors.full_hash` 及其内部用途不动 | nfs `namespace.rs`、`mutate.rs`，`tests/integration.rs` |
| T2 | websdk 类型的注释改为“只有对象条目才有 `obj_id`” | websdk `src/nfsp.ts` |
| T3 | NFSP 文档 §3.3 写明 `ident.obj_id` 是 FileObject / DirObject 的 ID，后端拿不到时不返回；§5.2 的示例同步 | `cyfs-ndn/doc/NamedFileSystem_Protocol_v0.md` |

### 5.2 第二步：按需求对象 ID，带缓存

| 编号 | 内容 | 位置 |
| --- | --- | --- |
| T4 | 新方法（建议叫 `snapshot`，见 Q1）：输入一个文件或目录的定位，输出 `{ obj_id, kind, size, item_count, source_changed, excluded }`。要求调用方对源有 `read`；目录逐项查 ACL，无权的项进 `excluded` | nfs 新增，复用 `share/snapshot.rs` 的 `share_plan_source` 与 `run_snapshot` |
| T5 | **缓存。** 一张可全量重建的缓存表，记录“节点、大小、修改时间、名字”对应的内容 ID 和 FileObject ID。命中条件见下 | nfs `filedb.rs`，做法同 `preview_inputs` |
| T6 | 把结果授予调用方：在 `ndm_holdings` 里登记“调用方的用户 + App 持有该对象”，使 `may_read` 通过。缓存命中时同样要登记 | nfs `ndm/mod.rs` |
| T7 | 保留：nfs_server 自己在一段时间内持有这个对象，保证调用方来得及把它绑到自己的业务上（安装器 staging、消息、桶）。机制和时长见 Q2。缓存命中时续期 | nfs，参考 `share/mod.rs` 的 `retain` / `release` |
| T8 | 大内容不阻塞请求：超过阈值时返回可查询进度、可取消的操作。分享模块已有同样的模式（`share.operation`） | nfs |
| T9 | `hello` 的能力列表里广告这个方法；websdk `NfspClient` 加对应调用 | nfs `server.rs`，websdk `src/nfsp.ts` |
| T10 | 文件选择器的 `ensureObjId` 对 Zone 里的文件调用这个方法 | websdk，见文件选择器设计稿 §4.3 |

**缓存的规则（T5）。**

- **命中**：同一个节点，大小和修改时间都没变，名字没变，并且缓存里的对象仍在 NamedStore（`share/snapshot.rs:435 is_stored`）。命中时不读文件内容，只做一次 `stat`。
- **只改了名字**：内容 ID 还有效，不重新分块和存储。名字和由扩展名推断的 mime 都在 FileObject 里，所以按新名字重新探测 meta（只读文件头）、重建 FileObject，得到新的对象 ID。
- **大小或修改时间变了**：重新读文件、分块、存储。
- **对象已被回收**（保留到期后被 GC）：按未命中处理，重新存储。因为构造是确定的，得到的 ID 和上次相同。
- **目录不单独缓存。** 目录快照里的每个文件走同一张缓存，没变的文件不重读，只重建 DirObject。分享模块的 `SourcePlan.fingerprint` 只含路径、总大小、项数、最大修改时间和排除数，不足以当作目录内容没变的依据，不要用它判断命中。
- 缓存是 nfs_server 的内部状态，不经 `ident` 对外暴露。
- 判据的盲区和 nfs_server 其他地方一样：内容变了但大小和修改时间都没变的文件会被当作没变（技术需求 §5.2 的已知边界）。

**其他说明。**

- 用 Store 模式，字节会在 NamedStore 里再存一份（Q3）。对同一个文件，`snapshot` 和分享得到的是同一个 FileObject 和同一批块，不会存两次。
- App 和 Agent 的会话现在进不了文件面（`auth.rs` 的 `require_file_actor`），这个方法同样只对用户会话开放。
- 快照是对象不可变，不是路径不可变。不给源路径发 `immutable` 缓存头，也不广告 Frozen（技术需求 §4.2 的降级契约不变）。

### 5.3 文档

| 编号 | 内容 | 位置 |
| --- | --- | --- |
| T11 | 更新 README 的能力表；技术需求 §3.6 里“重算完整 hash，产生新 obj_id”改为“产生新的整文件摘要”，§5.1 注明 commit 路径的 link 登记推迟到 fs_meta | `src/frame/nfs_server/README.md`、`product/bucky_file/nfs_server.md` |

## 6. 待裁决

| 编号 | 问题 | 建议 |
| --- | --- | --- |
| Q1 | 新方法叫什么 | NFSP 的 `snapshot`。协议里已有的 `publish_dir` 语义相近但只管目录，而且“发布”在产品里另有含义 |
| Q2 | 快照由谁保留、保留多久 | 绑进一个 Zone 桶（仿 `zone/nfs-server/_/shares`），带到期时间，默认 24 小时，每次调用续期。调用方要长期用就自己持有 |
| Q3 | 用 Store 模式还是 link 模式 | Store。代码现成，对象不会因为源文件被改而读不出来；代价是多占一份空间。link 模式属于裁决 1 推迟的那条路 |
| Q4 | 对象桶投影文件的 `obj_id` 是否保留 | 保留。它们本来就是对象，ID 真实且零成本，符合裁决 1 的原则。如果“都不返回”包括它们，T1 改为全部去掉 |

## 7. 约束

- 开发阶段不做兼容：字段去掉后不保留旧行为。
- 改协议字段要同时检查 nfs_server、websdk 类型、Desktop、测试和文档（AGENTS 的联动规则）。websdk 的 `dist/` 纳入了 Git，改完源码要重新构建。
- `snapshot` 返回的 ID 必须读得到、调用方有读权；做不到就报错，不返回读不到的 ID。
- 无权限的源和不存在的源，对调用方的表现必须一致（现有 ACL 约定）。
- 不新增依赖。

## 8. 验证

- `cargo test -p nfs_server -- --test-threads=1`，新增用例：
  - `stat` / `list` 的 `ident`：普通文件没有 `obj_id`；对象桶投影文件仍有。
  - `snapshot` 一个文件：返回的 ID 是 `cyfile:`，NamedStore 里能读出对象和全部内容；调用方能通过 `may_read`，其他用户不能。
  - 缓存：同一文件再次 `snapshot` 返回同一个 ID，且没有读文件内容（用读计数或把源文件换成不可读来验证）；改内容后返回新 ID；只改名后返回新 ID，没有重新分块，块不重存。
  - 对象被回收后再次 `snapshot`：重新存储，ID 与之前相同。
  - 第二个用户对同一文件 `snapshot`：命中缓存，并且他也能读到对象。
  - 无读权的文件返回和不存在一样的错误；目录里无权的子项出现在 `excluded`，不在对象里。
  - 目录里改了一个文件后再次 `snapshot`：只重读这一个文件。
  - 超过 32 MiB 的文件得到 ChunkList 内容。
- 跨实现一致性：同一个文件（一张带 EXIF 的图片、一个超过 32 MiB 的文件）分别用 `snapshot` 和浏览器 `ndm.importFiles` 得到 ID，两者必须相同。不同就是 FileObject 构造分叉，按 websdk 标准对象 TODO 的同步约定修。
- 联调：用 `snapshot` 得到的 ID 调 `apps.staging.finalize { source_obj_id }`，能通过来源校验并完成检查。
- Desktop 预览在 T1 之后行为不变，普通文件的信息面板不再有“Object ID”一行（`pnpm test:e2e` 的预览用例）。
- 未在真实 Zone 验证的项如实记录。

## 9. 本次不做

- commit 时以 link 模式把内容登记进 NamedStore、让普通文件默认带对象 ID（裁决 1）。README“后续接入”第 2 项保持原样。
- fs_meta 及“每个文件天然有对象 ID”的底层。
- 给 App / Agent 会话开放文件面。
- 把预览管线的输入版本从整文件摘要换成对象 ID。
- Frozen、`publish_dir` 的完整语义。

## 10. 验收

- `ident.obj_id` 在任何响应里都不再出现 `sha256:` 形式的值；普通文件没有这个字段。
- 对一个有读权的 Zone 文件，一次 `snapshot` 调用得到 `cyfile:` ID，安装器和消息附件能直接使用，内容不经过浏览器。
- 对同一个没变的文件第二次调用，不读文件内容。
