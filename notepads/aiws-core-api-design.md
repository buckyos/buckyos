# AIWS 核心 API 设计草案 v0

状态：草案 v0，未实施、未验证。2026-11-12 按 owner “细节要看了效果才知道对不对，先拍个脑门设计”的要求写出，给 [核心 API 调整 TODO](aiws-core-api-unification-todo.md) 的 M0 事项各定一个取值。裁决 R1–R19 见 TODO §0.1，本文不重复理由。

读法：每一节是一个可以直接照着实现的取值。实现中发现不对就改，改动回写本文。§12 列了最没把握、最可能改的地方。

## 1. 接口总览

| Agent 要做的事 | kRPC | 对象入口 | CLI（`buckyos aiws …`） |
| --- | --- | --- | --- |
| 列一个节点的直接子项 | `doc.list` | Workspace index，`op=ls` | `ls [路径]` |
| 按名称、类型找候选 | `doc.find` | Workspace index，`op=find` | `find <字面串>` |
| 在正文里找字面串 | `doc.query { text }` | Workspace index，`op=grep` | `grep <字面串>` |
| 对表做 SQL 查询 | `doc.query { sql }` | 首批不提供 | `sql "<SELECT …>"` |
| 看一个实体是什么 | `doc.read { as: "meta" }` | 实体对象的 `meta` 属性 | `stat <路径>` |
| 读纯文本呈现 | `doc.read { as: "body" }` | 实体对象的 `body` 属性 | `cat <路径>` |
| 读原生结构 | `doc.read`（现有） | 实体对象的 `content` 属性 | `read <路径>` |
| 路径转引用 | `doc.resolve`（现有） | 路径对象 | `resolve <路径>` |
| 写入 | `doc.commit`（现有） | 首批不提供 | `table …`、`text …`、`mv`、`rename`、`set-prop`（§9） |
| 读 URL 表的远端行 | `doc.source_query` | 不提供 | 不提供 |
| （对象入口内部用） | `doc.index { op: ls \| find \| grep, … }`（internal） | Workspace index 的绑定 | — |

方法调整：

| 方法 | 变化 |
| --- | --- |
| `doc.list` | 新增，取代 `doc.list_children`（后者删除） |
| `doc.find` | 新增 |
| `doc.query` | 只接受 `text` 或 `sql` 二者之一；`view_id`、`source_id`、`filter`、`sorts`、`fields`、`group` 等参数删除 |
| `doc.source_query` | 新增，承接 URL 表的远端查询，参数就是 `doc.query` 现在给 URL 表用的那组 |
| `doc.read` | 增加 `as`、`path`、`version`、`cursor`、`max_bytes` |
| `doc.resolve` | 路径按 `name` 解析，规则见 §2；结果多 `path`、`name`；固定版本读取不变 |
| `doc.index` | 新增，internal：对象入口 Workspace index 的唯一绑定，按 `op` 转 `doc.list` / `doc.find` / 文本查询（对象提供者框架一个 index 只能绑一个方法） |
| `doc.outline` | 不变，保留给 Desktop 一次取整棵树 |

所有接受 `entity_id` 的读取方法同时接受 `path`，二者给一个。

run token 的方法白名单：`doc.list_children` 换成 `doc.list`；`doc.query` 原本就在白名单里，换成文本和 SQL 之后保留；`doc.find` 不加入。这一条与权限优化 TODO 的 B-2 有关，那边可以再收紧。

## 2. 路径与命名

### 2.1 路径语法

- 路径是 `/` 加各级实体的 `name`，从 Workspace 根开始：`/data/销售`、`/surfaces/看板/text-3`。根是 `/`。
- 第一段可以写成 `~<entity_id>`，表示从这个实体往下：`~sales`、`~board/text-3`。它用于两种情况：调用者手里只有 ID；调用者读不到某一级祖先，此时返回的路径从最高的可读祖先起写成 `~<id>/…`，不暴露读不到的祖先的名字和 ID。
- 名字不能以 `~` 开头，不能是 `.` 或 `..`；其余沿用现有 `check_name`（1–128 个字符，不含 `/` 和控制字符）。
- 路径进 URL 时逐段做百分号转义；在 CLI 和 SQL 里原样写。
- 解析逐段进行，每一段都要求调用者可读；读不到和不存在返回同一个 `NOT_FOUND`。

### 2.2 `name` 必填与自动命名

名字是否“跟随”不存储（v0 的 `name_auto` 字段没有做，§13）：名字仍是规则会给出的那个（标题的名字，或它加 `-<n>` 后缀；没有标题时是本类型的计数名字）就算跟随，否则就是有人定下的名字，不再自动变化。`meta.name_follows` 报告当前是否跟随。`entity.rename { name: null }` 把名字交还给规则，立刻按规则重新生成。

名字按下表生成：

| 类型 | 名字来源 |
| --- | --- |
| 有标题的实体（容器、Block、Wish、控件定义、视口、演示路径） | 联动 `payload.title`；标题为空时用计数名字 |
| Record | 联动属性 `title`，没有时 `name`（存储为 `p:title` / `p:name`） |
| 资源引用 | 联动 `title`，没有时 `file_name` |
| 表、富文本 | 没有标题键（表的 schema 不收 `title`，富文本的键只能是 `source`）：名字就是它的标签；不给名字时用计数名字 `table-<n>`、`doc-<n>` |
| 计数名字的前缀 | Block 用视图类型（`text-3`、`table-1`；非字母数字换成 `-`，如 `ext-acme-vote-1`），容器用 kind（`folder-2`、`group-1`），其余用短名：`table`、`doc`、`record`、`file`、`note`、`wish`、`def`、`dir`、`viewport`、`path`、`task`、`response`、`result`，未知类型取 type_id 最后一段 |

- **联动。** 标题变了（`entity.set_keys` 改了来源键）且旧名字跟随旧标题，内核在同一个 Commit 里按新标题重新生成名字，`meta_rev` 前进。撤销标题时名字跟着回来（它又跟随了）。标题转名字：去掉控制字符，`/` 换成 `-`，去掉首尾空白和开头的 `~`，截到 64 个字符；结果为空、`.`、`..` 就用计数名字。
- **相撞。** 跟随的名字与同级已有的活名字相同时加 `-2`、`-3`……取第一个没被占用的。`entity.create` 显式给的名字如果正是标题的名字，也按跟随处理（相撞时加后缀）——实施中发现许愿格结果、Desktop 等调用方习惯把标题同时写成 name，而同级的实体现在都有了跟随标题的名字，按显式名字报错会让这些调用失败；其他显式名字相撞照旧 `NAME_CONFLICT`。`tree.move` 和 `entity.restore` 时跟随的名字在新位置相撞就加后缀，不再失败；显式名字相撞仍是 `NAME_CONFLICT`。
- **计数。** `n` 等于同一父节点下所有同前缀名字（包括已删除的实体）里最大的序号加一，删掉的实体恢复时名字不会被别人占着。
- **谁分配。** 规划器在 `entity.create` 时分配，写进规范化后的操作（`norm.name`），重放和副本拿到同一个名字；离线副本用同一份规划代码，先得到预测值，服务端接受时以服务端为准。一个 Commit 里连续创建时，规划器按父节点缓存同级名字（遇到非创建操作清空）。
- **个人和参与者范围。** 同级唯一按“命名空间”算：共享实体只和共享实体比；个人 / 参与者实体和共享实体以及自己同一 scope 的实体比（不和别人的个人实体比）。所以别人后来建了同名共享实体时，个人实体被“遮住”：解析时共享优先，其次调用者自己的，再按同级顺序；被遮住的在 `ls` 里标 `shadowed: true`，路径写成 `~<id>`。
- **存量回填。** 存储格式升到 6：打开 5 的 Workspace 时，在一个事务里按树的顺序给没有名字的实体（根除外）生成名字，再把版本写成 6，不经 Commit、不动任何 rev。导入旧包时规划器不起名（包的 content root 要逐字节校验），校验通过后同样回填。离线副本每次打开都重新引导，拿到的就是回填后的名字。

### 2.3 类型过滤器

`type` / `not_type` 的取值是类型短名：`table`、`richtext`、`record`、`annotation`、`block`、`container`、`asset`、`dir`、`wish`、`def`，也接受完整的 `type_id`。容器可以细分为 `container:folder`、`container:surface`、`container:group`、`container:control`。

默认集合：`ls` 不过滤；`find` 和 `grep` 默认排除批注、视口、演示路径、任务、应答、结果，要搜它们就显式写进 `type`。

## 3. 公共 meta

`doc.read { as: "meta" }` 的结果，也是 `ls` / `find` 每一项的超集。读 meta 不扫描正文和记录（表的 `body.etag` 由表、字段和被引用实体的版本算出）。

```json
{
  "entity_id": "sales",
  "ref": { "workspace_id": "ws_…", "entity_id": "sales", "mode": "live" },
  "path": "/data/销售",
  "name": "销售", "name_follows": false, "title": null,
  "type": "table", "type_id": "buckyos.table-source", "kind": null,
  "schema_id": null, "schema_version": 1,
  "scope": "shared",
  "parent": { "entity_id": "data", "path": "/data" },
  "version": { "epoch": "…", "head_seq": 120, "content_rev": 118, "meta_rev": 90, "life_rev": 5, "struct_rev": 12 },
  "body": { "state": "full", "format": "text/csv", "format_version": 1, "etag": "…", "records": null, "bytes": null },
  "data": { "mode": "local" },
  "children": 0,
  "capabilities": ["read", "append", "update", "delete", "structure"],
  "ops": ["entity.delete", "entity.rename", "…", "table.insert_records", "table.set_values", "…"],
  "head_seq": 120
}
```

| 字段 | 说明 |
| --- | --- |
| `name_follows` | 名字是否还跟随标题或计数（§2.2）；不存储，按规则判断 |
| `kind` | 容器的 kind、Block 的视图类型、批注的 kind，其余 `null` |
| `body.state` | `full`（能读到全文）、`definition`（只有定义，URL 表）、`fallback`（未知类型、降级实体以及视口、演示路径、会话实体的原生 JSON）、`unavailable`（附 `reason`：`binary` / `too_large`） |
| `body.etag` | 文本投影的版本标识：格式版本加所依赖实体的 `content_rev` / `meta_rev`（画布、文件夹含子项；表含字段的 `def_rev` / `type_rev` 和 `value` / `body` 引用的目标） |
| `body.records` / `bytes` | 知道就给精确值，不知道给 `null`；不为了填它去扫描（资源引用给 `bytes`） |
| `data` | 数据实体才有：`mode` 为 `local`、`url_query`、`synced`（payload 带 `source` 的本地副本），`source` 是来源摘要，`derived` 是生成记录摘要；只填现在真实存在的状态 |
| `shows` | Block：它显示的数据 `{ entity_id, path, type }`（读不到时 `{ entity_id, available: false }`），表视图再加 `view_sql`、`view_count_sql`、`view_group_sql?`、`view_diagnostics?`，视图编译不过时 `view_error`（§7.4）；数据控件和组合控件：各绑定显示的数据 |
| `definition` | 有 `def_ref` 的实例：定义 `{ entity_id, path, type }` |
| `capabilities` | 调用者当前能做什么，沿用现有计算 |
| `ops` | 这个类型接受哪些 Commit 操作，来自 `operations.json` 的 `types` 列（§10） |
| `degraded`、`lock_holder`、`availability` | 有就给：降级原因、锁的持有者、资源的可用性 |

## 4. body：纯文本呈现

### 4.1 读取

```text
doc.read { workspace_id, path | entity_id, as: "body", selector?, cursor?, max_bytes?, parts? }
→ { entity_id, path, text, format, format_version, etag, state, reason?, complete, next_cursor?, parts? }
```

- body 由若干“部分”顺序拼成，部分之间用换行分隔。部分对应一个 D04 选择器（一条记录、一个富文本块、一个目录成员、一个键），或者对应另一个实体（画布的一个 Block、文件夹的一个子项，`entity_id`）。分页以部分为单位，不在部分中间截断；单个部分超过 `max_bytes` 时才在字符边界切开，并标出 `partial`，下一页从切开处接着读。
- `max_bytes` 默认 32 KiB，范围 64…256 KiB。`complete` 为 false 时带 `next_cursor`。cursor 绑定 `etag`，文本变了返回 `SNAPSHOT_EXPIRED`。表的 cursor 记的是下一条记录的 `record_id`（按 `record_id >= ?` 续扫，不从头解码），表头只在第一页，几页拼起来就是完整的 CSV。
- 带 `selector` 时只返回那一个部分的文本（表的一条记录连表头一起给）。
- 请求里加 `parts: true` 时，额外返回本页每个部分的 `selector` / `entity_id` 和它在 `text` 里的起止偏移（UTF-8 字节）。

### 4.2 各类型的呈现

| 类型 | 格式 | 内容 |
| --- | --- | --- |
| 富文本 | `text/markdown` | 现有的富文本转 Markdown；一个块一个部分（`richtext_block`） |
| 表（本地行） | `text/csv` | 第一个部分是表头行，列为 `_id` 加各活字段的字段名；之后一条记录一行（`table_record`），按 `record_id` 排序；值按 §4.3 的显示形式，RFC 4180 转义。记录带正文时，那一行末尾多一列 `_body`，值是正文实体的路径 |
| URL 表 | `text/plain` | 表名、字段名和类型、来源说明，加一行“行在远端，未物化”；`state: definition` |
| Record | `text/plain` | 每个属性一行 `键: 值`；有正文时空一行后接正文的 Markdown |
| 批注 | `text/plain` | 第一行 `Note on <目标路径>`，引用原文（有的话），之后是批注文本 |
| 文件夹、data、surfaces | `text/plain` | 每个可读子项一行：`<name>[/] <类型短名或 kind> "<标题>"` |
| 画布、分组 | `text/markdown` | `# <标题>`；之后按阅读顺序（先上后下、再左后右，没有坐标时按兄弟顺序）每个 Block 一段，段首是 `[<name>]`，内容是该 Block 的文字；连接线略过 |
| Block | `text/markdown` | 文字视图（mindnode、shape、richtext、label）没有绑定时就是标题本身；其余是 `<视图类型> "<标题>"`，再写它显示什么（路径）：表视图写 `table view of <表路径>`、视图的文字说明、显示的列和视图 SQL；便签写批注文本；`config` 里的 `text`、`label`、`caption`、`alt`、`url`。**不内联它显示的数据的内容**（§12.6、§13） |
| 控件容器（脑图等） | `text/markdown` | 脑图：子 Block 的 Markdown 大纲（`- ` 每层缩进两格，带 slot 时 `[slot]`）；数据控件：`<kind> control, shows <绑定>: <路径>` |
| 资源引用 | `text/plain` | 文件名、媒体类型、大小；媒体类型是 `text/*` 且不超过 256 KiB 的内联原文；其余 `unavailable`，`reason: binary` |
| 资源目录 | `text/plain` | 每个成员一行（`folder_member`）：名字、媒体类型、大小 |
| Wish | `text/markdown` | 任务说明、输入和输出的路径、当前状态 |
| 控件定义 | `text/markdown` | 自描述（`agent` 文档）、参数、绑定声明 |
| 扩展控件实例 | — | 首批没有做控件自己声明的文本模板，按 Block 的通用规则（§13） |
| 未知类型、降级实体、视口、演示路径、任务、应答、结果 | `application/json` | payload 的规范化 JSON；`state: fallback` |

### 4.3 类型化值的显示形式

body 和 SQL 的显示列用同一张表。

| 字段类型 | body 里的文本 | SQL 显示列 | SQL `#raw` 列 |
| --- | --- | --- | --- |
| text | 原文 | TEXT | 同显示列 |
| number | 十进制数字 | INTEGER / REAL | 同显示列 |
| decimal | 存储的十进制串 | REAL（`CAST`，可比较、可求和） | 精确的十进制串 |
| boolean | `true` / `false` | 1 / 0 | 同显示列 |
| date、datetime | 存储的 ISO 串 | TEXT | 同显示列 |
| select | 选项标签 | TEXT（标签） | `option_id` |
| multi_select | 标签用 `; ` 连接 | TEXT（标签的 JSON 数组） | `option_id` 的 JSON 数组 |
| object_ref | 目标的标题，后跟 `(<路径>)` | TEXT（目标路径） | 引用对象的 JSON |
| currency | `<币种> <金额>` | REAL（金额）；另有 `<字段名>#currency` 列放币种 | `{ currency, amount }` 的 JSON |
| json | 紧凑 JSON | TEXT | 同显示列 |
| 公式值、空值 | 空 | NULL | 存储的原值或 NULL |

## 5. 发现

```text
doc.list { workspace_id, at?, type?, not_type?, schema?, limit?, cursor?, version? }
→ { at: { entity_id, path }, items: [ … ], next_cursor?, head_seq }

doc.find { workspace_id, at?, depth?, name?, type?, not_type?, schema?, limit?, cursor? }
→ { items: [ … ], complete, next_cursor?, head_seq }
```

- `at` 是路径或 `entity_id`，缺省为 `/`。
- 每一项：`entity_id`、`name`、`path`、`type`、`kind`、`title`、`children`、`content_rev`、`body.state`、`body.format`；`find` 多一个 `match: "name" | "title"`，个人实体被遮住时多 `shadowed: true`。
- `ls` 按树的顺序返回直接子项；默认 100 条，最多 1000。
- `find` 按先序遍历；`name` 是对名字和标题做不区分大小写的字面子串匹配，不给 `name` 就只按类型和 Schema 筛。`depth` 缺省不限，最多 16 层。默认 50 条，最多 200；一次调用最多遍历 20 000 个实体，没走完时 `complete: false` 并给 `next_cursor`。
- cursor 绑定 `head_seq`；树变了返回 `SNAPSHOT_EXPIRED`，调用者重新开始。

## 6. 文本查询

```text
doc.query { workspace_id, text, case?, at?, depth?, type?, not_type?, limit?, cursor? }
→ { hits: [ { entity_id, path, type, on, selector?, snippet, more? } ],
    coverage: { complete, scanned, skipped: [ { entity_id, reason } ] },
    next_cursor?, head_seq }
```

- 字面子串匹配，`text` 长 1–256 个字符，原样比较，不做分词和正则；传了 `regex` 之类未定义的参数返回 `INVALID_OPERATION`。
- 默认不区分大小写（两边都做 Unicode 小写转换）；`case: true` 时按码点精确比较。不做全半角、繁简和音调的归一。
- 匹配对象依次是名字、标题、body 的各个部分；`on` 取 `name`、`title` 或 `body`。body 命中带 `selector`（那个部分的 D04 选择器）；表按单元格匹配，命中给 `table_cell { record_id, field_id }`。容器（文件夹、画布、分组、控件）只匹配名字和标题：它们的 body 是子项，子项自己会被搜到，不重复命中。范围是 `at` 本身（不是根时）和它下面的子树，先序。
- 一段文字里多次出现只给一个命中（带第一处的片段），其余计入 `more`。
- `snippet` 是命中处前后各 40 个字符，换行换成空格。
- 每个实体在一页里最多给 5 个命中，其余数量放在最后一个命中的 `more` 里；要看全部就把 `at` 设成那个实体再查。
- 默认 20 个命中，最多 100。一次调用最多扫描 8 MiB 文本或 1 秒，先到为止（core 没有时钟，时间由服务传入的判断函数决定）；没扫完时 `coverage.complete: false` 并给 `next_cursor`，停在一张表中间时 cursor 带记录 id，下一次从那条记录接着扫。空结果只有在 `coverage.complete: true` 时才表示“没有”。
- `coverage.skipped` 列出没有被搜到正文的实体及原因：`definition`（URL 表）、`binary`、`too_large`。
- 首批逐实体扫描，不建索引。

## 7. SQL 查询

### 7.1 请求与结果

```text
doc.query { workspace_id, sql, params?, tables?, max_rows?, expect_head? }
→ { columns: [ "…" ], rows: [ [ … ] ], truncated,
    read: [ { table, entity_id, path, content_rev } ], head_seq }
```

- `sql` 是单条 `SELECT`，最长 16 KiB；末尾以外出现 `;`（引号和注释里的除外）就拒绝，rusqlite 的 `prepare` 会默默忽略第二条语句。可以用 `WITH`、FROM 子查询、JOIN 和窗口函数；不支持递归 CTE。`params` 是按位置绑定的参数数组。
- `max_rows` 默认 1000，最多 5000；结果总字节数上限 4 MiB；执行时间上限 2 秒。超过行数时 `truncated: true` 并返回前 `max_rows` 行；超过字节数或时间返回 `LIMIT_EXCEEDED`，`detail` 写明是哪一项。
- `read` 是这条语句实际读到的表（逻辑表的名字、实体、路径和 `content_rev`），来自 authorizer 的记录。视图被展开后 authorizer 报的是底层 `table_records` 的列，不是逻辑表的列，所以没有给列（§13）。
- `expect_head: { epoch, seq }` 给了就要求与服务端 head 相同，否则返回 `SNAPSHOT_EXPIRED`，`sub_code: HEAD_MISMATCH`，`data` 带当前 `epoch`、`head_seq`。持有副本的 Desktop 标签页发起时带自己已确认的 head，本地还有未确认的修改时直接拒绝（`SNAPSHOT_EXPIRED` / `LOCAL_CHANGES`），离线时报告不可用；不持有副本的会话和 Agent、CLI 不带。

### 7.2 表名和列

- 表名是表实体的路径，写成带引号的标识符：`FROM "/data/销售"`。也可以用 `"~<entity_id>"` 或直接写 entity id；阶段工具里还可以写句柄 `"@T1"`。
- 逻辑表**按需建**：语句第一次提到一个名字时 SQLite 报 `no such table`，服务按路径 / id 解析它（每一步过 ACL），为它建临时视图后重新准备语句；一条语句最多 64 张。不再预先给所有可读表建视图（§12.4）。
- `tables: { 别名: 路径或 entity_id }` 可选。给了以后，SQL 里用别名作表名，并且只有这几张表。
- 系统列：`_id`（record_id）、`_rev`（记录版本）、`_body`（记录正文实体的路径，没有为 NULL）、`_record`（`doc.read` 的记录形式 `{ record_id, rev, values, revs, meta, body_ref? }` 的 JSON 文本，只含活字段：不持有副本的 Desktop 用它渲染和编辑，§11）。
- 每个活字段两列：`"<字段名>"` 是显示列，`"<字段名>#raw"` 是原生列，取值见 §4.3（显示列里公式值、类型不对的值为 NULL，和求值器把它们当空一致）。字段名在一张表里已经保证唯一。字段名与系统列相同时，字段列改名为 `"<字段名>#field"`。
- 只包含未删除的记录和未删除的字段。

示例：

```sql
SELECT "客户", SUM("金额") AS total
FROM "/data/销售"
WHERE "状态" = '已完成' AND "日期" >= '2026-10-01'
GROUP BY "客户" ORDER BY total DESC LIMIT 10
```

### 7.3 执行与限制

- 实时 Workspace 每次查询新开一个只读连接（`BEGIN DEFERRED` 后读一次 head，确认和服务看到的 head 相同）；阶段工具用阶段快照自己的只读连接（它的读事务固定了版本）。视图建在连接的 temp 库里，定义为对 `table_records` 的 `json_extract`，限定 `source_id` 和未删除；查询结束删掉。
- authorizer 只在**调用者的语句**准备和执行期间生效：服务自己在同一连接上做的读（解析路径、建逻辑表、整理 `read`）不受它管——阶段快照的读和 SQL 共用一个连接，不这样就会把服务自己的读也拒掉。生效时只放行：`SELECT`；对这些临时视图的读取；经由这些视图（authorizer 回调里的 accessor）对 `table_records` 的读取；`json_each` / `json_tree`；函数（拒绝 `load_extension`、`readfile`、`writefile`、`edit`、`fts3_tokenizer` 等）。其余一律拒绝，包括对 `entities`、`grants`、`commits`、`sqlite_master` 和直接对 `table_records` 的读取，`ATTACH`、`PRAGMA`、写入、建对象和递归 CTE。被拒绝时返回 `INVALID_OPERATION`，`sub_code: SQL_NOT_ALLOWED`；引用不存在或不可读的表统一返回 `NOT_FOUND`。
- 注册的函数：`aiws_dec_cmp(a, b)`（十进制串的精确比较，-1 / 0 / 1）、`aiws_label(key, map)`、`aiws_labels(array, map)`（选项标签、引用路径的映射，映射表写在视图定义里）、`aiws_pick(object, keys)`（`_record` 只留活字段），排序规则 `aiws_dec`（十进制精确排序）。
- progress handler 每 1000 步检查一次时间，超时中断。
- 语法错误返回 `INVALID_OPERATION`，`sub_code: SQL_ERROR`，带 SQLite 的报错文本。超时 `LIMIT_EXCEEDED`，`data.limit: "time"`；结果超过 4 MiB `LIMIT_EXCEEDED`，`data.limit: "bytes"`。
- URL 表不建视图，引用它返回 `INVALID_OPERATION`，`sub_code: NOT_MATERIALIZED`。
- 首批只对 HEAD 执行；阶段工具在阶段快照的连接上用同一套代码。
- 打开了 rusqlite 的 `hooks`（authorizer、progress handler）、`functions`（自定义函数）和 `collation`（排序规则）特性。

### 7.4 表视图转 SQL

表视图 Block 保存的定义不变。core 的 `sqlview::compile_view(表名, 活字段, 视图定义, 投影)` 输出一条等价的 SELECT（不带 LIMIT）、一条计数语句和分组语句；投影是 `display`（`_id` 加显示列，Agent 读的）或 `records`（只有 `_record`，Desktop 渲染用的）。它出现在该 Block 的 `meta.shows.view_sql` 里，是服务端读“这个视图显示哪些行”的唯一途径（阶段工具、许愿格的 `table_view` / `table_query` 输入），也以 WASM 的 `view_sql` 给不持有副本的 Desktop 用。

| 视图定义 | SQL |
| --- | --- |
| `fields` | 选出的显示列，前面加 `_id` |
| `cmp eq / ne / lt / lte / gt / gte` | 每个叶子写成 `(<显示列> IS NOT NULL AND COALESCE(<比较>, 0))`：空值（含公式）对除 `is_empty` 外的运算都是假，`ne` 也是，`not` 包在外面时为真，与求值器一致。select、引用、货币对 `#raw` 比较，decimal 和货币金额用 `aiws_dec_cmp`，其余对显示列比较 |
| `contains` / `starts_with` | `instr(…) > 0` / `substr(…) = …`，区分大小写，与求值器一致（不用 `LIKE`，它对 ASCII 不区分大小写） |
| `in` | 每个值一个等值比较，`OR` 起来 |
| `has_any` / `has_all` | 对 `#raw` 的 `json_each` 做 `EXISTS` / 计数 |
| `is_empty` / `is_not_empty` | `IS NULL`，多选再加空数组 |
| `and` / `or` / `not` | 对应的布尔运算 |
| `sorts` | 每个键写成 `(<显示列> IS NULL), <键> [DESC]`，空值两个方向都排最后；select 的键是按选项顺序的 `CASE`，decimal 是 `#raw COLLATE aiws_dec`，货币先币种后金额；末尾加 `_id` |
| 手工排序 | `ORDER BY` 里的 `CASE "_id" WHEN … END`，没有手工键的排最后再按 `_id` |
| `group` | 另给一条 `GROUP BY` 的计数语句（`view_group_sql`，键升序）；多选按 `LEFT JOIN json_each` 展开计数，空数组和空值计入 `""`；布尔的键是 `true` / `false`。`count_desc` 和 `group_limit` 由调用方在结果上做 |

选项的比较用 `option_id`，SQL 里在旁边用注释带上标签，方便 Agent 读懂和改写。对照测试（`store/tests/sql.rs`）：40 条记录、8 种字段类型、26 个视图定义（各运算符、空值、公式外的类型、多键排序、货币、多选、分组、手工排序），求值器的行、顺序和分组计数与这条 SQL 逐行相同。

## 8. 对象入口

**Workspace 对象** `obj://aiworkspace/<ws_id>` 的 index。查询参数写在对象 URL 上，index 的 `query` 动作执行它，`page` 续页；`query` 动作的输入对象（`query`）里给了同名键时，以输入对象为准；`page_size` 和 `cursor` 也从动作输入取。框架里一个 index 只能绑一个 kRPC 方法，所以加了 internal 方法 `doc.index { op, … }`，按 `op` 转到 `doc.list` / `doc.find` / 文本查询。Workspace 对象另有属性 `info`（摘要）和 `outline`（`doc.outline`，一次取整棵树）。

| 参数 | 适用 | 对应 |
| --- | --- | --- |
| `op` | 全部 | `ls`（默认）→ `doc.list`；`find` → `doc.find`；`grep` → `doc.query { text }` |
| `at`、`type`、`not_type` | 全部 | 同名参数；列表用逗号分隔 |
| `schema` | `ls`、`find` | 同名参数 |
| `depth` | `find`、`grep` | 同名参数 |
| `name` | `find` | 同名参数 |
| `text`、`case` | `grep` | 同名参数 |

列定义（一份，三种动作共用）：`type`、`kind`、`name`、`title`、`path`、`parent`、`on`、`selector`、`snippet`。后三列只有 `grep` 填。每一行的 `object` 是实体对象的 URL；`grep` 命中 body 时 URL 带上 `selector`。

**实体对象** `…/entities/<id>` 的属性：

| 属性 | 对应 |
| --- | --- |
| `meta` | `doc.read { as: "meta" }` |
| `body` | `doc.read { as: "body" }`；URL 查询参数 `cursor`、`max_bytes`、`parts=true`、`selector` |
| `content`（现有） | `doc.read`；URL 带 `mode=fixed&version=` 时把版本传下去（`version` 没有 `mode=fixed`、`mode=fixed` 没有 `version` 都是参数错误） |
| `relations`、`freshness`、`versions`、`reference`（现有） | 不变 |

`meta` 是新的摘要属性，取代 `content` 作为对象卡片读取时的默认摘要。

**固定版本。** `doc.read` 接受 `version`（`{ epoch, seq }` 或 `{ object_id }`），读原生内容时就是 `doc.resolve` 现有的固定版本读取（没有保留点时 `VERSION_NOT_RETAINED`，对象不在时 `REFERENCE_BROKEN`），结果是 `doc.resolve` 的形状（`fixed: true`、`object`）。首批 `as: "meta"`、`as: "body"`、固定版本带 `selector`、`doc.list`、`doc.find`、`doc.query` 带 `version` 都返回 `UNSUPPORTED_VERSION`，不退化为 HEAD。

**路径对象** `…/path/<路径>` 不变，按 §2 的规则解析。

## 9. 写入 helper 与 CLI

helper 在 websdk 的 `src/aiws_write.ts`（包根导出），纯函数返回操作数组；`commitOps` 负责提交。表参数是 `doc.read` 读到的表（`content.fields` 给字段 id、名字、类型、选项），记录 / 块 / 实体参数是对应的读取结果——`table.set_values`、`table.delete_records`、`entity.rename` 的 `expect` 是必填的，所以这几个 helper 必须拿到读取结果。

| 函数 | 生成的操作 | 它替调用者做的事 |
| --- | --- | --- |
| `tableInsert(table, rows, { newRecordId? })` | `table.insert_records` | 字段名换 field_id，选项标签换 option_id，按字段类型规范化值；行里的 `_id` 作记录 id |
| `tableSet(table, recordId, values, read)` | `table.set_values` / `table.unset_values` | 同上；用读到的记录填每个单元格的 `expect` |
| `tableDelete(table, recordIds, reads)` | `table.delete_records` | 填 `expect` |
| `fieldAdd(table, def, { backfill? })`、`optionAdd(table, field, label, { option_id?, index? })` | `table.add_field`、`table.add_option` | 生成 ID 和排序键 |
| `textAppend(doc, markdown)` | `richtext.insert_blocks` | Markdown 转富文本块，接在末尾 |
| `textReplaceBlock(doc, blockId, markdown, read?)`、`textDeleteBlocks(doc, blockIds, reads?)` | `richtext.replace_block`（多出的块 `insert_blocks` 接在后面）、`richtext.delete_blocks` | 转换并填 `expect` |
| `rename(entity, name \| null)`、`move(entity, parentId, afterKey?, beforeKey?)`、`setProps(entity, key, props, read?)` | `entity.rename`、`tree.move`、`entity.set_props` | 填 `expect`，算排序键 |
| `commitOps(client, ws, ops, { message?, epoch?, idempotencyKey?, attempts?, … })` | — | 取 `epoch`，生成幂等键，经 `commitWithParts` 分块提交；传输出错时用 `doc.get_submission` 查结果，没接受就用同一个键重发（3 次）；冲突或拒绝时抛出 `AiwsFailure`（冲突在 `data.conflicts`） |

另有 `cellValue`、`fieldNamed`、`markdownToBlocks`。Markdown 转换是 core `markdown.rs` 的子集：标题、段落、软 / 硬换行、嵌套列表和任务列表、围栏代码、引用、分隔线、GFM 表格、行内代码 / 粗体 / 斜体 / 删除线、链接、`[label](entity:<id>)` 对象链接；不支持缩进代码、引用式链接、HTML、脚注。

客户端方法（`aiworkspace_client.ts`）：`read`、`readMeta`、`readBody`、`listEntities`、`findEntities`、`textQuery`、`sql`、`sourceQuery`（`list` 已经是 `ws.list`，所以叫 `listEntities`）。

CLI 动词（都要 `--workspace`；路径以 `/` 或 `~` 开头，否则当 entity id）：

```text
buckyos aiws ls [路径] | find [字面串] [--at] | grep <字面串> [--at] [--case] | stat <路径> | cat <路径> [--max-bytes] | resolve <路径>
buckyos aiws sql "<SELECT …>" [--table 别名=路径,别名=路径] [--params '[…]'] [--max-rows]
buckyos aiws table add <表> --values '<对象或数组>' | set <表> <record_id> --values '<对象>' | rm <表> <id1,id2>
buckyos aiws table field-add <表> --name <字段名> --type <类型> [--options a,b] | option-add <表> <字段名> <标签>
buckyos aiws text append <文档> --file <md> | --text <md> ; replace <文档> <block_id> --file | --text ; rm <文档> <b1,b2>
buckyos aiws mv <路径> <新父路径> | rename <路径> <新名字> | --auto | set-prop <路径> <config|bindings|on> '<JSON>'
```

`--values` 的键是字段名，值是显示形式（select 写标签）。CLI 的解析器没有可重复的选项和变长位置参数，所以多个表、多个 id 用逗号分隔。`cat` 不给 `--max-bytes` / `--cursor` 时自动续读整段。现有的 `query` 动词删除，`read` 保留（也接受路径）。

## 10. 能力描述

- `schemas/operations.json` 每个操作加 `types`：适用的 `type_id` 数组，`["*"]` 表示所有实体。`core/tests/schemas.rs` 核对它与规划代码一致。
- `meta.ops` 由它按实体类型筛出；调用者能不能做，看 `meta.capabilities` 与该操作的 `capability`。
- helper 覆盖的操作在 `schemas/` 下各有一份参数 Schema，`buckyos aiws <动词> --help` 和 `buckyos command describe` 从命令定义输出，不另写文档。
- 对象入口首批不声明写动作。

## 11. 其他接口

- **URL 表。** `doc.source_query { workspace_id, source_id, filter?, sorts?, fields?, limit, cursor?, consistency?, source_revision? }`，参数和结果与现在 `doc.query` 对 URL 表的行为相同；对本地表调用返回 `INVALID_OPERATION`。`doc.source_capabilities` 不变。
- **扩展控件宿主接口 6。** `aiws.query(sql, params?)` 返回 §7.1 的结果，转到服务端执行；持有副本的标签页带自己已确认的 head（`expect_head`），有未确认的本地修改时拒绝（`SNAPSHOT_EXPIRED` / `LOCAL_CHANGES`），离线时抛出明确的错误。`aiws.input(name)` 的 `fields` / `rows` 不变（不经 SQL，离线可用）。`aiws.outline` 的 `resolve` 按 §2 的路径（或 entity id）解析，`find` 按名字或标题不区分大小写地匹配，结果多 `name`、`path`。宿主接口 5 及以下的 `aiws.query` 调用时直接拒绝。服务端 `HTML_API_LATEST` 和 websdk 的构建检查都认 6。
- **Desktop。** 持有副本的标签页继续用求值器渲染表视图。不持有副本的会话（`OnlineWorkspaceSession.query`，以及副本会话里 URL 表的回退）：读出视图 Block 和表，用 WASM 的 `view_sql`（投影 `records`）编译，`doc.query { sql, tables: { t: <表> } }` 取一页（`LIMIT ? OFFSET ?`，cursor 是偏移量）、计数语句取 `total`、分组语句取 `groups`（`count_desc`、`group_limit` 在客户端做）；`_record` 解析回原来的 `QueryRow`，表格视图、数据控件、声明式控件、字段管理等调用方都不用改。URL 表走 `doc.source_query`。
- **阶段工具。** `ws_find` 用 §6 的文本查询；`ws_read` 对富文本、记录、批注、Wish、定义等读 §4 的 body，对表和表视图 Block 按编译出的视图 SQL 读一页带句柄的行，并把视图的 SQL 一起给出；`ws_query` 只收 `sql`（表名可以写路径、id 或句柄 `"@T1"`），读到的表按执行阶段的规则记入读集；路径都用 §2；仍在阶段快照的连接上执行。许愿格 `table_view` / `table_query` 输入的物化也改用视图 SQL。

## 12. 最没把握的地方

v0 时按“最可能要改”排序列出的八处，实施后的结论：

1. **表的 body 用 CSV、值用显示形式。** 保留。写入有 helper 接标签；SQL 同时给显示列和 `#raw`。
2. **名字联动标题。** 保留联动，但不存 `name_auto`，改成“名字仍是规则给出的那个就跟随”（§2.2）。路径失效的频率要等真实使用看。
3. **个人实体与共享实体重名时“共享优先、个人被遮住”。** 保留；被遮住的在 `ls` 里标出，路径用 `~<id>`。
4. **每次 SQL 查询为所有可读表建视图。** 改了：逻辑表按需建（§7.2），没有 500 张的上限。
5. **decimal 在 SQL 显示列里是 REAL。** 保留；精确比较和排序用 `#raw` 与 `aiws_dec_cmp` / `aiws_dec`。
6. **Block 的 body 内联它显示的富文本。** 改了：不内联，只写路径（§4.2）。读画布看结构，文本查询也不会在文档、Block、画布上重复命中同一段文字。
7. **run token 保留 `doc.query`。** 照草案保留，`doc.list` 替换 `doc.list_children`，`doc.find` 不加；是否收紧留给权限优化 TODO。
8. **预算数值。** 照草案。SQL 的 2 秒在测试里用五张表的笛卡尔积确认会如期中断；10 万行的基线还没在 Rust 里重测（TODO T0-6）。

### 12.1 已经实测的假设

2026-11-12 用 Python 自带的 sqlite3（SQLite 3.46.1）在开发机上验证，脚本没有入库。表的形状与文档库一致：共用的 `table_records`、每条记录一个 JSON，10 万条记录、每条 20 个字段，另有一张不该被读到的表。

| 假设 | 结果 |
| --- | --- |
| 主库只读打开时，能在连接的 temp 库里建视图 | 可以 |
| field_id 含 `.`、`-` 时的 JSON 路径 | 写成 `'$."f.a-1"'` 可用 |
| authorizer 能区分“经由视图读 `table_records`”和直接读 | 能：经由视图时回调带视图名，直接读没有 |
| 直接读 `table_records`、内核表、`sqlite_master`、`sqlite_temp_master`，以及写入、`ATTACH`、`PRAGMA`、建临时表、标量子查询里绕过视图 | 全部被拒绝 |
| 普通 CTE、FROM 子查询、窗口函数 | 在同一个 authorizer 下正常执行 |
| 递归 CTE | 被拒绝（它读的是无库名的临时表），所以首批不支持 |
| progress handler 做时间预算 | 表自连接在 0.3 秒处被中断，之后连接仍可用 |

10 万行上的耗时：`count(*)` 9 毫秒；两个条件的全表筛选 66 毫秒；按显示列分组求和并排序 124 毫秒；全表排序取前几行 73 毫秒。这只是 SQLite 本身的耗时，不含开连接、建视图、鉴权和序列化，服务端的 SQLite 版本和机器也不同，只能说明量级。

## 13. 实施记录（v0 → v1）

2026-11-12 实施时改动的取值（都已改在上面各节里）：

- **`name_auto` 不存储**（§2.2）。存一个布尔要改存储结构、行增量、包和副本；改成按规则判断是否跟随，效果相同：手工改成别的名字就不再跟随，`rename { name: null }` 交还给规则。代价：手工把名字改成恰好等于标题的名字时，它仍算跟随。
- **哪些类型有标题**（§2.2）。表和富文本没有标题键，名字就是标签；Record 跟随 `p:title` / `p:name`，资源引用跟随 `title` / `file_name`。
- **显式名字等于标题的名字时按跟随处理，移动和恢复不再因跟随的名字相撞而失败**（§2.2）。实施后 wish 结果和 Desktop 的调用（把标题同时写成 name）开始撞上同级实体跟随标题的名字；Block 在分组间拖动也会撞上 `text-1` 之类的计数名字。
- **导入不起名，校验后回填**（§2.2）。包的 content root 要逐字节对上，规划器在导入时起名会让旧包无法导入。
- **Block 的 body 不内联它显示的数据**（§4.2、§12.6）。扩展控件声明自己文本模板的做法首批没有做，按 Block 的通用规则。
- **body 的分页**（§4.1）：表头只在第一页，表的 cursor 记记录 id；`ReadCtx` 加了 `scan_records_from`。
- **文本查询**：容器只匹配名字和标题，表按单元格匹配，一段文字只给一个命中；时间预算由服务传入（core 没有时钟）（§6）。
- **SQL**：逻辑表按需建；可以直接写 entity id；加了 `_record` 列和 `aiws_label` / `aiws_labels` / `aiws_pick` / `aiws_dec`；`read` 只给表不给列；authorizer 只在调用者的语句期间生效（阶段快照共用连接）；`json_each` / `json_tree` 放行；单语句检查自己做（§7）。实施中踩到的坑：authorizer 回调里拿锁，而整理 `read` 时持锁读同一连接，会在阶段快照上自锁。
- **视图转 SQL**：`compile_view` 同时给计数和分组语句，有两种投影；`count_desc` / `group_limit` 由调用方做（§7.4）。
- **对象入口**：多了 internal 方法 `doc.index`（§8）。
- **协议 1.2、存储 6**：`doc.query` 的旧参数被拒绝，Desktop 按协议版本提示不匹配。
