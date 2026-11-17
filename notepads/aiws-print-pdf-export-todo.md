# AIWS 打印（导出 PDF）TODO

状态：2026-11-16 已按本文实施（T1–T12），未提交；本机 Zone 上真实 DV 通过（§9）。实施中新出现的待确认问题见 §6.2（Q23–Q38）。立项时的说明：owner 四轮答复（2026-11-14）已定页面规格、PDF Frame 的匹配、按路径打印的定位、字体缺失的处理（§6.1）。2026-11-16 owner 定了渲染路线：从交付功能的角度，打印走无头浏览器方案，用 Workflow 的 `media.page`（§4）。自建渲染管线以后仍有价值，继续在[渲染管线方案讨论](<../doc/workspace/BuckyOS AI Workspace 渲染管线方案讨论.md>)里讨论。同日 owner 答复了当时待确认的全部问题（§6.1），字体包带出的 Q21、Q22 和字体的两处实现缺口，owner 同日也定了：按建议做，缺口都修（§3.5、T12）。

2026-11-16 更新：当时“另行讨论”的无头浏览器已经实现，就是 Workflow 的系统定义 `media.page`（在 [Agent 浏览器实施 TODO](agent-runtime-browser-todo.md) §5.7–§5.9 里定案）。本文按已提交的实现和 owner 的路线裁决改了结论（§0）、现状（§2）、渲染路线（§4）、要做的（§5）和确认项（§6），新增 Q18–Q20。owner 答复 §6.2 以后，答案并进了设计（§3）、任务（§5）和验收（§7）。

日期：2026-11-14，2026-11-16 更新。源码基线：buckyos `7682af2da`（工作区另有未提交的改动）；11-16 的更新对照 `565220b7d`。

来源：

- 内核文档 [10 演示与打印](../doc/workspace/kernel/10-演示与打印.md)的“PDF 导出与打印”一节。
- owner 2026-11-14 的四条思路（§1）和同日的四轮答复（§6.1）。

## 0. 结论

- 打印就是把画布导出成一份 PDF 文件，存入 Zone 或下载到本机，不放回工作区（Q10）。系统里没有任何打印机相关的画面。
- 入口文案是“导出可打印 PDF”（Q8）。对用户的说法是导出当前画布。流模式画布直接打印，内容自动分页。自由画布按它的演示路径打印，一步一页，没有演示路径时提示无法打印。引导路径不算（Q6）。
- 演示路径可以跨画布，所以沿路径可以打印出跨画布的内容。这一点不专门宣传，留给愿意研究的用户发现。
- 打印时选页面规格（A4、A5 等），还可以选一个 PDF Frame 控件。PDF Frame 带页眉、页脚和页码，按页面比例适用，一个工作区可以有多个。不选也能打印。
- 工作区设置把 `sans`、`serif`、`mono` 对应到具体字体（Q17）。字体以 ext-tool 字体包的形式按语言提供，节点从上游官方地址下载，流程同 Chromium（Q16）。工作区用到的字体没有安装时，打印前提示用户可能失真，并引导去安装字体包。
- 内容经无头浏览器变成 PDF（owner 2026-11-16，§4）：AIWS 服务把要打印的页面物化成一个自包含的打印包（Q20），交给 Workflow 的 `media.page`，在 Zone 里用 Chromium 分页（Q18）、排页眉页脚和页码（Q19）、输出 PDF。不改 Block 的渲染契约，不引入写 PDF 的库。只会输出 HTML 的 Block 也走这条路。
- 自建渲染管线以后仍有价值，在渲染管线方案讨论里继续。以后改走它时，打印换一个后端，页面模型、PDF Frame、对话框这些不变（§4.2）。

## 1. owner 的四条思路

| # | 原话 | 落到设计上 |
| --- | --- | --- |
| P1 | 打印的本质是导出 pdf，我们的系统里不会出现任何打印机管理画面 | 产出是 PDF 文件，默认存入 Zone。不做打印机选择、打印队列、份数、双面这类设置。用户要纸质件时，用自己设备上的 PDF 阅读器打印。我的推论：也不调用浏览器的 `window.print()`，它会弹出带打印机列表的系统对话框 |
| P2 | 打印是针对画布的 | 入口在“画布”菜单，从当前画布发起。没有“打印整个工作区”，数据模式里没有打印入口 |
| P3 | 画布是流模式直接可以打印，普通画布需要有至少一个演示路径（不满足条件提示无法打印） | 流模式按内容顺序自动分页。自由画布按演示路径出页，一步一页。条件不满足时入口仍然可见，点击后说明原因，并给出去建演示路径的入口 |
| P4 | 打印前可以选择一个可选特定的 frame 控件（工作区可以有多个，和页面比例有关），该控件可以配置页眉页脚，支持页码等 | 新增 PDF Frame 控件（§3.3），对应文档 10 里的“背景控件”。打印对话框里选一个，或者不选 |

这些与文档 10 有两处不同，实施时回写文档（T10）：

- 文档 10 的“手工排版”要用户另外指定页面区域和页序。按 P3，自由画布的页面区域和页序直接取演示路径，不另建一套页面定义。
- 文档 10 提到“打印整个工作区”。按 P2 没有这个范围。跨画布的内容沿演示路径打印。

## 2. 现状

Desktop 的路径都在 [src/frame/desktop/src/app/aiworkspace/](../src/frame/desktop/src/app/aiworkspace/) 下，服务的路径都在 [src/frame/aiworkspace/](../src/frame/aiworkspace/) 下。渲染管线的现状另见[渲染管线方案讨论](<../doc/workspace/BuckyOS AI Workspace 渲染管线方案讨论.md>) §2。

| 环节 | 现在的情况 | 位置 |
| --- | --- | --- |
| 打印与 PDF 输出 | 没有。没有打印宿主、页面定义和分页；`BlockDefinition` 和 component.json 不能声明打印能力。AIWS 的服务和 Desktop 里都还没有调用 Workflow 的地方 | [kernel/README.md](../doc/workspace/kernel/README.md)“演示与打印” |
| Block 的呈现 | 各个表现都是 React 组件，输出都是 DOM | `ui/blocks/registry.ts` |
| HTML 扩展 Block | 运行在页面内的同源 iframe 里。不运行时显示实例上保存的一张快照图片（`config.snapshot`），没有就显示占位 | `ui/extensions/HtmlBlockHost.tsx` |
| 画布布局 | Surface 的 `layout.mode` 是 `free` 或 `flow` | `api/types.ts` |
| 演示路径 | 实体 `buckyos.show-path`，放在工作区的 `shows` 下，属于工作区，不属于某个画布。`purpose` 是 `presentation` 或 `guide`。`stage` 是舞台尺寸，预设 16:9 和 4:3。每一步指向一个 Frame 或一个 Viewport。一条路径的步骤可以落在不同画布上。落在流模式画布上的步骤判为问题 `flow`，不能播放 | `presentation/model.ts` 的 `resolveSteps`、`playable`、`pathsOf` |
| 呈现模式 | `CanvasMode` 只有 `edit`、`view`、`presentation_edit`、`show`，没有打印 | `ui/blocks/registry.ts` 的 `modePolicy` |
| Frame | 普通 Block（`view.type = frame`），是带标题的区域。`presentation.*` 只对 Frame 有效。没有打印用途的声明 | `ui/blocks/frameBlock.tsx`、`core/src/types.rs` |
| 组合控件 | `composition.json` 声明布局（`stack`、`grid`、`fixed`）和 slot，slot 可以放在 `top` 或 `bottom`，还可以从数据投影出 label 等节点 | `controls/composite.ts` |
| 画布配置 | Surface 的 `config` 只有 `background` 和 `page_size`。`page_size` 是 draw.io 导入带来的，不参与分页 | [kernel/README.md](../doc/workspace/kernel/README.md)“演示与打印” |
| 导出入口 | “画布”菜单组里已有 `surface.export-drawio`。工作区导出包默认存入 Zone（系统文件选择器的保存模式），其次下载到本机 | `ui/framework/builtin.ts`、`ui/home/exportPackage.ts` |
| 固定基线 | 放映用 `clone_for_show` 复制出一份一致的只读状态 | `store/src/show.rs` |
| 权限 | `export` 是按实体判定的能力，导出模板时只检查所选的画布。物化时可以去掉讲稿备注（`without_notes`） | `core/src/access.rs`、`core/src/materialize.rs` |
| 读写 PDF 的库 | Desktop 只有 `pdfjs-dist`，用来读和显示 PDF。没有写 PDF 的库。Zone 一侧可以经 `media.page` 用 Chromium 写 PDF（下面“系统渲染”一行） | `desktop/package.json` |
| 字体 | Desktop 的中文靠设备上的系统字体，前端不带字体文件。内容里能选的只有三个通用字体族：图形的文字可以选 `sans`、`serif`、`mono`。Zone 一侧，扩展包 ext-core 带了 Noto Sans CJK、Noto Serif CJK（常规体和粗体）、DejaVu、Liberation。1.3.1 起包里用 fontconfig 的工具（`soffice` 和 Chromium）共用一份配置，都读得到这些字体和简体中文优先的回退规则，也读节点的系统字体目录 `/usr/share/fonts`。节点字体集的摘要进 `media.page` 的缓存指纹，摘要取自 Workflow 自己的 `fc-list`，看不到 ext-core 带的字体。系统里没有让用户安装和管理字体的地方 | `desktop/src/index.css`、`import-drawio/shapes.ts`、`publish/exttool/ext-core/build.sh`、[media tool baseline](<../doc/workflow/media tool baseline.md>) |
| Zone 里的工具 | ext-core 1.3.2 有 `soffice`、`pdftoppm`、`magick`、`node`、`deno`，以及 `page-render`、`agent-browser`、`playwright-core`。Chromium（159）是 ext-core 的在线项：不在包里，节点装好包以后从官方地址下载，状态在 `tools/online/chromium.json` | [ext-manifest.json](../publish/exttool/ext-core/ext-manifest.json)、`doc/app_service/paios容器需求.md` §6.4 |
| 系统渲染 | Workflow 的系统定义 `media.page`：把一份 HTML，或根上有 `index.html` 的 zip，输出成 PDF 或 PNG。在 Zone 里用一次性的无头 Chromium 渲染，没有网络。本机 Zone 上 `WF-DV-017` 通过。细节见 §4.1 | [page.rs](../src/kernel/workflow/src/functions/media/page.rs)、[media.page.json](../src/kernel/workflow/defs/media.page.json)、[page-render.mjs](../publish/exttool/ext-core/browser/page-render.mjs) |

## 3. 设计

### 3.1 流程

1. **入口。** 只在“画布”菜单里，手机主菜单里同样有，文案是“导出可打印 PDF”（Q8）。对用户的说法是打印当前画布。需要 `export` 能力，没有时置灰并写明原因。
2. **条件。**
   - 流模式画布：直接进入对话框。
   - 自由画布：需要一条经过本画布的演示路径（`purpose: presentation`），引导路径不算（Q6）。一条都没有时不进入对话框，提示“这个画布没有演示路径，无法打印”，并给出进入演示路径编辑的入口。
3. **对话框。** 手机上是整屏。
   - 演示路径：只在自由画布上出现，多于一条时选择。
   - 页面规格：A4、A5、A3、Letter、16:9、4:3，以及纵向或横向。
   - PDF Frame：可选，默认“不使用”。列出工作区里调用者读得到的 PDF Frame，比例与所选规格一致的才可选。
   - 字体提示：有字体缺失时显示（§3.5）。
   - 页数和问题清单：会被跳过的步骤、读不到的内容、用占位代替的内容。路径上有其他画布的页时，在页数里注明，不另作说明。
4. **保存位置。** 与工作区导出包一致，先选位置再生成。默认存入 Zone，用系统文件选择器的保存模式选文件夹和文件名，默认文件名是画布名。其次是“下载到本机”。
5. **生成。** 显示进度，可以取消。生成期间继续编辑不影响这次输出。
6. **结果。** 成功后提示保存到了哪里。失败时说明原因和受影响的页，不留下半份文件。

### 3.2 页面从哪里来

一页由三部分组成：页面尺寸、内容区、页面装饰（页眉、页脚、页码）。页面尺寸由对话框里选的页面规格决定。

| | 流模式画布 | 自由画布 |
| --- | --- | --- |
| 内容 | 整个画布，按阅读顺序 | 所选路径的全部可播放步骤，按路径顺序。路径上落在其他画布的步骤照常输出 |
| 分页 | 按内容区的宽度排版，按内容区的高度自动分页 | 一步一页。步骤的矩形等比缩放后放进内容区并居中，不裁切，不拆页 |
| 默认页面规格 | A4 纵向（Q5） | 路径的舞台比例 |
| 没选 PDF Frame | 内容区是页面四边各减 20 mm（Q5） | 内容区是整页 |
| 选了 PDF Frame | 内容区、页眉页脚取自 PDF Frame | 同左。步骤的比例与内容区不同时留白 |
| 背景 | 画布背景 | 路径背景；Frame 步骤用它自己的背景 |

自由画布沿用放映的规则：

- 步骤里声明隐藏的 Block（`step.hide`）在该页不出现。
- 停用的步骤、目标已删除的步骤、落在流模式画布上的步骤跳过，并列入问题清单。
- 旋转过的 Frame 按不旋转输出。
- 讲稿备注不输出。
- 路径上有调用者读不到的画布时，那些步骤跳过并列入问题清单。

流模式的分页规则（文档 10“流式自动分页”）。分页在打印包里做，用 Chromium 量出的高度（Q18）：

- 分页点只能落在合法断点上：Block 之间，富文本的段落和行之间，表格的行之间。
- Block 可以声明“不可拆分”和“与下一块同页”，后者用来避免标题留在页尾。
- 表格打印全部行，跨页时重复表头。
- 单个不可拆分的内容高于一页时，等比缩小到一页，并列入问题清单。不能靠裁掉内容来凑页。
- 页眉、页脚和边距先从页面高度里扣除，再分页。

两种画布都适用：

- 打印是一个独立的呈现模式，不运行宏和许愿格，不同步数据源，不播放媒体，不产生任何写入。
- 视频输出封面图。音频输出带名称的占位，并列入问题清单。
- 选区和个人折叠状态不输出。手写笔迹是内容，照常输出。评论和批注不输出（Q7）。
- 主题按页面背景的深浅取浅色或深色（沿用 `stageTheme`），不随 Desktop 当前的主题。
- PDF Frame 实例本身不作为内容输出。它放在流模式画布里时，分页跳过它。

### 3.3 PDF Frame 控件

对应文档 10 的“背景控件”。界面上的中文名待定。

- **形态。** 随标准库发行的组合控件，与待办、日历一样是内置的。实例放在画布上，直接在画布上编辑（Q2、Q11）。
- **页面比例。** 每个 PDF Frame 有一个页面比例：A 系列（A3、A4、A5 共用）、Letter、16:9、4:3，或自定义，再分纵向和横向。控件在画布上的外形始终保持这个比例。
- **多个。** 一个工作区可以有多个 PDF Frame。不同比例各有各的，同一比例也可以有几个不同设计的，打印时选其中一个。
- **按比例套用。** 打印时把 PDF Frame 等比缩放到所选的页面规格上，比例一致才可选。A4 和 A5 比例相同，用同一个 PDF Frame，A5 上页眉页脚的字相应变小。
- **页眉和页脚。** 各是一个 slot，可以放文本（label）、图片和形状。高度可调，可以为 0。
- **内容区。** 页眉和页脚之间、左右边距以内的区域。编辑时显示为占位，打印时放入当页的内容。
- **页码等变量。** label 增加一种取值来源“页面上下文”，有页码 `page`、总页数 `pages`、标题 `title`、日期 `date`。值由打印包里的分页脚本在排好页以后给出（Q18、Q19）。在画布上编辑时显示示例值。
- **不复制数据。** 每页的页眉页脚都是同一个实例的投影。
- **画在页面里。** 页眉、页脚和页码排进每一页，`margin_mm` 给 0，不用 Chromium 的页眉页脚模板（Q19）。

### 3.4 固定基线、权限与失败

- **基线。** 开始时固定一份一致的只读状态，生成期间的编辑不进入这次输出。由 AIWS 服务在生成打印包时做，参照放映克隆（`clone_for_show`），打印包生成以后就释放（Q20）。
- **权限。** 需要 `export`：流模式查当前画布，自由画布查路径经过的每个画布。只输出调用者读得到的内容。读不到的 Block 留空并计入问题清单，不暴露它的名称。
- **资源。** 图片等在排版前备齐，缺失的列入问题清单。
- **失败。** 成功表示全部页面都已生成。失败或取消时不写出文件。Zone 里已有同名文件时由选择器询问，不直接覆盖。

### 3.5 字体

owner 已定：打印的时候，如果工作区使用的字体有缺失，提示用户打印可能失真，并提示用户去安装字体。2026-11-16 又定了字体从哪里来（Q16）和怎么确定用到哪些字体（Q17）。

- **字体包（Q16）。** 以 ext-tool 扩展的形式，按语言准备字体包。要注意版权：下载流程和 Chrome for Testing 一致，包里不带字体文件，只声明在线项，节点从上游的官方地址下载，我们不分发。只收许可证允许嵌入 PDF 的字体（我的补充：打印要把字体嵌进 PDF）。一个 ext-tool 包，每种语言一个在线项，不自动下载，由管理员按需下载（Q21）；首批收哪些语言见 Q22。
- **默认字体。** ext-core 已经带的 Noto Sans CJK、Noto Serif CJK、DejaVu、Liberation 继续作为默认字体，新装的系统打印时不报缺失。Noto CJK 含简繁日韩的字形；没标语言的汉字按简体中文的字形出，日文工作区要在下一条的设置里对应到 `Noto Sans CJK JP` 等。
- **工作区用到的字体（Q17）。** 工作区设置里把 `sans`、`serif`、`mono` 对应到具体字体，可选的是默认字体和字体包里的字体。没有设置时用系统默认字体。检查的就是这三个对应。
- **检查。** 进入打印对话框时，看这三个对应的字体在节点上是否就绪：默认字体，或所在字体包的在线项已下载好。
- **提示。** 有缺失时在对话框里列出缺失的字体名，带“去安装”的入口，指到对应的字体包。只有管理员能下载；不是管理员时说明要请管理员安装（我的推论）。
- **不阻止打印。** 用户可以继续，缺失的字体用替代字体输出。这是我的理解：原话是“可能失真”，不是“无法打印”。
- **屏幕不改。** 屏幕上的字体不因此调整。

字体在 Zone 一侧有两处缺口，owner 2026-11-16 定了都修（T12）：

- **Chromium 要看得到字体包。** ext-core 的 fontconfig 配置现在只读包里的字体目录和 `/usr/share/fonts`。字体包是另一个包，它的在线项落在 `tools/online/<在线项>/`，ext-core 不知道有哪些。
- **缓存要随字体变。** `media.page` 的缓存指纹里，字体摘要是 Workflow 启动时用搜索路径上的 `fc-list` 算的：ext-core 不带 `fc-list`，算到的是宿主的系统字体，看不到 ext-core 的字体，也看不到字体包；而且只在启动时算一次（Workflow 只在启动时探测工具，见 Agent 浏览器 TODO 的 Q43）。不改的话，下载了字体包以后，同一份打印包会命中之前用替代字体出的 PDF。这个缺口现在就存在：ext-core 换了字体而 `page-render` 的版本没变时，旧缓存同样会被命中。

## 4. 渲染路线

owner 的判断（第三、四轮）：

- 核心问题是 Block 输出 HTML 再经浏览器变成 PDF，还是 Block 直接画 PDF。前者所见即所得，后者打印质量高。
- 只会输出 HTML 的 Block 也必须能打印，这是单个 Block 的兼容路径，为的是开发者的学习成本。
- 往直接画的方向走，带来的是渲染管线的重大变化：尽量不让 Block 直接依赖下面是 HTML，而是用我们自己的、类似 canvas / DOM 的结构来组织渲染。不写 render 的 Block-control 反而改动较小。
- 这是内核级别的改动，专门写文档深入讨论。

讨论放在[渲染管线方案讨论](<../doc/workspace/BuckyOS AI Workspace 渲染管线方案讨论.md>)。它比较了三种走法（不改管线靠浏览器引擎、给 Block 加第二份渲染、改成宿主定义的节点树），讨论了节点树的层次、词汇、文字和字体、富文本和表格、HTML 的位置、迁移路径和风险，列了 11 个待裁决的问题。

owner 的裁决（2026-11-16）：

- 从交付功能的角度，选择无头浏览器方案。对应讨论文档里的 X（不改管线）：Block 照旧输出 HTML，打印交给 Zone 里的 Chromium，也就是 §4.1 的 `media.page`。
- 自建渲染管线以后仍有价值，讨论文档继续保留，作为长期方向。

### 4.1 已有的系统渲染：`media.page`

owner 当时说无头浏览器另行讨论，别处也有这个需求。它后来并进了 Agent 浏览器的需求，在那里定案（D11、D21、D29、D31–D34）并实现：Chromium 作为 ext-core 的在线项装到节点上，ext-core 的命令 `page-render` 用它出 PDF 或 PNG，Workflow Service 把它包成内置函数 `media.page_to_pdf`、`media.page_snapshot` 和系统定义 `media.page`。参数和行为以 [workflow service](<../doc/workflow/workflow service.md>) 和 [media tool baseline](<../doc/workflow/media tool baseline.md>) §3 为准，下表只列和打印有关的部分。

| 方面 | 已实现的情况 | 对打印意味着什么 |
| --- | --- | --- |
| 输入 | 只接受对象：`workflow.create_run` 的触发参数 `obj_id`，指向一份 HTML 文件，或根上有 `index.html` 的 zip。读权限按调用者自己的令牌查。网址不是它的输入 | 要打印的内容得先物化成一个自包含的“打印包”，存成调用者读得到的对象。它不能直接打开工作区的实时页面。打印包由 AIWS 服务生成（Q20），存成什么对象是 T7 的事 |
| 网络 | 渲染时没有网络：只加载包里的文件和 `data:`、`blob:`。其余请求一律拦下，记进运行记录的 `warnings` | 图片、样式表都要放进包里，和 §3.4 的“资源在排版前备齐”一致。字体不放进包里，由节点上的默认字体和字体包提供（§3.5）。HTML 扩展 Block 在包里拿不到 AIWS 的接口和数据：T8 要把它要的数据一起放进包，做不到时用它的快照图片（`config.snapshot`） |
| 脚本 | 页面里的脚本照常运行。`wait_ready` 是一个表达式，为真以后才输出；之后还会等 `document.fonts.ready`，可以再加 `wait_ms` | 分页放在 Chromium 里做：包里带分页脚本，量高度和输出用同一个引擎、同一套字体，排好以后置一个标记（Q18） |
| 纸张 | `paper` 是 A3、A4、A5、Letter，或 `宽x高mm`；`landscape`；`background` 打印背景 | §3.1 的规格都给得出，16:9 和 4:3 用自定义尺寸 |
| 边距和页眉页脚 | `margin_mm` 一个值，四边相同。`header_html`、`footer_html` 是 Chromium 的打印模板（每份不超过 64 KiB），画在边距里，带 `pageNumber`、`totalPages`、`date`、`title` 类名的元素会被填上值 | 模板和页面是分开的，只适合简单的文字。PDF Frame 的 slot 要能放任意 Block（Q11），所以边距给 0，页眉、页脚和页码由我们排进每一页（Q19） |
| 输出 | 结果类型 `media.derived_pdf`：`pdf_content_id`、`pdf_obj_id`、`pages`。Run 到终态后，服务替调用者登记 NDM 持有，调用者之后能读这份 PDF。Chromium 输出的 PDF 嵌入字体、文字可选，`pdftotext` 能抽出中文 | 看起来可以直接交给 `useSaveToZone`：它按对象 ID 在 Zone 里提交，字节不经过浏览器，手机上也一样。这一步还没验证过。§7 的“文字可选可搜”这条路线天然满足 |
| 缓存 | 纯函数。缓存键是输入内容、参数，加上工具指纹（`page-render` 和 Chromium 的版本、节点字体集的摘要） | 同一份打印包、同样的参数再打印一次直接命中缓存。打印包本身是固定的，“生成期间的编辑不进入输出”（§3.4）在 PDF 这一步自然成立，固定基线仍要在生成打印包时做。字体摘要看不到字体包，要补（§3.5） |
| 隔离 | 渲染进程清空环境（拿不到服务令牌）、单独进程组、断网；Linux 上 Workflow 以 root 运行时降到 `nobody` | 扩展 Block 的第三方 HTML 可以放进打印包 |
| 限制 | Browser 车道一个节点同时只跑一个。步骤 300 秒超时，整个 Run 10 分钟。zip 解包后不超过 512 MiB、不超过 1 万个文件 | 多人同时打印会排队。进度只有 Run 的状态，没有逐页进度；取消用 `workflow.cancel_run` |
| 可用性 | 节点还没下载好 Chromium 时，`media.page` 以 `tools_missing` 失败（`details.online_item` 是 `chromium`），不重试，不进缓存。管理员可以在“设置 › 扩展工具”里下载 | 对话框要处理这种情况：说明原因；调用者是管理员时给出下载入口，做法同「Agent 浏览器」App |
| 平台 | 在 Linux x86_64 上验证过。macOS、Windows 的分支没有运行过 | Desktop Lite 在 macOS、Windows 上的打印要另外验证 |

### 4.2 这一期怎么做，和以后的自建管线

这一期：

- Block 的渲染契约不改，`BlockDefinition` 和 component.json 不新增打印用的渲染入口。
- AIWS 服务把要打印的页面物化成打印包（Q20）：快照、资源和打印用的渲染入口。渲染入口用 Desktop 现有的 Block 组件构建，只带打印模式；它在 Chromium 里渲染各 Block，分页（Q18），排页眉页脚和页码（Q19）。交给 `media.page` 输出 PDF。
- 打印质量就是 Chromium 打印的质量。只会输出 HTML 的扩展 Block 和内置 Block 走同一条路，没有单独的兼容路径，限制见 §4.1 的“网络”一行。

以后改走自建管线时：

| 讨论文档里的走法 | 打印怎么变 |
| --- | --- |
| 改成节点树 | 打印成为节点树的一个后端。它可以输出 HTML 或 SVG 再交给 `media.page` 写 PDF，也可以自己写 PDF。只会输出 HTML 的 Block 继续用这一期的做法 |
| 给 Block 加第二份渲染 | 图形类、脑图、表格换成各自的打印渲染，其余 Block 继续用这一期的做法 |

两种情况下，页面模型、取页、分页规则、PDF Frame、对话框、基线和权限都不变。打印包的格式和分页脚本是这一期新做的，换后端时要重看。

## 5. 要做的

渲染路线已定（§4），§6.2 原来的问题 owner 都已答复。T1–T6 和打印包的格式无关，可以和 T7 并行。T12 的缓存部分不依赖别的任务，可以最先做。

页面与界面：

- [x] **T1 页面模型与取页。**
  - 页面规格、内容区、页面装饰的结构。问题清单的结构。
  - 自由画布按演示路径取页：复用 `resolveSteps`、`playable`、`stageView`，取整条路径的可播放步骤。
  - 自由画布没有可用路径时的判定和提示。
- [x] **T2 分页规则。**
  - 按 §3.2 的规则选分页点，做成纯函数，输入是各块的高度和断点，便于单测。它在打印包里运行（Q18）。
  - 重复表头、超高内容缩小到一页的规则。
- [x] **T3 PDF Frame 控件。**
  - 标准库里的组合控件：页面比例、方向、页眉 slot、页脚 slot、内容区。
  - label 的“页面上下文”取值来源，编辑时的示例值。
  - 等比套用到页面规格上，比例一致才可选。
  - 控件目录、图标和 i18n。
- [x] **T4 入口、对话框与保存。**
  - `surface` 菜单组里的入口和手机主菜单里的入口。文案“导出可打印 PDF”，英文建议 “Export printable PDF”。
  - 对话框：演示路径、页面规格、PDF Frame、字体提示、页数和问题清单。手机上整屏。
  - 保存：复用 `useSaveToZone` 存入 Zone，其次下载到本机。不放回工作区（Q10）。
  - 进度和取消。
- [x] **T5 基线、权限与失败行为。**
  - 固定基线（§3.4）。
  - `export` 检查，读不到的内容留空。
  - 讲稿备注、选区、个人状态不输出。
  - 失败和取消不留文件。
- [x] **T6 工作区字体与检查提示（§3.5）。**
  - 工作区设置：`sans`、`serif`、`mono` 各对应一个具体字体，可选默认字体和字体包里的字体（Q17）。打印包按这个对应生成 CSS。
  - 进入对话框时检查三个对应的字体是否就绪；缺失时提示，带去字体包的入口。
  - ext-core 的 fontconfig 加上字体包的目录。
  - `media.page` 的缓存指纹包含已就绪的字体包。

输出（无头浏览器路线，§4）：

- [x] **T7 打印包与 PDF 输出。**
  - 打印呈现模式：各类 Block 在打印包里的表现，不运行宏和许愿格、不产生写入（§3.2）。
  - 打印用的渲染入口：用 Desktop 现有的 Block 组件构建，只带打印模式，随打印包一起放（Q20）。
  - 打印包的格式：快照、每页的尺寸、页面装饰、资源怎么放进包里，分页脚本和“排好了”的标记（Q18、Q19）。
  - AIWS 服务生成打印包：固定基线、按调用者的权限过滤、去掉讲稿备注（T5）。存成什么对象、以谁的身份调 `media.page`，让调用者读得到生成的 PDF。
  - 调 `workflow.create_run`、`start_run`，等结果，取消。
  - 节点还没下载 Chromium 时的提示和下载入口。
  - 把 `pdf_obj_id` 交给 `useSaveToZone` 存进 Zone；下载到本机时从 NDM 读。
- [x] **T8 HTML 扩展 Block。** 扩展 Block 的 HTML 和它要的数据一起放进打印包。打印包里没有网络、没有 AIWS 接口，做不到时用快照图片，并列入问题清单。
- [x] **T9 表格的打印内容。** 全部行、列宽、按行分页、重复表头。表格在屏幕上是虚拟滚动，打印包里要放全部行。

字体包：

- [x] **T11 按语言的字体包（Q16、Q21、Q22）。**
  - 一个 ext-tool 包（名字实施时定，暂称 `ext-fonts`），每种语言一个在线项，`auto: false`：装上包不自动下载，管理员在缺字体提示或“设置 › 扩展工具”里下载。下载、失败、重试走在线项的现有流程。
  - 首批：阿拉伯文、希伯来文、泰文、天城文，各一套无衬线和衬线，用 SIL OFL 1.1 的 Noto 系列；拉丁文另加几套常用的 OFL 字体。下载地址只用 notofonts 等上游在 GitHub 上的官方发布。
  - ext-tool 包和在线项：每个字体的官方下载地址、`sha256`、许可证文本，按 `doc/app_service/paios容器需求.md` §6.4 的在线项规则写清单。
  - 只收许可证允许嵌入 PDF 的字体；合规清单里写明我们不分发。
  - “设置 › 扩展工具”里能看到、下载、重试；打印对话框的缺字体提示能跳过去。
- [x] **T12 字体的两处缺口（§3.5）。**
  - 缓存：`media.page`（以及同样带字体摘要的 `media.to_pdf`）的指纹改为包含 ext-core 的包版本和已就绪的字体在线项（id、版本），每次调用时现读，不依赖启动时的探测，下载字体包以后不用重启 Workflow。宿主系统字体的 `fc-list` 摘要照旧保留。这一项现在就能做，不等 T11。
  - 让 Chromium 看得到字体包：建议字体包在自己的包目录里带一份 fontconfig 片段，列出它各个在线项的目录（用相对路径）；ext-core 的 fontconfig 配置用 `<include ignore_missing="yes">` 引入这份片段。没装字体包时什么都不发生。和 T11 一起做。
  - 验证：下载一个字体包以后，不重启服务，`page-render` 输出的 PDF 用上了新字体，第二次打印不命中旧缓存；`soffice` 的冒烟照常通过。

收尾：

- [x] **T10 文档、手册与测试。**
  - 内核文档 10：手工排版改写为“自由画布的页面取自演示路径”，“背景控件”对应 PDF Frame，去掉“打印整个工作区”，写明页面规格在打印时选择、字体缺失的提示。
  - 内核文档 10 里“各 Block/control 声明打印能力”改写为：这一期 Block 不声明打印能力，打印经打印包和 `media.page` 输出。
  - 内核文档 05（Frame 的打印页用途）、06（呈现模式）、12（验收行），以及 kernel/README 的缺口清单。
  - UI 框架 PRD 的菜单表。
  - 新增 RPC、CLI 或 component.json 字段时，更新服务 README 的方法表、控件开发文档和手册 `aiworkspace_run`、`aiworkspace_template_dev`。
  - 内核文档 10 的“生成文件作为普通资产保存”改为只存入 Zone 或下载到本机（Q10）。
  - 测试：分页纯函数的单测；Desktop e2e 覆盖流模式、自由画布、无路径提示、PDF Frame 和字体提示；真实 Zone 和手机上的 DV，包括节点还没下载 Chromium 的情况。

## 6. 确认与待确认

### 6.1 已确认

owner 2026-11-14 的四轮答复，Q1 的后半是 2026-11-16 补的：

| # | 问题 | 结论 |
| --- | --- | --- |
| Q3 | “和页面比例有关”怎么理解 | 打印的时候选页面规格（比如 A4、A5），并且可以选不同的 PDF Frame |
| Q12 | PDF Frame 与页面规格怎么匹配 | 按比例设计，打印时等比缩放到所选规格上，比例一致才可选。owner 对 Q3 的整体答复是“OK”，这条规则当时列在 Q3 下 |
| Q4 | 演示路径跨多个画布时打印哪些步骤 | 整条路径。对用户只说打印是单画布的，培训简单。愿意研究的用户会发现用跨画布的路径可以打印跨画布的内容，这对用户是高级功能 |
| Q13 | 按路径打印的入口 | 只在画布菜单里，不在演示路径编辑器里另放入口。这是我从 Q4 的结论推出的 |
| Q1 | 渲染路线 | 是渲染管线的重大变化，属于内核级别的改动，专门写文档讨论（§4）。无头浏览器也另行讨论，别处也有这个需求。无头浏览器后来在 Agent 浏览器 TODO 里定案并已实现：Workflow Service 的内置函数，降权到 `nobody`，不用容器；Chromium 是 ext-core 的在线项（§4.1）。2026-11-16：从交付功能的角度，打印选无头浏览器方案；自建渲染管线以后仍有价值，继续讨论（§4） |
| Q15 | 屏幕和打印的字体不一致怎么办 | 打印时提示用户：有工作区使用的字体丢失了，打印可能失真；提示用户去安装字体（§3.5） |

owner 2026-11-16 对当时待确认问题的答复。写“按建议”的，是同意了当时列的建议：

| # | 问题 | 结论 |
| --- | --- | --- |
| Q16 | 字体装在哪里，系统默认带哪些 | owner：“最好能以 ext-tool 扩展的形式，准备一下各个语言的字体包（这个要注意版权问题，和 chrome for test 的按需下载流程一致）”。我原来建议的 Zone 字体库、字体随打印包走都不采用。默认字体仍是 ext-core 已带的。组织方式和首批语言见 Q21、Q22（§3.5） |
| Q17 | “工作区使用的字体”怎么确定 | 按建议：工作区设置里把 `sans`、`serif`、`mono` 对应到具体字体，没有设置时用系统默认字体，检查的就是这几个对应 |
| Q2 | PDF Frame 放在哪里 | 按建议：普通控件实例，放在任意画布上；打印对话框列出整个工作区里的 PDF Frame |
| Q5 | 流模式的默认页面规格和没选 PDF Frame 时的边距 | 按建议：A4 纵向，四边 20 mm |
| Q6 | 引导路径（`purpose: guide`）能不能用来打印 | 按建议：不能，只认 `presentation` |
| Q7 | 评论和批注是否输出 | 按建议：不输出 |
| Q8 | 入口文案 | owner：“导出 可打印PDF”。中文界面写成“导出可打印 PDF”，中英文之间的空格按现有界面文案的习惯 |
| Q9 | Agent 和 CLI 是否在本期提供 | 按建议：本期不提供。打印包由服务端生成（Q20），以后 Agent 和 CLI 可以走同一条链路 |
| Q10 | 生成的 PDF 是否同时登记为工作区资产 | 按建议：只存入 Zone 或下载到本机，不自动放回工作区，用户需要时自己把文件插入画布。文档 10 的说法要改（T10） |
| Q11 | PDF Frame 的实现形态 | 按建议：组合控件，页眉和页脚是 slot，可以放任意 Block |
| Q18 | 流模式的分页在哪里做 | 按建议：在 Chromium 里做。打印包带 T2 的分页纯函数，在页面里量出各块高度、排好页，置“排好了”的标记，再输出 |
| Q19 | 页眉、页脚和页码由谁画 | 按建议：由我们排进每一页，`margin_mm` 给 0，不用 Chromium 的页眉页脚模板 |
| Q20 | 打印包怎么生成 | 按建议：AIWS 服务生成。固定基线、按调用者权限过滤、去掉讲稿备注，把快照、资源和一个打印用的渲染入口打成 zip；渲染入口用 Desktop 现有的 Block 组件构建，只带打印模式。字体按 Q16 由节点上的字体包提供，不放进包里。以谁的身份调 `media.page`、PDF 怎么交到调用者手里，实施时定（T7） |
| Q21 | 字体包怎么组织，什么时候下载 | 按建议：一个 ext-tool 包，每种语言一个在线项，`auto: false`，管理员在缺字体提示或“设置 › 扩展工具”里按需下载 |
| Q22 | 首批准备哪些语言、哪些字体 | 按建议：阿拉伯文、希伯来文、泰文、天城文各一套无衬线和衬线，SIL OFL 1.1 的 Noto 系列；拉丁文另加几套常用的 OFL 字体；不收商业字体和不允许嵌入 PDF 的字体 |
| 字体缺口 | §3.5 列的两处实现缺口（Chromium 看不到字体包、缓存指纹看不到字体） | 两处都修（T12） |

写 PDF 用 Chromium，不引入写 PDF 的库。

### 6.2 待确认

2026-11-16 实施中出现的，都已按括号里的做法实施，owner 不同意再改：

| # | 问题 | 现在的做法 |
| --- | --- | --- |
| Q23 | 字体包的在线项按什么分 | 每个字体族一个在线项。上游（notofonts 等）每个字体族单独发布一个归档，而一个在线项只能有一个归档，做不到 Q21 的“每种语言一个”。界面按字体名列出 |
| Q24 | 阿拉伯文、希伯来文进不进字体包 | 不进。ext-core 的 LibreOffice 字体目录已经带 Noto Sans Arabic、Noto Naskh Arabic、Noto Sans Hebrew、Noto Serif Hebrew（常规体和粗体），列为默认字体；再放进包会有同名族的两份。要更多字重时再定怎么处理重名 |
| Q25 | 首批拉丁文字体 | Inter、Source Sans 3、Source Serif 4、JetBrains Mono，都是 OFL、有 GitHub 官方发布。Source Sans 3 的发布归档里没有许可证文件（许可证在上游仓库和字体的 name 表里），我们不分发，节点直接从 Adobe 的发布下载，认为可以收 |
| Q26 | 正式安装包是否播种 ext-fonts | 开发环境和 exttool 种子镜像已经播种（包里没有字体，只有在线项声明）；正式安装包是否钉版播种待定 |
| Q27 | macOS、Windows 节点 | 那里的 Chromium 不读 fontconfig，ext-core 的默认字体和字体包都看不到，对话框的字体检查会显示“就绪”。要不要特别处理 |
| Q28 | 工作区字体设置放在哪里 | 仓库里没有统一的工作区设置页：做成“工作区 ▸ 打印字体…”（同“设置封面…”，要 `manage`），打印对话框里给管理员一个链接 |
| Q29 | 字体是否就绪怎么判断、谁能看 | 所有报告了 ext-fonts 的节点都就绪才算就绪（不知道 Workflow 在哪个节点渲染）。非管理员读不到扩展工具状态，显示“无法检查”；要不要给普通成员一个只读接口 |
| Q30 | 便签算内容还是批注 | 便签（不挂在内容上的批注）照常打印，回复不打印；挂在内容上的评论和批注不输出（Q7） |
| Q31 | 未经管理者确认的 HTML 扩展代码打印时运行吗 | 运行。打印环境断网、没有令牌、写入全部拒绝；这样才满足 §7“只提供 HTML 的扩展 Block 不改动就能出现在 PDF 里” |
| Q32 | PDF Frame 的名字和默认内容 | “PDF 页框”（英文 “PDF page frame”）；插入时页眉是 `{title}`、页脚是 `{page} / {pages}` |
| Q33 | PDF Frame 的单位与默认值 | 用 Frame 自身的画布单位，自然尺寸下 1 单位 = 1 pt（A4 纵向 595 × 842）；页眉、页脚、左右边距默认 56（约 20 mm），左右边距一个值 |
| Q34 | PDF Frame 的编辑细节 | 换比例时保持缩放倍数，页眉页脚里的 Block 按左中右三分区保持靠边；用手柄缩放时松手才回到比例；流式页上只显示预览，编辑页眉页脚要去自由画布；带外的内容打印时裁掉 |
| Q35 | 页面信息替换的范围 | `{page}` 等只在 PDF Frame 里（编辑时示例值）和打印时替换，画布其他地方的 label 原样显示 |
| Q36 | 流模式打印的版式 | 第一行是画布标题；组打印成“组标题 + 缩进的内容”，不是一个整体 |
| Q37 | 自由画布上的表格 | 只打印 Block 框里放得下的行（与屏幕和放映一致）；全部行只在流模式 |
| Q38 | 页码上下文的日期格式 | 打印人语言的长日期（“2026年11月16日”），与 PDF Frame 在画布上的示例值一致 |

## 7. 验收

- 流模式画布：含长富文本、跨页表格和图片的画布导出为多页 PDF，没有内容被裁掉，表格打印出全部行，跨页有表头。
- 自由画布：一步一页，顺序、隐藏和背景与放映一致，路径上其他画布的步骤都在。没有演示路径时提示无法打印，不产生文件。引导路径不出现在路径选择里。
- 页面规格：同一个画布选 A4 和 A5 各导出一次，页面尺寸正确，分页随之变化。
- PDF Frame：选中后每页都有页眉页脚，页码和总页数正确。同一比例有两个 PDF Frame 时可以任选。比例不一致的不可选。不选也能打印。
- 字体：工作区用到的字体没有安装时，对话框里列出缺失的字体并给出去字体包的入口；继续打印能得到文件。字体齐全时没有提示。管理员下载字体包以后再打印，PDF 里用的是新字体，不命中之前的缓存。
- 工作区设置把 `sans` 对应到别的字体以后，PDF 里的字体随之改变。
- HTML 扩展 Block：只提供 HTML 的扩展 Block 不做任何改动就能出现在 PDF 里。
- PDF 里的文字可选可搜。
- 界面里没有打印机选择、打印队列之类的画面，全程不出现浏览器的打印对话框。
- 节点还没下载 Chromium 时，对话框说明原因，管理员能从这里去下载；下载好以后不用重启就能打印。
- 在手机上能完成发起、选页面规格和 PDF Frame、存入 Zone，并在 Zone 里看到文件。
- 生成期间别人继续编辑，输出仍是开始时的内容。
- 没有 `export` 能力的人用不了入口。读不到的内容、讲稿备注、评论和批注不出现在 PDF 里。
- 生成的 PDF 不出现在工作区里。
- 打印过程不产生 Commit，不运行宏和许愿格。

## 8. 不在本期

- 选择页码范围，或只打印选中的内容。
- 首页不同、奇偶页不同、装订边距、水印。
- PDF 书签、PDF/A、表单域。
- 一次打印多个流模式画布。
- 用许愿格生成或调整页面布局（文档 10 提到，放到后续）。
- 让系统渲染直接打开工作区的实时页面（网址输入）。`media.page` 只接受对象输入，Workflow 对不带 `obj_id` 的输入不做鉴权（§4.1）。
- Agent 和 CLI 打印（Q9）。
- 用户上传自己的字体文件。字体只来自默认字体和字体包。
- 屏幕上用字体包里的字体显示。屏幕仍用设备上的字体（§3.5）。

## 9. 实施记录（2026-11-16）

源码基线 `565220b7d`，全部未提交。做法：先写共享模块和契约，再分五个包并行（字体与 Workflow、打印包服务、打印渲染器、对话框与流程、PDF Frame），最后集成并在本机 Zone 上做真实 DV。

### 9.1 链路

1. 画布菜单“导出可打印 PDF…”（`surface.export-pdf`，手机精简菜单里同样有）→ 对话框（路径、页面规格与方向、PDF Frame、字体提示、页数和问题）。
2. 先选位置：`useSaveToZone`（系统文件选择器的保存模式，默认 Zone），在它的 `grant()` 里才生成；其次“下载到本机”。
3. kRPC `print.package`：服务在一次 `with_ws` 里取出要打印的行（固定基线），在锁外打成 zip，存进工作区 owner 的 `exports` 桶，用 `bucket.grant_holdings(caller.principal, caller.app_id, [obj_id])` 交给调用者（aiworkspace 属于 `frame` 组，有 `obj://ndm/grant`）。
4. Desktop 以用户自己的身份跑 Workflow 系统定义 `media.page`：`paper` 总是 `宽x高mm`、`margin_mm: 0`、`wait_ready: window.__AIWS_PRINT_READY__ === true`。Run 结束时 Workflow 把 PDF 的持有登记给调用者，`pdf_obj_id` 直接交给 `commit_object` 存进所选文件夹；下载到本机时从 NDM 按对象读。取消就是 `cancel_run`。
5. 打印包里的渲染器读 `data.js`，用 WASM `Replica` 从包里的行建出只读 Workspace（同会话回放），用 Desktop 现有的 Block 在新的呈现模式 `print` 下排页，排好后置就绪标记。

渲染器是 Desktop 构建里的第二份嵌套构建 `dist/aiws-print/`（`index.html`、`print.css`、一个经典脚本 `print.js`，两份 WASM 以 data: URL 内联）：page-render 以 `file://` 打开打印包，Chromium 在 file:// 下拦模块脚本和本地 fetch。服务从它找到的 Desktop 构建目录（`locate_web_dir`）取 `aiws-print/` 放进包根。

### 9.2 落点

| 部分 | 位置 |
| --- | --- |
| 共享纯函数（页面规格、内容区、PDF Frame 比例匹配、步骤适配、页面上下文、流式分页、字体目录、打印包契约） | `src/frame/aiworkspace/print/`（Desktop 别名 `@print/*`，`deno test -A`） |
| 打印包服务 | `store/src/print.rs`、`server/src/print.rs`，`methods.rs` 登记（experimental），`objects.rs` / `server/src/buckets.rs` 的 `grant_holdings`，`session/mod.rs` 提出 `query_rows` 共用；错误子码 `PRINT_PATH_REQUIRED`、`PRINT_PATH_UNUSABLE`、`PRINT_FRAME_UNUSABLE`、`PRINT_RENDERER_MISSING`；服务 README “Printing” |
| 打印渲染器 | Desktop `src/app/aiworkspace/print/renderer/`（`main.tsx`、`session.ts` 的 `PrintSession`、`FreePages.tsx`、`FlowPages.tsx`、`PrintTable.tsx`、`ready.ts` 等），`vite.config.ts` 的 `aiwsPrintRenderer()`；`print` 模式进了 `registry.ts` 的 `modePolicy`、`RenderHost`、`BlockHost`、资产与媒体、HTML 扩展 Block（`HtmlPrint`）、许愿格和课堂 Block |
| 对话框与流程 | Desktop `ui/print/`（`request.ts`、`generate.ts`、`PrintDialog.tsx`、`PrintFontsDialog.tsx`、`fonts.ts`、`extTools.ts`），`builtin.ts` 入口（另有 `ws.print-fonts`），`api/client.ts` 的 `printPackage`，`FrameworkHost` 挂 `PrintHost` |
| PDF Frame 控件 | `buckyos.container` kind `pdf-frame`（`controls/core.json`，核心校验 `config` 和两个带的 `slot`）；`controls/pdfframe.ts`；Desktop `ui/controls/pdfframe/`（`api.tsx` 是给打印用的接口：`listPdfFrames`、`PdfFrameDecoration`、`PageContextValue`、`partOfPdfFrame`）；label 的 `{page}` `{pages}` `{title}` `{date}`；服务 README “PDF Frame”；Agent 手册 `aiws/controls.json` 重新生成 |
| 字体缓存指纹（T12） | Workflow `functions/media/tools.rs` 的 `insert_font_fingerprint`：`media.page` 和 `media.to_pdf` 的指纹含宿主 `fc-list` 摘要、`env/ext-core`（调用时读 `current.json`）、`env/ext-fonts`（调用时读 `online/*.json` 里就绪的 ext-fonts 在线项） |
| Chromium 看得到字体包（T12） | ext-core 1.3.3 的 `conf.d/64-ext-fonts.conf` 相对 `<include>` 字体包的 `store/ext-fonts/fonts.conf`（实测 fontconfig 2.18.3：include 的相对名按物理路径解析，三个候选只有一个存在）；`build.sh` 的 `ext_fonts_include` |
| 字体包 ext-fonts（T11） | `publish/exttool/ext-fonts/`（清单只有在线项、`provides` 为空，`fonts.conf`、`build.py`、测试、合规说明）；8 个在线项（Noto Sans/Serif Thai、Noto Sans/Serif Devanagari、Inter、Source Sans 3、Source Serif 4、JetBrains Mono），`auto: false`；`prepare_ext_tools.py` 每次本地构建它；CI `.github/workflows/ext-fonts.yml`；没有命令的包：buckyos-api `ext_tools.rs` 校验与 `ext_tools_install.rs` 安装，websdk `cli/modules/pikg_protocol.ts` 同步（另一个仓库） |
| 文档 | 内核文档 05、06、10、12 和 kernel/README 的缺口清单，UI 框架 PRD 的菜单表（桌面和手机），`paios容器需求.md` §6.4 字体包，`media tool baseline.md` 指纹，`doc/pikg/format.md`，`AGENTS.md` 在线项一句，Desktop AIWS README “Printing” |

### 9.3 与本文的偏差

- 字体包每个字体族一个在线项，阿拉伯文和希伯来文用 ext-core 自带的默认字体（Q23、Q24）。
- 工作区字体做成“打印字体…”单独入口（Q28）；“去安装”只能跳到“设置 › 扩展工具”，定位不到字体包（设置的启动参数只有页面）。
- PDF Frame 只要求读得到，不要求 `export`：导出权限按被打印的画布判定。
- 页码上下文的日期用长日期格式（Q38），§3.3 写的是“日期 `date`”，没规定格式。
- 打印包里的资产文件按媒体类型带扩展名（`assets/<对象文件名>.svg` 等）：file:// 下 Chromium 不嗅探 SVG。
- 同一 head、同一参数生成的打印包内容字节相同（zip 时间固定、`printed_at` 取 head Commit 的时间），重复打印命中 `media.page` 的缓存；包的对象 ID 因文件名带随机 id 而不同，缓存按内容命中，不受影响。

### 9.4 验证

- Rust：`cargo test -p aiworkspace-core -p aiworkspace-store -p aiworkspace -- --test-threads=1` 407 过、0 失败；`cargo test -p workflow` 101 过；`cargo test -p buckyos-api --lib` 411 过。打印包测试 `store/tests/print.rs`（跨画布路径、去讲稿备注、guide / 无路径 / 不经过本画布被拒、无 `export` 被拒、读不到的画布和 Block 不进包、批注不进包、表格残留列、资产与扩展名、视频本体不进包、PDF Frame、zip 结构、无写入、字节确定、core 的 Replica 能装下）、`server/tests/print.rs`（持有只交给调用者、各种拒绝）。
- Deno：`print/` 18 条、`controls/` 38 条。字体包 python 测试 6 条、ext-core 31 条、`src/tools/tests` 134 条。
- Desktop：`tsc -p tsconfig.app.json`、`tsconfig.node.json` 无错误，58 个改动文件 `eslint` 干净；文案守护只有本来就失败的 “backend speaks English”（34 处都在旧的服务代码里）；`npx vite build` 产出 `dist/aiws-print/`（`print.js` 18.4 MB，打成包压缩后约 5.9 MB）。
- 渲染器离线验证：样例包用节点上的 `page-render` 出 PDF，逐页看图（P3 另有加长样例：A4 14 页、A5 30 页，段落在行间断开，跨页表格重复表头，PDF Frame 每页页眉页脚，深色主题，`sans` 映射后嵌入的字体随之改变，HTML 扩展 Block 真跑、崩溃的出占位，`pdftotext` 里跨页内容不重复）。
- 本机 Zone 真实 DV（`tests/dv/aiworkspace.print.spec.ts`，`BUCKYOS_UI_DV_BASE_URL=https://test.buckyos.io BUCKYOS_TEST_ADMIN_PASSWORD=… pnpm exec playwright test --config=playwright.dv.config.ts tests/dv/aiworkspace.print.spec.ts`，3 条全过）：
  - 流模式：90 行表格加富文本的画布，A4 存进 Zone 里新建的文件夹（`commit_object` 直接用 Workflow 登记给用户的 PDF 对象，验证了 Q20 留下的“PDF 怎么交到调用者手里”）；A5 加放在流式页上的 A 系列 PDF Frame 下载到本机：页面 148 × 210 mm、8 页、每页页眉“DV 项目工作区”和页脚“P n / 8”、表格的行全在且各只出现一次、Frame 实例本身没作为内容输出。
  - 自由画布：演示 fixture 的五步跨画布路径，默认 16:9，PDF 5 页 297 × 167 mm，选了另一画布上的 16:9 PDF Frame，每页“P n / 5”。
  - 手机（Pixel 7）：对话框占满屏幕，从精简主菜单进入，存进 Zone。
  - 都在工作区自己的源（`aiws-<id>.<zone>`）上跑：`print.package` 登记的持有、Workflow 的读权限判定、NDM 读取和 `commit_object` 用的是同一个会话，一致。
- 本机 Zone 上的 API 探针（脚本在会话 scratchpad）：`print.package` 0.7 s，Run 3 s；同一画布打两次，包内容相同，第二次 0.6 s 命中缓存、PDF 相同。字体包：工作区 `sans` 设为 Noto Sans Thai，在线项未下载时打印，PDF 里没有它；管理员 `buckyos ext-tools download ext-fonts noto-sans-thai`（6 s 就绪，不重启任何服务）后再打同一份内容，Run 没有命中旧缓存，PDF 嵌入 NotoSansThai-Regular / Bold。本机节点装的是 ext-core 1.3.1，这一条是把 1.3.3 新增的那一个 `conf.d` 文件临时放进已安装的 ext-core 测的，测完已删除。
- 本机 Zone 在 16:26 被重新部署过一次（同一工作树的构建），上面的 DV 跑在它上面；`ext-fonts` 已由 node_daemon 从种子装上，`noto-sans-thai` 已下载。

### 9.5 没做、没验证的，和风险

- ext-core 1.3.3 没有完整构建（只单独执行了新增的函数），没有发布；R2 上只有 1.3.1（1.3.2 没发过，1.3.3 会带上它的改动）。在那之前节点上的 Chromium 看不到字体包。
- 发布顺序：先把 websdk 的 pikg 校验（允许没有命令、只有在线项的包）发到 npm，再打 `ext-core-v1.3.3`、`ext-fonts-v1.0.0`；`ext-fonts.yml` 没在 GitHub 上跑过。
- macOS、Windows、aarch64 没验证；macOS / Windows 上 Chromium 不读 fontconfig（Q27）。真机手机没验证（只有 Playwright 的 Pixel 7）。
- 节点还没下载 Chromium 的情况只在 e2e（模拟 Workflow）里验证，真实节点上没删浏览器测。
- 打印里没测过的 Block：待办、日历、脑图、手写笔迹、连接线、音频卡片、旋转的 Frame、超高不可拆内容的缩小（代码路径有，没有样例触发）。
- HTML 扩展 Block 在打印里用不了 `aiws.asset()`（file:// 读不到字节）和 `aiws.query(sql)`（打印 store 没有 SQL），用到的退回快照图片。`buckyos.dir` 成员文件的字节、URL 查询表的远端数据不进包，这类 Block 打印出来没有内容。
- `print.js` 18.4 MB：两份 WASM 约 9 MB，其余是 Block 注册连带的模块图。瘦身要把一些 import 改成懒加载，这次没做。
- `create_run(ensure)` 的合并键不含字体包状态；打印走普通 `create_run`，不受影响。
- 系统文件选择器的保存模式会 `stat` 用户给的名字，不存在时浏览器控制台记一条 404；真实 Zone DV 的夹具已忽略这一条。
