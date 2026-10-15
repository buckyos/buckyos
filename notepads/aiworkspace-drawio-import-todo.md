# AI Workspace draw.io 导入 TODO

日期：2026-10-14

目标：把 draw.io（diagrams.net，mxGraph 模型）文件导入 AI Workspace 的自由画布，图形、连线、容器、分页尽量保真；不能保真的部分明确降级并保留原始样式，便于以后导出回写。本文按本次源码核对列出现状、需要新增或扩展的标准对象、映射规则和分期 TODO。

**状态（2026-10-14）：D0–D5 已实施。** §6 按推荐项拍板；与本文原设想不同的地方见 §8 实施回写，验证见 §9，未做项与风险见 §10。

## 0. draw.io 模型速览

| 概念 | 文件中的样子 | 说明 |
| --- | --- | --- |
| 文件 | `<mxfile>` → 多个 `<diagram name=… id=…>` | 一个 `diagram` 是一页；内容是 `<mxGraphModel>` 明文，或 deflate(raw)+base64+URL 编码后的文本（默认压缩） |
| 载体 | `.drawio` / `.xml`；`.drawio.png`（PNG `tEXt` 块 `mxfile`，URL 编码）；`.drawio.svg`（`<svg content="…">`，HTML 转义）；`.xml` 图形库 `<mxlibrary>` | 复制粘贴时剪贴板里也是 `<mxGraphModel>` 文本 |
| 单元 | `<mxCell id value style vertex=1|edge=1 parent source target collapsed>` + `<mxGeometry x y w h relative as="geometry">` | id `0` 是根，`1` 是默认图层；`parent=0` 的 cell 是图层；顶点可以作为其他顶点的父（分组、容器、泳道），子几何相对父 |
| 带数据的单元 | `<UserObject label=… link=… tooltip=… 任意属性><mxCell …/></UserObject>`（或 `<object>`） | "编辑数据"、链接、占位符 `%name%`、标签 |
| 样式 | `key=value;` 串，首段可以是形状名 | 顶点：`shape rounded arcSize fillColor strokeColor strokeWidth dashed dashPattern opacity gradientColor fontColor fontSize fontFamily fontStyle(位) align verticalAlign spacing* whiteSpace html rotation flipH flipV aspect image imageAspect container collapsible swimlane startSize horizontal points(连接点) locked noLabel`；边：`edgeStyle(none / orthogonalEdgeStyle / elbowEdgeStyle / entityRelationEdgeStyle / …) elbow curved rounded exitX exitY exitDx exitDy exitPerimeter entryX entryY … startArrow endArrow startFill endFill startSize endSize jumpStyle flowAnimation` |
| 边几何 | `<mxGeometry relative=1>` 内 `<mxPoint as="sourcePoint|targetPoint">`（悬空端）、`<Array as="points">`（绝对坐标折点）、`<mxPoint as="offset">` | 边标签是 `edgeLabel` 样式的子 cell，几何 `relative=1`，`x ∈ [-1, 1]` 为沿线位置，`y` 为法向偏移 |
| 形状来源 | 内置几何名（`rectangle ellipse rhombus triangle hexagon parallelogram trapezoid cylinder3 cloud document process card note callout cross step singleArrow doubleArrow line text image label swimlane table tableRow …`）；`shape=mxgraph.<lib>.<name>` 内置图形库；`shape=stencil(<deflate+base64 的 mxStencil XML>)` 内嵌模板 | `mxgraph.*` 一部分是 XML 模板（可解释），一部分是 JS 绘制（aws4、azure 等很多），无法从文件还原 |

## 1. 现状对照（路径相对 `buckyos/`）

| 范围 | 已有 | 对 draw.io 的缺口 |
| --- | --- | --- |
| 形状 | [builtin.tsx](../src/frame/desktop/src/app/aiworkspace/ui/blocks/builtin.tsx) `shape`：`config.shape ∈ rect/ellipse`、`fill`、`stroke`；文字是 `title` 单行；`chrome: 'none'`；锚点按 `rect/ellipse` 轮廓的 16 方位（[anchors.ts](../src/frame/desktop/src/app/aiworkspace/ui/canvas/connectors/anchors.ts)） | 只有两种几何；没有描边宽度、线型、透明度、圆角、无填充/无描边、字号/字色/粗斜体/对齐、多行文字、翻转、方向；自定义连接点不能随对象声明 |
| 文字 | 富文本 Block 需要一个 `buckyos.richtext` 数据实体（[catalog.ts](../src/frame/desktop/src/app/aiworkspace/ui/canvas/catalog.ts) `needs`）；`config.text_size/text_color` | draw.io 一张图几十上百个纯文字标签，每个都建数据实体会污染数据树；没有“纯 UI 文字”对象 |
| 框 / 容器 | `frame`（标题 + 颜色，纯 UI，不是父容器）；`group` 容器，子 `placement` 相对父（[plan.rs](../src/frame/aiworkspace/core/src/plan.rs) `child_allowed`） | 没有泳道（标题条 + 横/纵向、嵌套 lane、堆叠布局）；`frame` 不能承载子对象 |
| 图片 | `asset` Block + `buckyos.asset-ref` 上传（[CanvasView.tsx](../src/frame/desktop/src/app/aiworkspace/ui/canvas/CanvasView.tsx) 只接受本地文件） | 导入时要从 data URI / 外链 / `.drawio.png` 自身取图；`label` 形状（图标 + 文字）没有对应 |
| 连接线 | [model.ts](../src/frame/desktop/src/app/aiworkspace/ui/canvas/connectors/model.ts)：`route ∈ straight/elbow/curve`、`controls`（端点系比例 + 偏移）、端点 = 坐标或具名锚点、15 种线帽（含 ER）、`dash/width/stroke/opacity/rounded`、一个标签 `{t, offset}` 文字取 `title`；core 校验见 [plan.rs](../src/frame/aiworkspace/core/src/plan.rs) `check_placement` | 没有 0–1 数值锚点（v0.4 只留具名锚点）；折点是绝对坐标要换算；线连线不支持；多标签、标签富文本、线帽尺寸、跳线、流动动画没有；`elbow` 的起始方向（`elbow=vertical`）没有 |
| 页 | 一个 Surface 一张画布，有 `title`、`icon` | 没有页背景色、页面尺寸、网格；draw.io 多页 → 多 Surface 需要一次导入建多个 Surface |
| 图层 | 无 | draw.io 图层有可见/锁定；画布没有“隐藏”键，只有 `locked` |
| 元数据 | Cell `config` ≤ 64 KiB（[types.rs](../src/frame/aiworkspace/core/src/types.rs) `MAX_CONFIG_BYTES`）；批注 `target` 可指向任意对象 | 没有对象链接（`link`）、提示（`tooltip`）、自定义属性（“编辑数据”）的落点 |
| 导入入口 | `ws.import` 只认 aiws 自己的包（[service.rs](../src/frame/aiworkspace/store/src/service.rs) `import`）；Markdown → 富文本（[markdown.rs](../src/frame/aiworkspace/core/src/markdown.rs)）；画布剪贴板只在 Workspace 内（[clipboard.ts](../src/frame/desktop/src/app/aiworkspace/ui/canvas/clipboard.ts)） | 没有任何第三方格式的解析、解压与映射；一次 commit 最多 10 000 个操作（`max_ops`），大图要分批 |
| 自定义图形 | HTML 扩展 Block、`Cell.config.snapshot` 静态预览、`asset` 能显示 SVG 文件但不能改色和拉伸 | 没有可参数化、可拉伸、带锚点的 SVG 形状对象；`mxgraph.*` 图形库和 `stencil(...)` 无处承载 |

## 2. 需要新增或扩展的标准对象

按“先能力后安全”和“对象 = 内容，宿主 = 状态”的既定原则；所有新键都是 Cell `config`，由前端注册表校验，core 只限制大小，不改 `FORMAT_VERSION`。

这些对象是常规画布的标准对象，导入器只是生产它们的一个来源；工具栏、属性面板、剪贴板、许愿格走同一套定义。每节末尾的“手工入口”列出离开导入器后用户自己用起来还要补的交互，与渲染器同阶段交付，不做成只有导入才能产生的东西。

### 2.1 扩展 `shape`（P0）

- [x] `config.shape` 扩成基本几何集，首批对应 draw.io 内置形状：`rect`、`ellipse`、`diamond`（rhombus）、`triangle`、`hexagon`、`parallelogram`、`trapezoid`、`cylinder`、`cloud`、`document`、`process`（双边矩形）、`card`、`note`（折角）、`callout`、`cross`、`step`（chevron）、`arrow`（singleArrow）、`double_arrow`、`line`、`text`（无填充无描边的纯文字）。每种给出轮廓路径和锚点表（`anchors.ts` 的 `Shape` 从 `rect | ellipse` 扩成按轮廓投影）。
- [x] 外观键：`stroke_width`、`dash`（复用连接线取值）、`opacity`、`radius`（圆角，`rounded/arcSize`）、`fill: null` 与 `stroke: null` 表示无填充/无描边、`direction ∈ e/s/w/n`（三角、箭头、chevron 的朝向）、`flip_h`/`flip_v`。
- [x] 文字键：`config.text`（多行纯文本，替代只有单行 `title`；`title` 继续作为大纲名）、`font_size`（数值，世界单位）、`font_color`、`bold/italic/underline`、`align ∈ left/center/right`、`valign ∈ top/middle/bottom`、`label_position ∈ inside/outside-bottom/…`、`wrap`。形状内富文本留到后续，不在本 TODO 内。
- [x] 对象自带连接点：`config.anchors: [{ id, x, y }]`，`BlockDefinition.anchors` 读取后作为“own set”提供给连接线，用来承载 draw.io 的 `points=[[x,y],…]` 和边上精确的 `exitX/exitY`。
- [x] **手工入口**：形状菜单从“矩形/椭圆”二选一改成分组的图形选择器（插入目录和对象工具栏共用）；工具栏加描边宽度、线型、透明度、圆角、字号、字色、粗斜体、对齐；属性面板补齐全部新键；`text` 形状进插入目录“文字”项，插入后直接进入编辑。

### 2.2 泳道 / 可承载子对象的框（P1）

- [x] 决定实现方式（§6 Q3）。推荐：导入时把 draw.io 容器（`swimlane`、`container=1`、`group`）映射为 aiws `group`，`group` 内第一个子对象是一个覆盖整个范围的 `frame`，其余子对象相对放置；`frame` 增加 `config.header`（标题条高度，0 表示只有边框）和 `config.orientation ∈ horizontal/vertical`（纵向标题），嵌套 lane 递归同构。不新增父容器类型。
- [x] 折叠（`collapsed=1`）按展开导入，记录原始折叠尺寸到 `config.drawio`。
- [x] **手工入口**：插入目录加“泳道”项，一次建好 `group + frame` 结构，可选横/纵向和 lane 数；工具栏加“添加 lane / 删除 lane”；把对象拖入或拖出 `group`/泳道范围时自动改父（现在画布没有这个行为，不做的话手工画泳道会很别扭，分组同样受益）。

### 2.3 图片（P1）

- [x] 导入器能把 data URI 图片上传为 `buckyos.asset-ref`；外链图片能取到就取，取不到用带链接文字的 `shape` 占位。
- [x] `label` 形状（左图右文）映射为 `group`（asset + text 形状）。
- [x] **手工入口**：无需新增，`asset` 已有文件选择；可顺带让画布接受拖入图片文件和粘贴剪贴板图片（导入器取图的那条上传路径复用）。

### 2.4 SVG 矢量形状（P2）

渲染侧的标准对象直接是 SVG，不做运行时的 mxStencil 解释器；mxStencil 只在导入期（`stencil(...)`）和构建期（内置图形库）被一次性转成 SVG。这样用户粘贴任意 SVG、许愿格让模型生成图形，都走同一个对象。

- [x] 新 `config.shape = 'svg'` + `config.svg: { viewBox, body, aspect: 'stretch' | 'fixed' }`。`body` 是 SVG 内容片段，受 64 KiB 限制；超限落到 asset SVG（只是图片：不能改色、没有锚点）。
- [x] 颜色参数化：转换时把 stencil 的 `fillstroke/fill/stroke` 写成 `fill="var(--aiws-shape-fill)"`、`stroke="var(--aiws-shape-stroke)"`（或 `currentColor`），`strokewidth` 与 `alpha` 同理，让 §2.1 的填充、描边、线宽、透明度工具栏对 SVG 形状照样生效；stencil 内写死的颜色原样保留。
- [x] 拉伸方式：`aspect: 'stretch'`（stencil 默认）→ `preserveAspectRatio="none"`，`'fixed'` → `xMidYMid meet`；描边加 `vector-effect: non-scaling-stroke`，避免非等比拉伸后线条变形。
- [x] 连接点：stencil 的 `<connections>` 转成 §2.1 的 `config.anchors`，没有的按包围盒给 16 方位。
- [x] 渲染为内联 SVG（`<img>` 传不进 CSS 变量）；插入前剥掉 `script`、`on*` 属性、`foreignObject` 和外部引用，成本很低，按“先能力后安全”记为已知风险。
- [x] 导入期转换器 mxStencil → SVG，支持子集：`path/move/line/quad/curve/arc/close/rect/roundrect/ellipse/text`、`fillstroke/fill/stroke/strokewidth/fillcolor/strokecolor/alpha/dashed`、`<background>/<foreground>`、`aspect`、`<connections>`。不支持的指令整段降级为矩形 + 名称标签并保留样式串。
- [x] `shape=mxgraph.<lib>.<name>`：构建期脚本把 jgraph/drawio 仓库（Apache-2.0）`stencils/*.xml` 的子集（flowchart、basic、bpmn、uml、er、network）预转成 SVG，按库拆包按需加载，运行时不认识 mxStencil；JS 绘制的库（aws4、azure、gcp、cisco 大部分）无法还原，降级为矩形 + 名称标签。许可证与体积在落地前核对。
- [x] 与 `asset` 的分工：draw.io 里 `shape=image;image=data:image/svg+xml,…` 本来就是图片的仍映射到 `asset`，不改色、不拉伸；只有 stencil 和图形库形状走 SVG 形状对象。
- [x] **手工入口**：插入目录加“SVG 形状”项，支持粘贴 SVG 文本/拖入 `.svg` 文件直接成形状（本来是图片的大 SVG 走 asset）；图形库面板按库浏览、搜索、点击或拖放插入；“编辑连接点”模式可后置，先用 `config.anchors` 表单。这是用户最直接感到能力变强的一块，D4 把面板和导入一起交付。

### 2.5 页与图层（P1）

- [x] Surface 增加 `config.background`（颜色）与可选 `page_size`；多页导入一次建多个 Surface，页名作 `title`。
- [x] 图层：推荐拍平进 z 顺序，`locked=1` 的图层给每个对象写 `locked: true`；隐藏图层默认跳过并在导入报告里列出；原图层名记入每个对象的 `config.drawio.layer`。是否需要“隐藏”作为对象标准键见 §6 Q4。
- [x] **手工入口**：Surface 管理面板加背景色设置；图层不做手工功能。

### 2.6 元数据（P2）

- [x] Cell 增加 `config.link`（URL 或 Workspace 对象引用）与 `config.tooltip`，hover 附件显示。
- [x] UserObject 自定义属性：推荐写成 `config.data: { k: v }`（字符串键值），不建 Record 实体；占位符 `%name%` 替换为静态值。
- [x] 所有对象保留 `config.drawio: { style, id, layer? }`，为导出回写和二次修正留原始信息。
- [x] **手工入口**：属性面板加链接、提示和键值数据表单；hover 附件显示链接按钮和提示。

### 2.7 连接线（P0 随转换器一起）

- [x] 边上多标签：第一个成为连接线 `title + label`，其余导入为跟随连线中点位置的 `text` 形状（不跟随，导入报告列出）。
- [x] `elbow` 起始方向键 `config.elbow ∈ horizontal/vertical`（可选，没有就按现有自动规则）。
- [x] 线帽尺寸、跳线、流动动画、线连线：不支持，降级并记录；线连线端点落为坐标。

## 3. 转换器与导入入口

- [x] 位置（§6 Q1）。推荐：一个无依赖的 TypeScript 模块 `src/frame/aiworkspace/import-drawio/`（XML → aiws 操作列表 + 导入报告），Desktop 和 aiws 程序宿主（Deno）都能用；连接线控制点换算需要的几何从 Desktop `connectors/geometry.ts` 抽到该模块。服务端 Rust 暂不做，`ws.import` 不改。
- [x] 解析：`<mxfile>`/`<mxGraphModel>` 明文；压缩页 base64 → inflateRaw（浏览器 `DecompressionStream('deflate-raw')`，Deno 同）→ `decodeURIComponent`；`.drawio.png` 读 `tEXt`/`zTXt` 的 `mxfile`；`.drawio.svg` 读根 `content` 属性；`<mxlibrary>` 作为“插入图形”来源（P2）。
- [x] 顶点映射：XML 顺序 → `order_key`；父子 → `group` 嵌套，几何相对父；`rotation` 归一到 `[0, 360)`；`style` 按 §2.1 键表映射，未知键原样进 `config.drawio.style`；`html=1` 的标签先去标签保留换行与 `<b>/<i>/<u>/<font color>` 转对应键，列表等复杂 HTML 退化为纯文本；整图包围盒平移到正象限并留边距。
- [x] 边映射：`edgeStyle` → `route`（无/`none` 带折点 → `straight` + 折点；`orthogonalEdgeStyle`/`elbowEdgeStyle` → `elbow`；`curved=1` 或 `entityRelationEdgeStyle` → `curve`）；绝对折点 → 端点系比例控制点；`exitX/exitY/entryX/entryY` → 先匹配对象已有锚点，否则给对象追加 `config.anchors` 条目并绑定；没有约束的浮动端按两对象中心连线投影到最近的 16 方位锚点；`sourcePoint/targetPoint` → 坐标端；`startArrow/endArrow × startFill/endFill` 的映射表（`classic/block/blockThin → triangle | triangle_outline`、`open/openThin → arrow`、`oval → circle*`、`diamond/diamondThin → diamond*`、`dash → bar`、`ERone/ERmany/ERmandOne/ERzeroToOne/ERzeroToMany/ERoneToMany → cardinality_*`，其余 → `arrow`）；`dashed/dashPattern` → `dashed|dotted`；边标签几何 `x ∈ [-1,1]` → `t = (x+1)/2`，`y` → `offset`。
- [x] 提交：对象先于连接线；每批不超过 `max_ops`（留余量，按 2 000 一批）；多批共用一个导入标签；报告列出降级项（未知形状、丢失的标签、未取到的图片、跳过的隐藏图层）。
- [x] 入口：插入目录“导入 draw.io…”（文件选择）、把 `.drawio/.xml/.drawio.png/.drawio.svg` 拖到画布、粘贴 `<mxGraphModel>` 文本；导入到当前 Surface（单页）或新建 Surface（多页）。
- [x] 测试：fixtures 放 `src/frame/aiworkspace/fixtures/drawio/`（流程图、泳道、ER、UML 类图、带图片与压缩页的多页文件、`.drawio.png`、含 `stencil(...)` 的图），单元测试比对操作列表；e2e 导入后截图核对浅色与深色主题。

## 4. 分期

| 阶段 | 内容 | 完成门槛 |
| --- | --- | --- |
| D0 | 拍板 §6；定键名与形状清单；写 fixtures | 每个 draw.io 样式键都能指出映射或降级 |
| D1 | §2.1 形状扩展 + `text` + `config.anchors`；§2.7 连接线补键；图形选择器与工具栏新键 | 不用导入器，手工能画出 draw.io 基本图形库里的图 |
| D2 | §3 转换器 + 文件/拖拽/粘贴入口 + 分批提交 + 导入报告 | 流程图、ER、泳道样例导入后与 draw.io 截图对照，连线绑定正确 |
| D3 | §2.2 泳道/容器 + 拖入自动改父、§2.3 图片 + 拖入/粘贴、§2.5 页与图层 | 多页、嵌套 lane、图片样例通过；手工能从插入目录建泳道并拖对象进出 |
| D4 | §2.4 SVG 形状对象 + 粘贴 SVG/图形库面板 + 导入期 mxStencil → SVG 转换 + 构建期图形库预转换 | 粘贴任意 SVG 可成形状并改色；bpmn/uml/network 样例的 XML 模板形状可渲染且连线能吸附 |
| D5 | §2.6 元数据 + 属性面板表单；导出回写 draw.io（反向映射，用 `config.drawio` 补全） | 导入再导出的文件在 draw.io 中打开布局一致 |

## 5. 已知有损项（导入报告须列出）

- JS 绘制的 `mxgraph.*` 图形库形状、渐变、阴影、手绘/漫画风格、`glass`。
- 线连线、线帽单独尺寸、跳线、流动动画、多标签跟随、标签富文本。
- 复杂 HTML 标签（表格、列表、嵌套字体）。
- draw.io `table/tableRow` 形状：首版按形状分组导入，不转表格数据实体。
- 外链图片取不到、隐藏图层、折叠容器。

## 6. 需要拍板

| 编号 | 问题 | 推荐 | 结论（2026-10-14 实施时按推荐落地） |
| --- | --- | --- | --- |
| Q1 | 转换器放 Desktop、共享 TS 模块还是 core Rust（wasm + 服务端） | 共享 TS 模块；Agent 通过 aiws 程序宿主调用；服务端暂不做 | 已定：`src/frame/aiworkspace/import-drawio/`，Desktop 经 `@drawio/*` 别名、Deno 直接引用；程序宿主接入未做（§10） |
| Q2 | 纯文字是新 Block 类型还是 `shape: 'text'` | `shape: 'text'`，少一套定义 | 已定：`shape: 'text'`，插入目录有独立的“文字”项（目录变体，§8-3） |
| Q3 | 泳道是新父容器类型还是 `group + frame` | `group + frame`，`frame` 加 `header/orientation` | 已定；另加“底板”语义（§8-6） |
| Q4 | 是否给对象加标准 `hidden` 键承载隐藏图层 | 暂不加；隐藏图层跳过并报告 | 已定：不加 |
| Q5 | `exitX/exitY` 精确约束：对象追加 `config.anchors` 还是吸附到 16 方位 | 追加 `config.anchors`，保真且不改协议 | 已定；与已有锚点重合的点不再追加（§8-9） |
| Q6 | 内置图形库预转成 SVG 后是否随 Desktop 打包（体积、许可证） | 按库拆分按需加载；先 flowchart/basic/bpmn/uml/er | 已定：flowchart/basic/arrows/bpmn/networks 五个包共约 190 KiB；uml/er 在 draw.io 中不是模板库（§8-12） |
| Q8 | 自定义矢量形状用 SVG 对象还是运行时 mxStencil 解释器 | SVG 对象（2026-10-14 已定）；mxStencil 只在导入期和构建期转换 | 已定 |
| Q7 | 导入是否同时支持导出回写 | 先导入；`config.drawio` 从 D2 起保留，为 D5 铺路 | 已定：D5 导出已实施 |

## 7. 参考

- 连接线方案的主流工具对照：[连接线实现方案讨论 §5](../doc/workspace/连接线实现方案讨论.md)
- 标准对象状态与工具栏契约：[标准对象的交互改进](../doc/workspace/标准对象的交互改进.md)
- 两棵树、`group`、`placement`、`locked`、`config` 限制：[aiworkspace README](../src/frame/aiworkspace/README.md)
- draw.io 文件格式与样式键：jgraph/drawio 仓库 `src/main/webapp/js/grapheditor/Graph.js`、`mxgraph/mxConstants.js`、`stencils/*.xml`；mxGraph 文档的 mxStencil 说明

## 8. 实施回写（2026-10-14）

实施中与上文设想不同、或上文没有写到的决定：

- 8-1 **“无填充 / 无描边”写 `'none'`，draw.io 的 `default` 写 `'auto'`**：`setCellConfig` 把 `null` 当作删除键（会退回默认色），所以不用 `fill: null`。`'auto'` 是跟随主题的颜色（填充 = 画布表面色，描边和文字 = 正文色），深色主题下导入的默认白底黑线图也能看清；导出写回 draw.io 的 `default`。连接线的 `default` 描边不写（用连接线自己的默认色）。
- 8-2 **补充的形状键**：`shape_size`（draw.io 的 `size`，各形状含义同 draw.io）、`label_fill`（`labelBackgroundColor`）。`text` 缺省时用 `title` 显示；编辑文字时，`title` 原来为空或等于旧文字的第一行，就跟着改成新文字的第一行（大纲名不失效）。
- 8-3 **目录变体**：`CatalogInfo.variants` 让一个定义出多个插入项（形状 → “文字”“SVG 形状”，框 → “泳道”），带预置配置、尺寸和插入前选项（泳道方向、条数）。
- 8-4 **轮廓**：连接线几何的 `Shape` 扩成 `rect | ellipse | { poly }`，多边形是形状路径的采样；悬停和吸附轮廓按多边形画。有绝对尺寸的部分（便笺折角、圆柱帽）按 160 × 120 量锚点。
- 8-5 **几何与锚点代码移到共享模块**：Desktop 的 `connectors/anchors.ts`、`connectors/geometry.ts` 只剩 re-export，导入器与画布用同一套函数。
- 8-6 **组的“底板”**：组的第一个子对象是覆盖整个组的框或形状时，它是组的外观。在画布上按它等于按组，所以选中、移动的是整条泳道或整个容器。改组大小时底板跟着改，工具栏和属性面板改的是底板，双击改底板标题。没有标题的组不再显示“分组”字样。
- 8-7 **自动改父**：拖动结束、以及在画布上点击放置新对象时，对象中心落在哪个（未锁定的）组里就进哪个组，落在所有组外就回到画布。适用于所有分组，不只泳道。
- 8-8 **表格**：draw.io `table` 按带标题栏的框导入（`header = startSize`），`tableRow` 成组；行和单元格的部分边框不画（计入报告）。
- 8-9 **连接点约束**：`exitX/exitY` 落在已有锚点（16 方位优先）时用已有 id，否则追加 `xy_<x>_<y>`。draw.io `points=` 中与轮廓锚点重合的点不追加；模板自带的具名连接点（`top/left/…`）保留其名。这样导入、导出、再导入后端点 id 不变。
- 8-10 **浮动端**：直线和曲线取离 draw.io 交点最近的锚点。直角线两端都浮动时：上下排列的对象上下相连，左右排列的对象左右相连，斜向的对象“横出竖入”，与 draw.io 的正交路由一致。`portConstraint` 优先。`elbow=vertical` / `elbowEdgeStyle` 体现在端点选边上，`config.elbow` 只为导出保留，路由本身不读它。
- 8-11 **draw.io 脚本绘制的图形**：常用内置形状（actor、umlActor、tape、delay、manualInput、cube、folder、component 等 22 种）写成 SVG 形状体（`builtinSvg.ts`）。BPMN 的 task/event/gateway 等按最接近的基本几何近似（内部图标丢失）。其余 `mxgraph.*` 按矩形 + 名称导入。三种情况都计入报告。
- 8-12 **图形库**：draw.io 现在把模板集打包在 `js/stencils.min.js` 里（二进制编码，已不是 `stencils/*.xml`）。构建脚本移植了它的解码器。UML、ER 在 draw.io 中是脚本形状，没有模板集，所以实际打包 flowchart、basic、arrows、bpmn、networks。draw.io 的模板 LICENSE 只限制在 Atlassian 产品中使用，与 BuckyOS 无关，许可说明随包放在 `LICENSE.txt`。Desktop 的 service worker 会预缓存这些包（约 190 KiB）。
- 8-13 **`<mxlibrary>`**：作为“插入图形来源”时，图形库文件的各项并排导入到当前画布；没有做成图形库面板里的自定义库。
- 8-14 **页面**：Surface 的 `config` 是 core 新增的键（仅 Surface，≤ 64 KiB，不进大纲），`FORMAT_VERSION` / `PROTOCOL_VERSION` 未升（开发期不兼容旧内核）。背景色在画布图标弹层设置。画布有自己的背景色时，按其明暗套用浅色或深色主题。`page_size` 只保存、导出，不在画布上画页面框。
- 8-15 **导入位置**：单页导入到当前自由画布的视图中央。若会盖住已有对象，就放到已有内容右侧。多页文件、或当前是流式页时，每页新建一张自由画布（名称、背景、页面尺寸来自页）。导入报告放在画布视图之外（切换画布不会丢）。
- 8-16 **粘贴与拖放**：画布复制、剪切时向系统剪贴板写入标记；粘贴时系统剪贴板里是外来内容（文件、图片、SVG、draw.io 模型，含 URL 编码）就导入外来内容，是标记或空就粘贴画布对象。拖入的图片按原尺寸（≤ 480）插入，SVG 成为可改色的 SVG 形状（> 48 KiB 时按图片插入）。
- 8-17 **配置写入串行化**：`setCellConfig` 改为同一 Block 的写入排队，每次基于最新配置合并（快速连点工具栏时第二次写不再因版本过期被拒）。键入打开编辑器的期间（Block 还在加载）敲下的字会补进编辑器。
- 8-18 **导出**：主菜单“导出为 draw.io…”导出当前自由画布。导入时保留的 `config.drawio.style` 是起点：画布管理的键按当前外观重写，未知键原样保留。连接线导出 `exitX/entryX`（锚点位置，`perimeter=0`），直角线导出所有拐点。SVG 形状若来自图形库或内嵌模板，写回原来的 `shape=`；其他 SVG 形状以 SVG 图片写出。富文本、便签、表格等没有对应的对象，导出为带文字的虚线框并计数提示。`link`、`tooltip`、`data` 写成 `UserObject`。
- 8-19 **元数据**：所有 Cell 的属性面板都有“链接、提示与数据”。链接可以是网址或 `#<对象 id>`，悬停时出现链接按钮；提示用作原生 tooltip。

## 9. 验证（2026-10-14）

- 共享模块：在 `src/frame/aiworkspace/import-drawio` 下运行 `deno test -A`，34 个用例全部通过：
  - 每个 fixture 的计划与 `fixtures/drawio/expected/*.plan.json` 一致；
  - 三种载体（.drawio、.drawio.png、.drawio.svg）读出同一模型；
  - 操作顺序与 order key 合法；
  - 5 000 个兄弟 key 长度 ≤ 12；
  - 标签、样式、形状锚点、SVG 清理均符合预期；
  - 六个 fixture 导出再导入后几何、绑定和外观一致；
  - 模板转换在真实 draw.io 包上通过。
- core：`cargo test -p aiworkspace-core -p aiworkspace-store -p aiworkspace` 通过，新增 `surface_config_holds_page_settings`。WASM 已用 `wasm/build.sh` 重建。
- Desktop：`pnpm check`、`pnpm exec eslint src/app/aiworkspace tests/aiworkspace` 均无错误。
- e2e：新增 `tests/aiworkspace/drawio.spec.ts`，DR-render、DR-01…DR-11 共 12 个用例全部通过。截图在 `test-results/aiworkspace-drawio/`，包括浅色和深色主题。
- AI Workspace 全量 e2e（`playwright.aiworkspace.config.ts`）：159 个通过，1 个失败。
  - 失败的是 CN17（`connectors.spec.ts:310`，期望 8 个吸附锚点，实际 16 个）。未改动的 HEAD 上同一断言同样失败，与本次改动无关。
  - 菜单变长后 V15/V16、UI-P01、离线快照等用例的最后一项菜单落到窗口外，因此“导入 draw.io…”放进「插入」子菜单，导出改为「导出」子菜单（`presentation.spec.ts` 同步改了一行）。
  - 中途 V16 存储失败、离线撤销两个用例曾连续失败，现象是“Worker 载入或运行出错”和 reload 时 `ERR_CONNECTION_REFUSED`。当时 HEAD 工作树在并行跑 e2e；重新构建后全部 18 个离线用例通过，两次复跑都没再出现。
- 与 draw.io 对照：用 draw.io 官方 viewer（`viewer-static.min.js`）渲染 flowchart、swimlane、er、uml、stencil 五个 fixture，与导入后的截图逐一对照。布局、连线走向、标签位置、模板形状外观一致；差异都已列入 §5 的有损项。

## 10. 未做与风险

- **程序宿主（Q1 的 Agent 调用）**：模块可在 Deno 中直接运行，但许愿格程序的结果目前只能是数据（表格、文字、记录、文件），还不能写画布对象，所以 Agent 还不能经程序宿主导入 draw.io。要做需新增一种画布结果，并在 Rust 侧规划。
- **“编辑连接点”模式**：未做，目前只有属性面板的连接点表单（本文原已写“可后置”）。
- **页面框与图层**：页面框不画；隐藏图层不导入（Q4）。
- **撤销**：分批导入时每批是一个撤销步骤。
- **SVG 形状**：模板的描边宽度不随形状放大而加粗（draw.io 会按缩放比例放大）；非等比拉伸时模板内的文字和圆角会变形。
- **内联 SVG 的安全**：存储前和绘制前都会清理（去掉脚本、事件属性、foreignObject 和外部引用）。剩余风险按“先能力后安全”记录在此。
- **未在 DV / 真实 Zone 验证**：本次改动 Surface 校验（`config`），部署需重建后台与 WASM。
- **文字编辑**：用 `contenteditable="plaintext-only"`，Firefox 需 136 及以上版本。
- **深色主题下的显式浅色**：draw.io 文件里写死的浅色（如泳道的浅色底）原样保留。其上跟随主题的文字（未设颜色的连接线标签）在深色主题下对比度低。draw.io 的“自适应颜色”没有照搬；单页导入到当前画布时也不采用文件的页面背景色。
