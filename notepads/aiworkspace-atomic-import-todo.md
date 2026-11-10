# AiWorkspace 导入原子化 TODO：一次导入 = 一个 Commit（分块暂存、一次提交）

日期：2026-11-10

状态：**M0–M5 已于 2026-11-10 实施（未提交）；§3 裁决项 owner 未另行裁决，按建议值实施。M6 没有在 DV Test 上做，改在本机用 release 后端跑了 dk10000 全书（§7.4）。** 实施结果、与方案的偏差和未验证项见 §7。

## 0. 起因与目标

2026-11-10 在 DV Test（test.buckyos.io）导入 `media_samples/dk10000.pdf`：建画布和第 1–3/7 批被接受，第 4 批被拒，画布上留下约 233 页，撤销栈里是 4 条。直接原因是 §1.5 的排序键溢出。更根本的问题是，导入本来就按设计分多个 Commit 提交（[PDF 方案](<../doc/workspace/BuckyOS AI Workspace PDF 导入与放映方案讨论.md>) §5：“每批 ≤ 2 000 操作”“任一批被拒绝：停止，报告说明已导入到第几页”“撤销按批”）。

**owner 裁决（11-10）**：导入是一次操作，要么整体成功，要么整体失败，撤销栈里只有一项。这是内核改动。

**怎么理解“队列里的 OP 原生支持 OP 列表”**：一个 Commit 本来就是一个 OP 列表，在一个 SQLite 事务里执行，也是最小撤销单位（[第一期内置对象详细设计](<../doc/workspace/BuckyOS AI Workspace 第一期内置对象详细设计.md>) §2.6 第 6 条、§2.7）。撤销栈的一项就是一个 Commit。所以不新增嵌套的“列表 OP”。缺的是：一个 Commit 必须装进一次请求（≤ 8 MiB、≤ 10 000 个操作）。本单让一个 Commit 可以分几次传到服务端，由服务端拼成完整请求后，照常在一个事务里执行。之后的撤销、重做、变化流、幂等记录全部沿用现有机制。

## 1. 现状（已对照源码）

### 1.1 一个 Commit 的上限

| 上限 | 值 | 位置 |
| --- | --- | --- |
| HTTP 请求体 | `max_commit_bytes`（8 MiB）+ 64 KiB | [server/src/lib.rs](../src/frame/aiworkspace/server/src/lib.rs) `Limits`、`build_router` 的 `DefaultBodyLimit` |
| 请求 JSON | `MAX_REQUEST_BYTES` 8 MiB（进程内调用也检查，包括 `doc.undo` 生成的补偿 Commit） | [store/src/workspace.rs](../src/frame/aiworkspace/store/src/workspace.rs) `commit_inner` 第 1 步 |
| 操作数 | `Limits.max_ops` 10 000（后台和浏览器副本共用） | [core/src/plan.rs](../src/frame/aiworkspace/core/src/plan.rs) `Limits`、`plan_commit` |

- 一个 Commit 是一个事务：`write_commit` 一次写入数据、`commits`、`commit_ops`（含逆操作）、`commit_deltas`（副本用的行增量，D20）和 head。
- 幂等：`request_digest = sha256(JCS(请求去掉 session_id 与 message))`（`CommitRequest::digest`）；只有 `accepted` 被记录。
- 撤销：`doc.undo` 读出目标 Commit 的全部逆操作，组成一个补偿 Commit 走同一条管线（`undo_inner`），因此同样受 8 MiB 和 10 000 的限制。`undo_group` 只存储，不参与撤销。
- 变化流：一个 Commit 对应一个事件（`get_changes`，带 `ops` 和 `rows`）。一页满 8 MiB 后停止，但至少返回一个完整事件。Desktop 直连会话默认也取 `ops`（`richtext/collab.ts`、`state/outline.ts` 会用到）。
- 后台暂存目前只有上传资产一种：`local.sqlite.staged_assets`，24 小时过期（`STAGED_ASSET_TTL_MS`），计入 `staged` 用量，由 `sweep` / `ws.clear_staged` 释放。第一期设计 §2.6 写的是“资产是唯一有后台暂存状态的东西”。

### 1.2 导入器现在都分多个 Commit

| 导入器 | 现在的提交方式 | 失败时 |
| --- | --- | --- |
| PDF（[pdfImport.ts](../src/frame/desktop/src/app/aiworkspace/ui/canvas/pdfImport.ts)） | 画布+文件夹+原件 1 个 → 页面按 `batches()` 每 2 000 个 → 路径 1 个 | 停在中途，前面的留下 |
| Office 幻灯片 / docx（[officeImport.ts](../src/frame/desktop/src/app/aiworkspace/ui/import/officeImport.ts)） | 同上；docx 用 `sizedBatches`（2 000 个 / 6 MiB） | 同上 |
| draw.io（[drawioImport.ts](../src/frame/desktop/src/app/aiworkspace/ui/canvas/drawioImport.ts)） | 新画布 1 个 → 每页对象、连线分别 `batches()` | 同上 |
| 表格（[tableImport.ts](../src/frame/desktop/src/app/aiworkspace/ui/import/tableImport.ts)、[import-table](../src/frame/aiworkspace/import-table/operations.ts)） | 每个 Commit ≤ 20 000 行 / 6 MiB（`ROWS_PER_COMMIT`、`COMMIT_BYTES`） | “已导入 x / y 行” |
| Agent 工具（[import-office/tools/import.ts](../src/frame/aiworkspace/import-office/tools/import.ts)、[import-table/tools/import.ts](../src/frame/aiworkspace/import-table/tools/import.ts)，经 websdk `AiWorkspaceClient.commit`） | 同上 | 同上 |

Desktop 的 `store.submit` 一次调用产生一个 Commit，并在撤销栈里推一项（[state/store.ts](../src/frame/desktop/src/app/aiworkspace/state/store.ts)、[state/undo.ts](../src/frame/desktop/src/app/aiworkspace/state/undo.ts)）。分 N 批就是 N 项。

### 1.3 发送路径

- 直连会话：`WorkspaceSession.commit` → `deliver()` → `AiwsClient.commit`（[api/client.ts](../src/frame/desktop/src/app/aiworkspace/api/client.ts)）。传输失败时先 `get_submission`，`not_found` 再原样重发（最多 3 次），否则结果未知。
- 浏览器副本：`submit_local` 先在 Worker 里用同一份内核本地规划，整个请求存进待发队列；发送循环逐个调用 `this.client.commit(request)`（[offline/replicaSession.ts](../src/frame/desktop/src/app/aiworkspace/offline/replicaSession.ts)）。
- Agent / Deno 工具：websdk [aiworkspace_client.ts](../../buckyos-websdk/src/aiworkspace_client.ts) 的 `commit(workspace_id, request)`。

这三条路径最后都落在两个 TS 函数上：Desktop `AiwsClient.commit` 和 websdk `AiWorkspaceClient.commit`。

### 1.4 实测（11-10，release，本机）

样本取自 DV 上被接受的三批真实 PDF 页面操作（去掉了需要已上传资产的图片项），在单个 Workspace 内复制成 1–3 份，各提交一次，再取变化流、撤销：

| 操作数 | 请求 | 提交 | 撤销 | `get_changes` 一个事件 |
| --- | --- | --- | --- | --- |
| 3 242 | 2.3 MB | 0.51 s | 0.37 s | 6.0 MB，0.11 s |
| 6 484 | 4.6 MB | 1.04 s | 0.75 s | 12.0 MB，0.23 s |
| 9 726 | 6.9 MB | 1.58 s | 1.15 s | 18.1 MB，0.35 s |

- 耗时随操作数线性增长：提交约 0.16 ms/个，撤销约 0.12 ms/个，两轮结果一致。提交期间该 Workspace 的写锁一直被占用。
- 变化流事件约为请求的 2.6 倍（`ops` 加 `rows` 增量），是大 Commit 的主要成本。
- 真实 PDF 导入的操作平均约 0.55 KB（含图片项）。dk10000 全书 806 页按前 233 页的密度（约 26 个操作/页）估算，约 2.1 万个操作、11–12 MB：提交约 3.4 s，撤销约 2.5 s，变化流事件约 30 MB。

### 1.5 前置修复（11-10，未提交）

导入器逐页调用 `keysAfter(上一页最后一个键)`，键约每两页长一个字符。带图页面过了约 130 页，内容文件夹的键就超过内核上限 64（`order_key.rs` 的 `MAX_KEY_LEN`）；任意页面过了约 321 页，Surface 层的键也超。超长后服务端报 `INVALID_OPERATION invalid order_key`。已改为整次导入在每个父级下只分配一次键：`import-drawio/operations.ts` 的 `allocateKeys` + `BuildTarget.keys`，PDF、Office（界面版和工具版）都已改用，并加了 800 页的回归用例。本单的“一次提交”依赖这个修复。

## 2. 方案

### 2.1 协议

**新方法 `doc.stage`**（stable）：

```
doc.stage { workspace_id, epoch, idempotency_key, part, operations }
  → { ok: true, part, digest, operations: <个数>, expires_at }
```

- 把一个 Commit 的第 `part` 块（从 0 起）暂存到服务端，按 `(调用者, idempotency_key, part)` 存放。同一块重发会覆盖，因此可以安全重试。
- `digest = sha256(JCS(operations))`，由服务端计算并返回，客户端只需原样带回，不必在 TS 里实现 JCS。
- 不做逐操作检查（权限、写锁、Schema），这些都留到最终提交。

**`doc.commit` / `doc.prepare` 增加 `staged`**：

```
doc.commit { ...原请求字段, staged: ["<digest of part 0>", "<digest of part 1>", ...], operations: [最后一块] }
```

- 服务端按顺序拼接：暂存块 0..N-1 + 本次 `operations`，得到完整请求，再走原来的 `commit_inner`。之后的一切不变：一个事务、一个 `commit_id`、一组 `commit_ops`、一个变化流事件、一个撤销单位。
- `request_digest` 仍按 §2.6 的规则计算，对象是最终请求（含 `staged` 摘要列表）。因此内容绑定在幂等键上：同键同内容会重放，同键不同内容报 `IDEMPOTENCY_MISMATCH`。成功后暂存块已被删除，重试最终请求依然能靠 `commits` 表重放。
- 校验：列出的块必须全部存在且摘要一致。块缺失或已过期时，用 `SNAPSHOT_EXPIRED`（可重试）加 sub_code `STAGED_PARTS_MISSING` 拒收。摘要不一致报 `IDEMPOTENCY_MISMATCH`。拼装后超出 §2.2 的上限报 `LIMIT_EXCEEDED`。不新增错误码。
- 消费规则：最终 `doc.commit` 一旦完成求值，无论结果是 accepted、conflict 还是 rejected，该键的暂存块都被删除。重放时也删除。`doc.prepare` 只拼装、不消费。

**为什么不选另外两种**：直接放大请求体上限，会让一个 30 MB 的请求整个穿过网关，没有分块重试，而且所有 kRPC 方法共用这个上限。把操作列表当资产上传，要依赖 NDM 导入、对象授权和 32 MiB 资产上限，提交路径会多出一整套上传机制。

### 2.2 服务端

- `local.sqlite` 新表 `staged_commits(principal, idem_key, part, ops_json, bytes, digest, expires_at, PRIMARY KEY(principal, idem_key, part))`。写入时 `check_quota(bytes)` 并 `add_usage("staged", +bytes)`；删除时扣回。`sweep` 清理过期块，`ws.clear_staged` 一并清除。Commit 成功后的删除放在 `write_commit` 之后，与现在 `staged_assets` 的处理方式相同。
- `doc.stage` 的准入与 `doc.commit` 第 1 步相同：`workspace_id`、`epoch`、有访问权、配额、演示锁、已结束的 session（Q3）。
- 上限分成两层：
  - **单次请求**：不变，仍是 `max_commit_bytes` 8 MiB（HTTP 层），`doc.stage` 和 `doc.commit` 都受它约束。
  - **拼装后的 Commit**：新增 `Limits.max_commit_total_bytes`（Q1 建议 32 MiB；与 server 的 `Limits.max_commit_bytes` 区分，后者仍是单次请求上限），`Limits.max_ops` 从 10 000 改为 50 000（Q1）。`commit_inner` 里的 `MAX_REQUEST_BYTES` 检查改用这个值，补偿 Commit（撤销 / 重做）也按它判断。浏览器副本的 `submit_local` 用同一组 `Limits`，在本地也检查。
  - 配额检查用拼装后的大小，扣除该键已计入 `staged` 的部分，避免重复计算。
- 变化流、`commit_deltas`、副本的 `apply_remote`：格式不变。一个最大的 Commit 事件约 80 MB（§1.4 的 2.6 倍），一页只放它一个。

### 2.3 客户端（TS）

- websdk `aiworkspace_client.ts` 导出一个函数（Q5），例如 `commitWithParts(call, request, { partBytes = 6 MiB, onPart? })`：
  - 请求序列化后不超过 `partBytes`：照旧调用一次 `doc.commit`。现有调用大多走这条路，行为不变。
  - 超过：按字节把 `operations` 切块，前 N−1 块依次 `doc.stage`，最后一块随 `doc.commit { staged }` 发出。每次发送都重新暂存全部块，所以 `deliver()` 的“`not_found` → 原样重发”不需要改。遇到 `STAGED_PARTS_MISSING` 时，整体重新暂存一次。
  - `AiWorkspaceClient.commit` 和 Desktop `AiwsClient.commit` 都改为调用它。Desktop 已依赖 websdk（`import … from 'buckyos'`），不另写一份。
- 浏览器副本不需要改：待发队列里存的是完整请求，发送时经同一个 `client.commit` 自动分块。本地规划受 §2.2 的同一组 `Limits` 约束。
- `store.submit` 和 `UndoCoordinator` 不改：一次 submit 产生一个 Commit，撤销栈推一项。

### 2.4 导入器：每次导入只调用一次 `store.submit`

- PDF、Office 幻灯片：画布、内容文件夹、原件、全部页面、放映路径放进同一个 Commit。删除“第几批”和“已导入到第几页”的逻辑，以及路径单独提交的那一步。
- Office docx、draw.io、表格：同样每次导入一个 Commit。表格不再按 20 000 行切分。
- Agent 工具（`import-office/tools/import.ts`、`import-table/tools/import.ts`）：同样一次 `client.commit`。
- 超过上限：提交前就能算出大小。超限直接报错，并给出能做的事（PDF / Office 缩小页范围，表格拆成几个文件），不再自动分成多个 Commit（Q4）。图片已经上传过的，会作为未引用的暂存资产在 24 小时后过期，与现在取消导入的情况相同。
- 进度：导入对话框的“提交”阶段先显示“上传 k / N”（`onPart`），再显示“写入中”（Q6）。
- 删除不再使用的代码和文案：`import-drawio` 的 `batches`、`import-office` 的 `sizedBatches`、`import-table` 的 `ROWS_PER_COMMIT` / `COMMIT_BYTES`，以及 i18n 里的 `*.importBatchLabel`、`*.error.stopped`、`aiworkspace.import.stopped`、`*.pathLabel`、报告项 `pdf_path_failed`。以实际引用为准。

## 3. 裁决项（建议值；owner 未另行裁决时按建议执行）

| # | 问题 | 建议 | 依据 |
| --- | --- | --- | --- |
| Q1 | 拼装后单个 Commit 的上限 | **32 MiB 且 ≤ 50 000 个操作**；客户端分块 6 MiB | §1.4：5 万个操作约 8 s 写锁，变化流事件约 80 MB；dk10000 全书约 2.1 万个操作、30 MB 事件，留有余量。DV 中如果浏览器处理最大事件明显卡顿，降到 16 MiB，并把“超大事件只给 touched”另立项 |
| Q2 | 暂存块的寿命 | **1 小时**；最终提交无论结果都消费；`ws.clear_staged` 一并清除 | 暂存只发生在在线发送的那一刻，紧接着就是最终提交。不需要像资产那样保留 24 小时 |
| Q3 | `doc.stage` 的准入 | 与 `doc.commit` 第 1 步相同，不做逐操作检查 | 防止无权者写垃圾数据；数量受配额和寿命约束；逐操作检查只在最终提交时做一次 |
| Q4 | 超过上限的导入 | **直接拒绝并提示缩小范围**；表格也改为一次提交 | 与“整体成功或整体失败”一致；自动分批正是这次要去掉的东西 |
| Q5 | TS 分块逻辑放在哪 | **websdk 导出，Desktop 复用** | 三条发送路径都汇聚到两个 `commit` 函数，只写一份 |
| Q6 | 提交阶段的进度 | “上传 k / N”，然后“写入中” | 暂存块对画布不可见，用户只看到进度 |

## 4. 里程碑

### M0 契约（只改文档）
- [第一期内置对象详细设计](<../doc/workspace/BuckyOS AI Workspace 第一期内置对象详细设计.md>) §2.6：加 `staged`、`doc.stage`、消费规则；`request_digest` 覆盖 `staged`；“资产是唯一有后台暂存状态的东西”改为“资产与提交暂存块”。
- [README](../src/frame/aiworkspace/README.md)：方法表加 `doc.stage`；写清两层上限；更新第 64 行表格导入那段（不再是“每个 Commit 至多 20 000 行”）。
- [Durable Data Schema](<../doc/workspace/BuckyOS AiWorkspace Durable Data Schema.md>)：`local.sqlite` 加 `staged_commits`。
- [PDF 方案](<../doc/workspace/BuckyOS AI Workspace PDF 导入与放映方案讨论.md>) §5 第 5 步和“撤销按批”：改为一次提交、一次撤销。

### M1 服务端
- core：`Limits.max_ops` 改为 50 000，新增 `Limits.max_commit_total_bytes`；`CommitRequest` 解析 `staged`。
- store：`staged_commits` 表与用量；`stage()`；`commit_inner` 在幂等检查之后、规划之前拼装；Commit 后删除暂存块；`sweep` / `clear_staged` 清理；`MAX_REQUEST_BYTES` 检查改用新上限。
- server：注册 `doc.stage`（方法表 stable），分发到 `with_ws`。

### M2 TS 客户端
- websdk：分块提交函数，`AiWorkspaceClient.commit` 改用它；加单测（小请求只调用一次；大请求按字节切块、摘要按顺序带回；遇到 `STAGED_PARTS_MISSING` 重新暂存一次）。
- Desktop：`AiwsClient.commit` 改用同一函数。

### M3 导入器
- 按 §2.4 改五个界面导入器和两个 Agent 工具；超限提示；进度；删除死代码和文案。

### M4 测试
- store（Rust）：分块拼装后被接受；重放（暂存块已删除）；同键不同块报 `IDEMPOTENCY_MISMATCH`；块缺失 / 过期报 `STAGED_PARTS_MISSING`；拼装后超限报 `LIMIT_EXCEEDED`；`prepare` 不消费；conflict / rejected 之后暂存块已删除；`sweep` 与 `clear_staged` 释放 `staged` 用量；对超过 8 MiB 的 Commit 撤销、再重做都成功；副本 `apply_remote` 能应用这个大事件。
- server：`doc.stage` 经 HTTP 的请求体上限。
- Desktop e2e：PDF / Office 导入后撤销栈只有一项，撤销一次画布清空，重做一次恢复；注入最终提交被拒（例如配额），画布上什么都不留；TI-11（“两个 Commit、取消后报告进度”）改为一次提交，取消时什么都不提交；离线副本模式下导入大文件。
- 各导入器的 deno 测试照常通过。

### M5 手册
- `aiworkspace_template_dev`、`aiworkspace_run`：写明“任意大小的 Commit 都是一次 `commit` 调用，客户端自动分块；上限 32 MiB / 50 000 个操作”。
- `src/tools/buckyos-agent/commands/import_table.ts`、`import_office.ts` 的帮助文字（如果提到分批）。

### M6 DV（test.buckyos.io）
- dk10000 全书一次导入成功；Ctrl+Z 一次全部撤回，Ctrl+Y 一次恢复；另一个浏览器同时打开，能收到这个事件并正常显示。记录提交耗时、事件大小、浏览器处理时间，回写 §1.4，验证 Q1 的数值。
- 离线副本模式重复一遍。

## 5. 验证命令

```bash
# 服务（在 buckyos/src）
cargo test -p aiworkspace-core -- --test-threads=1
cargo test -p aiworkspace-store -- --test-threads=1
cargo test -p aiworkspace -- --test-threads=1
# 导入模块（各自目录下）
deno test -A   # import-drawio、import-office、import-pdf、import-table
# Desktop（在 buckyos/src/frame/desktop）
node_modules/.bin/tsc -p tsconfig.app.json --noEmit && pnpm lint
node_modules/.bin/playwright test -c playwright.aiworkspace.config.ts tests/aiworkspace/pdf-import.spec.ts tests/aiworkspace/office-import.spec.ts tests/aiworkspace/drawio.spec.ts tests/aiworkspace/table-import.spec.ts tests/aiworkspace/offline.spec.ts
# （`pnpm test:e2e` 用默认配置，testDir 是 tests/e2e，跑不到这些用例）
# websdk（在 buckyos-websdk）
pnpm build   # Desktop 从 dist 取 SDK
pnpm test
```

## 6. 不在本单

- 超大变化流事件的瘦身（只给 `touched`，直连会话重读、副本重新 bootstrap）。等 M6 的数据说明需要再立项（Q1）。
- 跨 Commit 的原子撤销（`undo_group` 生效）。本单让导入只有一个 Commit，用不到它。
- 导入以外的大编辑（例如 Agent 一次写入很多内容）：自动受益于分块提交，不另做处理。
- 排序键空间耗尽时的兄弟重排（`ORDER_KEY_EXHAUSTED`）。

## 7. 实施回写（code agent 填写）

状态：2026-11-10 实施完成，buckyos 与 buckyos-websdk 两个仓库均未提交、未部署到 DV。

### 7.1 落点

| 部分 | 位置 |
| --- | --- |
| 上限与请求字段 | `core/src/plan.rs`：`Limits.max_ops` 50 000、`Limits.max_commit_total_bytes` 32 MiB、`CommitRequest.staged`；`core/src/replica.rs` 的 `submit_local` 在本地按同一上限检查大小 |
| 暂存表 | `store/src/schema.rs` 的 `LOCAL_ADDED_DDL`（`staged_commits`），`Workspace::open` 每次执行 `CREATE TABLE IF NOT EXISTS`，已有的 Workspace 不需要迁移 |
| 暂存、拼装、消费 | `store/src/workspace.rs`：`stage_commit`（`doc.stage`）、`commit_inner` 里幂等检查之后的拼装、`consume_staged`（`commit()` 返回前执行）、`drop_staged_commits`（`sweep` 和 `ws.clear_staged`）；演示克隆和 session 克隆清空该表（`show.rs`、`session/mod.rs`） |
| 方法注册 | `server/src/lib.rs`：`METHOD_LEVELS` 加 `doc.stage`（stable），分发到 `stage_commit` |
| TS 分块 | websdk `src/aiworkspace_client.ts`：`commitWithParts`、`splitCommitOperations`、`commitOverLimit` 和三个常量；`AiWorkspaceClient.commit` / `prepare` 改用它；`src/browser.ts` 补了 `export * from './aiworkspace_client'`（浏览器入口原先不导出这个模块，Desktop 拿不到） |
| Desktop 客户端 | `api/client.ts` 的 `commit` / `prepare`；`CommitOptions.onPart`、`SubmitOptions.onPart` 把分块进度传到导入对话框 |
| 导入器 | 公用的 `ui/import/submitImport.ts`（超限检查、一次 `store.submit`、进度）；`pdfImport.ts`、`officeImport.ts`（docx 与 pptx）、`drawioImport.ts`、`tableImport.ts`；Agent 侧 `import-office/tools/import.ts`、`import-table/tools/import.ts` |
| 测试 | `store/tests/staged.rs`（5 个）、`server/tests/contract.rs`（`doc.stage` 的契约步骤和 413）、websdk `tests/aiws_commit_parts.test.ts`（7 个）、Desktop e2e：`pdf-import.spec.ts` PI-05、`table-import.spec.ts` TI-11（改写）、`offline.spec.ts` 最后一个用例 |

### 7.2 与方案的偏差（请 owner 过目）

1. **补偿 Commit 的上限是请求的两倍。** §2.2 写的是撤销 / 重做也按 `max_commit_total_bytes` 判断。重做的补偿 Commit 带着完整内容和版本期望，会比原请求略大；按同一上限判断时，一个贴着上限被接受的导入可能撤销后无法重做。实现里 `opts.undoes` 存在时字节和操作数上限都乘 2。
2. **`doc.stage` 自己也检查总量。** 同一个键已暂存的字节加上本块超过 32 MiB 时，`doc.stage` 直接报 `LIMIT_EXCEEDED`，不等到最终提交；`part` 限制在 0–63。
3. **消费的范围比方案宽。** 任何一次非 dry-run 的 `doc.commit` 返回前，都会删除该调用者在这个幂等键下的暂存块，包括在入口检查就被拒的请求（例如 `epoch` 不符）。客户端每次发送都重新暂存全部块，所以不影响重试。
4. **`ws.clear_staged` 的返回值多了两个字段**：`commit_parts`、`commit_part_bytes`（契约 fixture 已重新录制）。
5. **draw.io 多页文件也是一个 Commit。** 多页时要在同一个 Commit 里建几张画布，导入器自己给后面的画布分配排序键和不重名的名字（`createSurfaceOps` 只看当前大纲）。
6. **表格导入：一次导入的多张表合成一个 Commit**，不是每张表一个；`table.insert_records` 每个操作除了 500 条记录的上限，还加了 1 MiB 的字节上限（`OP_BYTES`），保证操作之间总能切块。Agent 工具 `commitPlans` 的错误对象去掉了 `table`、`rows_done` 两个字段（已无意义）。
7. **表格导入对话框在发送和写入阶段禁用“取消”**，与 PDF、Office 一致。此前 TI-11 测的是“第一批提交期间取消，报告进度”，这个场景不存在了。
8. **删除的文案键**除 §2.4 列出的之外，还有 `aiworkspace.pdf.error.canvasFailed`、`aiworkspace.office.error.pageFailed`、`aiworkspace.drawio.newCanvasFailed`、`aiworkspace.drawio.importStopped`、`aiworkspace.import.labelPart`、报告项 `office_path_failed`。新增 `*.error.failed` / `aiworkspace.drawio.importFailed` / `aiworkspace.import.failed`（“导入没有写入，什么都没有留下”）、`aiworkspace.import.overLimit.*`、`aiworkspace.import.progressSend` / `progressWrite`。
9. **测试钩子** `__aiwsTestHooks.commitPartBytes`：让 e2e 用很小的块走 `doc.stage` 路径。只在测试钩子启用时生效。

### 7.3 验证

| 检查 | 结果 |
| --- | --- |
| `cargo test -p aiworkspace-core -- --test-threads=1` | 通过 |
| `cargo test -p aiworkspace-store -- --test-threads=1` | 通过（含 `staged.rs`：拼装、重放、缺块 / 过期 / 摘要不符、他人的块、conflict 后已消费、拼装后超限、`prepare` 不消费、`sweep` 与 `clear_staged`、10 MiB 的 Commit 撤销再重做、副本 `apply_remote` 应用该事件、副本本地超限） |
| `cargo test -p aiworkspace -- --test-threads=1` | 通过（契约 fixture 重新录制，差异只有新增的 `doc.stage` 步骤和 `ws.clear_staged` 的两个字段） |
| `deno test -A`：import-drawio / import-office / import-pdf / import-table | 34 / 47 / 36 / 33 通过 |
| websdk `jest` | 新增 7 个通过；全量 1178 通过、1 个失败：`file_meta.test.ts` 的 “real samples › dk10000.pdf”，它把 `media_samples` 里的文件都当图片，与本单无关 |
| Desktop `tsc --noEmit` | 通过 |
| Desktop `pnpm lint` | 4 个错误，改动前后相同（`ai-center/.../RequestFields.tsx` 等，与本单无关） |
| Desktop e2e（`playwright.aiworkspace.config.ts`）：pdf-import、table-import、office-import、drawio、offline | 48 个通过 |

e2e 覆盖到的行为：PI-05 用 4 KB 的块导入三页 PDF，最终提交被注入拒绝时画布、页面、路径都没有留下，`head_seq` 不变；放行后是多次 `doc.stage` 加一次 `doc.commit`，撤销栈一项，撤销一次回到导入前，重做一次恢复。TI-11 导入 20 001 行，一个 Commit，撤销、重做各一次。离线副本用例：表格导入先存本地，发送循环分块送出，结算后撤销一次撤回。

### 7.4 dk10000 全书实测（本机，release 后端，standalone aiworkspace + nfs_server，Chromium）

dk10000.pdf 是 403 页（§1.4 按 806 页估算有误）。可编辑模式、不保留原件（文件 272 MB，超过 32 MiB 的资产上限）。

| 项 | 值 |
| --- | --- |
| 结果 | 403 页：8 412 段文字、1 942 张图片、94 个图形；13 997 个操作，一个 Commit |
| 转换加上传图片 | 约 255 s |
| 请求 | `doc.stage` 6.29 MB（0.29 s）+ `doc.commit` 2.54 MB（1.92 s），合计 8.8 MB |
| 变化流事件 | 1 个，26.0 MB，`doc.get_changes` 0.98 s |
| 撤销 / 重做 | `doc.undo` 1.68 s / 1.17 s；界面从点击到大纲变化 1.8 s / 1.9 s |
| 用量 | 内容 6.8 MB，资产 321 MB，暂存 0 |
| 另一个窗口 | 同一 Workspace 的第二个窗口保持连接；没有切到新画布去看显示 |

按这组数据，Q1 的 32 MiB / 50 000 余量充足：这本书用了上限的 28%（字节）和 28%（操作数）。

### 7.5 没有做、没有验证的

- **M6 的 DV（test.buckyos.io）没有做**：没有部署，Zone 网关链路上的 8 MiB 请求、真实身份、第二个浏览器显示新画布、离线副本下导入这本书，都没有验证。
- **副本模式下带图片的导入会被本地拒绝**（`DEPENDENCY_UNAVAILABLE: asset … has not been uploaded`）：副本的本地规划不知道刚上传的暂存资产。改动前就是这样（旧流程第一个 Commit 就带原件的 asset-ref），只是现在整次导入一起被拒。本单没有处理，需要另立项。
- `doc.prepare` 分块时暂存的块不被消费，要等 1 小时过期，期间计入 `staged` 用量。
- 最终提交失败后，`store.submit` 照旧把这次输入留在“修改状态”列表里（需要处理），里面是整次导入的操作。
- 撤销在“部分可撤”的路径上逐个试探逆操作（`undo_inner`），两万个操作时会很慢；没有测。
- WASM 包（`desktop/src/app/aiworkspace/wasm/aiworkspace_wasm_bg.wasm`）已用 wasm-bindgen 0.2.130 重建，因为 core 的上限和请求字段变了；本机为此装了 rustup 的 `wasm32-unknown-unknown` target。
- Agent 命令 `import_table` / `import_office` 只做了 `deno check`，没有对真实服务跑；它们用的 websdk 来自 `rootfs/libexec/buckyos-tool/dist`，要重新构建后才带分块提交。
- 只测了 Chromium。
