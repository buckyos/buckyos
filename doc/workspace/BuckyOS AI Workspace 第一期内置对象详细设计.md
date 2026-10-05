# BuckyOS AI Workspace 第一期内置对象详细设计

> 状态：实施设计 v0.1（配套《第一期核心架构设计与验证》v0.1）。
>
> 读者：实现 `src/frame/aiworkspace` 与 Desktop `aiworkspace` 应用的 CodeAgent 与开发者。
>
> 目标：回答“内置对象怎样才能在第一期架构上真正可用”。先给出对第一期文档的 Review 结论与裁决，再把内核公共机制和每个内置对象细化到可以直接编码、写 fixtures 和写测试的程度。

## 0. 定位与阅读方式

本文是[《第一期核心架构设计与验证》](<BuckyOS AI Workspace 第一期核心架构设计与验证.md>)（下称“第一期文档”）的下一层设计，不替代它。关系如下：

- 第一期文档规定**边界和验收**（V01–V24、M0–M5）。本文不放宽其中任何“必须”。
- 本文规定**机制和数据结构**：版本令牌、操作目录、提交管线、类型适配器接口、各内置对象的 payload/操作/冲突/撤销/物化规则、存储 DDL、服务方法。
- 本文中标注【裁决】的条目是对第一期文档留白或歧义处的明确选择；标注【偏离】的条目与第一期文档的“建议”不同，已写明原因，实施时按本文执行并在 M1 回写第一期文档；标注【探针】的条目必须在 M1 用代码证实，证伪时按给出的退路执行。
- 仓库现状类事实（服务接入、ndn-lib 行为、依赖版本）集中在 §8，均注明核实方式。未核实的写明“未核实”。

阅读顺序：§1 Review → §2 内核公共机制 → §3 各内置对象 → §4 存储 → §5 服务接口 → §6 离线副本 → §7 受控加工 → §8 仓库接入事实 → §9 任务拆分。只实现某个对象的 CodeAgent 至少要读 §2 全部和 §3 对应小节。

**引用原则：尽力使用 ObjectId，但不把它作为所有数据源的前提。** 本文的默认物化、内容相等和去重规则用于普通内置对象；少数明确的大数据场景按 §2.4、§3.4.11 使用 URL + 查询参数，不要求全量物化。对查询定义做内容寻址，不能替代对查询结果的版本固定。

---

## 1. Review：第一期文档的架构问题与裁决

第一期文档的边界划分是自洽的：后台权威、命令唯一入口、稳定身份与内容寻址分层、CRDT 关在类型适配器之后。问题不在方向，而在若干**机制没有落到可实现的粒度**，其中几处如果让各对象的实现者各自发挥，会得到互不兼容的实现。按影响排序：

| 编号 | 问题 | 为什么必须现在定 | 裁决落点 |
| --- | --- | --- | --- |
| R1 | **版本令牌没有定义。** §4.3 示例用 `"field-revision-8"`，§3.3 列了 `content_revision`/字段版本，但没有说它是什么、谁生成、如何比较、离线链式提交如何续接。 | OCC、冲突报告、撤销适用性判断、事件、订阅追赶、派生结果新鲜度全部依赖同一个令牌。 | §2.2：单一 `seq` + “版本格” |
| R2 | **CRDT 更新如何成为一次 Commit 没有协议。** §4.4 只说“不能先改共享 CRDT 再回滚”，没有说富文本操作在提交信封里长什么样、后台内存文档与 SQL 事务谁先谁后。 | 这是 V10（混合原子批次）和 V06/V07 的实现核心，也是崩溃恢复是否正确的关键。 | §3.3.4 |
| R3 | **显式草稿与 CRDT 的矛盾只有原则没有出路。** §4.2 要求“依据基准生成允许分享的语义变更，在共享分支重新应用”，这等于要求一套不经过 CRDT 历史的富文本语义操作。 | V14（私有四次修改对端只见结果）在富文本上不可验证，除非定义这套操作。 | §3.3.5：块级语义操作 |
| R4 | **“Rust 能解释并校验富文本”意味着 ProseMirror↔CRDT 的映射是协议的一部分。** 第一期文档把它当作“binding 选型”，但 binding 是 JS 库的实现细节，升级一次小版本就可能改变容器布局。 | Rust 后台要解码、校验、并且要能**写**（Agent/Mock/撤销都在后台改富文本）。映射不成文，后台就只能转发二进制。 | §3.3.3：映射成文 + 双端 fixtures |
| R5 | **权限模型缺失，但 V17、V21 要验收它。** 文档反复要求“可信接收端检查”，却没有主体、能力、授权范围的最小定义。 | 读投影、订阅过滤、append-only 这些会影响读接口和变化流的形状，后补代价很高。 | §2.9 |
| R6 | **内部引用携带 `workspace_id` 与 Fork、内容寻址冲突。** §2.1 建议内部引用用 `(workspace_id, entity_id)`，§3.5 又要求 Fork 时“内部引用全部重绑定”。 | 重绑定会改写所有含引用对象的内容，使 Fork 后 ObjectId 全部变化，去重和“Fork 前后内容相等”的校验都失效。 | §2.4【偏离】：持久化的内部引用不写 `workspace_id` |
| R7 | **NamedObject 的物化边界未定。** 哪些字段进入被哈希内容？1 万行的表如何成为一个可验证对象？快照根是否确定？ | V01/V02/V04/V05 都要比较 ObjectId。把名字、位置、`revision` 混进被哈希内容，会让“改名后固定 ObjectId 内容不变”不成立。 | §2.10 |
| R8 | **幂等字段冗余且语义不全。** `submission_id`、`idempotency_key`、`session_id` 三者并存；未说明 conflict/rejected 结果是否也被幂等记录。 | 决定 `commits` 表的唯一约束和重试行为。 | §2.6 |
| R9 | **`prepare` 返回“候选标识”隐含后台要保存候选状态**（TTL、配额、崩溃清理），与“prepare 不持有锁”并不等价。 | 影响服务状态面和 Agent 大批次的协议。 | §2.6：无状态 dry-run + 摘要 |
| R10 | **变化通知与离线追赶没有机制。** `subscribe` 只有一行语义，§7 的“重连先取远端 head/变化”没有接口。 | 在线刷新、离线重连、多视图同步是同一个问题，应该只有一个答案。 | §2.8：按 `seq` 拉取的变化流 + 唤醒提示 |
| R11 | **“不得另建一套权威引擎”与浏览器离线执行的落点不明。** 文档允许“Rust/WASM 内核或受限适配器”二选一，没有说内核怎样切分才可能被复用。 | 决定 crate 划分。存储和运行时依赖一旦混进内核就无法编译到 WASM。 | §2.1、§6 |
| R12 | **撤销的两套机制没有统一的归属规则。** CRDT UndoManager 只存在于编辑器会话内存；补偿提交需要逆操作。哪些提交可以被后台补偿没有定义。 | V13 要求“编辑器和全局不重复撤销”，需要先定义每个操作的撤销能力类别。 | §2.7 |
| R13 | **“分页必须固定快照”在 SQLite 工作库上没有可实现的定义。** 工作库不是 MVCC 多版本存储，无法在别人提交后继续读旧版本。 | 决定 `query` 的 cursor 结构和错误码。 | §2.5 |
| R14 | **§3.4 表中 `outbox` 归属混淆。** 同一张表组里既像后台的待发通知，又像客户端的待提交队列。 | 两者是不同进程中的不同表。 | §4：后台没有 outbox，变化流即 `commits`；客户端有 `pending_submissions` |
| R15 | **标识符的生成方、字符集和冲突处理未定义**；`commit_id` 与 `seq` 的关系未定义。 | 离线创建对象必须由客户端生成 ID；字段 ID 会进入 JSON 路径和索引表达式，字符集必须受限。 | §2.3 |
| R16 | **派生值来源与人工覆盖放在哪里没有定义**（§5.3 末段）。 | 影响记录存储布局和 Mock 的冲突行为。 | §3.4.7 |
| R17 | **视图引用的字段/选项被删除后的行为未定义。** | 静默丢掉一个过滤条件会让视图显示更多行，是数据暴露问题而不只是显示问题。 | §3.5.4 |
| R18 | **私有草稿和个人 Overlay 是否存入后台未定义。** | 决定后台是否需要“个人作用域”存储和导出过滤。 | §2.9、§3.7：草稿只在客户端；注释有 `scope` |
| R19 | **里程碑顺序有两处隐含倒置。** 权限检查被 V21 要求但没有出现在 M2 的交付物里；富文本 Rust 端编解码是 M3 的前提却只在 M1 以“探针”出现。 | 避免 M3 才发现后台改不了富文本。 | §9 |
| R20 | **后台内存中的 CRDT 文档何时更新没有规定。** | 落盘前更新内存文档，事务失败后内存状态就被污染，且无法回滚。 | §3.3.4：先 fork 校验，SQL 提交成功后才换入 |
| R21 | **历史裁剪与旧离线客户端。** §5.2 只要求“压缩前验证重连规则”。 | 第一期如果裁剪 CRDT 历史，V06“完整恢复后继续合并原 lineage 增量”会在某些路径失败。 | §3.3.8：第一期不裁剪历史，只合并存储 |
| R22 | **部分读权限主体与变化流/离线副本的关系。** | append-only 主体不能读整表，那么它也不能拿到含该表操作的变化流，更不能做全量离线副本。 | §2.8、§2.9 |
| R23 | **对象树该不该用 CRDT。** 分层架构设想以 Yjs 承载对象树；第一期文档 §4.5 让树移动由后台串行化并校验预期父节点。 | 两人同时拖动同一个对象时报版本冲突，对画布体验是错的；但把树做成 CRDT 结构会失去拒绝非法修改和按子树过滤权限的能力。 | §2.2.3、§3.1【偏离】：取 CRDT 的合并语义，不取其数据结构 |
| R24 | **没有“自动合并结果不被接受”时的出口。** 所有并发策略都是自动的；当某个对象的协作反复出问题时，管理员除了收回所有人的写权限没有别的手段。 | 需要一个由权限体系延伸出来的、可按对象开启的悲观策略。 | §2.2.3、§2.11：写锁 |

另有三处不构成问题但需要统一口径：

1. **Cell 与内容实体的树位置。** 第一期文档 §2.3 说“每个结构节点有一个父节点”“同一 TableSource 可被多个 Cell 引用”，没有说 TableSource 自己挂在哪里。【裁决】所有实体都在结构树里且只有一个父节点；内容实体（RichText、TableSource、RecordObject、AssetRef）可以挂在页面或分组下而不被直接渲染，Cell 通过引用展现它们。见 §3.1。
2. **“已提交富文本的 AST 是投影”与“Agent 读 AST”。** AST 投影必须与某个 `content_rev` 严格对应并由后台生成，客户端上传的 AST 永远不被信任。见 §3.3.6。
3. **“提交策略 immediate/explicit”是客户端行为，不是后台模式。** 后台只看到 Commit；两种策略的区别是客户端何时、以哪种操作形态生成 Commit。见 §2.6。

---

## 2. 内核公共机制

### 2.1 Crate 与模块划分

【裁决】后台拆成三个 crate，依赖方向单向向下：

```text
src/frame/aiworkspace/
  core/        # package: aiworkspace-core   —— 纯逻辑，可编译到 wasm32-unknown-unknown
  store/       # package: aiworkspace-store  —— SQLite 实现、NamedObject/资产适配、导入导出
  server/      # package: aiworkspace        —— 进程入口、服务协议、鉴权、运行时接入
  schemas/     # 内置类型的 JSON Schema、富文本 schema 定义（Rust 与前端共同读取）
  fixtures/    # 有效/非法文档、跨语言 ID 向量、操作序列与期望结果
```

`aiworkspace-core` 的硬约束：不依赖 tokio、文件系统、rusqlite、网络、系统时间和随机数；这些通过 trait 由调用方注入。它包含：标识符与引用类型、canonical 编码与 ObjectId 计算、内置类型的 schema 校验、操作规划（§2.5 的 `plan`）、过滤表达式求值、富文本编解码与语义操作、逆操作生成、错误码。

这样切分的理由：浏览器离线副本（§6）加载同一个 `core` 的 WASM 构建，使用同一套校验、规划和编码；它只是没有授权终审权。第一期文档“不得另建一套拥有不同业务规则的权威引擎”由此在结构上得到保证，而不是靠两套实现对齐。【探针】M1 必须证明 `core` 连同所选 CRDT 库能编译为 WASM 并在 Worker 中运行；若 CRDT 库的 Rust crate 无法进入同一个 WASM 模块，退路是前端使用该库的官方 JS/WASM 包，`core` 的富文本部分通过窄接口（导入更新、导出 AST）调用它，其余逻辑仍然共用。

`core` 通过两个 trait 读取状态，不关心背后是 rusqlite、浏览器 SQLite 还是测试用内存表：

```rust
/// 规划阶段的只读视图。实现方保证同一个 ReadCtx 生命周期内读到一致状态。
pub trait ReadCtx {
    fn entity(&self, id: &EntityId) -> Result<Option<EntityRow>>;
    fn edge(&self, child: &EntityId) -> Result<Option<TreeEdge>>;
    fn children(&self, parent: &EntityId) -> Result<Vec<TreeEdge>>;
    fn field(&self, source: &EntityId, field: &FieldId) -> Result<Option<FieldRow>>;
    fn fields(&self, source: &EntityId) -> Result<Vec<FieldRow>>;
    fn record(&self, source: &EntityId, record: &RecordId) -> Result<Option<RecordRow>>;
    fn scan_records(&self, source: &EntityId, visit: &mut dyn FnMut(&RecordRow) -> Result<Flow>) -> Result<()>;
    fn richtext(&self, id: &EntityId) -> Result<Option<RichTextHandle>>;
    fn inbound_refs(&self, target: &EntityId) -> Result<Vec<RefEdge>>;
    fn asset(&self, object_id: &ObjId) -> Result<Option<AssetRow>>;
}

/// 提交阶段由引擎调用；类型适配器不直接拿到它。
pub trait WriteCtx: ReadCtx {
    fn apply(&mut self, write: StoreWrite) -> Result<()>;
}
```

`StoreWrite` 是一个封闭枚举（`PutEntity`、`PutEdge`、`PutField`、`PutRecord`、`PutRichTextUpdate`、`PutRef`、`DelRef` 等）。类型适配器只能**返回** `StoreWrite` 列表，不能自己执行写入。这是“Adapter 不能自行提交”的结构性保证。

### 2.2 版本模型：一个 `seq`，若干版本格

【裁决】每个 Workspace 有一个单调递增的 64 位整数 `seq`。每次被接受的 Commit 使 `seq` 加一，Commit 的顺序号就是它的 `seq`。`head_seq` 是最后一次已接受 Commit 的 `seq`，空 Workspace 的 `head_seq` 为 0。

所有 OCC 判断基于**版本格（version cell）**：一个可被独立竞争的最小状态单元。每个版本格保存一个 `rev`，值为最后一次修改它的 Commit 的 `seq`；从未被写过的格 `rev` 为 0。

| 版本格 | 存放位置 | 何时变化 |
| --- | --- | --- |
| `entity.meta_rev` | `entities` | 实体 `name` 或 `write_policy` 变化 |
| `entity.content_rev` | `entities` | 实体内容的任何变化（粗粒度，用于订阅、缓存失效、物化缓存键） |
| `entity.life_rev` | `entities` | 创建、删除、恢复 |
| `edge.struct_rev` | `tree_edges` | 父节点、兄弟顺序、位置摆放变化 |
| `key_revs[<key>]` | `entities.key_revs_json` | 键式文档（§2.2.1）的某个顶层键变化 |
| `field.def_rev` | `table_fields` | 字段定义的任何变化（含改名） |
| `field.type_rev` | `table_fields` | 影响已有值合法性的变化：类型、scale、必填/可空、选项删除 |
| `record.rev` | `table_records` | 记录创建、删除、任一字段值变化 |
| `value_revs[<field_id>]` | `table_records.revs_json` | 该记录该字段的值被设置、清空或迁移 |
| `field.values_rev` | `table_fields` | **任一记录**在该字段上的值变化（列级；用于保护“按该字段筛选”的读集合） |
| `source.members_rev` | `entities.key_revs_json` 的保留键 `#members` | 记录被插入、删除或恢复（成员集合变化） |

这个设计直接满足第一期文档的几条要求：

- **发现“改走又改回”**：`rev` 只增不减，值相同但 `rev` 不同即被识别（§4.5）。
- **独立字段互不冲突**：两人改同一记录的不同字段，各自校验各自的 `value_revs`，都通过。
- **不以整文档版本拒绝**：`head_seq` 只用于变化流追赶，不参与写冲突判断。
- **`content_rev` 与细粒度 `rev` 并存**：前者让“这张表变了没有”是 O(1) 判断，后者决定“这次写入冲突没有”。

富文本的并发文本编辑不走版本格，由 CRDT 合并（§3.3）；但富文本实体仍有 `content_rev` 和 `life_rev`，并参与删除竞争判断。

`rev` 是 Workspace 内部的工作状态令牌，**不进入任何被哈希的 NamedObject 内容**（§2.10），也不跨 Workspace 比较。对外协议中它是 JSON 整数；`seq` 在第一期不会超过 2^53，直接用 JSON number。

#### 2.2.1 键式文档（KeyedDoc）

RecordObject、Cell、Container、AssetRef、Annotation 的 payload 都是“一个 JSON 对象，顶层键各自独立演进”。统一用一个内核结构承载：payload 保存在 `entities.payload_json`，每个顶层键的 `rev` 保存在 `entities.key_revs_json`。统一的操作是 `entity.set_keys` / `entity.unset_keys`（§2.5）。各类型只需声明 schema 与键级约束，不重复实现并发控制。

顶层键是并发的最小单位。类型设计时要据此划分键：希望能被两人同时修改而不冲突的内容必须放在不同顶层键下（例如 TableView 的 `filter` 与 `sorts` 是两个键）。

#### 2.2.2 前置条件

每个“覆盖型”操作内联携带它所覆盖版本格的期望值；Commit 还可以附带额外的读集合前置条件。

```json
{ "expect": { "rev": 8 } }          // 目标版本格的 rev 必须等于 8
{ "expect": { "rev": 0 } }          // 目标从未被写过（新建或首次赋值）
{ "expect": "any" }                 // 显式放弃检查；只有声明了 force 能力的调用方可用，见下
```

- 覆盖型操作（设置字段值、设置键、改名、删除、改字段定义）**必须**带 `expect`。树的移动与摆放属于自动合并（§2.2.3），`expect` 可选。缺失即 `INVALID_OPERATION`，不默认为后写覆盖。
- `"any"` 仅供迁移工具和导入暂存区内部使用，服务协议入口对普通主体拒绝它。第一期不向 UI/Agent 开放强制覆盖。
- 追加型操作（插入记录、创建实体、追加子节点）不需要 `expect`，它们的冲突只有“ID 已存在”和“父节点已删除”。
- Commit 级 `preconditions[]` 表达读集合：`{ "target": <selector>, "expect": { "rev": N } }`。受控加工用它声明“我读过的输入没有变”（§7）。读集合分两种，缺一不可：**读过的值**用单元格的 `value_revs`；**决定哪些记录参与计算的条件**用集合级版本格，选择器 `table_field_values`（对应 `field.values_rev`）和 `table_members`（对应 `source.members_rev`）。只保护前者会漏掉“新记录进入了筛选范围”这类变化。

**离线链式提交的续接规则**：客户端的本地副本为每个版本格保存“已确认 rev”。同一个格被两个待提交 Submission 先后修改时，后一个 Submission 中的 `expect.rev` 先写入一个占位（`{ "after": "<前一个 Submission 的 idempotency_key>" }`）；前一个被接受并得到 `seq = S` 后，客户端把占位改写为 `{ "rev": S }` 再发送。后台不理解占位，收到占位即 `INVALID_OPERATION`。这样后台不需要“依赖链”特判，且不会掩盖其他人在两次提交之间对同一格的修改：别人插进来修改过，`rev` 就不是 `S`。

#### 2.2.3 三种并发策略

每个操作属于下面三种策略之一。前两种是操作的固有属性，由 `OpSpec.class` 声明；第三种是管理员按对象开启的附加约束，叠加在前两种之上。

| 策略 | 行为 | 默认适用 | 对应 `OpClass` |
| --- | --- | --- | --- |
| **自动合并** | 按后台到达顺序生效，不因别人的并发修改而报冲突；只在结果非法时拒绝 | 树的移动与摆放、追加型操作（创建实体、插入记录）、富文本的 CRDT 更新 | `Merge`、`Append`、`Create` |
| **版本校验** | 携带 `expect`；同一版本格被别人先改过则显式冲突 | 表格单元格、键式文档的键、字段定义、改名、删除、块级富文本操作 | `Overwrite`、`Structural` |
| **写锁** | 同一时间只有锁的持有者能修改该对象；其他人的任何修改被拒绝 | 默认不启用；由有 `manage` 能力的人对具体对象开启（§2.11） | 叠加约束 |

划分的依据是“并发修改后，自动得到的结果是否总是可以接受”：

- 两人把同一个卡片拖到不同位置，以后到者为准是所有人都能理解的结果，报冲突反而打断操作 → 自动合并。
- 两人把同一个预算单元格改成不同的数，以后到者为准等于静默丢掉一个人的输入 → 版本校验。
- 自动合并或版本校验在规则上都没错，但某个对象的协作结果反复让人不满意（例如一份需要一人统稿的正文）→ 由人决定改用写锁。写锁是核心机制上的出口：它不试图把合并做得更聪明，而是让这个对象不再发生并发修改。

【偏离】**对象树采用自动合并的语义，但不使用 CRDT 数据结构。** 第一期文档 §4.5 要求树移动“校验预期父节点”，本文放宽为后到者生效（规则见 §3.1）。不把树做成 Yjs 或 Loro 文档的理由：

1. Yjs 的共享类型没有“移动”操作，嵌套类型不能改挂父节点；用删除加插入模拟移动在并发下会重复或丢失节点，用“`id → 父节点`”映射加后写者胜会产生环。
2. CRDT 的更新不能被拒绝，而树有必须拒绝的情况：不允许的子类型、同级重名、删除仍被引用的源、根不可移动、成环。要拒绝就得为树再建一套“确认文档 + 工作文档”。
3. 一个 CRDT 文档是一个同步单位，能同步它的人得到整棵树及其历史。按子树的读权限、个人作用域实体（§2.9）都无法在其上过滤。
4. 写锁（§2.11）要求后台能拒绝非持有者的修改，这与第 2 点是同一个前提。
5. 有权威后台时，CRDT 额外提供的只是“没有仲裁者也能收敛”，第一期不需要。兄弟顺序用分数索引，并发插入本来就不冲突。

本文规定的树操作集合（创建到某父某位置、移动到某父某位置、摆放、删除）与可移动树 CRDT 的操作集合一致，合并规则（后写者胜、成环的移动被丢弃、删除优先）也一致。以后出现没有单一权威后台的场景（例如两个各自权威的 Zone 互相同步），可以把树的存储换成真正的可移动树 CRDT 而不改变操作协议。这是 M5-4 需要回写第一期文档的条目。

### 2.3 标识符

| 标识 | 生成方 | 格式 | 说明 |
| --- | --- | --- | --- |
| `workspace_id` | 后台（创建/Fork/导入为新实例时） | `ws_` + 26 位小写 base32（130 bit 随机） | 恢复同一 Workspace 时沿用包内 ID |
| `entity_id`、`record_id`、`field_id`、`option_id`、`block_id` | **调用方**（客户端、Agent、Mock），以支持离线创建 | `^[a-z0-9][a-z0-9_-]{0,63}$` | 建议默认生成 20 位小写 base32（100 bit 随机）；fixtures 可用可读 ID 如 `tasks` |
| `commit_id` | 后台 | `cm_` + 26 位 base32 | 与 `seq` 一一对应；对外引用用 `commit_id`，排序和追赶用 `seq` |
| `idempotency_key` | 调用方 | 1–128 字节可见 ASCII | 作用域见 §2.6 |
| `session_id` | 调用方 | 同 `entity_id` 字符集 | 只用于追踪和撤销栈归属，不参与授权 |
| `run_id` | 后台（`proc.start` 返回） | `run_` + 26 位 base32 | §7 |
| `lineage_id` | 创建 CRDT 文档的一方 | 同 `entity_id` 字符集 | §3.3 |
| `ObjectId` | 由内容计算 | CYFS `ObjId` 字符串形式 | 复用 ndn-lib，见 §8 |

规则：

1. 字符集受限是硬约束。`field_id` 会出现在 SQLite JSON 路径和可能的表达式索引中，`entity_id` 会出现在导出包文件名中。后台对所有调用方提供的 ID 做正则校验，不做转义后放行。
2. 调用方生成的 ID 与已有 ID（含 tombstone）冲突时返回 `ID_CONFLICT`（归入 `INVALID_OPERATION` 的子码），整批拒绝。不自动改名。
3. 作用域：`entity_id` 在 Workspace 内唯一；`record_id`、`field_id` 在所属 TableSource 内唯一；`option_id` 在所属字段内唯一；`block_id` 在所属 RichText 内唯一。
4. 这些 ID 都不是 ObjectId，不得以 ObjectId 的字符串形式生成。

### 2.4 引用

持久化的引用结构（出现在 payload、表格 `object_ref` 值、富文本嵌入节点属性中）：

```json
{
  "entity_id": "tasks",
  "version": { "mode": "live_head" },
  "selector": { "kind": "table_cell", "record_id": "task-42", "field_id": "status" }
}
```

【偏离】**指向本 Workspace 的引用不写 `workspace_id`**；只有跨 Workspace 引用才写。服务接口返回的“已解析引用”总是带上完整的 `workspace_id`。原因见 R6：

- Fork 不需要改写任何对象内容，Fork 前后的内容根 ObjectId 相等（§2.10），这成为 V05 的一条可自动化断言。
- 相同内容在不同 Workspace 中得到相同 ObjectId，底层才有可能去重。
- 第一期文档要求的“内部引用全部重绑定”在此模型下自动成立：省略即“本 Workspace”。

`version` 取值：

| mode | 附加字段 | 第一期 |
| --- | --- | --- |
| `live_head` | 无 | 实现 |
| `fixed_revision` | 内容寻址时带真实 `object_id`；URL 查询源可改带 `source_revision`，两者按定位类型区分；可选 `seq` 仅作提示 | ObjectId 路径在 `entity_versions`/对象存储中解析；源版本路径要求源支持历史读取与保留，不能用普通 ETag 冒充 |
| `published_channel` | `channel` | 结构保留，解析返回 `UNSUPPORTED_VERSION`，不降级为 `live_head` |

**URL 查询引用。** 另允许 `QueryReference` 分支，字段为 `kind: "url_query"`、`source_url`、结构化 `query`、`version`、`consistency`（`snapshot` 或 `best_effort`）。它无需 `entity_id` 或 `object_id`；通常由一个有稳定实体身份的 TableSource 持有，Cell 继续引用该实体。`source_url` 与查询参数共同构成定位，查询参数采用适配器定义的受控协议。源码和 wire schema 均应使用引用联合类型，禁止以空/伪造 ObjectId 占位。

- `version.mode = live_head` 是缺省。只有适配器验证源提供真实的指定版本读取能力，才能使用 `fixed_revision + source_revision`。源版本令牌是源的承诺，不具备 ObjectId 的内容自验证能力。
- 查询规范化须保留参数语义；查询摘要只标识定义，不标识结果内容。持久配置不带凭据或临时 cursor；缓存按源、查询、授权范围和实际版本隔离。
- 解析结果报告能力及状态：是否支持固定版本、一致分页、稳定行键、离线切片和写入。不能因 URL 可解析就假设这些能力存在；源暂时不可达不妨碍保存引用定义。
- 第一期间接读取 URL 源由服务层 Source Adapter 执行；`core::plan` 保持纯函数，不在写事务中联网。TableSource 的查询结果不自动进入 Workspace 的本地提交历史。

`selector.kind` 第一期取值：`entity`（缺省）、`table_field`、`table_record`、`table_cell`、`richtext_block`、`doc_key`（键式文档的一个顶层键）；另有两个只用于前置条件的集合选择器 `table_field_values`（`field_id`）与 `table_members`。未知 kind 在读取时原样保留，解析时返回 `MISSING_EXTENSION`。

**引用索引。** 后台维护 `refs` 表（§4），每条边记录来源实体、来源内选择器、目标、引用种类（`bind` 数据绑定、`embed` 富文本嵌入、`value` 表格值、`anchor` 注释锚点、`asset` 资产、`body` 记录正文）。边由类型适配器在规划操作时增量给出（`OpPlan.ref_changes`），与数据在同一事务落盘。`refs` 是边的**集合**：键为（来源实体、来源选择器、种类、目标），同一来源位置可以有多条指向不同目标的边，指向同一目标的重复引用只算一条。它支撑三件事：删除前的引用检查、快照的依赖枚举、资产保留根。`refs` 是可重建的派生数据：`aiworkspace-store` 必须提供全量重建并与增量结果比对的校验工具，作为测试的一部分。

### 2.5 类型适配器、操作规划与查询

#### 2.5.1 TypeAdapter

```rust
pub trait TypeAdapter: Send + Sync {
    fn type_id(&self) -> &'static str;                    // 如 "buckyos.table-source"
    fn supported_schema_versions(&self) -> (u32, u32);    // 闭区间
    fn ops(&self) -> &'static [OpSpec];                   // 本类型拥有的操作

    /// 纯函数：读取候选状态，产出本操作的全部效果。不得有副作用。
    fn plan(&self, ctx: &dyn ReadCtx, op: &Op, env: &PlanEnv) -> Result<OpPlan, WsError>;

    fn resolve_selector(&self, ctx: &dyn ReadCtx, entity: &EntityRow, sel: &Selector)
        -> Result<ResolvedTarget, WsError>;
    fn read(&self, ctx: &dyn ReadCtx, entity: &EntityRow, sel: &Selector, proj: &Projection)
        -> Result<serde_json::Value, WsError>;

    /// 普通内容返回 ObjectId；大数据例外可保留 URL 查询定义。§2.10
    fn materialize(&self, ctx: &dyn ReadCtx, entity: &EntityRow, sink: &mut dyn ObjectSink)
        -> Result<MaterializedContent, WsError>;
    /// 恢复对象内容或 URL 查询定义；后者不抓取整份远端数据。
    fn load(&self, src: &dyn ObjectSource, content: &MaterializedContent, target: &EntityId)
        -> Result<Vec<StoreWrite>, WsError>;
}

pub enum MaterializedContent {
    Object { object_id: ObjId },
    UrlQuery { reference: QueryReference, definition: serde_json::Value },
}

pub struct OpSpec {
    pub name: &'static str,            // "table.set_values"
    pub capability: Capability,        // 执行它所需的能力，§2.9
    pub undo: UndoClass,               // §2.7
    pub class: OpClass,                // Create | Overwrite | Append | Merge | Structural
}

pub struct OpPlan {
    pub checks: Vec<CellCheck>,        // (版本格, 期望) —— 来自 op.expect 和操作隐含的检查
    pub writes: Vec<StoreWrite>,       // 候选写入
    pub ref_changes: Vec<RefChange>,
    pub inverse: Option<Vec<Op>>,      // 逆操作；UndoClass::Compensable 时必须给出
    pub touched: Vec<Touched>,         // 事件用：稳定 selector + 变化类别
    pub reads: Vec<CellRead>,          // 规划时读过的版本格及当时的 rev，用于锁外规划后的复核
}
```

`PlanEnv` 由引擎注入：本次 Commit 将获得的 `seq`、可信主体、`origin`、确定性的时间戳（后台接收时间）、各项限额。适配器不得自己读时钟或生成随机数，这样同一输入在后台、WASM 副本和测试中得到同样的计划。

`QueryReference` 的字段见 §2.4。普通对象返回 `Object`；URL TableSource 可返回 `UrlQuery`，其中 `definition` 保存对应类型校验过的小型表定义/schema（不重复保存 `source_ref`，不含远端记录集），确保导入能恢复配置。也可将这些元数据和查询说明一起保存为一个 `Object`，并显式报告其中的 URL 依赖。后者的 ObjectId 只验证定义，仍不固定记录集；不得为了满足接口签名强制物化大表。请求“完整自包含内容”的调用方需另检查物化结果与依赖能力。

#### 2.5.2 候选状态与原子性

引擎对一个 Commit 的处理（细化第一期文档 §4.4 的五步）：

1. **入口校验**：协议版本、`epoch`（§2.6）、大小与操作数限额、ID 字符集、每个操作名存在且主体具备 `OpSpec.capability`（初查）。
   **幂等查询在这里进行，先于任何依赖文档当前状态的步骤**：按 `(可信主体, idempotency_key)` 查 `commits`。命中且摘要相同 → 直接返回保存的原结果（`replayed: true`），不再规划；命中但摘要不同 → `IDEMPOTENCY_MISMATCH`。否则一个已成功但响应丢失的“创建实体”请求在重试时会先在规划阶段撞上自己造成的 `ID_CONFLICT`。入口校验与后续步骤都在写执行者内串行执行，所以查询之后到提交之前不会有同键请求插入；`commits` 上的唯一约束是防实现缺陷的最后保障，不是并发控制手段。
2. **规划**：建立一个覆盖层 `Overlay`（`ReadCtx` 的实现：先查本批已产生的写入，再查底层存储）。按顺序对每个操作调用 `plan`，把 `writes` 放入覆盖层，使后续操作看到前面操作的效果（同批内先建表再插入记录是合法的）。任何一步失败即整批失败，覆盖层丢弃，存储和内存中的 CRDT 文档均未被触碰。
3. **全局约束**：在覆盖层上检查树约束（无环、父节点存在且未删除、根不可移动）、本地引用完整性（本批新增引用的目标存在或在本批创建）、计算依赖无环（第一期只有 Mock 的声明依赖）、限额。URL 查询引用在这里校验配置与使用能力，不在事务内联网取数，也不把暂时不可达误判为本地悬空实体；实际读取时由服务层报告可用性与源授权结果。
4. **事务提交**：复核两件事：主体当前授权（终查）；`checks` 与 `reads` 中每个版本格的当前 `rev`。`checks` 不满足 → `REVISION_CONFLICT`/`TARGET_DELETED`/`SCHEMA_CONFLICT`。`reads` 有变化但 `checks` 全部满足 → 在临界区内基于当前状态重新规划一次（小批次直接重做；见下方并发模型）。全部通过后在**一个 SQLite 事务**内写入：全部 `StoreWrite`、`refs` 变化、`commits` 行、`commit_ops` 行、`head_seq`。
5. **发布**：事务提交成功后，才把新的 CRDT 文档换入内存、唤醒变化流等待者、返回结果。进程在第 4、5 步之间崩溃时，重启后内存状态从数据库重建，等待者通过 `seq` 追赶，客户端通过幂等键查询结果。

【裁决】并发模型：**每个打开的 Workspace 一个写执行者**（一个持有写连接和内存 CRDT 文档的 actor/任务，串行处理 Commit）。读走独立的只读连接（SQLite WAL 模式）。第一期所有 Commit 的第 2–4 步都在写执行者内完成；只有受控加工的输入读取和候选生成在外部完成（它们产出的是普通 Commit 请求）。这比“锁外规划 + 锁内复核”简单且无正确性风险；`reads` 字段保留在 `OpPlan` 中，用于 M5 性能验证后是否把大批次规划移出临界区的决策，第一期不实现锁外规划。验收指标：1000 行批量修改的临界区耗时，见第一期文档 §8.3。

#### 2.5.3 操作目录总览

操作名是协议的一部分，格式 `<域>.<动词>`。完整参数见各对象小节。另有两个只由撤销路径生成、服务入口拒绝调用方直接提交的内部操作：`table.restore_records`（§3.4.3）与 `table.restore_field`（§3.4.4）。资产没有专用操作：登记资产就是创建一个 `buckyos.asset-ref` 实体（§3.6）。

| 域 | 操作 | 类别 | 撤销 |
| --- | --- | --- | --- |
| `entity` | `create`、`delete`、`restore`、`rename`、`set_keys`、`unset_keys`、`set_write_policy` | Create / Structural / Overwrite | Compensable |
| `tree` | `move`、`place` | Merge（后到者生效；可选 `expect`） | Compensable |
| `table` | `insert_records`、`delete_records`、`set_values`、`unset_values`、`set_body`、`add_field`、`update_field`、`delete_field`、`add_option`、`update_option`、`delete_option`、`migrate_field` | Append / Overwrite / Structural | Compensable（`migrate_field` 见 §3.4.6） |
| `richtext` | `apply_update` | Merge | SessionLocal |
| `richtext` | `insert_blocks`、`replace_block`、`delete_blocks`、`move_block` | Overwrite（块级） | Compensable |

通用操作 `entity.create` 的参数：`entity_id`、`type_id`、`schema_version`、可选 `name`、`parent_id`、`order_key`、可选 `placement`、`payload`（交给对应适配器校验并展开为 `StoreWrite`）。创建与挂树是一个操作，不存在“已创建但无父节点”的中间状态。

#### 2.5.4 读取与查询

- `read`：给定引用，返回授权投影后的内容、`content_rev`、对应版本格的 `rev`。读取结果里的每个可写单元都带 `rev`，客户端据此构造 `expect`。
- `query`（表格）：受控过滤 AST（§3.5.3）、排序、limit、cursor。

【裁决】内置表查询一致性的可实现定义：cursor 是不透明字符串，内部编码 `(source content_rev, field type_rev 摘要, 过滤与排序的摘要, 最后一行的排序键, 最后一行的 record_id)`。翻页时若该 TableSource 的 `content_rev` 已变化，返回 `SNAPSHOT_EXPIRED`（`retryable: true`），调用方从第一页重来，或在请求中显式传 `consistency: "best_effort"` 接受键集分页的弱一致结果（UI 的无限滚动使用它，并在收到变化事件后刷新）。内置表的导出、物化、受控加工输入可在一个只读事务里读取。这满足第一期文档 §3.4“不能把变化中的 offset 分页称为一致读取”：一致模式下要么读到同一版本，要么明确失败。

URL 大表采用源的分页/版本协议，不把本地 TableSource 定义的 `content_rev` 当成远端记录集的版本。支持 snapshot token 时跨页绑定同一 token；不支持时只能显式返回 `best_effort`，或拒绝需要一致输入的任务。引用型导出只保存定义，按需物化时只读取所需结果范围；禁止为复用内置表路径先全量下载。源不支持的过滤/排序不能静默忽略，也不默认全量拉回再计算。

过滤表达式的**语义权威是 `core` 中的 Rust 求值器**（同一份代码在 WASM 副本中运行）。SQL 下推是 `store` 的优化，必须通过差分测试证明与求值器结果逐行一致；第一期基线实现可以对单个 TableSource 做全表扫描后在 Rust 中过滤排序，1 万行规模下先测量再决定是否加表达式索引（第一期文档 §3.4 的要求）。

### 2.6 Commit 协议（对第一期文档 §4.3 的细化）

请求：

```json
{
  "protocol_version": "0.1",
  "workspace_id": "ws_...",
  "epoch": "ep_...",
  "idempotency_key": "s-a/17",
  "session_id": "session-a",
  "origin": "human",
  "run_id": null,
  "undo_group": "ug-17",
  "message": "把任务 42 标为完成",
  "preconditions": [],
  "operations": [
    {
      "op": "table.set_values",
      "source_id": "tasks",
      "field_type_revs": { "status": 3 },
      "values": [
        { "record_id": "task-42", "field_id": "status", "value": "option-done", "expect": { "rev": 8 } }
      ]
    }
  ]
}
```

【裁决】

1. **去掉 `submission_id`。** 幂等身份只有一个：`(workspace_id, 可信主体, idempotency_key)`。客户端需要的本地关联 ID 就用 `idempotency_key`。
2. **请求摘要**：`request_digest = sha256(JCS(请求去掉 session_id 与 message 后的对象))`。同键同摘要 → 返回原结果并带 `replayed: true`；同键不同摘要 → `IDEMPOTENCY_MISMATCH`。
3. **只有 `accepted` 结果被幂等记录**，记录就是 `commits` 表的行（唯一约束 `(principal, idem_key)`），永久保留到该提交被历史保留策略清除为止（第一期不清除）。`conflict` 与 `rejected` 不产生任何持久状态：用同一个键重发会被重新求值。这是安全的，因为它们没有副作用；也是必要的，因为“未知结果后重发原请求”必须能在第一次其实没到达时成功。客户端在冲突后修改了操作内容，必须换新键。
4. `get_submission(epoch, idempotency_key)` 返回 `accepted`（含原结果）或 `not_found`。`not_found` 的含义是“后台在这个 `epoch` 内没有接受过这个键”，调用方可以安全地原样重发。
7. **`epoch`（历史代次）**：每个 Workspace 有一个 `epoch`（`ep_` + 26 位 base32），在创建时生成，并且**每当它的提交历史被替换时重新生成**（以 `restore` 语义导入，§4.4）。`seq`、所有 `rev`、幂等记录、变化流位置都只在一个 `epoch` 内有意义。`ws.get_info`、`get_changes`、`wait_changes`、`replica.bootstrap` 的响应和 kevent 唤醒提示都带当前 `epoch`；`commit`、`prepare`、`undo`、`get_changes`、`get_submission` 的请求必须带调用方所依据的 `epoch`，与当前不一致 → `EPOCH_MISMATCH`（不可重试）。它使“同一个 `workspace_id` 被恢复成另一段历史”对所有旧客户端显式可见，而不是让旧的 `seq` 和 `rev` 在新历史里被误解释。
5. `origin` 取值 `human`、`agent`、`program`、`import`、`system`，只进入历史和事件，不影响授权。可信主体、接收时间、`seq`、`commit_id` 由后台填写。
6. `undo_group` 可选，由调用方给出，用于把一次用户可理解的动作标成一个撤销单位；一个 Commit 就是最小撤销单位，`undo_group` 只是给 UI 的分组提示，后台不做跨 Commit 的原子撤销。

响应三态：

```json
{ "status": "accepted", "commit_id": "cm_...", "seq": 19, "replayed": false,
  "touched": [ { "entity_id": "tasks", "selector": { "kind": "table_cell", "record_id": "task-42", "field_id": "status" },
                 "change": "value", "rev": 19 } ],
  "server_ops": [] }

{ "status": "conflict", "code": "REVISION_CONFLICT", "retryable": false,
  "conflicts": [ { "op_index": 0, "item_index": 0,
                   "selector": { "kind": "table_cell", "record_id": "task-42", "field_id": "status" },
                   "expected_rev": 8, "current_rev": 12, "current_value": "option-open" } ] }

{ "status": "rejected", "code": "INVALID_SCHEMA", "retryable": false,
  "errors": [ { "op_index": 0, "path": "/values/0/value", "detail": "unknown option" } ] }
```

- `conflicts[].current_value` 只在主体对该目标有读权限时返回。
- 一次响应报告本批**全部**可检测的冲突，而不是遇到第一个就返回，便于 UI 一次呈现。
- `server_ops`：后台在接受时为该 Commit 生成的、客户端需要应用的内容。第一期只有一种：块级富文本操作在后台执行后产生的 CRDT 更新（§3.3.5）。

**`prepare`**：与 `commit` 相同的请求体，执行 §2.5.2 的第 1–3 步和第 4 步的只读复核，返回将要发生的 `touched`、冲突或错误、以及 `request_digest`。【裁决】后台不保存候选状态；`prepare` 之后的 `commit` 重新发送完整请求。迁移类操作（§3.4.6）的预检报告通过 `prepare` 返回。资产是唯一有后台暂存状态的东西（§3.6）。

**两种提交策略（客户端行为）**：

| 策略 | 表格/键式文档 | 富文本 |
| --- | --- | --- |
| `immediate` | 每个编辑动作完成（单元格失焦、回车、拖动结束）即形成一个 Commit | 编辑器产生的 CRDT 更新按 300–1000 ms 防抖或失焦合并为一个 `richtext.apply_update` Commit |
| `explicit` | 修改累积在客户端草稿区，用户提交时把草稿折叠为每个版本格一次写入，形成一个 Commit | 草稿在私有的文档分叉上编辑；提交时与基准做块级差分，生成块级语义操作（§3.3.5），不发送草稿的 CRDT 历史 |

### 2.7 撤销与重做

每个操作声明撤销类别：

| 类别 | 含义 | 实现 |
| --- | --- | --- |
| `Compensable` | 后台保存了逆操作，可以在任意时间、由任意有权主体生成补偿提交 | `commit_ops.inverse_json` |
| `SessionLocal` | 只能由产生它的编辑会话撤销；撤销动作本身是一次新的普通 Commit | 客户端 CRDT UndoManager；后台不保存逆操作 |
| `None` | 不可撤销，接口明确返回 `NOT_UNDOABLE` | 第一期无此类内置操作，保留给以后的外部动作 |

**后台补偿撤销**（`undo` 接口，参数 `commit_id`、`idempotency_key`、`mode`）：

1. 读取目标 Commit 的全部逆操作。若其中含 `SessionLocal` 操作 → `NOT_UNDOABLE`，并指出是哪些操作。
2. 适用性检查：目标 Commit（设其顺序号为 `S`）写过的每个版本格，当前 `rev` 必须仍等于 `S`。等于 → 该格可撤销；不等 → 该格已被后续提交修改，列入冲突。这是“补偿的目标已被别人修改时默认返回冲突”的精确定义。逆操作的 `expect` 就是 `{ "rev": S }`，所以这一步由普通的 Commit 管线完成，不需要特殊路径。
3. `mode: "all_or_nothing"`（缺省）：有任何冲突即返回 `conflict`，附可撤销部分与冲突部分的清单。`mode: "partial"`：调用方必须回传上一步返回的 `plan_digest`，后台只执行清单中可撤销的部分，形成一个新的 Commit。这对应第一期文档“若允许部分撤销，必须先返回可执行部分并形成显式新命令组”。
4. 补偿 Commit 记录 `undoes: <commit_id>`。**重做就是撤销这个补偿 Commit**，同样受适用性检查约束。不存在独立的 redo 存储。
5. 授权：缺省只允许撤销自己的 Commit；撤销他人的需要 `manage` 能力。补偿 Commit 中的每个逆操作仍按普通操作检查当前权限和写锁（§2.11）。
6. 自动合并类操作的撤销同样受适用性检查约束：逆操作总是带 `expect: { "rev": S }`。我移动了一个对象，别人随后又移动了它，这时撤销我的移动会报告冲突，而不是把对象拉回我移动之前的位置。“正向操作不报冲突”与“撤销不覆盖别人的后续修改”并不矛盾：前者是当前用户此刻的意图，后者是对一个可能已经过时的旧意图的回退。

**客户端 UndoCoordinator**（一个会话一个）：维护单一撤销栈，栈元素是以下之一：

- `{ kind: "editor", entity_id }`：指向某个富文本编辑器内部 UndoManager 的一步；
- `{ kind: "commit", commit_id }`：一个已被接受的 `Compensable` Commit；
- `{ kind: "pending", idempotency_key }`：尚未发送的本地 Submission（撤销即从待提交队列移除并回滚本地工作视图，不产生网络请求）。

Ctrl+Z 只弹出栈顶一个元素并交给对应执行者；编辑器自身的 undo 快捷键绑定必须被禁用并改为调用 Coordinator。这样“编辑器 undo”和“全局 undo”在结构上不可能同时触发（V13）。一次 Agent/Mock 批次是一个 Commit，因此是一个栈元素；它如果包含富文本修改，用的是块级语义操作（`Compensable`），可以整体补偿。

### 2.8 变化流与事件

【裁决】只有一种权威的变化传播机制：**按 `seq` 拉取的变化流**。推送只是“有新 `seq` 了”的唤醒提示，不携带必须被处理的内容。

```text
get_changes(workspace_id, after_seq, limit, filter?) →
  { head_seq, changes: [ CommitEvent... ], more: bool }

wait_changes(workspace_id, after_seq, timeout_ms) →
  { head_seq }            // 长轮询：head_seq > after_seq 或超时即返回
```

`CommitEvent`：

```json
{
  "seq": 19, "commit_id": "cm_...", "origin": "human", "run_id": null, "undoes": null,
  "idempotency_key": "s-a/17",
  "author": "did:...", "accepted_at": "2026-10-04T08:00:00.000Z",
  "touched": [
    { "entity_id": "tasks", "selector": { "kind": "table_cell", "record_id": "task-42", "field_id": "status" },
      "change": "value", "rev": 19 }
  ],
  "ops": [ ... ]
}
```

- `change` 类别：`created`、`deleted`、`restored`、`renamed`、`moved`（父或顺序）、`placed`（摆放/尺寸）、`value`（业务内容）、`schema`（字段定义）、`view`（Cell/TableView 配置）、`text`（富文本内容）。依赖失效只看 `value`/`schema`/`text`/`deleted`；`moved`/`placed`/`view`/`renamed` 不使数据加工结果过期。这是 V19 的判定依据，由适配器在 `OpPlan.touched` 中给出，不由订阅方猜测。
- `ops`：做全量副本的客户端需要它来把本地确认基准推进到该 `seq`。对富文本，`ops` 中含 CRDT 更新字节（base64）。只需要失效通知的调用方传 `filter: { "detail": "touched" }` 省去 `ops`。
- 去重与顺序：事件以 `seq` 全序，客户端保存“已应用到的 `seq`”。重复投递、乱序唤醒都不影响正确性。
- `idempotency_key` **只在调用方就是该提交的作者主体时返回**（幂等键的作用域是主体，对他人没有意义也不应可见）。它是客户端把变化流中的提交与自己的待提交操作对应起来的唯一依据：响应丢失时客户端不知道 `seq` 和 `commit_id`，只知道自己发出的键。
- 权限过滤：`touched` 与 `ops` 中只包含主体**当前**有读权限的实体的条目；被完全过滤掉的 Commit 仍返回一个只含 `seq` 的空事件，使客户端的 `seq` 连续。主体对某实体只有 `append` 没有 `read` 时，看不到该实体的任何内容事件（R22）。
- 保留：第一期变化流就是 `commits`/`commit_ops` 表，不清理。以后引入保留期时，`after_seq` 早于保留下界返回 `BASE_TOO_OLD`，客户端改为重新获取检查点。这个错误码第一期就要定义并在客户端处理（走“重新准备离线副本”的路径），即使后台暂时不会返回它。

唤醒提示的传输（长轮询之外是否使用系统已有的事件通道）见 §8。无论采用哪种，客户端逻辑都是“被唤醒 → `get_changes`”。

### 2.9 最小权限模型

第一期不做完整的共享产品，但读写接口的形状必须按有权限的情况设计。

**主体**：由服务运行时验证后的调用身份（用户 ID，以及可用时的应用 ID）。独立测试模式用配置文件中的固定测试身份表和静态 token。客户端请求体中任何自称的身份字段都被忽略。

**能力**：`read`、`append`、`update`、`delete`、`structure`（创建/移动/改名/改 schema）、`comment`（创建和修改自己的注释）、`export`、`manage`（授权、撤销他人提交、Fork、删除 Workspace）。

**授权记录**存放在 Workspace 文件夹内的本地库 `local.sqlite`（不是文档库，不随普通导出带走，见 §4.1）：

```text
grants(workspace_id, subject, scope_entity_id NULL, capabilities)
```

- `subject`：用户 ID，或 `*`（任何已认证主体）。第一期不实现组。
- `scope_entity_id` 为空表示整个 Workspace；非空表示该实体及其结构子树。
- 有效能力 = Workspace 级授权 ∪ 从目标实体沿结构树向上各级授权的并集。第一期没有“拒绝”规则，没有字段级/记录级授权；`append` 是唯一比 `read` 更窄的写能力，专门用来验证“能追加但不能读”。
- 创建者获得 Workspace 级全部能力。跨 Workspace 引用不传递授权：解析目标时按目标 Workspace 的授权独立检查。

**检查点**（每一处都是必须）：

| 接口 | 检查 |
| --- | --- |
| `resolve`/`read`/`list_children` | 目标 `read`。无权时：调用方对其父节点有 `read`（本来就能看到它存在）则返回 `PERMISSION_DENIED`；否则返回 `NOT_FOUND`，不泄露它是否存在 |
| `query` | TableSource `read` |
| `commit`/`prepare` | 每个操作按 `OpSpec.capability` 对其目标检查；`table.insert_records` 需要 `append` 或 `update`；仅有 `append` 的主体，接受响应的 `touched` 只含它自己插入的记录 ID。之后检查写锁（§2.11） |
| `get_changes` | 逐条目按 `read` 过滤（§2.8） |
| `undo` | 见 §2.7 |
| `checkpoint`/`export`/`fork` | `export`/`manage`；导出内容再按 `read` 过滤，过滤掉的实体列入 Manifest 的“未包含”清单 |
| 离线副本准备 | 需要 Workspace 级 `read`；部分读权限的主体第一期不能准备离线副本，返回明确错误 |

**个人作用域**：`entities.scope` 取 `shared` 或 `user:<主体>`。第一期只有 Annotation 可以是个人作用域（§3.7）。个人作用域实体只对其所有者可见，不进入普通导出，不进入他人的变化流。私有草稿和客户端撤销栈**不上传后台**，只存在于浏览器离线副本（§6）；这意味着第一期换设备不能延续草稿，这是已知限制，写入交付文档。

### 2.10 NamedObject 物化与快照

Workspace 管理的工作状态在 SQLite 中；普通内容的不可变版本按需物化为 CYFS NamedObject。物化发生在：显式 `checkpoint`、`export`、`fork`、解析内容寻址的 `fixed_revision` 引用被创建时、受控加工需要固定输入时。不在每次 Commit 时物化。URL 大表可仅保存定义及源版本引用，不要求在这些动作中取得整个远端记录集。

#### 2.10.1 内容对象：哪些东西进入哈希

【裁决】普通实体版本默认采用 JSON NamedObject（`jobj`），被哈希内容为：

```json
{ "ws_type": "buckyos.table-source", "schema_version": 1, "content": { ... } }
```

**不进入**内容对象的：`entity_id`、`name`、父节点、顺序、摆放、任何 `rev`/`seq`、作者、时间。它们属于结构索引或历史。推论：

- 改名、移动一个实体，它的内容 ObjectId 不变（V02）。
- 两个内容相同的实体有相同的内容 ObjectId。
- 同一内容多次物化得到同一 ObjectId；`entity_versions(entity_id, content_rev) → object_id` 是纯缓存，可重建。

各类型 `content` 的确定性规则在 §3 各小节给出。共同规则：对象内数组如果语义上是集合，必须按规定的键排序后编码；可选字段缺省就省略，显式 `null` 保留（第一期文档 §2.2 第 4 条）；适合物化的大内容通过 FileObject 引用，不内联。无法合理物化的大表允许保留 `UrlQuery` 定位；上述内容相等、去重断言不适用于尚未取得的远端记录集。

#### 2.10.2 结构索引与内容根

```json
// ContentRoot（jobj）
{
  "ws_type": "buckyos.workspace-content",
  "format_version": "0.1",
  "root_entity_id": "root",
  "entities": { "encoding": "jsonl+jcs", "count": 12, "file": "<FileObject ObjId>" },
  "resolved_refs": [
    { "from": "summary", "to": { "workspace_id": "ws_other", "entity_id": "prices" }, "object_id": "<ObjId>" }
  ],
  "unresolved_refs": []
}
```

`entities` 文件每行一个实体条目的 JCS 编码，按 `entity_id` 字节序排序：

```json
{ "entity_id": "tasks", "type_id": "buckyos.table-source", "schema_version": 1, "name": "任务",
  "parent_id": "page-1", "order_key": "a1", "object_id": "<内容对象 ObjId>" }
```

- 只含 `scope = shared` 且未删除的实体。`placement` 存在时写入条目；`write_policy` 不是 `open` 时写入条目。
- 普通条目携带 `object_id`；无 ObjectId 的大表条目改带 `source_ref: QueryReference` 和 `definition`（小型表定义/schema），与 `object_id` 分支互斥，不写空 ObjectId。条目的类型、结构关系和查询定义仍由 ContentRoot 固定；远端数据没有因此被固定。也可把同一份小型定义物化后走 `object_id` 分支，仍需报告 URL 依赖。
- 父子关系只在这里表达；内容对象之间不互相包含对方的 ObjectId 来表达树（避免第一期文档 §2.3 说的哈希循环）。内容对象内部的**引用**（如 Cell 的 `source_ref`）在 `live_head` 模式下只含 `entity_id`，不含 ObjectId，因此也不形成循环。
- `resolved_refs`：本快照中已固定的外部依赖，记录内容 ObjectId 或真实的源版本定位；未能固定的跨 Workspace/URL 依赖进入 `unresolved_refs` 并在导出 Manifest 中报告。本 Workspace 内已物化的 `live_head` 对象不必重复列出；但内置 TableSource 持有的动态 URL 依赖仍需报告。`entities` 固定一个查询定义不代表固定其结果。这就是“依赖解析清单”。

```json
// SnapshotRoot（jobj）—— 把内容根绑定到某个 Workspace 的某个历史位置
{
  "ws_type": "buckyos.workspace-snapshot",
  "format_version": "0.1",
  "workspace_id": "ws_...",
  "seq": 42,
  "commit_id": "cm_...",
  "content": "<ContentRoot ObjId>",
  "lineage": { "forked_from": { "workspace_id": "ws_src", "snapshot": "<SnapshotRoot ObjId>" } }
}
```

拆成两层的理由：**ContentRoot 只由内容决定**。由此得到三条可以直接写成测试断言的不变量：

1. 导出 → 在空环境导入（恢复同一 Workspace）→ 再物化，ContentRoot ObjectId 相等（V04）。
2. Fork 完成时，新 Workspace 的 ContentRoot ObjectId 等于来源快照的 ContentRoot ObjectId；之后任一方修改，两者分叉（V05）。
3. 只做改名、移动或调整摆放时，这些信息属于结构索引，因此 ContentRoot 变化而各内容对象不变。测试断言“所有内容对象的 ObjectId 不变，仅 `entities` 文件变化”（V02）。

创建时间、物化者等放在 `snapshots` 表和导出 Manifest，不进入被哈希内容。

对含 URL 依赖的文档，以上相等性只覆盖包内内容、结构与原样保留的查询定义。导入/Fork 的校验不得重新查询远端后拿新结果比较 ContentRoot；查询结果是否可复现、离线可用，按源版本和实际纳入的结果切片单独验收。第一期默认仍给小型 ContentRoot/SnapshotRoot 使用 ObjectId，但不要求其每个数据依赖都具备 ObjectId。

#### 2.10.3 保留根

`store` 维护资产与对象的保留根集合，取并集：当前工作状态中的全部 `asset` 引用；`snapshots` 表中 `retained = 1` 的快照可达的全部对象；`commit_ops` 中出现过的资产引用（历史可撤销）；暂存区中未过期的上传。第一期不做自动回收，只实现“列出不在保留根中的对象”的诊断接口和对应测试（V22：有引用的不在可回收清单里）。向底层 named store 的 pin/保留接口如何对接见 §8。

### 2.11 写锁

**用途。** 让某个对象在同一时间只有一个写入者。它是权限体系的延伸：授权决定“谁有资格改”，写锁决定“有资格的人里此刻谁在改”。两种典型用法：

1. 只允许一个人改：不需要写锁，把该对象的写能力只授给一个主体即可（§2.9）。
2. 少数人都可以改，但不允许同时改：对该对象开启写锁，这些人竞争同一把锁。

**策略属于文档，锁状态属于部署。**

- `write_policy`：实体公共信封上的属性，取 `open`（缺省）或 `lock_required`。由 `entity.set_write_policy`（参数 `entity_id`、`policy`、`expect` 对 `meta_rev`）修改，需要 `manage` 能力。它和 `name` 一样不进入内容对象，而是记录在结构索引条目中（非 `open` 时写入），因此随导出、Fork 保留。
- 锁本身是 `local.sqlite` 中的一行租约，不进入文档库、变化流、快照和任何导出。导入或 Fork 得到的 Workspace 保留策略，但没有任何人持有锁。

**锁保护什么。** 对一个 `lock_required` 的实体，下列操作要求调用方持有它的锁：

| 实体类型 | 受保护的操作 |
| --- | --- |
| 任何类型 | 针对该实体内容的全部写操作：`entity.set_keys`/`unset_keys`、`table.*`、`richtext.*` |
| Container | 另加其**直接子节点的成员与布局**变化：在其下创建实体、移入、移出、摆放、删除直接子节点 |

不受该实体的锁保护的：它自身的改名、移动、删除（由它的父容器的锁与普通权限决定）；锚定在它上面的注释（附加层，不是它的内容）；它的后代的内容（各自看自己的策略，锁不沿树继承）。记录正文（§3.4.5）是独立的 RichText 实体，表被锁不等于各条正文被锁。

**租约。**

```sql
-- local.sqlite
CREATE TABLE locks (
  entity_id   TEXT PRIMARY KEY,
  lock_id     TEXT NOT NULL UNIQUE,       -- lk_ + 26 位 base32
  principal   TEXT NOT NULL,
  session_id  TEXT NOT NULL,
  acquired_at TEXT NOT NULL,
  expires_at  TEXT NOT NULL,              -- 后台时钟
  last_write_at TEXT
) WITHOUT ROWID;
CREATE TABLE lock_events (                -- 申请、释放、过期接手、强制解除的记录
  id INTEGER PRIMARY KEY AUTOINCREMENT, entity_id TEXT NOT NULL, event TEXT NOT NULL,
  principal TEXT NOT NULL, session_id TEXT, by_principal TEXT, at TEXT NOT NULL
);
```

- 持有者是 **(主体, `session_id`)**。同一个人的两个标签页或两台设备是两个会话，互相竞争。这是有意的：锁要能挡住持有者自己另一个窗口里的陈旧修改。
- 租约时长缺省 60 秒，持有者每 20 秒续约。后台以自己的时钟判断过期；过期的租约在下一次有人申请时被接手，无需清理任务。
- 空闲释放：持有者连续 10 分钟没有对该对象的被接受提交，续约被拒绝（`LOCK_LOST`），锁释放。防止有人开着窗口离开而长期占锁。时长是 Workspace 设置项。
- 申请要求调用方对该实体有写能力（`update`、`append`、`delete`、`structure` 之一）；没有写能力的人不能占锁。

**接口**（kRPC，结果结构同 §5）：

| 方法 | 说明 |
| --- | --- |
| `lock.acquire { entity_ids[], session_id }` | 全部成功或全部失败。成功返回各 `lock_id` 与 `expires_at`。有任一被他人持有 → `LOCK_HELD`，附持有者主体、起始时间、到期时间（调用方对该实体有 `read` 时）。对 `open` 的实体申请是空操作 |
| `lock.renew { lock_ids[] }` | 延长到期时间。已过期且被他人接手，或触发空闲释放 → `LOCK_LOST` |
| `lock.release { lock_ids[] }` | 主动释放。编辑器失焦一段时间、关闭、切换对象时调用 |
| `lock.break { entity_id }` | 需要 `manage`。立即解除并写入 `lock_events`。被解除的持有者下一次续约或提交得到 `LOCK_LOST` |
| `lock.list { entity_ids? }` | 返回当前有效的锁；需要对相应实体有 `read` |

`doc.read`、`doc.list_children` 返回的实体信封带 `write_policy` 和当前持有者（若有），UI 据此显示“由某人编辑中”。锁状态变化通过 kevent 发布到 `/aiworkspace/<workspace_id>/locks`，负载只有一个递增计数，客户端收到后调用 `lock.list`；与变化流的唤醒一样，它只是提示（§8.2）。

**提交时的检查。** 在 §2.5.2 第 4 步与授权终查同一位置、同一写执行者内进行：对本批每个受保护的目标，`locks` 中存在未过期的行且 (主体, `session_id`) 与提交者一致。

| 情况 | 结果 |
| --- | --- |
| 无人持有 | `rejected`，`LOCK_REQUIRED` |
| 他人持有 | `rejected`，`LOCK_HELD`（附持有者信息） |
| 调用方曾持有但已过期或被解除 | `rejected`，`LOCK_LOST` |
| 一批操作涉及多个受保护对象，只持有其中一部分 | 整批拒绝，逐个报告 |

- 因为检查和写入在同一个串行执行者内，不存在“检查时持有、写入时已被接手”的窗口，不需要额外的防护令牌。
- 锁不替代版本校验和 CRDT 合并：持有者的提交照常走原有管线。持有锁时这些检查几乎不会失败，但它们能挡住持有者自己的陈旧请求。
- Agent、受控加工（§7）、撤销（§2.7）没有例外：它们的提交者必须持有锁。`proc.apply` 与 `doc.undo` 带 `session_id`，以调用者的会话身份接受检查。
- 对一个已有待提交修改的对象开启 `lock_required`，这些修改到达时得到 `LOCK_REQUIRED`，按冲突内容保留在客户端，不丢失。

**客户端行为。**

- 进入 `lock_required` 对象的编辑状态前先 `lock.acquire`。失败则保持只读并显示持有者；可以提供“请求交接”（只是通知持有者，不是抢占）。
- 持有期间自动续约。续约失败（`LOCK_LOST`、网络不可达直到本地判断租约已过期）→ 立即把编辑器切为只读，尚未被接受的修改保留为待处理内容（§6.5 的“需要处理”），明确告知用户锁已失去。
- **离线**：离线时拿不到锁，已有的租约也会在离线期间过期。因此第一期规定 `lock_required` 的对象在后台不可达时只读。不提供长期“签出”；它需要另行设计签出期限、强制收回和离线修改的归属，留待以后。
- 在线直连模式（未持有离线副本的标签页，§6.1）同样可以申请锁；锁与离线副本的持有者身份无关。

**测试要点（建议作为新增验收项回写第一期文档）。** 两人竞争同一把锁，恰好一人成功；非持有者的提交被拒绝且其内容在客户端可恢复；持有者进程被杀，租约到期后另一人接手，原持有者恢复后的迟到提交得到 `LOCK_LOST` 且不被应用；`lock.break` 后同样；空闲释放；同一用户的第二个标签页拿不到锁；对被锁容器的子节点做移动被拒绝，而修改该子节点的内容不被拒绝；导出再导入后策略保留、锁不存在；Mock 在未持锁时 `LOCK_REQUIRED`。

---

## 3. 内置对象

每个对象按同一结构描述：用途与边界 → payload → 操作 → 约束与冲突 → 撤销 → 物化 → 必须有的 fixtures。类型 ID 一览：

| `type_id` | 角色 | 内容承载 | 并发机制 |
| --- | --- | --- | --- |
| `buckyos.container` | 根、工作页、分组 | 键式文档 + 树边 | 版本格 |
| `buckyos.record` | 有 schema 的小型业务对象 | 键式文档 | 版本格（顶层键） |
| `buckyos.richtext` | 富文本 | CRDT 状态 + AST 投影 | CRDT 合并 / 块级 OCC |
| `buckyos.table-source` | 结构化表 | `table_fields` + `table_records` | 版本格（字段定义、记录、单元格） |
| `buckyos.cell` | 展现实例，含 TableView | 键式文档 | 版本格（顶层键） |
| `buckyos.asset-ref` | 文件/图片引用 | 键式文档 | 版本格 |
| `buckyos.annotation` | 最小 Overlay | 键式文档 | 版本格 |

所有类型的 `schema_version` 第一期均为 1。后台遇到不认识的 `type_id` 或更高的 `schema_version`：实体行原样保存（`payload_json` 不解析、不改写），`read` 返回原始 payload 并带 `degraded: "MISSING_EXTENSION"` 或 `"UNSUPPORTED_VERSION"`，对它的任何内容操作被拒绝，`entity.delete`/`tree.move` 仍然允许；物化时把原始 payload 作为 `content` 原样编码（V20）。

### 3.1 Container：根、工作页、分组

**用途与边界。** 承载结构树。第一期只有一个工作页，但模型不为“只有一页”做特化。

```json
{ "kind": "page", "layout": { "mode": "flow" }, "title": "项目工作区" }
```

- `kind`：`root` | `page` | `group`。每个 Workspace 恰有一个 `root`，其 `entity_id` 固定为 `root`，在创建 Workspace 时由后台生成，不可删除、移动、改 `kind`。`root` 的子节点只能是 `page`。
- `layout.mode`：`flow`（子节点按 `order_key` 纵向排列）或 `free`（子节点按 `placement` 摆放）。布局模式只影响展现，不影响树约束。
- `title` 是显示标题；实体公共信封里的 `name` 是可选的查找名。两者是不同的键、不同的版本格。

**树边。** 父子关系只存在于 `tree_edges`，一行一个子节点：`(child_id, parent_id, order_key, placement_json, struct_rev)`。不存在 `children[]` 数组（第一期文档 §2.3“只选一份权威表示”）。

- `order_key`：分数索引字符串（字符集 `[0-9a-z]`，按字节序比较）。在两个兄弟之间插入时取两者之间的键；`core` 提供 `order_key_between(a, b)`，前后端共用。兄弟顺序定义为 `(order_key, child_id)` 的字节序，因此两个客户端并发生成了相同的 `order_key` 也有确定顺序，不需要拒绝。
- `placement`（仅当父容器是 `free` 布局时有意义）：`{ "x": number, "y": number, "w": number, "h": number, "z": integer }`，有限数，`w`/`h` 大于 0。它是“这个子节点在这个父节点里的摆放”，属于边而不是子实体的内容，所以把一个 Cell 移到另一个容器不改变 Cell 的内容 ObjectId。

**哪些实体可以做子节点。** 所有实体都在树中且只有一个父节点（§1 末尾裁决 1）：

| 父 | 允许的子 |
| --- | --- |
| `root` | `page` |
| `page`、`group` | `group`、`cell`、内容实体（`richtext`、`table-source`、`record`、`asset-ref`）、共享 `annotation` |
| `table-source` | 由记录 `body_ref` 拥有的 `richtext`（§3.4.5） |
| 其他 | 无 |

容器渲染时只展现 `cell` 与 `group`；内容实体出现在“数据”大纲里，不直接占据版面。这样“同一 TableSource 被多个 Cell 引用”不需要多个父节点。

**操作。**

| 操作 | 参数 | 检查 |
| --- | --- | --- |
| `entity.create` | 见 §2.5.3 | 父存在、未删除、允许该子类型；ID 未被占用 |
| `tree.move` | `entity_id`、`new_parent_id`、`order_key`、可选 `placement`、可选 `expect`（对 `struct_rev`） | 自动合并，规则见下 |
| `tree.place` | `entity_id`、可选 `order_key`、可选 `placement`、可选 `expect`（对 `struct_rev`） | 仅改摆放与顺序，不改父；自动合并，规则见下 |
| `entity.rename` | `entity_id`、`name`（可为 null 表示清除）、`expect`（对 `meta_rev`） | 见下方命名规则 |
| `entity.delete` | `entity_id`、`subtree`: `"reject_if_children"` \| `{ "delete": [<entity_id>...] }`、`expect`（对 `life_rev`） | 见下方删除规则 |
| `entity.restore` | `entity_id`、`expect`（对 `life_rev`） | 父仍存在且未删除，否则 `REFERENCE_BROKEN` |

**树操作的合并规则。** `tree.move` 与 `tree.place` 不因别人的并发修改而冲突，按后台接受顺序逐个生效：

| 并发情况 | 结果 |
| --- | --- |
| 两人移动或摆放同一节点 | 都被接受，后到者的位置是最终位置。两个 Commit 都在历史中 |
| 两人向同一父节点插入或移入不同节点 | 都被接受；顺序由 `(order_key, child_id)` 决定 |
| 移动会成环（A 移到 B 下、B 移到 A 下并发） | 先到者被接受；后到者在规划时发现成环 → `rejected`，`INVALID_OPERATION` 子码 `TREE_CYCLE` |
| 被移动的节点已被删除 | `TARGET_DELETED`，不复活 |
| 目标父节点已被删除，或不允许该子类型 | `TARGET_DELETED` / 子码 `CHILD_NOT_ALLOWED` |
| 移入后与新父节点下的兄弟重名 | 子码 `NAME_CONFLICT`（名字唯一性不能靠后写者胜解决） |
| 目标父节点或被移动节点所在的父节点开启了写锁而调用方不持有 | §2.11 |

- 调用方不带 `expect` 时即上述语义，交互式的拖动、排序都应不带。带了 `expect` 则照常校验 `struct_rev`，用于两类调用方：撤销生成的逆操作（§2.7 第 6 条），以及明确要求“只有在没人动过时才移动”的程序。
- `struct_rev` 仍然在每次移动、摆放时更新，变化流的 `moved`/`placed` 事件照常产生。
- 被拒绝的情况都是“结果非法”，不是“有人抢先”。离线期间排队的移动在重连变基时按同样规则重新规划，成环或目标已删除的进入待处理区（§6.3）。
- 改名、删除、容器自身的键（`layout`、`title`）不属于自动合并，仍用版本校验。

**命名与路径。** `name` 可选；非空时在同一父节点下唯一（区分大小写，按原样比较，不做 Unicode 归一化），字符限制：1–128 个 Unicode 标量值，不含 `/` 和控制字符。路径形如 `/页面名/表名`，只用于查找：`resolve_path` 返回 `entity_id`，任何持久绑定保存的都是 `entity_id`。同名冲突返回 `INVALID_OPERATION`（子码 `NAME_CONFLICT`）。`title` 不要求唯一。

**删除规则。**

1. 删除是 tombstone：`entities.deleted_seq` 置为本次 `seq`，行保留。被删实体的 `read` 返回 `TARGET_DELETED`。
2. `subtree: "reject_if_children"`（缺省）下有未删除子节点即拒绝并返回子节点列表。要连同子树删除，调用方必须**显式列出它打算删除的全部后代** `{ "delete": [...] }`；后台计算实际子树，若其中有不在清单里的节点 → `REVISION_CONFLICT`，返回多出来的节点。因为移动是自动合并的，别人可能刚把一个对象拖进这个容器；显式清单保证没有人会因此删掉自己没见过的东西。清单中已不在该子树下的节点（被人移走了）被忽略，不被删除。
3. 引用检查：被删集合之外的实体若通过 `bind`、`embed`、`value`、`body` 引用了被删集合中的实体，拒绝并返回 `REFERENCE_BROKEN` 与引用方列表（来源实体 + 选择器）。`anchor` 引用（注释）不阻止删除，注释随后显示“目标已删除”。调用方要么先解除引用，要么把引用方纳入同一批删除。
4. 对已删除实体的迟到写入返回 `TARGET_DELETED`，写入不被应用，实体不被复活（V11）。只有显式 `entity.restore` 能恢复。
5. 第一期不做物理清除。

**撤销。** 全部 `Compensable`：`create` ↔ `delete`，`delete` ↔ `restore`（子树删除的逆操作恢复同一批实体），`move`/`place`/`rename` 的逆操作是带原值和 `expect: { "rev": S }` 的同名操作（§2.7 第 6 条）。

**物化。** `content` = payload 原样。子节点列表不进入 Container 的内容对象，只在结构索引里（§2.10.2）。

**Fixtures。** 两人并发移动同一节点（都接受，后到者定位置，先到者撤销时报冲突）；并发成环（恰好一个被拒绝）；移动到已删除父；移动与删除竞争；删除子树时有人刚移入新节点（冲突并列出该节点）；删除被引用源；并发相同 `order_key`；移入导致重名；根不可删除。

### 3.2 RecordObject：有 schema 的小型业务对象

**用途与边界。** 一个独立的、属性数量有限的业务对象（一组配置、一张名片、一个指标卡的数据）。它的作用是用最简单的类型验证“schema → 属性编辑 → 命令 → 版本”的完整闭环。多条同构数据应使用 TableSource，不是一组 RecordObject。

```json
{
  "schema": {
    "properties": [
      { "key": "owner",    "name": "负责人", "type": "text", "required": true },
      { "key": "deadline", "name": "截止",   "type": "date" },
      { "key": "budget",   "name": "预算",   "type": "decimal", "scale": 2 }
    ]
  },
  "props": { "owner": "林", "deadline": "2026-10-20", "budget": "1200.00" }
}
```

- 属性的值类型、规范化和校验规则与表格字段**完全共用** §3.4.2 的实现。`key` 的字符集同 `field_id`。
- 【裁决】schema 内联在对象里，第一期不建独立的 Schema 注册表实体。把共享 Schema 提升为可引用实体是后续工作，届时 `schema` 键允许换成引用；现在的内联形式是其退化情形。
- 为使属性各自独立并发，`props` 的存储按属性拆开：键式文档的顶层键是 `schema` 和 `p:<key>`（每个属性一个顶层键）。对外读写呈现为上面的嵌套形式，由适配器转换。

**操作。** 通用的 `entity.set_keys` / `entity.unset_keys`：

```json
{ "op": "entity.set_keys", "entity_id": "project-info",
  "keys": [ { "key": "p:owner", "value": "王", "expect": { "rev": 4 } } ] }
```

适配器在 `plan` 中校验：`p:<key>` 必须在 schema 中声明，值符合类型；修改 `schema` 键时，校验现有属性值在新 schema 下仍然合法，否则拒绝（第一期 RecordObject 不提供迁移，改类型的正确做法是新增属性、写入、删除旧属性）。删除 schema 中的属性必须在同一操作中 `unset` 对应的 `p:<key>`。

**冲突。** 不同属性互不冲突；同一属性旧基准竞争 → `REVISION_CONFLICT`；`schema` 键与属性写入竞争 → 属性写入方的 `plan` 在新 schema 下重新校验，不合法则 `SCHEMA_CONFLICT`。

**物化。** `content` = `{ "schema": ..., "props": { ... } }`（嵌套形式）。

### 3.3 RichText

#### 3.3.1 选型结论

【裁决】默认路线：**ProseMirror 文档模型 + Loro**（Rust crate `loro` 与 npm `loro-crdt` 是同一份 Rust 内核，二进制格式自 1.0 起声明稳定）。编辑器绑定以 `loro-prosemirror` 为起点。依据（2026-10-04 调研，版本以实际引入时为准）：

| 需求 | Loro | Yjs/yrs |
| --- | --- | --- |
| 后台在隔离分叉上校验候选更新 | `LoroDoc::fork()`；`import` 返回 `ImportStatus { success, pending }`，`pending` 非空即缺依赖 | 无 fork；需新建文档并全量导入状态，再 `apply_update` |
| 前后端同一实现 | 是（JS 包是 Rust 内核的 WASM 构建） | 否（yrs 是 Yjs 的 Rust 重新实现） |
| 回滚 | 无（官方文档明确不是 ACID 事务） | 无 |
| 稳定相对位置 | `Cursor` | `StickyIndex` |
| 本地撤销 | `UndoManager`，只撤销绑定 peer 的本地操作，可按 origin 前缀排除 | `UndoManager`，按 origin 过滤 |
| 编辑器绑定成熟度 | `loro-prosemirror` 0.4.x，社区小，有未关闭的正确性 issue | `y-prosemirror` 1.3.x 久经使用；但其主干正在向 Yjs v14 的新映射迁移 |

两条路线都**没有**事务回滚，所以“SQLite 是原子性边界、CRDT 文档是缓存”的管线（§3.3.4）与选型无关。Loro 的主要风险集中在绑定层，本文通过把映射成文（§3.3.3）并自带双端 fixtures 来对冲：绑定可以被替换或自行维护，映射规范和后台实现不变。

【探针】M1 必须用代码回答第一期文档 §6.3 的六个问题，并额外回答：(a) Rust 端按 §3.3.3 解码前端产生的文档，与前端 `doc.toJSON()` 逐字段一致；(b) Rust 端执行块级操作后，前端编辑器正确显示且可继续编辑；(c) npm 与 crates.io 的版本不同步时（调研时为 1.16.4 与 1.16.2）跨版本导入导出正常；(d) 导入失败时文档状态不被部分修改（上游在 1.16.4 的发布说明中声称导入具有原子性，需在实际使用的 crate 版本上验证；本设计不依赖它，因为失败的导入只发生在分叉上）。**退路**：若 (a)(b) 因绑定缺陷无法在 M1 时间盒内达成，改用 yrs + `y-prosemirror` 1.3.x，§3.3.3 改写为 `XmlFragment` 映射，§3.3.4–3.3.8 的协议不变（`fork` 用“新文档导入全量状态”替代）。

不使用 Loro 的可移动树承载 Workspace 结构树：树由 Command Engine 以版本格管理（第一期文档 §6.2 的较小组合）。

#### 3.3.2 文档 schema：`buckyos.richtext.basic.v1`

schema 定义文件放在 `src/frame/aiworkspace/schemas/richtext.basic.v1.json`，前端由它生成 ProseMirror `Schema`，后台由它驱动校验器。两端不各写一份。

| 节点 | 内容 | 属性 |
| --- | --- | --- |
| `doc` | `block+` | 无 |
| `paragraph` | `inline*` | `block_id` |
| `heading` | `inline*` | `block_id`、`level`（1–3） |
| `bullet_list` | `list_item+` | `block_id` |
| `ordered_list` | `list_item+` | `block_id`、`start`（≥1，缺省 1） |
| `list_item` | `paragraph (paragraph \| bullet_list \| ordered_list)*` | `block_id` |
| `object_embed`（块级原子） | 无 | `block_id`、`ref`（§2.4 引用，目标必须是 `buckyos.cell`） |
| `text` | | |
| `hard_break`（行内原子） | | |
| `object_link`（行内原子） | 无 | `ref`（任意实体引用）、`label`（引用失效时的回退文本） |

Marks：`strong`、`em`、`strike`、`code`、`link`（`href`，只允许 `http:`、`https:`、`mailto:` 和以 `#` 开头的文内锚）。`code` 与其他 mark 互斥。

`block_id` 规则：

- 所有带 `block_id` 的节点在一个文档内唯一，格式同 §2.3。
- 由前端插件负责分配：新建块、回车拆分出的后半块、粘贴进来的块（无论来源是否带 ID，一律重新分配）。拆分时前半块保留原 ID。合并两块时保留前一块的 ID。
- 后台把重复或缺失的 `block_id` 当作 `INVALID_SCHEMA` 拒绝，不代为修补。并发合并导致重复（两人对同一块做了会复制 ID 的操作）时，先到者被接受，后到者被拒绝并在客户端触发“以确认状态为准重建工作文档”（§3.3.7）。

限额（缺省值，可配置）：单文档块数 ≤ 5000，文本总量 ≤ 500 000 个 UTF-16 码元，嵌套深度 ≤ 8，单次更新 ≤ 1 MiB。

#### 3.3.3 ProseMirror ↔ Loro 映射（协议的一部分）

沿用 `loro-prosemirror` 0.4.x 的容器布局，并在此成文。绑定升级若改变下列任何一条，视为协议变更，必须同步更新本节、Rust 编解码与 fixtures，并提升 `richtext_states.encoding`。

- 文档根：`LoroDoc` 的根容器 `LoroMap`，键名 `"doc"`。
- 每个非文本节点是一个 `LoroMap`，含三个键：`"nodeName"`（字符串）、`"attributes"`（`LoroMap`，属性名 → JSON 值）、`"children"`（`LoroList`）。
- `children` 的元素是子节点的 `LoroMap`，或承载连续行内文本的 `LoroText`。
- marks 以 `LoroText` 的富文本属性表示：`{ <markName>: <mark 属性对象或 true> }`。
- 行内原子节点（`hard_break`、`object_link`）是 `children` 列表中的**独立 `LoroMap`**，把所在的文本流切成前后两个 `LoroText`；连续的文本节点合并为一个 `LoroText`。【探针结论，2026-10-05】依据 `loro-prosemirror` 0.4.4 `src/lib.ts` 的 `normalizeNodeContent`/`createLoroMap`，并经实测：Rust 建的 `n-intro` 段在 npm 端读作 `[LoroText, LoroMap(object_link), LoroText]`，绑定写入的含 `hard_break` 段为 `[LoroText, LoroMap(hard_break), LoroText]`。Rust 端 `core/src/richtext.rs` 按此编解码。§3.3.1 的 (a)(b)(c) 已由 Desktop 端到端测试证实（npm 1.16.4 ↔ crate 1.16.2 双向），未启用退路；(d) 不被依赖，未单独验证。
- mark 的值是该 mark 的 attrs 对象（无属性时为空对象 `{}`）；节点 attrs 中的 `null` 不写入 `attributes`，schema 缺省值照常写入（规范 AST 再把缺省值省略）。mark 的扩展方式与绑定一致：schema 中 `inclusive: false` 的 mark（`code`、`link`）为 `none`，其余为 `after`；两端必须用同一配置。

`core::richtext` 提供：

```rust
pub fn decode_ast(doc: &LoroDoc) -> Result<Ast, WsError>;                 // 容器 → AST
pub fn validate_ast(ast: &Ast, schema: &RichTextSchema, limits: &Limits) -> Result<BlockIndex, WsError>;
pub fn build_doc(ast: &Ast, peer: PeerId) -> Result<LoroDoc, WsError>;     // AST → 新 lineage
pub fn apply_block_ops(doc: &LoroDoc, ops: &[BlockOp]) -> Result<(), WsError>;   // 在给定文档上执行块级操作
pub fn diff_blocks(base: &Ast, target: &Ast) -> Vec<BlockOp>;              // 草稿提交用
```

`BlockIndex` 是 `block_id → { parent, node_type, hash, struct_rev }`：

- `hash = sha256(JCS(该块节点的 AST，含子节点))` 的十六进制前 32 位，是块**内容**的版本令牌。
- `struct_rev` 是块**位置**的版本令牌：最后一次改变该块位置的 Commit 的 `seq`。后台在接受每个富文本 Commit 时比较前后两个 `BlockIndex` 来更新它：新出现的块取本次 `seq`；`parent` 变了的块取本次 `seq`；`parent` 未变时，对该父节点的子块序列（只看前后都存在的子块）求最长公共子序列，不在其中的块视为被移动，取本次 `seq`；其余块保持原值。因此在某块旁边插入或删除别的块不会改变它的 `struct_rev`，只有它自己被移动才会。这个比较对 `apply_update`（编辑器里拖动块）和块级操作一视同仁，是 `core` 中的确定性函数（并列时取字典序最小的公共子序列下标组合），前后端结果一致。
- `struct_rev` 是工作状态令牌，保存在 `richtext_states.block_index_json`，不进入内容对象；导入和 Fork 后全部取导入提交的 `seq`。

二者共同构成块级 OCC 的令牌（§3.3.5）。

双端 fixtures（`fixtures/richtext/`）：每个用例含 (1) 前端生成的 Loro 快照字节，(2) 前端 `doc.toJSON()`，(3) 期望的规范 AST。Rust 测试断言 `decode_ast(1) == 3`；前端测试断言由 (1) 重建的编辑器文档 JSON 等于 (2)。用例至少覆盖：中文与 emoji（代理对）、相邻与重叠 marks、三层嵌套列表、空段落、`object_embed`、`object_link`、`hard_break`、仅含一个空段落的文档。

#### 3.3.4 `richtext.apply_update`：CRDT 更新成为一次 Commit

```json
{ "op": "richtext.apply_update", "entity_id": "project-notes",
  "lineage_id": "ln_7f3k...", "update": "<base64：Loro Updates 导出字节>" }
```

写执行者对它的处理（对应 §2.5.2 的步骤）：

1. **规划**（不触碰权威状态）：
   - 实体存在、未删除、`lineage_id` 与库中一致，否则 `TARGET_DELETED` / `INVALID_OPERATION`（子码 `LINEAGE_MISMATCH`）。
   - `candidate = authoritative_doc.fork()`。同批内此前的操作若已修改过该文档，则在覆盖层保存的候选文档上继续。
   - `status = candidate.import(update)`。解码失败 → `INVALID_OPERATION`。`status.pending` 非空 → `rejected`，`code: "BASE_UNKNOWN"`，`retryable: true`：客户端基于后台没有的操作，应先追赶变化流再重发。
   - `ast = decode_ast(candidate)`；`validate_ast` 检查 schema、限额、`block_id` 唯一。
   - 引用检查：新出现的 `object_embed`/`object_link` 目标必须存在、未删除、类型符合，且主体对目标有 `read`。
   - 产出 `StoreWrite::PutRichTextUpdate { entity_id, update_bytes }`、新的 AST 投影与 `BlockIndex`、`ref_changes`（由前后 AST 的引用集合差得到）、`touched`（变化的 `block_id` 列表，类别 `text`）。
2. **事务**：与同批其他操作的写入一起，在一个 SQLite 事务中写入 `richtext_updates` 行、更新 `richtext_states` 的 AST 投影列和实体 `content_rev`。
3. **发布**：事务提交后，把 `candidate` 换入为新的权威内存文档。事务失败或同批任一操作失败：丢弃 `candidate`，权威文档从未被修改。

这就是第一期文档 §4.4 要求证明的性质：候选校验失败时，权威 CRDT、持久化状态、已广播状态三者均未被污染。**持久化的权威表示是 `richtext_states.snapshot` + 其后的 `richtext_updates` 行**；内存文档是它的缓存，进程重启后按 `snapshot` 导入、再按 `seq` 顺序导入各更新即可重建。由于 CRDT 导入满足交换与幂等，重建结果与崩溃前的内存文档一致。

`richtext.apply_update` 的操作类别是 `Merge`：它不携带 `expect`，不与其他人的并发文本修改冲突。它仍然会因为实体被删除、schema 不合法、权限不足而被拒绝。

**混合批次的原子性（V10）**：一个 Commit 同时含 `richtext.apply_update`、`table.set_values`、`tree.move`，最后一步失败 → 第 1 步的 `candidate` 被丢弃，前面操作的 `StoreWrite` 只存在于覆盖层，也被丢弃。测试断言：数据库无变化、`head_seq` 不变、内存文档导出字节不变、变化流无新事件。

#### 3.3.5 块级语义操作

用于三个场景：Agent/受控加工修改富文本；显式草稿提交；后台补偿撤销。它们是 `Compensable` 的 `Overwrite` 操作，冲突粒度是块。

```json
{ "op": "richtext.insert_blocks", "entity_id": "summary",
  "position": { "after": "blk-intro" },
  "blocks": [ { "type": "paragraph", "attrs": { "block_id": "blk-s1" },
                "content": [ { "type": "text", "text": "本周完成 3 项，逾期 1 项。" } ] } ] }

{ "op": "richtext.replace_block", "entity_id": "summary",
  "block_id": "blk-s1", "expect": { "hash": "9f2c..." },
  "node": { "type": "paragraph", "attrs": { "block_id": "blk-s1" }, "content": [ ... ] } }

{ "op": "richtext.delete_blocks", "entity_id": "summary",
  "blocks": [ { "block_id": "blk-s1", "expect": { "hash": "9f2c...", "struct_rev": 17 } } ] }

{ "op": "richtext.move_block", "entity_id": "summary",
  "block_id": "blk-s1", "expect": { "struct_rev": 17 }, "position": { "before": "blk-end" } }
```

- `position`：`{ "after": <block_id> }`、`{ "before": <block_id> }`、`{ "first_child_of": <block_id> }`，或 `{ "start": true }` / `{ "end": true }`（文档顶层）。锚点块不存在 → `REFERENCE_BROKEN`。
- 各操作必须携带的期望：

  | 操作 | `expect.hash` | `expect.struct_rev` | 理由 |
  | --- | --- | --- | --- |
  | `replace_block` | 必须 | 不需要 | 改内容不关心它被挪到了哪里 |
  | `move_block` | 不需要 | 必须 | 两人基于同一位置移动同一块时，后到者的 `struct_rev` 已变，冲突；内容被别人改过不妨碍移动 |
  | `delete_blocks` | 必须 | 必须 | 不删除别人刚改过或刚移动过的块 |

  不等 → `REVISION_CONFLICT`，响应中给出当前令牌与（有读权限时）当前块 AST 及其当前位置。块不存在 → `TARGET_DELETED`。
- `replace_block` 的 `node.attrs.block_id` 必须等于被替换块的 ID；节点类型可以改变（段落改标题）。
- 后台执行方式：在 `candidate` 分叉上调用 `apply_block_ops`，使用后台保留的 peer 身份。执行产生的 CRDT 更新从分叉导出，作为 `PutRichTextUpdate` 落盘，并放入响应的 `server_ops` 和变化流事件的 `ops`，客户端把它导入自己的确认文档。后续校验与 §3.3.4 相同。
- 逆操作（设原 Commit 的顺序号为 `S`）：`insert_blocks` ↔ `delete_blocks`（期望插入后的 hash 与 `struct_rev = S`）；`replace_block` ↔ 以旧节点为内容的 `replace_block`（期望替换后的 hash）；`move_block` ↔ 移回原位置的 `move_block`（期望 `struct_rev = S`，原位置用移动前的前一个兄弟或父节点表示）；`delete_blocks` ↔ 在原位置的 `insert_blocks`（锚点取删除前的前一个兄弟，不存在则父的 `first_child_of`）。逆操作引用的锚点块已不存在 → `REFERENCE_BROKEN`，按 §2.7 作为该撤销的冲突项报告。于是“撤销我的移动”在别人又移动过该块之后会报告冲突，而不是把块拉回去。

**显式草稿提交。** 客户端在私有分叉上编辑（草稿的 CRDT 历史永不离开客户端）。提交时：`ops = diff_blocks(基准 AST, 草稿 AST)`，每个被替换或删除的块带上基准时的 `hash`，形成一个 Commit。对端只看到块级结果，看不到草稿的四次中间修改（V14）。代价是显式模式下同一块的并发修改表现为冲突而不是字符级合并；这是有意的取舍，第一期文档 §4.2 要求的正是“共享分支重新应用，冲突时保留草稿”。提交被接受后，客户端丢弃草稿分叉，从确认文档重新分叉。

`diff_blocks` 的规则：按 `block_id` 对齐；两侧都有且 hash 相同的块不产生操作；容器块（列表、列表项）若仅子块变化，则递归到子块而不替换整个容器；同一父节点下顺序变化产生 `move_block`。它是确定性纯函数，放在 `core`，fixtures 给出 (基准, 目标, 期望操作序列)。

#### 3.3.6 读取与 AST 投影

- `read`（选择器 `entity`）返回：`{ "editor_schema": "buckyos.richtext.basic.v1", "content": <AST>, "content_rev": N, "blocks": { <block_id>: { "hash": ... } } }`。AST 来自 `richtext_states.ast_json`，它总是由后台在接受 Commit 时从候选文档解码得到，与 `content_rev` 严格对应。客户端上传的 AST 不被存储也不被信任。
- `read`（选择器 `richtext_block`）返回单个块的 AST 与 hash。
- `get_collab_state(entity_id)` 返回 `{ lineage_id, engine, encoding, snapshot: <base64>, seq }`，供编辑器建立确认文档。它需要 `read` 能力，并且**只**通过这个专用接口提供，不混入普通 `read`，也不进入普通导出。

#### 3.3.7 客户端文档管理

每个打开的富文本，客户端持有两个 Loro 文档：

- **确认文档**：只导入已被后台接受的更新（自己的和变化流里别人的）。
- **工作文档**：`确认文档.fork()` 之后叠加本地未确认的编辑，绑定到编辑器。

流程：编辑器事务 → 工作文档产生本地更新 → 防抖合并 → 作为 `richtext.apply_update` 提交。接受 → 把同一更新导入确认文档。收到别人的更新 → 导入确认文档和工作文档（CRDT 合并，编辑器选区由绑定保持）。**被拒绝**（schema 不合法、目标已删除、权限被撤回）→ 把工作文档当前的 AST 存为可恢复草稿（本地持久化，用户可查看和导出），丢弃工作文档，从确认文档重新分叉并重建编辑器状态，界面明确提示“修改未被接受，已保留为草稿”。不能让工作文档长期携带后台永远不会接受的操作。

本地撤销：每个编辑器一个 Loro `UndoManager`，绑定工作文档，排除来源为远端导入的操作。它产生的撤销也是普通本地更新，走同一条提交路径。由 §2.7 的 UndoCoordinator 决定何时调用它。

#### 3.3.8 存储、lineage 与历史

- `lineage_id` 标识一条协作历史。创建富文本时生成；`richtext_states` 记录 `engine = "loro"`、实际使用的库版本、`encoding`（映射规范版本，第一期为 `pm-loro-1`）。
- 创建：`entity.create` 的 payload 二选一：`{ "content": <AST> }`（后台用 `build_doc` 以后台 peer 建立文档）或 `{ "lineage_id": ..., "snapshot": <base64> }`（离线创建时由客户端用同一个 `core` 构建；后台解码并完整校验后接受）。
- 存储合并：当某实体自上次快照以来的更新超过 200 条或累计超过 1 MiB，写执行者在空闲时把当前权威文档导出为**完整历史**快照（`ExportMode::Snapshot`）写回 `richtext_states.snapshot`，并删除已包含的 `richtext_updates` 行。这只改变存储形态，不丢失历史，不产生新的 `seq`，在一个事务内完成。
- 【裁决】**第一期不裁剪 CRDT 历史**（不使用浅快照）。Loro 的浅快照会使早于裁剪点的对端无法合并，需要另行设计陈旧客户端的拒绝与重放协议；这与 V06“完整恢复后可继续合并原 lineage 增量”直接冲突。磁盘增长作为第一期文档 §8.3 的测量项记录，裁剪留到有数据之后决定。`BASE_TOO_OLD` 错误码为此预留。
- 变化流中的富文本更新字节保存在 `commit_ops.op_json`（对 `apply_update` 是请求中的原始字节，对块级操作是后台生成的字节）。存储合并不删除它们；它们的保留期随变化流（第一期不清理）。

#### 3.3.9 物化、导出与 Fork

- 内容对象：`{ "ws_type": "buckyos.richtext", "schema_version": 1, "content": { "editor_schema": "buckyos.richtext.basic.v1", "doc": <AST> } }`。规范 AST 的确定性规则：`attrs` 中等于 schema 缺省值的属性省略；`marks` 按 mark 名字节序排序；相邻且 marks 相同的 `text` 节点合并；空 `content` 数组省略。`decode_ast` 直接产出这个规范形式。AST 的 canonical JSON 超过 256 KiB 时，`doc` 改为 `{ "encoding": "json+jcs", "file": "<FileObject ObjId>" }`。
- **普通分享导出**只含内容对象，不含任何 CRDT 字节。导入方用 `build_doc` 建立**新的 lineage**。因此被删除的文字、作者、编辑顺序都不会随包泄露（V14）。Loro 的“仅状态”导出仍然携带原 peer 与操作 ID，不能当作新 lineage 使用。
- **Fork**：与普通导入相同，新 Workspace 中的富文本是新 lineage。`block_id` 保持不变，因此锚定在块上的注释和 `object_embed` 引用继续有效。
- **个人恢复备份**：额外包含 `collab/<entity_id>.loro`（完整历史快照）和 `lineage_id`；恢复到同一 `workspace_id` 时沿用原 lineage，离线客户端此后提交的旧更新仍可合并（V06）。
- 锚点重映射：第一期注释只锚定到 `block_id`（§3.7），不依赖 CRDT 相对位置，所以新 lineage 不需要重映射。字符范围锚点的结构已保留但未实现，实现时必须同时实现重映射或显式失效。

#### 3.3.10 Fixtures 与测试要点

映射一致性（§3.3.3）；两个客户端离线各自编辑后以两种顺序提交、并重复提交其中一个更新，最终 AST 相同（V07）；缺依赖更新返回 `BASE_UNKNOWN`；重复 `block_id` 被拒绝且权威状态不变；混合批次回滚（V10）；块级操作 hash 冲突；`diff_blocks` 往返（把结果操作应用到基准得到目标）；进程在事务提交后、发布前被杀死，重启后内存文档与数据库一致；普通导出包中搜索不到已删除的文本片段。

### 3.4 TableSource

#### 3.4.1 模型

普通内置表 = 一组有稳定 ID 和类型的字段 + 一组有稳定 ID 的记录。不存在行号、列号。单元格不是实体，用选择器 `table_cell` 寻址。本节至 §3.4.10 定义默认内置存储路径；明确的大数据场景可用 §3.4.11 的 URL 查询路径，复用 schema 和 TableView，无需为记录集生成 ObjectId。

实体 payload（键式文档，只含表级元数据）：

```json
{ "title_field_id": "title", "description": "项目任务" }
```

字段定义一行一个，存于 `table_fields`：

```json
{ "field_id": "status", "name": "状态", "type": "select",
  "required": false, "nullable": false,
  "options": [ { "option_id": "option-open", "label": "进行中" },
               { "option_id": "option-done", "label": "完成" } ],
  "maintained_by": "human" }
```

公共字段属性：`field_id`、`name`（同表内唯一，可改）、`type`、`required`（缺省 false）、`nullable`（缺省 false）、`unique`（缺省 false，仅 `text`/`number`/`decimal`/`date` 可设）、`description`、`maintained_by`（`human` | `program`，缺省 `human`，见 §3.4.7）。字段顺序是 `table_fields.order_key`（表的缺省列序；视图可以各自覆盖）。

记录一行一条，存于 `table_records`：`values_json`（`field_id → 值`）、`revs_json`（`field_id → rev`）、`meta_json`（`field_id → 单元格元数据`，见 §3.4.7）、可选 `body_ref`。

`title_field_id` 指向一个 `text` 字段；记录的显示标题从它读取，不另存副本。

#### 3.4.2 值类型与规范形式

`core::value` 是唯一实现，表格字段和 RecordObject 属性共用。**写入时规范化，存储的永远是规范形式**；不合法输入被拒绝，不做“尽力转换”。

| 类型 | JSON 形态 | 规范化与校验 |
| --- | --- | --- |
| `text` | string | 原样保存，不做 Unicode 归一化、不裁剪空白。长度 ≤ 65 536 个 UTF-16 码元。空字符串是合法值，与未设置不同 |
| `boolean` | boolean | |
| `number` | number | 有限 IEEE-754 双精度。`-0` 规范化为 `0`。**整数绝对值超过 2^53 − 1 拒绝**（提示改用 `decimal`） |
| `decimal` | string | 字段声明 `scale`（0–18）。输入匹配 `^-?\d+(\.\d+)?$`，有效数字 ≤ 38 位，小数位数 ≤ `scale`，否则拒绝（不舍入）。规范形式：去掉多余前导零，小数部分补零到恰好 `scale` 位，负零写作正零，如 `"1200.00"` |
| `date` | string | `YYYY-MM-DD`，公历有效日期，年份 0001–9999 |
| `datetime` | string | 表示时间点。输入须带时区偏移或 `Z`；规范形式为 UTC、毫秒精度、`Z` 结尾：`2026-10-04T08:00:00.000Z`。无偏移的输入被拒绝。显示时区是视图配置，不是值的一部分 |
| `select` | string | 必须是该字段现有的 `option_id` |
| `multi_select` | array of string | 元素须为现有 `option_id`；去重；规范顺序为 `option_id` 字节序（显示顺序按字段的 options 顺序，与存储顺序无关） |
| `object_ref` | object | §2.4 引用结构；字段可声明 `target_types`（允许的目标 `type_id` 列表）；目标须存在且未删除 |

`date` 与 `datetime` 之间没有隐式转换。`number` 与 `decimal` 之间没有隐式转换。JSON number 中的大整数限制与 canonical 编码一致：ndn-lib 使用的 `serde_jcs` 对超过 2^53 的整数按原样输出，与 JS 端按双精度格式化的结果不同，所以在值层面直接禁止。

**三种“空”**：

| 状态 | 存储 | 产生方式 | 约束 |
| --- | --- | --- | --- |
| 未设置 | `values_json` 中无此键 | 插入时未提供；`table.unset_values` | `required: true` 的字段不允许 |
| 显式空 | 键存在，值为 `null` | `table.set_values` 写入 `null` | 仅 `nullable: true` 的字段允许 |
| 空文本 | `""` | 写入 `""` | 只是一个普通文本值 |

`required` 约束“必须有键”，`nullable` 约束“值可否为 null”，两者独立。UI 中“清空单元格”对应 `unset_values`（字段非必填时）。

#### 3.4.3 记录与单元格操作

```json
{ "op": "table.insert_records", "source_id": "tasks",
  "field_type_revs": { "title": 1, "status": 3 },
  "records": [ { "record_id": "task-43", "values": { "title": "写验收用例", "status": "option-open" } } ] }

{ "op": "table.set_values", "source_id": "tasks",
  "field_type_revs": { "status": 3 },
  "values": [ { "record_id": "task-42", "field_id": "status", "value": "option-done", "expect": { "rev": 8 } } ] }

{ "op": "table.unset_values", "source_id": "tasks",
  "values": [ { "record_id": "task-42", "field_id": "due", "expect": { "rev": 5 } } ] }

{ "op": "table.delete_records", "source_id": "tasks",
  "records": [ { "record_id": "task-41", "expect": { "rev": 12 } } ], "owned_bodies": "delete" }
```

- `field_type_revs`：调用方声明它写值时依据的各字段 `type_rev`。与当前不等 → `SCHEMA_CONFLICT`（“字段改类型与基于旧 schema 写入竞争”）。字段改名、改描述、新增选项只改 `def_rev`，不使待提交的值写入失效。
- `set_values` 的每一项独立携带 `expect`（对 `value_revs[field_id]`；从未写过为 0）。一个操作可含多条记录多个字段，是批量写的基本形式；1000 行批量修改是一个操作里的 1000 项。
- 记录已删除 → `TARGET_DELETED`。记录不存在（从未创建）→ `NOT_FOUND`。
- `insert_records`：`record_id` 已存在（含已删除）→ `ID_CONFLICT`。插入时对所有 `required` 字段检查。新记录各已提供字段的 `value_revs` = 本次 `seq`。
- `delete_records` 的 `expect` 针对 `record.rev`：记录的任何单元格在调用方读取后被别人改过，删除就冲突。这是有意的保守选择（不删除别人刚改过的记录）。删除是 tombstone（`deleted_seq`），记录行保留以支持撤销和迟到写入的正确报错。
- `unique` 字段：在事务内检查未删除记录中无重复值（未设置和 `null` 不参与）。冲突 → `rejected`，`code: "INVALID_OPERATION"`，子码 `UNIQUE_VIOLATION`，附冲突的 `record_id`。它在权威提交边界检查，不依赖单元格 OCC（第一期文档 §4.5 末行）。
- 仅有 `append` 能力的主体只能执行 `insert_records`，且不能通过 `body_ref`、`object_ref` 值指向它无权读取的实体。

逆操作：`insert_records` ↔ `delete_records`；`set_values`/`unset_values` ↔ 以旧值恢复的 `set_values`/`unset_values`（旧状态是“未设置”则用 `unset`）；`delete_records` ↔ 内部操作 `table.restore_records`（恢复 tombstone，仅由撤销路径生成，不对外开放）。

#### 3.4.4 字段与选项操作

| 操作 | 要点 |
| --- | --- |
| `table.add_field` | 完整字段定义 + `order_key`。`required: true` 的新字段必须带 `backfill` 值（对所有现有记录写入，计入同一 Commit），否则拒绝 |
| `table.update_field` | 可改 `name`、`description`、`order_key`、`maintained_by`、`required`、`nullable`、`unique`。`expect` 对 `def_rev`。收紧约束（改为必填、不可空、唯一）时扫描现有值，有违例即拒绝并返回违例 `record_id`（最多 100 个 + 总数）；收紧成功则提升 `type_rev` |
| `table.delete_field` | `expect` 对 `def_rev`。字段 tombstone；各记录中的值**保留在存储中但不再可读写**，使撤销可以无损恢复（逆操作是下方的内部操作 `table.restore_field`）。被 Cell/TableView 引用时不阻止删除，响应的 `touched` 列出受影响的视图（§3.5.4）。`title_field_id` 指向的字段不可删除 |
| `table.add_option` | `field_id`、`option`（`option_id` + `label`）、可选位置。只提升 `def_rev` |
| `table.update_option` | 改 `label` 或位置。只提升 `def_rev`；业务值不变 |
| `table.delete_option` | 必须给出 `on_values`: `"reject_if_used"`（缺省，返回使用它的记录数）\| `"unset"` \| `{ "replace_with": <option_id> }`。受影响单元格在同一 Commit 中被改写并提升各自 `value_revs`；提升 `type_rev`。选项不留 tombstone，逆操作是在原位置以原 `option_id` 执行 `add_option`，并恢复被改写的单元格 |
| `table.restore_field`（内部） | `delete_field` 的逆操作，见下 |

**`table.restore_field`**：`{ source_id, field_id, expect: { rev: S } }`，其中 `S` 是删除该字段的 Commit 的顺序号。

- 只能由 `doc.undo` 生成；能力要求与 `delete_field` 相同（`structure`）。`add_field` 仍然不能复用已删除字段的 `field_id`，恢复是唯一让 tombstone 字段重新生效的途径。
- 适用性：`def_rev` 仍等于 `S`（删除后没有别的操作动过这个 tombstone）。
- 执行前在当前状态下重新校验，任何一项不满足都作为撤销的冲突项报告，不做静默调整：
  - 字段 `name` 未被其他存活字段占用，否则 `NAME_CONFLICT`；
  - 字段为 `required` 时，所有存活记录都有该字段的残留值（删除期间新插入的记录没有），否则 `SCHEMA_CONFLICT` 并给出缺值记录数；
  - 字段为 `unique` 时，残留值在存活记录中仍然唯一；
  - `object_ref` 残留值的目标仍然存在，否则 `REFERENCE_BROKEN`。
- 效果：清除 `deleted_seq`；`def_rev`、`type_rev`、`values_rev` 取本次 `seq`；各记录中该字段的残留值重新可读写，它们的 `value_revs` 保持删除前的值（删除期间没有任何操作能改动它们）。删除期间基于旧 `type_rev` 排队的写入因此得到 `SCHEMA_CONFLICT`。
- 恢复后，原先引用该字段的视图配置自动重新有效（§3.5.4），因为视图配置从未被改写。
- 它自身的逆操作是 `delete_field`（重做删除）。

字段改名保留 `field_id`，所有按 `field_id` 的引用、选择器、视图配置不受影响（V08）。

#### 3.4.5 记录正文 `body_ref`

记录可带一个指向 RichText 的引用作为详情。该 RichText 是普通实体，结构父节点是这张 TableSource，引用种类为 `body`。创建方式：同一 Commit 中先 `entity.create`（父 = TableSource）再 `table.insert_records` 或 `table.set_body`（`record_id`、`body_ref` 或 null、`expect` 对 `record.rev`）。`delete_records` 的 `owned_bodies: "delete"`（缺省）把这些 RichText 一并标记删除；`"detach"` 保留它们并清除引用。一个 RichText 至多被一条记录用作正文。

#### 3.4.6 字段类型迁移 `table.migrate_field`

```json
{ "op": "table.migrate_field", "source_id": "tasks", "field_id": "due_text",
  "expect": { "rev": 2 },
  "to": { "type": "date" },
  "on_failure": "reject" }
```

第一期必须实现的转换：`text → date`、`text → number`、`text → decimal(scale)`、`number → decimal(scale)`、`select → text`（值变为选项的 `label`）。其他组合返回 `INVALID_OPERATION`（子码 `MIGRATION_UNSUPPORTED`）。

- 转换函数是 `core` 中的纯函数，严格解析：`text → date` 只接受 `YYYY-MM-DD`；`text → number/decimal` 只接受 §3.4.2 的规范输入语法（允许首尾 ASCII 空白）。不猜测本地化格式。
- **预检**：对迁移操作调用 `prepare`，返回报告 `{ total, convertible, failing: { count, sample: [ { record_id, value } ≤100 ] }, unset }`，不修改任何状态。
- `on_failure`：`"reject"`（缺省，有任何失败即整个 Commit 被拒绝）或 `"unset"`（失败的单元格变为未设置；字段是 `required` 时不允许）。
- 执行：在一个事务内改写字段定义和全部受影响单元格，提升 `def_rev`、`type_rev` 和每个被改写单元格的 `value_revs`。事务失败则没有任何单元格被改写（V09“无半张表生效”）。1 万行在单个事务内完成；规模上限由限额控制（缺省 100 000 行），超限返回 `LIMIT_EXCEEDED`，分片迁移留待以后。
- 迁移后，基于旧 `type_rev` 的待提交写入得到 `SCHEMA_CONFLICT`。
- 撤销：`Compensable`，逆操作内部携带迁移前的字段定义和每个被改写单元格的旧值（存于 `commit_ops.inverse_json`；这是第一期最大的逆操作载荷，按每 Commit 逆数据 ≤ 64 MiB 限额，超过则该 Commit 被标记为不可补偿并在提交响应中告知）。适用性检查同 §2.7：任何被改写单元格之后又被修改，撤销报告冲突。

#### 3.4.7 派生字段、来源与人工覆盖

- 字段级：`maintained_by: "program"` 声明该字段通常由加工程序写入。它是提示而非锁；人仍然可以改。
- 单元格级元数据 `meta_json[field_id]`：

```json
{ "derived": { "run_id": "run_...", "program": "mock.task-summary@1",
               "inputs": [ { "entity_id": "tasks", "content_rev": 31 } ] },
  "manual_override": true }
```

- 规则由后台在规划时执行，不信任客户端传入的标志：`origin` 为 `program`/`agent` 且带 `run_id` 的写入 → 写入 `derived`，清除 `manual_override`；`origin` 为 `human` 的写入落在已有 `derived` 的单元格上 → 保留 `derived`（作为历史来源），置 `manual_override: true`。
- `meta_json` 是内容的一部分：进入物化（来源可追溯是文档内容），随值一起被撤销恢复。
- 加工程序刷新时的保护见 §7：它的写入携带读取时的 `expect.rev`，人工修改过的单元格 `rev` 已变，必然冲突，不会被悄悄覆盖。

#### 3.4.8 读取与查询

- `read`（`entity`）：表级元数据 + 字段定义（含 `def_rev`、`type_rev`）+ 记录数 + `content_rev`。不含记录。
- `read`（`table_record`/`table_cell`）：值、`rev`、`meta`。
- `query`：见 §2.5.4 与 §3.5.3。返回行 `{ record_id, values, revs, meta?, body_ref? }`，可用 `fields` 参数限定返回的字段。
- 已删除字段的值不出现在任何读取结果中。

#### 3.4.9 物化

内容对象：

```json
{ "ws_type": "buckyos.table-source", "schema_version": 1,
  "content": {
    "title_field_id": "title",
    "fields": [ <字段定义，按 (order_key, field_id) 排序，不含 rev，不含已删除字段> ],
    "records": { "encoding": "jsonl+jcs", "count": 10000, "file": "<FileObject ObjId>" }
  } }
```

记录文件：每行一条未删除记录的 JCS 编码 `{ "record_id", "values", "meta"?, "body_ref"? }`，按 `record_id` 字节序排序，行尾 `\n`，UTF-8，无 BOM。`values` 不含已删除字段的残留值。空表的 `records` 为 `{ "encoding": "jsonl+jcs", "count": 0 }`，不带 `file`。

这样同一逻辑内容总是得到同一 ObjectId，与插入顺序、存储布局、是否经过导出导入无关。每次物化重写整个记录文件，代价 O(记录数)；物化只发生在检查点，不在每次 Commit。1 万行 × 20 字段的物化耗时是 M2 的测量项，超出预算时可评估按 `record_id` 哈希前缀分桶（未变化的桶复用 FileObject），或在明确的大数据场景采用 §3.4.11 的 URL 查询引用。两者按数据性质和成本选择，不强制为了 ObjectId 先分桶再全量物化；格式结构变化需体现在 schema 版本中。

#### 3.4.10 Fixtures 与测试要点

每种值类型的合法/非法输入表与规范输出（Rust 与 WASM 构建跑同一份表）；三种“空”的往返；同记录异字段并发都成功、同单元格并发冲突、改回原值仍冲突（V11）；删除后迟到写入 `TARGET_DELETED`；唯一约束在并发插入下恰好一个成功；迁移预检与执行、中途注入失败后无任何单元格变化（V09）；字段改名后旧选择器仍可解析（V08）；物化确定性（打乱插入顺序、导出再导入，ObjectId 不变）；1 万行 fixture 的单元格写入不触及其他记录行（通过统计 SQL 写入行数断言）。

#### 3.4.11 URL 查询型大表

**使用范围。** 默认仍采用内置表和 ObjectId；仅在持续更新、全量物化成本过高或应按查询访问的大数据源中使用此模式。适配决策记录规模/成本依据，不能用它绕过普通表的版本、权限与命令要求。

TableSource 增加显式 `data_mode`：`embedded`（缺省）或 `url_query`。后者在 payload 中保存 §2.4 的 `source_ref: QueryReference` 及必要表定义，例如：

```json
{
  "data_mode": "url_query",
  "title_field_id": "event_id",
  "source_ref": {
    "kind": "url_query",
    "source_url": "https://data.example.com/datasets/events",
    "query": { "fields": ["event_id", "created_at", "amount"] },
    "version": { "mode": "live_head" },
    "consistency": "best_effort"
  }
}
```

实现约束：

1. Workspace 保存定义和必要 schema，可对这份小型元数据使用 ObjectId；记录集无需 ObjectId，`table_records` 不承担远端全量镜像。schema 缓存需关联实际源版本（若源提供），变化后重新校验视图字段。
2. `table.query` 由服务层 Source Adapter 执行，返回分页数据、实际一致性、源 revision/snapshot token（若有）、cursor 和能力声明。定义的 `content_rev` 只跟踪定义修改，不代表查询结果版本；远端内容更新通过源通知/刷新获知，不能凭本地 revision 不变断言结果仍新鲜。
3. TableView 的字段投影、过滤和排序按适配器能力下推，与基础查询组合；不支持的条件明确拒绝。缺少稳定源行键时仅提供结果展示，不提供跨查询行锚点或编辑，不把页号/offset 当成稳定 `record_id`。
4. 第一期 URL 源只读。`table.insert_records/set_values/migrate_field` 等修改记录/schema 的命令返回 `INVALID_OPERATION`（子码 `SOURCE_READ_ONLY`）；修改源配置属于本地定义变更，仍经命令、OCC 和历史。未来远端写入另行定义条件写与补偿，不纳入本地 SQLite 原子批次。
5. 普通检查点、引用型导出和 Fork 保存定义，不请求全表；完整自包含导出只对明确选取且成功取得的范围提供保证。源支持固定版本时保存源定位与版本，不把源令牌称为 ObjectId；不支持时列入未固定依赖。
6. 可把用户确认的一次查询结果/分区物化为新内置 TableSource 或确定结果对象，获得该切片的内容身份与离线能力。记录查询来源、取得时间、源版本（若有）和范围；它不能冒充整个源的副本，也不意味着原始读取过程天然一致。
7. 断网时只有已取得的缓存范围可按明确策略读取；首次读取失败显示不可用。普通浏览允许显式 `best_effort`，需要严格一致输入的受控加工应要求源快照能力或明确拒绝。导入不会自动联网取数。

V24 fixtures：按页生成的大数据源（无 ObjectId）、同一 URL 两种查询、无稳定行键、支持/不支持源快照 token、权限范围不同、源不可达。断言不全量取数、缓存隔离、定义往返一致、能力不被夸大，以及仅物化所选结果切片。URL 参数和服务身份的访问检查不能通过任意网络请求旁路。

### 3.5 Cell 与 TableView

#### 3.5.1 模型

Cell 是版面上的一个展现实例：它引用一个内容实体，并保存“如何展现”的配置。Cell 不持有业务数据。TableView 是 `view.type = "table"` 的 Cell。

```json
{
  "source_ref": { "entity_id": "tasks", "version": { "mode": "live_head" } },
  "view": { "type": "table" },
  "title": "未完成任务",
  "fields": [ { "field_id": "title", "width": 320 }, { "field_id": "status", "width": 120 },
              { "field_id": "due", "width": 140 } ],
  "filter": { "op": "cmp", "field_id": "status", "operator": "ne", "value": "option-done" },
  "sorts": [ { "field_id": "due", "direction": "asc" } ],
  "group": null,
  "manual_order": null
}
```

顶层键即并发单位：`source_ref`、`view`、`title`、`fields`、`filter`、`sorts`、`group`、`manual_order`、`options`。两人同时一个改筛选一个调列宽不冲突；同时调列宽则后者冲突（列宽是低价值冲突，UI 的处理是静默以最新状态重试一次，这是 UI 策略，不是后台放宽检查）。

`view.type` 与允许的源类型：

| `view.type` | 源 `type_id` | 专有键 |
| --- | --- | --- |
| `table` | `buckyos.table-source` | `fields`、`filter`、`sorts`、`group`、`manual_order` |
| `richtext` | `buckyos.richtext` | 无（`options` 可含 `readonly`） |
| `record` | `buckyos.record` | `fields`（属性 `key` 的显示顺序） |
| `asset` | `buckyos.asset-ref` | `options.fit`: `contain` \| `cover` |

操作：`entity.create`、`entity.set_keys`/`unset_keys`、`tree.*`。`source_ref` 在 `plan` 中校验：目标存在、类型与 `view.type` 匹配、主体对目标有 `read`。Cell 对源的引用种类是 `bind`，因此删除源会被拒绝并列出这些 Cell。删除 Cell 不影响源。

变化类别：对 Cell 的所有 `set_keys` 产生 `view` 类别事件，不使依赖源数据的加工结果过期（V19）。

#### 3.5.2 个人视图状态

临时筛选、临时排序、滚动位置、选区、展开状态属于会话，保存在客户端（`SessionStore`），不产生 Commit。“保存视图”才把当前会话状态写成 `set_keys`。第一期不在后台保存个人视图偏好。

#### 3.5.3 过滤、排序、分组

过滤表达式是受控 AST，不接受任何字符串形式的表达式或 SQL 片段：

```text
Filter := { "op": "and" | "or", "args": [Filter, ...] }      // args 1..32 个
        | { "op": "not", "arg": Filter }
        | { "op": "cmp", "field_id": ..., "operator": ..., "value"?: ... }
嵌套深度 ≤ 5，节点总数 ≤ 64。
```

运算符与类型矩阵（未列出的组合在保存视图和查询时都返回 `INVALID_OPERATION`）：

| 运算符 | text | number / decimal | date / datetime | boolean | select | multi_select | object_ref |
| --- | --- | --- | --- | --- | --- | --- | --- |
| `eq`、`ne` | ✓ | ✓ | ✓ | ✓ | ✓ | | ✓（按 `entity_id`） |
| `lt`、`lte`、`gt`、`gte` | | ✓ | ✓ | | | | |
| `contains`、`starts_with` | ✓ | | | | | | |
| `in`（值为数组） | ✓ | ✓ | ✓ | | ✓ | | |
| `has_any`、`has_all`（值为数组） | | | | | | ✓ | |
| `is_empty`、`is_not_empty`（无 `value`） | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ |

语义：

- `value` 按字段类型做与写入相同的规范化后再比较；`decimal` 按十进制数值精确比较，不转浮点。
- `is_empty`：未设置或 `null`；对 `multi_select` 还包括空数组；空字符串 `""` **不是**空。
- 未设置或 `null` 的单元格对 `is_empty` 以外的所有运算符求值为假；`ne` 也为假（要包含空值须显式 `or is_empty`）。`not` 是对子表达式布尔结果取反。
- 文本比较区分大小写，按 Unicode 标量值序列比较；`contains`/`starts_with` 按原样子串匹配。不做本地化排序规则，第一期不提供拼音序。

排序：`sorts` 最多 4 项，可排序类型为 text、number、decimal、date、datetime、boolean（false < true）、select（按字段 options 的当前顺序）。空值（未设置/`null`）无论升降序都排在最后。最终以 `record_id` 字节序作为决胜键，保证全序稳定。

分组：`group: { "field_id": ... }`，仅支持 select、boolean、date；分组是展现行为，查询接口返回的仍是带排序的平面行，另附每组计数。

手工顺序：`manual_order: { <record_id>: <order_key> }`，仅在 `sorts` 为空时生效；不在表中的记录按 `record_id` 排在其后。它是视图的键，不改变记录本身。整键一个版本格，两人同时手工排序同一视图会冲突；第一期接受这一点。

查询请求既可以引用一个已保存的 TableView（`view_id`，使用其 `filter`/`sorts`），也可以直接给出 `source_id` + 过滤排序；还可以在 `view_id` 之上叠加会话级的额外过滤（`and` 组合）。

#### 3.5.4 悬空的字段与选项

视图配置引用了已删除的字段，或过滤值引用了已删除的选项：

- 后台**不**自动改写视图配置（那是对他人文档内容的隐式修改）。
- `read` 该 Cell 时附带 `diagnostics: [ { "key": "filter", "code": "FIELD_DELETED", "field_id": ... } ]`。
- 过滤表达式含失效子句时，`query` 返回 `INVALID_OPERATION`（子码 `VIEW_BROKEN`）而不是忽略该子句：忽略一个过滤条件会让视图显示更多行，对经由该视图获得数据的下游是错误结果。UI 显示“此视图的筛选引用了已删除的字段”，并提供一键移除失效条件（生成普通的 `set_keys`）。
- `fields`、`sorts`、`group` 中的失效项在查询时跳过并在结果的 `diagnostics` 中报告，因为它们不改变行集合。
- 字段恢复（撤销删除）后，原视图配置自动重新有效，因为配置从未被改写。

#### 3.5.5 物化与 Fixtures

`content` = payload 原样（内部引用不带 `workspace_id`）。Fixtures：同源两个视图各自保存配置互不影响；改一个视图的筛选不改变源的 `content_rev`；矩阵中每个合法/非法组合；空值排序位置；`decimal` 的 `"10.00"` 与 `"9.50"` 的数值序；失效过滤；求值器与 SQL 下推的差分测试（随机生成过滤表达式和数据）。

### 3.6 AssetRef

**用途与边界。** 把一个不可变的文件内容（图片、附件）接入 Workspace。内容本身在 CYFS named store 中，AssetRef 实体只保存对它的引用与基础元数据。

```json
{ "object_id": "cyfile:7d28f1f3...", "media_type": "image/png", "size": 482133,
  "file_name": "架构图.png", "image": { "width": 1920, "height": 1080 } }
```

- `object_id` 是 FileObject 的 ObjectId。【裁决】在 JSON 中一律使用 `类型:十六进制` 形式，这是 ndn-lib `ObjId` 的 serde 形式；base32 形式只出现在 URL 中。
- `media_type`、`size` 由后台在登记时从已校验的内容得出，不采信客户端声明（客户端声明只作为 `file_name` 的来源）。
- 替换图片 = `set_keys` 改 `object_id`（及随之变化的元数据），旧内容因历史引用继续保留。

**上传与登记流程**（资产准备不在 Commit 的 ACID 范围内，顺序是“先有已验证内容，再提交引用”）：

1. 客户端把字节上传到后台的资产暂存接口（§5），后台边接收边计算 chunk 哈希，写入 named store，生成 FileObject，并以租约方式保留（有 TTL）。返回 `object_id`、`size`、探测到的 `media_type`。单文件上限缺省 256 MiB；超过单 chunk 大小的文件按 ChunkList 组织。
2. 客户端提交 `entity.create`（类型 `buckyos.asset-ref`）引用该 `object_id`。`plan` 检查：该对象存在于 `assets` 表且状态为 `available`。不存在 → `DEPENDENCY_UNAVAILABLE`，Commit 被拒绝，不产生悬空引用（V22）。
3. Commit 接受后，租约转为该 Workspace 名下的长期保留。未在 TTL 内被引用的暂存内容可被清理。

离线时添加图片：字节先存入浏览器本地资产区，对应的 Commit 留在待提交队列并标记“依赖资产上传”；重连后先上传、再发送 Commit。

**状态。** `read` 返回 `availability`: `available` | `missing`（named store 中取不到）| `corrupt`（取到但校验失败）。它是读取时的诊断，不是持久化字段。缺失的资产不阻止文档打开，Cell 显示占位与原因。

**物化。** `content` = payload 原样。导出时资产字节是否进包由导出模式决定；未包含的列入 Manifest 的缺失清单。

**Fixtures。** 引用未上传对象被拒绝；上传后 Commit 失败，内容不出现在任何实体中且租约到期后进入可回收清单；被历史 Commit 引用的旧图片不在可回收清单；篡改 named store 中的字节后 `availability` 为 `corrupt`。

### 3.7 Annotation：最小 Overlay

**用途与边界。** 证明“附加层不污染业务数据、锚点不随显示变化漂移、权限与正文独立”。不是完整评论系统。

```json
{ "target": { "entity_id": "tasks",
              "selector": { "kind": "table_cell", "record_id": "task-42", "field_id": "budget" } },
  "kind": "note", "body": "请核对预算来源", "style": { "color": "#FFF2CC" } }
```

- `target.selector.kind` 第一期：`entity`、`table_record`、`table_cell`、`table_field`、`richtext_block`。字符范围锚点（`richtext_range`）的结构保留、不实现。
- `kind`：`note` | `highlight`。`body` 是纯文本（≤ 4000 字符）。
- 实体的 `scope`（§2.9）：`shared` 或 `user:<主体>`，创建时确定，不可更改。个人注释的结构父节点记录为其目标所在的页面，但它不出现在其他主体的 `list_children`、变化流和导出中。
- 权限：创建需要对目标有 `read` 且对 Workspace 有 `comment`；修改/删除自己的注释需要 `comment`，他人的共享注释需要 `manage`。对业务数据只读的主体可以有 `comment`。
- 引用种类 `anchor`：不阻止目标删除。目标（实体、记录、字段、块）被删除后，`read` 注释返回 `anchor_state: "target_deleted"` 并保留原 `target`，**不**尝试改挂到相邻对象。目标恢复后自动回到 `resolved`。
- 排序、过滤、改列名、插入其他行、移动块都不影响锚点，因为锚点只含稳定 ID（V08）。

操作：`entity.create`、`entity.set_keys`（`body`、`style`）、`entity.delete`。`target` 创建后不可改（要改就删除重建），避免“把别人的注释挪到另一条记录”这类路径。

物化：共享注释的 `content` = payload；个人注释不进入快照。

### 3.8 贯穿样例 fixture：`project-workspace`

第一期文档 §8.1 的样例落成一份确定的 fixture（`fixtures/project-workspace/`），所有层的测试共用，ID 固定：

```text
root                              container(root)
└─ page-main                      container(page, flow)        name: "项目工作区"
   ├─ notes                       richtext                     name: "项目说明"
   ├─ tasks                       table-source                 name: "任务"
   │  └─ task-42-details          richtext（task-42 的 body）
   ├─ project-info                record                       name: "项目信息"
   ├─ diagram                     asset-ref                    name: "架构图"
   ├─ cell-notes                  cell(richtext → notes)
   ├─ cell-all-tasks              cell(table → tasks)          title: "全部任务"
   ├─ cell-open-tasks             cell(table → tasks, filter status ≠ done)   title: "未完成任务"
   ├─ cell-info                   cell(record → project-info)
   ├─ cell-diagram                cell(asset → diagram)
   ├─ summary                     richtext                     name: "任务摘要"（由 Mock 生成）
   ├─ cell-summary                cell(richtext → summary)
   └─ note-budget                 annotation(shared) → tasks / task-42 / budget
```

- `tasks` 字段：`title`(text, required)、`status`(select: `option-open`/`option-done`)、`budget`(decimal, scale 2)、`due`(date)、`owner`(text)、`risk`(select, `maintained_by: program`，由 Mock 写入)。
- `notes` 的正文中有一个 `object_embed` 指向 `cell-open-tasks`，一个 `object_link` 指向 `task-42-details`。
- fixture 以“创建它的 Commit 序列”（JSON 文件）而不是数据库文件的形式保存；测试通过服务接口重放这些 Commit 来建立样例。这同时是服务接口的冒烟测试，也保证 fixture 不随存储布局变化而失效。配套给出重放后期望的 ContentRoot ObjectId（M1 生成后固定）。
- 性能 fixture（1 万记录 × 20 字段、10 万字符约 1000 块、1000 个实体）由带固定种子的生成器产生，同样以 Commit 序列的形式灌入。

---

## 4. 存储

### 4.1 两类数据库

【裁决，已确认】**每个 Workspace 是一个文件夹**，文件夹是它在后台的完整单元；服务没有全局数据库。

```text
<数据目录>/workspaces/<workspace_id>/
  doc.sqlite        # 文档库：§4.2 的全部表。可携带的用户文档的工作形态
  local.sqlite      # 本地库：grants、runs、upload_sessions。属于本部署，不进入任何导出
  staging/          # 上传暂存、导出任务产物、导入暂存
<数据目录>/trash/<workspace_id>-<时间>/     # ws.delete 与 restore 覆盖时移入
```

| 库 | 内容 | 访问方式 |
| --- | --- | --- |
| `doc.sqlite` | §4.2 的全部表 | rusqlite；一个写连接由该 Workspace 的写执行者独占，若干只读连接 |
| `local.sqlite` | `grants`（§2.9）、`locks` 与 `lock_events`（§2.11）、`runs`（§7）、`upload_sessions` | rusqlite；同一写执行者持有 |

- Workspace 目录就是 `workspaces/` 下的子文件夹列表：`ws.list` 扫描文件夹并按各自 `local.sqlite` 的授权过滤，结果在内存中缓存，创建、删除、导入时更新。第一期不为它建索引库；Workspace 数量大到扫描成为瓶颈时再加可重建的索引，语义不变。
- 创建 Workspace 的顺序：在 `staging` 同级的临时名下建好两个库并写入 `root` 与所有者授权，最后原子重命名为 `<workspace_id>`。启动时清理未完成的临时文件夹。半建成的文件夹永远不会以正式名字出现。
- 提交管线需要读 `local.sqlite`（授权终查、运行是否已取消）并写 `doc.sqlite`。两者由同一个写执行者串行访问，因此“事务内读到的授权/运行状态”有明确含义；它们不构成跨库原子事务，也不需要：`local.sqlite` 的写入（授权变更、运行状态变更）都是独立操作。
- 后续可以在保持以上语义的前提下优化物理布局（合并库、共享索引等），不属于第一期。

文档库 pragma：`journal_mode=WAL`、`synchronous=FULL`、`foreign_keys=ON`。`synchronous=FULL` 是因为“已应答的提交在断电后仍存在”是验收要求；实测吞吐不足时再评估，不能默认降级。

### 4.2 文档库 DDL（`storage_schema_version = 1`）

```sql
CREATE TABLE workspace_meta (
  key   TEXT PRIMARY KEY,
  value TEXT NOT NULL
) WITHOUT ROWID;
-- 键：workspace_id, epoch, format_version, storage_schema_version, head_seq, head_commit_id,
--     forked_from（JSON，可无）, created_at

CREATE TABLE entities (
  entity_id      TEXT PRIMARY KEY,
  type_id        TEXT NOT NULL,
  schema_version INTEGER NOT NULL,
  scope          TEXT NOT NULL DEFAULT 'shared',        -- 'shared' | 'user:<principal>'
  name           TEXT,
  write_policy   TEXT NOT NULL DEFAULT 'open',          -- 'open' | 'lock_required'；版本格为 meta_rev（§2.11）
  payload_json   TEXT NOT NULL DEFAULT '{}',            -- 键式文档；未知类型时原样保存
  key_revs_json  TEXT NOT NULL DEFAULT '{}',
  created_seq    INTEGER NOT NULL,
  meta_rev       INTEGER NOT NULL,
  content_rev    INTEGER NOT NULL,
  life_rev       INTEGER NOT NULL,
  deleted_seq    INTEGER                                 -- NULL = 存活
) WITHOUT ROWID;

CREATE TABLE tree_edges (
  child_id       TEXT PRIMARY KEY REFERENCES entities(entity_id),
  parent_id      TEXT NOT NULL REFERENCES entities(entity_id),
  order_key      TEXT NOT NULL,
  placement_json TEXT,
  struct_rev     INTEGER NOT NULL
) WITHOUT ROWID;
CREATE INDEX tree_edges_parent ON tree_edges(parent_id, order_key, child_id);
-- 同父同名唯一由引擎在事务内检查（需排除已删除实体，无法用简单唯一索引表达）

CREATE TABLE refs (                                       -- 引用边的集合；一行 = 一条不重复的边
  src_entity_id    TEXT NOT NULL REFERENCES entities(entity_id),
  src_selector     TEXT NOT NULL DEFAULT '',             -- 规范化 selector JSON；实体级为 ''
  kind             TEXT NOT NULL,                        -- bind|embed|value|anchor|asset|body
  dst_workspace_id TEXT NOT NULL DEFAULT '',             -- '' = 本 Workspace
  dst_entity_id    TEXT NOT NULL DEFAULT '',             -- kind='asset' 时为 ''
  dst_object_id    TEXT NOT NULL DEFAULT '',             -- 资产或 fixed_revision 的 ObjectId；无则 ''
  dst_query_json   TEXT NOT NULL DEFAULT '',             -- QueryReference 规范 JSON；无则 ''，不含凭据/cursor
  PRIMARY KEY (src_entity_id, src_selector, kind, dst_workspace_id, dst_entity_id, dst_object_id, dst_query_json)
) WITHOUT ROWID;
CREATE INDEX refs_dst_entity ON refs(dst_workspace_id, dst_entity_id);
CREATE INDEX refs_dst_object ON refs(dst_object_id);

CREATE TABLE table_fields (
  source_id   TEXT NOT NULL REFERENCES entities(entity_id),
  field_id    TEXT NOT NULL,
  def_json    TEXT NOT NULL,
  order_key   TEXT NOT NULL,
  def_rev     INTEGER NOT NULL,
  type_rev    INTEGER NOT NULL,
  values_rev  INTEGER NOT NULL DEFAULT 0,               -- 列级：任一记录在该字段上的值变化
  deleted_seq INTEGER,
  PRIMARY KEY (source_id, field_id)
) WITHOUT ROWID;

CREATE TABLE table_records (
  source_id   TEXT NOT NULL REFERENCES entities(entity_id),
  record_id   TEXT NOT NULL,
  values_json TEXT NOT NULL DEFAULT '{}',
  revs_json   TEXT NOT NULL DEFAULT '{}',
  meta_json   TEXT NOT NULL DEFAULT '{}',
  body_entity_id TEXT REFERENCES entities(entity_id),
  created_seq INTEGER NOT NULL,
  rev         INTEGER NOT NULL,
  deleted_seq INTEGER,
  PRIMARY KEY (source_id, record_id)
) WITHOUT ROWID;

CREATE TABLE richtext_states (
  entity_id      TEXT PRIMARY KEY REFERENCES entities(entity_id),
  lineage_id     TEXT NOT NULL,
  engine         TEXT NOT NULL,                          -- 'loro'
  engine_version TEXT NOT NULL,                          -- 写入快照时的库版本
  encoding       TEXT NOT NULL,                          -- 'pm-loro-1'
  snapshot       BLOB NOT NULL,                          -- 完整历史快照
  snapshot_seq   INTEGER NOT NULL,                       -- 快照已包含到的 seq
  ast_json       TEXT NOT NULL,                          -- 与 entities.content_rev 对应的规范 AST
  block_index_json TEXT NOT NULL
) WITHOUT ROWID;

CREATE TABLE richtext_updates (
  entity_id TEXT NOT NULL REFERENCES entities(entity_id),
  seq       INTEGER NOT NULL,
  idx       INTEGER NOT NULL,                            -- 同一 Commit 内对同一实体的第几次更新
  update_bytes BLOB NOT NULL,
  PRIMARY KEY (entity_id, seq, idx)
) WITHOUT ROWID;

CREATE TABLE commits (
  seq            INTEGER PRIMARY KEY,                    -- 无 AUTOINCREMENT；由引擎取 head_seq+1
  commit_id      TEXT NOT NULL UNIQUE,
  principal      TEXT NOT NULL,
  app_id         TEXT,
  session_id     TEXT,
  origin         TEXT NOT NULL,
  run_id         TEXT,
  undo_group     TEXT,
  undoes         TEXT,                                   -- 被本提交补偿的 commit_id
  idem_key       TEXT NOT NULL,
  request_digest TEXT NOT NULL,
  message        TEXT,
  accepted_at    TEXT NOT NULL,
  result_json    TEXT NOT NULL,                          -- 幂等重放时原样返回的 accepted 响应
  UNIQUE (principal, idem_key)
);

CREATE TABLE commit_ops (
  seq          INTEGER NOT NULL REFERENCES commits(seq),
  op_index     INTEGER NOT NULL,
  op_json      TEXT NOT NULL,                            -- 规范化后的操作（含后台生成的富文本更新）
  inverse_json TEXT,                                     -- NULL = 该操作不可后台补偿
  touched_json TEXT NOT NULL,
  PRIMARY KEY (seq, op_index)
) WITHOUT ROWID;

CREATE TABLE assets (
  object_id   TEXT PRIMARY KEY,                          -- FileObject ObjectId（type:hex）
  media_type  TEXT NOT NULL,
  size        INTEGER NOT NULL,
  first_seq   INTEGER NOT NULL                           -- 首次被引用的提交
) WITHOUT ROWID;

CREATE TABLE entity_versions (                            -- 物化缓存，可重建
  entity_id   TEXT NOT NULL,
  content_rev INTEGER NOT NULL,
  object_id   TEXT NOT NULL,
  PRIMARY KEY (entity_id, content_rev)
) WITHOUT ROWID;
CREATE INDEX entity_versions_object ON entity_versions(object_id);

CREATE TABLE snapshots (
  snapshot_id  TEXT PRIMARY KEY,                         -- SnapshotRoot ObjectId
  content_root TEXT NOT NULL,
  seq          INTEGER NOT NULL,
  kind         TEXT NOT NULL,                            -- checkpoint|export|fork_source|pin
  retained     INTEGER NOT NULL DEFAULT 1,
  created_at   TEXT NOT NULL,
  created_by   TEXT NOT NULL
) WITHOUT ROWID;
```

说明：

- `refs` 的主键包含目标：同一来源位置可以有多条指向不同目标的同类引用（一个段落里的两个 `object_link`、一个 `multi` 语义的值），而指向同一目标的重复引用合并为一条边。目标列用空字符串而不是 NULL 表示“无”，因为 SQLite 的主键/唯一约束把 NULL 视为互不相等，无法去重。适配器给出的 `ref_changes` 是“该来源位置的引用集合由 A 变为 B”，引擎按集合差增删行；只要该位置仍有一处引用指向某目标，那条边就保留。
- URL 查询边填写 `dst_query_json`，其余目标列为空；不要求构造 `dst_object_id`。此处保存的是引用定义，不是执行缓存键；缓存另按可信授权范围隔离。`entity_versions` 只缓存实际物化的 ObjectId，URL 查询结果没有 ObjectId 时不插入空 ID 行。Source Adapter 枚举 URL 依赖供导出报告使用，不把它们当作本地 named store 的 GC 保留根。
- 后台文档库**没有 outbox 表**。变化流即 `commits` + `commit_ops`，它们与数据在同一事务写入，“落盘前不发通知”和“崩溃后可重建通知”都由此得到（R14）。
- 幂等结果即 `commits` 行（§2.6 第 3 条），没有单独的 `idempotency_results` 表。
- `table_records` 第一期不建值索引。若 M2 测量证明需要，按字段追加部分表达式索引：`CREATE INDEX ... ON table_records(json_extract(values_json, '$."<field_id>"'), record_id) WHERE source_id = '<id>' AND deleted_seq IS NULL`；`field_id`/`entity_id` 的字符集限制保证它们可以安全拼入 DDL。
- 已删除字段的值保留在 `values_json` 中（§3.4.4），读取与物化路径必须按 `table_fields.deleted_seq` 过滤；这一点要有专门的测试。
- 打开文档库时检查 `storage_schema_version`：高于支持范围 → 拒绝打开并返回 `UNSUPPORTED_VERSION`，不修改文件；低于当前版本的升级脚本在预发布阶段可以不提供，但同样必须拒绝而不是清空。

### 4.3 写执行者与内存状态

- 每个打开的 Workspace 一个写执行者：一个专用线程（或 `spawn_blocking` 中的长驻任务）持有 rusqlite 写连接，通过通道接收命令。仓库规范要求 rusqlite 调用不得阻塞 async 运行时（§8.3）。
- 内存状态只有两类：已加载富文本的权威 `LoroDoc`（按需加载，LRU 淘汰，淘汰前无需回写，因为权威状态在库中）；`head_seq` 与变化流等待者列表。除此之外不做内存缓存，直到测量证明需要。
- 空闲 Workspace 在超时后关闭（提交 WAL 检查点、释放连接）。再次访问时重新打开。

### 4.4 导出包、导入与 Fork

包结构沿用第一期文档 §3.5，补充确定的文件布局：

```text
example.bcanvas/                      # 展开目录；打包时为不压缩或 deflate 的 zip
  manifest.json
  objects/<hex>.<obj_type>            # NamedObject 的 canonical JSON 原文，文件名即 ObjId::to_filename()
  chunks/<hex>.<chunk_type>           # 包内包含的 chunk 数据
  collab/<entity_id>.loro             # 仅“个人恢复备份”模式
  personal/...                        # 仅“个人恢复备份”模式：个人注释、（由客户端附加的）草稿与待提交队列
```

`manifest.json`（不是 NamedObject，不参与任何哈希）：`format`（`buckyos.ai-workspace`）、`format_version`、`export_mode`（`share` | `personal_backup`）、`self_contained`（bool）、`workspace_id`、`snapshot`（SnapshotRoot ObjectId）、`content_root`、`objects[]` 与 `chunks[]`（各含 ID、字节数）、`types[]`（用到的 `type_id` 与 `schema_version`、富文本 `editor_schema`）、`missing[]`（未包含的资产或对象及原因：`not_authorized` | `unavailable` | `external`）、`excluded_entities[]`（因权限被排除的实体数量，不列 ID）、`exported_at`、`exported_by`。

另设 `external_sources[]`，保存 URL 查询引用、是否固定源版本、是否已纳入结果切片和实际能力；不要求这些项填写 ObjectId 或未知的全量字节数。`missing[]` 也允许以查询引用定位缺失依赖。`self_contained: true` 只适用于声明范围的数据与资源均已纳入的包；根 ObjectId 只校验包内定义，不能作为外部结果已固定或可离线的证明。

**导入**分两步，全部在暂存目录中进行，成功后才登记到服务目录：

1. 校验：`format_version` 受支持；zip 条目路径不含 `..`、绝对路径或符号链接；总大小和条目数在限额内；每个 `objects/` 文件的内容是 canonical JSON 且哈希等于文件名；每个 chunk 哈希正确；从 SnapshotRoot 出发可达的内容对象都存在或已列入 `missing`；URL 依赖有合法定义且列入 `external_sources`，不强制解析成内容对象；结构索引满足树约束；每个内容对象通过对应适配器的 schema 校验（未知类型按降级规则保留）。
2. 装载：在暂存目录新建文档库，生成**新的 `epoch`**，调用各适配器的 `load` 生成 `StoreWrite`，写入为 `seq = 1` 的一次 `origin: "import"` 提交；写入对象与 chunk 到 named store 并保留；重新物化并断言 ContentRoot 等于包内的 `content_root`（不等即实现缺陷，导入失败）。

导入时必须选择语义，不提供缺省值：

- `new`（等同 Fork）：新 `workspace_id`，`lineage.forked_from` 指向包内快照。导入者成为所有者。
- `restore`：沿用包内 `workspace_id`。
  - 该 ID 在本部署中不存在：导入者成为所有者。
  - 该 ID 已存在：必须显式 `replace: true`，并且**调用方必须对现有 Workspace 拥有 `manage` 能力**（按现有文件夹的 `local.sqlite` 判定；包内的任何身份或授权信息都不作为依据）。不满足时返回与“该 ID 不存在且无权创建”不可区分的 `PERMISSION_DENIED`，不透露目标是否存在。校验通过后先把现有文件夹整体移入回收目录，再原子换入新文件夹；现有的 `local.sqlite` 中的 `grants` 迁入新文件夹（恢复内容不改变谁有权访问），`runs` 与上传会话不迁移。

**恢复不延续旧历史。** 导出包（包括个人恢复备份）不携带 `commits`/`commit_ops`，所以恢复后的 Workspace 从 `seq = 1` 开始，旧的 `seq`、`rev`、幂等记录都不再存在。新的 `epoch` 把这一点变成协议事实（§2.6 第 7 条）：恢复前在线或离线的客户端下一次请求得到 `EPOCH_MISMATCH`，必须重新准备副本；它们的待提交操作按 §6.4 保留为可导出的本地内容，不会被当作新历史上的提交重放，也不会因为幂等记录消失而被执行第二次。富文本的协作 lineage 是另一回事：个人恢复备份带回原 lineage，旧客户端的富文本更新在重新同步后仍可合并（§3.3.9）。

**Fork**（服务内）：物化来源的当前内容 → 以 `new` 语义导入该内容根。新 Workspace 的授权只有执行 Fork 的主体；不复制 `grants`、`runs`、上传会话、个人作用域实体。富文本为新 lineage（§3.3.9）。

---

## 5. 服务接口

传输与鉴权的仓库事实见 §8.2。本节定义方法语义。所有方法走 kRPC，请求参数里都有 `workspace_id`（`ws.create`/`ws.list`/`ws.import` 除外）。

【裁决】kRPC 的错误通道只能承载一个字符串，无法满足第一期文档 §4.7 的结构化错误要求。因此：**业务结果一律放在 `result` 中**，形如 `{ "ok": true, ... }` 或 `{ "ok": false, "error": { "code", "retryable", "detail", ... } }`（`doc.commit`/`doc.prepare`/`doc.undo` 用 §2.6 的三态结构）。kRPC 层的 `error` 只用于令牌无效、方法不存在、请求无法解析这类协议层失败。客户端不得靠子串匹配错误文本来区分业务情况。

| 方法 | 能力 | 说明 |
| --- | --- | --- |
| `ws.create` | 已认证 | `{ title, workspace_id? }` → 新 Workspace（含 `root`）。调用方成为所有者 |
| `ws.list` | 已认证 | 返回调用方至少有 `read` 的 Workspace |
| `ws.get_info` | `read` | 标题、`epoch`、`head_seq`、`format_version`、调用方的有效能力 |
| `ws.delete` | `manage` | 移入回收目录；第一期不物理删除 |
| `ws.grant` / `ws.revoke` / `ws.list_grants` | `manage` | §2.9 |
| `doc.resolve` | `read` | 引用或路径 → 已解析引用、`type_id`、`content_rev`、有效能力 |
| `doc.read` | `read` | §2.5.4；可批量 |
| `doc.list_children` | `read` | 子节点的信封（不含内容）与树边 |
| `doc.query` | `read` | §2.5.4、§3.5.3 |
| `doc.get_collab_state` | `read` | §3.3.6 |
| `doc.prepare` | 按操作 | §2.6 |
| `doc.commit` | 按操作 | §2.6 |
| `doc.get_submission` | 已认证 | 只能查询自己主体的幂等键 |
| `doc.undo` | 见 §2.7 | |
| `doc.get_changes` / `doc.wait_changes` | `read` | §2.8 |
| `doc.checkpoint` | `export` | 物化当前状态，返回 SnapshotRoot 与 ContentRoot ObjectId |
| `doc.export` | `export` | `{ mode, self_contained, snapshot? }` → 导出任务 ID；包通过 HTTP 下载 |
| `asset.begin_upload` | 目标 Workspace 的 `update` 或 `structure` | `{ size, file_name? }` → `upload_id`；上传完成后 `asset.finish_upload` 返回 `object_id`、`media_type`、`size`（§3.6） |
| `ws.begin_import` | 已认证 | → `upload_id` |
| `ws.import` | 已认证；`restore` 覆盖已有 Workspace 时须对其有 `manage` | `{ upload_id, semantics: "restore" \| "new", replace? }`，规则见 §4.4 |
| `ws.fork` | 来源 `export` | → 新 `workspace_id` |
| `replica.bootstrap` | Workspace 级 `read` | 离线副本初始数据：见 §6.2 |
| `proc.start` / `proc.get` / `proc.apply` / `proc.cancel` | `read` + 写目标能力 | §7；`proc.apply` 带 `session_id`，写入受锁保护的对象时调用方须持有锁 |
| `lock.acquire` / `lock.renew` / `lock.release` / `lock.break` / `lock.list` | 见 §2.11 | 写锁 |
| `diag.list_unretained` | `manage` | §2.10.3 |

非 kRPC 的 HTTP 路由（同一端口，与 kRPC 路由并列）：

| 路由 | 用途 |
| --- | --- |
| `PUT /kapi/aiworkspace/upload/<upload_id>`（支持分段） | 资产字节与导入包上传；`upload_id` 由 `asset.begin_upload` / `ws.begin_import` 返回 |
| `GET /kapi/aiworkspace/asset/<workspace_id>/<object_id>` | 读取资产内容（支持 Range）；按引用它的实体检查 `read` |
| `GET /kapi/aiworkspace/export/<export_id>` | 下载导出包 |

这些路由用 `Authorization: Bearer <session token>` 鉴权，与 kRPC 用同一套令牌校验。

限额缺省值（配置项，写入服务 settings）：单个 Commit 请求体 ≤ 8 MiB、操作数 ≤ 10 000、`table.set_values` 项数 ≤ 50 000；`doc.query` 的 `limit` ≤ 1000；`doc.get_changes` 的 `limit` ≤ 500 个提交或 8 MiB；`doc.wait_changes` 的超时 ≤ 30 s。超限统一返回 `LIMIT_EXCEEDED` 并指出是哪一项。

### 5.1 错误码

在第一期文档 §4.7 的基础上补充（标 ＊ 的是新增）：

| code | retryable | 说明 |
| --- | --- | --- |
| `INVALID_SCHEMA`、`INVALID_OPERATION`、`LIMIT_EXCEEDED` | 否 | `INVALID_OPERATION` 带子码：`ID_CONFLICT`、`NAME_CONFLICT`、`UNIQUE_VIOLATION`、`LINEAGE_MISMATCH`、`MIGRATION_UNSUPPORTED`、`VIEW_BROKEN`、`TREE_CYCLE`、`CHILD_NOT_ALLOWED` |
| `NOT_FOUND`、`TARGET_DELETED`、`REFERENCE_BROKEN` | 否 | |
| `PERMISSION_DENIED` | 否 | |
| `REVISION_CONFLICT`、`SCHEMA_CONFLICT`、`IDEMPOTENCY_MISMATCH` | 否 | 需要调用方处理后以新键重新提交 |
| `UNSUPPORTED_VERSION`、`MISSING_EXTENSION`、`DEPENDENCY_UNAVAILABLE` | 否 | |
| `STORAGE_FULL`、`STORAGE_IO_ERROR`、`WRITER_BUSY` | 是 | |
| ＊`BASE_UNKNOWN` | 是 | 富文本更新依赖后台没有的操作；先追赶再重发 |
| ＊`BASE_TOO_OLD` | 否 | 变化流或协作历史已不覆盖调用方的基准；重新准备副本 |
| ＊`EPOCH_MISMATCH` | 否 | Workspace 的历史已被恢复操作替换；调用方的 `seq`、`rev`、幂等键全部失效，须重新同步（§2.6 第 7 条、§6.4） |
| ＊`SNAPSHOT_EXPIRED` | 是 | 一致分页期间源已变化 |
| ＊`NOT_UNDOABLE` | 否 | 目标提交含不可后台补偿的操作 |
| ＊`RUN_CANCELLED` | 否 | 提交所属的加工运行已取消 |
| ＊`LOCK_REQUIRED` | 否 | 目标要求写锁而调用方未持有；先 `lock.acquire` |
| ＊`LOCK_HELD` | 否 | 锁由他人（或自己的另一个会话）持有；附持有者信息 |
| ＊`LOCK_LOST` | 否 | 调用方的租约已过期、被空闲释放或被强制解除 |

---

## 6. 浏览器离线副本

### 6.1 组成

```text
Desktop 页面（主线程）
  ├─ 编辑器 / 视图 / UndoCoordinator / SessionStore
  └─ ReplicaClient（与 Worker 通信的薄层）
        │ postMessage
Replica Worker（每个源一个持有者）
  ├─ aiworkspace-core（WASM）：校验、规划、编码、过滤求值、富文本编解码
  ├─ SQLite WASM + OPFS：副本库（每个 Workspace 一个）
  ├─ 资产区（OPFS 目录）：已缓存与待上传的资产字节
  └─ CommitTransport：与后台的 commit / get_changes / wait_changes
```

【裁决】存储：官方 `@sqlite.org/sqlite-wasm` 的 **`opfs-sahpool` VFS**，运行在专用 Worker 中。选它的原因是它不要求 COOP/COEP 响应头和 `SharedArrayBuffer`（Desktop 的静态资源由 control-panel 的目录服务提供，不能假设可以加这两个头，见 §8.4），且是官方推荐给“无法设置这些头”的场景的 VFS。它的限制是同一数据库不能被多个标签页同时打开，这正好与“单写入者”策略一致：

- 用 `navigator.locks` 以 `aiworkspace-replica:<workspace_id>` 为名申请独占锁，拿到锁的标签页成为该 Workspace 副本的持有者。
- 拿不到锁的标签页进入**在线直连模式**：不打开副本，所有读写直接走后台接口，界面标明“此窗口未启用离线”。后台不可达时该窗口只读提示，不允许编辑。它不是第二个写入者，也不存在两个实例各自覆盖整库的可能（V16）。
- 持有者关闭后锁释放，其他标签页可以在用户操作下接管。

OPFS 或 Worker 不可用、持久化存储申请被拒绝、配额不足：明确显示“离线不可用”及原因，进入在线直连模式。第一期不承诺 IndexedDB 回退。

应用资源缓存：新增 Service Worker，缓存 Desktop 入口、`aiworkspace` 应用的分包、WASM 模块。Desktop 目前没有 Service Worker（§8.4），它的作用域、与 control-panel 静态服务的缓存头配合、版本更新策略是 M1 的探针项；探针失败时第一期文档 §7 第 2 条不满足，离线验收不能通过，不得用“已打开的页面断网后还能用”代替。

### 6.2 副本库与准备

`replica.bootstrap` 返回一个新生成的副本库文件，以及对应的 `epoch` 与 `head_seq`。客户端把它写入 OPFS 作为副本库的**确认层**。显式准备还包括：把当前引用的资产字节拉入资产区（可按大小上限跳过并列出）。

副本库**按白名单从零构建**，不是“复制文档库再删除敏感行”：后台在一个只读事务中，向一个空的新数据库逐表写入允许的行。删除式做法会在空闲页和 WAL 中残留被删内容，也容易在新增表时漏掉过滤。

| 文档库表 | 进入副本的内容 |
| --- | --- |
| `workspace_meta` | 全部 |
| `entities`、`tree_edges`、`refs` | `scope = 'shared'` 的实体，加上调用方自己的个人作用域实体；边与引用只保留两端都在集合内的 |
| `table_fields`、`table_records` | 全部（第一期没有字段/记录级授权；已删除字段的残留值按 §4.2 的规则一并剔除） |
| `richtext_states` | 全部列。`snapshot` 是完整协作历史，这是有 `read` 权限的协作者本来就能通过 `get_collab_state` 取得的内容 |
| `richtext_updates` | 不复制；先在源事务内把它们并入 `snapshot` 的导出结果 |
| `assets` | 全部 |
| `commits`、`commit_ops` | **不复制。** 历史载荷里有他人个人注释的创建内容、被撤销的旧值、他人的幂等键和结果。副本不需要历史：它从 `head_seq` 的状态开始，之后的变化来自已按权限过滤的变化流；客户端撤销自己的提交只需要 `commit_id`，它来自提交响应 |
| `entity_versions`、`snapshots` | 不复制（可重建的缓存与本部署的保留记录） |
| `local.sqlite` 的任何表 | 不复制 |

测试必须包含：另一主体创建、修改、删除一条个人注释后，为调用方生成副本，对副本文件做**原始字节搜索**，注释正文的任何历史版本都不出现（V14）。同样的白名单构建方式用于导出时生成的任何数据库形态的产物。

副本库在确认层之外增加客户端专有表：

```sql
CREATE TABLE pending_submissions (
  local_order   INTEGER PRIMARY KEY AUTOINCREMENT,
  idem_key      TEXT NOT NULL UNIQUE,
  request_json  TEXT NOT NULL,        -- 完整 Commit 请求（可含 §2.2.2 的占位）
  preimage_json TEXT NOT NULL,        -- 回滚本地工作视图所需的前像
  state         TEXT NOT NULL,        -- queued | sending | unknown | conflict | rejected | blocked_asset
  result_json   TEXT,                 -- 冲突/拒绝的详情
  created_at    TEXT NOT NULL
);
CREATE TABLE drafts (                  -- explicit 策略下的私有草稿；被拒绝的富文本内容
  draft_id TEXT PRIMARY KEY, entity_id TEXT NOT NULL, kind TEXT NOT NULL,
  base_json TEXT NOT NULL, content BLOB NOT NULL, updated_at TEXT NOT NULL
);
CREATE TABLE replica_meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);
-- epoch, confirmed_seq, principal, prepared_at, schema versions
CREATE TABLE richtext_working (entity_id TEXT PRIMARY KEY, working_snapshot BLOB NOT NULL);
```

### 6.3 工作视图 = 确认层 + 待提交操作

本地编辑的处理（一个 OPFS 事务内）：

1. 用 WASM 内核对操作做与后台相同的 `plan`（权限检查除外），得到 `StoreWrite`。
2. 记录受影响行的前像到 `preimage_json`，把 `StoreWrite` 应用到副本库的数据表，插入 `pending_submissions` 行。
3. 事务提交完成后才向 UI 报告“已保存到本设备”。**内存状态变化、SQL 已入队都不算保存完成**。

于是副本库的数据表始终是“确认层 + 全部待提交操作”的工作视图，而每个待提交操作都可以凭前像撤回。这避免了维护两份全库。

收到远端变化（`get_changes`）时的**变基**，同样在一个事务内：

1. 按 `local_order` 逆序，用前像撤回全部未确认的待提交操作，数据表回到确认层。
2. 应用远端 `CommitEvent.ops`（远端已被接受，直接写入，不重新校验业务规则），推进 `confirmed_seq`。事件带有 `idempotency_key` 且与某个 `pending_submissions` 行的键相同 → 这是自己的提交已被接受（不论当初响应是否到达、该行处于 `sending` 还是 `unknown`），删除该行。匹配只按键，不按内容猜测。
3. 按 `local_order` 顺序重新 `plan` 并应用剩余的待提交操作，重新生成前像。重新规划失败（目标已被删除、`expect` 已不成立、schema 变化）的操作**不应用**，保留 `request_json`；依赖它的后续操作（引用了它创建的 ID，或 `expect` 占位指向它）同样不应用。状态处理：原状态为 `queued` 的置为 `conflict`；原状态为 `sending` 或 `unknown` 的**保持原状态**，因为它是否已被后台接受尚未确定，只有后台的答复（变化流中出现该键、`get_submission`、或一次明确的 `conflict`/`rejected` 响应）才能给它定性。

这满足第一期文档 §3.4 末段的全部要求：不以远端快照覆盖本地工作视图；本地临时状态不被当作已接受版本；被拒绝的候选留在待处理区。

富文本在变基中的处理：确认文档导入远端更新；工作文档也导入同样的更新（CRDT 合并，不需要撤回重放）。`richtext_working` 保存工作文档快照以便关闭后恢复。

### 6.4 发送与重连

发送循环（持有者 Worker 中，串行）：

1. 取 `local_order` 最小的 `queued` 行。若依赖资产未上传 → 先上传；上传失败置 `blocked_asset` 并继续等待。
2. 改写 `expect` 占位（§2.2.2），置 `sending`，调用 `doc.commit`。
3. `accepted` → 不立即删除行；等变化流带回该 `seq` 时在变基第 2 步删除，这样确认层的推进只有一条路径。
4. `conflict` / `rejected` → 置相应状态，保存 `result_json`，执行一次变基把它从工作视图中撤出，通知 UI。**后续不依赖它的待提交操作继续发送。**
5. 传输失败或超时 → 置 `unknown`。恢复连接后：先追赶变化流到当前 `head_seq`（若该提交其实已被接受，它的事件带着同一个键出现，在变基第 2 步被识别并删除）；追赶完成后仍为 `unknown` 的，调用 `doc.get_submission`：`accepted`（追赶之后才被接受的极端情况）→ 再追赶一次；`not_found` → 置回 `queued` 原样重发（同键同内容）。不生成新键。

重连顺序固定为：重新认证 → `ws.get_info`（确认仍有权限且 `epoch` 未变；`PERMISSION_DENIED` 则停止发送，全部待提交保留并提示）→ `get_changes` 追赶并变基 → 处理 `unknown` → 发送循环。先追赶后查询是安全的，因为第 2 步按键识别自己的提交，`unknown` 行在被定性之前不会被标成冲突。`BASE_TOO_OLD` 或 `EPOCH_MISMATCH` → 停止发送，提示需要重新准备离线副本；在用户确认前不丢弃 `pending_submissions` 与 `drafts`，并提供把它们导出为个人恢复备份附加部分的入口。`EPOCH_MISMATCH` 时所有待提交（含 `unknown`）一律不再自动发送：它们是针对已被替换的历史生成的，是否仍然适用只能由用户在新内容上重新判断。

权限在离线期间被撤回、目标被删除、基准过旧，三种情况下本地输入都仍在 `pending_submissions`（或 `drafts`）中可查看、可导出（V17）。

写锁与离线副本的关系见 §2.11：`lock_required` 的对象在后台不可达时只读，副本对它们不接受本地编辑；在线时持有锁期间产生的待提交操作，若在发送前失去了锁，会得到 `LOCK_LOST` 并进入待处理区。

### 6.5 状态呈现

UI 必须能区分并显示每个编辑的四种状态：`未保存`（本地事务未完成）、`已保存到本设备`、`已提交`（后台 `accepted`）、`需要处理`（冲突/拒绝/被阻塞）。存储写入失败（配额、Worker 崩溃、事务错误）时保持“未保存”并持续显示，提供把当前内存中的未保存内容导出为文件的入口；不显示成功提示后丢弃。

---

## 7. 受控加工与确定性 Mock

目的：用与后续真实 Agent 相同的候选协议验证暂存、校验、一次应用、人工修改保护、取消后迟到结果拒绝（V18）。Mock 没有任何绕过 Command Engine 的入口；它是服务进程内的一个模块，通过与外部调用方相同的内核接口读取并提交。

**运行生命周期**（`runs` 表在该 Workspace 的 `local.sqlite`）：

```text
proc.start { workspace_id, program: "mock.task-summary@1", params, idempotency_key }
   → run_id，状态 planning
planning → running → validating → waiting_confirmation → applying → succeeded
                └────────────────────────────────────────────→ failed | cancelled
```

- `proc.start` 时固定输入：在一个只读事务中读取 `tasks` 的全部所需字段，记录 `inputs: [{ entity_id, content_rev }]` 和每个将被覆盖的单元格的当前 `rev`、`summary` 各块的 hash。
- 程序产出**候选**：一个标准 Commit 请求（`origin: "program"`、`run_id`），保存在 `runs.candidate_json`。它是普通数据，不是已应用的状态。
- `proc.get` 返回状态与候选的 `prepare` 结果（影响范围、冲突）。缺省 `auto_apply: false`：调用方审阅后调用 `proc.apply { run_id }`，后台此时才执行 `doc.commit`。`auto_apply: true` 只在调用方显式传入时生效。
- `proc.cancel`：把运行置为 `cancelled`。提交管线在事务内检查 `run_id` 对应运行的状态，已取消 → `RUN_CANCELLED`。取消与提交的竞争以事务内读到的状态为准：先提交成功的，取消返回“已应用”及 `commit_id`，不伪报为无修改。

**`mock.task-summary@1` 的确定行为**（对固定输入产生固定输出，不读时钟、不用随机数；“今天”由 `params.today` 传入）：

1. 对每条未完成任务计算 `risk`：`due` 早于 `today` → `option-high`；7 天内 → `option-medium`；否则 `option-low`；无 `due` → 不设置。写入 `table.set_values`，每项的 `expect.rev` 为读取时的 `rev`。
2. 跳过 `meta.manual_override` 为真的单元格，并在候选的 `warnings` 中列出它们。
3. 生成 `summary` 富文本：不存在则 `entity.create` + 对应 Cell；存在则用块级操作替换由上次运行生成的块（`block_id` 固定为 `sum-<名称>`，`expect.hash` 为读取时的 hash）。
4. `preconditions`（读集合）：(a) 所用字段的 `type_rev`；(b) 决定参与范围的条件：`status` 字段的 `table_field_values` 与 `tasks` 的 `table_members`，取读取时的值；(c) 计算中读取的每个 `due` 单元格在读取时的 `rev`。(b) 保证“哪些任务未完成”这个成员集合没有变：新插入的任务、从完成改回未完成的任务都会使它不成立。它是保守的（任何任务的 `status` 变化都会使候选失效，包括不影响结果的变化），第一期接受这一点，换取实现简单且不漏。**不**对整个 `tasks.content_rev` 加前置条件：`owner`、`title`、`budget` 等无关字段的修改不应使候选失效（核心设计 §16.9“并发判断针对实际读写集合”）。

**验证点**：

- 输入固定：运行开始后，(1) 修改一条被读取任务的 `due`；(2) 插入一条新的未完成任务；(3) 把一条已完成任务改回未完成。三种情况下 `proc.apply` 都因读集合前置条件不成立而 `REVISION_CONFLICT`，运行进入 `failed`，文档无任何变化，旧结果保留。修改任意任务的 `owner`（不在读集合内）不影响应用。
- 人工修改保护：人把某个 `risk` 改掉后重新运行 → 该单元格被跳过并出现在 `warnings`；若在候选生成之后、应用之前才被人修改 → `expect.rev` 不成立，整批冲突。
- 错误候选：通过 `params.inject` 让程序产出非法操作（未知选项、悬空引用、超限）→ `prepare` 与 `commit` 都整批拒绝。
- 一次应用是一个 Commit，可由 `doc.undo` 整体补偿。
- 新鲜度：订阅方凭变化流中的 `value`/`schema` 类别事件，把“基于 `inputs` 中某实体旧 `content_rev` 的结果”标记为过期；Cell 的移动或筛选修改不产生这类事件（V19）。

真实 Agent 接入时替换的只是“候选从哪里来”；`runs` 生命周期、候选格式、`prepare`/`commit` 链路不变。Mock 的所有输出在 `runs` 与 `meta.derived.program` 中带有 `mock.` 前缀，UI 必须显示它是模拟结果。

---

## 8. 仓库接入事实（M0 输入）

以下为 2026-10-04 对本机各仓库的只读核查结果，路径相对 `buckyos/`，除非另注。它们是事实记录，不是设计；实现前按行号复核，仓库在演进。

### 8.1 CYFS / ndn-lib

| 事实 | 位置 | 对本设计的影响 |
| --- | --- | --- |
| `ObjId` 的 serde 形式是 `类型:十六进制`；`Display` 输出 base32，而固有方法 `to_string()` 输出十六进制 | `cyfs-ndn/src/ndn-lib/src/object.rs` L22–39、L136–139、L247–251 | JSON 中统一用十六进制形式；Rust 代码里不要用 `format!("{}", id)` 生成要存储的字符串 |
| `build_named_object_by_json` 在 `serde_jcs` 出错时回退为 `"{}"`（2026-10-05 复核：本地 `cyfs-ndn` 已改为 panic 并新增可失败的 `try_build_named_object_by_json`；buckyos 经 git 依赖取得的版本尚无后者） | 同上 L375 | `core::canonical` 不调用它，改为自行 `serde_jcs::to_string(..)?` 后 `build_obj_id`；先做严格预校验 |
| `serde_jcs` 0.2.0 对超出 2^53 的整数原样输出，与 RFC 8785/JS 端不一致 | `ndn-lib/Cargo.toml`；上游 issue | 预校验拒绝此类整数（§3.4.2 已在值层禁止） |
| `serde_json::Value` 无法表达重复键；解析时重复键的处理未核实 | — | 导入路径用自定义的严格解析器检测重复键后再转为 `Value` |
| 文档写“字符串应为 NFC”，代码中没有任何归一化 | `cyfs-ndn/doc/CYFS Protocol/CYFS Protocol.md` L445 | 与第一期文档一致：Workspace 不做归一化；跨语言向量中加入非 NFC 字符串以固定这一行为 |
| `jobj` 不是声明的常量，是分发路径对无类型 JSON 的缺省类型；自定义类型名须匹配 `[A-Za-z0-9-]{1,64}` | `ndn-lib/src/cyfs_dispatch.rs` L136–140 | 第一期全部 Workspace 对象使用 `jobj`，与第一期文档 §2.1 一致 |
| `NamedDataMgr::put_object` 不校验 ID 与内容是否匹配；同 ID 不同内容返回 `AlreadyExists` | `cyfs-ndn/src/named_store/src/named_store.rs` L215–219 | `store` 的对象适配层写入前自行校验，并且总是写入 canonical 原文 |
| 递归保留与“完整存储”判断只认识 `cydir`、`cyfile`、`clist` | `named_store/src/store_db.rs` L784–827 | 对 Workspace 自定义对象做 `Recursive` pin 只保护根。适配层必须按 §2.10.3 的保留根集合**逐个显式 pin**，owner 用 `aiworkspace:<workspace_id>`；暂存上传用带 TTL 的 `Lease` |
| pin 接口：`pin(obj_id, owner, PinScope, ttl)`、`unpin`、`unpin_owner` | `named_store/src/ndm.rs` L1279–1305 | 同上 |
| chunk 写入 `put_chunk_by_reader` 单 chunk 上限 32 MiB；`clist` 成员必须是带长度前缀的 mix 类型；`clist` 的 ID 算法带长度前缀，`verify_named_object` 对它不适用 | `ndm.rs` L1214–1216；`chunk/chunk_list.rs` L35–74 | 大于 32 MiB 的资产与记录文件用 ChunkList；校验走各类型自己的算法 |
| buckyos 以 git 依赖（`branch = "main"`）引用 `ndn-lib`、`named_store`；服务通过 `runtime.get_named_store()` 取得存储 | `src/Cargo.toml` L55–58；`src/kernel/buckyos-api/src/runtime.rs` L291–310 | `aiworkspace-store` 走同一模式；独立测试模式自建一个临时目录下的 store |
| 浏览器侧已有 ndn-lib 的移植：`buildNamedObjectByJson`、`ObjId`、`FileObject`、`SimpleChunkList` 等 | `buckyos-websdk/src/ndn_types.ts` | 优先复用。已知与 Rust 不一致处：`OBJ_TYPE_CHUNK_LIST = 'cl'`（Rust 为 `clist`）、拒绝 `-0`；涉及 ChunkList 的 ID 计算前必须先修正并加向量 |
| 现成向量：8 条标准对象 `类型:hex:{json}`；RFC 8785 规范串用例 | `cyfs-ndn/doc/CYFS Protocol/CYFS协议现有标准对象实例.md` L36–80；`object.rs` L652–711 | V01 的向量集 = 这些 + Workspace 各内置类型的内容对象、ContentRoot、非法输入表 |

### 8.2 服务运行时与协议

| 事实 | 位置 | 对本设计的影响 |
| --- | --- | --- |
| `src/frame/aiworkspace` 已存在但为空目录且未被 git 跟踪（2026-10-05：已按 §2.1 建立 `core`/`store`/`server`，另加 `wasm`） | — | 直接在其中建立 §2.1 的结构 |
| 仓库自带服务实现技能与交付清单 | `harness/SKILLS/{implement-system-service,buckyos-intergate-service,design-krpc-protocol,design-durable-data-schema,service-dv-test}`、`harness/checklists/System Service Delivery Checklist.md` | 实现时必须遵循。其中集成技能的 RBAC 步骤和端口表已过时：RBAC 现在在 `rbac_config.rs` |
| 新服务的全部接入点可对照加入 nfs-server 的提交 `0a39fec1` | 见下方清单 | |
| 启动序列：`init_buckyos_api_runtime(name, None, KernelService)` → `login()` → `set_main_service_port()` → `get_my_settings()` → `set_buckyos_api_runtime()` | `src/frame/nfs_server/src/main.rs` L131–189；`src/frame/msg_center/src/main.rs` L1246–1330 | 系统模式照此；带 `--data-dir` 时进入独立模式（nfs_server 的双模式 `main` 是模板） |
| kRPC 请求 `{method, params, sys:[seq, token?, trace_id?]}`，令牌在 `sys[1]`；错误是纯字符串且 HTTP 状态恒为 200 | `buckyos-base/src/kRPC/src/protocol.rs` | §5 的结构化结果裁决 |
| `buckyos-http-server` 的 `serve_http_by_rpc_handler` 请求体上限 1 MiB，可用 `_with_limit` 提高；`Runner::new(port)` 绑定 `0.0.0.0` | `buckyos-base` | 提高到 §5 的 8 MiB；绑定地址是既有行为，鉴权必须在服务内完成 |
| 网关对 `/kapi/<service>` 的通用转发不做逐请求鉴权 | `src/rootfs/etc/boot_gateway.yaml` L217–293 | 每个方法都要自己验证令牌 |
| 取调用方身份：`runtime.enforce(&req, action, resource)`，或 `verify_trusted_session_token` + `validate_verify_hub_token_claims(.., TokenUse::Session)`；用户为 `token.sub`，应用为 `claims.target.canonical_key()` | `runtime.rs` L1535、L1595–1657；`src/kernel/task_manager/src/server.rs` L150–175 | §2.9 的“可信主体”。Desktop 登录目标是 `system:control-panel`，所以来自 Desktop 的调用应用身份是它 |
| 混合自定义 HTTP 路由与 kRPC 的写法 | `msg_center/src/main.rs` L196–269 | §5 的上传/下载路由照此 |
| 类型化客户端模式：trait + `enum Client { InProcess, KRPC }` + `ServerHandler` | `src/kernel/buckyos-api/src/task_mgr.rs` L1989–3050 | `buckyos-api/src/aiworkspace_client.rs` 照此；进程内变体同时用于测试和 Mock |
| 推送：kevent。服务端 `runtime.get_kevent_client().pub_event(path, json)`；浏览器 `buckyos.subscribeKEvent(patterns, cb)`。kevent 流端点没有鉴权，且可能丢事件 | `runtime.rs` L2237；`buckyos-websdk` `sdk_core.ts` L383；`src/kernel/kevent/src/http.rs` | 【裁决】唤醒提示发布到 `/aiworkspace/<workspace_id>/head`，负载只有 `{ "epoch": "ep_...", "head_seq": N }`，不含任何内容或实体 ID。客户端收到后调用 `get_changes`；同时保留 `wait_changes` 长轮询作为兜底。`workspace_id` 是高熵随机值，路径本身不泄露内容 |
| 没有面向浏览器的 WebSocket 服务；SSE 只有 nfs_server 在用 | — | 第一期不引入新的推送通道 |

新服务接入点清单（以 nfs-server 为对照，`aiworkspace` 各需一处）：

1. `src/Cargo.toml` workspace members（三个 crate）。
2. `src/bucky_project.yaml`：`modules` 与 `apps.buckyos.modules`（模块键必须等于二进制名）。
3. `src/kernel/buckyos-api/src/aiworkspace_client.rs`（服务名、端口、Settings、客户端）+ `lib.rs` 的 `mod`/`pub use` 与 `generate_aiworkspace_doc()`。
4. `src/kernel/scheduler/src/system_config_builder.rs` 的 `add_aiworkspace()`，`scheduler/src/main.rs` 的构建链与测试断言。
5. `src/kernel/buckyos-api/src/rbac_config.rs`：`g, system:aiworkspace, frame`。
6. `node_control.rs` 的 `PROCESS_KILL_BASELINE`；`src/stop.py`、`src/rootfs/bin/stop.py`、`stop.ps1`、`stop_osx.sh`。
7. `src/rootfs/bin/aiworkspace/kernel_pkg.toml`；根 `.gitignore`。
8. `doc/port_usage.md`。端口：**4120**（已确认；核查时未被任何内核/框架代码引用）。
9. 网关：使用通用 `/kapi/aiworkspace` 规则，不需要改 `boot_gateway.yaml`。

已有 zone 不会自动获得新服务的 spec，DV 环境需按既有做法重建或手写 system_config 键。

### 8.3 存储位置与数据库库

- 路径规范（`doc/path_usage.md` L74–99、L136–145）：软重置会删除服务数据与缓存分区，保留 `user_data`（`data/$appid`）与 `storage`。Workspace 文档是用户数据，必须放在软重置保留的位置。`runtime.get_data_folder()` 对内核/框架服务返回 `$ROOT/data/<app_id>`，与 `user_data` 分区一致。**建议**：文档库放在 `runtime.get_data_folder()/workspaces/`。注意 `buckyos-kit` 的 `get_buckyos_service_data_dir()` 返回的是 `data/var/<name>`（会被软重置清除），不要用它存文档。
- 仓库中 rusqlite（0.32.1，bundled）与 sqlx 并存。实现技能要求 rusqlite 调用走 `spawn_blocking`、开 WAL。交付清单倾向结构化数据使用系统 RDB 实例（sqlx，由 scheduler spec 声明）。
- 【已确认 2026-10-04】每个 Workspace 一个文件夹，内含 rusqlite 管理的 `doc.sqlite` 与 `local.sqlite`（§4.1）；不使用系统 RDB 实例，没有服务级数据库。这与交付清单“结构化数据使用系统 RDB 实例”的一般建议不同，理由是文件夹即文档的可迁移单元，且写执行者需要对单写连接做精确事务控制和一致性备份。

### 8.4 Desktop

| 事实 | 位置 | 影响 |
| --- | --- | --- |
| 应用注册需要改 6 处：应用目录项、内置应用白名单、懒加载面板、图标、i18n 词条、桌面网格位置 | `src/frame/desktop/src/mock/data.ts`（生产环境也使用）、`src/app/backend-apps.ts` L4–17、`src/app/registry.tsx` L29–48、`src/components/DesktopVisuals.tsx` L28–48、`src/i18n/dictionaries.ts` | 新应用 ID `aiworkspace`，面板 `src/app/aiworkspace/AIWorkspaceAppPanel.tsx` |
| 没有文件类型关联机制；文件浏览器固定用 Preview 打开 | `src/app/filebrowser/FileBrowserView.tsx` L309–316 | “默认应用”在第一期的含义是内置应用入口；`.bcanvas` 双击打开不在范围内，从应用内导入 |
| 后端调用：`buckyos.getServiceRpcClient(name)` → `/kapi/<name>/` | `buckyos-websdk/src/runtime.ts` L276–290 | 前端 API 层 `src/app/aiworkspace/api/` |
| 技术栈：Vite 8、React 19、TS 5.9（`erasableSyntaxOnly`、`verbatimModuleSyntax`，不能用 enum/namespace）、pnpm；lint 严格 | `package.json`、`tsconfig.app.json` | 新目录保持 0 lint 问题 |
| 当前**没有** Service Worker、WASM、SQLite WASM、ProseMirror、Tiptap、Yjs、Loro 依赖；仅 canvas 原型用了一个 Web Worker 和 IndexedDB | `package.json` | §6 的离线能力全部是新增；Vite 对 WASM 与 Worker 的打包配置属于 M1 探针 |
| 产物复制到 `bin/control-panel/web/`，由 control_panel 的目录处理器提供 | `src/bucky_project.yaml` L66–69、L118；`src/frame/control_panel/src/main.rs` L1165–1181 | 不能假设可设置 COOP/COEP 头 → `opfs-sahpool`（§6.1）；Service Worker 的响应头要求需在 M1 实测 |
| 测试：Playwright（mock、real、dv 三套配置）；数据模型单测用 `node --experimental-strip-types` | `playwright*.config.ts`、`tests/datamodel/` | 真实后台的 e2e 参照 `tests/e2e/fixtures/nfs-copy.ts` 启动独立模式进程 |

### 8.5 新增依赖（已确认）

仓库 `AGENTS.md` L90 规定“引入新的依赖项或通用组件时，必须和用户确认”，L91 规定 git 依赖只能用 `branch = "main"`。负责人已于 2026-10-04 确认：本期是严肃的架构验证，只要设计合理，可以引入成熟的第三方库。下表依赖可直接添加；表外的新依赖按同一标准（成熟、许可兼容、设计上确有必要）自行判断并记入附录，拿不准的再询问。版本为调研时所见：

| 依赖 | 用途 | 调研时版本 / 许可 |
| --- | --- | --- |
| Rust `loro` | 富文本 CRDT（后台与 WASM 内核） | 1.16.2 / MIT |
| npm `loro-crdt`、`loro-prosemirror` | 浏览器端 CRDT 与编辑器绑定 | 1.16.4、0.4.4 / MIT |
| npm `prosemirror-model`、`-state`、`-view`、`-transform`、`-keymap`、`-commands`、`-schema-list` | 编辑器。是否加 Tiptap（`@tiptap/core` 3.x，MIT）由前端实现者在 M3 决定；内核不依赖它 | `prosemirror-model` 1.25.x / MIT |
| npm `@sqlite.org/sqlite-wasm` | 离线副本存储 | 3.53.x / Apache-2.0 |
| Rust WASM 工具链（`wasm-bindgen` 及构建步骤） | 把 `aiworkspace-core` 编译给浏览器 | 需同时确认它如何进入 `buckyos-build` 流程 |
| Rust 十进制运算库（如 `rust_decimal`）**或**自实现 | `decimal` 的规范化与比较。第一期只需要解析、规范化、比较，不做算术；自实现约百行，建议不引入依赖 | — |

不建议引入：Rust `prosemirror` crate（0.5.x，近期才活跃、使用量极小）。第一期的富文本 schema 是冻结的小集合，校验器按 §3.3.2 的定义文件自行实现并用双端 fixtures 保证一致，比依赖一个未经验证的通用移植更可控。

---

## 9. 实施任务拆分

在第一期文档 M0–M5 的框架内细化。每个任务给出交付物和它使哪些验收项可测。任务内部顺序即建议的提交顺序；带 ⟂ 的任务之间没有依赖，可并行。

### M0：确认与骨架

| 任务 | 交付物 |
| --- | --- |
| M0-1 复核 §8 全部事实并更新行号 | 本文 §8 的修订 |
| M0-2 存储决策、依赖、端口已确认（见附录），无需再问 | — |
| M0-3 建立三个 crate 的空骨架并进入 workspace 与构建配置 | `cargo build`、`buckyos-build` 通过 |

### M1：格式、编码与探针

| 任务 | 交付物 | 验收 |
| --- | --- | --- |
| M1-1 `core::id`、`core::canonical`（严格预校验 + JCS + ObjectId）、`core::value`（§3.4.2） | Rust 单测；向量文件 `fixtures/vectors/*.json` | V01（Rust 侧） |
| M1-2 ⟂ `core` 的 WASM 构建与浏览器测试页；同一批向量在浏览器中通过；复用或修正 websdk `ndn_types.ts` | 构建脚本；浏览器测试 | V01（浏览器侧） |
| M1-3 ⟂ 富文本探针：schema 定义文件 → 前端 PM schema；`loro-prosemirror` 接入；`decode_ast`、`build_doc`、`apply_block_ops`；§3.3.1 的全部探针问题 | 双端 fixtures；ADR（结论、实际版本、失败用例、是否启用退路） | 为 V06、V07 奠基 |
| M1-4 ⟂ 离线存储探针：`opfs-sahpool` Worker、Web Locks 持有者、Service Worker 缓存应用资源、断网冷启动 | 探针页面；ADR（支持的浏览器与版本、失败时的状态） | 为 V15、V16 奠基 |
| M1-5 各内置类型的 JSON Schema、操作请求/响应的 JSON Schema、错误码表 | `schemas/`；有效与非法 fixtures | |
| M1-6 性能预算：按第一期文档 §8.3 写下目标硬件与各项延迟/内存预算 | 预算表 | |

M1 出口：第一期文档 §9 末段的八个问题逐条有书面答案，其中富文本互操作与离线 VFS 两条必须有可运行的证据。

### M2：内核与后台

| 任务 | 交付物 | 验收 |
| --- | --- | --- |
| M2-1 `store`：文档库 DDL、打开/版本检查、写执行者、`ReadCtx`/`WriteCtx` 的 rusqlite 实现 | | |
| M2-2 引擎：§2.5.2 管线、覆盖层、幂等、`commits`/`commit_ops` | 故障注入钩子（事务前、事务中、提交后发布前） | V10、V12 |
| M2-3 通用操作与 Container 适配器（§3.1）；`refs` 维护与重建校验 | | V02、V03 |
| M2-4 ⟂ RecordObject、Cell（不含表格查询）、Annotation、AssetRef 适配器 | | V08（注释部分）、V22 |
| M2-5 ⟂ TableSource 适配器：记录与单元格、字段与选项、迁移、唯一约束 | | V08、V09、V11 |
| M2-6 过滤求值器、排序、`query` 与 cursor | 差分测试框架（即使暂无 SQL 下推，也先固定求值器行为） | V08 |
| M2-7 权限：`local.sqlite`、`grants`、各检查点、读投影 | | V21 |
| M2-7b 写锁：`write_policy`、租约表、`lock.*` 接口、提交检查、kevent 提示 | 真实进程集成测试（含杀进程后租约到期接手） | §2.11 测试要点 |
| M2-8 变化流：`get_changes`、`wait_changes`、权限过滤、kevent 唤醒 | | V19 |
| M2-9 撤销：逆操作、`doc.undo`、部分撤销 | | V13（后台部分） |
| M2-10 物化、`checkpoint`、导出、导入、Fork、保留根 | | V04、V05、V20 |
| M2-11 `server`：双模式 `main`、kRPC 分发、令牌校验、上传下载路由、`buckyos-api` 客户端 | 独立模式集成测试（真实进程） | V23（后台部分） |
| M2-12 `project-workspace` fixture（Commit 序列）与性能 fixture 生成器 | 期望 ContentRoot | |

M2 出口：不启动任何前端，通过服务接口重放 fixture、修改、查询、导出、导入、Fork；杀进程重启后已接受提交仍在；混合批次故障注入无部分提交。

### M3：富文本接入与 Desktop 应用

| 任务 | 交付物 | 验收 |
| --- | --- | --- |
| M3-1 RichText 适配器：`apply_update`、块级操作、AST 投影、存储合并、物化 | | V06、V07、V10（含富文本） |
| M3-2 Desktop 应用骨架：注册 6 处、API 层、在线直连模式下的大纲、流式页面、Cell 容器 | | V23（前端部分） |
| M3-3 ⟂ 表格视图：虚拟滚动、单元格编辑、字段与选项管理、筛选排序、保存视图、迁移预检界面 | | V08、V09 |
| M3-3b 写锁的前端：申请与续约、只读态与持有者显示、失锁处理、管理员开启策略与强制解除 | | §2.11 测试要点 |
| M3-4 ⟂ 富文本编辑器：schema、`block_id` 插件、确认/工作文档、`object_embed` 渲染、中文输入法验证 | | V06 |
| M3-5 UndoCoordinator；冲突呈现；四种保存状态 | | V13 |
| M3-6 注释与资产的最小界面 | | V08、V22 |
| M3-7 URL 大表只读适配、引用/物化联合类型、查询缓存隔离与定义导出 | 按页生成且不提供 ObjectId 的源 fixture；不全量读取与切片物化测试 | V24 |

### M4：离线与受控加工

| 任务 | 交付物 | 验收 |
| --- | --- | --- |
| M4-1 Replica Worker：`replica.bootstrap`、副本库、本地规划与前像、变基 | | V15 |
| M4-2 发送循环、未知结果处理、重连顺序、资产延迟上传 | | V12、V17 |
| M4-3 `explicit` 策略：表格草稿折叠、富文本私有分叉与 `diff_blocks` 提交 | | V14 |
| M4-4 多标签页持有者、存储故障注入（写满、事务失败、Worker 退出） | | V16 |
| M4-5 ⟂ `runs`、`proc.*`、`mock.task-summary@1` 与界面入口 | | V18、V19 |
| M4-6 个人恢复备份（后台部分 + 客户端附加部分）与普通分享导出的隐私测试 | | V14、V04 |

### M5：系统接入与验收

| 任务 | 交付物 |
| --- | --- |
| M5-1 §8.2 清单中的全部系统接入点；DV 环境启动、鉴权路径、启停 | V23 |
| M5-2 V01–V24 自动化全部接入 CI 可执行的命令；崩溃测试三个位置 | 测试报告 |
| M5-3 按 M1-6 的预算在同配置下复测 | 性能报告，超预算项给出瓶颈与边界 |
| M5-4 回写：把本文的【偏离】与【裁决】同步进第一期文档；更新已知限制清单 | 文档修订 |

### 9.1 验收项与本文章节对照

| 验收 | 主要依据 |
| --- | --- |
| V01 | §2.10.1、§8.1 |
| V02 | §2.10.1、§3.1、§3.4.4 |
| V03 | §3.1、§2.5.2 第 3 步 |
| V04、V05 | §2.10.2 的不变量、§4.4 |
| V06、V07 | §3.3.4、§3.3.7、§3.3.8、§3.3.9 |
| V08 | §3.4、§3.5、§3.7 |
| V09 | §3.4.2、§3.4.6 |
| V10 | §2.5.2、§3.3.4 |
| V11 | §2.2、§3.1 删除规则、§3.4.3 |
| V12 | §2.6、§6.4 |
| V13 | §2.7 |
| V14 | §3.3.5、§3.3.9、§2.9 个人作用域 |
| V15、V16、V17 | §6 |
| V18、V19 | §7、§2.8 |
| V20 | §3 开头的降级规则、§4.2 版本检查 |
| V21 | §2.9 |
| V22 | §3.6、§2.10.3、§8.1 的 pin 事实 |
| V23 | §8.2、§8.4 |
| V24 | §2.4、§2.5.4、§2.10、§3.4.11、§4.4 |

---

## 10. 已知限制与留待后续的决定

第一期按本文实现后明确存在的限制，需写入交付文档：

1. 私有草稿与客户端撤销栈只在产生它的浏览器中；换设备不延续（§2.9）。
2. 显式提交策略下，富文本同一块的并发修改表现为冲突，不做字符级合并（§3.3.5）。
3. 由编辑器按键产生的富文本提交不能被后台补偿撤销，只能在原编辑会话内撤销（§2.7）。
4. CRDT 历史不裁剪，富文本存储只增不减（§3.3.8）。
5. 没有字段级、记录级授权，没有组，没有“拒绝”规则（§2.9）。
6. 部分读权限的主体不能准备离线副本（§2.9）。
7. 文本排序按码点，不支持本地化排序规则（§3.5.3）。
8. 没有文件类型关联，`.bcanvas` 只能在应用内导入（§8.4）。
9. 不做资产与对象的自动回收（§2.10.3）。
10. 手工排序以整个视图为一个并发单位（§3.5.3）。
11. URL 大表路径第一期只读；未提供源快照或未取得的记录集不承诺可复现与离线。普通内置表和可物化结果仍优先使用 ObjectId（§3.4.11）。
12. 写锁只有实体级粒度，不沿树继承，没有排队和长期签出；要求写锁的对象离线时只读（§2.11）。
13. 对象树的自动合并依赖权威后台定序；离线期间的移动要到重连后才确定最终位置（§2.2.3、§3.1）。
14. 对象与资产未接入 Zone named store；单个资产或物化文件不超过 32 MiB（附录 2026-10-05）。
15. 86k 字符/1000 块的富文本上，一次 `richtext.apply_update` 提交在后台约 50 ms（候选分叉 + 全文解码校验 + AST 投影重写）；编辑反馈在本地，提交按 300–1000 ms 合并。大文档的增量校验留待测量后决定（验收报告性能一节）。

验证后才能决定、且本文已给出判据的事项：是否需要记录值索引（§4.2）；是否把大批次规划移出写临界区（§2.5.2）；记录文件是否分桶（§3.4.9）；是否启用富文本退路（§3.3.1）；`synchronous` 级别（§4.1）。

## 附录：决策记录

实施中每次确认或偏离在此追加一行。

| 日期 | 条目 | 结论 | 依据 |
| --- | --- | --- | --- |
| 2026-10-04 | §8.5 新增依赖 | 可以引入成熟的第三方库；本期定位是严肃的架构验证 | 负责人确认 |
| 2026-10-04 | §4.1 / §8.3 存储 | 每个 Workspace 一个文件夹，逻辑上简单优先；以后在保持语义的前提下再优化 | 负责人确认 |
| 2026-10-04 | §8.2 端口 | 4120 | 负责人确认 |
| 2026-10-05 | §4.4、§5 覆盖导入授权 | `restore` 覆盖已有 Workspace 须对其有 `manage`；授权沿用现有文件夹，不取自包 | 外部评审第 1 项 |
| 2026-10-05 | §6.2 副本构建 | 按白名单从零构建；不含 `commits`/`commit_ops`；字节级搜索测试 | 外部评审第 2 项 |
| 2026-10-05 | §2.5.2 幂等查询时机 | 移到入口校验，先于规划 | 外部评审第 3 项 |
| 2026-10-05 | ObjectId 与大数据引用 | 尽力使用 ObjectId；少数明确的大数据场景允许 URL + 查询参数，显式声明能力损失，不强制全量物化 | 负责人明确 |
| 2026-10-05 | §2.6 第 7 条、§4.4 `epoch` | 恢复生成新历史代次，旧客户端得到 `EPOCH_MISMATCH` 并重新同步 | 外部评审第 4 项 |
| 2026-10-05 | §2.8、§6.3、§6.4 自身提交的识别 | 变化流向作者回传 `idempotency_key`；`unknown` 行在被后台定性前不标为冲突 | 外部评审第 5 项 |
| 2026-10-05 | §3.3.3、§3.3.5 块位置令牌 | `BlockIndex` 增加 `struct_rev`；移动与删除校验它 | 外部评审第 6 项 |
| 2026-10-05 | §4.2 `refs` 主键 | 主键包含目标；空串代替 NULL；DDL 与最小样例已执行 | 外部评审第 7 项 |
| 2026-10-05 | §2.2、§7 集合级读集合 | 新增 `field.values_rev` 与 `source.members_rev`；Mock 用它们保护筛选成员集合 | 外部评审第 8 项 |
| 2026-10-05 | §3.4.4 字段恢复 | 新增内部操作 `table.restore_field` 及其校验规则 | 外部评审第 9 项 |
| 2026-10-05 | §2.2.3、§3.1 对象树并发 | 取 CRDT 的合并语义（后到者生效、成环丢弃、删除优先），不用 CRDT 数据结构；删除子树须显式列出后代。偏离第一期文档 §4.5，待回写 | 负责人确认 |
| 2026-10-05 | §2.11 写锁 | 新增按对象开启的写锁（实体级租约）；要求写锁的对象离线只读，第一期不做签出 | 负责人确认 |
| 2026-10-05 | §2.1、§2.10 内核与 ndn-lib | `core` 不依赖 ndn-lib（它带 tokio/reqwest，不能进 WASM）：canonical 编码、`jobj`/`cyfile` ObjectId、`mix256` chunk id 按同一规则自实现，由 `store/tests/kernel.rs::v01_object_ids_match_ndn_lib` 与 ndn-lib 逐项比对，`fixtures/vectors/object-ids.json` 供 Rust 与浏览器 WASM 共用 | 实施：`core` 须可编译为 `wasm32-unknown-unknown` |
| 2026-10-05 | §2.10.3、§3.6、§8.1 对象存储 | 对象与 chunk 存于 `<数据目录>/objects/{objects,chunks}/`，文件名即 `ObjId::to_filename()`，写入与读取都校验内容与标识一致。**尚未接入 Zone named store**（无 pin/Lease、不可经 NDN 获取）；`store::objects::FsObjectStore` 是替换边界。单文件 = 单个 `mix256` chunk，上限 32 MiB，未写 ChunkList | 实施取舍；named store 接入为后续集成任务 |
| 2026-10-05 | §2.5.1 TypeAdapter 形态 | 未做成 trait 对象注册表：`core::plan` 按操作名分派到 `plan`/`plan_table`/`plan_richtext`，键式文档类型的校验与引用枚举在 `core::types`，物化在 `core::materialize`。“适配器只能返回写入、不能自行提交”由 `Overlay` 结构保证（规划函数拿不到存储）。操作目录见 `schemas/operations.json`（与代码有一致性测试） | 实施：内置类型数量固定，trait 分发未带来收益 |
| 2026-10-05 | §2.7 部分撤销 | `mode: "partial"` 的可撤销部分通过对每个逆操作单独试算得到；`plan_digest` = 可撤销逆操作列表的摘要 | 实施 |
| 2026-10-05 | §4.4 导入 | 包内容不直接写库：`core::materialize::load_ops` 把内容根转换为一批 `entity.create`/`table.insert_records` 等操作，以 `origin: "import"` 经**同一个规划器**执行（导入模式下引用目标允许在同批稍后创建、批末统一校验），随后重物化并断言 ContentRoot 相等 | 实施：每个内容对象因此通过与普通写入相同的 schema 校验 |
| 2026-10-05 | §5 HTTP 路由 | 导出包与副本库的下载路由带 `workspace_id`：`/export/<workspace_id>/<export_id>`、`/replica/<workspace_id>/<replica_id>`；新增 `doc.outline`、`doc.source_capabilities`、`diag.verify_refs` | 实施：没有服务级数据库可用来反查 export_id |
| 2026-10-05 | §6.3 工作视图 | 采用“确认层 + 按序重放待提交操作”（第一期文档 §3.4 末段允许的做法），不记录前像；引擎是 `core::replica`，后台测试与浏览器 Worker 共用 | 实施；见第一期文档 §11-12 |
| 2026-10-05 | §6 离线副本 | `opfs-sahpool` 在无 COOP/COEP 的静态服务下可用；Service Worker 冷启动在 `vite preview` 下通过。CommitTransport 留在页面而非 Worker；无 `preimage_json`/`richtext_working`；`blocked_asset` 未实现。仅 Chromium 验证 | 实施；见验收报告 §5.2 |
| 2026-10-05 | §3.4.11 URL 源 | 服务只访问已注册的 Source Adapter（按 URL scheme）；内置仅测试用 `fixture://` 生成源，没有通用 HTTP 抓取。未注册的 scheme：定义照常保存，查询返回 `DEPENDENCY_UNAVAILABLE` | 实施：文档中的 URL 不能驱使服务发起任意网络请求 |
