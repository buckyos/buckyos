# AI Workspace 脑图与 Block-control TODO

日期：2026-10-27

状态：**已实施（2026-10-28，未提交）**，实施记录与验收见 [doc/workspace/脑图实施记录与验收报告.md](../doc/workspace/脑图实施记录与验收报告.md)；下面的勾选与“实施说明”为准。依据 [doc/workspace/脑图对象方案讨论.md](../doc/workspace/脑图对象方案讨论.md) v0.4（R1–R24 全部已定）推导；第 2 节“现状”是推导时对源码的核对（基线 `02655b25e`），实施前重新核对当前分支。

目标：在 aiws 自由画布默认提供脑图对象，产品形态参考 Miro。脑图不是专用对象，而是**第一个按 Block-control 机制实现的内置 control**：一个有自己布局算法、动作与键盘接管的容器，子节点是普通 Block。本 TODO 同时落地 control 机制里脑图用到的部分；Button 这类“皮 + 内容”的组合 control 只预留 schema，不在本单实现。

## 1. 范围与已定决定

来自方案 §10 的裁决，本单不再讨论，只复述实施需要的结论：

- **结构是 UI、内容绑数据**（R1/R21）：树结构只在 BlockTree，数据源模式不显示；节点绑定的数据实体照常落画布内容区，按 D05 的 owns/bind 管。
- **模型 (b2)**（R1–R4）：容器 kind `mindmap` = control id；每个节点一个 `buckyos.cell`，`parent_id` 即父子、`order_key` 即兄弟序；Cell 可有子 Cell（必需）；边隐式，由容器画；关系线仍用普通连接线。
- **`auto` placement**（R5/R6/R7/R19）：`x`/`y`/`w`/`h` 各自可为数值或字面量 `"auto"`；`auto` 只允许出现在 layout 容器的直接子节点；公式只读容器 config、直接子节点结构、直接子节点已解析尺寸；连接线是唯一跨 Block 例外、第二遍算；pin 即把分量写成数值，“回到自动布局”即写回 `"auto"`。
- **节点内容任意 Block**（R10/R17）：节点 `view.type` 不限；文字节点文字放 `title`（≤ 256 字）且 `w`/`h` 可 `auto`；非文字节点 `w`/`h` 必须数值，core 按 `view.type` 校验。
- **折叠是 `user_state`**（R8）：不写文档、不进撤销与导出；发布页、导出、Agent、放映看全展开。
- **拖出容器拒绝**（R9）：跨容器靠复制粘贴子树（`entity.clone`），不做“拖入即挂接”。
- **四个方向首版都支持**（R13）：`right` / `left` / `both` / `down`，同一 `tree` 算法换参数。
- **布局算法在共享 TS 模块**（R15），纯函数；**不版本化**（R18），用布局快照测试防回归。
- **pin 住的节点按自由主题语义**（R16）：脱离排布、兄弟不留位、边仍画；拖到节点上是重挂，拖到空处是 pin。
- **slot 与路径寻址**（R22）：子 Cell 可带容器内唯一 `slot`，路径 `bkg.text` 逐级解析；脑图不用，本单只做 schema、校验与 outline 投影。
- **内容子节点 / 模板子节点**（R23）：脑图只有内容子节点；`Chrome` 接口留给第一个需要模板子节点的 control。
- **允许 control 嵌套**（R24）：每个容器只算自己的直接子节点、只按自己的方向；内层对外层是一个尺寸已解析的子节点；pin/重挂/键盘接管按最近的容器。
- **Agent 接口走通用机制**（R11）：每个 control 定义自带 `agent` 段；目录渐进披露；许愿格结果加通用 `control` 种类；import/export 按 control 分派。
- **术语**：对父容器尺寸的引用一律写 `parent`，`father` 只在引 Bolt 时出现。
- 版本（R12）：新 kind 属 FORMAT 1.x 可加，PROTOCOL 升小版本；旧读者对未知 kind 降级为不可识别的 group。
- 手机端首版只查看与折叠（R14）。
- 遵守 [AGENTS.md](../AGENTS.md)：不新增第三方布局库；Reingold–Tilford 一类算法自己写在共享模块里。

## 2. 当前代码与接入点

下列链接相对本文。

| 范围 | 现状与需要处理的差距 |
| --- | --- |
| [core/src/types.rs](../src/frame/aiworkspace/core/src/types.rs) `validate_container` | 容器 kind 只允许 `folder` / `surface` / `group`（内部另有 root/data/surfaces）；`layout` 只接受 `{ mode: flow \| free }`；`config` 只允许在 `surface` 上；kind 创建后不可改。脑图需要 kind `mindmap`、容器 `config`、`layout.mode = tree`（或由 kind 推导）。`MAX_CONFIG_BYTES = 64 KiB`。 |
| [core/src/plan.rs](../src/frame/aiworkspace/core/src/plan.rs) `child_allowed` | `surface` / `group` 下只能放 `group` 容器或 Cell；Cell 作为父节点落入 `_ => false`。需要：`mindmap` 容器下放 Cell 或容器（R24）；`mindmap` 内的 Cell 下可放 Cell。 |
| 同文件 `check_placement` | 只接受有限数值，`w`/`h` > 0；没有 `"auto"`；校验不知道父容器是谁。需要：分量可为 `"auto"`，且只在 layout 容器的直接子节点上；非文字节点的 `w`/`h` 必须数值（按 `view.type`）。`max_ops = 10_000`。 |
| [core/src/read.rs](../src/frame/aiworkspace/core/src/read.rs) `outline_extras` | 容器投影 `kind` / `layout` / `system` / `surface_id` / `content_folder_id` / `icon` / `locked`，Cell 投影 `view_type` / `view_version` / `source_id` / `def_id` / `locked`，另有 `title`。需要加 `slot`；`placement` 已投影，`"auto"` 分量要原样透传。 |
| [schemas/commit-request.schema.json](../src/frame/aiworkspace/schemas/commit-request.schema.json) `$defs.placement`、[schemas/operations.json](../src/frame/aiworkspace/schemas/operations.json)、[fixtures/protocol/stable-methods.json](../src/frame/aiworkspace/fixtures/protocol/stable-methods.json) | placement 的 JSON Schema 是数值；契约 fixture 要重录；[aiworkspace README](../src/frame/aiworkspace/README.md) 的协议表要加版本说明。 |
| [core/src/markdown.rs](../src/frame/aiworkspace/core/src/markdown.rs) | `to_richtext(markdown, prefix, resolve)` / `from_richtext(ast)` 已能双向转换嵌套列表；脑图 ↔ 大纲的转换在共享 TS 模块里做，不进 core。 |
| [wasm/build.sh](../src/frame/aiworkspace/wasm/build.sh) | core 改动后要重建 WASM，Desktop 离线校验才与服务一致。 |
| [state/outline.ts](../src/frame/desktop/src/app/aiworkspace/state/outline.ts) | 从变化流增量维护 outline，payload 键白名单（`title`、`view`、`kind`、`layout`、`config`…）决定哪些写入触发投影更新；需要加 `slot`，`placement` 的 `"auto"` 直接进 `EntityEnvelope.placement`。[api/types.ts](../src/frame/desktop/src/app/aiworkspace/api/types.ts) 的 `Placement` 现在是四个 number。 |
| [ui/canvas/layout.ts](../src/frame/desktop/src/app/aiworkspace/ui/canvas/layout.ts) `layoutSurface` | 流式页用 `autoY` 顺序堆叠；group 的外观靠第一个 frame/shape 子 Block 当 `backingId`；连接线在最后 `resolveConnectors` 第二遍算；`Laid` 记录 rect / bounds / depth / paint。需要：遇到 control 容器调用它的布局器填 `auto` 分量，嵌套自底向上，外框写进容器的 `Laid.rect`。[useSurfaceLayout.ts](../src/frame/desktop/src/app/aiworkspace/ui/canvas/useSurfaceLayout.ts) 是唯一调用处。 |
| [ui/blocks/registry.ts](../src/frame/desktop/src/app/aiworkspace/ui/blocks/registry.ts)、[registerAll.ts](../src/frame/desktop/src/app/aiworkspace/ui/blocks/registerAll.ts) | 只有 `BlockDefinition`（按 Cell `view.type` 解析）；没有容器定义。[connectors/definition.tsx](../src/frame/desktop/src/app/aiworkspace/ui/canvas/connectors/definition.tsx) 是“宿主自己画、定义只给工具栏/动作”的先例。 |
| [ui/canvas/CanvasView.tsx](../src/frame/desktop/src/app/aiworkspace/ui/canvas/CanvasView.tsx) | 插入走 `definition.create(...)` 返回一组操作；键盘：方向键微移（`step`）、Enter 进入编辑、流式页 ↑↓ 选块；`topLevel` 处理祖先已选略过子项。需要：选中 mindmap 内节点时键盘由容器接管；拖动落点判定按“最近候选父节点”而不是 `groupAt` 矩形包含。 |
| [ui/canvas/clipboard.ts](../src/frame/desktop/src/app/aiworkspace/ui/canvas/clipboard.ts)、[surfaceOps.ts](../src/frame/desktop/src/app/aiworkspace/ui/canvas/surfaceOps.ts) | 复制粘贴走 planner 的 `entity.clone`（同画布克隆、跨画布引用）；子树复制要带上 `parent_id` 重映射与 `"auto"` placement。 |
| [ui/canvas/render/RenderHost.tsx](../src/frame/desktop/src/app/aiworkspace/ui/canvas/render/RenderHost.tsx) | 三级裁剪、LOD、`MAX_MOUNTED = 400`、按 Laid 命中；节点是 Cell 就走现有路径。边画在 SVG 层（同连接线 `paint.ts`）。 |
| [ui/canvas/FlowSurface.tsx](../src/frame/desktop/src/app/aiworkspace/ui/canvas/FlowSurface.tsx) | 对 group 只有名字和删除；脑图在流式页要整体渲染（固定高度、内部可平移缩放）。 |
| [ui/blocks/shapeBlock.tsx](../src/frame/desktop/src/app/aiworkspace/ui/blocks/shapeBlock.tsx) `saveText` | 原地改字、首行同步 `title`：文字节点复用。 |
| [state/userState.ts](../src/frame/desktop/src/app/aiworkspace/state/userState.ts) | `UserWorkState.get/set(key)`，服务端 + IndexedDB；现有键如 `datatree:expanded`、视口键。折叠集合用新键 `mindmap:collapsed:<container>`。 |
| [ui/canvas/drawioExport.ts](../src/frame/desktop/src/app/aiworkspace/ui/canvas/drawioExport.ts)、[import-drawio/](../src/frame/aiworkspace/import-drawio/) | 共享 TS 模块的先例：Deno 与浏览器同用，Desktop 通过 `@drawio/*` 别名（tsconfig.app.json / tsconfig.node.json / vite.config.ts）引用。布局器与大纲转换按同样方式放 `src/frame/aiworkspace/controls/`。 |
| [server/src/wish/tools.rs](../src/frame/aiworkspace/server/src/wish/tools.rs)、[aiws/README.md](../src/frame/aiworkspace/aiws/README.md)、[aiws/renderers.json](../src/frame/aiworkspace/aiws/renderers.json)、[aiws/prompts/](../src/frame/aiworkspace/aiws/prompts/) | 许愿格结果种类 `text \| record \| file \| html`；程序宿主 API `aiws.result.html(...)`；Renderer 目录 `renderers.json` 由 `server/src/wish/mod.rs` 内嵌并在 `prompts/execute.md` 作为 `catalog/renderers.json` 给模型。control 目录与 `agent` 段按同一条路给模型。 |
| [cmd_skills/aiworkspace_template_dev.md](../src/apps/jarvis_agent/agent/cmd_skills/aiworkspace_template_dev.md) | Agent 手册；写了 `buckyos aiws` 的读动词（`info` / `read` / `query`）与 run 动词。`buckyos aiws` 的实现位置推导时未定位，实施 M3 前先找到。 |
| [core/src/types.rs](../src/frame/aiworkspace/core/src/types.rs) `show-path.steps` | 放映 step 已有 `hide` 列表；脑图放映分步展开复用它，不加属性。 |
| i18n [src/frame/desktop/src/i18n/aiworkspace/](../src/frame/desktop/src/i18n/aiworkspace/) | 所有新文案 en + zh 同时加。 |
| 测试 | core：`core/tests/*.rs`（`connector.rs`、`phase2.rs` 是最近的样式）；Desktop e2e：`tests/aiworkspace/*.spec.ts`（`connectors.spec.ts`、`probe.spec.ts`）；验证命令见 §9。 |

## 3. M0：内核契约

目标：core 接受脑图文档，拒绝不合规的形状；旧读者能降级；离线 WASM 同步。

- [x] `validate_container`：kind 增加 `mindmap`（容器 kind = control id；kind 集合由一张常量表 `CONTROL_KINDS` 给出，便于下一个 control 加入）；`mindmap` 允许 `config`（对象，≤ `MAX_CONFIG_BYTES`，键：`direction` ∈ right/left/both/down、`edge` ∈ curve/elbow/straight、`spacing { sibling, level }`、`branch_colors?`）；`layout` 对 `mindmap` 固定为 `{ mode: "tree" }`，由 kind 推导写入，不允许手填其他值；`locked` 规则同 group。
- [x] `child_allowed`：`mindmap` 容器下允许 Cell 与容器（容器 kind 为 `group` 或任一 control kind，R24）；`mindmap` 内的 Cell 下允许 Cell 与容器；其他位置的 Cell 仍不能有子节点。“在 mindmap 内”的判定沿父链向上找最近的容器，与 `surface_of` 同一写法。
- [x] `tree.move` / `entity.delete(subtree)` / `entity.clone`：对“Cell 有子节点”重新核对循环检测、子树删除展开、克隆的 `parent_id` 重映射；克隆子树时 `"auto"` 分量原样复制。 （实施说明：`entity.clone` 只复制数据实体，Block 与容器由 Desktop 重建；子树复制在 `clipboard.ts`，`"auto"` 原样复制。）
- [x] `check_placement`：分量可为有限数值或字符串 `"auto"`（`rotation` 不参与）；`"auto"` 只允许在 layout 容器（kind 属 `CONTROL_KINDS`）的直接子节点上，其他位置拒绝；容器自身 `x`/`y` 必须数值，`w`/`h` 可 `"auto"`。校验需要父节点：把父容器 kind 传进 `check_placement`，并在 `tree.move` 换父时重新校验（搬出 control 的节点若带 `"auto"` 则拒绝，Desktop 侧据 R9 不会发起）。
- [x] 非文字节点尺寸规则：`view.type` 不在“文字类”集合（首版 `mindnode`、`shape`、`richtext`）内的节点，`w`/`h` 必须数值。集合是 core 常量，Desktop 注册表以同一份为准（放共享模块导出的 JSON，core 用 `include_str!`）。
- [x] `slot`：Cell payload 新键 `slot`（`^[a-z0-9][a-z0-9_-]{0,31}$`），仅允许在 control 容器的直接子节点上，同父内唯一（planner 在同一提交内与现存子节点一起查重）；进 `outline_extras` 投影。
- [x] 未知 kind 降级：核对 Desktop 对 `validate_container` 之外 kind 的实际行为（`layoutSurface`、`DataTree`、`FlowSurface`、导入），保证“不可识别的 group”能打开、保存、导出不丢字段；写一条 e2e。
- [x] 协议与文档：`PROTOCOL` 升小版本；`commit-request.schema.json` 的 `$defs.placement` 改为 `number | "auto"`；`operations.json` 的 note 更新；`stable-methods.json` 重录；aiworkspace README 协议表加一行。
- [x] fixtures：`fixtures/mindmap/`（最小树、四方向、pin 过的节点、嵌套 control、非文字节点、含 slot 的组合容器）；`core/tests/mindmap.rs` 覆盖：合法形状通过；`auto` 出现在自由画布、非文字节点 `auto`、Cell 在普通 group 下带子节点、slot 重复、搬出容器带 `auto` 各一条拒绝；克隆子树与删子树。
- [x] 重建 WASM，Desktop 离线校验与服务一致（`offline.spec.ts` 样式加一条）。

完成条件：三个 crate 测试全过；旧 fixture 不变；新 fixture 经 `doc.commit` 写入并读回与 outline 投影一致。

## 4. M1：布局器、容器定义与渲染

目标：有脑图文档就能在自由画布上正确画出来（含边、折叠、pin、嵌套），不含交互。

共享模块 `src/frame/aiworkspace/controls/`（Deno 与浏览器同用，别名 `@controls/*` 仿 `@drawio/*`）：

- [x] `tree.ts`：分层树布局纯函数。输入：直接子节点列表（id、order_key、已解析尺寸、是否 pin、pin 的数值、`side` 配置）、折叠集合、容器 config；输出：每个 `auto` 分量的值与容器外框。四方向共用一套，`both` 把一级子节点按 `side` 分配、`auto` 时按子树大小平衡；折叠子树按一个节点算；pin 住的节点跳过且其子树相对它排（自由主题）。
- [x] `measure.ts`：文字节点尺寸公式（标题、字体、最大宽度常量）；浏览器用 canvas `measureText`，Deno 用等宽近似；按（文字、字体、最大宽度）缓存。差异是接受的（方案 §5.4）。
- [x] `outline.ts`：Markdown 嵌套列表 ↔ 脑图操作序列（`entity.create` 容器 + 节点，节点 placement 全 `"auto"`）；反向脑图 → Markdown。行内格式保留。M3 的 Agent 入口与 Desktop 的“转成脑图/转成大纲”都用它。
- [x] 快照测试：固定输入 → 固定输出的 JSON 快照（四方向、折叠、pin、嵌套各一），作为“不版本化”的回归护栏（R18）。

Desktop：

- [x] `ui/controls/registry.ts`：`ContainerDefinition`（方案 §6.2 草案）：`kind`、`configSchema`、`children { accepts, containers, slots? }`、`layout { direction, algorithm, run }`、`Chrome?`、`create`、`actions?`、`toolbar?`、`hover?`、`keys?`、`agent`。注册表按 kind 解析，未知 kind 返回降级原因。`mindmap` 定义在 `ui/controls/mindmap/`。
- [x] `layoutSurface`：遇到 control 容器时，先递归算完子容器（自底向上），再调用 `layout.run` 填直接子节点的 `auto` 分量，数值分量照用；容器外框写入其 `Laid.rect`；父赋尺寸的内层 control 把该分量视为数值；连接线仍在最后一遍。缓存键：容器子树 `struct_rev` + 折叠集合 + pin 集合 + config。
- [x] 折叠输入：`user_state` 键 `mindmap:collapsed:<container>`（已折叠节点 id 数组）；缺省全展开；被折叠节点的子树不进 `Laid`（不挂载、不命中），残留 id 忽略。
- [x] `mindnode` Block（`ui/controls/mindmap/nodeBlock.tsx`）：文字节点 Renderer，复用 `shapeBlock` 的 `saveText`，`title` 跟首行；`shape` / `anchors` 给连接线用；显示 pin 标记与折叠柄（折叠柄只在有子节点时）。
- [x] 任意 Block 作节点：`BlockHost` 对 mindmap 内的任何 `view.type` 照常渲染；非文字节点要求数值尺寸由 M0 保证，Desktop 插入时给默认尺寸。
- [x] 隐式边：容器 `Chrome` 在 SVG 层按父子关系画边（curve / elbow / straight），分支颜色在一级子节点上向下继承、节点 `config.branch_color` 可覆盖，粗细按层级衰减；折叠节点只画到它为止；千节点合并为一条 path。边不参与命中。 （实施说明：折叠柄改在画布浮层，屏幕尺寸固定。）
- [x] 容器外观：外框 `w`/`h` 为 `auto`，不可缩放；hover 显示容器名；`backingId` 逻辑对 control 容器不生效。
- [x] 流式页：mindmap 容器作为一个自包含 Block 整体渲染（固定高度、内部可平移缩放），`FlowSurface` 要能渲染“有自己外观的 group”。
- [x] 放映：`show-path.steps.hide` 对节点生效即可，不加属性。
- [x] 探针：`probe.spec.ts` 加千节点脑图（一帧内算完布局、平移不卡）与 10 层嵌套 control 两条。

完成条件：导入 M0 的 fixture 后画布显示正确；四方向快照与浏览器渲染一致；折叠、pin、嵌套三种 fixture 各有截图 e2e。

## 5. M2：交互

- [x] 插入：插入目录加“脑图”（group `layout`，`create` 建容器 + 根节点，根节点进入编辑）；在节点上“添加子节点 / 兄弟节点”走容器 actions。
- [x] 键盘接管（选中 mindmap 内节点时由容器 `keys` 处理，其余交回画布）：Tab 加子并编辑；Enter 加兄弟并编辑（编辑中 Enter 结束、Shift+Enter 换行，同 shape）；← → ↑ ↓ 在树上导航（`both` 时左右按侧翻转，`down` 时上下为父子）；Delete 删子树（复用 `deleteSelection` 的 descendants）；Ctrl+/ 折叠展开；Ctrl+D 复制子树（`entity.clone`）。方向键微移在容器内不再生效。
- [x] 拖动重挂：拖的是子树；松手前用“假设重挂到这里”实时预览（布局器以候选父与插入位为输入重算）；落点判定按最近候选父节点与插入位置；落到另一个节点上是重挂（`tree.move`，placement 仍 `auto`）；落到容器内空处是 pin（`x`/`y` 写数值，一次提交一次撤销）；拖出容器外松手拒绝并回原位（R9）。
- [x] pin 节点：有可见标记；工具栏与右键“回到自动布局”（写回 `"auto"`）；文字改长时 `w`/`h` 仍跟随（分量级 `auto` 的意义）。
- [x] 复制粘贴：同画布粘贴子树克隆（含 `"auto"` 与 pin 数值）；跨画布按 D05 引用；粘贴到另一棵脑图的某节点下成为其子树；从画布普通对象粘贴进脑图节点下即成为节点（非文字对象要有数值尺寸）。
- [x] 折叠：点击折叠柄或 Ctrl+/ 写 `user_state`；折叠状态不进撤销栈；协作者各自独立。
- [x] 容器工具栏与属性面板：方向、边样式、间距、分支配色；节点属性面板：分支色、side（仅一级）、回到自动布局。 （实施说明：未单列属性面板，全部放在近旁工具栏。）
- [x] 框选与多选：按 `topLevel` 规则，祖先已选不重复选子项；多选节点删除/移动按子树处理。
- [x] 连接线：普通连接线可绑定脑图节点（关系线）；节点随布局移动时连接线跟随（第二遍已保证）。
- [x] 锁与批注：节点 `locked`、批注、`presentation` 照常可用；锁住的节点不能重挂与 pin。
- [x] 手机端：只查看与折叠（`touch.ts` 现状），编辑动作禁用并提示。
- [x] e2e：`tests/aiworkspace/mindmap.spec.ts` 覆盖键盘建树、拖动重挂与预览、pin 与回到自动、拒绝拖出、复制粘贴子树、折叠与协作者独立、锁住节点。

完成条件：不看文档能用键盘和鼠标把一棵 50 节点的脑图建出来、整理好、折叠好；所有写操作一次手势一次 commit 一次撤销。

## 6. M3：Agent、许愿格与转换

- [x] `ContainerDefinition.agent`：每个 control 带 Markdown 自述（是什么、config 各项含义、子节点规则、`doc.commit` 建实例与子节点的最小操作序列、交换格式、禁忌：不要写坐标，非文字节点给尺寸）。`mindmap` 的 `agent` 写好并由共享模块导出。
- [x] 目录：共享模块生成 `aiws/controls.json`（kind、一句话、何时用、`agent` 全文），`server/src/wish/mod.rs` 内嵌，`prompts/analyze.md` / `execute.md` 把它作为 `catalog/controls.json` 给模型（与 `renderers.json` 并列）。
- [x] 许愿格结果：`tools.rs` 结果种类加 `control`（`{ kind, config?, children: 大纲 }`）；程序宿主 API 加 `aiws.result.control(name, { kind, config, children })`，`children` 接受 Markdown 大纲或嵌套数组；服务端落地时用共享 `outline.ts` 生成容器与节点操作，`output_mode` 的 overwrite / new 语义不变；“扩展这一分支”是以某节点 Cell 为输入的 wish（核对 `REF_KINDS` 允许 Cell 作 `input` 目标，不允许则加）。
- [x] Desktop：流式页嵌套列表“转成脑图”、脑图“转成大纲插入文档”两个动作，同一转换函数。
- [x] 手册：`aiworkspace_template_dev.md` 加“control 目录”一节（只列目录与取 `agent` 全文的方法）与“从 Markdown 大纲建脑图”范例；`aiworkspace_run.md` 加 `control` 结果用法。
- [x] CLI：先定位 `buckyos aiws` 的实现；加 `controls list` / `controls describe <kind>`（直接读共享模块目录，不经服务）与按 control 分派的 `import` / `export`（脑图：Markdown 大纲、OPML）。 （实施说明：CLI 在 websdk 仓库；读不到共享模块，经服务 `control.list/describe/import/export`。动词形如 `buckyos aiws controls list`。）
- [x] 测试：`server/tests/wish_control.rs`（control 结果落地、重跑覆盖、扩展分支）；共享模块的大纲转换单测；e2e `wish.spec.ts` 加一条“许愿得到脑图”。

完成条件：Agent 只凭 `controls.json` 和手册就能从一段 Markdown 建出脑图并扩展一个分支，全程不写坐标。

## 7. M4：导入导出与收尾

- [x] OPML 双向（共享模块，与大纲同构）。
- [x] draw.io 导出：隐式边物化为 mxCell edge，节点为带绝对坐标的 vertex（运行共享布局器取坐标）；导入不识别脑图语义，按普通形状。
- [x] 发布页 / HTML 预渲染：全展开形态，边与节点静态 SVG。 （实施说明：没有单独预渲染管线；访客无折叠状态即全展开；静态 SVG 用于流式页与许愿格预览。）
- [x] 手机端查看与折叠的 e2e（`mobile.spec.ts`）。
- [x] FreeMind / XMind 导入列为后续，不在本单。 
- [x] 文档：Desktop README 加 `ui/controls/`；aiworkspace README 加共享模块与协议说明；方案文档 §11 改“已实施”并写实施记录与验收报告（沿用 `doc/workspace/*实施记录与验收报告.md` 格式）。

## 8. 不在本单

- 用户定义的组合 control（Button 类）：`slot`、`Chrome`、`algorithm: 'fixed'` 只预留 schema 与接口；模板子节点不实现。
- 自顶向下的 layout 算法；表达式位置。
- 流式页迁到 `auto` 原语。
- 泳道、网格、FreeMind / XMind 导入。

## 9. 验证命令

- core：`cargo test -p aiworkspace-core -p aiworkspace-store -p aiworkspace`；WASM：`PATH=/tmp/dev-cache-root/cargo/bin:$PATH CARGO_TARGET_DIR=/tmp/dev-cache-root/cargo-target bash src/frame/aiworkspace/wasm/build.sh`。
- 共享模块：`deno test src/frame/aiworkspace/controls/`。
- Desktop：`pnpm check && pnpm exec eslint src/app/aiworkspace tests/aiworkspace && pnpm build`；e2e：`PATH=/tmp/dev-cache-root/cargo/bin:$PATH AIWS_BIN=/tmp/dev-cache-root/cargo-target/debug/aiworkspace pnpm exec playwright test --config=playwright.aiworkspace.config.ts`（探针跑在 `vite preview` 上）。
- 真实 Zone DV：全新安装（不写兼容），导入 M0 fixture，验 Agent 路径（M3）。
