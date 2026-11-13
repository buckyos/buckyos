# AIWS 核心 API 调整 TODO：统一发现、读取、查询与公共属性

状态：首批（M0–M4）已实施并提交（buckyos `c12d8c9e3`）；2026-11-12 真实 Zone DV 与 T0-6 性能验收已做（§9），T5-2 文档与 Schema 同步、T5-3 向 PRD 交付已做（PRD §11.0“首批交付”），这些在那次提交之后，未提交；T5-1 的 §7 场景未逐条对账。2026-11-12 设计 review 的 owner 裁决已回写（§0.1）；M0 的取值见 [设计 v1](aiws-core-api-design.md)（实施中改过的取值记在它的 §13）。

日期：2026-11-12

本任务是 [AIWS AI Native 内核升级 PRD](aiws-ai-native-kernel-upgrade-prd.md) 的前置任务，范围按 R9 收窄：PRD 中依赖统一发现的部分（§8.1、§8.3 的 Jarvis / xllm 接入）以本单首批为前置；许愿格阶段结构、宏边界和数据源定义的设计与实现不等待本单。本文是实施 TODO，不表示下述新接口已经存在。

## 0. 目标与范围

本次意图：**统一接口负责模糊的读和查询，以及公共属性；精确的读和写由具体实体类型提供更准确的接口。**

从 Agent 看，一个 Workspace 是一个逻辑目录，每个实体都有可寻址路径，尽量提供完整的文本 `body`，并具有统一 `meta`。Agent 可以先按位置、名称、类型或正文找到对象，读取必要上下文，再根据对象声明的类型能力精确读写。目录抽象不要求额外生成一套物理文件。

“模糊”指调用者尚不知道准确 ID、字段或对象类型时的探索：列目录、按词查找、读概况、取得候选。它不表示读取可以丢失语义，也不要求模型参与每次查询。模糊匹配不能直接决定写入目标；进入修改或任务输入绑定前，必须形成准确的实体引用、选择器和版本条件。

本单交付：

- Workspace、画布、Block、文档、表格等对象共用的寻址与公共属性。
- 统一 read/query 的发现、文本读取、元数据筛选和正文搜索能力。
- 各实体类型的精确读写能力描述，以及已有 API 的接入映射。
- 大表与长文本的有界读取、分页、版本和查询成本约束。
- DID object、kRPC、SDK、CLI、阶段工具和 Agent 手册的一致契约。

后续 PRD 负责许愿格阶段重构、宏执行边界、自定义同步程序等业务升级。本单提供这些能力被发现、描述和访问的基础，不提前实现其全部生命周期。

### 0.1 Review 裁决（2026-11-12）

| # | owner 裁决 | 落点 |
| --- | --- | --- |
| R1 | 每个 Workspace 是一个 DID object，只有一个 index。不拆子对象，不改对象提供者框架；不同的查询由对象 URL 的查询参数定义动作和参数 | §3.1 |
| R2 | 不传版本就是 HEAD；指定固定版本时读到该版本或明确失败，不退化为 HEAD | §3.3、T2-1 |
| R3 | `doc.query` 直接说明只有两种：对数据源的文本查询；对表的 SQL 查询，SQL 原样传入 | §3.1、§3.4 |
| R4 | 本单沿用现有 ACL；调用者矩阵、白名单、泄露检查等权限梳理拆到 [AIWS 权限优化 TODO](aiws-permission-optimization-todo.md) | §3.2、§5 |
| R5 | 路径统一成一套，解析和列举都过 ACL；提供按实体类型保留 / 排除的过滤器 | §2.1 |
| R6 | `body` 是每类实体在纯文本环境下的呈现（以后的读屏支持用同一份）；搜索在 `body` 上做 | §2.3、§3.2 |
| R7 | URL 表没有本地行：`body` 只给定义并标明未物化，搜索只覆盖名称和定义，行读取走现有远端查询 | §2.3、§3.4 |
| R8 | 精确写入就是按对象类型构造 Commit 操作，经 `doc.commit` 提交；能力目录从 `schemas/operations.json` 派生；提供 helper | §4 |
| R9 | 从底层往上做，拆成首批和后续；PRD 不整体等待本单 | §6 |
| R10 | 复用已有的文本投影和读取方法；`body` 投影与字面匹配放 core，索引放 store | §2.3、§6 |

裁决取代了初稿的三处表述：“不直接开放文档库原始 SQL”和“不做任意跨实体 SQL/Join”被 R3 取代；“不再用同一个 query 名称暗示不同集合”被 R1 取代（同一个 index，用参数区分）。

第二轮裁决（2026-11-12，针对第一轮回写后列出的确认项）：

| # | owner 裁决 | 落点 |
| --- | --- | --- |
| R11 | `doc.query` 不保留 `view_id` 加 filter / sorts 的读法；服务接口只有文本和 SQL 两种，接口更纯粹 | §3.1、§3.5 |
| R12 | 底层本身就是 SQLite：SQL 加上限制后直接执行，不做翻译层 | §3.4 |
| R13 | 写入 helper 按建议做，覆盖高频操作 | §4 |
| R14 | 路径段取实体的 `name`。希望每个实体都有一个有语义的 name，同一父节点下不允许重名 | §2.1 |
| R15 | 对象入口写动作的审批门晚定；首批不声明写动作 | §4、§6 |

第三轮裁决（2026-11-12，针对 R11 和 R14 带出的确认项）：

| # | owner 裁决 | 落点 |
| --- | --- | --- |
| R16 | core 的求值器暂时保留；SQL 只在服务端执行，可以要求发起方的本地版本与远端相同 | §3.4、§3.5 |
| R17 | 接口设计优先，没有正式发版，不背兼容负担：扩展控件的 `aiws.query` 也改成 SQL | §3.5 |
| R18 | URL 表的远端读行迁到单独的方法 | §3.5 |
| R19 | `name` 必填。Block 用自增的名字，或者跟着某个属性联动；这是强化语义的代价，为 Agent 长期节省 token 是值得的 | §2.1 |

owner 确认项到此全部裁决完；剩下的是 M0 的细节（§8）。

## 1. 当前实现与缺口

以下现状依据源码静态核对；没有重新执行运行测试。

| 范围 | 当前实现 | 本单需要补齐 |
| --- | --- | --- |
| 身份与路径 | 实体已有稳定 ID、树边和名称；Surface 是 `buckyos.container` 的 `kind: surface` 实体。共享范围内同一父节点下的 `name` 已由内核保证唯一（创建、改名、移动、恢复时返回 `NAME_CONFLICT`）；但 `name` 可以为空，Block 等多数实体没有名字，个人或参与者范围的实体不参与唯一性检查。`doc.resolve` 按 `name` 逐段解析，遇到后一类重名时取第一个可读的；阶段工具的路径用标题拼，不检查祖先是否可读，两套路径互不相通 | 一套按 `name` 的路径、每个实体都有名字、类型过滤和统一公共属性（R5、R14） |
| 对象入口 | DID object 有根、Workspace、实体、路径四类；根的 query 转 `ws.list`，Workspace 的 query 转 `doc.outline`。每种对象只有一个 index，现有绑定忽略 cursor 和 page_size；实体 URL 的 `mode=fixed&version=` 被 `content` 属性丢弃，读到的是 HEAD | 用 URL 查询参数表达不同查询并分页（R1）；固定版本不退化（R2）；表实体不能只暴露原生 `content` |
| 原生读取 | `doc.read` 按实体及选择器返回结构，可用 `targets[]` 批量读；表实体返回字段、记录数等，不直接返回全部行；`doc.list_children` 已有但不分页 | 跨实体一致的 meta/body 契约，区分原生内容、全文、预览和摘要 |
| 表格查询 | `doc.query` 接受表或表视图，支持过滤、投影、排序、分组及游标；本地表每页扫描整表、收集匹配行并排序，并返回精确 `total` 和全量 `groups`。URL 表（`data_mode: url_query`）没有本地行，查询连同这套 filter 参数转给远端。这套参数的使用方有 Desktop 的表格视图和数据控件、扩展控件的 `aiws.query`、日程命令、CLI 和阶段工具 | 服务接口改为只有文本和 SQL（R3、R11），使用方逐个迁移（§3.5）；URL 表单独规定（R7） |
| 文本查找 | Wish 阶段 `ws_find` 在运行上下文中做有限量的子串查找：每张表至多计 1000 行，总共 30 条命中；匹配对象是原生值的 JSON 串，select 存的是 option_id，搜标签搜不到 | 普通 Agent 可用、在 `body` 上匹配、版本明确、能返回精确命中位置的文本查询（R3、R6） |
| run 读取 | run token 限制方法并使用 run 身份；直接 `doc.read/query` 仍读 Workspace 当前状态、检查 ACL；阶段工具与程序宿主另走运行上下文/快照 | 显式区分实时读取与阶段快照，不能仅凭使用 run token 就宣称读到了声明输入的快照 |
| 存储 | 每个 Workspace 使用文档 SQLite；实体共用 `entities`，业务表共用 `table_fields/table_records`，由 `source_id` 区分；一条记录的全部字段值在一个 `values_json` 里，除主键外没有索引。浏览器离线副本的工作视图在 WASM 内存里，表格查询走 core 的求值器；副本里的 SQLite 只保存待提交内容、草稿和元信息，没有文档数据，不能在上面跑 SQL | 服务端把表实体表示为可用 SQL 查询的逻辑表；不要求每个实体对应一张物理 SQL 表 |
| 能力目录 | kRPC 方法表（名字和级别）、README 方法表、stable 方法的形状夹具、阶段工具、CLI/skills 和对象描述分别有部分说明。`schemas/operations.json` 列了 40 个 Commit 操作及其权限、是否需要 `expect`、undo 类别，没有参数 Schema 和适用类型；`doc.resolve` 已按调用者返回 `capabilities`。Agent 现在手写 `doc.commit`，只有日程（todo / cal）、`place` 和导入有 helper | 从已有清单派生类型能力描述，不新建一份；补写入 helper（R8） |
| Source 写入 | 外部来源实体对普通编辑只读，刷新经候选校验与提交 | 公共属性准确表达可读、可写和同步能力；可写数据源必须显式声明写入语义 |

源码入口：[对象提供者](../src/frame/aiworkspace/server/src/obj_provider.rs)、[读取入口](../src/frame/aiworkspace/store/src/reads.rs)、[原生读取](../src/frame/aiworkspace/core/src/read.rs)、[表格查询](../src/frame/aiworkspace/core/src/filter.rs)、[存储结构](../src/frame/aiworkspace/store/src/schema.rs)、[阶段上下文](../src/frame/aiworkspace/store/src/wish/context.rs)。现有完整 API 清单见 [12-接口治理与验收](../doc/workspace/kernel/12-接口治理与验收.md)。

## 2. 统一对象模型与公共属性

### 2.1 Workspace 是逻辑目录

- Workspace 是根；数据树、画布集合、画布内 Block 子树都可按需遍历。画布属于实体，不另建一套身份和读写模型。
- **一套路径（R5）。** `doc.resolve`、对象入口的路径对象、阶段工具和 CLI 打印的路径使用同一套规则和同一份实现（放 core）。现在按 `name` 解析和按标题拼路径的两套做法合并为一套：路径段取实体的 `name`（R14），标题只用于显示和 `find` 的匹配。Surface 页面 URL、发布名和 Workspace `name` 是各自的命名空间，沿用 [数据模型](../doc/workspace/kernel/02-数据模型与引用.md)“名称与 URL 投影”的规则，不并入实体路径，只规定它们到实体引用的转换。
- **每个实体都有名字（R14、R19）。** `name` 必填，同一父节点下不重名，这样路径本身就是确定的、有语义的。这是为 Agent 付的代价：路径读得懂，就不用为了弄清一个 ID 指什么再读一次。
  - 创建时给了名字就用它；没给时由内核生成。
  - 有标题类属性的实体，名字跟着这个属性联动：属性变了，名字和路径随之变化，ID 和已保存的引用不变。
  - 没有可联动属性的实体（多数 Block）用类型加自增序号，例如 `text-12`。
  - 自动生成的名字由内核在提交时分配，离线副本先按同样的规则预测，以服务端结果为准；并发创建不能因为名字相撞而失败。
  - 同级唯一在共享范围内已由内核保证；个人或参与者范围的实体现在不参与检查，要纳入或另行规定。已有数据做一次回填。

  哪种类型联动哪个属性、相撞时怎么加后缀、手工改名之后是否停止联动、个人范围实体和回填方式由 M0 定（§8）。名字的字符规则沿用现有 `check_name`（1–128 个字符，不含 `/` 和控制字符，可以是中文），进 URL 时按路径段转义。回填完成后不再需要基于 ID 的路径段，只保留给调用者不可读的祖先。
- **路径过 ACL。** 解析每一段、列举每一项都按调用者的 `can_read` 判断。路径里调用者不可读的祖先用 ID 段表示，不显示其名称。同级唯一之后路径不再有歧义；`find` 按名称或标题匹配时仍可能有多个候选，返回候选及判别信息，不自动选第一个。
- **类型过滤器。** 列举、查找和解析接受保留 / 排除实体类型的过滤器（例如只看数据、排除 Block、排除批注）；默认集合 M0 定。
- Desktop 扩展桥里给 HTML 控件用的 `outline`（`resolve` / `find`）是另一份按标题或名字查找的实现，也改用这一套。
- 路径、标题用于发现，`workspace_id + entity_id` 是持续引用和重试的身份。解析结果同时返回规范引用、对象 URL 和当前路径。
- 重命名或移动不改变 ID；旧名称被复用时，已绑定的引用不能指向新对象。
- `children` 表示树的直接子项；`references` 表示引用关系；表记录不是目录子实体。遍历引用须显式请求并限制深度，默认不递归展开绑定数据。

### 2.2 公共 meta

以下是需要统一的语义组，字段拼写和嵌套结构在 M0 确定。优先复用已有字段，避免复制出第二份持久化状态。

| 属性组 | 必须表达的内容 | 修改规则 |
| --- | --- | --- |
| 身份与类型 | Workspace/实体 ID、`type_id`、业务 `schema_id` 与版本、规范引用与对象 URL | 身份由内核维护；类型与 Schema 修改按实体契约 |
| 名称与位置 | 名称、显示标题、父实体、当前路径、节点种类 | 改名、改标题、移动分别映射现有操作；路径是派生值 |
| 版本 | epoch、实体内容与元信息版本、本次实际读取版本；文本投影格式版本 | 服务端生成；派生文本依赖其他实体时，版本标识覆盖这些依赖 |
| 内容表示 | 可用表示、媒体类型、全文/预览/摘要的区别、文本可用性；大小与记录数的精确性或未知状态 | 由类型适配器提供；不能为返回 meta 强制扫描全部正文或大表 |
| 来源与可用性 | 本地内容或 Source、来源说明、当前数据版本、是否有有效数据、最近成功版本及失败状态 | 只读状态；来源配置与同步状态分别由对应能力更新 |
| 数据时间 | 已有的业务时间、成功同步时间、最近尝试时间分别表达 | 不合并为一个含义不明的 updated_at；未实现或未知时明确缺失 |
| 关系 | 子节点与引用的读取入口、Block 的绑定对象、数据源与视图的关系 | 使用现有树边、引用和绑定规则 |
| 能力 | 支持的读取表示、集合查询、精确操作、参数/结果 Schema、权限需求、写入范围与当前可用状态 | 类型支持与当前调用者获准使用分开表达；每次调用仍重新授权 |

公共属性一致不意味着任意 `meta` JSON 可被覆盖。公共可修改字段只提供已定义的操作映射，保留 `expect`、幂等键及 Commit 原子性；ID、版本、权限计算结果、同步状态等不接受普通属性写入。用户私有状态继续使用独立命名空间，不混入共享正文或公共搜索。

后续 PRD 新增的数据源状态可以扩展这些属性，但本单只适配当前真实存在的状态。不能为尚无实现的“未满足定义”返回伪造的就绪状态。

### 2.3 文本 body

`body` 回答一个问题：这个实体在纯文本环境里应该显示成什么（R6）。每种实体类型都要给出自己的回答，扩展控件通过自描述或契约声明，没有声明时用通用回退。Agent 读取、正文搜索和以后的读屏支持用的是同一份文本。

`body` 是可重建的文本投影，实体原生结构仍是数据真相源。投影应尽量覆盖该表示定义的全文，并说明格式、范围和版本；`content` 现有原生含义不能被静默改成摘要或文本。

- 类型化的值按显示形式写出：select 写标签而不是 option_id，引用写目标的标题并保留可还原的引用，日期和数字用确定的格式。因此选项改名、被引用实体改名都会改变 `body`，而记录版本不变；`body` 的版本标识要覆盖这两类依赖。
- 投影实现放在 core，服务、阶段工具和浏览器副本共用（R10）；投影格式变化时提升格式版本，并重建 Desktop 的 WASM。
- 复用已有实现：富文本转 Markdown（`markdown::from_richtext`）、控件的 Markdown 大纲（`format_markdown_outline`、`control.export`）。

| 实体 | 首批文本表示 | 精确读取保留什么 |
| --- | --- | --- |
| RichText | Markdown 全文，保留可识别的正文块和对象引用 | 原生 AST、块 ID、协作版本 |
| Record / Annotation | 确定格式的字段文本或 JSON，正文存在时可单独读取 | 字段类型、属性与锚点 |
| Table（本地行） | 有明确编码规则的 CSV/JSONL 全文，可分段读取；Schema 与预览单独返回 | 字段/记录 ID、类型化值、表视图条件及版本 |
| URL 表（`data_mode: url_query`，R7） | 表定义、字段和来源说明，标明行未物化；不提供行的全文 | 行读取走远端查询和 `source_revision`（filter 读法去掉后由单独的方法承接，§8）；搜索只覆盖名称和定义，SQL 不可用 |
| Surface / Block / Container | 标题、结构、配置中适合读取的内容、子节点或绑定引用 | 几何布局、绑定、原生配置及子实体；默认不内联整张绑定表 |
| 目录 | 当前目录项的文本清单，可分页 | 子实体或资源成员的稳定定位 |
| AssetRef | 原文件已有文本，或已存在的正文提取/转录 | 原文件、媒体信息和提取来源；无文本时明确 unavailable |
| Wish / 控件定义 | 已保存的任务说明、输入输出约定、控件自描述等文本 | 原生任务配置、运行/控件类型的精确能力 |
| 未知类型 | 可用原生内容的保真序列化，标识为 fallback；无可用表示则说明原因 | 不猜测业务语义，不因无法渲染而丢弃内容 |

文本中的引用必须能还原为实体或子项定位。CSV/JSONL 是数据表示，不自动获得把整份文件覆盖回表的权限；省略嵌入资源或未提取的二进制正文时，必须声明覆盖范围。OCR、转录、模型摘要本身不在本单新增。

## 3. 统一的模糊读取与查询

### 3.1 read/query 的职责

沿用 PRD §8.1 的 read/query/xcall 对象入口。按 R1，每个 Workspace 是一个 DID object，只有一个 index；查询的动作和参数写在对象 URL 的查询参数里，不拆子对象，也不改 buckyos-api 的对象提供者。按 R3、R11，`doc.query` 只有文本查询和 SQL 查询两种，现有 `view_id` 加 filter 的读法不保留。`list/stat/search` 可以作为 CLI 便捷动作，但不各自维护另一套协议。

| 入口 | 意图 | 承载 | 结果要求 |
| --- | --- | --- | --- |
| read：meta | 不预先知道类型，先了解对象及其能力 | 实体对象的属性 | 有界的公共属性、内容可用性、类型操作入口 |
| read：body | 像读文本文件一样读取纯文本呈现 | 实体对象的属性；分段参数放在 URL 查询参数里（属性读取没有别的入参） | 正文分段、定位、格式、读取版本、完整性及续读位置 |
| 发现：`ls` | 浏览数据目录、画布列表、Block 子树 | Workspace index | 直接子项，按需字段投影、分页；不自动返回整棵 Workspace |
| 发现：`find` | 按范围、路径/名称、类型、Schema 和公共属性找候选 | Workspace index | 稳定引用及匹配原因；递归范围和深度显式指定 |
| 文本查询：`grep` | 在数据源的 `body` 里找字面串 | `doc.query` 文本模式；Workspace index 的 `grep` 转到它 | 命中片段、实体引用、子项 selector、文本版本与检索覆盖范围 |
| SQL 查询 | 对表做类型化查询，包括读某个已保存表视图显示的行 | `doc.query` SQL 模式（§3.4）；结果是行而不是对象，不进 index | 行、涉及的表及其版本、预算与截断状态 |

Workspace 对象的查询参数草案，作为 M0 的起点，不是定稿：

```text
obj://aiworkspace/<ws_id>?op=<ls|find|grep>&at=<路径或 entity_id>&…
```

| 参数 | 适用动作 | 说明 |
| --- | --- | --- |
| `op` | 全部 | `ls`（默认）、`find`、`grep` |
| `at` | 全部 | 起点：路径或 `entity_id`；缺省为 Workspace 根 |
| `depth` | `find`、`grep` | 递归深度；`ls` 只列直接子项 |
| `type`、`not_type` | 全部 | 保留 / 排除的实体类型（R5） |
| `schema` | `ls`、`find` | 业务 `schema_id` |
| `name` | `find` | 名称或标题里的字面串 |
| `text`、`case` | `grep` | 字面串、是否区分大小写 |
| `version` | 全部 | 固定版本；不传就是 HEAD（R2） |

- 分页用 index 已有的 `page_size` 和 cursor。现有绑定忽略这两个值，要补上。
- 一个 index 只有一份列定义：三种动作共用一份列（类型、名称、路径、父实体、引用），`grep` 的命中位置和片段列在其他动作下为空。
- 参数是对象 URL 的一部分，按 [key url](../doc/key%20url.md) §6 做规范化序列化；带参数的 URL 是同一对象的一个视图，可以写进手册和 CLI。
- `ls` / `find` 落到哪个 kRPC 方法（扩展 `doc.list_children` 还是新增）由 M0 定；`doc.outline` 保留给 Desktop 一次取整棵树。

统一接口应让 Agent 在不知道 Entity ID 时开始探索，但不提供“猜到一个 ID 就修改”的隐式动作。唯一目标解析后也要返回身份和实际范围，供后续精确调用使用。

### 3.2 正文搜索与定位

- 匹配对象是 `body`（R6），不是原生值：调用者搜到的和读到的是同一份文本，命中位置才能在 `body` 里对上。现有 `ws_find` 对原生值 JSON 串的匹配改用这份实现。
- 首批至少支持字面子串搜索、大小写选项、路径/类型/Schema 范围筛选；未支持的正则或语义检索明确拒绝，不能按其他方式悄悄解释。
- 文本匹配覆盖已声明可搜索的正文，不只搜标题、摘要或第一页。表格支持定位到记录和字段，富文本支持定位到正文块；标题和正文命中应可区分。URL 表只覆盖名称和定义（R7）。
- 命中返回规范引用、当前路径、selector、片段、读取版本。selector 用已冻结的 D04 目录（`table_record`、`table_cell`、`richtext_block`、`folder_member`、`entity_prop`），首批不新增种类；文本行号/偏移仅在该文本版本内有效，不能成为修改的持久身份。
- 首批是带扫描预算的逐实体匹配，不建索引；超出预算时返回已覆盖的范围和续查位置，不能以空结果冒充“确定不存在”。索引放到后续（§6），届时它是可重建的派生数据，更新、删除、恢复、改名、移动、选项改名和被引用实体改名都要有维护规则，落后时返回覆盖/新鲜度状态。
- 沿用现有 ACL（R4）：每个命中在查询时按 `can_read` 判断，数量在过滤之后计，片段、目录项及聚合正文同样过滤；以后的索引不保存可见性。不能通过有权读取的画布扩大对其绑定数据的读取权。调用者矩阵和泄露检查的系统梳理在 [权限优化 TODO](aiws-permission-optimization-todo.md)。

### 3.3 完整性、分页和成本

统一读取结果需要表达：目标引用、本次读取版本、表示/Schema、数据或候选、是否完整、下一页位置以及必要诊断。正文不可用、尚无有效数据、有效空内容、部分返回是不同状态；具体字段在 M0 冻结。

- **版本（R2）。** 不传版本就是 HEAD。指定固定版本（`{ epoch, seq }` 保留点或 `{ content_root }`）时，读到该版本或明确失败，不退化为 HEAD。现在实体对象 URL 的 `mode=fixed&version=` 被 `content` 属性丢弃，先修掉。`body` 和发现在固定版本上是否可用，由 M0 按各版本形式逐项写明；不支持的明确拒绝。
- 长文本和表格全文支持按块、行或字节预算续读；定义编码边界，不能在 UTF-8 字符或记录中间静默截断。预览必须标识，续读应能覆盖声明的完整内容。
- 游标绑定目标、查询/表示、版本和读取范围。版本或权限变化时明确失效或按已声明的一致性继续，不能拼接多个版本后称为完整快照。
- 本地表目前的 snapshot 分页是版本变化后拒绝续页，不是长期保留旧数据库快照。需要对正在变化的大表或长文做一致的全量读取时，先建保留点（`doc.checkpoint`），再按 `{ epoch, seq }` 固定版本读，不为续读另造长期快照。
- `fields`、返回行数和输出字节上限不能被当成后端成本上限。补齐扫描量、执行时间和输出预算；超限给结构化结果，不静默缩小数据范围。

### 3.4 表的 SQL 查询

按 R3、R12，对表的类型化查询直接写 SQL。服务端的文档库本身就是 SQLite，SQL 加上限制后原样执行，不做翻译层，也不再设计一套隐藏在 filter 参数后面的下推和等价执行。下面是实现约束和 M0 要定的规则。

- **每张表一层只读视图。** 这是“直接执行”唯一要加的一层。所有表的记录共用一张 `table_records`，字段值在一个 JSON 里；没有这层视图，调用者就得按 field_id 写 `json_extract`，而且 authorizer 只能管到表和列，管不到行，无法把查询限制在调用者可读的那几张表上。视图把一张逻辑表限定到自己的记录，每个活字段一列；它是执行查询的连接里的临时视图，不改存储结构。只为调用者可读、有本地行的 Table 实体建视图。表名用实体路径还是 `name`、列名用字段名还是 ID、转义规则由 M0 定。
- **只读且受限。** 只接受单条 SELECT，在只读连接上执行。用 SQLite authorizer 做白名单：只能读这些逻辑表和允许的函数；禁止写入、ATTACH、PRAGMA、临时对象和内核表（`entities`、`grants`、`commits` 等）。这需要打开 rusqlite 的 `hooks` 特性，现在只开了 `bundled`。
- **权限。** SQL 能引用的表就是调用者 `can_read` 的表，沿用实体级 ACL，不提供行列级 ACL。在可读范围内允许跨表 JOIN。authorizer 记下实际读到的表和列，随结果返回，作为这次读取的范围和各表版本。
- **成本。** 用 progress handler 限制执行步数和时间，另限制输出行数和字节数；超限返回结构化错误，不静默截断。递归 CTE 也靠这份预算打断。逐行取 JSON 值意味着全表扫描，在预算内如实执行；需要时再加表达式索引，属于存储优化，不改契约。
- **语义。** 手写 SQL 的比较、排序和空值按 SQLite 的规则，文档里直接写明；由视图定义编译出的 SQL 例外，它必须与求值器结果一致（§3.5）。列里放显示值还是原生值由 M0 定；建议与 `body` 一致放显示值（select 放标签），另给原生值列。
- **分页和总数。** 调用者自己写 `LIMIT` / `OFFSET` 和 `count(*)`，服务不再替 SQL 查询计算 `total`。返回各表的版本，调用者据此判断两页之间数据是否变化。
- **不可用的情况。** URL 表没有本地行，SQL 不可用，返回明确错误（R7）；首批只对 HEAD 和阶段快照执行，其他固定版本明确拒绝。
- **浏览器副本上没有 SQL（R16）。** 副本的工作视图在 WASM 内存里，不是 SQLite（§1）。SQL 只在服务端和阶段快照上执行；Desktop 渲染表视图继续用 core 的求值器。
- **从 Desktop 发起时要求版本一致（R16）。** 请求带上本地已确认的 `epoch` 和 seq，并且本地没有未提交的修改；与服务端 head 不一致时先同步再执行。这样 SQL 的结果和本地看到的是同一份数据。离线时 SQL 不可用，明确报告。Agent 和 CLI 没有本地状态，不受这条约束。
- 让副本也能跑 SQL 要把它的工作视图换成 SQLite，列入后续（§6 L-6）。

### 3.5 去掉 filter 读法之后

按 R11，`doc.query` 去掉 `view_id`、`source_id`、`filter`、`sorts`、`fields`、`group` 这组参数，现在的使用方要逐个迁移。`doc.query` 是 stable 方法，变更随协议版本并更新形状夹具。

| 使用方 | 现在怎么用 | 迁移 |
| --- | --- | --- |
| Desktop 表格视图 | 把 Block 里保存的 filter / sorts / group / 手工排序交给查询 | 持有副本的标签页在本地求值，不经服务接口，不受影响；不持有副本的标签页和不支持副本的浏览器现在直接调服务的 `doc.query`，要改成 SQL |
| 数据控件、声明式控件、字段管理、课堂 feed、许愿格界面 | 只传 `source_id` 和分页，读一张表的全部行 | 同上；服务端路径改成带 `LIMIT` / `OFFSET` 的 SELECT |
| 扩展控件的 `aiws.query`（宿主接口） | HTML 控件把这组参数原样传给宿主 | 也改成 SQL（R17），不保留旧参数，宿主接口版本相应提升；它转到服务端执行，遵守版本一致的要求，离线不可用。按绑定读字段和行的 `input` 接口不带查询语言，保持不变，离线可用 |
| websdk 日程（`aiws_schedule.ts`）、`buckyos aiws query` | `source_id` 加 filter | 改成 SQL；命令行参数改为 SQL 和文本两种 |
| 阶段工具 `ws_query`、许愿格输入的物化 | 在快照上直接调 core 的求值器，不经 kRPC | `ws_query` 改成在快照连接上执行 SQL；`table_view` / `table_query` 输入（D04 冻结）的定义格式不变，物化改用下面的“视图定义转 SQL” |
| URL 表的远端查询 | 这组参数连同 `source_revision` 转给远端 | 迁到一个只对 URL 表有效的单独方法（R18），方法名 M0 定；`doc.query` 保持两种模式 |

- **视图定义的格式不变，变的是谁来执行。** Block 里保存的 filter / sorts / group / 手工排序仍是表视图的定义。服务端需要“这个视图显示哪些行”时，由 core 把视图定义编译成 SQL 再执行。Agent 读一个表视图 Block 时拿到这条 SQL，可以原样执行，也可以在它上面改写。
- **会有两个执行引擎。** 浏览器副本不能跑 SQL，持有副本的 Desktop 仍用 core 的求值器渲染视图，服务端跑编译出的 SQL。同一个视图定义在两边的结果必须一致，对照测试要覆盖求值器现有的规则：空值在升序和降序下都排最后，select 按选项顺序排序，decimal 和货币按精确十进制比较（SQLite 没有内建，要注册自定义函数或排序规则），以及多选包含、分组计数和手工排序。求值器因此暂时保留在 core 里（R16），直到副本能跑 SQL（§6 L-6）。

## 4. 按实体类型精确读取与写入

统一入口找到对象以后，类型描述告诉 Agent 如何准确操作，而不是要求模型自行猜测 payload 字段。优先把已有能力声明出来；只有确有缺口时才增加类型接口。

| 类型 | 精确读取 | 精确写入或动作 |
| --- | --- | --- |
| Table / Table view | Schema、指定记录/格、SQL 查询（§3.4）；视图的 filter/sorts/fields 范围 | 字段和记录操作、列值修改；稳定 record_id/field_id 与版本条件 |
| RichText / Record / Annotation | AST、正文块、属性、锚点 | 现有富文本操作、属性修改和注释操作；不把文本投影当作无损编辑格式 |
| Surface / Block / Container | 结构、布局、绑定、实例配置、子树 | 创建、移动、布局、绑定和配置修改；关联创建仍是一个 Commit |
| AssetRef / 资源目录 | 原文件、成员与对象版本 | 导入对象、修改目录成员；沿用对象桶和保留规则 |
| Wish / Source | 配置、输入输出、运行状态、候选、来源与可读版本 | 声明已有启动、刷新、取消、应用等动作；后续生命周期升级由 PRD 扩展 |
| 控件定义 / 模板 | 自描述、参数、依赖与可放置/导入能力 | 复用 build、import、place、upgrade、template 等现有组合流程 |

按 R8，精确写入就是按对象类型构造 Commit 操作，经 `doc.commit` 提交。不为每个类型另设写入 RPC，首批也不经对象入口的动作。

- **能力描述从已有清单派生。** `schemas/operations.json` 已列出每个操作所需的权限、是否必须带 `expect` 和 undo 类别；给它补一列“适用类型”，由 `core/tests/schemas.rs` 核对。调用者当前可用的能力用 `doc.resolve` 已返回的 `capabilities`。不另建一份能力清单。
- **参数 Schema 只补 helper 覆盖的操作。** 其余操作继续以 README 和 operations.json 的说明为准，不要求本单给 40 个操作全部写 Schema。
- **公共属性的修改映射到已有操作。** 改名是 `entity.rename`，移动是 `tree.move`，属性是 `entity.set_props` / `entity.set_keys`。
- **高频操作做 helper（R13）。** 放在 websdk，沿用 `aiws_schedule.ts` 的做法：纯函数构造操作，client 负责提交。它替 Agent 做手写时最容易出错的三件事：
  - 用刚读到的版本填 `expect`；
  - 生成幂等键，结果未知时用 `doc.get_submission` 恢复；
  - 把名称换成 ID：字段名、选项标签、Markdown 转富文本块。

  首批覆盖表格（记录、单元格、字段、选项）、富文本（按块增删改）和公共属性。Block 和控件沿用已有的 `place`、导入和控件命令；其余操作仍可直接写 `doc.commit`。`buckyos aiws` 提供对应动词。
- **对象入口首批不声明写动作（R15）。** 2026-10-29 的裁决是等第一个写动作出现时再定审批门（[对象 URL TODO](done/agent-object-url-todo.md) 的选项 A / B / C），R15 确认晚定。启动许愿格运行、触发同步这类动作要先有这个裁决，列入后续（§6）和 [权限优化 TODO](aiws-permission-optimization-todo.md)。

写入仍经 Commit 或已有候选 Apply 路径；保留 accepted/conflict/rejected、提交结果恢复及结构化错误，不能只返回一句自然语言“已完成”。

可写数据源的描述区分三类：本地可编辑数据、由同步执行器发布的本地副本、支持远端回写的来源。当前具备哪一种就声明哪一种；仅有同步发布权不代表普通 Agent 可编辑。远端回写只有在 Adapter 定义授权、冲突、幂等及未知结果恢复后才能开放，本单不承诺实现通用远端回写。

## 5. 承接 AI Native PRD 的 Agent 需求

| PRD 要求 | 本单交付的前置能力 |
| --- | --- |
| §8.1：按需探索数据源树、canvas、Block 子树、控件和表数据 | 统一公共属性、有界 read/query、正文搜索及类型能力目录 |
| §5.2：区分所选视图与整个底层数据源 | 返回准确的 Block/view/source 引用、筛选投影和 selector，不能丢失用户所见范围 |
| §5.3、§5.5：任务输入及运行快照确定 | 探索结果转为可持久化的精确输入绑定，区分 live 发现与本次运行读取版本 |
| §8.2：身份、权限、提交与恢复 | 所有入口沿用同一调用者和授权范围；精确动作声明前置条件、写入范围及结果查询 |
| §8.3：Jarvis 复用当前选择与位置 | 选择上下文携带 Workspace、Surface、Block、实体/视图及选择器；交接不依赖聊天里的名称 |
| §8.4：能力目录不分叉 | 以服务方法及配套 Schema 为依据，对象卡片/Profile、CLI、skills 和阶段工具派生或交叉校验 |
| §7.2、§7.3、§7.8：数据来源、可用版本与时间 | 公共状态能表达现有来源及失败保留语义；为后续“未取得数据”保留明确状态，不能伪装有效空表 |

首个 Agent 验收流程：只知道“销售看板” → `find` 得到候选 → 确定画布和 Block → 读表视图 Block，得到它对应的 SQL → 执行或改写这条 SQL 查询相关记录 → 用 helper 构造 Commit 操作并提交 → 查询接受事实并回读。各步只取得需要的内容。

同一描述契约供普通协作者、Wish 准备阶段和运行执行器使用，但读取上下文必须明确。当前 run token 直接调用 `doc.read/query` 仍是按 run 身份与 ACL 读取实时 Workspace，不自动切换为声明输入或快照；阶段工具与程序宿主使用另一条上下文/快照路径。本单将统一接口接入阶段宿主时，须由宿主限定输入范围和版本，不能转调实时读取后冒充快照。已有 run-token 直读路径是否进一步收紧，列入后续 PRD §6 的宏边界收敛；本单不得扩展其写入或管理权限。run token 的方法白名单按设计草案 §1 处理：`doc.list` 接替 `doc.list_children`，`doc.query` 原本就在白名单里、换成文本和 SQL 后保留，新增的 `doc.find` 不加入；是否再收紧在 [权限优化 TODO](aiws-permission-optimization-todo.md) 里定。

## 6. 实施任务

按 R9 从底层往上做：先做 core 里与入口无关的纯逻辑，再做服务方法，然后是对象入口和 SDK，最后是 Agent 接入。任务分首批和后续：首批交付后 PRD 的 Jarvis / xllm 发现即可接入，后续不阻塞 PRD。2026-11-12 实施了首批，勾选的是已实施并有本地测试的；验收（M5）里真实 Zone 和性能基线未做。

### M0：契约（首批）

T0-2 到 T0-5 的取值见 [设计 v1](aiws-core-api-design.md)，已在实现里站住；T0-6 的数据、基线和通过条件在设计 §12.2，验收用例是 `store/tests/perf_core_api.rs`（release、`--ignored`）和真实 Zone 的 `tests/dv/aiworkspace.perf.spec.ts`。

- [x] **T0-1** 清点现有 `ws.*`、`doc.*`、`proc.*`、对象属性/索引、SDK/CLI 和阶段工具，形成“保留、调整、移除”的映射，记录方法级别（stable / internal / experimental）和协议影响。stable 方法的变更（`doc.resolve` 的重名行为、`doc.query` 的两种模式）同步 `fixtures/protocol` 的形状夹具和协议版本。
- [x] **T0-2** 定路径与命名规则（各类型联动的属性、自增名字的格式、相撞时的后缀、手工改名后是否停止联动、个人或参与者范围的实体、存量回填、转义、不可读祖先、类型过滤器的默认集合）、公共 meta 字段、Workspace index 的查询参数表和列定义、读取结果的版本与完整性字段。
- [x] **T0-3** 逐类型写出纯文本呈现（§2.3 表格的每一行，含扩展控件的声明方式和通用回退）、类型化值的显示规则，以及 `body` 版本覆盖的依赖。
- [x] **T0-4** 定 SQL 查询的规则（§3.4）：表名列名、列值形式、允许的函数、预算数值、返回的范围与版本；定视图定义转 SQL 的规则和 §3.5 各使用方的迁移方式。
- [x] **T0-5** 给 `operations.json` 补适用类型，定 helper 的范围和命令动词。
- [x] **T0-6** 固定性能验收数据和预算：至少十万行、二十列的本地表及长文档；记录现有基线，定 `body` 续读、文本查询和 SQL 查询的通过条件。（11-12：设计 §12.2；验收中改了三处，见 §9。）

### M1：core 层（首批）

- [x] **T1-1** 统一路径：按 `name` 解析、由实体生成路径、类型过滤，全程过 ACL；`doc.resolve` 和阶段工具改用这一份实现。`name` 改为必填：内核在提交时为没给名字的实体生成联动或自增的名字，联动属性变化时改名，副本按同一规则预测；回填已有数据。
- [x] **T1-2** 公共 meta 和各类型的 `body` 投影；区分全文、预览、不可用；读取概况不加载全表。
- [x] **T1-3** `body` 的有界续读和版本标识；聚合内容记录依赖版本，并服从每个被引用对象的权限。
- [x] **T1-4** 在 `body` 上的字面匹配，命中给出 D04 selector 和片段；带扫描预算和续查位置。
- [x] **T1-5** 视图定义转 SQL 的编译（filter / sorts / group / 手工排序），以及它与现有求值器的对照测试。
- [x] **T1-6** 改了 core 之后重建 Desktop 的 WASM，确认离线副本不受影响。

### M2：服务方法（首批）

- [x] **T2-1** 修固定版本读取（R2）：实体对象 URL 带固定版本时读到该版本或明确失败；补对象测试。
- [x] **T2-2** 发现方法（`ls` / `find`）和 `doc.query` 文本模式，带分页、预算和完整性状态；URL 表按 R7 处理。
- [x] **T2-3** `doc.query` SQL 模式：只读连接、逻辑表、authorizer 白名单、步数/时间/输出预算、返回读取范围和版本；验证越权表、内核表、写语句和超预算查询都被拒绝。
- [x] **T2-4** 去掉 `doc.query` 的 filter 读法（R11）：迁移 §3.5 列出的服务端使用方，URL 表的远端查询迁到单独的方法（R18），更新协议版本和形状夹具。

### M3：对象入口与 SDK（首批）

- [x] **T3-1** Workspace index 按查询参数执行 `ls` / `find` / `grep` 并支持分页；实体对象增加 `meta` 和 `body` 属性。
- [x] **T3-2** websdk 客户端方法和写入 helper；`buckyos aiws` 增加发现、文本查询、SQL 查询和写入动词，更新命令说明。同步 `buckyos-api` 的共享类型。
- [x] **T3-3** Desktop 不持有副本时的表格视图和数据控件改走 SQL，发起 SQL 前检查版本一致；扩展控件的 `aiws.query` 改成 SQL 并提升宿主接口版本，扩展桥的 `outline` 改用统一路径；日程命令改用 SQL。同步控件开发文档和示例。

### M4：Agent 接入（首批）

- [x] **T4-1** 阶段 `ws_outline/ws_find/ws_profile/ws_read/ws_query` 改用 core 的同一份路径、`body` 和匹配实现，保留阶段上下文与快照范围；`ws_neighbors` 等标为场景专用。验证阶段读取与普通实时读取分别返回准确的范围和版本。
- [x] **T4-2** 更新 `aiworkspace_run`、`aiworkspace_template_dev` 手册，加入“发现 → 精读 → SQL 查询 → helper 写入 → 回读”的闭环；检查 `ui_core` 等入口手册是否引用了变化的命令。手册指向命令说明和 operations.json，不复制一份定义。

### M5：验收与交付（首批）

- [ ] **T5-1** 完成下节的契约、权限、正确性和性能验收；补齐真实 Zone 的 Agent 发现与精确操作流程，分别记录本地模拟与真实环境证据。（11-12：真实 Zone 的 Agent 流程、DV 和性能验收都已做，见 §9 与设计 §12.2。）
- [x] **T5-2** 同步服务 README、Schema、内核文档的 [数据模型](../doc/workspace/kernel/02-数据模型与引用.md)、[Source](../doc/workspace/kernel/04-数据源发布与同步.md)、[接口治理](../doc/workspace/kernel/12-接口治理与验收.md)。按实际变化调整协议/存储/文本表示版本，不能仅修改文档而宣称新接口稳定。（11-12：见 §9；其间补了参数 Schema 和 `meta.data.write`，各有测试。）
- [x] **T5-3** 向 PRD 交付公共属性与类型能力描述、接口映射、Agent 调用示例、验收证据及已知限制；更新前置任务状态，标明 PRD 哪些后续需求尚未实现。（11-12：PRD §11.0“首批交付”。）

### 后续（不阻塞 PRD）

- [ ] **L-1** 文本查询的索引及其更新 / 重建路径，暴露覆盖和新鲜度。
- [ ] **L-2** 表的表达式索引等 SQL 执行优化。
- [ ] **L-3** 对象入口声明写动作（启动许愿格运行、触发同步等），依赖审批门裁决。
- [ ] **L-4** 其余 Commit 操作的参数 Schema（helper 覆盖的 12 个已有，T5-2），以及由方法表生成或校验对象描述、CLI 和手册。
- [ ] **L-5** 固定版本上的 `body`、发现和 SQL。
- [ ] **L-6** 浏览器副本的工作视图换成能执行 SQL 的存储；之后删除 core 的求值器，只留一个执行引擎。

## 7. 验收场景

| 场景 | 通过条件 |
| --- | --- |
| Agent 只知道名称或一段正文 | 能找到候选并逐步读取；无需下载全部 Workspace；多个候选不会被自动用于写入 |
| 没给名字的新实体、已有的无名实体、同级重名、路径转义、移动后旧名复用 | 每个实体都有名字：新实体得到联动或自增的名字，已有实体完成回填；显式给出的重名被拒绝，自动生成的名字并发相撞时不失败；每个实体均可按路径定位；已保存引用仍指向原 ID |
| 联动属性（如标题）被修改 | 名字和路径随之变化，ID、引用和授权不变；离线副本预测的名字与服务端不同时以服务端为准 |
| 同一路径经 `doc.resolve`、对象入口、阶段工具和 CLI 使用 | 一套规则：任何一处打印的路径都能在其他几处解析到同一实体；类型过滤器按声明保留 / 排除 |
| 画布与 Block 绑定同一大表 | 画布正文可理解结构和引用；默认不重复内联整表；精确查询保留所选视图范围 |
| 名称、正文中部、表格末页包含搜索词 | 搜索覆盖声明的全部内容；命中可定位到实体及正文块/记录/字段，不能只搜预览 |
| 按 select 的选项标签、引用目标的标题搜索 | 能命中，且命中位置与读到的 `body` 一致；选项或被引用实体改名后，`body` 版本随之变化 |
| 实体对象 URL 带固定版本 | 读到该版本或明确失败；不传版本读到 HEAD；任何路径都不把固定版本读成 HEAD |
| URL 表 | `body` 给定义并标明未物化；文本查询的覆盖范围如实报告；SQL 被明确拒绝；远端行读取在 filter 读法去掉后仍可用 |
| 同一个表视图在持有副本的 Desktop、不持有副本的 Desktop 和 Agent 的 SQL 下读取 | 行集合和顺序一致；筛选、排序、分组计数、手工排序都覆盖 |
| 扩展控件的 `aiws.query`、日程命令、`buckyos aiws query` 迁移之后 | 都只接受 SQL 或文本；示例和手册同步，没有残留的 filter 读法调用；扩展控件按绑定读行在离线时仍可用 |
| Desktop 有未提交的本地修改、落后于服务端或离线时发起 SQL | 先同步再执行；离线时明确报告不可用；不返回与本地所见不一致的结果 |
| 长文本、多字节字符、完整表格导出 | 续读无隐式丢字/丢记录；完整性和版本明确，不能拼接变化前后的内容冒充快照 |
| 未提取的二进制、未知实体类型 | 明确文本不可用或 fallback 范围；保留原始读取入口，不编造全文 |
| 无有效数据、有效空表、刷新失败但有旧版本 | 状态和可读版本可区分；未实现的未来状态不被伪造为成功 |
| 不可读实体或被撤权的绑定数据 | 目录、搜索片段、数量、聚合正文、缓存和续读均遵守可见性 |
| 公共属性被修改 | 可写字段通过明确操作生效；只读/派生属性不能经通用 meta/body 覆盖 |
| 类型专用读写与统一入口混合使用 | ID、类型化值、视图范围、权限、版本和错误一致；提交未知时可恢复接受事实 |
| 数据源写入能力不同 | 普通可编辑、本地同步发布、远端回写明确区分；未声明能力被拒绝 |
| 阶段宿主与 run token 调用统一发现和动作 | 阶段路径不能越出宿主限定输入/快照；run-token 直读按其声明的实时/ACL 语义返回，不伪称快照，也不新增任意写入或管理权 |
| 文本查询超出扫描预算 | 返回已覆盖的范围和续查位置；无法保证完整时不返回误导性的完整空结果 |
| SQL 查询引用不可读的表、内核表，或含写语句、ATTACH、PRAGMA | 全部被拒绝，错误不泄露不可读表是否存在；可读范围内的 JOIN 正常执行，并返回实际读到的表和版本 |
| 大表上的 SQL 查询、`body` 续读和文本查询 | 满足 M0 固定的延迟/步数/输出预算；超预算返回结构化错误，不静默截断 |
| Agent 用 helper 修改记录、富文本块和公共属性 | `expect` 来自刚读到的版本；版本过期得到冲突而不是覆盖；结果未知时能恢复接受事实，重发不重复写入 |
| 同一对象从 kRPC、DID object、CLI、阶段工具访问 | 共同语义一致，阶段工具额外遵守快照范围；能力目录与实际实现不存在虚构或遗漏映射 |

## 8. 本单不做与实施前需定下的细节

本单不做：物理文件系统挂载、每实体一张物理 SQL 表、开放内核物理表或可写 SQL、模型语义搜索、OCR/转录流水线、通用文本覆盖写入、完整远端回写连接器、权限模型的变更（见 [权限优化 TODO](aiws-permission-optimization-todo.md)）。许愿格准备/运行重构、宏语言和 AI 节点、同步程序的调度/隔离/凭据/修复、自动运行和自动应用继续由后续 PRD 处理。

需要 owner 确认的事项已由三轮裁决（R1–R19）全部定下。下面的 M0 事项是实现细节，[设计草案 v0](aiws-core-api-design.md) 已各给了一个取值（未经验证，最没把握的几项列在草案 §12）；实现中修订时改草案，本文只跟进任务状态。

M0 需要给出具体决定的事项：

1. `ls` / `find` 的 kRPC 方法名、Workspace index 的查询参数和列定义、实体对象的属性映射及现有方法调整清单。
2. 路径与命名规则：各类型联动哪个属性，自增名字的格式，相撞时的后缀，手工改名之后是否停止联动（建议停止），个人或参与者范围的实体是否纳入同级唯一，存量回填；各类型的纯文本呈现；公共属性的修改映射。
3. 字面搜索与大小写的准确规则；是否纳入受预算约束的正则，未纳入时如何声明能力。
4. SQL 的表名列名、列值形式、允许的函数和预算数值；视图定义转 SQL 的规则；版本一致检查的字段；URL 表远端读行的方法名；扩展控件宿主接口的新版本号。
5. 实际性能预算。

当前开发阶段允许 breaking change。接口收敛时同步相关客户端和文档，不因本单另建未经要求的兼容层；涉及现有持久数据时明确是否需要转换，禁止静默误读。

本次创建 TODO 只改变规划文档，不改变现有协议、权限或 Agent 手册的可执行用法。未来实施按 M4/M5 联动手册与测试；现有可参考入口包括 [对象测试](../src/frame/aiworkspace/server/tests/objects.rs)、[方法契约测试](../src/frame/aiworkspace/server/tests/contract.rs)、[查询语义测试](../src/frame/aiworkspace/core/src/filter.rs)、[Wish 测试](../src/frame/aiworkspace/server/tests/wish.rs)，以及 [Agent 运行手册](../src/apps/jarvis_agent/agent/cmd_skills/aiworkspace_run/SKILL.md) 和 [构建工作区手册](../src/apps/jarvis_agent/agent/cmd_skills/aiworkspace_template_dev/SKILL.md)。

## 9. 实施回写

- **契约决定与映射**（2026-11-12）：取值在 [设计 v1](aiws-core-api-design.md)；与 v0 的差异和原因在它的 §13（名字跟随不存储、表和富文本没有标题、显式给的标题名按跟随处理、移动和恢复让名、导入后回填、Block 的 body 不内联、逻辑表按需建、`_record` 列、authorizer 只管调用者的语句、对象 index 走 internal `doc.index`）。
- **实现**（未提交）：
  - core：`naming.rs`、`path.rs`、`body.rs`、`search.rs`、`meta.rs`、`sqlview.rs`；`plan.rs` 的起名与联动、`OPERATION_TYPES`（`schemas/operations.json` 的 `types` 列）；`ReadCtx::scan_records_from`；协议 1.2；`HTML_API_LATEST` 6。
  - store：`sql.rs`；`reads.rs` 的 `read_as` / `list` / `find` / `query` / `source_query`；存储 6（打开 5 时回填名字，导入校验后回填）；阶段工具 `ws_find` / `ws_read` / `ws_query` 与输入物化改用 core 和视图 SQL。
  - server：`doc.list`、`doc.find`、`doc.source_query`、`doc.index`；对象入口；`doc.list_children` 删除；run token 白名单 `doc.list` 替换 `doc.list_children`。
  - Desktop：无副本时表视图走 WASM `view_sql` + SQL（`api/tableQuery.ts`），副本会话的 `sql()`，扩展宿主接口 6，扩展 `outline` 用统一路径（`api/paths.ts`）；WASM 已重建。
  - websdk：client 新方法、`aiws_write.ts`、日程读行走 SQL、`AIWS_HTML_API_VERSION` 6、CLI 动词；`dist/` 重新构建。
  - 文档：服务 README 新节“Finding, reading and querying”和方法表、协议说明；Desktop README；kernel 文档 02 / 07 / 12；控件例子 photo-vote（digest 已重算）；手册 `aiworkspace_run`、`aiworkspace_template_dev`；许愿格分析提示词。
- **验证**：
  - `cargo test -p aiworkspace-core`、`-p aiworkspace-store`、`-p aiworkspace` 全部通过；新测试 `core/tests/paths.rs`（名字、路径与 ACL、body 分页、文本查询、meta）、`store/tests/sql.rs`（authorizer 拒绝项、时间预算、26 个视图定义的 SQL 与求值器逐行相同、阶段快照上的 SQL）、`server/tests/objects.rs`（index 的 ls / find / grep 与分页、meta / body、固定版本不退化）；`fixtures/protocol/stable-methods.json` 用 `AIWS_REGEN=1` 重录。
  - Desktop `tsc --noEmit -p tsconfig.app.json` 通过。e2e（`playwright.aiworkspace.config.ts`，objects / sources / flow / table-import / features / extensions / wish / control-examples / calendar / session / mobile / offline 共 103 条）：99 过；失败的 4 条里，FP03（新表的标题行现在显示它的名字 `table-1`，不再是“无标题”）和 Q33（用 API 6 当“比宿主新”的版本，6 现在是真的）改了测试后重跑通过；UI-M01 在未改的 HEAD 上就失败（已知基线问题）；OX06/07 失败在“分组后按 Ctrl+Shift+G 解组”：trace 里分组提交成功、解组没有发出任何请求，规划器单测里同样的解组被接受，问题在画布快捷键这一层，本单没有改那里，但没能在 HEAD 上跑基线确认（worktree 需要单独 `pnpm install`，锁文件不入库）。DV 见下。
  - websdk：`pnpm test` 的 aiws 各套件通过（另有与本单无关的 `file_meta` 真实样本失败）；`pnpm run test:cli` 有三个既有失败（期望 manifest_version 3，HEAD 已写 5）。
- **真实 Zone DV**（2026-11-12，提交 `c12d8c9e3` 之后部署到本机 Zone `test.buckyos.io`）：
  - 跑法：`uv run buckyos-build.py -s aiworkspace desktop` → `uv run start.py`；`BUCKYOS_UI_DV_TLS_SPKI=<证书 SPKI> BUCKYOS_UI_DV_GATEWAY_IP=127.0.0.1 BUCKYOS_TEST_ADMIN_PASSWORD=… AIWS_DV_MEDIA_DIR=<媒体目录> pnpm exec playwright test -c playwright.aiworkspace.zone.config.ts`；`playwright.dv.config.ts` 的用例要加 `BUCKYOS_UI_DV_BASE_URL=https://test.buckyos.io`（默认的 `sys.` 子域现在登录被拒：`49ba79f7b` 起 control-panel 的系统 token 只发给 Zone 根域）。
  - 新用例 `tests/dv/aiworkspace.coreapi.spec.ts`（T5-1 的 Agent 流程）：只用 `buckyos aiws` CLI，按“未完成”`find` 到视图 → `ls` → `stat` 取 `view_sql` → `sql` 得到的行与顺序和 Desktop 画布 Block 渲染的相同 → `grep 进行中` 命中 select 单元格 → 聚合 SQL → `table set`（字段名、选项标签）后 SQL 回读、Desktop 跟着变 → `text append` 后 `cat` 回读 → 表的 CSV；对象入口的 meta / body / index grep，未保留的固定版本 404；旧 `doc.query { source_id }` 被拒。第二条：owner 能看到的每个 Workspace（含先于名字存在的日程）的全部实体都有名字。两条都过。
  - DV 发现并修掉的问题：① 无副本 Desktop 按视图读行时，`records` 投影没按视图的 `fields` 裁剪，隐藏字段的值也回来了（旧 `doc.query { view_id }` 不给）——改在 `compile_view` 里用 `aiws_pick` 裁 `_record`，Desktop `tableQuery.ts` 把视图的 `fields` 传给编译器，store `tests/sql.rs` 加了与求值器的对照；② 对象入口把 `VERSION_NOT_RETAINED` / `REFERENCE_BROKEN` 当 500，改为 404，`UNSUPPORTED_VERSION` 改为 400（`server/tests/objects.rs` 断言状态码）。另有两处是新用例自己写错（文档里也嵌了同一视图，要限定画布 Block；样例表有记录正文，CSV 表头带 `_body`，设计 §4 的措辞同步改准）。
  - `playwright.aiworkspace.zone.config.ts` 全量 20 条：coreapi 2 条在修复后重跑通过；其余 18 条 13 过、5 失败，5 条都是 controls 那次（[控件机制 TODO](aiworkspace-control-mechanism-todo.md) §7.4）记下的既有失败：journeys 2 条与 `aiworkspace.spec` 2 条找中文标签而页面是英文（“展开”“作为新的工作区”“已授予…”，实际授权成功），mobile 1 条断言手机只读。integrity（SQL 分页钉住 head）、logic（视图完成 / 重开）、controls 全过。
  - DV 修复之后跑了 Desktop e2e 全量（`playwright.aiworkspace.config.ts` 全部 293 条，不只上面的 103 条）：278 过、6 跳过、9 失败。UI-M01、OX06/07 同上；DR-05、DR-09（Playwright trace 文件 ENOENT）和 H03（笔输入时序）单独重跑通过；稳定失败的 4 条都不经过本单改的读取 / 名字 / SQL：CN17（某缩放下锚点显示 16 个而非 8 个）、i18n（存下的语言 reload 后列表页仍是英文）、OI-03（pptx 图表要 LibreOffice）、UI-P16（访客看不到批注工具，与 11-12 owner 定的“访客默认查看”一致）。没在改动前的提交上跑基线确认。
  - `playwright.dv.config.ts` 的 `aiworkspace.table` 5 条、`aiworkspace.extensions` 4 条全过。extensions 第一条起初稳定失败在“插入实例后马上在 `doc.outline` 里找它”：实例在插入目录关闭后才提交，用例改成轮询；画面上实例和行数都正常。同日早些时候 controls 那次这条不轮询也能过，提交为什么晚了一点没有查。
- **性能验收（T0-6）**（2026-11-12）：
  - `cargo test -p aiworkspace-store --release --test perf_core_api -- --ignored --nocapture`（10 万 × 20 的表、千行表、两个视图、接近上限的长文档；约 35 s，全部通过条件满足）；真实 Zone `tests/dv/aiworkspace.perf.spec.ts`（同一份数据经网关提交，约 1 分钟，通过）。数据、通过条件、基线表和已知限制在设计 §12.2。
  - 验收中发现并修掉的：① 逻辑表之间的 JOIN 没有统计信息时不建自动索引，10 万 × 1000 超 2 s 预算——store 维护 `table_records` 的 `sqlite_stat1`（打开时没有就建、写入累计超过十分之一时重建，`workspace.rs` `refresh_table_stats`），写法要把小表物化（`WITH d AS MATERIALIZED`，0.12 s），直接写的 JOIN 超时错误带提示，README 和 `aiworkspace_run` / `aiworkspace_template_dev` 手册写明；② 内置 SQLite 带 STAT4，有样本时规划器选错 JOIN 顺序（80 s），统计后删掉 STAT4 样本；③ `doc.read` 表为给 `record_count` 解码全部记录（10 万行 550 ms，无副本 Desktop 每次渲染视图都读）——`ReadCtx::record_count`，store 用 SQL 计数，12 ms，Zone 上视图渲染 1438 → 512 ms。
  - 未改的已知限制：查询持有 Workspace，提交最多等一个预算（2 s / 1 s）；直接写的大表 JOIN 仍超预算（L-2 表达式索引才能根治）；常见词的文本查询每次用满预算；持有副本的标签页用求值器，10 万行视图首页约 0.6–1.3 s（Rust 求值器，浏览器更慢，未测）。
- **未做 / 已知限制**：扩展控件自己声明 body 文本模板没做；固定版本上的 `body` / 发现 / SQL（L-5）；文本查询索引（L-1）。
- **文档与 Schema 同步（T5-2）**（2026-11-12）：
  - 内核文档：[02](../doc/workspace/kernel/02-数据模型与引用.md) 新增“实体名字与路径”和“公共属性、纯文本呈现与 SQL 逻辑表”；[04](../doc/workspace/kernel/04-数据源发布与同步.md) 新增“统一读取看到的数据源”；[12](../doc/workspace/kernel/12-接口治理与验收.md) 改正版本（协议 1.2、存储 6，另列 `body` 格式 1、HTML 宿主接口 6）和方法计数（127：stable 43、internal 15、experimental 69），补对象入口的 HTTP 状态、查询类错误、HTML 宿主接口 6 的 `query`、阶段工具 `ws_find` / `ws_read` / `ws_query` 的新含义、命令映射里的写入动词和 helper、五条数据验收场景，新增“查询预算与性能验收”，证据表和验证命令加上本单的用例。
  - 服务 README：存储 6、Source 实体的 `meta.data`、对象入口的 HTTP 状态、参数 Schema。设计文档改为 v1 的状态。
  - Schema：helper 覆盖的 12 个操作在 `commit-request.schema.json` 的 `$defs` 里有参数 Schema，`operations.json` 的 `params` 指向它，`protocol_version` 从过时的 1.0 改为 1.2。`core/tests/schemas.rs` 新测试对照规划器：helper 生成的操作符合 Schema 并被接受，Schema 列出的参数规划器都认，未列出的参数两边都拒绝。写的时候测试抓到一处 Schema 写错（删除块的 `expect` 必须带 `struct_rev`）。
  - 同步时发现并补上：带 Source 的副本和 URL 表在 `meta` 里的 `capabilities` / `ops` 照 ACL 和类型给出，Agent 会以为能编辑（§4 要求区分三类）。`meta.data` 加了 `write: edit | refresh | none`（`core/tests/paths.rs` 新用例，三种状态与规划器的 `SOURCE_READ_ONLY` 一致）；websdk `AiwsMeta.data` 的类型同步。另补 URL 表的用例（`store/tests/sql.rs`：`body` 只有定义、文本查询搜到定义并报告跳过了行、SQL `NOT_MATERIALIZED`）；它抓到文本查询把 URL 表的定义整个跳过、只搜名字和标题，与 R7 不符，`core/src/search.rs` 已改。
  - 版本：`data.write` 是 1.2 里只增不改的结果字段，协议不升，stable 形状夹具用 `AIWS_REGEN=1` 重录（只多这一个键）；存储和 `body` 格式在提交之后没有变化。core 改了 `meta.rs` 和 `search.rs`，WASM 已重建，产物与之前逐字节相同（Desktop 的 WASM 不含这两处代码）。
- **向 PRD 交付（T5-3）**（2026-11-12）：[PRD](aiws-ai-native-kernel-upgrade-prd.md) §11.0“首批交付”：PRD 需要与交付接口的对照、Agent 调用示例（真实 Zone DV 的 CLI 流程）、验收证据、已知限制，以及 PRD 中前置任务没有覆盖的需求（xcall、run token 收紧、程序 API 的 `resolve` / `find` / `outline`、声明清单与按 `field_id` 取值、`aiws.sql`、“尚未取得数据”的状态、远端回写、Jarvis 接入）。PRD 里与实现不符的两处已更正：run token 白名单里 `doc.list_children` 已换成 `doc.list`；§6.7 说 SQL 执行器“记下实际读到的表和列”，实际只记表。
- 前置任务已交接 AI Native PRD（PRD §11.0）。T5-1 未勾选：§7 的场景没有逐条对账，已知没有专门证据的是 run token 直读的范围、Desktop 落后或有未确认修改时的整条流程、公共属性经通用入口被修改。
