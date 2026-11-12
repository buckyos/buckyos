# AI Workspace 控件机制完善 TODO：以官方日历、待办（列表 + 看板）为核心

> 状态：2026-11-11 立项，未实施。同日 owner 补充：**最核心的例子是官方控件日历和 TODO List（支持看板模式）**，[D4 Timer TODO](./opendan-d4-timer-reminder-todo.md) 依赖这些 AIWS 默认能力；电视屏、限时可见、照片投票作为补充例子。同日 owner 裁决了 Q1–Q5（§3.1，原话照录，含之后对 Q3–Q5 的追问和补充），其余按建议（§3.2）；§3.3 的 Q19–Q21 是改写后新出现的三项，owner 同日确认按建议。**所有待裁决项已定，可以开工。**现状核实于 2026-11-11（buckyos `39157cfa2` 加工作区改动），下文行号以此为准。
>
> **原则（owner，Q1）：尽量不扩展 Block，用 label、media、svg 拼装 Block 控件来实现。日历的形态参考 [calendar.online](https://calendar.online/b4e9c0b12f4d16005e3d) 等经典日历控件。这个例子的目标是开发交互优秀的定制控件，而不是能用就行。**
>
> 目标：
>
> 1. **用 label、media、svg 加 Block-control 拼出官方日历和待办**，交互达到经典日历、看板产品的水平。它们读写同一份标准数据（表格），Agent 命令也读写这份数据；D4（日程与提醒）、Goal TODO 的 E2 / E3（“等日历 / 待办接入”）依赖它们。
> 2. **拼装中缺的能力补进宿主**，并以带版本的声明开放给组合控件定义（`composition.json`），让扩展用同样的方式做出同类控件（甘特图、销售漏斗之类）。三个补充例子按同一原则拼装，作为扩展机制的验收。
>
> 依据：内核设计 [05 Block、组合控件与标准库](../doc/workspace/kernel/05-Block控件与标准库.md)（“原子集合应小而稳定；复合业务界面优先由这些单元与 control 组合”）、[06 布局、模式与用户状态](../doc/workspace/kernel/06-布局模式与用户状态.md)、[07 事件、宏与公式](../doc/workspace/kernel/07-事件宏与公式预留.md)、[02 数据模型与引用](../doc/workspace/kernel/02-数据模型与引用.md)、[04 数据源、发布与同步](../doc/workspace/kernel/04-数据源发布与同步.md)、[10 演示与打印](../doc/workspace/kernel/10-演示与打印.md)、[11 权限、资源与保留](../doc/workspace/kernel/11-权限资源与保留.md)、[kernel README 的 GAP 一节](../doc/workspace/kernel/README.md)；[扩展实现思路](<../doc/workspace/BuckyOS AI Workspace 扩展实现思路.md>) §5；服务 [README](../src/frame/aiworkspace/README.md) 的 “Block-controls”“Extension controls”“Extension controls P2 / P3”“Validator classes” 四节；[D4 Timer TODO](./opendan-d4-timer-reminder-todo.md) §3 D、§5、§7 T1 / T4 / T5；[Goal Session 继续推进 TODO](./opendan-goal-session-next-todo.md) §9 第 2 条；[第四期规划](<../doc/workspace/BuckyOS AI Workspace 第四期规划.md>) §7.3.2。产品原则：目标用户是手机用户，体验按手机评估。
>
> 关于参考页：owner 给的 calendar.online 共享页是浏览器端渲染的，抓取只得到页面标题；下文对它的引用来自该站的公开文档（[视图](https://calendar.online/c/documentation/views?lang=en)、[约会](https://calendar.online/c/documentation/appointments?lang=en)、[提醒](https://calendar.online/c/documentation/reminder?lang=en)），2026-11-11 读取。

## 0. 例子

### 0.1 官方日历（核心）

来源：D4 选定“AIWS 保存日程、提醒规则和例外，Calendar 是其上的业务模型和 Canvas 视图”（D4 §0、§3 D）；T4 要求 Canvas 控件显示规则、例外、近期执行与维护状态，通知支持定位、推迟、跳过。

数据与业务需求：

- 日程是一张表（Q3）：标准字段加一个扩展 JSON 字段，重复规则、提醒、地点、颜色等放在扩展字段里。事件说明是记录的富文本正文。
- 提醒意图、例外、发生项按 D4 §5.1 表达；维护状态（覆盖到哪天、最近一次成功）也写在 AIWS 里（D4 §5.4）。
- 用户的私人日程放在约定的私人日程 Workspace 里，数据源 ID 固定（Q5）；团队日程放在团队 Workspace，一个 Workspace 可以有多个日历。跨 Workspace 聚合 v1 不做（D4 §5.3）。
- Agent 改了，画布上实时看到。

交互目标（参考 calendar.online 和主流日历）：

- **视图**：日、周、月、日程列表。年视图和多个日历并排（calendar.online 的 Year、Planner）放在后续。
- **导航**：今天、前后翻页、视图切换、小月历跳转；键盘 `T` 回到今天、`D` / `W` / `M` 切视图、`←` / `→` 翻页。
- **显示**：全天行；跨天事件画成横条；周视图和日视图按时间排列，重叠的事件并排；月格放不下时显示“+N 更多”，点开是当天的完整列表；当前时间线；按日历着色。
- **新建**：点空白处，或在时间格上拖出一段，就地输入标题；需要更多字段时展开完整表单（标题、时间、全天、日历、地点、说明、重复、提醒）。
- **修改**：拖动改期，时间吸附 15 分钟，可以跨天；拖底边改结束时间；复制。拖动时实时预览，松手才提交；可以撤销。
- **重复事件**：拖动或编辑时询问“仅此一次 / 此后全部 / 全部”。
- **触摸**：长按后拖动（calendar.online 是按住 2 秒）；也可以用“移到…”。

拼装方式：

- 日历是一个 Block-control，绑定日程表。
- 看得见的部分都由原子 Block 渲染：日期数字、时间刻度、事件标题用 label；事件色块、今天的圆圈、当前时间线、全天横条用 svg；事件的封面图或附件用 media。
- 事件对应的 Block 是从记录**投影**出来的（G2），不持久化：Agent 加一条记录，日历上就多一个事件，不需要建 Block。
- 弹层（事件详情、快速新建、重复设置、选日期）用宿主的记录表单和标准字段编辑器，不用 Block 拼（G5）。

### 0.2 官方待办：列表 + 看板（核心）

来源：Goal TODO §9 第 2 条，owner 原话：“修改用户的 todolist / 日历 / notebook 这些接入后也是命令而已，现在不支持就没有这些交付方法。”

需求与交互目标：

- 待办是一张表（Q3），标准字段（标题、状态、截止、排序）加扩展 JSON 字段。
- **列表模式**：快速添加（回车后继续添加下一条）、勾选完成、拖动排序（显示插入位置）、隐藏已完成、多选后批量操作；键盘上下移动、空格完成。
- **看板模式**：按状态分列，列的顺序就是状态选项的顺序；卡片拖到另一列就改了状态，拖动时显示占位；列内排序；折叠列；加一列就是加一个状态选项（需要 `structure` 权限，没有时显示原因）。
- 两种模式看同一份数据，随时切换；卡片或条目打开详情（记录正文是富文本）。
- Agent 用命令增删改，画布上实时看到；手机上完整可用。

拼装方式：

- 待办是 Block-control，绑定待办表。卡片由 svg 背景、label 标题、label 截止日期、svg 勾选框拼成；列标题是 label；分列和列内排序是布局算法（G3）。
- 卡片是记录的投影（G2）。这不同于[扩展实现思路](<../doc/workspace/BuckyOS AI Workspace 扩展实现思路.md>) §5 的“Block 看板”：那里的卡片是持久化的画布 Block，适合白板式整理。两种看板都用 Block-control，区别在于子节点来自记录还是来自画布，文档里要分开写（G23）。

### 0.3 D4 和 Goal 需要本单提供的东西

| D4 / Goal 的需要 | 本单提供 | 不在本单 |
| --- | --- | --- |
| 日程和待办的业务数据（D4 T1） | 日程表、待办表的标准字段、扩展 JSON 字段和内置校验（G8）；重复事件的表示（G10）。提醒意图、例外、发生项的字段由 D4 T1 在同一套约定里补 | 提醒的展开、续排、workflow Once |
| 找到用户的日程和待办（D4 §3 D） | 约定的私人日程 Workspace，里面数据源 ID 固定（G9） | 团队日程的发现 |
| Agent 改日程和待办的命令（D4 T5、Goal E2 / E3） | 读写标准数据的 TS 业务操作模块（G11），控件和命令共用 | 命令行本身、手册 |
| 用户在哪里查看和编辑（D4 Q-T3、T4） | 两个控件（G1–G7）和维护状态的显示（G15） | 任务中心的执行诊断页 |
| 通知定位、Goal 交付的 URL（D4 T4、Goal C0 `--url`、F2） | 定位到记录的地址（G12）和单控件全屏页（G14） | 通知上的推迟、跳过按钮（调用 G11 的操作，D4 实现） |

### 0.4 补充例子（按同一原则拼装）

**电视屏。** 原文：在画布中插入一个 video 控件做电视机屏幕。观众在标准读模式下从当前进度开始播放；owner 换视频后，观众跟着切换并从 0 开始；owner 设置视频源和当前进度。

- 控件里放一个 media Block 作屏幕，外框用 svg。控件保存播出锚点 `{ pos_ms, at_ms, playing }`，由宿主按锚点驱动 media 播放：观众位置 = `pos_ms` +（服务端当前时间 − `at_ms`），暂停时就是 `pos_ms`。
- 换视频时，同一个 Commit 里改 media 的资产和锚点（`{0, 现在, 在播}`）。owner 本机拖动只是预览，点“设为播出进度”才提交。
- 标准 media Block 本身不改：它的播放位置仍是运行状态（05），“按锚点播放”是控件对子 media 的驱动（G17）。

**限时可见。** 原文：显示用户打开画布的时间；到时间后，隐藏一些 Block。

- 控件里一个 label 显示剩余时间，一个 slot 放要限时的 Block。每个用户分别计时；隐藏的只是这个用户看到的呈现，不改共享文档。

**照片投票。** 原文：用户可以在一组照片中选 3 张；封面会自动变成得票最高的 3 张。

- 控件绑定一个资源目录，每张照片投影成一个 media Block，叠加 svg 勾选标记和 label 票数；封面区投影得票前 3 张。
- 每人一份选择，最多 3 张，可以改选，只能改自己的。封面前 3 张由读取方计算，不写回。

**一起听（HTML 路线的例子，Q21）。** 上面三个补充例子都按原则用 `composition.json` 拼装，HTML 路线的缺口（§2.5）没有例子覆盖，所以加这一个。它需要实时动画（播放中的波形、逐行滚动的歌词），用 label、media、svg 拼不出来，适合用 HTML。

- 控件绑定一个音频资源目录作为播放列表，目录里可以放同名的 `.lrc` 歌词文件。
- owner 在工具栏或属性面板上：开始一起听、暂停、下一首、选播放模式。这些是清单里声明的动作，不需要先激活 HTML（H2）。
- 正在听的人跟着 owner：owner 换歌或暂停后，已经在运行的播放器立刻跟上（H1）；播放位置按服务端时间计算（H5）。
- 每个人的音量，以及“自己听”时的播放位置，记在本人的私有状态里，下次打开接着听（H4）。
- 手机上能点按激活、拖进度、切歌（H3）；一小时的播客也能直接从中间开始播，不用先下载整个文件（H6）。

### 0.5 状态归属

| 状态 | 日历 | 待办 | 电视屏 | 限时可见 | 照片投票 |
| --- | --- | --- | --- | --- | --- |
| 共享文档（经 Commit） | 事件记录（含重复规则、例外）、维护状态；控件的默认视图 | 待办记录（含状态、排序）；控件的默认模式 | media 的资产、播出锚点、`on_end` | 时限、slot 里的 Block | 照片目录、每人的选择 |
| 用户私有（User State） | 本人切换的视图、日期范围 | 本人切换的模式、筛选、折叠的列 | 音量偏好 | 首次打开时间 | — |
| Session 临时 | 拖动预览、打开的弹层、选中 | 拖动预览、打开的弹层、选中 | 本机暂停、“回到直播” | 剩余时间、是否已隐藏 | 未提交的勾选 |
| 读取方计算（不写回） | 投影出的事件 Block、重复事件在可见范围内的发生项 | 投影出的卡片 | 当前播放位置 | — | 投影出的照片、得票前 3 |

## 1. 现状（已对照代码）

### 1.1 已有，可以直接用

| 能力 | 现状 | 用在 |
| --- | --- | --- |
| 代码注册的 Block-control | `ContainerDefinition`（[ui/controls/registry.ts](../src/frame/desktop/src/app/aiworkspace/ui/controls/registry.ts)）：`configSchema`、子节点规则与 slot、布局算法（`tree` / `fixed` + `solve`）、`Chrome`（在子节点下面画隐式边等）、`Flow`（流式页里的整体表现）、`create`、`toolbar`、`keys`、`Inspector`、给 Agent 的 `agent` 描述。脑图就是这样实现的 | 日历、待办的宿主骨架 |
| 声明式组合控件 | `composition.json`：slot 里是真实 Block；布局 `stack` / `fixed` / `grid`，算法随宿主发布、不从包里加载（K2）；`chrome`；`events` 转发；不运行代码 | 补充例子；G13 开放能力的落点 |
| svg | `shape` Block：draw.io 的基本几何，或自带的 SVG 正文（经过清理），带多行文字（[shapeBlock.tsx](../src/frame/desktop/src/app/aiworkspace/ui/blocks/shapeBlock.tsx)） | 色块、勾选框、时间线、外框 |
| media | `asset` Block：图片显示本身，视频、PDF 等显示海报，可播放的就地播放 | 封面图、照片、屏幕 |
| 业务 schema 标识 | 实体有 `schema_id`（`<name>@<version>`，[model.rs:113](../src/frame/aiworkspace/core/src/model.rs)），`entity.create` 可以带上（[plan.rs:1497](../src/frame/aiworkspace/core/src/plan.rs)） | 日程表、待办表打上标准标识 |
| 业务校验分层 | 内置类型的 schema 类校验只在准入时运行，重放不运行（服务 README “Validator classes”）；只覆盖内置类型 | 开始 ≤ 结束等约束的落点 |
| 实体 ID 由客户端给 | 小写字母或数字开头、不超过 64 字节（[id.rs:10](../src/frame/aiworkspace/core/src/id.rs) `is_valid_id`） | 固定数据源 ID（`calendar`、`todo`） |
| 指定 ID 建 Workspace | `ws.create` 可以带 `workspace_id`，但格式必须是 `ws_` + 年月 + 4 位（[service.rs:269](../src/frame/aiworkspace/store/src/service.rs)） | 私人日程 Workspace 不能直接用语义 ID，见 G9 |
| 查询引擎 | `doc.query` 支持筛选、排序、分组计数、游标分页和手工排序（[filter.rs:399-436](../src/frame/aiworkspace/core/src/filter.rs)） | 投影的数据来源 |
| 多格同次提交 | `table.set_values` 一次写多格，每格可带版本条件 | 拖动改期、拖卡片改状态 |
| 记录正文 | 记录可以有富文本正文（`check_body`） | 事件说明、待办详情 |
| 变更推送 | 客户端用 `doc.wait_changes` 长轮询（[client.ts:130](../src/frame/desktop/src/app/aiworkspace/api/client.ts)，服务端 [lib.rs:869](../src/frame/aiworkspace/server/src/lib.rs)） | Agent 的修改实时显示、观众跟随 |
| 打开链接 | 目标只有 Surface 和 Block（`LaunchTarget { surfaceId, blockId }`，[WorkspaceShell.tsx:40](../src/frame/desktop/src/app/aiworkspace/ui/shell/WorkspaceShell.tsx)） | 定位到记录要扩展 |
| 用户私有状态 | `ws.get_user_state` / `ws.set_user_state`（stable），按服务修订号条件写 | 个人视图、首次打开时间 |
| 按属性写实例 | `entity.set_props`：`config` / `bindings` / `on` 每个属性单独版本格 | 控件默认视图、播出锚点 |
| 事件与动作 | `events` + 实例 `on` 绑定 `write` / `wish` / `open` / `notify`；以点击者身份执行 | 投票提交 |
| 设计态与运行态 | 本地 `interactive` 开关：编辑模式默认设计态，查看模式默认运行态 | 所有例子 |
| Range 播放 | `asset.stream_url`（[stream.rs:35](../src/frame/aiworkspace/server/src/stream.rs)），放映会话之外可用，`experimental` | 电视屏 |
| 跟随校正 | 放映会话的 `targetPosition` / `correction`（[session/media.tsx:22](../src/frame/desktop/src/app/aiworkspace/session/media.tsx)） | 电视屏 |

### 1.2 已知缺口（已核实）

和核心例子相关：

- **没有 label。** 05 标准库列了 label，但没有独立实现（kernel README GAP）。
- **控件的子节点只能是持久化的树节点。** `ContainerDefinition` 的 `solve` 读的是 outline 子树，组合控件的 slot 里也是真实 Block；没有“从记录投影出子节点”的能力。
- **布局算法只有 `tree`、`fixed`（代码注册）和 `stack`、`fixed`、`grid`（组合控件）。** 没有按字段值决定位置的算法（按日期进月格、按时间进时间格、按状态分列）。
- **控件里的拖动是树操作。** 脑图里拖动是换父节点、改顺序；没有“拖动 → 改记录字段”的映射。组合控件不运行代码，外框也不接管子节点的点击（服务 README “Design and run mode”）。
- **没有 JSON 字段类型。** 现有 `text`、`boolean`、`number`、`decimal`、`date`、`datetime`、`select`、`multi_select`、`object_ref`、`currency`，没有 Q3 要的扩展 JSON 字段。
- **没有日程、待办的标准表**，也没有按 `schema_id` 挂的业务校验。
- **`manual_order` 是视图 Cell 上一个整体替换的 key**（[types.rs:419](../src/frame/aiworkspace/core/src/types.rs)，只有一个版本号），两个人同时拖动不同的卡片会冲突。
- **Workspace 没有唯一名字。** 只有 ID 和标题（`ws.list` 只返回这两项，[service.rs:320](../src/frame/aiworkspace/store/src/service.rs)）；`ws.create` 可以指定 ID，但格式是时间格式；`system.ensure` 只能建放映会话的文件夹（[plan.rs:723](../src/frame/aiworkspace/core/src/plan.rs)）。所以没法约定“每个用户的私人日程 Workspace”。
- **表格和 Record 的字段没有 JSON 类型。** 任意 JSON 只存在于实体按 key 分格的对象里（02 的“按 key 分格的对象”原语，例如 Block 的 `config`，每个属性整体替换、不超过 64 KiB）；表格和 Record 共用的字段类型（[value.rs:14](../src/frame/aiworkspace/core/src/value.rs) `FieldType`）没有 JSON。
- **链接定位不到记录。**
- **没有 App 模式**（kernel README GAP），也没有一个控件占满页面的形态。
- **内置 Block 和控件的契约只写在 TS 代码里**，没有带版本的公开声明（kernel README GAP）。

和补充例子相关：

- **宿主不给 Block 提供服务端时间**；放映会话有自己的时钟（[session/channel.ts:82](../src/frame/desktop/src/app/aiworkspace/session/channel.ts)），普通 Workspace 没有。
- **控件不能驱动子 media 的播放**；普通查看下媒体整文件下载后播放（[ui/media/source.ts:33](../src/frame/desktop/src/app/aiworkspace/ui/media/source.ts)）；资产上限 32 MiB（[aiworkspace_client.rs:187](../src/kernel/buckyos-api/src/aiworkspace_client.rs)），从 Zone 导入也受限（[lib.rs:985](../src/frame/aiworkspace/server/src/lib.rs)）。
- **画布上没有按条件隐藏 Block 的机制**（放映步骤的 `hide` 只在放映中生效）。
- **没有“每人只能写自己那份”的受控写入**：11 规定没有行级 ACL，“受限表单应使用受控提交与读取入口”，普通 Workspace 没有这样的入口。放映会话的 `participant:<principal>` 作用域（[model.rs:53](../src/frame/aiworkspace/core/src/model.rs)）是先例。
- **分组计数不展开多选**：`groups` 把整个字段值当一个键。
- **没有数据变化触发的自动化**（kernel README GAP）。
- **组合控件和 HTML Block 在手机上只读**（服务 README “HTML parent, grid layout, fallbacks”）。投票要在手机上做，组合控件这一半是关键路径（G22）。
- **HTML 路线的缺口**（§2.5 的 H1–H6）：
  - HTML 控件挂载时一次性拿到 `config`，之后收不到实例属性的变化（[HtmlBlockHost.tsx:46-73](../src/frame/desktop/src/app/aiworkspace/ui/extensions/HtmlBlockHost.tsx)）。
  - 扩展清单声明不了动作、工具栏、右键菜单、属性面板入口（kernel README GAP）。
  - 手机上只读。
  - 没有本人私有状态的 API，也没有服务端时间。
  - 绑定的资产只能整块读成字节（[bridge.ts:392](../src/frame/desktop/src/app/aiworkspace/ui/extensions/bridge.ts)），拿不到可以 Range 播放的地址。

## 2. 差距与方向

### 2.1 拼装能力（核心）

| # | 差距 | 方向 |
| --- | --- | --- |
| G1 | **label 原子 Block** | 按 05 实现 label：显示常量或绑定值，不拥有另一份数值真相；带格式（日期、时间、数字、相对时间）、字体颜色、单行截断或换行；在宽度约束内报告固有尺寸。label、media、svg 三者都要能在一个控件里大量出现而不卡（G7） |
| G2 | **数据投影子节点** | 控件声明**模板**：一棵由 label、media、svg 组成的小树，值写成 `{{item.<field>}}` 并可带格式。宿主对每个数据项（记录、目录成员、日期范围里的每一天或每个时段）实例化一份模板，和持久化的子节点一起参与布局和绘制。投影节点**不持久化**（Q19），身份由“控件、模板节点、记录 ID（重复事件再加发生项）”推导，在刷新、重排之间保持稳定。选中、批注、链接、Agent 寻址都落到它对应的记录上（02 的 `table_record` 选择器）；在 label 上就地编辑就是改那条记录的字段（带版本条件的 Commit）。05 补一类子节点：“投影子节点”，与 slot 里的用户内容区分 |
| G3 | **按数据决定位置的布局算法** | 随宿主发布（K2）：`list`（按排序字段）、`columns`（按选项字段分列，列内按排序字段）、`month`（7 列网格，跨天事件画成横条，放不下显示“+N 更多”）、`time_grid`（纵向时间轴，按开始和结束定位，重叠的并排）、`agenda`（按日期分组的列表）。算法只读模板实例、字段值和尺寸测量，只产生位置，不提交数据（05 §布局归属与组合操作） |
| G4 | **手势映射到数据写入** | 控件声明手势和数据写入的对应：拖到另一个格子或列 → 改开始和结束（保持时长）或改状态和排序；拖边 → 改结束；在空白处拖出一段 → 新建记录并预填时间；点击 → 打开详情；在 label 上编辑 → 改字段。拖动时只在本机预览（Session 状态），松手提交一个 Commit，相关字段带版本条件；冲突时回到服务端结果并提示；进入撤销栈。触摸：长按后拖动，同时提供“移到…”菜单（06）。重复事件提交前询问“仅此一次 / 此后全部 / 全部”（G10） |
| G5 | **控件弹层** | 宿主提供锚定在投影节点上的弹层，桌面上是浮层，手机上是底部抽屉：事件详情、快速新建（就地输入标题）、“+N 更多”的当天列表、重复设置、选日期。内容用宿主的记录表单和标准字段编辑器，不用 Block 拼；重复设置和选日期作为标准字段编辑器补进来 |
| G6 | **控件状态分域** | 控件的每个配置项声明状态域（05：“定义必须明确，不能由组件临时决定”）：实例 `config` 存作者设的默认视图；个人切换的视图、日期范围、筛选、折叠的列写本人 User State（`block:<entity_id>/view`）；拖动预览、打开的弹层留在 Session。有写权限的人可以“设为默认”（Q6） |
| G7 | **性能与测量** | 投影节点按可见范围实例化，屏幕外的不挂载；label、svg 走轻量渲染，不占 05 成本声明里的编辑器或 HTML 额度。起测目标：500 条事件的月视图在中档手机上 1 秒内显示，拖动预览每帧更新 |

### 2.2 标准数据

| # | 差距 | 方向 |
| --- | --- | --- |
| G8 | **JSON 字段类型、标准表与内置校验** | 内核加 `json` 字段类型：值是对象，按 02 的值 Schema 规定大小和嵌套深度上限，整体替换（02 不提供任意路径修改，同一格的并发修改按版本条件冲突）。定义日程表和待办表的标准字段与扩展字段约定（Q3），表实体带 `schema_id`。按 `schema_id` 挂内置 schema 类校验（只在准入时运行，与现有分层一致）：结束不早于开始、全天事件用日期、状态取值、扩展字段里已约定键的形状。待办的顺序用记录里的排序字段（`order_key` 字符串），不用视图的 `manual_order`：每条记录单独写，并发拖动不冲突，Agent 也能插到指定位置 |
| G9 | **按唯一名字新建或获得 Workspace；私人日程的固定数据源 ID** | 通用能力（Q5）：Workspace 增加一个**名字**，新建时确定，之后不能改；名字在同一个所有者下唯一。新增“按名字取得，没有就新建”的接口（例如 `ws.ensure { name, title }`），并发调用只建出一个；`ws.list` 返回名字。标题照旧可以改，和名字无关。私人日程就是名字为 `schedule` 的 Workspace（具体名字实施时定），第一次使用时建出，里面的日程表、待办表用固定实体 ID（例如 `calendar`、`todo`），Agent 和命令直接按名字和 ID 访问。Workspace 删除后名字释放，再次取得会新建 |
| G10 | **重复事件** | 采用主流日历的定义方式（Q4）：语义取 iCalendar 的 RRULE，覆盖用户在主流日历里能选到的选项：不重复、每天、每个工作日、每周的周几、每月第几天、每月第几个周几（含最后一个）、每年；自定义间隔；结束于某天或若干次之后。规则写在扩展字段里，例外和“仅此一次”的改动按 iCalendar 的做法表达：被排除的日期，以及引用主事件和原始时间的覆盖记录。展开模块用 TS 自己实现，只接受上述子集，超出的明确报错，不静默忽略。控件按可见范围展开显示（派生显示，不写回）；Agent 续排用同一个模块。需要节假日数据的规则（例如“每月最后一个工作日”按法定工作日算）只显示 Agent 写入的发生项，并标注覆盖到哪天 |
| G11 | **业务操作模块** | 新建、改期、完成、改状态、仅此一次、推迟、跳过，定义成标准数据上的命名操作，用一个 TS 模块实现（放 websdk，按仓库“工具优先 TS + websdk”的原则）。控件、Agent 命令、通知上的按钮都调用它（07：菜单、按钮、Agent 用同一套动作语义） |

### 2.3 接入

| # | 差距 | 方向 |
| --- | --- | --- |
| G12 | **定位到记录** | 链接目标从 `{ surfaceId, blockId }` 扩展到记录，重复事件再加发生项（D4 §5.5：发生项身份与计划时刻分开）。宿主先定位控件，控件翻到那一天或那一列，选中对应的投影节点；没有画布实例时落到 G14 的全屏页。02 的 URL 投影补记录选择器。用于通知、`notify send --url`、Goal C0 的 `--url`、Agent 回复里的链接 |
| G13 | **公开契约与能力开放** | 官方日历、待办用与 `component.json` 相同的形状声明属性、绑定、模板、布局、手势、事件、动作和 `agent` 自述，带版本，随标准库发布。G2–G6 的能力随后以声明形式开放到 `composition.json`（新的 `manifest_version`），扩展不写代码也能做同类控件（Q20） |
| G14 | **手机与单控件全屏页** | 手机上日历默认日程列表、待办默认列表；看板一列一屏、左右滑动；拖动都有“移到…”。提供一个控件占满页面的页面形态（App 模式的最小版本，06），作为通知和 Goal 交付 URL 的落点（Q8） |
| G15 | **维护状态的显示边界** | 覆盖期限、最近一次成功等维护状态由 Agent 写进 AIWS 的记录（D4 §5.4），控件只读 AIWS；workflow 的执行记录留在任务中心，控件只放链接（04：核心业务依赖通过数据实体提供） |

### 2.4 补充例子需要的

| # | 差距 | 例子 | 方向 |
| --- | --- | --- | --- |
| G16 | **可信时间** | 电视屏、限时 | 宿主给控件和 Block 一个校准过的 `serverNow()`。服务端：`doc.wait_changes` 的返回带上 `server_ms`，客户端用往返时间的一半校准（Q10）。05 的宿主输入表补上可信时间；模板里可以用 `{{viewer.remaining}}` 一类的观众上下文 |
| G17 | **控件驱动子 media 播放；媒体** | 电视屏 | 组合控件可以声明“slot 里的 media 按某个配置项给的锚点播放”，由宿主执行，不需要 HTML 激活；观众本机暂停、静音、“回到直播”只影响本机。视频单独设大小上限（Q16）；普通查看改用 `asset.stream_url`，并升为 stable |
| G18 | **控件读写本人私有状态** | 限时 | 与 G6 同一条通道。首次打开时间用服务端时钟写入（Q12） |
| G19 | **slot 对单个观众的可见性** | 限时 | `composition.json` 的 slot 加声明式可见性规则，例如 `visible: { from: "first_open", for_ms: "{{config.limit_ms}}" }`，宿主逐个观众求值，只影响本机呈现，不形成 Commit。设计态下全部显示并标“限时内容”；Static、打印、导出不按时间隐藏，只做标注。06 补：这是呈现，不是成员关系或权限 |
| G20 | **每人一份的受控提交** | 投票 | 表格加“按提交者归属的记录”：服务端填写的提交者字段（唯一，客户端无法伪造），持有 `append` 的人只能建、改、删自己那一条；`multi_select` 加 `max_items`，内核强制（Q14）。02 / 11 的协议变更 |
| G21 | **聚合读取与投影查询结果** | 投票 | `doc.query` 分组时对 `multi_select` / `object_ref` 列表按元素展开计数，可按计数排序并取前 N；G2 的投影可以以查询结果为数据项（封面区投影前 3 张，同票按目录顺序）。匿名投票还需要“只读聚合”权限，v1 不做（Q13） |
| G22 | **手机上组合控件可交互** | 投票 | 手机上组合控件里的子 Block 照常交互，投影节点的点击能触发手势映射（G4）。现在的“只读”是扩展 P3 的保守决定，与产品原则冲突 |

### 2.5 HTML 路线（例子：一起听）

| # | 差距 | 方向 |
| --- | --- | --- |
| H1 | **实例属性变化通知运行中的 HTML** | HTML API 加 `aiws.onConfig(cb)`，同时刷新 `aiws.context.config`。05 生命周期里的“输入变化”写明包括实例属性 |
| H2 | **清单声明动作和属性面板入口** | `component.json` 加 `actions`，与内置 `BlockAction` 对齐（`modes`、`needs`、参数），加 `inspector` 入口。宿主把动作放进近身工具栏和右键菜单，查看模式下也按权限出现。动作不要求 HTML 已激活：宿主执行声明的处理（`on` 里的 `write`，参数可以用 `{{now}}`，即 G16 的服务端时间），或激活后转给 HTML。最小尺寸、缩放方式、打印能力这次不做（kernel README GAP 的其余部分） |
| H3 | **手机上 HTML 可交互** | 手机上能点按激活、`emit`，触摸事件交给 HTML |
| H4 | **本人私有状态** | `aiws.userState.get/set`，命名空间 `block:<entity_id>/`，按服务修订号条件写；与 G6 同一条通道 |
| H5 | **可信时间** | `aiws.serverNow()`，与 G16 同一个时钟 |
| H6 | **绑定媒体的流式地址** | 绑定的音视频资产可以取得 Range 播放地址（走 `asset.stream_url`），不用整块读成字节 |

### 2.6 文档修订（G23）

- **05**：标准库补 label 的契约，并加入日历、待办；Block-control 增加“投影子节点”（与 slot 里的用户内容区分）、按数据定位的布局、手势到数据写入、控件弹层；两种看板（记录投影、画布 Block）分开写；宿主输入加可信时间；标准控件的公开契约。
- **06**：控件配置项的状态分域；slot 对单个观众的可见性；单控件全屏页与 App 模式的关系。
- **02**：`json` 字段类型的值 Schema；Workspace 的唯一名字（新建时确定、不可改）与按名字取得；URL 投影里的记录选择器；标准业务 schema 与内置校验的登记方式；按提交者归属的记录、`max_items`。
- **11**：按提交者归属的记录的写入资格。
- **扩展实现思路 §5**：看板一行拆成两行（记录投影、画布 Block）。

## 3. 裁决

### 3.1 owner 已裁决（2026-11-11）

| # | 问题 | owner 原话 | 落实 |
| --- | --- | --- | --- |
| Q1 | 日历和待办是独立的 Block 类型，还是表格视图的布局 | “原则：尽量不扩展block 用lable\meida\svg 拼装block控件完成实现。 日历的形态可以参考 https://calendar.online/b4e9c0b12f4d16005e3d 等经典日历控件， 这个例子的目标就是开发交互优秀的定制控件，而不是能用就行。” | 不做成表格视图的布局，也不加新的原子 Block（label 是 05 已列、未实现的标准 Block，补上它不算扩展）。日历、待办都是 Block-control，用 label、media、svg 拼装，缺的能力补进宿主（G1–G7），交互目标见 §0.1、§0.2 |
| Q2 | 待办看板是记录视图还是 Block 组合 | “同上” | 看板也是用 label、media、svg 拼装的 Block-control，卡片是记录的投影（G2） |
| Q3 | 标准 schema 由谁定、放在哪 | “calendar 的数据源应该是一个表格，todolist也是。但是item有扩展的json字段” | 日程、待办各是一张表，标准字段加扩展 JSON 字段；内核加 `json` 字段类型（G8）。owner 追问“系统类型是否支持 json”，核实结果：任意 JSON 只在实体按 key 分格的对象里有（例如 Block 的 `config`），表格和 Record 的字段类型没有，所以要加（§1.2） |
| Q4 | 周期事件在日历上怎么显示 | “有常见的重复事件定义方法（主流的，用户理解的重复事件的方法）”；对落实方式：“同意” | 按主流日历的定义方式：RRULE 语义、主流日历界面能选到的选项；展开代码用 TS 自己写，日历和 Agent 共用（G10） |
| Q5 | 用户的日历和待办放在哪，Agent 怎么找到 | “是表格数据源的话，应该约定一个用户的私人日程workspace，里有一个确定的数据源（id确定）”；补充：“可以增加用唯一名字新建/获得ws的功能（新建的时候确定不能改）” | 新增按唯一名字新建或获得 Workspace 的通用能力，名字新建时确定、不能改；私人日程是其中一个名字，日程表、待办表用固定 ID（G9） |

### 3.2 按建议执行

| # | 问题 | 结论 |
| --- | --- | --- |
| Q6 | 视图模式、筛选归谁 | 实例 `config` 存默认，个人切换写 User State，有写权限的人可以“设为默认”（G6） |
| Q7 | 待办的截止日期是否显示在日历上 | 是。日历控件可以绑定多个数据源（日程表、待办表），每个数据源有自己的字段映射；v1 截止日期只读显示，点开进入待办 |
| Q8 | 手机形态 | 日历默认日程列表，待办默认列表；看板一列一屏；拖动都有“移到…”；提供单控件全屏页，作为通知和 Goal URL 的落点（G14） |
| Q9 | 三个补充例子做成内置还是扩展 | 扩展控件，打成一个示例扩展包，作为扩展机制的验收。按 Q1 的原则用 `composition.json` 拼装，不走 HTML。官方日历和待办是内置标准库（G13 给出公开契约）。owner 追问 HTML 差哪几项、能否找例子覆盖：差 H1–H6（§2.5），例子见 Q21 |
| Q10 | 服务端时间怎么给 | `doc.wait_changes` 的返回带 `server_ms`，客户端用往返时间的一半校准；首次渲染前再调一次轻量的 `ws.time`（G16） |
| Q11 | 限时可见是展示效果还是访问控制 | v1 只做展示效果，文档写明不是权限边界；真正的限时访问另立 |
| Q12 | 打开时间按首次还是每次 | 首次打开，用服务端时钟写进本人 User State |
| Q13 | 投票是否匿名 | v1 记名，能读画布就能读明细；匿名投票另立 |
| Q14 | “每人最多 3 张、只能改自己的”放在哪 | 内核（G20） |
| Q15 | “封面”指什么 | 控件自己的封面区及其缩略、静态表现；Workspace 列表没有封面概念，不在本单 |
| Q16 | 视频大小上限 | 按媒体类型单设：视频 2 GiB，其他资产仍是 32 MiB |
| Q17 | 电视屏播完后的默认行为 | 停在末帧，循环作为属性选项 |
| Q18 | 投票的照片组从哪来 | 控件绑定一个资源目录，照片投影成 media Block（G2）；不做“slot 里放图片 Block” |

### 3.3 改写后新出现的三项（owner 2026-11-11：Q19、Q20 “按建议”，Q21 “好的”）

| # | 问题 | 结论 | 理由 |
| --- | --- | --- | --- |
| Q19 | 投影出来的事件、卡片 Block 要不要持久化 | **不持久化**，身份由控件、模板节点和记录 ID 推导；选中、批注、链接都落到记录上 | 持久化的话，Agent 加一条记录还得同步建 Block，两份数据会不一致；05 也要求 Block 不复制一套独立的业务数据 |
| Q20 | 官方控件先用代码注册实现，还是先做声明式能力 | **先用代码注册**（`ContainerDefinition`，与脑图相同）把两个控件做到交互目标；过程中把 G2–G6 做成宿主的通用能力，P2 再以声明形式开放到 `composition.json` | 交互质量是这个例子的目标，先在代码里打磨；能力做成通用的，开放时不需要重写 |
| Q21 | 用哪个例子覆盖 HTML 路线的缺口 | **一起听**（§0.4）：一个 HTML 扩展控件，绑定音频目录，播放中画波形、滚动歌词；owner 用声明的动作控制，大家跟着听。一个例子覆盖 H1–H6 | 实时动画用 label、media、svg 拼不出来，用 HTML 合理，不违背 Q1 的原则；电视屏验证组合控件驱动 media 的路线，两条路线各有一个例子 |

## 4. 实施顺序

| 阶段 | 内容 | 验收 |
| --- | --- | --- |
| P0 待办 | G1 label；G2 投影；G3 的 `list`、`columns`；G4；G5；G6；G7；G8（`json` 字段类型、待办表、排序字段）；G9；G11（待办的操作）。手机形态随做 | §5.1 |
| P1 日历 | G3 的 `month`、`time_grid`、`agenda`；G8（日程表与校验）；G10；G11（日程的操作）；G15。手机形态随做 | §5.2；D4 T4 以此为界面 |
| P2 接入与开放 | G12；G13；G14；与 D4 T5、Goal C0 / E2 的命令对接 | §5.1、§5.2 里 Agent 与通知的部分 |
| P3 补充例子 | G16–G22；H1–H6 | §5.3–§5.6 |
| 贯穿 | G23 文档修订，随各阶段一起提交 | — |

## 5. 验收场景

在真实 Zone 上跑：owner 加两个成员，其中一个用手机。交互质量按 §0.1、§0.2 的交互目标逐条过，不只看功能能不能用。

### 5.1 待办

1. 第一次使用时，私人日程 Workspace 和固定 ID 的待办表自动建好；Agent 按固定 ID 能直接读写。
2. 列表模式：快速添加（回车继续）、勾选完成、拖动排序（显示插入位置）、隐藏已完成、多选批量完成；键盘上下移动、空格完成。手机上同样能做，拖动可以换成“移到…”。
3. 切到看板：同一份数据按状态分列；把卡片拖到“进行中”，拖动时有占位，松手后状态改变；列内排序；折叠列。
4. A 切成看板不影响 B 的列表；有写权限的人“设为默认”后，新打开的人看到看板。
5. 只读成员不能拖、不能加；没有 `structure` 权限的人加列时看到原因。
6. 两人同时拖动不同的卡片：都成功。两人同时拖同一张卡：一个成功，另一个回到服务端结果并看到提示。拖动可以撤销。
7. Agent 用命令加一条待办、改一条状态：两端在一次变更推送内看到。
8. 选中一张卡片、批注它、复制它的链接：都落到那条记录上；刷新后选中和批注仍在。
9. Agent 回复或通知里的链接：打开后定位到那一条；手机上落在单控件全屏页。

### 5.2 日历

1. 日、周、月、日程列表四种视图切换；今天、前后翻页、小月历跳转；键盘快捷键。手机上默认日程列表。
2. 显示：全天行、跨天横条、重叠事件并排、“+N 更多”、当前时间线、按日历着色。
3. 点空白处或拖出一段时间新建，就地输入标题；展开完整表单。
4. 拖动改期（吸附 15 分钟、跨天）、拖底边改结束；拖动时实时预览，松手提交，开始和结束在同一个 Commit 里；可以撤销。直接调 API 写入“结束早于开始”被拒（内置校验）。
5. 手机上长按后拖动改期；也能用“移到…”。
6. 重复事件：在表单里选“每个工作日”“每月最后一个周五”等主流选项；拖动其中一次时询问“仅此一次 / 此后全部 / 全部”，三种结果都正确。说“今天不用了”后写入例外，日历上今天那一次消失。
7. 需要节假日的规则：显示 Agent 写入的发生项，并标注覆盖到哪天。
8. 事件详情显示提醒、例外、维护状态（覆盖到哪天、最近一次成功）；执行记录链接到任务中心。
9. 从通知“定位”进来：定位到那一次发生项。
10. 待办的截止日期显示在日历上，点开进入待办。
11. 私人和团队 Workspace 隔离：团队成员看不到私人日程。
12. 500 条事件的月视图在中档手机上 1 秒内显示，拖动预览不掉帧。

### 5.3 电视屏

1. owner 插入电视屏，从 Zone 选一个大于 32 MiB 的视频，开始播出。
2. 观众 A 在 2 分钟后打开，点一下屏幕出声：从约 2 分钟处开始播放，误差小于 1 秒。
3. owner 暂停播出，A 停在同一位置；继续播出，A 跟上。
4. owner 换视频：A 在一次变更推送内切换，从 0 开始，没有出现“新视频配旧进度”。
5. 把 owner 设备时钟调慢 30 秒后再操作：A 的位置不受影响。
6. A 本机暂停、静音只影响自己；“回到直播”回到当前播出位置。
7. A 只有读权限：看不到 owner 的操作，直接调 API 改属性被拒。
8. 播完停在末帧；Static 和打印显示海报。

### 5.4 限时可见

1. owner 把 2 个 Block 放进限时控件，时限 1 分钟。
2. A 打开后看到剩余时间，1 分钟后这两个 Block 对 A 消失；B 晚 30 秒打开，也晚 30 秒消失。
3. A 刷新页面：仍然隐藏。
4. owner 在编辑模式下始终看到全部，带“限时内容”标记；打印和导出不隐藏，只做标注。

### 5.5 照片投票

1. owner 插入投票控件，绑定一个有 8 张照片的资源目录。
2. A、B 都在手机上各选 3 张并提交；A 改选一次。
3. 三端封面区显示同样的前 3 张，同票按目录顺序。
4. 直接调 API：提交 4 张被拒；改别人的选择被拒；伪造提交者字段被拒。
5. 两人同时提交都成功，封面最终一致；缩略和静态表现显示当前前 3 张。

### 5.6 一起听

1. owner 插入一起听控件，绑定一个音频目录（含一个一小时的播客和带 `.lrc` 歌词的歌曲）。
2. owner 不激活控件，直接在工具栏上点“开始一起听”：一个 Commit 写入播出锚点（H2）。
3. A 在手机上点按激活：从当前位置开始播放，误差小于 1 秒；波形和歌词随播放滚动（H3、H5）。
4. owner 点“下一首”、暂停、继续：A 已经在运行的播放器立刻跟上，不需要重新激活（H1）。
5. 播客从第 40 分钟开始播：不需要先下载整个文件（H6）。
6. A 调低音量、切到“自己听”并听到一半后关掉；再打开时音量和播放位置都还在，B 不受影响（H4）。
7. A 只有读权限：工具栏上看不到 owner 的动作。

## 6. 关联

- [D4 Timer TODO](./opendan-d4-timer-reminder-todo.md)：T1（业务模型）按 G8–G10 的约定补提醒、例外、发生项；T4（界面）就是 §5.2；T5（命令）用 G11 的操作模块；Q-T3、Q-T6 由本单 Q4、Q5 的裁决回答了一部分（重复事件的表示、用户日程的位置），D4 那边要同步改写。
- [Goal Session 继续推进 TODO](./opendan-goal-session-next-todo.md)：D4、E2、E3 “等日历 / 待办接入”，接入点是本单 P0–P2；C0 的 `--url` 与 F2（为 Goal 做一个基于 AIWS 的 app）落在 G12、G14。
- [Jarvis 默认 Goal TODO](./jarvis-default-goal-session-todo.md) §5.2、§5.3：“正式日程存 workflow”的旧表述按 D4 改为 AIWS 业务数据。
- [kernel README 的 GAP](../doc/workspace/kernel/README.md)：label 未实现（G1）、内置契约没有公开声明（G13 只做两个官方控件）、App 模式（G14 只做最小版本）、扩展清单缺动作和属性面板入口（H2 只补这两项）、数据变化自动化（本单不依赖）。

## 7. 实施记录（2026-11-11 至 11-12，已完成、未提交）

按 §4 的顺序实施，未提交。随做随记，最后整理成交付说明。

### 7.1 已完成

| 项 | 落点 | 验证 |
| --- | --- | --- |
| G8 `json` 字段类型 | core `value.rs`、`filter.rs`（只有 `is_empty` / `is_not_empty`）；Desktop `values.ts`、`ValueEditor.tsx`（JSON 文本框）、`fieldOps.ts` 可新建 | core 单测 |
| G8 标准表与内置校验 | 契约 `src/frame/aiworkspace/schemas/standard-tables.md`；core `standard.rs`：**在整个 Commit 末尾**对动过的记录判一次（逐操作判会让“定时 ↔ 全天”这类改动的撤销在中间态被误拒），只在准入时跑；标准字段不能删、不能改类型；RRULE 子集校验 | `core/tests/standard.rs` |
| G9 Workspace 唯一名字、`ws.ensure` | store `service.rs`（服务级索引 + 锁，并发只建一个）、`workspace.rs`（`local_meta.name`）；`ws.create { name? }`、`ws.list` / `ws.get_info` 返回 `name`；删除释放；fork / 新导入不带名字；放映克隆去掉名字 | `store/tests/names.rs`（8 线程并发） |
| G16 服务端时间（P3 提前做） | `doc.wait_changes` 带 `server_ms`；`ws.time` | server 合约夹具重录 |
| 数据控件 kind | `controls/core.json` 加 `todo`、`calendar` 和 `data_controls`；core：可带 `bindings`（`bind` 引用，插入时一起建的表归控件所有）、不收子 Block、必须数值尺寸、`layout { mode: "data" }`；大纲投影 `bindings`（名字 → 实体 ID） | core 单测、e2e |
| G10 重复规则 | websdk `src/aiws_rrule.ts`：解析 / 输出 / 描述（中英）/ 主流日历的“重复”菜单选项 / 按时区展开（DTSTART 恒为第一次，EXDATE 计入 COUNT） | `tests/aiws_rrule.test.ts` 16 条 |
| G11 业务操作 | websdk `src/aiws_schedule.ts`：标准表字段、建表操作、排序键（与 core 同算法）、待办的增 / 完成 / 移动 / 改、日程的发生项展开（覆盖记录、排除、Agent 写入的发生项）、仅此一次 / 此后全部 / 全部的改与删、跳过、推迟；`ScheduleClient`（`ws.ensure("schedule")` + 固定 ID `todo` / `calendar` + 一个放着两个控件的页面） | `tests/aiws_schedule.test.ts` 14 条 |
| G1 label | Desktop `ui/blocks/labelBlock.tsx`：常量（title）或绑定值（Record 属性 / 表格单元格），格式（日期、相对时间、数字…），`LabelArt` 供投影用 | tsc、lint |
| G2 投影 | 共享 `controls/projection.ts`（模板、占位符、身份 `<控件>~<模板节点>~<数据项>`、可见范围才实例化）；Desktop `ui/controls/data/ProjectedView.tsx`（label / shape / asset 轻量绘制，主题色 token） | deno 测试 |
| G3 布局 | 共享 `controls/datalayout.ts`：`list`、`columns`、`month`（泳道、跨周横条、“+N 更多”）、`time_grid`（重叠并排、向右伸展）、`agenda` | deno 测试 7 条 |
| G4 手势 | Desktop `ui/controls/data/gestures.ts`（鼠标 4px 起拖；触摸长按起拖，不动抬起就是菜单；Esc 取消）、`write.ts`（一次 Commit、本地叠加、被拒回滚并提示） | e2e |
| G5 弹层 | `ui/controls/data/ControlPopover.tsx`（屏幕坐标、手机底部抽屉、`ControlMenu`） | e2e |
| G6 状态分域 | `ui/controls/data/viewState.ts`：实例 `config` 是默认，本人切换写 `block:<id>/view`，“设为默认”一个 `entity.set_props` | e2e TD03 |
| 宿主接入 | `ContainerDefinition` 加 `sizing: 'box'`、`Body`；`layout.ts` / `RenderHost` 让数据控件像 Block 一样缩放，编辑模式下选中后 body 才接管指针（之前框体负责移动），查看模式直接可用；`DataFlow` 用于流式页和单控件页 | e2e |
| 待办控件（P0） | `ui/controls/todo/`：列表（快速添加、勾选、拖动排序带插入线、已完成折叠、多选批量、键盘）、看板（按状态分列、拖到别列改状态带占位、列内排序、折叠列、加一列 = 加状态选项，无 `structure` 显示原因）、详情弹层（标题、状态、截止、优先级、标签、正文） | `tests/aiworkspace/todo.spec.ts` 5 条全过 |

| 日历控件（P1） | `ui/controls/calendar/`：日 / 周 / 月 / 日程四视图、今天 / 翻页 / 小月历、T D W M A ← → N 快捷键；全天行、跨天横条、重叠并排、“+N 更多”、当前时间线、按日历着色、待办截止日期（只读）；点空白或拖出时段新建（就地输入标题，“更多选项”展开完整表单：全天、起止、重复、地点、提醒、颜色、日历）；拖动改期（15 分钟、跨天，开始和结束同一个 Commit，Alt 拖动是复制）、拖底边改结束；重复事件先问“仅此一次 / 此后全部 / 全部”；详情弹层显示重复规则、例外、Agent 写入的发生项、维护状态（覆盖到哪天、最近一次成功、错误，链到任务中心）、正文；跳过这次、复制、删除 | `tests/aiworkspace/calendar.spec.ts` 6 条全过 |
| Agent 命令（P2 的一部分） | websdk `cli/modules/aiws_schedule.ts`：`buckyos aiws todo list/add/done/undone/update/move/rm`、`buckyos aiws cal list/add/update/rm/skip/postpone`，都经 `ScheduleClient`（只用 stable 方法） | `tests/aiworkspace/schedule.spec.ts`（同一路径对真实服务：并发首次使用只建一个、增删改、跳过、推迟、控件实时看到） |

| G12 定位到记录 | `links.ts`：地址加 `record`、`occ`（发生项的原始开始时间）、`page=control`；`ui/controls/data/focus.ts`：Shell 请求、控件 body 挂载后接收（待办：展开所在分组 / 列并选中；日历：翻到那一天、选中并打开详情）；“复制链接”“批注”（锚到 `table_record` 选择器）进了待办菜单和日程详情；有批注的条目带标记 | TD06、CA07 |
| G14 单控件页 | `ui/controls/data/ControlPage.tsx`：`page=control`、手机上打开记录链接、控件菜单“单独页面打开”；手机默认：日历日程列表、待办列表 | TD06 |
| G13 公开契约 | 模板移到共享 `controls/schedule.ts`；`controls/contracts.ts` 按 `component.json` 的形状生成 `aiws/contracts/todo.json`、`calendar.json`（标识 `did:bns:buckyos#todo`、`kind: builtin`、版本 1：属性与个人保留的键、绑定与 `schema_id`、模板、布局、手势及其写入、菜单动作、Agent 自述），`control.describe` 返回 `control.contract` | deno 测试（文件是最新生成、模板只用 label / shape / asset、手势的角色出自模板）、`server/tests/control.rs` |
| G13 能力开放（manifest 4） | `composition.json`：`projections`（`records` / `members` / `counts` / `viewer` 数据源，`mark` 标出本人选择，`list` / `grid` 布局，空的投影不占位）、事件 `from { projection, role }` 与默认处理 `on`、`media`、slot `visible`；`component.json` 的 `actions`；内核 `store/src/ext.rs`、websdk `aiws_ext.ts`、JSON Schema 同步；大纲给组合定义带 `def_actions` | `server/tests/ext.rs` `control_examples_and_manifest_4`、websdk `aiws_ext.test.ts` |
| G16 / G17 电视屏 | `ui/media/BroadcastMedia.tsx`：按锚点和服务端时钟跟播（偏差 > 400 ms 跳转、80–400 ms 调速），本机暂停 / 静音 / 回到直播，有修改权的人开始 / 暂停 / 设为播出进度 / 从头 / 换视频（资产与锚点同一 Commit）；空的 media slot 显示“选择视频或音频”（系统选择器，Zone 优先）；视频音频上限 2 GiB（内核 `max_media_bytes`，Desktop `assetLimitOf`） | e2e CE01（设备时钟慢 30 秒仍误差 < 1 秒、暂停跟随、读者看不到播出控制、读者改锚点被拒） |
| G18 / G19 限时可见 | `visibility.ts`：首次打开时间按服务端时钟写进本人 User State，到时只对本人隐藏；编辑模式全部显示并标“限时内容”；观众投影（`viewer`：`remaining_ms`、`expired`）、label 新格式 `countdown` | e2e CE02（倒计时、到时隐藏、重新打开仍隐藏、文档不变、编辑模式可见） |
| G20 / G21 照片投票 | 内核（后台子任务）：`submitter` 字段、`max_items`、`doc.query` 的 `group_order` / `group_limit`、多选按元素计数；Desktop 写入 `toggle`（按选项名或 ID 改本人那条记录）、`addOptions`（按目录文件建选项，选项 ID 由文件名生成）；计数和标记按选项名对上照片 | `core/tests/vote.rs`、e2e CE03（准备选票、三张上限、两人、封面一致、API 改别人 / 伪造提交者 / 四张都被拒） |
| G22 手机 | 运行模式只对访客锁定，手机不再锁 | 未在真机验证 |
| H1–H6 一起听 | HTML API 4：`onConfig`、`serverNow`、`userState`、`mediaUrl`；声明动作在近身工具栏（查看模式也有，不必先激活），`forward` 先启动再转交 | e2e CE04（工具栏开始写锚点且不启动控件、读者没有动作、跟播误差 < 1 秒、下一首立即跟上、音量进本人状态） |
| 示例包 | `doc/workspace/control-examples/`：`tv-screen`、`time-limited`、`photo-vote`（composite）、`listen-together`（HTML），`aiws-manifest.json` manifest 4 | websdk `verifyBundle` 整包核对、服务端逐个 `ext.build` |

### 7.2 偏差与说明

- 数据控件的子节点只有投影，不收 slot 里的真实 Block（G2 写的“和持久化子节点一起参与布局”留给 G13 把能力开放到组合控件时再做：组合控件已有 slot，投影区画在它的外观层上）。
- `table.set_values` / `delete_records` / `add_option` 一律要求 `expect`（内核原有规则），业务操作模块总是带上读到的版本。

- 日历的周 / 日视图把跨天的定时事件画在全天行（主流日历的做法），月视图按开始时间排同一天的事件。
- 待办截止日期在日历上是只读的灰色横条，点开是待办的摘要。
- 日历“当前显示哪天”同时记在会话里：个人视图的服务端值晚到时会覆盖刚写的本地值（用户状态模块原有的竞态：写入已发出、拉取的结果更旧），会话值保证链接定位后不被拉回。
- 组合控件的事件可以带默认处理 `events.<e>.on`（实例没绑自己的处理时用它）：照片投票插入后点照片就能投，不需要再给实例配 `on`。
- 照片文件名不是合法的选项 ID：选项 ID 由文件名生成（小写字母数字加短哈希），选项名就是文件名；`toggle`、计数、本人标记都按选项名对上照片。“准备选票”是投票控件声明的动作（需要 `structure`）。
- 声明动作放在 Canvas 的近身工具栏统一处理（HTML Block 和组合控件都走这里），不放进 Block 自己的 `toolbar`（后者只在编辑模式出现）。
- 投票的子任务把媒体上限命名为 `max_media_bytes`，覆盖音频和视频（一起听要放一小时的播客；音频原本就到 2 GiB），不是只放开视频。
- 桌面端读者打开画布默认仍是编辑模式（原有行为），要切到查看模式才能投票 / 一起听；手机上自由画布默认查看模式。
- Agent 生成的链接形如 `<workspace page>?id=schedule-board&block=schedule-board-calendar&record=<id>&occ=<原始开始>`（私人日程的页面和控件 ID 固定）。

### 7.3 待做与未验证

G23 文档已改（2026-11-12）：kernel 05（宿主可信时间、HTML API 4、声明动作、投影子节点、数据控件、manifest 4 组合定义、label 契约、两个标准控件与公开契约、两种看板）、06（单控件页、控件配置的状态分域、slot 对单个观众的可见性）、02（URL 目标参数、Workspace 唯一名字、`json` 值 Schema、按提交者归属的记录与 `max_items`、分组计数、标准业务 schema 的登记与校验）、11（按提交者归属的记录的写入资格、按类型的资产上限）、kernel README GAP、扩展实现思路 §5 看板拆两行、服务 README。手册已改：`aiworkspace_template_dev`（`json` 类型、标准表、数据控件与 `buckyos aiws todo` / `cal`、manifest 4、新的写入种类）。

没做：H2 的属性面板入口（`inspector`，GAP 里保留）；声明动作只进近身工具栏，不进右键菜单；扩展还做不出记录投影看板（组合控件的投影只有 list / grid）；单控件页只支持两个数据控件。

未验证：手机真机、macOS / Windows、SN Zone；打印 / 静态表现；日历拖动、看板拖动等交互只在本地 e2e 跑过；流式地址在服务端从头读到目标位置（`named_store` 有按偏移读的接口，未接；本机 8.3 MB 文件跳到 5.5 MB 处 167 ms，大文件深处未测）。

基线问题：Playwright 夹具（本地 `api.demo` 和真实 Zone DV 的 `aiworkspace.fixtures.ts`）在 Node 里加载 Desktop 的 `api/client.ts`，websdk 的 Node 入口原来不导出 `commitWithParts`，这些用例在未改动的代码上就加载失败。DV 时已修：websdk `src/node.ts` 加导出（`parseAccessDenial` 本来就经 `sdk_core` 导出，上面原记录有误）。UI-M01 在“桌面上下文仍被认成手机”（`data-phone="true"`）处失败，与本单无关。

### 7.4 本机 Zone DV（2026-11-12）

部署：`uv run buckyos-build.py -s aiworkspace desktop`（同时重建 SDK / CLI），核对 rootfs 其余二进制与 `/opt/buckyos` 相同后 `uv run start.py`。

新增 DV 用例 `src/frame/desktop/tests/dv/aiworkspace.controls.spec.ts`（已加进 `playwright.aiworkspace.zone.config.ts`）：owner 在桌面，临时成员在手机（Pixel 7 上下文），四个示例按仓库里的样子 `ext.build`。跑法：`AIWS_DV_MEDIA_DIR=<媒体目录> BUCKYOS_UI_DV_TLS_SPKI=<Zone 证书 SPKI> BUCKYOS_TEST_ADMIN_PASSWORD=… playwright test -c playwright.aiworkspace.zone.config.ts aiworkspace.controls`。媒体用 ffmpeg 生成：`tv-big.webm`（44 MB，VP8 + Opus，200 秒）、`tv-small.webm`、`podcast-1h.webm`（8.3 MB，Opus，1 小时）、`a song.webm` / `a song.lrc` / `b song.webm`。

结果：5 条全部通过。

| 场景 | 验到的 |
| --- | --- |
| 私人日程（§5.1、§5.2 部分） | devtest 原来没有 `schedule`：CLI 第一次 `todo add` 建出 Workspace、两张表和日程页（测完删除）；控件里快速添加（回车继续）、勾选完成与 CLI 列表一致；CLI 加、改的待办在打开的控件里 10 秒内出现；截止日期显示在日历全天行；CLI 加的事件出现在周视图；直接写“结束早于开始”被内核拒；`record=` 链接选中那条，`page=control` 是单控件页；成员列表里没有、读不到，成员自己的 `schedule` 是另一个 Workspace |
| 电视屏（§5.3 1–7） | 系统选择器从 Zone 选 44 MB 视频；owner 设备时钟慢 30 秒，锚点仍按服务端时钟；2 分钟后成员手机打开、点出声，在 121.7 秒处，偏差 0.17 秒；暂停停在同一位置、继续跟上；成员本机暂停只影响自己，“回到直播”回到播出位置；换视频后从 0 开始；读者看不到操作，API 改锚点被拒 |
| 限时可见（§5.4 1–4，打印未测） | 时限 60 秒：A 在 61.1 秒消失，B 晚 30 秒打开、92.5 秒消失；A 刷新后仍隐藏；文档没变；owner 编辑模式全部可见并带“限时内容”标记 |
| 照片投票（§5.5 1–5，缩略 / 静态未测） | owner 在工具栏“准备选票”；两人手机上同时投第一票都记上；各选 3 张、成员改选一次；第 4 张被拒并提示；三端封面一致（同票按目录顺序）；成员经 API 改 owner 的记录、伪造提交者、选 4 张都被拒 |
| 一起听（§5.6） | owner 不运行控件，在工具栏“开始”，只写一个 Commit；成员手机点按后从直播位置播、显示 3 行歌词；下一首、暂停、继续，手机上已运行的播放器跟上；播客从第 40 分钟播：Range 从 5.5 MB 处读（文件 8.3 MB），跳转 167 ms；成员调的音量是自己的用户状态，刷新后还在，文档里没有；读者桌面工具栏上没有任何动作 |

回归：既有真实 Zone DV `aiworkspace.extensions`（控件在 Desktop 新建与重建、库、两个账号互用、CLI 打包安装）4 条全过。`playwright.aiworkspace.zone.config.ts` 原有 11 条 6 过 5 失败，都与本单无关：journeys 2 条、`aiworkspace.spec` 2 条找中文标签而页面是英文（新成员和 Workspace 子域按浏览器语言）；`aiworkspace.mobile` 1 条断言“手机只读”（10-15 写的），样例的文档页在手机上是可编辑的富文本（flow 页不在本单改动内），另有手机项目的 Service Worker 证书错误（`BUCKYOS_UI_DV_TLS_SPKI` 对它不起作用）。

DV 中改的是用例本身，产品代码没改：admin 的“我的文件”是 `/home`（列出所有成员），要先进自己的文件夹；已选中照片上盖着“本人”圆环，Playwright 的可点检查拦住点按（真实手指落在圆环上，同样是 `pick`），改为按坐标点；手机刷新后要等 HTML 控件的启动层出现再点，太早点只会选中。

DV 中看到、待 owner 定：

- CLI 第一次建日程时，标题跟运行 CLI 的机器语言（这次是 Schedule / Calendar / To-do）。Agent 容器是英文环境时，中文用户的日程也会是英文标题。
- 手机刷新后选中状态恢复，读者在查看模式下看到“Open definition”工具条；刷新后一起听是否自动续播没有断言（截图停在 ▶ 40:01）。
- DV 临时成员删除后 `/home/dvaiws*` 文件夹留在 Zone（夹具原有行为）。

### 7.5 owner 反馈：插入待办 / 日历提示“没有绑定表格”（2026-11-12）

原因：插入时表其实建好了、绑定也写进去了，但 Desktop 本地的大纲镜像（`state/outline.ts` 的 `extras()`）不投影数据控件和组合控件的 `bindings`，服务端的 `outline_extras` 会投影。刚插入的控件要刷新页面才有绑定。组合控件在页面打开时插入也一样。已补上，`entity.set_props { key: "bindings" }` 之后也会刷新。

按 owner 的要求改了插入规则（`ContainerDefinition.source`，`ui/controls/data/source.ts`、`SourceChooser.tsx`）：

- 没有对应 `schema_id` 的表：新建一张。
- 只有一张：直接绑定。
- 有多张：弹出选择，可选“新建表格”，取消就不插入。

没绑定表格的控件显示“选择或新建表格”，按同一规则绑定；在编辑模式下要先选中控件才能点。原来的提示“重新插入一个即可新建”已去掉。

顺带修复：同一画布上新建第二张待办表会因同级重名被拒（`NAME_CONFLICT`，原来在一个画布上插两个待办就会遇到），新表现在用 `uniqueName` 取名（`待办 (2)`）。

只有一张时直接绑定，是按 owner 原话（“没有就新建、有多个要选择”）理解的；想在已有一张时另起一张，目前只能先插入再在别处新建，要不要在只有一张时也问，待 owner 定。日历的可选 `todo` 绑定插入时不自动处理。

验证：`tests/aiworkspace/todo.spec.ts` TD10（没有 / 一张 / 多张选已有 / 多张新建 / 取消 / 日历 / 未绑定控件就地绑定）。

同日 owner 在 DV 发现日历合并不了待办（契约里 `todo` 是可选绑定，应可绑多张）。原来日历只读一个 `todo` 绑定，界面上也没有地方加绑定。已改：

- 绑定按名字分类：`todo`、`todo_<名字>` 是待办表，其他名字是日历表；契约加 `todo_*`，日历契约版本升到 2（`CONTRACT_VERSION` 改为按控件分）。
- 日历菜单加三项：“再显示一张日历表”（没有未显示的就新建）、“显示待办表的截止日期”（只剩一张未显示的直接加）、“不再显示某张表”（只去掉绑定）。
- 插入日历时，工作区里只有一张待办表就一并绑定。
- 隐藏开关也适用于待办表。

验证：`calendar.spec.ts` CA10。契约编进了 aiworkspace 服务（`control.describe`），要重新部署服务 Agent 才能读到新契约。
