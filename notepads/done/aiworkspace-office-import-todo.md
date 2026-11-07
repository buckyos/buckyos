# AI Workspace Office 文档导入（pptx / docx）TODO

日期：2026-10-31（2026-11-01 用户裁决 Q1–Q14，见 §11；同日 O0–O6 已实施，实施回写见 §12–§15）
来源：2026-10-31 与用户讨论"aiws 支持导入 pptx / docx，开源基础设施成熟度"的结论；本文把结论落成实现思路与任务单。
基线：当前工作区（draw.io / PDF / 表格导入、流式页 Notion 化、脑图、内核 M0–M5、nfs_server 预览管线 `repr` 均已实施）。路径相对 `buckyos/`。

目标：用户把 `.pptx` / `.docx` 拖进 Workspace、粘贴文件，或从"导入"菜单选择后，得到**可编辑的 Workspace 内容**：

- pptx → 每张幻灯片一张画布页（Frame + text / shape / asset 对象，与 PDF 导入同一产物形态，放映即幻灯片）；
- docx → 一张流式页（一个 richtext Block，图片为块级 embed 的 asset 实体）；
- 在装有 LibreOffice 的环境额外提供**"原样导入"（高保真）**：服务端转 PDF，再走现有 import-pdf 拆成可编辑对象；Preview 组件顺带获得 Office 文件预览。

三条路线分期，纯前端路线不依赖任何外部进程；LibreOffice 路线按"有则使用、没有就失败"，不隐式下载安装。`.xlsx` 已由表格导入覆盖，不在本文范围。

## 0. Review 结论（开源现状 ↔ 现有实现）

1. **开源现状（10-31 评估）**：浏览器端没有生产级的"pptx → 可编辑模型"解析器。pptxtojson（MIT，PPTist 作者）是唯一同类，作者自述"样式与源文件仍有差异，不建议直接用于生产"，覆盖常见商务 PPT 约七八成，缺口在 placeholder 的 layout→master 继承、主题色 lumMod/lumOff、文本 autofit、渐变/效果、EMF/WMF；PPTist 本体 AGPL 只能参考不能拷。docx 端 mammoth.js（BSD）成熟但产物是 HTML 且刻意丢版式，pandoc 单二进制质量更高但也只保结构。版式引擎只有 LibreOffice / ONLYOFFICE x2t（C++，装完 400 MB–1 GB，保真度取决于字体）。**结论：可编辑路线自写 OOXML 解析（复用已有 zip / XML / 几何基础设施，产物直接是 aiws 模型，浏览器与 Deno 同一份代码）；高保真路线借 LibreOffice。**
2. **基础设施已经齐了，新模块只写"OOXML → aiws 模型"的映射：**
   - zip：[xlsx.ts](../../src/frame/aiworkspace/import-table/xlsx.ts) `isZip / zipEntries / zipRead`（stored + deflate-raw，`DecompressionStream`，浏览器与 Deno 同源）；
   - XML：[xml.ts](../../src/frame/aiworkspace/import-drawio/xml.ts) `parseXml / elements / findAll / textOf`（命名空间前缀按字面匹配，OOXML 的前缀固定 `w: a: p: r:`，够用）；
   - 几何：[stencil.ts](../../src/frame/aiworkspace/import-drawio/stencil.ts) 已经是一个"guide 公式 → 路径"的解释器，DrawingML `prstGeom` 的 187 个预设几何同样由 guide 公式（`avLst / gdLst / pathLst`）定义，规范附带的 `presetShapeDefinitions.xml` 可以像 [builtinSvg.ts](../../src/frame/aiworkspace/import-drawio/builtinSvg.ts) 那样预编译成表；
   - 产物与提交：[operations.ts](../../src/frame/aiworkspace/import-drawio/operations.ts) `PagePlan / PlanItem / buildOperations / ImportReport / batches`，`PlanItem` 已带 `rotation`，shape config 已有 `text / font_size / bold / italic / font_color / align / valign / line_height / font_family`（[convert.ts](../../src/frame/aiworkspace/import-drawio/convert.ts) 第 223–236 行、import-pdf 新增的两个键）；
   - 页布局：[layout.ts](../../src/frame/aiworkspace/import-pdf/layout.ts) `pageScale / gridOrigins / stageOf`（长边 1920，页网格）。
3. **pptx 产物形态 = import-pdf 的页**：group + 覆盖整页的 Frame（`presentation.background` 页底色，[import-pdf/convert.ts](../../src/frame/aiworkspace/import-pdf/convert.ts) 第 144–145 行）+ 对象按绘制顺序；`pdf_source` 这类来源标记换成 `office_source`。放映、缩略图、页名（取幻灯片标题占位符）全部沿用。
4. **docx 产物形态 = 流式页 + richtext**：`createSurfaceOps(store, title, 'flow')`（[surfaceOps.ts](../../src/frame/desktop/src/app/aiworkspace/ui/canvas/surfaceOps.ts) 第 76 行）+ `buckyos.richtext` 实体，`payload.content` 是 ProseMirror JSON AST（[builtin.tsx](../../src/frame/desktop/src/app/aiworkspace/ui/blocks/builtin.tsx) 第 173–177 行）。节点集由 core 定义并校验（[richtext.rs](../../src/frame/aiworkspace/core/src/richtext.rs)）：`paragraph heading bullet_list ordered_list list_item blockquote code_block horizontal_rule table table_row table_cell object_embed object_link hard_break`，marks `strong em strike link`；限额 `Limits { max_blocks, max_text_utf16, max_depth, max_update_bytes }`（具体数值 O0 核实）。
5. **富文本没有行内图片**：图片只能是块级 `object_embed`（`attrs.ref.entity_id` 指向一个 `buckyos.asset-ref` 实体）。core 的 Markdown → richtext（[markdown.rs](../../src/frame/aiworkspace/core/src/markdown.rs) 第 270 行）把 `![]()` 当链接处理、不产 embed，所以 **docx 不能以 Markdown 作中间格式**，要直接产 richtext AST；Word 的行内图片提升为所在段落之后的块级 embed 并计入报告。
6. **服务端 Office → PDF 的位置是 nfs_server 预览管线**：它已经有"外部工具有则用"的全部机制——[tools.rs](../../src/frame/nfs_server/src/preview/tools.rs) 的三平台探测/版本/`hello` 广告、`preview_works` 记录与 `work_key` 缓存、`preview-cache` 桶与 `bkt_` 令牌、并发/超时/负缓存/Retry CAS（[mod.rs](../../src/frame/nfs_server/src/preview/mod.rs) 头注释）。只需加一个工具 `soffice`、一个 `Kind::Document`、一个 `Profile::PreviewDocument`（产物 `application/pdf`）。放在 aiws 服务里另起转换器要重做这一整套记录/缓存/鉴权，不值。顺带收益：Preview 组件的 runtime 已接受 `pdf`（[mediaTypes.ts](../../src/frame/desktop/src/components/preview/mediaTypes.ts) 第 306 行 `acceptTypes`、`pdfInline`），docx/pptx/xlsx/odt 附件在 MessageHub / HomeStation / 文件浏览器里也能预览。
7. **Lite 版同样适用**：nfs_server 在 Lite 版是原生进程，用户机器上装了 LibreOffice 即可用；三平台路径与"有则用、无则失败"按既定原则（Agent 标准环境 CLI 工具全平台，不隐式下载安装；见 nfs_server README「预览管线」的工具节）。
8. **入口样板齐全**：`classifyFile`（[drawioImport.ts](../../src/frame/desktop/src/app/aiworkspace/ui/canvas/drawioImport.ts) 第 174 行）加 `office` 类；PDF 的 ask 步（插入为文件 / 导入为页面）与请求槽 [pdfImportRequest.ts](../../src/frame/desktop/src/app/aiworkspace/ui/canvas/pdfImportRequest.ts)；[TableImportDialog.tsx](../../src/frame/desktop/src/app/aiworkspace/ui/import/TableImportDialog.tsx) 挂在 WorkspaceShell 层（数据树模式也能开）——Office 对话框同挂 WorkspaceShell；文件 Block 的"展开为可编辑页面"（PDF 已有）。
9. **文件嗅探冲突**：docx / pptx / xlsx 都是 zip。`tabularKind`（[files.ts](../../src/frame/aiworkspace/import-table/files.ts)）已认 xlsx；office 判定看 `[Content_Types].xml` 的主部件类型或 `word/document.xml` / `ppt/presentation.xml` 是否存在。`classifyFile` 顺序：tabular → office → other。Preview 的 `mediaTypes.ts` 第 212 行已有"zip 容器常是 Office 文档"的提示逻辑，可共用判定。
10. **Agent 侧先例**：[import_table.ts](../../src/tools/buckyos-agent/commands/import_table.ts)（Deno 宿主，共享模块由 `buckyos-build.py _prepare_agent_tools` 复制到 `vendor/frame/aiworkspace/` 并改写 deno.json 别名）。Office 可编辑路线不需要像素（EMF/WMF 转换除外），Deno 宿主可行；LibreOffice 路线进 Agent 要 import-pdf 在 Deno 渲染底图（@napi-rs/canvas，PDF 导入 P1 未做），本文列为 P2。
11. **限额**：一次 commit ≤ 8 MiB（`Limits.max_commit_bytes`）且 ≤ 10 000 操作，PDF 用 2 000 操作/批；资产 ≤ 32 MiB（`ASSET_LIMIT`）；SVG 大小上限沿 import-pdf `vector.ts`。

## 1. 现状对照

| 范围 | 已有 | 缺口 |
| --- | --- | --- |
| 文件分流 | `classifyFile`：drawio / svg / image / pdf / tabular / other | 没有 office 类；docx/pptx 落为 asset Block（文件） |
| 画布页导入 | import-pdf（页 → PagePlan → buildOperations，底图栅格化回退，页网格布局，书签作页名） | 没有从 OOXML 产 PagePlan 的转换器 |
| 流式页导入 | 脑图从 Markdown 大纲 / OPML（`importOutline`）；富文本编辑器粘贴 Markdown | 没有"从文件建流式页 + richtext"的导入器；没有 richtext AST 的程序化构造工具 |
| 几何 | mxGraph stencil 解释器、内置 SVG 表、pdf 路径 → rect/line/ellipse/svg | 没有 DrawingML prstGeom 表、custGeom 解释 |
| 颜色/主题 | draw.io 颜色解析（`style.ts`） | 没有 OOXML 主题（`a:clrScheme / fontScheme`）、`schemeClr` 修饰符（lumMod/lumOff/tint/shade/alpha）换算 |
| 图片 | 上传资产（`uploadAsset`）、PNG/JPEG 直接用；PDF 图片经 canvas 重采样 | EMF/WMF 无转换器；pptx `srcRect` 裁剪要 canvas |
| 服务端转换 | nfs_server `repr`：ffmpeg/ffprobe 探测、档位 `preview-image / preview-video / preview-audio / thumbnail`、`hello.repr.profiles` | 没有 `soffice` 探测、`Kind::Document`、PDF 档位；`repr` 只在 ffmpeg+ffprobe 都在时广告（`Probe::Missing`） |
| Preview 组件 | runtime 接受 pdf；nfspProvider 读 `reprCapability`；`profile_for` 只映射图/视/音 | Office MIME 没有 renderer，显示为不可预览 |
| websdk | `NfspClient.repr / getRepr`，`ReprCapability.profiles: string[]` | 不用改 wire；`purpose` 仍是 `preview | thumbnail` |
| Agent | `import_table`；`aiws import`（Markdown/OPML → 脑图） | 没有 `import_office` |
| i18n | `aiworkspace.pdf.* / aiworkspace.import.*` | 新文本区 `aiworkspace.office.*` |

## 2. 总体方案

```text
                      ┌─ 可编辑路线（纯前端，浏览器 / Deno 同一份代码，无依赖）──────────────────────────┐
.docx ──zip+XML──▶ docx.ts ──▶ DocPlan { title, richtext AST, assets[] } ──▶ 流式页 + richtext 实体 + asset-ref 实体
.pptx ──zip+XML──▶ pptx.ts ──▶ PagePlan[]（每页 group+Frame+对象）────▶ buildOperations ──▶ 画布页（同 PDF 导入）
                      └──────────────────────────────────────────────────────────────────────────────────┘
                      ┌─ 原样路线（后端转换 + 前端拆对象，LibreOffice 有则用）──────────────────────────────┐
.docx/.pptx/.doc/.ppt/.odt/.odp ──上传为资产──▶ nfs_server repr{purpose:'preview'} ──soffice──▶ PDF（preview-cache）
                                              ──▶ Desktop 取 read_url 字节 ──▶ 现有 import-pdf ──▶ 画布页
                      └──────────────────────────────────────────────────────────────────────────────────┘
```

- **可编辑路线是主线**：没有任何外部进程，Lite 版、浏览器、Agent 都一样；覆盖"文字 + 图片 + 基本形状 + 表格"的常见文档，图表 / SmartArt / OLE / EMF 留占位框并写进报告。
- **原样路线是补充**：转换在 nfs_server（"有则用"），导入在前端（现有 import-pdf 零改动）。它同时覆盖旧二进制格式（.doc/.ppt）与 ODF，可编辑路线不做这些。
- **宿主只做"需要环境的事"**：读文件、上传资产、canvas 裁剪 / 重采样、`repr` 轮询、提交、进度；共享模块纯函数、金样测试。
- **core 不动**：不加实体类型、不加操作；richtext AST 按现有 schema 构造，提交前用 WASM 校验（O0 确认暴露的校验入口）。

### 2.1 模块与落点

```text
src/frame/aiworkspace/import-office/           共享 TS 模块（Desktop 别名 @office/*；复用 ../import-table/xlsx.ts 的 zip、../import-drawio 的 xml/operations/svg）
  package.ts     OPC 包：[Content_Types].xml、_rels/.rels、部件 rels 解析（r:id → 目标部件 / 外链）、主部件定位、文件类型判定（docx | pptx | 其他）
  units.ts       EMU / 二十分之一磅 / 半磅 → 世界单位；pptx 页缩放沿 import-pdf layout
  theme.ts       a:theme：clrScheme（12 色 + 映射 clrMap）、fontScheme（major/minor latin/ea）；颜色求值：srgbClr / schemeClr / sysClr / prstClr + lumMod/lumOff/tint/shade/alpha（HSL 算法按 Office）
  text.ts        a:txBody / w:p 的段落与 run 模型（公共：字号、粗斜删、颜色、对齐、项目符号、超链接）
  docx.ts        WordprocessingML → richtext AST（styles.xml 样式链、numbering.xml 列表、表格、图片、脚注、修订）→ DocPlan
  geometry.ts    prstGeom 预设几何表（由 tools/presets.ts 从 presetShapeDefinitions.xml 预编译）+ guide 公式解释器 + custGeom → SVG 路径
  pptx.ts        PresentationML：slide/layout/master 继承、占位符解析、sp/pic/grpSp/graphicFrame/cxnSp → PlanItem；页背景；隐藏页 → PagePlan[]
  plan.ts        DocPlan / OfficePlan 类型；资产去重（按内容 SHA-256）、占位框回退、ImportReport 条目
  subdoc.ts      OPC 子包：把一个不可解析对象（图表 / SmartArt / OLE / EMF 图片 / 公式）连同可达部件抽成只含它的最小 docx / pptx（§5.3），供 soffice 栅格化成图片
  operations.ts  DocPlan → 流式页操作（surface + richtext + asset-ref）；PagePlan 直接走 @drawio/operations
  mod.ts         公开入口；docx_test.ts / pptx_test.ts / geometry_test.ts（金样）；tools/presets.ts（预编译）、tools/import.ts（Deno 宿主）
src/frame/aiworkspace/fixtures/office/         docx/ pptx/ expected/*.plan.json；src/make_*.py（python-docx / python-pptx 生成）+ 若干真实 Office / LibreOffice 保存件
src/frame/desktop/src/app/aiworkspace/ui/import/
  officeImportRequest.ts   请求槽（照 pdfImportRequest）：source、step 'ask' | 'options'、mode 'editable' | 'faithful'、finish
  ImportOfficeDialog.tsx   ask 步 + 选项 + 进度 + 报告；挂 WorkspaceShell（同 TableImportDialog）
  officeImport.ts          宿主：解析（Worker）→ 不可解析对象栅格化（§5.3，soffice 有则用）→ 资产上传（含 canvas 裁剪 / 重采样）→ 分批提交 → ImportOutcome；faithful 模式：上传原件 → repr → 轮询 → PDF 字节 → pdfImports
  officeReader.worker.ts   zip + XML 解析放 Worker（大文件不卡 UI）
src/frame/desktop/src/components/preview/      mediaTypes（Office MIME → renderer 'pdf' 需 pipeline）、nfspProvider（document → preview-document）
src/frame/nfs_server/src/preview/              tools.rs（soffice）、media.rs（Kind::Document 嗅探）、convert.rs（Profile::PreviewDocument）、rpc.rs（profile_for）、mod.rs（PROFILES、converter_id）、tests/preview.rs、README「预览管线」
src/tools/buckyos-agent/commands/import_office.ts   Agent 命令（editable 模式）；buckyos-build.py vendor 复制加 import-office
src/apps/jarvis_agent/agent/cmd_skills/aiworkspace_template_dev.md   手册加"导入 Office 文档"一节
src/frame/desktop/src/i18n/aiworkspace/office.ts   `aiworkspace.office.*`（index.ts parts + i18n 测试 PARTS 各加一行）
src/frame/desktop/vite.config.ts / tsconfig.app.json / tsconfig.node.json   别名 `@office/*`、`server.fs.allow`
```

## 3. docx → 流式页：映射规则（默认值即产品行为）

### 3.1 块结构

| Word | richtext | 备注 |
| --- | --- | --- |
| `w:p` | `paragraph` | 空段落保留（Word 用空段落分隔），连续 ≥2 个空段落压成 1 个并计数（不进报告） |
| 样式链解析到 `Heading1–6` / `w:outlineLvl` | `heading{level}` | 样式继承：`w:pStyle` → styles.xml `w:basedOn` 链；标题 7–9 → `heading 6` |
| `w:numPr`（numbering.xml `numFmt`） | `bullet_list` / `ordered_list` + `list_item`，`ilvl` 嵌套 | `bullet` / `none` → bullet，其余（decimal、lowerLetter…）→ ordered；跨段落同 `numId` 合并成一个列表；编号起始值不保留（报告 `list_start`） |
| 样式 `Quote` / `IntenseQuote` | `blockquote` | |
| 样式名含 `Code` / `HTMLPreformatted`，或整段字体为等宽 | `code_block` | 弱规则：只认样式名，不猜字体（Q3） |
| `w:tbl` | `table / table_row / table_cell`，首行 `w:tblHeader` → cell `header` | `gridSpan` / `vMerge` 展开为重复单元格（文本只在首格），报告 `merged_cells`；嵌套表格降为父单元格内的段落，报告 |
| `w:drawing`（`a:blip r:embed`）、`w:pict/v:imagedata` | 块级 `object_embed` → `buckyos.asset-ref` 实体 | 行内图片提升到所在段落之后；PNG/JPEG/GIF/BMP/TIFF 直接上传（BMP/TIFF 由宿主 canvas 转 PNG，Deno 宿主原样上传并报告）；EMF/WMF → 按 §5.3 栅格化成 PNG 后当普通图片；无 soffice 时跳过 + 报告 `unsupported_image` |
| `w:hyperlink`（`r:id` 外链 / `w:anchor` 书签） | `link` mark（`http/https/mailto`） | 书签内链 → 纯文本 |
| `w:r` 属性 `w:b / w:i / w:strike / w:dstrike` | `strong / em / strike` | 下划线、颜色、字号、字体、高亮：richtext 没有，丢且不报告（预期行为） |
| `w:br`（文本内）/ `w:br type=page` | `hard_break` / 无 | 分页不分块（流式页连续） |
| `w:footnote / w:endnote` 引用 | 文末 `horizontal_rule` + `ordered_list` | 引用处写 `[n]` 文本 |
| `w:ins / w:del`（修订） | 取 `ins`、丢 `del` | 默认"接受全部修订"，报告修订计数（Q4） |
| `w:comment*` | 丢 | 报告 `comments_dropped` |
| 页眉页脚、目录域（`TOC`）、`w:sdt` 内容控件的外壳 | 丢 / 透传内容 | `w:fldSimple / fldChar` 取缓存结果文本 |
| `w:txbxContent`（文本框）、`wps:wsp` 形状 | 内容提升为段落 | 报告 `floating_text` |
| Word 里的图表 `c:chart`、SmartArt、OLE | 按 §5.3 栅格化成块级图片 | 无 soffice 时丢 + 报告 |
| 数学公式 `m:oMath` | 按 §5.3 栅格化成块级图片；无 soffice 时占位文本 `[公式]` | 报告 |

### 3.2 产物

- **一张流式页，一个 richtext Block**（Q2）：标题取 `docProps/core.xml` `dc:title`，缺省用文件名；文档内第一个 `Title` 样式段落不去重。超过 core `max_blocks / max_text_utf16` 时按 `heading 1` 切成多个 richtext Block（第一个 Block 也放在流式页上，顺序保留）；图片实体放在页的内容目录（`contentFolderId`，同 PDF 导入 `firstOps` 的文件 Block）。
- **原文件保留**：≤ 32 MiB 时上传原件，作为一个 asset Block 放在流式页末尾（与 PDF 导入一致，用户可随时"原样导入"或下载）。
- **block_id**：构造时生成（`randomId('b')`），提交前用 WASM 校验 AST（限额、id 唯一、embed 引用存在）。
- 提交：`createSurfaceOps` + 资产实体 + richtext 一次 commit；超限按 Block 拆批（`batches`）。

## 4. pptx → 画布页：映射规则

### 4.1 页与布局

- `p:presentation/p:sldSz`（EMU）→ 页矩形，按 `pageScale` 缩放到长边 1920；页按 `gridOrigins / defaultColumns` 网格排列，顺序 = `p:sldIdLst`；`p:sld show="0"` 默认跳过并报告（选项可含）。
- 页 = `group` + `frame`（标题 = 幻灯片标题占位符首行，缺省 `第 n 页`；`config.office_source = { file, slide }`），`payload.presentation.background` = 页底色。
- 背景 `p:bg`：`solidFill` → 页底色；`gradFill` → 一个全页 svg shape（线性/径向渐变）放在最底层；`blipFill`（图片背景）→ 全页 asset；母版/版式背景按继承取。

### 4.2 继承与主题

- 占位符（`p:nvPr/p:ph type/idx`）按 `type` + `idx` 从 layout → master 取 `spPr`（位置、尺寸）、`txBody` 的 `lstStyle`（级别字号、项目符号、颜色、对齐）与 `bodyPr`；slide 自己有的覆盖。
- 主题：`a:clrScheme`（经 master `p:clrMap` 映射 `bg1/tx1…`）、`a:fontScheme`（`+mj-lt / +mn-lt / +mj-ea / +mn-ea` → 实际字体名，CJK 用 `ea`）；`a:lstStyle` 的 `defPPr/lvl1pPr…` 与 `presentation.xml` 的 `defaultTextStyle` 是最后的兜底。
- 颜色：`srgbClr / schemeClr / sysClr / prstClr` + 修饰符 `lumMod / lumOff / tint / shade / alpha / satMod` 按 Office 的 HSL 算法（与 import-table 的 xlsx 主题色处理若有重叠则抽到 `theme.ts` 共用）。

### 4.3 对象

| PresentationML | PlanItem | 备注 |
| --- | --- | --- |
| `p:sp` 有文本、几何为 `rect` 且无填充无描边 | `shape`（文本框）：`config.text`、`font_size`（`sz/100` pt → 世界单位按页缩放）、`bold/italic/font_color/align/valign/line_height/font_family` | 段落为单位，多段用换行；项目符号作前缀字符（`a:buChar`、`a:buAutoNum` 用序号）；`normAutofit fontScale` 应用缩放；`bodyPr` insets 与 `anchor` → `valign`；每段只能一种样式：取第一段 run 的样式，段内混合样式报告 `rich_label`（同 draw.io 的处理） |
| `p:sp` 预设 `rect / roundRect / ellipse / line / straightConnector1 / bentConnector*` | `shape`（rect / ellipse / line）| 直线取两端点；弯折连接线降为折线 svg |
| `p:sp` 其他 `prstGeom`、`custGeom` | `shape`（svg）| `geometry.ts` 预设几何表 + guide 解释器；填充 solid/gradient（渐变 → SVG 渐变）、pattern → 纯色 + 报告；`ln` 宽度/颜色/虚线；阴影/发光/3D 丢 |
| `p:pic`（`a:blip r:embed`） | `asset` | `srcRect` 裁剪与大图重采样由宿主 canvas 做（Deno 宿主保留整图 + 报告）；EMF/WMF → §5.3 栅格化成 PNG，无 soffice 时占位框（虚线 rect + 文件名）+ 报告 `unsupported_image`；`a:extLst` 里的 svgBlip 优先用 SVG |
| `p:grpSp` | 展开为平铺对象 | 应用 `grpSpPr/a:xfrm` 的 `chOff/chExt → off/ext` 仿射；组的旋转应用到子对象；不建 aiws group（PlanItem 的 group 只用于页） |
| `p:graphicFrame` 表格 `a:tbl` | 网格：每格一个 `shape`（rect + 文本），列宽 `a:gridCol`、行高 `a:tr h`，合并格按跨度扩大 | P0 画网格（Q6）；不建 TableSource |
| `p:graphicFrame` 图表 `c:chart` | §5.3 栅格化成 `asset`（原矩形、原 z 序） | 无 soffice 时占位框 + 报告 `chart`；P1 可解析 bar/line/pie 画 svg |
| `p:graphicFrame` SmartArt `dgm:relIds` | 若有 `dsp:drawing` 回退部件（Office 2010+ 保存时写入）→ 当形状处理；否则 §5.3 栅格化成 `asset` | 两者都不行才占位框 + 报告 |
| OLE / 媒体 `p:oleObj / a:videoFile` | 有 `p:pic` 预览图则用预览图；否则 §5.3 栅格化；再不行占位框 + 报告 | 视频只取预览帧 |
| `p:cxnSp` 连接线 | `shape`（line / svg） | 不建 aiws 连接线对象（连接线需要锚定的两端对象，P1 评估） |
| `a:xfrm rot/flipH/flipV` | `rotation`；翻转 → svg 内烘焙 | PlanItem 已有 rotation |
| 备注页 `p:notes` | 丢（Q8） | 报告 `notes_dropped` |
| 动画、切换、链接动作 | 丢 | 不报告 |

### 4.4 回退

- 一页对象数 > 阈值（沿 import-pdf 的 `PATH_LIMIT` 思路）或几何解释失败 → 该页报告并保留已解释部分；浏览器没有渲染器，"整页栅格化"只在有 soffice 时可做（整页按 §5.3 当一个子文档栅格化，作为该页的底图 asset），否则对话框在报告处提示"原样导入"。
- 字体：`font_family` 原样写入（Desktop 渲染用系统回退字体）；不做字体嵌入（`ppt/fonts/*.fntdata` 忽略）。

## 5. 原样路线（LibreOffice → PDF → import-pdf）

### 5.1 nfs_server 预览管线扩展

- **工具**（tools.rs）：`soffice`，查找顺序同 ffmpeg：`$BUCKYOS_ROOT/libexec/buckyos-tool/bin/` → `PATH` → 平台固定路径（Linux `/usr/bin/soffice`、`/usr/lib/libreoffice/program/soffice`、`/opt/libreoffice*/program/soffice`；macOS `/Applications/LibreOffice.app/Contents/MacOS/soffice`；Windows `%ProgramFiles%\LibreOffice\program\soffice.exe`、`%ProgramFiles(x86)%\...`）。版本 `soffice --version` 首行（`LibreOffice 24.8.4.2 …`），进 `converter_id`（`nfs.preview/1@soffice-24.8.4.2`）。探测结果进 `hello.repr`（`soffice: {path, version}` 或 `missing`）。
- **`Toolchain` 改为可选工具集**：`ffmpeg/ffprobe` 与 `soffice` 各自独立；`repr` 特性在**任一**工具存在时广告，`profiles` 只列可用档位（websdk `ReprCapability.profiles: string[]` 已能表达；Desktop nfspProvider 按 profiles 判断，不再只看 feature）（Q7）。
- **`Kind::Document`**（media.rs）：zip + `[Content_Types].xml` 含 `wordprocessingml / presentationml / spreadsheetml` 主部件；OLE2 魔数 `D0 CF 11 E0`（.doc/.ppt/.xls）；ODF（zip 首条目 `mimetype` 为 `application/vnd.oasis.opendocument.*`）；RTF `{\rtf`；扩展名只作辅助。
- **`Profile::PreviewDocument`**（convert.rs）：产物 `application/pdf`，结果类型 `document`（`result_type_of` 加一项，Desktop runtime 以 `pdf` 接受）。命令：

  ```text
  soffice -env:UserInstallation=file:///<nfs 数据根>/preview/tmp/profile-<attempt_id>
          --headless --norestore --nologo --nolockcheck --nodefault
          --convert-to pdf --outdir <tmp/attempt> <tmp/attempt/input.<原扩展名>>
  ```

  每次 attempt 独立 `UserInstallation`（否则第二个实例静默退出、残留锁文件）；输入复制为带真实扩展名的临时文件（soffice 按扩展名选过滤器）；Windows 用 `hide_child_console_async`，参数逐个 `Command::arg`；超时 `max(120 s, 10 s/MiB)`；并发 1（soffice 内存重，与 ffmpeg 的并发槽分开）；完成后删 profile 目录。`fidelity` 注记 `"由 LibreOffice 转换，字体与版式可能与原件有差异"`。
- **缩略图**：`("thumbnail", Kind::Document)` P0 返回 `no-renderer`；P1 用 `--convert-to png`（首页）出 `thumbnail` 档。
- **字体**：README 写明"保真度取决于节点字体，CJK 文档请安装 Noto CJK"；不在 `hello` 里枚举字体。
- **测试**：`tests/preview.rs` 加 docx/pptx/odt → PDF（无 soffice 自动跳过）、Document 嗅探、`soffice` 缺失时 profiles 不含 `preview-document` 但 ffmpeg 档位照常。

### 5.2 Desktop 侧

- `ImportOfficeDialog` 的模式选项：**可编辑（解析）** / **原样（LibreOffice）**，后者只在 `hello.repr.profiles` 含 `preview-document` 时可选，否则灰显并说明"需要内容服务器安装 LibreOffice"。
- 原样流程：上传原件为资产（本来就要保留）→ `repr({ source: { obj_id } }, { purpose: 'preview' })` → `get_repr` 轮询到 `completed` → `fetch(read_url)` 取 PDF 字节 → `pdfImports.open({ source: { name, bytes, original: { object_id, file_name } }, step: 'options' })`。`PdfImportSource` 新增 `original?`：有则 import-pdf 的文件 Block 指向原件（docx/pptx），不再上传 PDF（PDF 留在 preview-cache，30 天未读自动过期）。import-pdf 转换器零改动。
- **Preview 组件**（独立收益，O5 内一起做）：`mediaTypes.ts` 对 Office / ODF / RTF MIME 返回 renderer `pdf` 且 `direct: false`；`nfspProvider.profile_for`/能力判断加 `document`；ContentPreview 已有 Direct/Pipeline 标记与"此浏览器不支持该格式"文案，缺 soffice 时按既有 `no-pipeline` 走。

### 5.3 不可解析对象栅格化（Q5 裁决：能变成图片就变图片；soffice 有则用）

可编辑路线里图表、SmartArt（无回退部件）、OLE、EMF/WMF、公式没有纯 TS 渲染器。有 soffice 时把它们**逐个**变成图片，而不是整份文档退回原样导入：

1. **抽子文档**（`subdoc.ts`，纯 TS）：从原包复制 `[Content_Types].xml`、`_rels/.rels`、主部件的 rels 与从目标对象可达的全部部件（图表 `charts/chart*.xml` 及其 `embeddings/`、`media/`、`theme/`、`styles.xml`、`numbering.xml`；pptx 还要 slideLayout / slideMaster / theme），主部件改写为只含该对象：docx 是一个只有这一段的 `w:body`（页边距设 0、纸张设为对象 `wp:extent`，没有尺寸的用 A4）；pptx 是只含这一 `p:graphicFrame`/`p:pic` 的单张幻灯片，`p:sldSz` 改成对象尺寸、位置归零、背景置空。子包通常几十到几百 KB。
2. **转换**：子包上传为临时对象（Q14）→ `repr { source, purpose: 'preview' }` → `preview-document` → PDF；一份文档的多个对象并行发起，服务端并发 1 串行执行。
3. **栅格化**：Desktop 用 pdf.js（`ui/media/pdfDoc.ts` `renderPage`）渲染第一页，像素尺寸按对象在画布上的尺寸 × 2（上限沿 import-pdf `RASTER_EDGE`），docx 子文档再按内容自动裁白边；编码为 PNG（有透明）或 JPEG（照片类），作为 `asset` 放回**原矩形、原 z 序**；报告记 `rasterized`（计数 + 对象名），不再算"不支持"。
4. **无 soffice**：行为退回占位框 / 跳过 + 报告，报告对话框提示"内容服务器安装 LibreOffice 后可把这些对象导入为图片"。
5. **Deno 宿主（Agent）**：没有 canvas，栅格化 P2（与 Q9 同期）。

同一机制顺带给 §4.4 的"整页栅格化"回退与 docx 的公式、EMF 使用；用 SVG 产物而不是位图（`soffice --convert-to svg`）留作 P1 评估（LibreOffice 的 SVG 导出会把文字转路径、体积大）。

## 6. 入口与 UI

- **拖放 / 粘贴文件 / 插入目录"导入文件"**：`classifyFile` 返回 `office`（`officeKind(name, bytes)`: `docx | pptx | doc | ppt | odt | odp | rtf`）→ ask 步："插入为文件" / "导入为页面"（pptx 类）或"导入为文档"（docx 类）。旧二进制与 ODF 的可编辑模式不可用，ask 步只给"插入为文件" / "原样导入"（有 soffice 时）。
- **主菜单"导入"区**与数据树"新建"：导入 Office 文档（文件选择器 accept 列表来自 `OFFICE_ACCEPT`）。
- **文件 Block**（asset-ref 为 Office 类型）右键："展开为可编辑页面 / 文档"（照 PDF 的"展开为可编辑页面"），来源已是资产则不重复上传。
- **选项（一屏）**：模式；pptx：包含隐藏页、每行页数；docx：接受修订 / 保留删除文本；"保留原文件"开关（默认开）。进度：解析 / 上传 / 提交；完成后 `ImportReportDialog`。
- 手机：与 PDF 导入一致，不特别处理。

## 7. Agent / CLI

- `src/tools/buckyos-agent/commands/import_office.ts`：`import_office <file> --workspace <ws> [--parent <surface|folder>] [--mode editable] [--dry-run]`，宿主为 Deno（`@office/tools/import.ts`）：解析 → 资产上传（经 AiWorkspace client，与 import_table 的 runtime 一样）→ 提交；`--dry-run` 打印页数 / Block 数 / 资产数 / 报告。Deno 没有 canvas：不裁剪、不重采样、BMP/TIFF 原样上传并报告。
- `--mode faithful`（LibreOffice）：需要 Deno 侧 import-pdf 宿主（@napi-rs/canvas 渲染底图），列为 P2（Q9）。
- 手册 `aiworkspace_template_dev.md` 加"导入 Office 文档"一节：什么时候用 editable、什么时候建议用户在桌面上原样导入。
- `buckyos-build.py _prepare_agent_tools / build_aios`：vendor 复制加 `import-office`，deno.json 加 `@office/`。

## 8. 分期任务

| 编号 | 任务 | 产出 / 验收 |
| --- | --- | --- |
| O0 | 核实与夹具：core richtext `Limits` 数值与 WASM 校验入口；画布 text 对象对段内混合样式的实际能力；soffice 三平台路径与版本串；`presetShapeDefinitions.xml` 来源（ECMA-376 Part 1 附录）与许可确认；用 python-docx / python-pptx 生成夹具 + 收集 3–5 份真实 Office / Keynote 导出 / WPS 保存的文件 | `fixtures/office/` + `src/make_*.py`；O0 结论回写本文 §12 |
| O1 | `import-office` 基础：`package.ts / units.ts / theme.ts / text.ts / plan.ts`，别名 `@office/*`，Deno 测试骨架 | 包解析、rels、主题色换算单测（含 lumMod/lumOff 对照 Office 实际值） |
| O2 | `docx.ts + operations.ts`：§3 全部规则 → DocPlan → 操作；金样 `expected/*.plan.json` | `deno test -A`；WASM 校验通过 |
| O3 | `geometry.ts + pptx.ts`：`tools/presets.ts` 预编译几何表、guide 解释器、§4 全部规则 → PagePlan[]；金样 | 预设几何逐个渲染对照（SVG 快照）；三份真实 pptx 的页数/对象数/报告金样 |
| O4 | Desktop 宿主：`classifyFile`、请求槽、对话框、Worker 解析、资产上传（裁剪/重采样/去重）、分批提交、文件 Block 展开、i18n；e2e `tests/aiworkspace/office-import.spec.ts`（OI-01 docx 建流式页与图片 embed、OI-02 pptx 建画布页与放映、OI-03 隐藏页/占位框/报告、OI-04 插入为文件后展开） | `pnpm check` + eslint + e2e 全绿；i18n 守卫 |
| O5 | nfs_server：`soffice` 探测、`Toolchain` 可选化、`Kind::Document`、`Profile::PreviewDocument`、`hello.profiles`、README；`tests/preview.rs`；Desktop：原样模式 + `PdfImportSource.original` + Preview 组件 Office 预览；websdk 不改 | `cargo test -p nfs_server`（有/无 soffice）；e2e OI-05 原样导入（CI 机器装 libreoffice-nogui）；Preview e2e 一例 docx 附件 |
| O5b | `subdoc.ts` + 宿主栅格化（§5.3）：docx 图表 / SmartArt / EMF / 公式，pptx 图表 / SmartArt / OLE / EMF；临时对象上传与清理（Q14）；Deno 金样只到子包（不渲染） | 子包能被 soffice 打开并转 PDF（`tests/preview.rs` 用夹具子包）；e2e OI-06 带图表的 pptx 导入后对象为图片、无 soffice 时为占位框 |
| O6 | Agent：`import_office`（editable）、手册、build 脚本 vendor | `--dry-run` 与真实提交各一例（本机 Zone） |
| O7 | DV：本机全新安装 Zone + 真实 Zone；LibreOffice 装 / 卸两种环境（`PREVIEW_DV_NO_SOFFICE=1`）；Windows / macOS 的 soffice 探测（Lite） | `tests/dv/aiworkspace.office.spec.ts`；§13 DV 记录 |

建议顺序：**O0 → O5 → O1 → O2 → O4（docx 部分）→ O3 → O4（pptx 部分）→ O5b → O6 → O7**。O5 投入最小（nfs_server 几百行 + Desktop 一个模式分支）且立刻在 OOD 上给出全保真的 pptx/docx 导入与附件预览；可编辑路线按 docx（规则少、收益稳）先于 pptx（几何与继承最重）。（Q13）

## 9. 验证

- 共享模块：`cd src/frame/aiworkspace/import-office && deno test -A`（`OFFICE_REGEN=1` 重生金样）。
- Desktop：`pnpm check`、eslint、`AIWS_BIN=… NFS_BIN=… npx playwright test --config=playwright.aiworkspace.config.ts tests/aiworkspace/office-import.spec.ts`。
- nfs_server：`cargo test -p nfs_server --test preview`（soffice 缺失自动跳过文档用例，但嗅探与"不广告"用例必须跑）。
- 真实样本：至少各 3 份来自 PowerPoint / Keynote 导出 / WPS / Google Slides 导出的 pptx，Word / Pages 导出 / WPS / Google Docs 导出的 docx；记录页数、对象数、报告条目数、解析耗时（目标：50 页 pptx 在浏览器 Worker 内 < 5 s）。
- DV 前置：OOD 上 `apt install libreoffice-nogui fonts-noto-cjk`（约 500 MB + 100 MB）；验证"卸载后 profiles 不含 preview-document、对话框灰显原样模式、Preview 显示不支持而非损坏"。

## 10. 不做 / 后续

- 导出为 pptx / docx（反向）：不在本文；richtext → docx 可用 WASM 侧 `richtext_to_markdown` + pandoc（有则用）作过渡，画布 → pptx 需另立 TODO。
- 图表解析为 svg（bar/line/pie）、SmartArt 无回退部件的布局算法、pptx 表格 → TableSource、连接线 → aiws 连接线对象：P1。
- EMF/WMF 的纯 TS 转换：不做；有 soffice 时按 §5.3 栅格化，没有则占位。
- `.doc/.ppt/.odt/.odp/.rtf` 的可编辑解析：不做（原样路线覆盖）。
- Agent 的原样模式：P2（依赖 import-pdf 的 Deno 宿主）。

## 11. 裁决（2026-11-01 用户裁决；均按建议，Q5 改为"能变成图片就变图片"，已落成 §5.3 / O5b）

| 编号 | 问题 | 建议 | 裁决 |
| --- | --- | --- | --- |
| Q1 | 可编辑路线自写 OOXML 解析，还是引 mammoth.js / pptxtojson | **自写**。理由：Deno 宿主没有 DOM（mammoth 产 HTML 还要二次转换）；产物直接是 aiws 模型；模块无依赖惯例；pptxtojson 的 JSON 仍要全部重映射且其缺口（继承、主题色）正是难点。两者只作规则对照参考，不进依赖 | 按建议 |
| Q2 | docx 整篇一个 richtext Block，还是按 H1 切多个 | **一个**，超 core 限额才按 H1 切 | 按建议 |
| Q3 | 代码块识别 | 只认样式名（`Code*` / `HTMLPreformatted`），不按字体猜 | 按建议 |
| Q4 | 修订默认 | 接受全部修订（取 ins 丢 del），选项可"保留删除文本为删除线" | 按建议 |
| Q5 | 图表 / SmartArt（无回退）/ OLE / EMF | 占位框 + 报告 + 报告对话框里提示"原样导入" | **能变成图片导入就好**：有 soffice 时逐对象抽子文档 → PDF → 栅格化为图片放回原位（§5.3）；无 soffice 才占位框 |
| Q6 | pptx 表格 | P0 画网格（rect + 文本），P1 评估转 TableSource | 按建议 |
| Q7 | `repr` 广告条件 | 任一工具存在即广告，`profiles` 表达细粒度；nfspProvider 改为按 profile 判断 | 按建议 |
| Q8 | 备注页 | P0 丢弃并报告；P1 作页外 text 对象或 Frame 描述 | 按建议 |
| Q9 | Agent 原样模式 | P2 | 按建议 |
| Q10 | 旧二进制与 ODF 的可编辑解析 | 不做，原样路线覆盖 | 按建议 |
| Q11 | soffice 并发与超时 | 并发 1、`max(120 s, 10 s/MiB)`、独立 UserInstallation | 按建议 |
| Q12 | 原样导入产物的"原文件" | 文件 Block 指向原件（docx/pptx），PDF 不入库（留 preview-cache） | 按建议 |
| Q13 | 分期顺序 | O5 先行（见 §8） | 按建议 |
| Q14 | §5.3 子包怎样送到 `repr`：wire 冻结只认 `at` / `source.bucket` / `source.obj_id` | 用 Workspace `session.uploadAsset` 上传为临时资产后 `source.obj_id`；O0 核实未被实体引用的上传在 GC 下会被回收，否则导入结束显式删除；不扩 `repr` wire（`source.inline`）| 按建议开工 |

## 12. 实施回写（2026-11-01，O0–O6）

### 12.1 O0 核实结论

| 项 | 结论 |
| --- | --- |
| richtext 限额 | `schemas/richtext.basic.v1.json`：`max_blocks 5000 / max_text_utf16 500000 / max_depth 8 / max_update_bytes 1 MiB`；depth 从 doc=0 起算，所以顶层列表最多 3 层（再深折到第 3 层，报告 `docx_list_depth`） |
| heading 级别 | **schema 只到 3 级**（不是 6）：Heading 1/2/3 → 1/2/3，4–9 → 3（报告 `docx_heading_level`），Title → 1 |
| 其他 schema 约束 | `table_cell` 只收 inline（多段落用 hard_break 连接）；`list_item` 首个子节点必须是 paragraph；`ordered_list.start` 存在（编号起始值**保留**，不再报告 `list_start`）；`code` mark 互斥；link 只收 `http: https: mailto: #` |
| WASM 校验入口 | `richtext_block_index(ast_json)`（canonicalize + validate_ast）；Desktop 提交前逐个 richtext 调用。Deno 侧用 `import-office/richtext.ts` 的 `validateDoc` 镜像这些规则（测试比对 schema JSON 的 limits） |
| `object_embed` 目标 | **只能引用 `buckyos.cell`**（`target_types: ["buckyos.cell"]`），不是 asset-ref；而 Cell 只能挂在 surface / group / control 下，流式页会把这些 Cell 也渲染一遍 → 见 §12.2 偏差 1 |
| 画布 text 能力 | shape 的 `config` 每个形状一种样式（`font_size / bold / italic / font_color / align / valign / line_height / font_family`）；`font_family` 只认 `sans / serif / mono`（不是字体名）→ 字体名按正则归类（宋/明/Times… → serif，Courier/Consolas… → mono） |
| soffice | 本机 Debian 13 `apt install libreoffice-nogui fonts-noto-cjk`：`/usr/bin/soffice`，`LibreOffice 25.2.3.2 520(Build:2)`；三平台路径见 nfs_server README「预览管线」 |
| presetShapeDefinitions.xml | ECMA-376 Part 1 附带的电子文件；取 Apache POI 仓库（Apache-2.0）原样携带的版本，187 个预设；预编译为 `import-office/presets.ts`（155 KB），生成脚本 `tools/presets.ts` |
| 夹具 | `fixtures/office/`：`src/make_docx.py`（python-docx）、`src/make_pptx.py`（python-pptx 1.0.2，Debian 无包，用 venv 装）、`src/make_lo.sh`（LibreOffice 另存 `*.lo.docx/pptx` 与 odt/odp）；`docx/real` 10 份、`pptx/real` 9 份取自 Apache POI test-data（Word / PowerPoint 2007–2016 保存件，含 EMF/WMF、chartex、SmartArt、表格样式、CJK）。**Keynote / WPS / Google 导出件未收集到**（无可用来源） |
| Q14 临时对象 | aiworkspace 的 staged asset TTL 24 h，但 `list_unretained` 只列不收（"Nothing is collected automatically in this phase"），也没有删除 RPC → 子包**不走 Workspace 资产**：新增 `session.uploadTransient`（NDM 上传、不绑定桶、不登记 staged），上传者的 NDM hold 24 h 内 `repr` 可按 `obj_id` 读，过期后无引用、可被回收；Workspace 不留任何记录 |

### 12.2 与正文的偏差（均因 O0 事实或实测）

1. **docx 图片不是 richtext 里的 embed，而是流式页上独立的图片 Block**（§3.1、§3.2、Q2 的落地方式）：embed 只能指向 Cell，Cell 放到流式页上就会被单独渲染一次，core 也不允许 Cell 挂在 folder 下。于是一份 docx = 一张流式页，Block 顺序 = 文档顺序：图片之间的文字连成一个 richtext Block，图片是 asset Block（asset-ref 在页面内容目录），原文件是最后一个 asset Block。没有图片的文档仍是一个 richtext Block（Q2 成立）；超限仍按 H1 切分。列表被图片打断时，后半段 `ordered_list.start` 接着编号。
2. heading 只到 3 级（见 §12.1）；编号起始值保留（schema 有 `start`）。
3. pptx 一个形状的文字样式取**字符数最多的样式**，而不是首个 run（首个 run 加粗会让整段列表变粗；对照 LibreOffice 渲染后改的）；仍报告 `pptx_mixed_style`。
4. 原样导入与子包转换的 `repr` 不走 Desktop 共享的 `nfspClient`（它用 SSO token、页面 origin），而是用 Workspace session 的内容服务器端点与 token（新增 `session.contentServer()`）建一个 `NfspClient`：上传与转换是同一服务器、同一主体，`ndm_holdings` 才能授权读取。
5. pptx 表格样式：优先读 `ppt/tableStyles.xml` 的 `wholeTbl / band1H / firstRow` 填充、`tcTxStyle` 文字色与 `tcBdr` 边框；文件里没有该样式 id 时按 Accent 1 近似并报告 `pptx_table_style`。
6. 子包复制部件时必须保留**隐式关系**（母版的 theme、版式的母版、图表的 chartStyle/colors、文档的 styles…）：只按 XML 里出现的 rId 复制会丢掉 theme，图表的柱子与网格线随之消失（e2e 截图发现，`subdoc.ts` `IMPLICIT`）。
7. 母版/版式上的非占位符形状（装饰、logo）作为每页的对象导入（与 PowerPoint 显示一致；`office_source.layer` 标记来源）。
8. `ImportOutcome` 增加 `kind: 'office'` 与 `hint`：无 LibreOffice 且报告里有本可栅格化的条目时，报告框底部提示"安装 LibreOffice 后…"。
9. 新建画布/流式页的 entity `name` 在兄弟间唯一（`createSurfaceOps`，同名加 ` (2)`），修掉"同一文件导两次被拒"——这是 PDF / draw.io 导入与手建同名画布共有的既有问题（DV 发现）。
10. Agent 命令的图片上传优先走调用者在 aiworkspace 服务下的 `assets` 桶（`asset.import_object {bucket_id, key}`，与桌面一致），不可用时退回 Agent 自己的 artifacts 桶 + `obj_id`。

### 12.3 落点

- 共享模块 `src/frame/aiworkspace/import-office/`（README 有文件表）：`package / units / theme / richtext / docx / text / geometry / presets / pptx / subdoc / plan / operations / mod` + `tools/presets.ts`、`tools/import.ts`（Deno 宿主）；测试 `package_test / docx_test / pptx_test / geometry_test`（`test_util.ts`），金样 `fixtures/office/expected/*.plan.json`（`OFFICE_REGEN=1`）。复用 import-table 的 zip、import-drawio 的 XML / PagePlan / `buildOperations` / keys / bytes、import-pdf 的 layout。
- Desktop：`ui/import/{officeImportRequest, officeImport, officeReader.worker, ImportOfficeDialog}.ts(x)`；e2e `tests/aiworkspace/office-import.spec.ts`（OI-01～08），DV `tests/dv/aiworkspace.office.spec.ts`；`classifyFile` 加 `office`（先于表格判定：.doc/.odt 的 OLE2 / ODF 容器会被表格嗅探误认成 .xls/.ods）；插入菜单"导入 Office 文档…"（`aiws-menu-import-office`）、拖放/粘贴 ask 步、文件 Block "展开为可编辑文档/页面"（`asset-expand-office`）；对话框挂 WorkspaceShell；`PdfImportSource.original`（原样导入时文件 Block 指向 Office 原件，Q12）；`session.uploadTransient / contentServer`；i18n `aiworkspace.office.*`（`office.ts`，i18n 守卫 PARTS 与 SOURCE_DIRS 已加）；别名 `@office/*`（vite + 两个 tsconfig + fs.allow）。
- nfs_server（O5，子代理实施）：`preview/{tools,media,convert,rpc,mod}.rs`、`server.rs`、`tests/preview.rs`（+4 个 office 夹具）、README；Desktop Preview 组件 `mediaTypes / nfspProvider / types / thumbnail / ContentPreview`。`hello.repr`：任一工具在即广告，`profiles` 只列可用档位，缺 soffice 时有 `missing: ["soffice"]` 与 `detail`；`converter_id` `nfs.preview/1@soffice-25.2.3.2`；结果 `result_type: "document"`、`media_type: application/pdf`、`plan: "soffice-pdf"`。soffice 的 profile 目录在 `<data_dir>/preview-tmp/profile-<attempt_id>`（随既有 scratch 目录在重启时清理）；docx/pptx/xlsx 按前 1 MiB 内的 zip 本地头条目名判定（nfs_server 无 deflate 依赖，不读 `[Content_Types].xml`）。
- Agent（O6）：`src/tools/buckyos-agent/commands/import_office.ts`（editable 模式；图片上传见偏差 10）；`deno.json` `@office/`；`buckyos-build.py AGENT_TOOL_SHARED_MODULES` 与 `build_aios` 的 vendor 列表加 import-pdf、import-office；手册 `aiworkspace_template_dev.md` 加"Office documents"一条；buckyos-agent readme 加一节。`buckyos_admin.md` 的预览转换器段落由 O5 子代理补了 soffice / profiles / 字体。

## 13. 验证（2026-11-01）

- 共享模块：`cd src/frame/aiworkspace/import-office && deno test -A` → **47 passed**（金样 13 份 docx、11 份 pptx；本机有 soffice，子包 docx（EMF、图表）与 pptx（图表）均被 LibreOffice 转成 PDF）。Office 主题色对照 Office 取色器：Accent 1 #4472C4 的 "淡色 40% / 80%、深色 25% / 50%" = #8FAADC / #DAE3F3 / #2F5597 / #203864，逐值相等。
- 保真对照（未入库的脚本 `$CLAUDE_JOB_DIR/tmp/viz`：plan → HTML 截图 vs LibreOffice PDF → pdftoppm）：basic.pptx、shapes / SmartArt / backgrounds（PowerPoint 2010–2016）、aptia（9 页真实商务稿）、LibreOffice 5.1 保存的 17 页稿——版式、主题色、占位符继承、渐变、表格样式边框、SmartArt 回退图逐页一致；已知差异：线条渐变取首色、平铺纹理被拉伸、图片填充的非矩形形状按外接矩形。性能：Deno 下 48 页 customGeo.pptx 158 ms。
- Desktop：`pnpm check`（`tsc -b`）通过；eslint 改动文件无告警；i18n 守卫 18/20，剩余 2 项为 HEAD 既有失败（`catalog.sample/extension` 未用、`outline.rs` 中文）。
- e2e `AIWS_BIN=… NFS_BIN=… npx playwright test --config=playwright.aiworkspace.config.ts tests/aiworkspace/office-import.spec.ts` → **8/8**（OI-07 拖入 .odt 只给原样导入；OI-08 同一文档导两次：两张页，name 不同、title 相同）（e2e 内容服务器即本机，有 LibreOffice）：OI-01 docx → 流式页（richtext/asset/richtext/原件）；OI-02 拖入 pptx → ask → 8 页 + 8 步路径、隐藏页略去；OI-03 含隐藏页 9 页、报告列 `pptx_notes_dropped` 与 `pptx_rasterized`、图表为图片；OI-04 插入为文件后展开（原件 object_id 复用，保留删除线文字）；OI-05 原样导入：LibreOffice → PDF → PDF 导入，内容目录保留 `basic.docx` 而非 PDF；OI-06 VariousPictures.docx 的 2 个 EMF + 1 个 WMF 栅格化为图片（截图核对）。无 LibreOffice 分支（占位框、提示、原样模式灰显）由同一用例按 `data-libreoffice` 走另一侧断言，本机未实跑。
- 回归：pdf-import / table-import / flow / drawio e2e 34/34；**全量 aiworkspace e2e 249 例 247 过**，2 例为既有失败：CN17（连接线锚点数，HEAD 同样挂）、`i18n.spec.ts:133` film demo 末段（10-17 `AccountAppearanceSync` 让语言跟随账号后，测试里先切回的 en 会覆盖 reload 前写入 localStorage 的 zh-CN；该测试 10-16 后未更新，与本次改动无关）。
- nfs_server：`cargo test -p nfs_server --test preview` 10/10（含 docx/pptx/odt/doc → PDF、Document 嗅探、缺 soffice 时 profiles 不含 preview-document 而 ffmpeg 档位照常）；`cargo test -p nfs_server` 全部套件通过；Desktop `preview-media-types` 测试通过。
- Agent：`deno run -A aicc-tool.ts import_office <docx|pptx> --dry-run` 正常；`_prepare_agent_tools` 生成的宿主副本（vendor 四个模块、deno.json 改写为 `./vendor/frame/…`）独立运行 `import_office` 与 `import_table` 的 dry run 正常；`tools.tests.test_buckyos_build_entrypoint` OK。

## 14. DV（2026-11-01，本机 Zone test.buckyos.io）

- 部署：只换 nfs-server 二进制（原子 mv 后 `pkill -x nfs_server`，node-daemon 秒级拉起，日志 `preview converter: soffice 25.2.3.2 (/usr/bin/soffice)`）、`bin/control-panel/web`、`libexec/buckyos-agent`；备份 `/home/lzc/project/deployment-backups/20261101-office-import/`。构建 `uv run buckyos-build.py -s nfs_server`、`-s desktop`（顺带重建 SDK/Tool 与 buckyos-agent 宿主副本）。DV 前置：本机 `apt install libreoffice-nogui libreoffice-writer-nogui libreoffice-impress-nogui fonts-noto-cjk poppler-utils`。
- `BUCKYOS_UI_DV_BASE_URL=https://test.buckyos.io BUCKYOS_TEST_ADMIN_PASSWORD=bucky2025 pnpm exec playwright test --config=playwright.dv.config.ts tests/dv/aiworkspace.office.spec.ts` → **3/3**（VariousPictures.docx → 流式页、EMF/WMF 经 LibreOffice 成图；basic.pptx → 8 页 + 路径、图表成图；原样导入 basic.docx → LibreOffice 排成 3 页 A4 → PDF 导入，内容目录保留 .docx）。
- 无 soffice：临时改名 `/usr/bin/soffice` 与 `/usr/lib/libreoffice/program/soffice`（nfs_server 也查固定路径）、重启 nfs_server，`PREVIEW_DV_NO_SOFFICE=1` 同一 spec → **3/3**（`hello` 不含 preview-document、报告为 `docx_unsupported_image` / `pptx_chart` 并显示提示、原样模式灰显），之后改回并重启（注意 `/usr/bin/soffice` 是符号链接，目标改名期间 `[ -e ]` 为假，恢复要用 `-L`）。
- Agent：宿主上用桌面登录 cookie 作 `BUCKYOS_APPCLIENT_SESSION_TOKEN`（`BUCKYOS_APP_ID=control-panel BUCKYOS_OWNER_USER_ID=devtest BUCKYOS_HOST_GATEWAY=127.0.0.1 BUCKYOS_ZONE_HOST=test.buckyos.io`），`/opt/buckyos/libexec/buckyos-agent` 下 `aicc-tool.ts import_office` 真实提交 basic.pptx（8 页 + 路径、2 张图、原件）与 basic.docx（4 个 Block、1 张图、原件）成功。DV 中发现并修复两处：① 借用的 control-panel 会话没有 Agent 的 artifacts 桶 → 图片上传改为先放调用者在 aiworkspace 服务下的 `assets` 桶（与桌面登录用户相同），`asset.import_object {bucket_id, key}`，失败再走 Agent 桶 + `obj_id`；② 同一文档导两次时 `name` 冲突（`name "…" already used under canvas-content`）——桌面的 `createSurfaceOps` 同样有此问题（PDF / draw.io 导入同名文件、手建同名画布都会被拒），改为 name 在兄弟间唯一（` (2)`），title 不变；Deno 宿主同样处理。
- 改了 `createSurfaceOps` 后重建 Desktop 再部署，`tests/dv/aiworkspace.office.spec.ts` + `tests/dv/aiworkspace.pdf.spec.ts` → **6/6**。PDF DV 首例原先在 reload 后二次 `openWorkspace` 时拼出 `aiws-<id>.aiws-<id>.test.buckyos.io`（T4-8 独立域名之后 DV 夹具 `workspaceOrigin` 的既有问题），已在夹具里去掉已有的 `aiws-…` 前缀。
- 真实 Zone（ssotest 三 Zone VM、Windows / macOS 的 soffice 探测）：**未做**。

## 15. 未做 / 风险

- Keynote / WPS / Google 导出的 pptx/docx 样本未覆盖（无来源）；Windows / macOS（Lite）的 soffice 路径探测与控制台隐藏只按代码实现，未实机验证。
- soffice 超时只杀直接子进程：Linux 上 soffice → oosplash → soffice.bin，soffice.bin 可能残留（彻底解决需按进程组杀，要引入 `libc` 依赖，待用户确认）。
- websdk 的 `ReprCapability` / `ReprResult` 类型比新 JSON 窄（缺 `soffice`、`missing`、`result_type: 'document'`），Desktop 侧做了类型放宽；websdk 类型待同步。
- 图表只能经 LibreOffice 成图（P1 的 bar/line/pie → svg 未做）；SmartArt 无回退部件、无 LibreOffice 时为占位框；pptx 表格不转 TableSource；连接线不建 aiws 连接线对象；备注页丢弃（Q8 P1 未做）。
- 已知保真差异：形状一种文字样式（取多数样式）、线条渐变取首色、纹理平铺被拉伸、图片填充的非矩形形状按外接矩形、竖排文字横排、镜像图片不镜像、页眉页脚不导入。
- Agent 的原样模式与子包栅格化（Deno 无 canvas）：P2。
- 子包临时上传依赖 NDM hold 过期后的存储回收（24 h），Workspace 内无记录；若 named store 不回收无引用对象，会残留少量几十 KB 的子包。
