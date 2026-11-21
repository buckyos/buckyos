# AIWorkspace 表格交互优化 TODO

2026-11-21 · 已实现，iPad 实机补验待完成 · 评审基线 `buckyos/main@6e3d91cbf` · 同日按评审修订，取舍与 owner 答复见文末「评审取舍」

用户在自由画布插入一张普通表，立即看到表名、三列表头和两条空记录。点左上数据格就能输入，点表名能选中整张表。在右下数据格按 Tab，光标进入新增行的第一格。用 iPad 手指也能选行列、调整大小、就近插入。以这条路径作为首个端到端用例。

## 评审时的现状与实现入口

| 入口 | 改动前的行为 | 本次改动 |
| --- | --- | --- |
| 自由画布的表格 Block | 截图中「日程表」来自 `table.hover → sourceLabel`。同一处还注册了下方单个 `add-row`。画布只为当前悬停或触摸选中的对象生成这些控件。静态表格使用 `readOnly compact`，激活后才挂编辑器。 | 表名成为常驻的整表选择入口。首击数据格需从静态渲染把目标交给编辑器。复用现有编辑器挂载预算和写锁。[表格 Block](../src/frame/desktop/src/app/aiworkspace/ui/blocks/builtin.tsx) · [标题取值](../src/frame/desktop/src/app/aiworkspace/ui/blocks/affordances.ts) · [BlockHost](../src/frame/desktop/src/app/aiworkspace/ui/blocks/BlockHost.tsx) · [画布事件与覆盖层](../src/frame/desktop/src/app/aiworkspace/ui/canvas/render/RenderHost.tsx) |
| 网格与视图 | 已激活的桌面网格支持单击改值、布尔勾选、列宽拖动和拖表头换列。手机网格为两次点按。行采用 34/44 px 固定高度虚拟化。网格和卡片可在菜单切换，按人和设备保存，静态缩略图强制网格。 | 沿用值编辑、冲突处理与列宽保存。改用独立把手拖列，接入行操作和可变行高。流式页已有可改名的 Block 标题，要合用一行。[表格编辑器](../src/frame/desktop/src/app/aiworkspace/ui/table/TableEditor.tsx) · [网格](../src/frame/desktop/src/app/aiworkspace/ui/table/TableGrid.tsx) · [流式页](../src/frame/desktop/src/app/aiworkspace/ui/canvas/FlowSurface.tsx) |
| 普通表与标准表 | 画布 `newTableOps` 和数据树 `newDataOps` 分别生成必填文本 `title`、布尔 `done` 两列，初始均为零条记录。已有数据源、标准待办/日程和导入各有创建路径。 | 两个普通新建入口共用 3×3 构造逻辑。绑定已有表只创建视图；标准表按自己的 schema 创建；导入按源数据的实际行列数落库。[画布新建](../src/frame/desktop/src/app/aiworkspace/ui/blocks/builtin.tsx) · [数据树新建](../src/frame/desktop/src/app/aiworkspace/ui/sources/dataOps.ts) · [表格导入](../src/frame/aiworkspace/import-table/operations.ts) |
| 表格数据与显示顺序 | 字段、记录和值属于 TableSource。Block 的 `payload.fields` 保存显示列、列序和宽度，`payload.manual_order` 支持按记录 ID 手排。直接打开数据源时，列设置来自个人状态 `tableview:<sourceId>`。 | 改字段名或插行列写数据源；调整外观、列序和行序写当前视图。两个 Block 可展示同一份数据并拥有不同布局。[显示列模型](../src/frame/desktop/src/app/aiworkspace/ui/table/model.ts) · [视图键校验](../src/frame/aiworkspace/core/src/types.rs) · [查询与排序](../src/frame/desktop/src/app/aiworkspace/api/tableQuery.ts) |

## 开发清单

### 1. 打通建表、schema 和快速新增

- [x] 普通表一次提交创建三个 `text` 字段和两条有独立 `record_id`、`values: {}` 的真实记录，画布入口在同一提交中创建 Block。三个字段都不设 `required`（可选字段不填就是不写键），首列可作为 `title_field_id`。首行只渲染字段名；查询应返回 **2 行、3 列**。六个空格的值都处于未设置状态。
- [x] **先定基础行序，它是尾格 Tab、底边「+ 行」和首个用例的前提。** 现在没有排序也没有手动键时，记录按 `record_id` 排列，而 Desktop 生成的 `record_id` 是随机串，新增记录会落在随机位置，新建表的第 3 条也一样。改为按 `RecordRow.created_seq` 排列，同一提交内再按 `record_id`；导入生成的 `r<prefix><n>` 顺序 ID 因此仍保持源数据行序。core 求值器和 SQL 编译器同时改，在线查询的逻辑表目前不含 `created_seq`，要补上。§3 的手动行序建立在这个基础顺序上。[ID 生成](../src/frame/desktop/src/app/aiworkspace/api/ids.ts) · [记录行](../src/frame/aiworkspace/core/src/model.rs) · [逻辑表与 SQL 编译](../src/frame/aiworkspace/core/src/sqlview.rs) · [内核排序](../src/frame/aiworkspace/core/src/filter.rs) · [导入的行 ID](../src/frame/aiworkspace/import-table/operations.ts)
- [x] 新建与快速加列使用同一套本地化名称，例如「列 A、列 B、列 C」，下一列为「列 D」；遍历全部字段避开重名，包括隐藏列、改过名的列和 Z 之后的列。列名和 `field_id` 分开生成。点右边缘或菜单「在左/右插入列」即生成可选文本列，可立即点表头改名。插入位置分三处落地：`table.add_field` 已接受 `order_key`，用它把新字段排在锚点字段旁，没有显式列清单的 Block 由此得到位置；有显式列清单的 Block 把新字段插到锚点旁，`addFieldOps` 已能在同一提交中写清单，但目前只追加到末尾；数据页在个人状态的 `order` 里插入。[字段操作](../src/frame/desktop/src/app/aiworkspace/ui/fieldOps.ts)
- [x] 抽出边缘加号、把手菜单、尾格 Tab 共用的插行操作，按**完整字段定义**组装，包含视图隐藏字段。普通可选字段省略键；有默认值的字段省略键，由服务在插入时填入（包括 `false`、`0`）。`required` 检查键是否存在，`nullable` 决定能否写显式 `null`；必填且可空、无默认值时显式写 `null` 并带相应 `field_type_revs`。清空单元格也要区分 unset 与 null。`unique`、`object_ref` 及服务回填的 `submitter` 禁用常量默认值，提交人交给服务填写；新增必填列遇到存量行时仍走已有 backfill 校验。
- [x] **新记录缺值时打开精简表单，这是所有表的通用规则。** 普通表和内置标准表行为一致，不为系统表开后门；schema 可以有「必填、不可空、无默认值」的字段，建表、加列和模板都不另加限制。边缘「+ 行」、「≡」菜单插入和尾格 Tab 遇到新记录还缺值的字段时不直接插行，而是在原处打开新增记录表单。表单默认只列缺值字段，点「展开全部字段」才显示其余字段，尽量不让用户面对大量输入框。缺哪些字段由 core 判定：schema 上必填、不可空又无默认值的字段，加上表的跨字段规则要求的字段，视图里隐藏的也算。core 的规则校验失败时已在错误数据里带 `field_id`，但没有导出给 Desktop；补一个 WASM 导出，按表的字段定义、`schema_id` 和当前草稿返回还缺的字段，表单随输入重算。UI 不写任何标准表专用判断，预填只来自 schema 默认值。提交后焦点落到新记录的第一可编辑格；取消不产生记录；提交失败时保留输入，并在对应字段上给出原因。在现有新增记录表单上改，它现在列出除提交人和引用外的全部字段。没有缺值字段的表仍一次点击直接插行。[新增记录表单](../src/frame/desktop/src/app/aiworkspace/ui/table/RecordPanel.tsx) · [跨字段规则](../src/frame/aiworkspace/core/src/standard.rs) · [WASM 导出](../src/frame/aiworkspace/wasm/src/lib.rs)
- [x] **标准待办和日程是这条通用规则的验收样本，不走专门路径。** 待办需要标题和有效状态；日程需要标题，定时事件需要 `start`，全天事件需要 `start_date`。待办的 `status` 在标准表定义里加默认值（初始选项 `todo`），新增时由服务填入，表单只问标题；删除被默认值使用的选项时 core 已有报告。日程的表单列出标题和 `start`，展开后勾选全天，表单改为要求 `start_date`；时间由用户选，不另写日程专用的初始化。标准表定义改动后，同步更新标准表契约、控件契约 `todo.json` / `calendar.json` 里给 Agent 的手册文本、`controls/schedule.ts` 和同级仓库的 `buckyos-websdk/src/aiws_schedule.ts`。[标准表契约](../src/frame/aiworkspace/schemas/standard-tables.md) · [待办控件契约](../src/frame/aiworkspace/aiws/contracts/todo.json) · [日程控件契约](../src/frame/aiworkspace/aiws/contracts/calendar.json) · [日程 provider](../src/frame/aiworkspace/controls/schedule.ts) · [默认值与选项删除](../src/frame/aiworkspace/core/src/plan_table.rs)

### 2. 区分整表、表头和数据格的点击

- [x] 常驻标题放在 Block 框内顶部，属于 Block 的命中区和打印内容，表格区域相应下移；表格 Block 不再生成悬浮的 `name` 标签。流式页复用已有的 Block 标题行，不另起一行。取值和改名统一到流式页 `BlockTitle` 的规则：Cell 的 `title` 优先，退回表的标题或名称，改名写 Cell 的 `title`；画布悬浮标签现在是表的标题优先，一并统一。字号比现在的悬浮标签明显大一级。画布上单击标题选中整个表格 Block，仍能用画布工具移动整表，双击标题就地改名；流式页保持单击改名，整表选择走 ⋮⋮ 把手。标题旁放可直接切换「网格 / 卡片」的按钮，沿用已有的按人按设备偏好，切换后在画布静止、编辑和再次打开时都看到选定模式。静态渲染目前固定为网格（`compact` 强制 `grid`），要补只读的静态卡片渲染；每个人看到的是自己的偏好，打印仍按网格输出。悬停标题显示字段数、记录数、来源或只读状态等已有属性；键盘聚焦和触控点击属性入口也可查看。移动指针到属性面板时保持可操作。
- [x] 在可编辑的自由画布、流式页和数据页，首次点中数据格就打开对应编辑器或执行一次类型操作。静态网格命中后携带 `record_id + field_id`，待编辑器及所需写锁就绪后聚焦同一格；布尔值只切换一次。手机切到网格也改为单点打开现有底部编辑条，默认卡片入口继续可用。标题、把手、菜单、画布链接按钮各自处理事件。只读、外部来源、锁冲突和公式格沿用现有能力与反馈。[编辑器宿主](../src/frame/desktop/src/app/aiworkspace/ui/table/StandardTableEditor.tsx) · [值编辑器](../src/frame/desktop/src/app/aiworkspace/ui/ValueEditor.tsx)
- [x] 表头外观和普通格对齐，保留 `columnheader` 语义与现有类型图标。点字段名就地改名，Enter/失焦提交一次、Esc 取消；空名、重名与并发改名给出反馈并保留输入。复用 `table.update_field`、`changes.name` 和编辑开始时的 `expect.rev = field.def_rev`。改名后其他引用该字段的视图显示新名，字段 ID、值和批注锚点继续有效。
- [x] 在表属性中加入「显示表头」，默认打开。隐藏后数据第一行仍是同一条记录；上边缘继续提供列把手，可从标题恢复表头。桌面单元格导航、辅助技术读取列名、复制结构化表格均按字段模型工作。

### 3. 行列把手、尺寸和顺序

- [x] 鼠标进入数据格时，同时显示对应行左缘、列上缘的小把手。点把手选中整行或整列，画出连续选区；行把手左侧、列把手上侧出现独立的「≡」菜单。拖动超过阈值才开始重排，放手显示最终位置。表头始终在首行，表头单元格只负责改名，列重排从列把手发起。已有记录的多选、复制、删除与详情入口仍可达。
- [x] 横向拖列分隔线调整列宽，纵向拖行分隔线调整行高。选中列时在列边缘显示可横拖的圆点，选中行时在行边缘显示可纵拖的圆点。iPad 点标题选中表后即可发现首组把手，点行左缘或列上缘可选择目标，再点「≡」或拖圆点。按实际指针和 hover 能力切换入口，使同一台 iPad 的手指、鼠标/触控板都可操作。给把手、圆点和菜单独立的触控命中区，区分滚动、画布平移和缩放。
- [x] 按 `field_id` 存列宽、按 `record_id` 存有改动的行高，设置合理上下限。拖动时本地预览，松手只保存一次，`pointercancel` 还原。将屏幕位移换算为表格本地尺寸，缩放后的画布也要跟手。更新虚拟列表的按行估高、测量和定位；滚出再滚回、重排、远端插入、隐藏表头后，尺寸仍跟随同一行列。[虚拟网格](../src/frame/desktop/src/app/aiworkspace/ui/table/TableGrid.tsx) · [分页器](../src/frame/desktop/src/app/aiworkspace/ui/tablePager.ts)
- [x] 列拖动复用 `saveColumns`。行拖动与就近插入建立在 §1 的基础顺序上，以现有视图 `manual_order` 为起点。无显式排序时，有手动键的行在前，无键行按基础顺序排列，所以只给新行一个键会让它跑到无键行前。在线查询由 WASM 把视图编译成 SQL 发 `doc.query`，手动键映射被展开为 `CASE` 并在 ORDER BY 中出现两次，查询文本上限为 16 KiB；按每条约 80 字节估算，两百条上下就会超限。先用 **1,000 条无手动键的已有记录**复现首次尾部追加和中间移动，再选能按页读取的表示，例如以基础顺序为底、只给移动过的行写可索引的局部顺序键。避免首次拖动就枚举全表、把所有 ID 写入配置或塞进查询文本。`manual_order` 目前只有 wish plan 写入（Desktop 还没有写入方），换表示时一起改它的写入和读取集合里对该键的跟踪，不保留旧格式。在线 SQL 和离线副本在分页前应用同一顺序，数据页的个人行序也接入两条路径。顺序变化触发 pager 重查。[内核排序](../src/frame/aiworkspace/core/src/filter.rs) · [SQL 编译](../src/frame/aiworkspace/core/src/sqlview.rs) · [查询长度上限](../src/frame/aiworkspace/store/src/sql.rs) · [在线查询](../src/frame/desktop/src/app/aiworkspace/api/tableQuery.ts) · [wish plan 的行序写入](../src/frame/aiworkspace/store/src/wish/plan.rs) · [读取集合](../src/frame/aiworkspace/store/src/wish/readset.rs) · [数据页的合成视图](../src/frame/desktop/src/app/aiworkspace/ui/TableViewCell.tsx)
- [x] 有显式排序时按排序结果显示，行菜单提示「清除排序后可手动移动」。筛选下以真实记录 ID 定位，保留未显示记录的相对顺序。新增行落在筛选外、排序到别处或超出 `row_limit` 时，提示已新增并提供打开该记录的入口。列序、行高、表头开关和风格仿照现有列宽的归属，Block 保存为带版本前提的视图配置，数据页保存为个人偏好；共享更新冲突时重新读取，避免覆盖他人设置。[视图保存](../src/frame/desktop/src/app/aiworkspace/ui/table/TableEditor.tsx)

### 4. 快速插入与键盘

- [x] 桌面鼠标靠近网格底边显示横跨表宽的「+ 行」，靠近右边显示纵跨表高的「+ 列」，点击一次即插入（新记录缺值时，「+ 行」打开精简表单）。离开时收起，右下角一次只命中一个按钮。移除画布 `bottom-out/add-row` 小加号和网格底部常驻的 `NewRecordRow`；把表头的 `QuickAddField` 详细配置留在字段管理入口。保留记录计数、选择工具栏，将卡片模式的新增动作放在其可点击菜单中。零行的表在触屏也能通过标题菜单加第一行。
- [x] iPad 的「≡」菜单根据选区提供「在上方 / 下方插入行」或「在左侧 / 右侧插入列」。鼠标也能使用同一菜单。插入位置以选定 ID 为锚；需要同时改 TableSource 和 Block 行序或显式列清单时放进同一 commit，刷新、撤销或请求重试都不会留下半个操作。区分行的 `append/update`、列的 `structure` 和共享视图的 `update` 权限；只有追加权限时也保留合法的新增入口。
- [x] Tab 按可见列前进，行尾换到下一行首格；Shift+Tab 反向移动。最后一条记录的最后一个可见格按正向 Tab，先完成当前编辑，再新增**恰好一行**并将焦点送到该记录的第一可编辑格；新记录缺值时改为打开精简表单。格已聚焦和正在输入两条路径共用逻辑。Tab 因此不会离开网格，要给出退出方式：格已聚焦（未在输入）时按 Esc，焦点回到表格 Block（画布上即选中整表），再按 Tab 离开；网格用 `aria-describedby` 写明这一方式。用查询总数、下一页和行数上限判定尾部，遇到分页边界先加载；仅被 `row_limit` 截断时提示查看全表。输入无效、提交冲突、无追加权时保住编辑或焦点；长按 Tab、失焦和重试不能重复插入。[键盘导航](../src/frame/desktop/src/app/aiworkspace/ui/table/TableGrid.tsx) · [编辑中 Tab](../src/frame/desktop/src/app/aiworkspace/ui/ValueEditor.tsx)

### 5. 图片、画布字段与表格风格

- [x] 将需求中的「行类型」落实为**字段（列）类型**。新增「图片」「画布」两个字段选项，底层优先复用 `object_ref`，值保存工作区对象引用。给字段增加可持久化、可校验的用途标记，供渲染和选择器区分现有通用引用。补齐 Desktop `FieldDef`、字段管理、图标、值编辑器以及记录表单中目前锁定/跳过 `object_ref` 的分支。服务端除 `target_types` 外还要校验具体目标，直接 API 写入也适用。[引用值与约束](../src/frame/aiworkspace/core/src/value.rs) · [引用写入检查](../src/frame/aiworkspace/core/src/plan_table.rs) · [Desktop 类型](../src/frame/desktop/src/app/aiworkspace/api/types.ts) · [记录面板](../src/frame/desktop/src/app/aiworkspace/ui/table/RecordPanel.tsx)
- [x] 图片格可选已有图片或上传，显示按比例缩放的缩略图，可替换、清空。目标为可读且媒体类型为图片的 `buckyos.asset-ref`。复用现有上传与鉴权读取，上传失败保留原值。画布格选择当前工作区另一个可读 `buckyos.container`，核对 `payload.kind === 'surface'`；显示目标当前标题，独立的「进入画布」按钮打开对应画布。改名后更新标题，目标删除或权限收回后显示不可用状态；普通文件夹和非图片资产应被拒绝。网格、卡片和表单使用同一引用解释。[图片上传](../src/frame/desktop/src/app/aiworkspace/ui/media/stage.ts) · [鉴权图片显示](../src/frame/desktop/src/app/aiworkspace/ui/extensions/AssetBlobImage.tsx) · [画布选择与导航](../src/frame/desktop/src/app/aiworkspace/ui/surfaces/SurfacePicker.tsx)
- [x] 在表属性中提供一键应用的风格预设。首批建议「标准网格」「隔行底色」「简洁横线」，用主题变量实现深浅色，保证选区、类型图标、输入焦点和只读状态可辨。缺省使用标准网格，可一键切回。建议将预设、表头开关、行高放在表格 Block 的 `payload.config` 内，写入时合并其他键并携带 `key_revs.config`。这样行高随 `config` 整键重写，两人同时调整不同行也会冲突，按共享更新冲突的做法重读后再写；删除记录留下的行高在下次写入时清掉。静态表、编辑态与打印预览读取同一配置，打印只呈现表格内容和外观。[网格样式](../src/frame/desktop/src/app/aiworkspace/ui/table/table.css) · [独立打印渲染](../src/frame/desktop/src/app/aiworkspace/print/renderer/PrintTable.tsx)

## 验收与回归

| 场景 | 可观察结果 |
| --- | --- |
| 从零插表 | 画布和数据树都得到 3 个可选文本字段、2 条空记录。单击左上格输入「323」，刷新后仍在原格。绑定同一数据源的第二个 Block 时记录数仍为 2；之后新增的第 3 条出现在底部。 |
| 桌面选择与外观 | 指针移开仍见框内的大标题和切换按钮。画布上单击标题只选整表、双击标题改名，点字段名只改名，点把手只选对应行/列。切到卡片后画布静止时也显示卡片，打印仍是网格。两个视图共享字段名和值；分别隐藏表头、设风格、调整行高列宽后，重开时各自恢复。 |
| 桌面快速新增和键盘 | 新记录不缺值的表，底边加一行、右边加一列都只需一次点击。尾格输入后按 Tab 正好增加一条，新行出现在底部，焦点落在新行；Shift+Tab、分页边界、校验失败不加行。格已聚焦时按 Esc 再按 Tab 能离开网格。撤销一次清除对应插入，原有格值与顺序正确。 |
| iPad 与手机 | iPad 在无 hover 条件下可用把手选区、打开「≡」、就近插入、拖圆点调整行高和列宽；接鼠标后可用边缘加号。手指正常滚动、双指缩放不会误提交。手机卡片可新建，切网格单点即可编辑。 |
| 必填字段、业务表与权限 | 普通表和标准表行为一致：新记录缺值时，点「+ 行」或在尾格按 Tab 打开精简表单，只列缺值字段，展开后可见全部字段，取消不产生记录。普通表加一个「必填、不可空、无默认值」的字段后同样如此。待办只问标题，状态由默认值填入；日程勾选全天后要求的字段从 `start` 换成 `start_date`。默认值含 `0/false`、必填可空、唯一列、系统提交人、隐藏必填列各有用例。标准待办和两类日程通过业务校验。只读/外部来源/锁冲突不会改数据，只有追加权限的用户不能顺带改 schema 或共享视图。 |
| 顺序、并发与引用 | 在 1,000 条无手动键的已有记录末尾追加，再将一条移到中间。在线和离线跨页读取都能看到相同顺序，其余 999 条的相对顺序不变，在线查询文本不超过上限。导入的表保持源数据行序；wish plan 写出的行序在新表示下仍正确。叠加过滤、显式排序、并发修改、断线重连和撤销，核对 ID 与记录数。图片能显示并替换，画布同名也进对目标，目标改名、删除、失去读权均有反馈。 |

交互回归见 [table.spec.ts](../src/frame/desktop/tests/aiworkspace/table.spec.ts) 与 [objects.spec.ts](../src/frame/desktop/tests/aiworkspace/objects.spec.ts)，其中覆盖独立把手、边缘加号与手机单点编辑。对照 [UI 框架 PRD §7.3](<../doc/workspace/BuckyOS AI Workspace UI 框架PRD.md>) 更新手势说明；schema 或表格引用协议变动时同步相关 SDK 类型、WASM、AIWS 模板手册，并按 AGENTS.md 的 cmd_skills 表检查 `aiworkspace_run`、`aiworkspace_template_dev` 两个手册。以下为实现后的回归入口。

```bash
# 在 buckyos/src 运行；若修改了 core，再运行 WASM 构建脚本供 Desktop 验证
cargo test -p aiworkspace-core
cargo test -p aiworkspace-store --test sql --test replica
bash frame/aiworkspace/wasm/build.sh

# 在 buckyos/src/frame/desktop 运行
pnpm run check
pnpm exec playwright test --config=playwright.aiworkspace.config.ts table.spec.ts objects.spec.ts flow.spec.ts phone.spec.ts features.spec.ts table-import.spec.ts print.spec.ts

# 改了 aiws_schedule.ts 时，在同级 buckyos-websdk 运行，再构建 dist 供 Desktop 使用
pnpm exec jest tests/aiws_schedule.test.ts tests/owner_schedule.test.ts
pnpm run build
```

以桌面鼠标、iPad 尺寸的无悬浮触控和手机分别记录结果。iPad 实机补验软键盘焦点、滚动与圆点拖动，并注明浏览器及是否连接触控板。

## 实现与验证记录

| 环境 | 已验证的交互 |
| --- | --- |
| 桌面 Chromium 156.0.8078.4，鼠标和键盘 | 画布、数据树新建普通表得到三列和两条空记录。自由画布首击输入 `323`，尾格 Tab 新增一条并聚焦新行。覆盖跨页导航、长按 Tab、冲突保留输入、Esc 退出、字段改名、视图重开，以及首次取写锁、只读和仅追加权限。 |
| iPad 第七代横屏模拟，1080 × 810，Chromium 无 hover 触控 | 在自由画布点标题后显示把手。触控选行列、打开菜单、就近插入、调整尺寸和取消拖拽均有用例。单指滚动表格和双指缩放画布后，服务端原值保持，下一次点按仍可编辑。 |
| Pixel 7 模拟，412 × 839，Chromium 触控 | 默认卡片可新增、筛选和删除。切到网格后，单点打开底部编辑条，横向滚动保留表头和首列。 |
| 打印 | 两个 Block 共享来源并各自设置标题、列宽、行高、表头和风格。通过真实 `print.package` 将生产渲染页面交给 Chromium 生成 PDF，分别核对；屏幕为卡片的视图也输出网格。 |

行序按创建序号及记录 ID 排列，视图仅为人工移动过的行保存 `row_order`。在 1,000 条记录上复现旧行为后，以每页 137 条比较在线 SQL 和离线副本的 ID 顺序。新查询使用绑定参数，回归断言 SQL 正文小于 1 KiB。图片和画布字段的服务端校验、目标删除和撤权，以及网格、卡片、表单中的显示均有回归覆盖。

七组 Playwright 回归连接真实服务运行，85 项中 84 项首轮通过，写锁释放用例重试通过。将该断言改为等待服务端确认释放后，再并发重复两次均通过。数据模式另跑 12 项全部通过。

Desktop 类型检查、WASM 构建、SDK 构建及两组日程测试（25 项）通过。Rust 的 SQL 和副本测试共 10 项通过。Core 全量测试 190 项通过，历史回放快照 `recorded_deltas_replay_without_the_planner` 失败；在改动前的 `a3ebe999f` 复现相同摘要差异。该快照中的 Loro 版本为 1.17.1，当前依赖为 1.18.0。额外运行的 i18n 审计在改动前后均为 58 项通过、3 项失败，报告的是同一组缺失键、未使用键和后端文案。

实机补验待完成。当前环境未连接 iPad 或实体触控板，Safari 软键盘焦点、实机滚动和圆点拖动仍需在设备上记录。

## 评审取舍

修订时替 owner 做的取舍，owner 11-21 已全部确认，正文已按此写，没有待确认项。

1. **必填字段是所有表的通用能力，新增时用精简表单。** owner 定：普通表也允许「必填、不可空、无默认值」的字段，新增时弹出的表单只列必填字段，展开才看到全部字段，尽量不让用户面对大量输入框；这是通用规则，不给内置的系统表开后门（§1「精简表单」一条）。原稿「允许为空或有合法默认值」的 schema 限制、给标题填本地化默认名直接插行、日程专用的时间初始化都不再采用。按这一原则另定的三点（我提出，owner 11-21 确认）：缺哪些字段由 core 经 WASM 判定，UI 不写标准表专用判断；待办的 `status` 改用 schema 默认值，表单只问标题；日程不做专用的时间初始化，预填只来自 schema 默认值。
2. **常驻标题放在 Block 框内顶部，取值和改名跟流式页一致**（已确认）。Cell 的 `title` 优先，改名写 Cell，不改表本身的名称；同一份数据的两个 Block 可以叫不同的名字。画布上单击选中、双击改名，流式页保持单击改名。
3. **打印仍按网格输出**，卡片只用于屏幕（已确认）。
4. **格聚焦时用 Esc 退出网格**，之后 Tab 离开；没有另设快捷键（已确认）。