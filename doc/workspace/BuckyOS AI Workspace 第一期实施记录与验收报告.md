# BuckyOS AI Workspace 第一期实施记录与验收报告

> 对应：[《第一期核心架构设计与验证》](<BuckyOS AI Workspace 第一期核心架构设计与验证.md>) v0.2（下称“第一期文档”）与[《第一期内置对象详细设计》](<BuckyOS AI Workspace 第一期内置对象详细设计.md>)（下称“详细设计”）。
>
> 读者：审查第一期交付、或在其上继续开发的人。
>
> 本文只记录**已经运行过的事实**：每一项结论后面是产生它的命令或测试。没有运行过的，写在 §6“未完成与未验证”。

## 1. 结论

- **后台内核已按正式实现完成并通过验收测试**：`src/frame/aiworkspace` 的 `core`（纯逻辑，可编译为 WASM）、`store`（SQLite 与对象存储）、`server`（独立进程与服务协议）。V01–V14、V18–V25 中属于后台的部分全部有自动化测试，共 58 个测试通过，其中 2 个针对真实进程（含三个位置的崩溃恢复）。
- **系统接入点已改完并能编译**（服务规格、RBAC、构建清单、启停脚本、端口表），scheduler 的启动配置测试通过。**没有在真实 Zone 中启动验证**。
- **Desktop 应用与浏览器离线已完成并通过端到端测试**：32 个 Playwright 用例对真实后台进程运行通过，覆盖 V01、V06–V08、V11、V13、V15–V18、V23、V25 的浏览器部分。只在 Chromium 上验证；系统模式下的传输只通过了类型检查。详见 §5。
- 第一期文档 §10 的七项门槛逐项对照见 §7。

## 2. 交付物

| 交付物 | 位置 |
| --- | --- |
| 内核、存储、服务、WASM 门面 | `src/frame/aiworkspace/{core,store,server,wasm}`，说明见其 `README.md` |
| 富文本 schema（前后端唯一来源） | `src/frame/aiworkspace/schemas/richtext.basic.v1.json` |
| 协议目录（与代码有一致性测试） | `schemas/operations.json`、`schemas/error-codes.json`、`schemas/commit-request.schema.json` |
| 贯穿样例（Commit 序列） | `fixtures/project-workspace/commits.json` |
| 跨语言向量 | `fixtures/vectors/object-ids.json` |
| 系统接入 | `src/Cargo.toml`、`src/bucky_project.yaml`、`kernel/buckyos-api/src/{aiworkspace_client.rs,lib.rs,rbac_config.rs,node_control.rs}`、`kernel/scheduler/src/{system_config_builder.rs,main.rs}`、`stop.py`、`rootfs/bin/{stop.py,stop.ps1,stop_osx.sh,aiworkspace/kernel_pkg.toml}`、`.gitignore`、`doc/port_usage.md` |
| 契约回写 | 第一期文档 §11；详细设计附录“决策记录”2026-10-05 各行 |

## 3. M1 必须回答的八个问题

| 问题 | 答案 | 证据 |
| --- | --- | --- |
| 哪些数据由后台权威管理，哪些是客户端副本/投影 | 权威：`doc.sqlite` 的全部表（实体、树边、字段、记录、富文本快照与更新、提交历史）。投影：`richtext_states.ast_json`（由后台解码得到）、`refs`、`entity_versions`。客户端：确认层副本 + 待提交队列 + 私有草稿，均不是权威 | `store/src/schema.rs`；`diag.verify_refs`（引用索引全量重建与增量结果相等） |
| 服务协议和身份边界 | kRPC `/kapi/aiworkspace`，业务结果在 `result`。身份只来自服务验证过的会话令牌（系统模式：verify-hub 令牌；独立模式：静态令牌表），请求体中的任何身份字段不被读取 | `server/src/auth.rs`；V23 用例中“请求体自称 principal 无效” |
| 在哪里判定事务成功 | 一个 Workspace 一个写执行者；规划在覆盖层上完成后，数据、引用、历史行（即幂等记录）、`head_seq` 在**一个 SQLite 事务**内写入。事务提交即成功；之后才换入内存 CRDT 文档并唤醒等待者 | `store/src/workspace.rs::commit_inner`；V10、崩溃恢复 |
| 富文本引擎如何在 Rust 与浏览器互操作 | 两端是同一个 Loro 内核；容器布局成文（`pm-loro-1`）。Rust 端解码、校验、并能写（块级操作、建文档） | `core/src/richtext.rs` 及其测试；浏览器方向见 §5 |
| 草稿怎么隔离 | 草稿只在客户端。即时策略提交 CRDT 更新；显式策略在私有分叉上编辑，提交时做块级差分，只发送块级语义操作 | V14 草稿用例：四次私有修改 → 一个 `replace_block`，变化流与协作状态中搜不到中间稿 |
| 普通导出是否新建 CRDT lineage | 是。分享包只含内容对象（规范 AST），导入方 `build_doc` 建新 lineage；个人恢复备份另带 `collab/*.loro` 并沿用原 lineage | V06、V14：分享包中搜不到已删除文字；备份恢复后旧离线增量仍可合并 |
| 离线采用什么 VFS/锁 | `opfs-sahpool` + Web Locks 单持有者，见 §5.2 | V15、V16 |
| NamedObject 如何在两端得到相同 ID | 同一份 Rust 代码（严格预校验 + JCS + SHA-256）原生与 WASM 各编译一次；与 ndn-lib 比对 | V01：`store/tests/kernel.rs::v01_object_ids_match_ndn_lib`；Node 中加载 WASM 跑 `fixtures/vectors/object-ids.json` 12 条全部一致 |

## 4. 验收矩阵（后台）

全部命令在 `buckyos/src` 下执行。

```bash
cargo test -p aiworkspace-core -p aiworkspace-store -p aiworkspace
```

2026-10-05 结果：`core` 19 + 7 + 3 + 1、`store` 6 + 12 + 6 + 2（另 1 个性能探测默认忽略）、`server` 2，**58 通过，0 失败**。

| 编号 | 覆盖的断言（摘要） | 测试 |
| --- | --- | --- |
| V01 | 同输入得到与 ndn-lib 相同的 canonical 文本、`jobj`/`cyfile` ObjectId、`mix256` chunk id；重复键、超 2^53 整数、负零、非有限数被拒绝；篡改内容校验失败；非 NFC 字符串原样进入哈希 | `store/tests/kernel.rs::v01_*`、`core/tests/vectors.rs`、`core/src/canonical.rs` 单测 |
| V02 | 改名、移动、建组后，除根以外所有内容对象 ObjectId 不变，仅结构索引变化；`fixed_revision` 在实体变化后仍解析到原内容；`published_channel` 返回 `UNSUPPORTED_VERSION`，不降级 | `kernel.rs::v02_v08_*` |
| V03 | 悬空父节点、不允许的子类型、非法 ID、同批成环、悬空引用、超限批次整批拒绝且不留痕迹；删除被引用源返回引用方列表 | `kernel.rs::v03_*`、`core/tests/planner.rs::tree_rules_*` |
| V04 | 导出后在另一部署恢复，ContentRoot 相等；导出后的修改不在包内；历史不随包；覆盖恢复须 `replace` + 对现有 Workspace 的 `manage`；恢复生成新 `epoch`；对象被篡改的包被拒绝且不出现任何 Workspace | `packages.rs::v04_*` |
| V05 | Fork 完成时 ContentRoot 与来源相等，之后各自分叉；不复制授权、个人注释；富文本为新 lineage；仅有 `export` 的人可以 Fork 自己读得到的内容 | `packages.rs::v05_*` |
| V06 | 重启后协作状态与 AST 投影一致并可继续编辑；个人备份恢复后原 lineage 的离线增量仍可合并；分享包恢复后同一增量得到 `LINEAGE_MISMATCH` | `collab.rs::v06_*`、`richtext_storage_compaction` |
| V07 | 两端离线编辑以两种顺序到达结果相同，重复提交不改变结果；缺依赖更新 `BASE_UNKNOWN` 且不改变任何状态；并发产生重复 `block_id` 时后到者被拒绝、权威状态不变；客户端 UndoManager 只撤销自己的编辑 | `collab.rs::v07_*` |
| V08 | 字段改名、插行、保存排序后，单元格选择器与注释仍指向原 record/field；同源两视图共享记录、配置独立；失效过滤使查询返回 `VIEW_BROKEN`，失效列被跳过并报告，撤销删除后自动恢复有效 | `kernel.rs::v02_v08_*`、`dangling_view_configuration`、`fixture_replays_*` |
| V09 | 值类型规范化与三种“空”往返；迁移预检报告；有失败且 `reject` 时无任何单元格变化；事务内注入失败后 ContentRoot 不变；旧 `type_rev` 的写入得到 `SCHEMA_CONFLICT`；迁移可整体撤销 | `collab.rs::v09_*`、`planner.rs::field_delete_restore_and_migration`、`core/src/value.rs` 单测 |
| V10 | 树 + 富文本 + 表格同批：校验失败、事务前失败、事务中失败三种情况下，`head_seq`、协作快照字节、ContentRoot、变化流条数均不变 | `kernel.rs::v10_*`、`planner.rs::mixed_batch_*` |
| V11 | 同记录异字段都成功；同字段旧基准冲突并返回当前值；改回原值仍冲突；一批内全部冲突一次报告；删除后迟到写入 `TARGET_DELETED` 且不复活；单元格写入只写一行记录 | `kernel.rs::v11_*`、`planner.rs::cells_conflict_*` |
| V12 | 已落盘但响应丢失：重开后 `get_submission` 为 `accepted`，原键重发 `replayed: true` 且不撞自己的 `ID_CONFLICT`；同键不同内容 `IDEMPOTENCY_MISMATCH`；`rejected` 不被记录 | `kernel.rs::v12_*`、`process.rs::crash_recovery_*` |
| V13 | 撤销、重做（撤销补偿提交）；他人改过其中一格后默认整体冲突并给出可撤销部分；`partial` 须带 `plan_digest`；非作者无 `manage` 不能撤销；编辑器会话的文本更新 `NOT_UNDOABLE` | `kernel.rs::v13_*`、`collab.rs::v07_*` |
| V14 | 分享包中搜不到已删除文字与他人个人注释；副本库原始字节中搜不到他人个人注释的任何历史版本，且不含 `commits`/`commit_ops`；已删除字段的残留值不进副本、不进导出 | `packages.rs::v14_*`、`collab.rs::v14_*` |
| V18 | 候选是数据；无关字段修改不使候选失效；三种读集合变化（改 `due`、新增未完成任务、完成改回未完成）均 `REVISION_CONFLICT` 且无任何应用；人工覆盖被跳过并告警；人工改写生成块后应用冲突；三种非法候选整批拒绝；取消后直接提交候选也 `RUN_CANCELLED`；未持锁 `LOCK_REQUIRED` | `collab.rs::v18_*` |
| V19 | 事件按 `seq` 连续；类别 `view`/`moved` 不使数据结果过期，`value` 使其过期；他人个人注释对我是只含 `seq` 的空事件；幂等键只回给作者；`epoch` 不符即 `EPOCH_MISMATCH` | `kernel.rs::v19_*` |
| V20 | 未知类型与更高 schema 版本：原样保留、只读、可移动；内容操作与新建被拒绝；导出再导入后原始 payload 不变；未知存储版本拒绝打开且文件字节不变 | `packages.rs::v20_*` |
| V21 | 无授权者看到的是 `NOT_FOUND`；只读拒写；append-only 可追加，响应只含自己插入的记录，不能读、不能查、不能改、变化流无内容、不能准备副本；子树授权只见子树 | `kernel.rs::v21_*`、`planner.rs::permissions_*` |
| V22 | 引用未上传对象 `DEPENDENCY_UNAVAILABLE` 且无悬空实体；媒体类型与大小取自已校验内容；被历史引用的旧图片不在可回收清单；过期且从未被引用的上传在清单内；篡改为 `corrupt`、移除为 `missing`，文档仍可打开；自包含导出如实报告缺失 | `packages.rs::v22_*` |
| V23（后台） | 独立进程仅经 HTTP 完成：建库、重放样例、读、查、冲突、资产下载、长轮询唤醒、Mock、URL 表、撤销、导出下载、另一部署导入、Fork、副本库下载；无令牌/错令牌进不了任何方法；**进程重启后已接受提交仍在** | `server/tests/process.rs::v23_*` |
| V24 | 保存定义与视图不取数；按页查询只生成所需行（百万行源，生成行数 < 100）；缓存按查询与主体隔离；源不能下推的过滤被拒绝；URL 源只读；无快照/无行键的源不被赋予这些能力；未注册 scheme 报不可用；导出保留定义、`self_contained: false`、导入不联网、ContentRoot 往返相等；断网不把未取得内容当作可用；只物化所选切片 | `packages.rs::v24_*` |
| V25 | 写锁（第一期文档 §11-9 的全部条目） | `kernel.rs::write_locks`、`collab.rs::v18_*` 第 8 步、`replica.rs` 第 4 步 |
| 崩溃恢复 | 提交混合批次时在事务前、事务中、落盘后应答前分别 `abort()`；重启后前两处无任何痕迹、第三处完整存在；原键重发恰好生效一次；内存 CRDT 文档与库一致 | `process.rs::crash_recovery_at_three_positions` |
| 离线引擎（V15/V17 的内核部分） | 离线编辑、链式依赖占位、关闭重开后由“确认层 + 待提交”重建工作视图；追赶后确认层 ContentRoot 与后台相等；同格冲突与目标被删的待提交转为冲突并保留；响应丢失按键识别不重复应用；权限撤回后输入保留；`lock_required` 对象不可离线编辑；恢复换代次后旧待提交不被重放 | `store/tests/replica.rs` |

## 5. Desktop 应用与浏览器离线

### 5.1 Desktop 应用（在线）

位置：`src/frame/desktop/src/app/aiworkspace/`（约 5100 行 TS/TSX，结构与运行方式见其 `README.md`）。已按详细设计 §8.4 的六处完成内置应用注册（应用 ID `aiworkspace`）。新增依赖：`loro-crdt` 1.16.4、`loro-prosemirror` 0.4.4、`prosemirror-{model 1.25.12, state 1.4.4, view 1.42.6, transform 1.12.2, keymap 1.2.3, commands 1.7.2, schema-list 1.5.1}`；未引入 Tiptap。ProseMirror `Schema` 由 `schemas/richtext.basic.v1.json` 生成。

验证（在 `src/frame/desktop` 下；测试启动**真实后台进程**与 Chromium，无 Mock 后台）：

```bash
pnpm check && pnpm build
PATH=<cargo bin>:$PATH pnpm exec playwright test --config=playwright.aiworkspace.config.ts
```

2026-10-05 结果：`tsc -b` 通过，构建通过，新代码 ESLint 0 问题；Playwright **18 通过**（由实现者运行两次、由本文作者独立复跑一次）。

| 验收 | 覆盖 |
| --- | --- |
| V23（前端） | 从桌面壳打开应用、重放样例（13 个实体）、所有读写经真实服务协议 |
| V01（浏览器） | 浏览器内经 WASM 内核跑 `fixtures/vectors/object-ids.json`：合法项 canonical 文本与 ObjectId 一致，非法项抛错，值规范化一致 |
| V06 | 中文组合输入（CDP 模拟 IME）写入富文本，刷新后仍在；后台解码的 AST 等于编辑器文档经 `richtext_canonicalize` 的结果 |
| V07 / V11 | 两个浏览器上下文（两个用户）：富文本并发编辑收敛；同记录异字段都成功；同字段后到者看到冲突界面（含对方的值）且自己的输入保留 |
| V08 | 单元格修改后“全部任务”“未完成任务”两个视图同时更新；会话筛选与“保存视图”分离；失效视图诊断 |
| V13 | Ctrl+Z 每次只撤销一步：先是编辑器一步，再是一次单元格提交，按栈序 |
| V18 | Mock 运行：候选、应用、模拟标注、整体撤销 |
| V25 | 写锁：开启策略、申请、持有者显示、强制解除、`LOCK_LOST` 后输入保留、重新申请 |
| §8.3 渲染 | 1 万行表滚动时行 DOM 数量有界 |
| 其他 | 未知结果经 `doc.get_submission` 与同键重发恰好生效一次；被拒绝的富文本更新保留为草稿并回到确认内容；导出/导入/Fork 与恢复后的 `EPOCH_MISMATCH`；粘贴总是新 `block_id`、合并保留前块 ID；后台不可达时如实显示 |

探针结论（详细设计 §3.3.1、§3.3.3）：行内原子节点是 `children` 中独立的 `LoroMap`（已回写详细设计）；npm `loro-crdt` 1.16.4 与 Rust `loro` 1.16.2 双向互操作（Rust 建的文档与后台块级操作产生的更新在浏览器导入无缺依赖，浏览器产生的更新被后台接受且 AST 一致；打开 Rust 建的文档不产生任何操作）。未启用 yrs 退路。

实施中由前端测试发现并修复的两处后台缺陷：`build_doc` 逐段标记时扩展型 mark 泄漏到后续文本（改为整段插入后再标记，测试 `build_keeps_mark_boundaries`）；替换资产对象后残留旧的 `image` 尺寸且派生键版本未更新（`packages.rs::v22_assets` 新增断言）。

在线应用**没有做**的部分（界面如实不提供，未伪装）：

- 系统模式下的传输（`buckyos.getServiceRpcClient('aiworkspace')` + 会话令牌）只通过了类型检查，全部测试走独立后台的开发覆盖项。
- 只实现 `immediate` 提交策略；显式草稿的界面未做（内核与后台的块级差分提交已有，V14）。
- 变化通知只有长轮询，未接 kevent；锁持有者每 5 秒随大纲刷新。
- 版面只渲染第一个页面的 `flow` 布局，忽略 `placement`；重排用按钮，无拖拽。
- 表格：无分组、手工顺序、列宽调整；筛选编辑器只能构造单个条件；`object_ref` 值只显示不可编辑；URL 查询表没有专门界面。
- 记录 schema 不可编辑；注释只能创建/删除、总是共享、类型 `note`。
- 没有授权、检查点、`diag.*` 的界面；撤销没有“部分撤销”界面（冲突时如实报告且不执行）。
- 富文本没有链接地址与有序列表起始值的界面、没有远端光标；真实操作系统输入法未测（只测了 CDP 模拟的组合输入）。
- 锁在焦点离开 30 秒后释放而不是失焦即释放；没有“请求交接”；20 秒续约路径未被测试覆盖。
- 应用内文案只有中文。

### 5.2 浏览器离线

位置：`src/frame/desktop/src/app/aiworkspace/offline/`（Replica Worker、持有者锁、副本会话）与 `src/frame/desktop/src/{serviceWorker.ts,service-worker/}`。新增依赖 `@sqlite.org/sqlite-wasm` 3.53.4-build2。

选型结论（ADR）：

| 问题 | 结论 | 证据 |
| --- | --- | --- |
| VFS | 官方 SQLite WASM 的 **`opfs-sahpool`**，运行在专用 Worker；每个 Workspace 一个副本库（确认层各表 + `pending_submissions`、`drafts`、`replica_meta`）。不提供 IndexedDB 回退，不提供纯内存“保存” | V15 用例断言 `crossOriginIsolated === false` 且副本可用：不需要 COOP/COEP 头 |
| 写入者 | `navigator.locks` 独占锁 `aiworkspace-replica:<workspace_id>`；拿不到锁的窗口不启动 Worker，在线直连并标明“此窗口未启用离线”，后台不可达时只读；持有者关闭后经用户操作接管 | V16 第二标签页用例 |
| 引擎 | Worker 内运行 WASM 内核的 `Replica`（与后台同一份规划代码）；SQLite 是它的持久形态。本地修改在插入 `pending_submissions` 行的事务提交后才显示“已保存到本设备”；确认层推进、`confirmed_seq`、待提交行的增删在一个事务内 | `core/src/replica.rs`、`store/tests/replica.rs`、V16 存储故障用例 |
| 应用资源 | Service Worker 预缓存入口、应用分包、两个 WASM 模块与 sqlite-wasm 文件，带版本缓存；`sw.js` 与 `index.html` 同级，作用域 `/` | V15：关闭页面、断网后冷启动成功 |
| 支持环境 | **仅在 Chromium 151（Playwright headless，Linux）上验证**。Firefox、Safari 未测。浏览器拒绝持久存储时显示“离线不可用”及原因 | `offline.spec.ts` |

验证：与 §5.1 同一条命令，2026-10-05 结果 **32 通过**（原 18 + 离线 14；实现者运行后由本文作者独立复跑）。离线用例使用真实后台进程、真实 Chromium OPFS、生产构建 + `vite preview`；断网通过切断每个用例专用的 TCP 中继实现，V15 另加 `context.setOffline(true)`。

| 验收 | 覆盖 |
| --- | --- |
| V15 | 准备离线 → 断网 → 改单元格并在富文本输入 → 关闭页面 → 仍断网时新开页面（经 Service Worker 冷启动）→ 内容与待提交队列都在 → 重连 → 经 kRPC 断言恰好被接受一次。未准备任何副本时离线打开、应用资源缺失时，均为明确状态而非白屏 |
| V16 | 第二标签页不是第二写入者，持有者关闭后才可接管并看到前者的待提交；注入事务失败、杀掉 Worker 后，不出现“已保存”而行不在盘上的情况，输入可导出，重开后恰好是已落盘的内容 |
| V17 | 离线期间他人删除记录、修改同一单元格、撤回权限、移除全部访问：重连后各本地输入进入“需要处理”且可读，无重复应用，被删记录不复活；响应丢失与请求未达两种未知结果按键定性，各恰好生效一次；历史被恢复替换（`EPOCH_MISMATCH`）后停止发送并保留输入 |
| 其他 | 离线撤销未发送的提交不经网络；Mock 与写锁在离线时如实说明需要后台；无 Workspace 级读权限者不能准备副本 |

与详细设计 §6 的差异：网络传输留在页面而不在 Worker（系统模式的会话在页面里）；没有 `preimage_json` 与 `richtext_working`，工作视图 = 确认层 + 重放（第一期文档 §11-12），离线富文本以待提交的 `richtext.apply_update` 行持久化；导出待处理输入为普通 JSON 下载，不是个人恢复备份的附加部分，也不能再导入。

离线部分**没有做或没有测**的：

- 离线添加/替换资产（`blocked_asset` 延迟上传）未实现，操作被明确拒绝。
- 冷启动只在桌面壳的 mock 运行时下验证；真实 Zone 中桌面壳自身的登录引导不具备离线能力，control-panel 静态服务上也未实际运行（从其源码判断无需额外响应头）。
- 真实配额耗尽未测（只测了注入的事务失败与 Worker 被杀）；Worker 死亡靠 10 秒调用超时发现。
- Service Worker 的更新流程、资产大小上限跳过清单、`BASE_TOO_OLD`、发送中途关闭标签页，均有实现但无测试。
- 从未准备副本的窗口在后台掉线时沿用在线行为（显示未保存并重试），不强制只读。

## 6. 性能探测

命令：`cargo test -p aiworkspace-store --release --test perf -- --ignored --nocapture`。

环境：AMD Ryzen 7 PRO 6850H（16 线程）、ext4、Linux 7.0、rustc 1.97、rusqlite 0.32.1（bundled SQLite，WAL，文档库 `synchronous=FULL`）、loro 1.16.2。fixture 由固定种子生成器经 Commit 灌入。这些是探测规模，不是容量承诺。

| 项目 | 预算 | 2026-10-05 实测 |
| --- | --- | --- |
| 1 万记录 × 20 字段：灌入（10 个提交，每个 1000 行） | — | 1.15 s，库 13.3 MiB |
| 单元格提交（含 fsync），只写 1 行记录 | p95 ≤ 20 ms | p50 6.6 ms，p95 7.4 ms |
| 1000 行批量修改，一个提交（写执行者临界区） | ≤ 300 ms | 106 ms |
| 撤销该批次 | ≤ 500 ms | 166 ms |
| 过滤 + 排序查询，每页 100（全表扫描基线，3306 行命中） | ≤ 200 ms | 首页 99 ms，翻页 99 ms |
| 无过滤查询，每页 100 | ≤ 200 ms | 127 ms |
| 全量物化（checkpoint） | ≤ 1 s | 311 ms；内容未变时再次物化 0.5 ms |
| 分享导出 | ≤ 1 s | 136 ms，包 0.8 MiB |
| 副本库构建 | ≤ 500 ms | 57 ms，5.7 MiB |
| 富文本 1000 块 / 8.6 万字符：建立 | — | 51 ms |
| 该文档上一次 `apply_update` 提交 | p95 ≤ 100 ms | p50 50 ms，p95 70 ms |
| 该文档上块级替换提交 | ≤ 100 ms | 52 ms |
| 一个提交创建 1000 个实体 | ≤ 300 ms | 82 ms |
| 1015 个实体的大纲 | ≤ 100 ms | 23 ms |
| 引用索引全量重建并比对 | — | 90 ms |
| 重新打开 Workspace / 首次加载大富文本 | ≤ 100 ms | 0.6 ms / 20 ms |
| 进程峰值 RSS | ≤ 256 MiB | 77 MiB |
| 322 个提交后的库大小（含 WAL） | — | 22.5 MiB |

对照第一期文档 §8.3 的三项“必须证明”：

- **单字段修改不需要整表重写**：`WriteStats.records == 1` 在 1 万行表上断言成立。
- **每次按键不需要整份 Workspace 导出**：`apply_update` 只追加一行更新并重写该文档自己的 AST 投影；物化只发生在 checkpoint/导出/Fork。
- **渲染不一次创建全部行 DOM**：属于前端，见 §5。

由测量得出的决定（详细设计 §10 末段列出的待定项）：

- 记录值索引：**暂不需要**。1 万行全表扫描 + Rust 过滤排序约 100 ms，在预算内；瓶颈是每行 JSON 解析与整页排序，规模再大一个量级时应先加按字段的表达式索引。
- 大批次规划移出写临界区：**暂不需要**（1000 行 106 ms）。
- 记录文件分桶：**暂不需要**（全量物化 311 ms，未变内容走缓存）。
- `synchronous`：保持 `FULL`（单元格提交 p95 7.4 ms）。
- 富文本大文档的提交成本（约 50 ms）来自候选分叉、全文解码校验与 AST 投影重写，与文档大小成正比。编辑反馈在本地、提交按 300–1000 ms 合并，本期可接受；增量校验留待有更大文档的数据后决定。

## 7. 第一期文档 §10 门槛对照

| 门槛 | 状态 |
| --- | --- |
| 1. 格式、NamedObject 映射、命令协议、错误码、内置类型 schema 有版本与可执行校验；示例包可独立打开 | 后台满足：`format_version`/`storage_schema_version`/`protocol_version`/`schema_version` 分别检查；协议目录有一致性测试；导出包在另一部署独立导入并校验。内置类型的 payload 校验是 Rust 代码（`core/src/types.rs`），**没有为每个类型单独发布 JSON Schema 文件** |
| 2. V01–V25 在明确环境中通过 | 满足：后台见 §4（58 通过），浏览器见 §5（32 通过，Chromium 151） |
| 3. 无已知数据丢失、草稿泄露、权限绕过、部分提交、无提示丢弃 | 后台无已知问题。实施中由测试发现并修复过一处泄露：副本库对已删除字段的残留值先插入后 UPDATE 清除，旧行字节留在空闲空间；现改为复制时过滤（`store/src/export.rs`） |
| 4. UI、Agent、类型编辑器与导入路径都用正式内核 | 后台满足：Mock、撤销、导入、Fork 都经同一个规划器与提交管线，没有第二个写入口 |
| 5. 恢复、普通分享、Fork 的语义与内容范围有区分 | 满足（V04、V05、V06、V14） |
| 6. 文档列明支持的环境与延期功能；未实现能力返回明确错误 | 见 §8；`published_channel`、未注册 URL scheme、超过 32 MiB 的资产、未知程序名均返回明确错误码 |
| 7. 以系统产物管理服务交付，Desktop 用同一协议 | 接入点已改、可编译；**未在真实 Zone 验证启动、鉴权与启停** |

## 8. 未完成与未验证

1. **未在真实 Zone/DV 环境运行系统模式**：`init_buckyos_api_runtime` → `login` → 令牌校验 → kevent 唤醒 → 网关 `/kapi/aiworkspace` 这条路径只通过了编译与 scheduler 配置测试。已有 Zone 不会自动获得新服务的 spec。
2. **未接入 Zone named store**：对象与资产在服务数据目录下的内容寻址目录中，没有 pin/Lease，不能经 NDN 获取。替换边界是 `store::objects::FsObjectStore`。
3. **资产与物化文件上限 32 MiB**（单 chunk），未写 ChunkList；详细设计中的 256 MiB 上限未达到。
4. **URL 查询源只有测试用的 `fixture://` 适配器**，没有接入任何真实数据源，也没有通用 HTTP 适配器。
5. **空闲 Workspace 不会自动关闭**（详细设计 §4.3）；打开过的 Workspace 一直持有连接。
6. **读与写共用一个连接和一把互斥锁**（详细设计建议读走独立只读连接）；并发读在写提交期间排队。
7. **内置类型没有逐个发布 JSON Schema 文件**（详细设计 M1-5）；已发布的是提交信封、操作目录、错误码表和富文本 schema。
8. 详细设计 §2.5.1 的 `TypeAdapter` trait 未按原样实现（见其附录）；第三方类型注册机制因此尚不存在，未知类型按降级规则保留。
9. `buckyos-build` 打包流程未实际执行；WASM 产物由 `wasm/build.sh` 生成，尚未接入统一构建。
10. Desktop 与离线层各自未做、未测的条目见 §5.1、§5.2 末尾；其中影响面最大的是：系统模式传输未经真实 Zone 验证、仅 Chromium、显式草稿策略无界面、离线不能添加资产。

## 修订记录

| 日期 | 内容 |
| --- | --- |
| 2026-10-05 | 初版：后台内核、服务、系统接入点、性能探测 |
| 2026-10-05 | 补 §5：Desktop 应用与浏览器离线的结果 |
