# BuckyOS AI Workspace PDF 导入与放映方案讨论

> 状态：v0.3（2026-10-28）**P0 已实施**（另含 P1 的 `font_family`、书签 / 标题作页名、"展开为页面"）；§9 全部按建议值落地，实施偏差见 §12，验证见 §13，未做见 §14。v0.1 的"整页栅格"改为降级档，主线是**尽量保留 PDF 结构、导入后可编辑**。
> 范围：把一份 PDF 导入 AI Workspace，得到一张可编辑的 Surface（每页一个页面组，页内文字、图片、图形是独立对象）和一条演讲路径，放映效果与 PDF 阅读器的全屏演示一致（整页铺满舞台、按页序切换、无动画、黑边补齐）。
> 依据：[多媒体对象](BuckyOS%20AI%20Workspace%20多媒体对象.md)、[第三期规划](BuckyOS%20AI%20Workspace%20第三期规划.md) §4–§10（Frame / 路径 / 舞台 / 放映）、[draw.io 导入 TODO](../../notepads/aiworkspace-drawio-import-todo.md)（`shape` 文字 / SVG 形状、共享转换模块、分批提交）。

## 1. 结论

1. **不新增实体类型，不改内核。** 一页 PDF = 一个 `group`（页面组）：第一个子对象是覆盖整组的 `frame`（放映步骤目标，复用泳道 backing 约定），其后按 PDF 绘制顺序是这一页拆出来的对象：文字 → `shape: text`，图片 → `asset`，矢量图形 → `shape: rect / line / svg`，拆不动的部分 → 一张"无文字底图"`asset`。整份 PDF 落在**一张新的自由 Surface** 上按行列排开，外加一条 `buckyos.show-path`，每页一步、`cut`、黑底。放映、提示器、引导、导出导入、离线副本沿用现有机制。
2. **文字永远可编辑**：pdf.js `getTextContent` 给出每段文字的内容、位置、字号；算子列表给颜色、字体名和粗斜体；带标签（tagged）的 PDF 还给段落 / 标题 / 图 / 表的结构树。文字按行、段聚成 `shape: text` 对象，不回流（逐行 `\n`，`wrap: false`），位置误差在像素级。
3. **图形分三档保真**：矩形 / 直线 → 标准形状（可改色、改线型）；一般路径与轴向 / 径向渐变 → `shape: svg`（可改色、可拉伸，≤ 48 KiB）；图案、网格渐变、软遮罩、混合模式或路径过多 → 用 pdf.js 的 `operationsFilter` 渲一张**不含文字和图片的底图**（整页或该元素包围盒），放在文字之下。所以任何页都不会出现"底图里的字和可编辑的字叠在一起"。
4. **转换产出复用 draw.io 导入的 `PagePlan`**（[convert.ts](../../src/frame/aiworkspace/import-drawio/convert.ts)）：PDF 转换器只负责"页 → PagePlan"，图片上传、`buildOperations`、分批提交、报告对话框全部是现成代码。
5. **"和 PDF 演示效果一致"由路径参数保证**：舞台比例取页面比例、路径背景黑、每步 `cut`、Frame 恰好等于页面矩形（放映时裁剪到 Frame，Frame 自身边框标题不显示）。按键、全屏 / 第二屏、激光笔、板书、提示器都是现成的。
6. **整页栅格**只作为用户可选的"高保真模式"和单页自动降级（§4.6），不再是主线。

## 2. 现状对照（路径相对 `buckyos/`）

| 范围 | 已有 | 缺口 |
| --- | --- | --- |
| PDF 读取 | [pdfPoster.ts](../../src/frame/desktop/src/app/aiworkspace/ui/media/pdfPoster.ts)：pdf.js 6.4.299 动态加载，只渲首页做 poster；未打包 CMap / 标准字体（老式中文 PDF 文字缺失） | 打包 `pdfjs-dist/cmaps`（1.7 MB）与 `standard_fonts`（820 KB）按需加载；读文字、算子列表、图片对象、结构树、大纲 |
| pdf.js 6.4 能力（已核对 `build/pdf.mjs` 并在 Node / Deno 实测，§5.1） | `getTextContent({ includeMarkedContent })`：文字项 `str / transform / width / height / fontName / hasEOL`，`styles[fontName].fontFamily / ascent / descent / vertical`；`getOperatorList()`：`fnArray / argsArray`，路径是 `constructPath(op, [drawOps, minMax])`（`DrawOPS` moveTo / lineTo / curveTo / quadraticCurveTo / closePath 的扁平数组）；图片 `paintImageXObject(objId)` → `page.objs.get(objId)`（已解码、已合成 SMask 的位图）；字体 `page.commonObjs.get(fontName)` 在 `fontExtraProperties: true` 时有 `name / isSerifFont / isMonospace / type`（粗斜体从 `name` 判断）；`getStructTree()` 给 `role`（P / H1 / Figure / Table / L…）和内容 id，与 `includeMarkedContent` 的标记对应；`getOutline()` 书签；`render({ operationsFilter, recordOperations, recordImages })`：**按算子过滤渲染**（实测可去掉文字与图片）、记录每个算子与图片的包围盒 | 无；只需在转换器里用起来 |
| 文字对象 | `shape: text`（[shapeBlock.tsx](../../src/frame/desktop/src/app/aiworkspace/ui/blocks/shapeBlock.tsx)）：`text` 多行、`font_size`（世界单位）、`font_color`、`bold / italic / underline`、`align / valign`、`wrap`、`label_fill` | 没有 `font_family`（P1 加一个键，取值 `sans / serif / mono` 或具体族名，缺省跟主题） |
| 图形对象 | `shape`：`rect / ellipse / line / …`，`fill / stroke / stroke_width / dash / opacity / radius / rotation`；`shape: svg`（draw.io 导入 §2.4）：可改色可拉伸的 SVG 体，≤ 48 KiB，`svgShapeFromDocument` 做清洗 | SVG 体里的渐变（`linearGradient / radialGradient`）和 `clipPath` 是否被清洗器保留需核对 |
| 图片 | `asset` Block + `buckyos.asset-ref`；`PlanImage { bytes, mediaType }` 由导入器统一上传 | 无 |
| 页面组 | `group` 第一个子对象为覆盖整组的 `frame`（backing），画布当一个对象选中、拖动 | 无 |
| 转换与提交 | [drawioImport.ts](../../src/frame/desktop/src/app/aiworkspace/ui/canvas/drawioImport.ts)：`PagePlan[]` → 上传图片 → `buildOperations` → ≤ 2 000 操作 / 批 → `ImportReport` | 多个 plan 放到**同一张** Surface 的行列排布（库条目模式已有"同一画布横排"，推广成网格即可） |
| 路径与舞台 | `createPathOp / createFrameOp / newStep`；任意 `StageSize`；`transition: cut`；Frame `presentation.background`；比例不符"完整放入 + 补边" | `createPathOp` 加 `steps` 参数，一次创建 |
| 放映 | Frame 步骤 = 相机铺满 + 裁剪遮罩；`cut` 无动画；预挂载下一步 | 无 |
| 画布 LOD | 屏幕上过小的 Block 画文字占位 | 全览时页面看不到缩略（§8 P1） |
| 入口 | 主菜单"导入 draw.io…"；`handleFiles` 按种类分派 | PDF 现在走"文件 Block"；要加分派与选择 |

## 3. 对象形态

```text
Surface《<文件名>》 (free)
├── group 页 1   placement = 页矩形 R1（页宽 → 1920 世界单位）
│   ├── frame  "1" | 书签名   (0,0,R1.w,R1.h)  config { color: 灰, fill: null }  presentation { background: <页底色> }
│   ├── asset  底图（仅当该页有拆不动的图形；无文字无图片）
│   ├── shape  rect   页里的色块 / 表格线 …
│   ├── asset  图片 1  config { pdf_source: { page: 1, image: 'img_p1_3' } }
│   ├── shape  svg    图表 / 图标
│   ├── shape  text   "标题"   config { text, font_size, font_color, bold, align, wrap: false, pdf_source: { page: 1, role: 'H1' } }
│   └── shape  text   "正文第一段 …"
├── group 页 2 …
canvas-content/<surface>-content
├── asset-ref deck.pdf（原件，可选保留；page_count、poster）
├── asset-ref deck-p001-img03.png …
shows/
└── show-path《<文件名>》 purpose=presentation  stage={1920, 1920·h/w}  background='#000000'
    steps: [{ target: { kind: 'frame', entity_id: 页 1 frame }, transition: 'cut' }, …]
```

- **顺序 = PDF 绘制顺序**（算子列表顺序），BlockTree 前序就是 z 序，叠放关系自然保真。底图（若有）排在 frame 之后、其余对象之前。
- **页矩形**：MediaBox（含 `/Rotate`）等比缩放，横版页宽 1920、竖版页高 1920；`cols = clamp(ceil(sqrt(N)), 1, 8)`，间距 160；左到右、上到下为页序。混合尺寸每页保持自己的比例，放映时补边。
- **舞台**：出现最多的页比例，`{ w: 1920, h: round(1920 · h / w) }`。
- **页底色**：算子列表里第一个覆盖 ≥ 95% 页面的纯色填充矩形不建对象，写成 Frame `presentation.background`（放映时也是这页的底色）；没有则白。
- **`config.pdf_source`**：每个拆出的对象记 `{ page, kind, id? }`（前端注册表校验，内核只限 64 KiB），用于报告、以后"重新从原件提取"和第二期活渲染。

## 4. 提取：一页 → `PagePlan`

转换器放在 `src/frame/aiworkspace/import-pdf/`（与 `import-drawio/` 并列的共享 TS 模块；输入是 pdf.js 已取出的纯数据，可用录制的 JSON 夹具做 deno 单测），浏览器侧 `ui/canvas/pdfImport.ts` 负责调 pdf.js、渲底图、上传与提交。

### 4.1 文字

1. `getTextContent({ includeMarkedContent: true })`。每个文字项取基线起点 `(transform[4], transform[5])`、字号 `hypot(transform[0], transform[1])`、宽高、字体名。
2. **成行**：同一基线（误差 < 字号 × 0.2）、x 相邻（间隙 < 字号 × 0.6，更大的间隙补空格）的项合并成一行；`hasEOL` 是行尾提示。
3. **成段**：
   - 有结构树（Office 导出的 PDF 默认带标签）：按 `P / H1–H6 / LI / TD / Caption / Figure` 等叶节点归属（内容 id ↔ `beginMarkedContentProps` 标记），一个结构元素 = 一个文字对象，`role` 记进 `pdf_source`。
   - 无结构树：行距 ≤ 字号 × 1.6、左缘或中线对齐、字号相同的相邻行归一段；否则分段。
4. **一个段 = 一个 `shape: text`**：`text` 为各行以 `\n` 相连（不回流，`wrap: false`）；矩形 = 各行并集外扩 2 pt；`font_size` 换算到世界单位；`align` 由各行相对矩形的左 / 中 / 右判定；`valign: top`。
5. **颜色、字体、粗斜体**来自算子列表：走一遍文本状态（`beginText / setFont / setTextMatrix / moveText / nextLine / showText / showSpacedText` 加字宽累计），得到每个 showText 运行的起点、填充色、字体对象；再按起点就近配给文字项。配不上的取黑色。字体对象 `bold / italic` 填 `bold / italic`；`name` 映射到 `font_family`（P1）：Helvetica / Arial / 微软雅黑 / 思源… → `sans`，Times / 宋体 → `serif`，Courier / Consolas → `mono`。
6. **拿不到可读文字**（Type3 字体、无 ToUnicode 的子集字体给出大量 U+FFFD、全是私有区码位）：该页标为"文字不可提取"，走 §4.6 的整页降级并记入报告。
7. 竖排文字（`vertical`）、文字沿路径（Tz / Ts 极端值）第一期不做对象，落到底图（见 §4.5 的过滤规则：这些 showText 不过滤）。

### 4.2 图片

- 算子列表遇到 `paintImageXObject / paintInlineImageXObject / paintImageMaskXObject`：从 `page.objs.get` 取已解码位图（SMask 已合成）；CTM 把单位正方形映到页坐标，得矩形与旋转（有倾斜 / 非 90° 旋转的取外接矩形并把 CTM 旋转角写入 `rotation`，拉伸差异忽略）。
- 编码：有 alpha → PNG；不透明且 ≥ 256×256 → JPEG 0.9；小图 PNG。同一 `objId` 多次绘制（重复 logo）只上传一次（`PlanImage.key` 相同）。图像蒙版（stencil mask）用当前填充色渲成 PNG。
- `recordImages` 的 `imageCoordinates` 可作校验（渲染时实际落点），不作主数据源。
- 走 `PlanImage { bytes, mediaType }` → 现有 `uploadImages`；上传失败按 draw.io 的做法变成带文字的虚框。

### 4.3 矢量图形

走一遍算子列表并维护图形状态（CTM、填充 / 描边色、线宽、虚线、透明度、裁剪、`paintFormXObjectBegin/End` 的嵌套矩阵）。每个 `constructPath` + 填充 / 描边算子产生一个候选：

| 形态 | 判定 | 对象 |
| --- | --- | --- |
| 轴对齐矩形 | drawOps 是 `moveTo + 3 lineTo + closePath` 且 CTM 无旋转 | `shape: rect`，`fill / stroke / stroke_width / dash / opacity` |
| 两点线段 | `moveTo + lineTo` | `shape: line` |
| 圆 / 椭圆 | 4 段贝塞尔且控制点符合 kappa 0.5523 | `shape: ellipse` |
| 其它路径 | — | `shape: svg`：`<path d>` + 填充描边；**连续**、同一 form、包围盒相交或相邻（间距 < 4 pt）的路径合并成一个 svg 对象（一张图表 / 一个图标是一个对象），超过 48 KiB 就按包围盒拆分 |
| 轴向 / 径向渐变 `shadingFill` | 有裁剪路径 | 并入 svg 对象：`linearGradient / radialGradient` + `clipPath` |
| 裁剪 | 覆盖整页的忽略；其余作为 svg 对象里的 `clipPath` | — |
| 图案填充、网格 / 函数渐变、软遮罩、非 normal 混合模式、knockout 组 | — | 不建对象，交给底图（§4.5） |

- 路径数阈值：一页超过 3 000 条路径（地图、复杂插图）时，矢量部分整体交给底图，只拆文字和图片；阈值在对话框可调。
- 细小装饰（面积 < 4 pt²）并入相邻 svg，不单独成对象。

### 4.4 结构树的其它用途

- `H1–H6` → 该页 Frame 的标题取第一个标题（无标题时取大纲书签，再无则"第 n 页"）；也作步骤标题。
- `Table` → 第一期仍是文字对象 + 表格线矩形（不转表格 Block）；`pdf_source.role` 保留，以后可提供"转为表格"。
- `L / LI` → 项目符号是单独的文字项，合并到同一段开头。
- `Figure` 的 `Alt` 文本写进对应图片对象的 `title`。
- `Link` 注释（`getAnnotations`）：页内跳转的链接记到对象 `config.pdf_source.link = { page }`，第一期不做交互。

### 4.5 底图（无文字、无图片的页面栅格）

`page.render({ canvas, viewport, operationsFilter })`，过滤器排除：已转成对象的路径算子（按算子下标）、全部 `showText*`（除 §4.1 第 7 条留给底图的那些）、全部 `paintImage*`、页底色矩形。剩下的就是"拆不动的"内容。渲染后：

- 若画布全透明（没有剩余内容）→ 不建底图对象；
- 否则取非透明像素的包围盒（`recordOperations` 的包围盒也可用），裁成 PNG（长边 ≤ 2560）作为一个 `asset`，位置为该包围盒；多块互不相邻的区域可拆成多张。

底图永远在文字和图片之下、Frame 之上；这样页面效果完整，而底图里不包含任何可编辑对象的像素。

### 4.6 降级档

| 档 | 触发 | 结果 |
| --- | --- | --- |
| 可编辑（默认） | — | §4.1–§4.5 |
| 文字不可提取 | §4.1 第 6 条 | 该页：整页栅格（含文字）+ 图片仍拆出；报告列出页码与原因 |
| 高保真 | 用户在对话框选，或单页路径 > 阈值且用户选择"复杂页整页保留" | 该页：一张整页栅格（长边 2560，可选 3840），无其它对象 |

三档的页面组结构相同（group + frame + 内容），路径步骤不受影响。

## 5. 导入流程（`ui/canvas/pdfImport.ts`，结构照 `drawioImport.ts`）

| 步 | 内容 | 失败处理 |
| --- | --- | --- |
| 1 读取 | 本地 `File` 或已上传的 PDF 资产；`getDocument({ data, cMapUrl, cMapPacked, standardFontDataUrl })` | 打不开 / 需口令：报错不落实体 |
| 2 对话框 `ImportPdfDialog` | 页数、页尺寸、首页预览（用首页试转换给出"文字 N 段 / 图片 N / 图形 N / 底图有无"的预估）；范围；模式（可编辑 / 高保真）；复杂页处理；是否保留原文件（> 32 MiB 禁用）；是否建路径；列数 | — |
| 3 转换 | 逐页串行：文字 + 算子列表 + 图片 + 结构树 → `PagePlan`；需要时渲底图；进度条 + 取消 | 单页异常：整页栅格代替，记入报告 |
| 4 上传 | `uploadImages`（并发 3）：拆出的图片、底图 | 失败：虚框 + 报告 |
| 5 提交 | 第一次：Surface + 内容文件夹 + 原件资产；随后每批 ≤ 2 000 操作（`buildOperations`，各页 plan 以网格 `origin` 放到同一 Surface）；最后：路径（带全部步骤） | 任一批被拒绝：停止，报告说明已导入到第几页 |
| 6 收尾 | 切到新 Surface，全览；通知"已导入 N 页（文字 x 段、图片 y、图形 z、底图 w 页），路径《…》已创建"+"开始放映"；报告对话框 | — |

- 取消：确认前无副作用；转换中取消 = 不提交。撤销按批。需要 `structure` 能力；离线会话入口禁用并说明。
- `pdfPoster.ts` 抽成 `ui/media/pdfDoc.ts`（打开文档 + CMap 配置、渲任意页、大纲、结构树），poster 与导入器共用；顺手修掉中文 poster 缺字。

**入口**：主菜单 › 插入 › "导入 PDF…"；拖放 / 粘贴单个 PDF 弹二选一（插入为文件 / 导入为可编辑页）；已有 PDF 文件 Block 工具栏"展开为页面…"（原资产直接作原件）；作品列表"从 PDF 新建作品"（P1）。

### 5.1 运行位置：浏览器为主，Deno 后端可行

pdf.js 不依赖 DOM 就能解析、取文字、取算子列表和图片位图；渲染需要一个 Canvas 2D 实现。2026-10-28 用 `pdfjs-dist@6.4.299` 的 `legacy/build/pdf.mjs` 实测（脚本在会话 scratchpad `pdfjs-probe/probe.mjs`）：

| 运行时 | 解析 / 文字 / 算子列表 / 图片位图 / 结构树 | 渲染（含 `operationsFilter` 去掉文字与图片） | 备注 |
| --- | --- | --- | --- |
| Deno 2.9.4 + `npm:@napi-rs/canvas@1.0.13` | 通过 | 通过（2 页测试 PDF 单页 15 ms，产出的 PNG 只剩矩形与线） | 全局 `DOMMatrix / Path2D / ImageData / OffscreenCanvas` 齐全；pdf.js 自己会 `import("@napi-rs/canvas")` |
| Node 24 | 通过 | 需要 pdf.js 与 `@napi-rs/canvas` 装在同一 `node_modules`（否则 `Path2D` 类不一致，`fill(path)` 报错） | fake worker 让进程不自动退出，要 `destroy()` / 显式退出 |

- 图片位图在非浏览器环境是 `{ width, height, kind, data }`（RGBA / RGB 原始字节，SMask 已合成），用 `@napi-rs/canvas` 或纯 JS 编成 PNG / JPEG。
- 字体元数据要 `getDocument({ fontExtraProperties: true })`：此时 `page.commonObjs.get(fontName)` 有 `name`（如 `Times-BoldItalic`）、`isSerifFont`、`isMonospace`、`type / subtype`；缺省只有 `_inspectFont`。粗斜体从 `name` 判断（`/bold/i`、`/italic|oblique/i`），`font_family` 从 `isSerifFont / isMonospace` 判断。浏览器同样适用。
- `standardFontDataUrl` / `cMapUrl` 在后端指向磁盘目录即可（`pdfjs-dist/standard_fonts`、`cmaps`）。

BuckyOS 里有 Deno 的位置：Agent 容器（`build_aios` 装 Deno v2.9.2，`deno-cache` 预热）、OOD 的工具运行时（`$BUCKYOS_ROOT/libexec/buckyos-tool/runtime/deno`，`aiworkspace_run` 手册已用它跑程序宿主）。`aiworkspace` 服务本身是 Rust，没有 JS 引擎；`exttool` 镜像只有 Deno 缓存没有二进制。

因此分层：

| 层 | 放哪 | 说明 |
| --- | --- | --- |
| 转换器 `import-pdf/` | 共享 TS 模块（与 `import-drawio/` 同样可在浏览器和 Deno 跑） | 输入 pdf.js 已取出的数据，输出 `PagePlan`；底图渲染通过一个 `render(page, filter) → PNG` 回调注入，浏览器给 `<canvas>`，Deno 给 `@napi-rs/canvas` |
| 浏览器入口 | Desktop `ui/canvas/pdfImport.ts`（第一期） | 用户自己导入 |
| Agent 入口（P1） | `buckyos-agent` 工具 `aiws import-pdf`（Deno）+ `cmd_skills` 手册 | Agent 用 websdk `aiworkspace` 客户端（kRPC）上传资产、`doc.commit`，以 Agent 身份写入，需要 Workspace 对该 Agent 的写权限或托管执行配置；适合"把这份 PDF 做成演示"的任务和大文件 |
| 服务端 kRPC（不做） | — | Rust 服务 spawn Deno 子进程要新增进程管理 / 沙箱 / 限额；已有托管执行把工作派给 Agent，没必要再造一条 |

Agent 容器要补的依赖：`pdfjs-dist`（含 `standard_fonts`、`cmaps`）与 `@napi-rs/canvas` 的 Linux x64 / arm64 预编译包进 `deno-cache`（约 10 MB / 平台），Deno 以 `--allow-ffi` 运行。不带 canvas 时转换器仍可工作但没有底图：需要底图的页退到"只拆文字和图片，其余丢弃并报告"。

## 6. 放映核对

| PDF 阅读器全屏 | 本方案 |
| --- | --- |
| 整页铺满，比例不符补黑边 | Frame = 页矩形；路径背景黑；contain 补边 |
| 翻页无动画 | 每步 `cut` |
| 方向键 / 空格 / PgUp·PgDn / Home·End、全屏、第二屏、激光笔、画笔 | 已有 |
| 页内超链接 / 页间跳转 | 记录不交互（§4.4） |
| 自带翻页效果（`/Trans`） | 不读 |
| 备注 | PDF 无；路径编辑里逐页填 |

拆解后的页在放映里与原 PDF 的差异来源只有两处：字体（浏览器字体替代，P1 的 `font_family` 把差距缩到同族）和字距（不保留字符间距 / 词间距）。图片、图形、底图按像素或矢量还原。

## 7. 导入后能编辑什么

| 对象 | 能做 | 不能 / 丢失 |
| --- | --- | --- |
| 文字 | 改文字、字号、颜色、粗斜体、对齐、移动、缩放、连接线锚点、批注 | 字体族（P1）、字距、行内混排样式（一段一种样式，按行分段时才保留行级差异）、文字沿路径 |
| 图片 | 移动、缩放、替换、适应方式、预览 | 裁剪路径（已烘焙进 PNG）、倾斜 |
| 矩形 / 线 / 椭圆 | 全部形状工具（颜色、线型、圆角、旋转） | — |
| svg 图形 | 移动、缩放、改填充 / 描边色 | 逐段编辑（没有路径编辑器） |
| 底图 | 移动、缩放、替换 | 内容不可编辑；可以删掉重画 |
| 页 | 整页移动、复制页（复制组 + 加步骤）、删页、换序（路径编辑）、改页底色、加备注 | — |

## 8. 限制与不做

- 加密 / 需口令的 PDF 不支持；XFA、表单域不进对象（外观流画进底图）。
- 注释（高亮、便签、链接）：`annotationMode: ENABLE` 时外观进底图；第一期不转成批注对象。
- 后端运行见 §5.1：转换器可在 Deno 里跑，但第一期只交付浏览器入口；Agent 侧入口是 P1。
- 全览缩略：第一期文字占位；P1 为页面组加一张 320 px 缩略 poster 并在简化层画它。
- 性能：100 页幻灯片预计转换 30–90 s（算子列表 + 底图渲染是大头），对象数几千到上万，分批提交；一页超过 3 000 条路径走底图。

## 9. 裁决（已定，均按建议值实施）

| # | 问题 | 决定 |
| --- | --- | --- |
| Q1 | 默认模式：可编辑拆解 vs 整页高保真 | 可编辑拆解；高保真为对话框选项与单页降级 |
| Q2 | 文字对象粒度：一段一个 vs 一行一个 | 一段一个（有结构树按结构元素，无则按几何聚类）；对话框可切"按行" |
| Q3 | 文字落到 `shape: text` 还是富文本 Block | `shape: text`（与 draw.io 一致，不污染数据树）；"文档型 PDF → 流式页富文本"留作第二期选项 |
| Q4 | 矢量合并策略与 3 000 路径阈值 | 如 §4.3；阈值可调 |
| Q5 | 一张 Surface 行列排布 vs 每页一张 Surface | 一张 Surface |
| Q6 | 路径背景与转场 | 黑底、`cut` |
| Q7 | 拖放 PDF 的默认行为 | 单个 PDF 弹二选一；多文件按文件插入 |
| Q8 | 是否默认保留原 PDF 资产 | 保留（数据树可见、Agent 可读） |
| Q9 | `font_family` 键（shape 新键，影响所有文字形状） | P1 加，三档 + 具体族名 |
| Q10 | 底图清晰度 | 长边 2560 PNG；高保真档可选 3840 |

## 10. 实施顺序

| 阶段 | 内容 | 验收 |
| --- | --- | --- |
| P0 | `pdfDoc.ts`（CMap / 标准字体打包）；`import-pdf/` 转换器：文字成行成段（几何聚类 + 结构树）、颜色 / 粗斜体配对、图片、矩形 / 线 / svg、底图过滤渲染、三档降级；`pdfImport.ts` + `ImportPdfDialog`；多 plan 同 Surface 网格；路径一次创建；主菜单入口；`classifyFile` 分派与二选一；报告 | 单测：录制的 PPT 导出 PDF、Word 导出 PDF、LaTeX PDF、扫描件（无文字）四种夹具的 PagePlan 快照；e2e：3 页混合比例 PDF → 1 张 Surface、3 个页面组、文字对象可双击改字、图片是 asset、路径 3 步、放映第 2 步遮罩洞 = Frame；中文 PDF poster 不再缺字 |
| P1 | `font_family`；"展开为页面"；书签 / 标题作页名；页面组缩略 poster；"从 PDF 新建" | e2e 覆盖三处入口；全览可见缩略 |
| P2（可选） | 文档型 PDF → 流式页富文本；`Table` → 表格 Block；页内链接可点；活渲染 `pdf-page` Block（整页高保真档的矢量替代） | 另立 TODO |

## 11. 验证

- 单测（deno，`import-pdf/*_test.ts`）：行 / 段聚类、颜色配对、矩形 / 椭圆识别、svg 合并与拆分、底图过滤器选取、页矩形与舞台比例、书签 → 页映射。
- e2e：`tests/aiworkspace/pdf-import.spec.ts`（扩展 `media.spec.ts` 的 `pdf()` 生成器：多页、含文字 / 矩形 / 内嵌图片），覆盖 §10 的验收项；`presentation.spec.ts` 加"导入的路径可直接放映"。
- 手工：真实 PowerPoint / Keynote / Google Slides 导出的 PDF 各一份（含渐变、图标、图表、中文字体），比对导入页与原页截图；100 页的时长与对象数；离线副本准备后可放映。

## 12. 实施回写（2026-10-28）

落点：共享转换器 `src/frame/aiworkspace/import-pdf/`（README 有文件表；Desktop 经 `@pdf/*` 别名引用）；夹具 `src/frame/aiworkspace/fixtures/pdf/`；Desktop `ui/canvas/{pdfImport.ts,ImportPdfDialog.tsx,pdfImportRequest.ts}`、`ui/media/pdfDoc.ts`（由 `pdfPoster.ts` 改名扩充）；e2e `tests/aiworkspace/pdf-import.spec.ts`。

与 §3–§5 的偏差和补充：

1. **转换器分两段**：`extract.ts` 把 pdf.js 的页（鸭子类型，不 import pdf.js）读成纯 JSON（文字项带所在 marked content、精简后的算子列表、字体、渐变、结构树、链接、书签）；`convert.ts` 只读这份 JSON。图片像素与底图渲染由宿主回调 `encodeImage` / `renderRaster` 完成，所以 Deno 单测用录制的 `*.pages.json` 即可。
2. **文字盒对位**：`shape: text` 的标签有 4 × 6 内边距、行高默认 1.25，浏览器字体的行盒把基线放在 `(行高 − 上下伸)/2 + 上伸` 处；盒子按此反推，使每行基线落在 PDF 的位置；宽度留 6 % 给替代字体。为此 `shape` 新增 **`line_height`**（字号倍数，0.8–3）键，与 `font_family` 一起进了 `@drawio/shapes` 的 `readLook` 和属性面板（Q9 提前到 P0）。首行缩进写成 em / en 空格。
3. **段落**：结构树里取**最外层**块角色（P / H1–H6 / LI / TD / Caption / Code…；Chromium 把文字包在 NonStruct 里，NonStruct 透明）；同一结构块内再按"同栏、字号、行距"切分。无结构树时按 §4.1 几何聚类，另允许首行缩进与短末行。行内宽间隙（pdf.js 把大空白写成一个空格项）不再连成一行，超过 1.6 × 字号即分开（页眉左右两端、无结构表格的列）。
4. **文字项与 showText 的配对是多对多**：一个文字项可由多个 showText 画出（Chromium 逐字切字体），所有覆盖它的 run 都从底图过滤掉，颜色 / 渲染模式取重叠最多的那个。
5. **图形**：先填充矩形、紧接同框描边矩形（CSS 边框、Ghostscript 输出）合并为一个带填充和描边的 `rect`；小于 48 pt 的标准形状（图标的零件）可并入相邻 SVG，细线（表格线、下划线）永不并入；渐变以 `gradientTransform` 写在页面坐标里；SVG 元素 id 加每次导入的随机前缀。
6. **图片按绘制比例重采样**：PDF 常把位图非等比拉伸到放置矩形，而 asset Block 按自身比例 contain，所以编码时输出成放置矩形的比例（`ImageRequest.aspect`）；镜像绘制在编码时左右翻转；被矩形裁剪且未旋转的图片按裁剪区编码；被非矩形裁剪、半透明、倾斜过大或解码失败的图片留在底图。长边上限 4096。
7. **底图**：`render({ operationsFilter, background: 'rgba(0,0,0,0)' })` 后扫描 alpha 取包围盒。pdf.js 6.4 给 `canvas` 参数时自己以 `alpha: false` 取 2D 上下文（未画处是不透明黑），所以透明渲染前先以 `alpha: true` 取一次上下文（`getContext` 返回最先创建的那个）——DV 上发现：混合模式的色块曾变成整页黑底图；过滤器在算子列表长度与转换时不一致时放弃（报告 `pdf_raster_failed`），不会画重。整页底图（高保真、文字不可读）不透明时存 JPEG。完全落在页外的残留（排版溢出的字形）不触发底图。
8. **页面组**：页矩形取整数世界单位（与舞台比例严格一致）；Frame 的 `config` 带 `fill`（页底色）、`stroke_width: 1`、`radius: 0`，Desktop 的无标题栏 Frame 现在绘制这三个键，画布上能看到白纸；`PagePlan` 条目新增 `payload`（Frame 的 `presentation.background`），由 `buildOperations` 合并。
9. **页名**：结构树第一个 H / Title → 书签 → "第 n 页"（同时是 Frame 标题和步骤标题）。
10. **原件**：随第一次提交写入新画布的内容文件夹（`page_count` + 首页 poster）；从 PDF 文件 Block "展开为可编辑页面…" 时直接引用已有对象，不再上传。
11. **入口**：主菜单 › 插入 › "导入 PDF…"；拖放 / 粘贴单个 PDF 先问"导入为可编辑页面 / 插入为文件"（多个文件照旧按文件插入）；PDF 文件 Block 工具栏 "展开为可编辑页面…"。需要 `structure` 能力且在线。导入结束切到新画布并全览（视口按当前窗口算好写入 user_state），通知带"开始放映"；有报告项时同时弹报告（也有"开始放映"）。
12. **CMap / 标准字体**：Vite 插件在 dev 中间件里直接读 `node_modules/pdfjs-dist`，构建时 emit 到 `aiworkspace/pdfjs/`（含三个 LICENSE），**不进 Service Worker 预缓存**（离线时 CJK 预定义编码的 PDF 仍缺字，联网即可）。
13. **上传**：每页转换完即上传该页图片（按内容 SHA-256 去重、3 路并发）并释放字节；确认前取消不写任何实体（已上传的孤儿对象由存储回收）。

## 13. 验证（2026-10-28）

- `cd src/frame/aiworkspace/import-pdf && deno test -A`：17 过（四个夹具的 PagePlan 金样；段落 / 行高 / 首行缩进 / 表格单元 / CJK / 字族；矩形、椭圆、旋转线、填充 + 描边合并；底图过滤集合；不可读文字、高保真、扫描件三种降级；`buildOperations` 产物；布局；SVG 超限；op 码与 pdf.js 一致；录制数据与当前提取一致）。`deno lint` 干净。
- 真实 PDF 抽测（`tools/record.ts` 同款脚本，不在仓库）：pdfTeX 的 685 页手册 0 异常、提取 + 转换约 4.7 s、12 页需底图；Ghostscript、PDF-XChange（Word 图表 3 506 条路径 → 图形进底图）、Chromium 输出均正常。
- Desktop：`pnpm check`、改动文件 eslint、`pnpm build` 通过；i18n 守卫除 HEAD 既有的两项（`canvas.catalog.sample/extension` 未用、`store/src/wish/outline.rs` 的中文字面量）外全过。
- e2e：`pdf-import.spec.ts` PI-01（菜单导入 3 页混合比例 → 1 张画布、3 个页面组、Frame 在首位、文字可双击改字、图片是 asset、原件保留、路径 3 步 cut 黑底 16:9、放映第 2 步遮罩洞 = 页、竖版页补边、中文经 CMap 读出）与 PI-02（拖放二选一、取消不写、文件 Block 展开且复用原件）通过；回归 `media / drawio / presentation`（35 例）与 `offline / offline-presentation`（11 例）通过。

### 13.1 真实 Zone DV（2026-10-28，`test.buckyos.io`，单 OOD）

- 部署：只改了 Desktop（无 Rust 改动；Zone 上的 aiworkspace / control-panel 是 14:30 由 HEAD 部署的），`uv run buckyos-build.py -s desktop` 后把 `rootfs/bin/control-panel/web/` 同步到 `/opt/buckyos/bin/control-panel/web/`；原 web 目录备份在 `/home/lzc/project/deployment-backups/20261028-pdf-import/web`。网关能取到 `aiworkspace/pdfjs/cmaps/*.bcmap`、`standard_fonts/*`，`sw.js` 不含它们。
- 浏览器：`BUCKYOS_UI_DV_BASE_URL=https://test.buckyos.io BUCKYOS_TEST_ADMIN_PASSWORD=… pnpm exec playwright test --config=playwright.dv.config.ts tests/dv/aiworkspace.pdf.spec.ts`，真实登录 devtest，三例全过、无浏览器错误、测试工作区自动删除：

| 用例 | 验到的事实 |
| --- | --- |
| 自造 3 页 PDF | 预估「2 段文字、1 张图片、1 个图形」；中文经 Zone 提供的 CMap 读出；第 3 页混合模式色块成为裁到色块大小的底图（报告列出 p3）；原件保留；路径 3 步 cut；图片与底图从 Zone 存储加载；刷新后可放映；转换 + 上传 + 提交 1.4 s |
| Chromium 幻灯片夹具 | 中英标题白色、3 个 LI、渐变 SVG、页底色 `#1e3a5f`；第 2 页圆、折线、半椭圆和多行段落与原页一致 |
| pdfTeX 手册（6 MB，第 21–28 页） | 8 个页面组、285 个对象，页名取书签（Introduction、Central Management…，目录页为 Page n）；6 MB 原件上传保留；1.2 s；报告 62 处混排样式（目录点线与页码） |

- **DV 发现并修正**：上面第 7 条的不透明画布问题（本地 e2e 原先没有需要底图的页，没测到）；`tests/aiworkspace/pdfFiles.ts` 的自造 PDF 第 3 页加了 Multiply 色块，PI-01 断言底图只裁到色块，作为回归。

## 14. 未做 / 风险

- P1 其余：页面组缩略 poster（全览时文字仍是占位）、"从 PDF 新建作品"、Agent 入口 `aiws import-pdf`（转换器已可在 Deno 跑，见 §5.1）。
- 行内混排只保留主样式（报告 `pdf_mixed_style`）；数学公式、上下标会拆成多个小文字对象；竖排文字进底图。
- 字体只到三档族名，字距与 PDF 不同；替代字体更宽时文字可能超出盒子（形状 `overflow: visible`，不会被裁）。
- 底图依赖 `page.render` 与 `getOperatorList` 产出同一算子列表（实测一致，不一致时放弃底图并报告）。
- 未用 PowerPoint / Keynote / Google Slides 原生导出的 PDF 实测（机器上没有），100 页幻灯片的时长与对象数未测；DV 只测了单 OOD、桌面浏览器。

