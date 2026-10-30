# AI Workspace 表格数据导入 TODO

日期：2026-10-29（2026-10-30 T0–T6 已实施，实施回写见 §13–§16）
需求：[BuckyOS Ai workspace 表格数据导入需求](<BuckyOS Ai workspace 表格数据导入需求.md>)（2026-10-29 口述整理稿）
基线：当前工作区（draw.io / PDF 导入、脑图、多媒体、内核 M0–M5 已实施）。路径相对 `buckyos/`。

目标：用户把 Excel / Numbers / 网页 / Notion / Miro / 另一个 aiws 表格里的内容复制后粘贴到 Workspace，或选择 CSV / xlsx / SQLite 文件，经一个一屏向导得到一张**结构化 TableSource**（字段名、字段类型、记录），默认选项就能完成；并为 Table 增加 Currency 字段类型。本文按源码核对列出现状、规则、分期任务与待裁决项，供实施 Agent 直接开工。

## 0. Review 结论（需求 ↔ 现有实现）

1. **目标结构已经很严格，导入器要做的是“整形”而不是“搬运”。** TableSource（`buckyos.table-source`）= 字段定义（`FieldDef`：`text | boolean | number | decimal | date | datetime | select | multi_select | object_ref`）+ 记录（`record_id → values`），写入时规范化、不合法即拒绝、不做尽力转换（[value.rs](../src/frame/aiworkspace/core/src/value.rs)，设计 §3.4.2）。所以类型识别、值整形、币种解析全部在**客户端导入层**完成，core 不加任何“宽松解析”。
2. **校验规则不用重写：** core 的 `normalize_value(field_def_json, value_json)` 已通过 WASM 暴露给浏览器（[wasm/src/lib.rs](../src/frame/aiworkspace/wasm/src/lib.rs) 第 66 行）。向导里“这列按 X 类型会有 N 个值不合法”就用它逐格算，和后端判定逐字一致，不会出现预览通过、提交被拒。
3. **“Workspace 表格属于 SQLite 类体系”只对一半。** 存储确实是 SQLite（`table_records.values_json`，[schema.rs](../src/frame/aiworkspace/store/src/schema.rs)），但一张 TableSource 不是一张 SQL 表，记录是 JSON 行。SQLite 文件导入只能是“解析 → 转换 → 提交”，不存在挂载或拷库。好消息：Desktop 已依赖 `@sqlite.org/sqlite-wasm`（离线副本用，[replica.worker.ts](../src/frame/desktop/src/app/aiworkspace/offline/replica.worker.ts)），浏览器内读用户的 `.sqlite` 不需要新依赖。
4. **剪贴板现状：** 画布只认自己的标记、draw.io 文本、SVG 和文件（[CanvasView.tsx](../src/frame/desktop/src/app/aiworkspace/ui/canvas/CanvasView.tsx) `onPaste`，约第 1074 行）；表格视图没有“复制”（[TableViewCell.tsx](../src/frame/desktop/src/app/aiworkspace/ui/TableViewCell.tsx) 无 clipboard 代码）。Excel / Numbers / Google Sheets / Notion 都同时写 `text/html`（`<table>`）和 `text/plain`（TSV），HTML 里还带原始值（Excel `x:num`、Google `data-sheets-value`），这是类型识别最可靠的来源；Miro 的 payload 待实测（T1 第一项）。
5. **文件入口现状：** 文件进画布有三条路都汇到 `handleFiles` → `classifyFile`（[drawioImport.ts](../src/frame/desktop/src/app/aiworkspace/ui/canvas/drawioImport.ts) 第 176 行）：拖放、粘贴文件、插入目录的“导入文件”；PDF 已经有“单个文件先问：作为文件 / 导入为页面”的样板（`openPdfImport` + [ImportPdfDialog.tsx](../src/frame/desktop/src/app/aiworkspace/ui/canvas/ImportPdfDialog.tsx) 的 `ask` 步）。CSV 的“插入文件”入口就是在 `classifyFile` 加 `tabular` 一类并复用这个问法；“插入表格”入口要在 table Block 的目录详情页和主菜单“导入”区各加一项。
6. **提交限额：** 一次 commit ≤ 10 000 个操作（`Limits.max_ops`），一个 `table.insert_records` ≤ 50 000 条（`max_set_values`），PDF 导入按 2 000 操作/批提交（[pdfImport.ts](../src/frame/desktop/src/app/aiworkspace/ui/canvas/pdfImport.ts)）。表格导入的瓶颈不是操作数而是请求体大小（aiws server 没找到显式 body 上限，网关未核），T0 要实测后定批次。
7. **Currency 是唯一要动 core 的部分**（新增 `FieldType::Currency`），影响面：`value.rs`（规范化、比较）、`filter.rs`（运算符矩阵）、`plan_table.rs`（`migrate_value`）、WASM、store `wish/results.rs`（`FIELD_TYPES` / `infer_type`，让许愿格结果也能出 currency 列）、Desktop 类型/显示/编辑/字段管理、aiws README 字段类型表、设计文档 §3.4.2。是“格式内的新增”还是要 bump 版本见 Q2。
8. 已有类型推断可以参考但不能直接复用：`store/src/wish/results.rs::infer_type` 面向程序产出的 JSON 值（数字已经是 number），导入面对的是字符串，规则要更丰富（千分位、日期写法、币种符号、百分号）。推断实现放在共享 TS 模块里，顺手把它的规则对齐（Currency、decimal 判定）。
9. 需求 §4.3 的“表头 / 一列 / 一行 / 单元格 → 字段名 / 字段 / 记录 / 值”在现有模型里是 1:1 的，没有阻碍；合并表头、标题区、多层表头按 §3.1 的规则降级并在报告里列出，不做智能识别。

## 1. 现状对照

| 范围 | 已有 | 缺口 |
| --- | --- | --- |
| 表模型 | `FieldDef`（9 种类型、`required / nullable / unique / scale / options`）、三种“空”（未设置 / null / `""`）、`title_field_id` 可缺省但若给必须指向 text 字段（[types.rs](../src/frame/aiworkspace/core/src/types.rs) `validate_table_meta`） | 没有 Currency；没有任何“从外部网格建表”的辅助 |
| 新建表 | Block 目录 `table` 项创建默认两列 `title/done`（[builtin.tsx](../src/frame/desktop/src/app/aiworkspace/ui/blocks/builtin.tsx) 第 94 行）；数据树“新建 → 表格”同样（[dataOps.ts](../src/frame/desktop/src/app/aiworkspace/ui/sources/dataOps.ts) `newDataOps`） | 没有“从文件 / 剪贴板建表” |
| 值整形 | [values.ts](../src/frame/desktop/src/app/aiworkspace/ui/values.ts) `parseInput` 只做单格文本→wire 形状；WASM `normalize_value` | 没有列级推断、批量整形、报告 |
| 字段类型迁移 | `table.migrate_field`（text→date/number/decimal、number→decimal、select→text，[FieldManager.tsx](../src/frame/desktop/src/app/aiworkspace/ui/FieldManager.tsx) `MIGRATIONS`） | 导入后用户改类型可走它；Currency 要补进矩阵 |
| 剪贴板 | 画布对象剪贴板（[clipboard.ts](../src/frame/desktop/src/app/aiworkspace/ui/canvas/clipboard.ts)，内存 + 系统剪贴板标记）；富文本编辑器把粘贴的 Markdown（含 `|` 表）转成富文本表格（[RichTextEditor.tsx](../src/frame/desktop/src/app/aiworkspace/richtext/RichTextEditor.tsx) `clipboardTextParser`） | 画布 / 数据树收到 TSV 或 HTML `<table>` 时没有去向；aiws 表格不能复制出去 |
| 文件分流 | `classifyFile`：drawio / image / svg / pdf / other；`handleFiles` 对 OPML / Markdown 大纲建脑图，其余作 asset Block | 没有 csv / tsv / xlsx / sqlite 类 |
| 导入骨架 | `ImportReport`（kind / message / count / samples）、`ImportReportDialog`、`batches()`、`ImportOutcome`（[drawioImport.ts](../src/frame/desktop/src/app/aiworkspace/ui/canvas/drawioImport.ts)）、请求槽 + 宿主对话框模式（[pdfImportRequest.ts](../src/frame/desktop/src/app/aiworkspace/ui/canvas/pdfImportRequest.ts)） | 直接复用 |
| 共享模块惯例 | `import-drawio/`、`import-pdf/`：无依赖 TS，浏览器与 Deno 同一份代码，Vite 别名 `@drawio/* @pdf/*`（[vite.config.ts](../src/frame/desktop/vite.config.ts) 第 91–97、165 行；[tsconfig.app.json](../src/frame/desktop/tsconfig.app.json) `paths`），i18n 检查覆盖目录 [aiworkspace-i18n.test.ts](../src/frame/desktop/tests/datamodel/aiworkspace-i18n.test.ts) `SOURCE_DIRS` | 新建 `import-table/` 照做 |
| Agent 侧 | `buckyos aiws read / query / info / run-*`（[aiws.ts](../src/rootfs/libexec/buckyos-tool/cli/modules/aiws.ts)）；手册 [aiworkspace_template_dev.md](../src/apps/jarvis_agent/agent/cmd_skills/aiworkspace_template_dev.md) 教 Agent 用 `doc.commit` 手写 `table.insert_records` | 没有“把这个 CSV 导成表”的现成工具，Agent 得自己推断类型 |
| 浏览器内解析能力 | pdf.js（已打包）、`@sqlite.org/sqlite-wasm`（已依赖）、`DecompressionStream`（zip 解压，浏览器与 Deno 都有）、`TextDecoder` 含 `gb18030` | 没有 CSV / xlsx 解析器 |

## 2. 总体方案

```text
来源                         共享模块 import-table（TS，无依赖，浏览器 / Deno 同一份）                 Desktop / CLI 宿主
剪贴板 text/html|text/plain ─┐                                                                  ┌─ 向导（一屏）：表头、字段名、类型、包含列、高级
CSV / TSV 文件 ──────────────┤→ Grid（字符串/原始值矩阵 + 单元格提示）→ infer（表头、类型、币种）→ TablePlan ─┤→ WASM normalize 逐格校验 → 报告
xlsx 文件 ───────────────────┤                                       （fields + records + report）  │
SQLite 文件（宿主注入 exec）─┘                                                                  └─ operations（entity.create + insert_records 分批）→ store.submit
```

- **一条管线，四个来源**：所有来源先变成 `Grid`（`rows: Cell[][]`，`Cell = { text: string; raw?: number | boolean | string; hint?: 'date' | 'datetime' | 'currency' | 'percent' | 'bool'; currency?: string }`），`raw/hint` 来自有类型信息的来源（HTML 原始值属性、xlsx 单元格类型与数字格式、SQLite 列类型），纯文本来源只有 `text`。后面的表头识别、类型推断、整形、提交对来源无感（需求 §七）。
- **推断永远无损**：只有当某列全部非空值都能整形为某类型时才推断为该类型，否则是 `text`。损失（留空、整列转文本）只在用户在向导里**手动改类型**时发生，并计入报告。
- **core 不动（Currency 除外）**：不加新实体、不加新操作，格式版本不变。
- **宿主只做“需要环境的事”**：读文件、剪贴板、sqlite-wasm、WASM 校验、上传原文件、提交、进度。与 import-pdf“pdf.js 由宿主注入”同一思路：`sqlite.ts` 接收 `{ exec(sql): Row[] }`，浏览器给 sqlite-wasm，CLI 给 Deno 的 `node:sqlite`。

### 2.1 模块与落点

```text
src/frame/aiworkspace/import-table/
  grid.ts        Grid / Cell 类型；去空行空列、矩形化、合并单元格展开
  csv.ts         编码嗅探（BOM → UTF-8 严格 → gb18030）、分隔符嗅探（, ; \t |）、RFC 4180（引号、换行、CRLF）
  html.ts        剪贴板 HTML `<table>` → Grid（thead/th、rowspan/colspan、Excel `x:num`、Google `data-sheets-value/numberformat`、aiws 自己的 data-aiws-table 标记）
  text.ts        TSV / 多空格对齐文本 → Grid；“像不像表格”的判定（给粘贴分流用）
  xlsx.ts        zip（stored/deflate-raw）+ workbook/sheets/sharedStrings/styles → 每个工作表一个 Grid（含 raw/hint/currency）
  sqlite.ts      由注入的 exec 读 sqlite_master、列声明类型、行 → Grid（含 raw/hint）
  infer.ts       表头判定、字段名/ID 生成、列类型推断、每类型的整形函数（string → wire 值）、币种解析表
  plan.ts        Grid + 用户选项（TableChoices）→ TablePlan { title, fields: FieldDef[], records, report: ImportReport }
  operations.ts  TablePlan → 操作批次（entity.create 表 [+ cell] + insert_records 分块）
  mod.ts         公开入口；*_test.ts；tools/import.ts（Deno CLI 用的宿主）
src/frame/aiworkspace/fixtures/table/        csv/ xlsx/ sqlite/ clipboard/ expected/*.plan.json
src/frame/desktop/src/app/aiworkspace/ui/import/
  tableImportRequest.ts   请求槽（照 pdfImportRequest.ts）：来源、step: 'ask' | 'wizard'、目的地、finish
  TableImportDialog.tsx   ask 步 + 向导 + 进度；宿主组件挂在 WorkspaceShell 层（PdfImportHost 只挂在 CanvasView.tsx 第 111 行，数据树模式也要能开向导，所以挂高一层）
  tableImport.ts          宿主：读来源 → Grid → plan → WASM 校验 → 上传原文件（可选）→ 分批 submit → ImportOutcome
  clipboardTable.ts       从 ClipboardEvent 取 html/text 并判定是否表格
  tabularFiles.ts         扩展名 / 魔数判定（csv tsv xlsx sqlite），读取为 Grid（xlsx/sqlite 走 Worker）
  sqliteReader.worker.ts  用 @sqlite.org/sqlite-wasm 打开字节（sqlite3_deserialize，只读），执行只读 SQL
src/frame/desktop/src/i18n/aiworkspace/import.ts    新文本区 `aiworkspace.import.*`（index.ts parts + i18n 测试 PARTS 各加一行）
src/frame/desktop/vite.config.ts / tsconfig.app.json / tsconfig.node.json   别名 `@table/*`、`server.fs.allow`
src/rootfs/libexec/buckyos-tool/cli/modules/aiws.ts   新 verb `import-table`
```

## 3. 规则（默认值即产品行为，向导只暴露必要的开关）

### 3.1 网格整理与表头

- 去掉全空的首尾行列；中间的全空行**丢弃并计数**（Excel 用户常留空行分隔，报告“跳过 N 个空行”）；列数取最大行，短行补空。
- 合并单元格（HTML `rowspan/colspan`、xlsx `<mergeCells>`）：值放左上格，其余为空；出现在表头行时，空表头按 §3.2 补名，报告“合并表头 N 处”。
- **表头判定**（`header: 'auto' | true | false`，默认 auto）：HTML 有 `<thead>` 或首行全 `<th>` → 是；行数 < 2 → 否；否则首行全部非空且满足其一 → 是：(a) 存在某列首格是文本而该列其余值推断为非文本类型；(b) 所有列其余值都是文本，且首行值互不重复且无一是数字。其他 → 否。向导顶部一个开关“首行是表头”，切换后字段名与预览即时重算。
- 多层表头、标题区、合计行**不识别**：用户通过“表头开关”和“包含列”处理，报告不替用户猜。

### 3.2 字段名与 ID

- 字段名 = 表头文本 trim 后截到 128 字符；为空或无表头 → `列 A`、`列 B`…（i18n `aiworkspace.import.columnName`，Excel 列字母）；重名 → `名称 (2)`。
- `field_id` = `c1 … cN`（按包含列顺序）；`record_id` = `randomId('r')`。不从来源主键取 ID（Q9 之外无再导入对齐需求）。
- `title_field_id` = 第一个 `text` 字段；一张表没有 text 字段就不设（core 允许缺省）。
- 不设 `required / nullable / unique`：Excel 数据有空洞，任何约束都会让整批插入失败。

### 3.3 列类型推断（按顺序取第一个“全部非空值通过”的类型）

| 顺序 | 类型 | 通过条件（对非空值） | 整形为 wire 值 |
| --- | --- | --- | --- |
| 1 | `boolean` | 全部 ∈ {true,false,yes,no,y,n,是,否,真,假,✓,✔,☑,×,✗}（忽略大小写）；**不含 1/0** | `true / false` |
| 2 | `number` | 全部匹配数字（允许前后空白、`+/-`、千分位 `^-?\d{1,3}(,\d{3})+(\.\d+)?$`、科学记数法），且全部为整数且 \|v\| ≤ 2^53−1 | JSON number |
| 3 | `decimal` | 同上但有小数位，或整数超 2^53−1；`scale` = 观察到的最大小数位（≤ 6；> 6 退回 `number` 并报告精度）。百分号：全部带 `%` → decimal 值 = 数/100（12.5% → `"0.125"`，scale+2），报告“按比例导入” | 规范 decimal 字串（用 core 规则补零） |
| 4 | `date` | 全部匹配 `YYYY-MM-DD`、`YYYY/M/D`、`YYYY.M.D`、`YYYY年M月D日`、`M/D/YYYY` 或 `D/M/YYYY`（列内有 >12 的一侧决定顺序；都 ≤12 时按向导“日期顺序”默认 M/D，Q4）、`D-Mon-YYYY`、`Mon D, YYYY`；来源 hint=date（xlsx 日期格式、SQLite DATE）直接通过 | `YYYY-MM-DD` |
| 5 | `datetime` | 日期 + `[ T]HH:MM(:SS(.fff)?)?` + 可选偏移/Z；无偏移按浏览器时区（高级项可改 UTC）；xlsx 带小数的日期序列号同样按本地时区 | RFC 3339 UTC `…Z` |
| 6 | `currency` | 见 §6.3：全部为“金额”或“金额+币种标记”，且至少一个带标记或来源 hint=currency；列默认币种 = 显式币种中最多的那个，否则界面语言默认（zh→CNY，其他→USD） | `{ amount, currency }` |
| 7 | `select` | 文本列，行数 ≥ 5，去重后 ≤ 20 个且 ≤ 行数/2，每个值 ≤ 64 字符；首个文本列（标题列）不参与 | option_id `o1…` 按首次出现；标签 trim |
| 8 | `text` | 其余 | 原样（≤ 65 536 UTF-16） |

- `multi_select`、`object_ref` 本轮不自动推断（P2：`,` `;` `、` 分隔且去重后 ≤ 20 → multi_select）。
- 用户在向导改类型后：对该列逐格跑 WASM `normalize_value`，显示“N/M 个值不合法”，不合法值按向导里的策略：**留空（默认，unset）** 或 **整列改为 text**；留空的格计入报告并给样例。
- 空单元格 → **未设置**（不写键），所有类型一致；`""` 不当作值（Q11）。

### 3.4 提交

- 单次导入 ≤ 20 000 行 → **一个 commit**（`entity.create` 表 [+ `entity.create` Block] + 若干 `table.insert_records`，每个 ≤ 500 条），一次撤销。
- > 20 000 行 → 第一个 commit 建表 + 前 20 000 行，之后每 20 000 行一个 commit，进度条，被拒即停并在报告里说导入到了第几行（与 PDF 导入一致）。总上限 100 000 行 / 1 000 000 单元格，超限拒绝并提示（Q6）。T0 用 standalone 和经网关各实测一次 20 000 × 20 的 commit 大小与耗时，超时或被网关拒就把 20 000 下调。
- 离线：不保留原文件时允许在离线副本上提交（core 规划在 WASM 里跑）；勾了保留原文件（要上传）才要求在线（PDF 是一律要在线，这里放宽）。

### 3.5 落点与 Block

| 入口 | 表的父节点 | Block |
| --- | --- | --- |
| 画布粘贴 / 画布文件（拖放、粘贴、插入目录、主菜单导入） | 该 Surface 的内容文件夹（画布私有，D05，和目录 `table` 项一致） | 一个 `table` Cell：自由画布放在粘贴点 / 落点 / 视口中心，尺寸 `w = min(960, 40 + 150 × 列数)`、`h = min(480, 70 + 34 × min(行数, 12))`；流式页按 `pasteOperations` 的 `into` 规则插在当前节点后 |
| 数据树（粘贴、导入表格文件） | 当前选中的文件夹（同 `DataTree` 新建规则） | 不建 Block |
| SQLite 多表 / xlsx 多工作表 | 同上，一张来源表一个 TableSource | 画布上按列排开，间距 40 |

表标题：文件名去扩展名 / 工作表名 / SQLite 表名 / `粘贴的表格 <本地时间>`；向导可改。保留原文件时另建一个 `buckyos.asset-ref`（同一文件夹，走 `uploadAsset`，≤ 32 MiB），不建 Block。

## 4. 剪贴板导入

- **判定**（`clipboardTable.ts`，在 `onPaste` 里排在 `copyMarker` 判定之后、draw.io / SVG 之前）：`text/html` 含 `<table`（忽略 `<meta>`/`<style>` 包裹）→ 表格；否则 `text/plain` 至少 2 行且各行 Tab 数一致且 ≥ 1、或 1 行且 ≥ 2 个 Tab → 表格（TSV）；多空格对齐文本不自动判定（误伤普通文本）。编辑器、输入框、面板里的粘贴照旧不接管（现有 `outside` 判定）。
- **来源差异**（`html.ts` 的提取优先级）：Excel → `<td x:num="1234.5">` 原始数、`mso-number-format` 样式里的格式串；Google Sheets → `data-sheets-value`（`{"1":2,"2":"文本"}` / `{"1":3,"3":1234.5}` / `{"1":4,"4":true}`）、`data-sheets-numberformat`（含货币 / 日期格式）；Numbers、Notion、网页 → 只有显示文本；aiws 自己 → `<table data-aiws-table='{"fields":[…]}'>`（§4.1）。有原始值就用原始值，显示文本只用来识别币种 / 百分号。
- **aiws 内部复制**（T6）：表格视图工具栏加“复制为表格”（当前筛选排序下已加载的行；超过已加载行数时按 `doc.query` 翻页取完，上限 20 000 行），写 `text/plain` TSV + `text/html` 表格并带 `data-aiws-table` 标记（字段名、类型、scale、currency、options 标签）。粘贴时读到标记就预填类型与选项，但**仍显示向导**让用户一键确认（Q8）。
- 向导里“来源：剪贴板”，不保留原文件选项。
- 数据树模式也接粘贴（`DataSourceView` 聚焦时），建到当前文件夹（T1 可后置到 T2）。

## 5. 文件导入

### 5.1 分流与入口

- `classifyFile` 加 `kind: 'tabular'`，子类 `csv | tsv | xlsx | sqlite`：扩展名 `.csv .tsv .txt(含 Tab) .xlsx .xlsm .sqlite .sqlite3 .db`；魔数：`PK\x03\x04` 且 zip 内有 `xl/workbook.xml` → xlsx；`SQLite format 3\0` → sqlite。`.xls`（BIFF）、`.numbers`、`.ods` 识别出来但只报“请另存为 xlsx / CSV，或直接复制粘贴”（Q3）。
- **插入文件（入口一）**：`handleFiles`、`InsertCatalog` 的 `asset` 项选中文件、`DataTree` 的“导入文件”三处，**单个** tabular 文件 → ask 对话框（主按钮“导入为表格”，次按钮“作为文件插入”）；多个文件一律作文件（与 PDF 规则一致）。
- **插入表格（入口二）**：(a) `InsertCatalog` 的 `table` 项详情面板加按钮“从文件导入…”（`aiws-catalog-import-table`，直接开文件选择器，跳过 ask）；(b) 主菜单“插入”末尾的导入区加“导入表格数据…”（`aiws-menu-import-table`，hint `.csv .xlsx .sqlite`，`accept` 列表见上）；(c) `DataTree` 操作栏加“导入表格文件”（`aiws-datatree-import-table`）。三处都进同一个 `tableImports.open({ source, step: 'wizard' })`。
- 两个入口共用 `tableImport.ts`，不存在两套解释规则（需求 §6.1 末句）。

### 5.2 CSV / TSV

- 编码：BOM 优先；无 BOM 先 `TextDecoder('utf-8', { fatal: true })`，失败再 `gb18030`，再 `utf-16le`；向导高级项可改（utf-8 / gb18030 / utf-16le / windows-1252）。
- 分隔符：对前 20 行统计 `, ; \t |` 各自“每行出现次数相同且 > 0”的候选，取其一；向导高级项可改。
- RFC 4180：双引号包裹、`""` 转义、引号内换行、CRLF / LF / CR。
- 上限：文件 ≤ 64 MiB（纯解析内存考虑；保留原文件仍受 32 MiB）。

### 5.3 xlsx（自研读取器，Q3）

- zip：只支持 stored(0) / deflate(8)，`DecompressionStream('deflate-raw')`；读 `[Content_Types].xml`、`xl/workbook.xml`（工作表名、`state="hidden"`、`workbookPr date1904`）、`xl/_rels/workbook.xml.rels`、`xl/sharedStrings.xml`（`<si>` 下 `<t>` 与富文本 `<r><t>` 拼接）、`xl/styles.xml`（`<numFmts>` + `<cellXfs>` 的 `numFmtId`）、各 `xl/worksheets/sheetN.xml`（`<dimension>`、`<row r>`、`<c r t s>` 的 `<v> <is> <f>`、`<mergeCells>`）。XML 解析复用 `import-drawio/xml.ts`。
- 单元格类型：`t="s"` 共享串、`str` 公式字串、`inlineStr`、`b` 布尔、`d` ISO 日期、`e` 错误（→ 空 + 报告）、默认数字。公式只取缓存值 `<v>`，报告“N 个公式按计算结果导入”。
- 数字格式 → hint：内置 id 14–22、27–36、45–47、50–58 与自定义格式中引号 / 方括号之外含 `y m d h s` 的 → `date`（有 `h/s` 或序列号带小数 → `datetime`；纯时间格式 → text 并报告）；含 `[$XXX-lcid]`、`[$¥-804]` 之类 → `currency`（代码直接取 `$` 后的字母，否则按 lcid：804→CNY、411→JPY、409→USD、809→GBP、407/40c→EUR…，再否则按符号走 §6.3 歧义规则）；含 `%` → `percent`。
- 序列号日期：1900 系统（含 Excel 的 1900-02-29 bug 偏移）与 1904 系统；日期序列号整数 → `date`，带小数 → `datetime`（按本地时区）。
- 工作表：向导“工作表”下拉（默认第一个非隐藏非空），勾选“每个工作表一张表”时全部导入（隐藏表默认不导）。
- 不做：`.xls`、`.numbers`、`.ods`（后者是 zip + `content.xml`，P2 容易补）、图表、批注、数据验证、条件格式、外部链接。

### 5.4 SQLite

- 浏览器：`sqliteReader.worker.ts`，`sqlite3InitModule` 后 `allocFromTypedArray(bytes)` + `sqlite3_deserialize(db, 'main', p, n, n, SQLITE_DESERIALIZE_FREEONCLOSE | SQLITE_DESERIALIZE_READONLY)`，不用 OPFS；文件 ≤ 256 MiB。只执行 `SELECT`，列名用 `PRAGMA table_info`。
- 表选择：`sqlite_master` 中 `type='table'` 且名字不以 `sqlite_` 开头（视图默认不列，高级项可勾；Q10）；向导列出表名 + 行数，默认全选。
- 列映射：声明类型按 SQLite 亲和性 — `INT*` → `number`（有 \|v\| > 2^53−1 → `decimal` scale 0）；`REAL/FLOA/DOUB` → `number`（有小数 → `decimal`）；`NUMERIC/DECIMAL(p,s)` → `decimal` scale s；`BOOL*` → `boolean`（0/1）；`DATE` → `date`（文本 ISO 或整数按 unix 秒）；`DATETIME/TIMESTAMP` → `datetime`；`TEXT/CHAR/CLOB` 与无声明 → 按 §3.3 对值推断；`BLOB` → 跳过该列并报告。`NULL` → 未设置。
- CLI 宿主：Deno `node:sqlite` 的 `DatabaseSync`（只读打开）。

## 6. Currency 字段类型

### 6.1 定义

- `FieldType::Currency`，wire 名 `currency`。`FieldDef` 新增 `currency: String`（ISO 4217 三位大写字母，currency 字段必填，其他类型出现即 `INVALID_SCHEMA`），复用 `scale`（currency 上可选，缺省 2；0–18）。
- 值的规范形式：`{ "amount": "<规范 decimal 字串，scale 位小数>", "currency": "USD" }`。**币种总是显式落库**（Q1）：写入时没给币种就补字段默认币种，之后改字段默认币种不改变已有值。
- core 接受的输入：上述对象；或裸 decimal 字串 / JSON number（→ 默认币种）。**不接受** `"USD 12"`、`"$12"` 之类带符号文本——符号与别名解析是导入层和编辑器的事，core 保持“不做尽力转换”。
- 比较与排序：按 `(currency, amount)` 字典序（不同币种不换算，先按代码聚在一起）；筛选运算符与 `decimal` 相同（eq ne lt lte gt gte is_empty is_not_empty），`eq` 的右值可以是对象或裸数字（默认币种）；`sortable` 为真；`unique` 不支持。
- 迁移矩阵新增：`decimal → currency`（补默认币种）、`number → currency`、`currency → decimal`（丢币种，预检报告“N 个值的币种不是默认币种”）、`currency → text`（`"USD 12.00"`）。
- 公式（`$formula`）返回类型表、`filter.rs` 第 358 行附近的分组字段白名单：不加 currency（分组按币种没有意义）。

### 6.2 改动清单

| 层 | 文件 | 改什么 |
| --- | --- | --- |
| core | [value.rs](../src/frame/aiworkspace/core/src/value.rs) | 枚举、`FieldDef.currency`、`validate`、`normalize`（对象 / 裸数）、`cmp_currency`、`migrate_value` 分支 |
| core | [filter.rs](../src/frame/aiworkspace/core/src/filter.rs) | 运算符矩阵、比较、`is_empty` |
| core | [plan_table.rs](../src/frame/aiworkspace/core/src/plan_table.rs) | `migrate_field` 允许的目标、`title_field_id` 仍限 text |
| core | tests | 合法 / 非法输入表加 currency 行；迁移测试；排序 |
| wasm | 无改动（`normalize_value` 走 core） | 重新 `wasm/build.sh` |
| store | [results.rs](../src/frame/aiworkspace/store/src/wish/results.rs) | `FIELD_TYPES` 加 `currency`；`declared_fields` 接受 `{ type: 'currency', currency, scale }`；`infer_type` 认 `{ amount, currency }` 对象 |
| aiws | [aiws/README.md](../src/frame/aiworkspace/aiws/README.md) 第 46 行 | 字段类型列表与数组形式示例 |
| Desktop | [api/types.ts](../src/frame/desktop/src/app/aiworkspace/api/types.ts) `FieldType` / `FieldDef`；[values.ts](../src/frame/desktop/src/app/aiworkspace/ui/values.ts)（`formatValue` 用 `Intl.NumberFormat(locale, { style: 'currency', currency })`、`editText` 为 `1234.50` 或 `CNY 500`、`parseInput` 解析 `<code|符号|别名>? 数字 <code>?`、`FIELD_TYPE_LABEL`、`operatorsFor`、`SORTABLE`）；[fieldOps.ts](../src/frame/desktop/src/app/aiworkspace/ui/fieldOps.ts) `CREATABLE_FIELD_TYPES`、`addFieldOps` 带 `currency`；[FieldManager.tsx](../src/frame/desktop/src/app/aiworkspace/ui/FieldManager.tsx) 新建 / 编辑字段的币种选择（常用 30 种 + 手输）、`MIGRATIONS`；[ValueEditor.tsx](../src/frame/desktop/src/app/aiworkspace/ui/ValueEditor.tsx)；[TableViewCell.tsx](../src/frame/desktop/src/app/aiworkspace/ui/TableViewCell.tsx) 类型图标（lucide `Banknote`）；记录视图 / 属性面板同样显示 | |
| 文档 | 第一期内置对象详细设计 §3.4.2 表加一行；aiworkspace README “Format version”段注明 1.1 新增值类型；Desktop README | |

### 6.3 导入时的币种解析（`infer.ts`）

- 一个值 = 可选前缀标记 + 金额（允许千分位、负号、括号负数不做）+ 可选后缀标记；标记与金额间可有空格。
- 代码：任意 `[A-Z]{3}` 若在内置 ISO 4217 表里（USD EUR JPY GBP CNY HKD TWD KRW SGD AUD CAD CHF INR RUB BRL MXN THB VND MYR IDR PHP NZD SEK NOK DKK PLN TRY ZAR AED SAR CZK HUF ILS）；别名：`RMB 人民币 元 ￥元` → CNY，`美元 美金` → USD，`日元 円` → JPY，`欧元` → EUR，`英镑` → GBP，`港币 港元` → HKD，`新台币` → TWD，`韩元` → KRW。
- 符号无歧义：`€` EUR、`£` GBP、`₩` KRW、`₹` INR、`₫` VND、`₽` RUB、`฿` THB、`₺` TRY、`R$` BRL、`HK$` HKD、`NT$` TWD、`US$` USD、`A$` AUD、`C$` CAD、`S$` SGD、`CN¥` CNY、`JP¥` JPY。
- 符号有歧义（Q12）：`$` → 若列默认币种的符号是 `$`（USD/AUD/CAD/…）取默认币种，否则 USD；`¥ / ￥` → 若默认币种 ∈ {CNY, JPY} 取默认，否则界面语言 zh → CNY、其他 → JPY。列里出现歧义符号时向导该列显示“`$` 按 USD”“`¥` 按 CNY”下拉，可改。
- 列默认币种 = 显式币种中出现最多的；全是裸数字（只有来源 hint=currency，如 xlsx 货币格式无 lcid）→ 界面语言默认。
- 不做：汇率、币种统一、跨币种汇总（需求 §5.3）。

## 7. 向导（一屏）

```text
┌ 导入表格  <来源名>                                   共 1 234 行 × 8 列 ┐
│ [x] 首行是表头      表格标题 [销售明细        ]   位置：当前画布      │
│ ┌──────┬──────────┬──────────┬───────────┬──────────────┐          │
│ │ 包含 │ [x]      │ [x]      │ [ ]       │ [x]          │          │
│ │ 字段 │ 月份     │ 金额     │ 备注      │ 状态         │  ← 可编辑 │
│ │ 类型 │ 日期 ▾   │ 货币 ▾   │ 文本 ▾    │ 单选 ▾       │  ← 下拉   │
│ │      │ 自动     │ 自动·USD │ 自动      │ 自动·4 个选项│  ← 徽标   │
│ ├──────┼──────────┼──────────┼───────────┼──────────────┤          │
│ │ 1    │ 2026-07-01│ 10.50   │ …         │ 进行中       │  ← 前 8 行 │
│ └──────┴──────────┴──────────┴───────────┴──────────────┘          │
│ ⚠ “金额”按 货币 有 3 个值无法转换（样例 "N/A"）：(•) 留空 ( ) 整列改为文本 │
│ ▸ 高级：编码 / 分隔符 / 工作表 / 表 / 日期顺序 M/D / 时区 / 保留原文件 │
│                                              [取消]  [导入 1 234 行] │
└───────────────────────────────────────────────────────────────────────┘
```

- `data-testid`：`aiws-table-ask`（`-table` / `-file` 两个按钮）、`aiws-table-dialog`、`aiws-table-header`、`aiws-table-title`、`aiws-table-include-<i>`、`aiws-table-name-<i>`、`aiws-table-type-<i>`、`aiws-table-currency-<i>`、`aiws-table-invalid-<i>`、`aiws-table-policy-unset|text`、`aiws-table-advanced`、`aiws-table-encoding`、`aiws-table-delimiter`、`aiws-table-sheet`、`aiws-table-all-sheets`、`aiws-table-tables`、`aiws-table-date-order`、`aiws-table-timezone`、`aiws-table-keep`、`aiws-table-start`、`aiws-table-cancel`、`aiws-table-progress`、`aiws-table-error`。
- 推断在 Grid 就位后同步算（≤ 5 000 行全量；更多行只用前 5 000 行推断，提交时全量校验，不合法按策略处理并报告）。改类型 / 改表头后即时重算该列的不合法计数。
- 进度：> 2 000 行显示进度条，可取消（第一个 commit 之前取消 = 无痕）。
- 完成：`store.notify('info', …)`；有报告项时弹 `ImportReportDialog`（`outcome.kind = 'table'`）；画布上选中新 Block 并 `camera.fit`。
- 样式复用 `aiws-dialog aiws-pdf-dialog` 的类，少量新类放 `aiworkspace.css`；手机宽度下预览表横向滚动。

## 8. Agent 侧

- CLI：`buckyos aiws import-table <file> --workspace <ws> [--parent <folder_id>=data] [--title <t>] [--header auto|yes|no] [--sheet <name> | --all-sheets] [--table <name>...] [--date-order md|dm] [--timezone <iana>] [--dry-run]`。`--dry-run` 打印 TablePlan（字段、类型、行数、报告），否则提交并打印 `entity_id`。实现放 `import-table/tools/import.ts`（Deno 宿主：读文件、`node:sqlite`、WASM 校验可省略——服务端会校验，把拒绝原因原样打印），`aiws.ts` 里的 verb 调它。
- 手册：[aiworkspace_template_dev.md](../src/apps/jarvis_agent/agent/cmd_skills/aiworkspace_template_dev.md) “Inputs and outputs” 加一条：用户给了 CSV / xlsx / SQLite 时先 `import-table --dry-run` 看计划再导，不要手写推断；字段类型表加 `currency`。许愿格手册（aiws README 第 46 行）同步。
- 用户在聊天里发表格文件给 Agent 的链路（消息附件 → Agent 读文件 → 导入）复用此 CLI，不在本 TODO 内另做。

## 9. 分期任务

### T0 共享模块与实测（无 UI）

- [x] 新建 `import-table/`：`grid.ts text.ts csv.ts html.ts infer.ts plan.ts operations.ts mod.ts`；`deno test -A` 跑 `*_test.ts`；fixtures：`csv/`（UTF-8 BOM、gb18030、分号、引号换行、空行、无表头、超大整数、百分比、千分位、M/D 与 D/M 歧义、币种混合）、`clipboard/`（见 T1 第一项）、`expected/*.plan.json` 黄金计划（`TABLE_REGEN=1` 重生成，照 import-pdf）。
- [x] Vite 别名 `@table/*`、`tsconfig.app.json / tsconfig.node.json` paths、`server.fs.allow`；i18n 测试 `SOURCE_DIRS` 加目录。
- [x] 实测提交规模（standalone 已测，经网关见 §15）：standalone 与经网关各提交一次 20 000 行 × 20 列（文本 + 数字），记录请求体大小、耗时、是否被拒；据此定 §3.4 的批次常量并写进 `operations.ts` 注释。
- [x] `plan.ts` 输出能直接喂 `buildOperations` 风格的 `ImportOp[]`（或自带 `Operation[]`）；`ImportReport` 从 `@drawio/convert` 复用。

### T1 剪贴板导入（需求 §四）

- [ ] **先采集真实剪贴板**（未做：本机无 macOS / Windows 桌面应用；fixtures 按各应用的已知格式手写，见 §13）：在 macOS / Windows 上用一段 devtools 脚本把 Excel、Numbers、Google Sheets、Notion、Miro、网页 `<table>` 复制出的 `text/html` 与 `text/plain` 存进 `fixtures/table/clipboard/<来源>.html|.txt`，`html_test.ts` 以它们为用例（Miro 若不出 HTML/TSV 则记录其 MIME 并在 §10 风险里登记）。
- [x] `ui/import/`：请求槽、`TableImportDialog`（ask + 向导 + 进度）、`tableImport.ts`（Grid → plan → WASM 校验 → 分批 submit → `ImportOutcome`）、`clipboardTable.ts`。
- [x] `CanvasView.tsx` `onPaste` 接表格判定（顺序：自己的标记 → 文件 → 表格 → draw.io → SVG）；自由 / 流式页落点按 §3.5。
- [x] i18n 新区 `import.ts`（en + zh），`index.ts` parts 与 i18n 测试 `PARTS` 加 `{ name: 'import', prefixes: ['import'] }`。
- [x] e2e `tests/aiworkspace/table-import.spec.ts`：TI-01～TI-04（§11）。

### T2 CSV 双入口与文件分流（需求 §6.1）

- [x] `classifyFile` 加 `tabular`（csv / tsv / xlsx / sqlite 子类 + 魔数）；`handleFiles`、`InsertCatalog` asset 项、`DataTree` 导入文件三处对单个 tabular 文件走 ask。
- [x] 入口二三处：`InsertCatalog` table 项“从文件导入…”、主菜单“导入表格数据…”、`DataTree` “导入表格文件”；`CanvasView` 加 `tableInput`（accept 列表）。
- [x] 高级项：编码、分隔符；保留原文件（默认 CSV 关）→ `uploadAsset` + `buckyos.asset-ref`。
- [x] 数据树模式粘贴（若 T1 未做）。
- [x] e2e TI-05～TI-07。

### T3 Currency（需求 §五）

- [x] core 改动（§6.2）+ 测试；`wasm/build.sh`；`results.rs`；aiws README。
- [x] Desktop 类型、显示、编辑、字段管理、筛选、排序、图标；记录视图与属性面板。
- [x] 导入层 §6.3 解析与列默认币种；向导的币种下拉与歧义符号选择。
- [x] 设计文档 §3.4.2、aiworkspace README、Desktop README 回写。
- [x] e2e TI-08；core 测试覆盖 §6.1 全部规则。

### T4 xlsx（需求 §6.2，Q3）

- [x] `xlsx.ts`（§5.3）+ fixtures（用 Python openpyxl 或手写 XML 生成：共享串、内联串、公式缓存、日期 1900/1904、货币 / 百分比 / 日期格式、合并单元格、隐藏表、多表）+ 测试。
- [x] 向导“工作表 / 每个工作表一张表”；多表落点。
- [x] `.xls / .numbers / .ods` 的友好拒绝文案。
- [x] e2e TI-09。

### T5 SQLite（需求 §6.4）

- [x] `sqlite.ts`（注入 exec）、`sqliteReader.worker.ts`、向导“表”多选（行数、视图开关）、列映射 §5.4。
- [x] fixtures：Python `sqlite3` 生成含 INTEGER / REAL / NUMERIC(10,2) / TEXT 日期 / BOOLEAN / BLOB / NULL / 视图 的库。
- [x] e2e TI-10。

### T6 aiws 内部复制与 Agent 工具

- [x] `TableViewCell` 工具栏“复制为表格”（TSV + 带 `data-aiws-table` 的 HTML）；粘贴时预填类型与选项。
- [x] `buckyos aiws import-table` + `tools/import.ts` + 手册两处。
- [x] 文档回写：本文件 §12 实施回写；aiworkspace README “File and folder imports” 加“表格数据导入”段；Desktop README 结构表加 `ui/import/`。

## 10. 待裁决（建议值已写入上文，裁决不同再改）

**裁决记录（2026-10-29）：** Q2、Q3 按建议定（不 bump 格式 / 协议版本，README 注明 1.1 新增 currency 值类型；xlsx 自研读取器，`.xls / .numbers / .ods` 本轮不做）。其余按建议值执行，未另行裁决。

| # | 问题 | 建议 |
| --- | --- | --- |
| Q1 | Currency 值是否总是显式落库币种，还是允许“无币种 = 继承字段默认” | 总是显式（排序 / 筛选 / 导出 / 跨表引用都不用再查字段定义；改默认币种不会悄悄改旧值） |
| Q2 | 新增值类型算不算格式 1.0 内“旧实现可忽略的新增”（D07） | 不 bump FORMAT / schema_version，PROTOCOL 保持 1.1，README 注明 “1.1 新增 currency 值类型”。理由：core / WASM / 服务同步发布，唯一的旧读者是 Agent 脚本（读 JSON 不校验）；若坚持严格 D07，则含 currency 字段的表 `schema_version = 2` 让旧端 degraded，但现有 `supports_schema_version` 只认 1 且无改版本的操作，代价大 |
| Q3 | xlsx 自研读取器 vs SheetJS | 自研（zip + XML 子集，~600 行，无依赖、Deno 可跑、与 import-drawio/pdf 同一风格）；SheetJS CE 在 npm 上停在 0.18.5 且需从其 CDN 装 tarball。`.xls / .numbers / .ods` 本轮不做 |
| Q4 | 都 ≤ 12 时日期顺序默认 | M/D（Excel en-US 习惯；中文 Excel 默认 `yyyy/m/d` 不产生歧义），向导高级项可改 |
| Q5 | select 自动推断是否默认开（≤ 20 个、≤ 行数/2、行数 ≥ 5） | 开；Excel 状态列是高频场景，误判时一键改回文本 |
| Q6 | 规模：20 000 行/commit、总 100 000 行 / 1 000 000 单元格 | 采用，T0 实测后微调 |
| Q7 | 保留原文件默认 | CSV 关（内容已是表）、xlsx / SQLite 开（可能有未导的工作表 / 表），> 32 MiB 禁用 |
| Q8 | 来源是 aiws 自己（带标记）时是否跳过向导 | 不跳，预填后一键确认（需求要求有向导；也给改标题 / 落点的机会） |
| Q9 | record_id 是否沿用来源主键 | 不沿用，`randomId('r')`；再导入对齐留以后 |
| Q10 | SQLite 视图、BLOB 列 | 视图默认不列（高级项可勾）；BLOB 列跳过并报告 |
| Q11 | 空单元格 | 未设置（unset）；不写 `""`、不写 null、不设 nullable |
| Q12 | `$` / `¥` 歧义默认 | §6.3 规则（默认币种符号一致→默认；否则 `$`→USD，`¥`→zh 界面 CNY / 其他 JPY），列级下拉可改 |

## 11. 验收（Desktop e2e `tests/aiworkspace/table-import.spec.ts`，截图到 `test-results/aiworkspace-table/`）

| 用例 | 场景 | 预期 |
| --- | --- | --- |
| TI-01 | 粘贴 Excel 风格 HTML（fixture，含 `x:num`、`$1,234.50`、日期、是/否） | 向导出现；表头、类型（text / currency USD / date / boolean）自动正确；直接点导入 → 画布出现 table Block，`doc.query` 行数与值一致 |
| TI-02 | 粘贴纯 TSV 无表头 | 表头开关默认关，字段名 `列 A…`；打开开关后首行变字段名 |
| TI-03 | 粘贴后把一列从“数字”改成“日期” | 显示 N 个不合法；默认留空；报告列出样例；提交后这些格未设置 |
| TI-04 | 流式页粘贴 | Block 插在当前节点后；撤销一步整表消失 |
| TI-05 | 拖一个 CSV 到画布 | ask 对话框；“导入为表格”建表；“作为文件插入”建 asset |
| TI-06 | 插入目录 table 项 → 从文件导入 → 选 gb18030 分号 CSV | 编码 / 分隔符自动正确；中文表头无乱码 |
| TI-07 | 主菜单“导入表格数据…”选 CSV，勾保留原文件 | 表 + asset-ref 同在内容文件夹 |
| TI-08 | 货币列：`200`、`RMB500`、`¥300`、`€10` 字段默认 USD | 值分别为 USD 200.00 / CNY 500.00 / CNY 或 JPY 300.00（按 Q12 与下拉）/ EUR 10.00；排序按 (币种, 金额)；筛选 `> 100` 命中默认币种值 |
| TI-09 | xlsx 两个工作表（日期序列号、货币格式 `[$¥-804]`、百分比、公式、合并表头） | 默认导第一张；勾“每个工作表一张表”得两张；类型 date / currency CNY / decimal / 公式按缓存值；合并表头补名并报告 |
| TI-10 | SQLite 三张表 + 一视图 + BLOB 列 | 列出三张表，默认全选；BLOB 列跳过并报告；NUMERIC(10,2) → decimal scale 2 |
| TI-11 | 20 001 行 CSV | 两个 commit，进度条；中途取消（第一个 commit 后）→ 报告说明导入到第几行 |
| TI-12 | aiws 表格“复制为表格”再粘贴到另一张画布 | 向导预填类型与单选选项，确认后值与选项标签一致 |

另：core `cargo test -p aiworkspace-core` 覆盖 Currency 全部规则；`deno test -A`（import-table）覆盖解析与推断黄金用例；`aiworkspace-i18n.test.ts` 通过；真实 Zone DV 跑 TI-01 / TI-05 / TI-08 各一次（iPhone 上粘贴 Numbers 表格一次，记录结果）。

## 12. 不做与风险

- 不做：`.xls` / `.numbers` / `.ods`、Notion / Miro 原生文件、汇率与币种统一、多层表头智能识别、再导入对齐（增量更新已有表）、`multi_select` / `object_ref` 自动推断、公式求值、单元格样式。
- 风险：Miro 剪贴板可能只有自有 MIME（T1 第一项实测后决定是否只接 `text/plain`）；Numbers 复制出的 HTML 不带原始值，百分比 / 货币只能靠显示文本解析；Safari 的 `clipboardData.getData('text/html')` 在某些来源为空（回退 TSV）；大文件在手机上内存紧张（xlsx / SQLite 解析放 Worker，超限提示）；网关请求体上限未知（T0 实测）。
- 实施回写：T0–T6 完成后在此追加“实施偏差”与“DV 记录”两节（照 draw.io / PDF TODO 的 §8–§10 写法）。

## 13. 实施偏差（2026-10-30）

按 §10 建议值实施；以下是与上文不同或上文没写到的地方。

1. **提交按行数和字节双重切分**（T0 实测推翻“瓶颈未知”）：standalone 调试版提交 20 000 行 × 20 列（10 列 ~14 字符文本、5 列整数、5 列两位小数）请求 7.82 MiB、8.9 s 接受；12.4 MB 的请求被 `413 length limit exceeded` 拒绝——服务的 `Limits.max_commit_bytes` 缺省 8 MiB（`server/src/lib.rs` `DefaultBodyLimit`）。所以 `operations.ts` 一个 commit 在 20 000 行或操作 JSON 达到 6 MiB（UTF-8）时结束，宽表/长文本会比 20 000 行更早切分；`COMMIT_BYTES` 写在文件头注释里。
2. **record_id 不是纯随机**：`r<一次导入一个随机前缀><行号定宽 base36>`。表格不排序时按 record_id 列出记录，纯 `randomId('r')` 会打乱源文件行序；现在默认顺序就是源文件顺序（TI-11 断言前三行 0/1/2）。Q9“不沿用来源主键”不变。
3. **Currency 筛选的比较运算只命中同币种**：§6.1 只写了“按 (currency, amount) 字典序”；若比较运算也按字典序，`> 100`（默认 USD）会命中所有代码大于 USD 的币种（VND…）。`filter.rs` 对 `lt/lte/gt/gte` 先比币种代码，不同即不命中；排序仍按 (代码, 金额)。`in` 也允许（与 decimal 一致）。`currency → currency`（改 scale）是新增的迁移；改默认币种走 `table.update_field { changes: { currency } }`（plan_table 新增可改键）。
4. **许愿格结果也能出 currency**：`results.rs` `FIELD_TYPES` / `infer_type`（`{ amount, currency }` 对象）/ `check_value`，`plan.rs` `store_value` / `field_plans` / 记录属性；结果字段默认币种 = 声明的 > 值里最多的代码 > USD。
5. **SQLite 与 xlsx 共用一个 Worker**：`ui/import/tableReader.worker.ts`（不是 §2.1 的 `sqliteReader.worker.ts`），xlsx 的 XML 解析也不占主线程。
6. **TEXT 类 SQLite 列按值推断**（§5.4 原样）；但“标题列不参与单选”规则对 SQLite 同样生效，所以 `orders.status`（`id/product_id/qty` 是数字、`placed` 是日期时间）作为首个文本列保持文本。
7. **Agent 命令落在 `src/tools/buckyos-agent/commands/import_table.ts`，不是 websdk 的 `buckyos aiws import-table`**：websdk CLI 构建时拿不到 buckyos 仓库的共享模块，按 AGENTS.md“Agent 专用的配方类工具放 `src/tools/buckyos-agent`”。命令经 deno.json 的 `@table/` 引用 `src/frame/aiworkspace/import-table/`；`buckyos-build.py` `_prepare_agent_tools`（宿主 `libexec/buckyos-agent`）与 `build_aios`（镜像）把 `import-table`、`import-drawio` 复制为命令旁的 `vendor/frame/aiworkspace/…` 并改写 import。参数同 §8，另有 `--views --encoding --delimiter --locale`；Deno 宿主逻辑在 `import-table/tools/import.ts`（单独运行即 dry-run）。
8. **剪贴板 fixtures 是手写的**：按 Excel（`x:num`、类样式 `mso-number-format`、`<!--StartFragment-->`、无引号属性）、Google Sheets（`data-sheets-value` / `data-sheets-numberformat`）、Numbers（只有显示文本、`<p>` 包裹、`<br>`）、Notion（`<thead>/<th>`）、网页（`rowspan/colspan`）的已知格式写成，没有在 macOS / Windows 上实采；Miro 未覆盖。
9. **xlsx 读取器的边界**：只读 `t=s/str/inlineStr/b/d/e/n`；纯时刻格式（`h:mm`）按文本导入并报告 `table_time_only`；`[$¥-804]` 这类按 lcid 定币种，无 lcid 的单独符号按 §6.3 歧义规则；合并单元格把被覆盖的格标成 `covered`，表头判定把它当“有名”（否则合并表头会让首行不满足“全部非空”）。
10. **向导细节**：不合法计数只对用户改过类型/scale/选项的列做（推断出的类型按构造无损），用 WASM `normalize_value`；提交时 `planTable` 对全部值调 WASM（同列同值缓存）。日期顺序与时区对所有工作表/表生效；多张表时上方是表的标签页。`data-testid` 按 §7，`aiws-table-policy-unset|text` 带列号后缀（`-<i>`），另有 `aiws-table-ask-table|file`、`aiws-table-size`、`aiws-table-dollar-<i>|yen-<i>`、`aiws-table-table-<name>`、`aiws-table-views`、`aiws-table-problem`、`aiws-near-table-copy`、`aiws-table-copy-<cell>`。
11. **“复制为表格”的入口**：表格视图条（`aiws-table-copy-<cell>`）和画布 Block 的近工具栏（builtin `table-copy`，经 intent 交给编辑器执行，等字段读到再复制）。剪贴板用 `ClipboardItem` 的 Promise 值写入（保住点击的用户激活）。`object_ref` 列按文本复制。
12. **入口**：画布粘贴（自己的标记 → 文件 → 表格 → draw.io → SVG）、拖放/粘贴文件（`classifyFile` 新增 `tabular`，单个问“导入为表格 / 作为文件插入”）、插入目录 file 项选表格文件（同样先问）、插入目录 table 项“从文件导入…”、主菜单“插入 → 导入表格数据…”、数据树“导入表格文件”与“导入文件”选单个表格文件（先问）、数据树模式粘贴。主菜单只在“插入”子菜单里加项（顶层不加，小窗口放不下）。

## 14. 验证（2026-10-30）

- `cargo test -p aiworkspace-core -p aiworkspace-store -p aiworkspace`：46 个测试二进制、224 个用例全部通过；新增 core `tests/currency.rs`（3）、`value.rs` 的 `currency` / `currency_migrate`、store `wish::plan::tests::currency_results` 与 results 推断断言。
- `cd src/frame/aiworkspace/import-table && deno test -A`：33 个（14 个 CSV、6 个剪贴板、xlsx 4 张表、SQLite 3 张表的黄金计划 + 规则用例）；import-drawio 34、import-pdf 17、controls 20 照旧通过。
- `uv run python -m unittest tools.tests.test_buckyos_build_entrypoint`：13 个通过（`_prepare_agent_tools` 的 vendor 复制与 import 改写）。
- Desktop：`tsc -p tsconfig.app.json`、`eslint src/app/aiworkspace` 干净；`deno test -A tests/datamodel/aiworkspace-i18n.test.ts` 新增 `import` 分区通过，仍有两项既有失败（`canvas.catalog.sample/extension` 未用、`outline.rs` 的“字”）。
- Desktop aiworkspace 全量 e2e（221 例，29 分钟）：218 通过；失败 3 例 —— CN17（`connectors.spec.ts:288`）与 `i18n.spec.ts:133` 是既有失败（未改动的 HEAD 上同样失败，见历次记录），wish T3-4（本地执行器恢复）单独复跑通过，属时序偶发。
- e2e `tests/aiworkspace/table-import.spec.ts` TI-01～TI-12 全部通过（`AIWS_BIN=/tmp/dev-cache-root/cargo-target/debug/aiworkspace NFS_BIN=…/nfs_server pnpm exec playwright test --config=playwright.aiworkspace.config.ts tests/aiworkspace/table-import.spec.ts`），截图在 `test-results/aiworkspace-table/`。
- Agent 命令：`import_table <xlsx|sqlite|csv> --dry-run` 在仓库与宿主副本（`rootfs/libexec/buckyos-agent`）都能出计划；`commitPlans` 对 standalone 服务提交 xlsx 两张表、SQLite 三张表、货币 CSV，重名得到 `select (2)`。

## 15. DV 记录（2026-10-30，本机 test.buckyos.io）

- 部署：`uv run buckyos-build.py -s aiworkspace desktop`，只替换 `/opt/buckyos/bin/aiworkspace/aiworkspace`（原子 mv 后杀进程，node-daemon 拉起新进程）、`/opt/buckyos/bin/control-panel/web`、`/opt/buckyos/libexec/buckyos-agent`；原文件备份在 `/home/lzc/project/deployment-backups/20261030-table-import/`。
- 浏览器：`BUCKYOS_UI_DV_BASE_URL=https://test.buckyos.io BUCKYOS_TEST_ADMIN_PASSWORD=bucky2025 npx playwright test --config=playwright.dv.config.ts tests/dv/aiworkspace.table.spec.ts`，5 例全部通过：TI-01（Excel HTML 粘贴：text / currency USD / date / boolean，值逐格正确，Block 显示 `1,234.50`）、TI-05（拖入 CSV：先问，表格 / 文件各一次）、TI-08（货币：默认改 USD、`¥` 按 CNY，排序 c,b,d,a，`> 100` 只命中 USD 200）、xlsx（Worker 读取、每个工作表一张表、原文件经 NDM 上传成 asset-ref）、20 000 行 × 20 列。首轮有 2 例在 `browser.newContext` 时 Chromium SEGV（未进入页面，环境问题），单独复跑通过。
- 经网关的 T0：20 000 × 20（CSV 4.55 MB）切成两个 commit——15 415 行 / 6.00 MiB 与 4 585 行 / 1.85 MiB——都被网关和服务接受；从点“导入”到对话框关闭 7.1 s（release 服务，含浏览器内 WASM 逐值校验，规划本身约 0.3 s）。6 MiB 的请求经网关通过；6–8 MiB 之间经网关未测。
- Agent 命令：宿主副本 `/opt/buckyos/libexec/buckyos-agent`，用注入会话（`BUCKYOS_APPCLIENT_SESSION_TOKEN` = devtest 桌面登录令牌，`BUCKYOS_HOST_GATEWAY=127.0.0.1`）运行 `import_table workbook.xlsx --workspace ws_dbqdy3zs2p4ekauuxqsdvqkvtr --all-sheets --locale zh`：两张表提交成功，Revenue 为 `currency CNY`、Rate 为比例 `0.125`、Total 为公式缓存值。该工作区留在 Zone 上。本机没有 devtest 的 AppClient 私钥，不注入令牌时 `initRuntime` 找不到私钥（与本功能无关）。
- 未在 DV 上跑：aios 容器内的 `import_table`（镜像未重建，`build_aios` 的 vendor 复制只在脚本层面检查过）、iPhone 粘贴 Numbers。

## 16. 未做

- 真实剪贴板采集（§9 T1 第一项）与 Miro；iPhone 上粘贴 Numbers 表格（§11 末）。
- `multi_select` / `object_ref` 自动推断、`.xls / .numbers / .ods`（§12 原样不做）。
- websdk 的 `buckyos aiws import-table`（见 §13.7）。
