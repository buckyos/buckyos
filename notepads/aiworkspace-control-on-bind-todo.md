# AI Workspace 控件 on_bind TODO：插入前由控件决定绑定

2026-11-12 立项。来源：owner 在 DV 里插入待办 / 日历后的讨论（[控件机制 TODO](aiworkspace-control-mechanism-todo.md) §7.5）。

## 0. 目标

owner 原话：

> 控件本身有一个 on_bind 事件（插入画布前调用，返回失败则不插入），可以进行自定义的 binding 逻辑，包括弹出必要的对话框。为了简化控件的开发，系统可以提供几个默认的对话框，比如选择 1 个或多个数据源。

> 其实我更希望这个扩展，基本都是靠标准的控件机制实现的。

范围（owner 2026-11-12 “按建议的范围”）：

- 本单做：内置控件（待办、日历）、组合控件用声明写的 on_bind / on_init_control、宿主的默认对话框、控件没写 on_bind 时的默认规则、Agent 走同一流程（`no_ui`）。
- 第二步另做：HTML 控件在 `behavior.js` 里写 `aiws.on('bind')`。它需要“插入前不显示地跑一次控件脚本”的新运行方式。

要收回的东西：11-12 为待办 / 日历加的专门实现（数据源选择、多张待办表、增删绑定的菜单、按表着色），改成这套通用机制的一个用法。

## 1. 现状（已对照代码）

### 1.1 绑定声明

- `component.json` 的 `bindings`：`{ <name>: { accepts?, required?, title?, description? } }`，内核 `core/src/types.rs` `check_def_bindings` 校验，不认识的键报错。`accepts` 只能写实体类型（如 `buckyos.table-source`、`buckyos.dir`），写不了“哪种表”。
- 内置控件的公开契约（`controls/contracts.ts`，生成 `aiws/contracts/*.json`）：形状同 `component.json`，另有 `schema_id`、`owned`，以及通配绑定名：日历的 `todo_*`、`*`（11-12 加的，契约版本 calendar 2）。契约只给 Agent 读（`control.describe`），宿主不读它，也不经过 `check_def_bindings`。
- 实例上的绑定存在 Cell / 容器的 `payload.bindings`：`{ <name>: { entity_id } }`，一个名字一张。写入用 `entity.set_props { key: "bindings", props: { <name>: {entity_id} | null } }`。内核按 Cell 绑定的规则校验，不校验所绑的表的 `schema_id`。

### 1.2 插入与改绑定

- 扩展（组合、HTML）从目录插入：`ui/canvas/InsertCatalog.tsx` 显示 `ui/extensions/InstanceForm.tsx`，每个绑定一个下拉框，选项是 `dataFor(store, accepts)`。必填绑定没选，就不能点“插入”。没有“新建”。
- 已有实例改绑定：组合控件的属性面板（`ui/controls/composite/Inspector.tsx`）用同一个 `InstanceForm`，失焦即提交。HTML 实例没找到改绑定的入口。
- 内置数据控件（11-12 加，未提交）：
  - `ContainerDefinition.source { binding, schemaId, table }`；
  - `CanvasView.insert` 里按 `source` 处理：没有就新建，一张就绑，多张弹 `SourceChooser`（单选，带“新建表格”）；
  - 未绑定的占位 `UnboundSource`；
  - 日历 `create` 里绑上全部待办表（`todo`、`todo_2`……）；
  - 日历菜单三项：再显示一张日历表、显示待办表的截止日期、不再显示某张表；
  - 每张表的颜色由日历的 `colorFor` 算，`config.colors` 可按绑定名覆盖。
- 新建的表放在当前画布的内容文件夹，名字与同级重名时用 `uniqueName` 加 ` (2)`，和控件在同一个 Commit 里创建，归控件所有（`owns`）。

### 1.3 例子

- 照片投票（`doc/workspace/control-examples/controls/photo-vote`）：
  - `photos`（目录）、`votes`（表）都是必填；
  - `votes` 表要使用者事先手工建好：一个 `submitter` 文本字段，加一个带 `max_items` 的多选字段 `picks`；
  - 插入后再跑动作 `prepare`，把照片名填成选项。
- 一起听：`playlist`（目录）必填，插入时从下拉框里选。

### 1.4 可复用的

- `SourceChooser`（单选，带新建，渲染在 `.aiws-root` 上）。
- websdk `standardTableOp`，以及内核 `core/src/standard.rs` 登记的标准表 `buckyos.todo@1`、`buckyos.calendar@1`。
- manifest 4 `actions` 的 `run` handler（write / wish / open / notify / config / forward），以及 `addOptions` 写入：可以作为声明式步骤的先例和执行器。
- `entity.set_props` 写 `bindings`，以及组合控件属性面板的 `submitProps`。

## 2. 方案

### 2.1 两个事件

**on_bind**：决定这个控件绑哪些数据。

- **什么时候调用**：
  - 从目录、工具栏、流式页插入画布前；
  - 未绑定控件的“选择或新建”；
  - 改绑定的入口（“显示哪些表…”、属性面板）；
  - Agent 放置控件时（Q6，见 2.8）。
  - 复制粘贴、导入不调用，这些情况下绑定已经写好了。
- **调用顺序**：一次插入可能带出一棵控件树，比如组合控件和它各个 slot 里的第一批 Block。按父到子的顺序调用，子控件在 `ctx.parent` 里拿到父控件已经定好的绑定（Q4）。
- **输入 `ctx`**：
  - `reason`：`insert` 或 `rebind`；
  - `no_ui`：为真时不能弹对话框，调用对话框就直接失败（Q6）；
  - `given`：调用方已经给定的绑定。Agent 放置时由 Agent 传入；改绑定时是现有绑定；
  - `parent`：父控件的声明和已定绑定（顶层控件没有）；
  - 控件自己的声明：`bindings`、`config` 默认值、标题；
  - 所在画布（surface、内容文件夹）；
  - `find(query)`：按实体类型、`schema_id` 列出工作区里的数据，带名字和所在位置；
  - `dialogs`：见 2.2。
- **输出**：
  - 成功：`{ bindings: { <name>: id | id[] }, create?: [...] }`。
  - 失败或取消：`null`，或带原因的失败。整次插入取消；`rebind` 时什么都不改。
  - `create` 是新建数据的清单，每项带一个临时引用 `ref`，`bindings` 里可以用这个 `ref`。可以新建三种：
    - 标准表（`schema_id`）；
    - 按字段建的表（`name`、`fields`、`title_field_id`）；
    - 空目录。

**on_init_control**（Q4）：所有 on_bind 都完成后调用，顺序反过来，从子到父。父控件最后检查子控件是否都已就绪。

- **输入**：自己最终的绑定、要新建的数据，以及每个子控件的结果（是否就绪、绑定）。
- **输出**：就绪，或者失败。失败时整次插入取消，并说明原因。
- **默认做法**：自己或任一子控件还缺必填绑定时失败。

**宿主怎么提交**：

- 新建的数据放进当前画布的内容文件夹，同级重名时改名，归建它的控件所有。
- 插入：整棵树的新建数据、控件、绑定放在一个 Commit 里。
- `rebind`：新建数据和 `entity.set_props { key: "bindings" }` 放在一个 Commit 里。
- 插入成功后，再执行控件声明的后续动作（2.6 的 `then`）。

### 2.2 默认对话框（宿主提供，各类控件共用）

| 对话框 | 参数 | 返回 |
| --- | --- | --- |
| `pickOne` | `title`、`note?`、`query`、`allow_new?`、`current?` | id，或 `{ new: true }`，或 `null`（取消） |
| `pickMany` | `title`、`query`、`preselect`（`all` / `none` / id 列表）、`allow_new?` | id 列表，或 `null` |
| `confirm` | `title`、`text` | 布尔 |

列表的每一行显示名字和所在的画布或文件夹，“新建”放在最后。手机上用底部弹出的面板。从现有的 `SourceChooser` 扩展而来。`no_ui` 时三者都直接失败，失败原因说明是哪个绑定要人来选。

### 2.3 绑定声明与存储

声明（`component.json` 和内置契约用同一种形状）加两个字段：

- `schema_id`：只接受这种标准表。宿主只列出这类表；是内核登记过的标准表时，宿主还能新建一张。
- `many`：可以绑多张。内置契约去掉通配名：日历改成 `events { required, many }` 和 `todo { many }`。

存储（Q2，改成数组）：实例的 `payload.bindings` 是 `{ <name>: { entity_id, selector? } | [{ entity_id, selector? }, …] }`。只有声明了 `many` 的绑定可以是数组，绑了零张时省略。

- **内核**：
  - `normalize_bindings` 接受数组，逐项检查目标，限制每个绑定最多几张；
  - `add_owns_edges` 和引用边逐项处理；
  - 大纲 `outline_extras` 的 `bindings` 输出为 `{ <name>: id | id[] }`；
  - `entity.set_props { key: "bindings" }` 写一个绑定时整体替换。
- **Desktop**：本地大纲镜像同步改；用到绑定的地方都改，包括组合控件的投影、日历和 `InstanceForm`。

`check_def_bindings` 认识 `schema_id` 和 `many`。内置契约由宿主读取（Desktop 已能 `import @controls/contracts`），作为内置控件的绑定声明。

### 2.4 默认规则（控件没写 on_bind 时，按声明执行）

0. 已经给定的（`given`）直接用。父控件已经绑了、类型对得上的数据优先：单张绑定在父控件那里恰好有一份时用它；`many` 绑定用父控件那里所有对得上的。
1. 其余按下表：

| 声明 | 没有可用数据 | 只有一份 | 有多份 |
| --- | --- | --- | --- |
| 必填，单张 | 能建就新建；建不了就失败，说明原因（Q5） | 直接绑 | `pickOne`，能建就带“新建” |
| `many` | 必填时同上；可选时不绑 | 绑上 | 全部绑上 |
| 可选，单张 | 不绑 | 直接绑 | 不绑 |

`rebind` 的默认做法：每个绑定一个对话框。单张绑定用 `pickOne`，标出当前选中的；`many` 绑定用 `pickMany`，预选现有的。

### 2.5 内置控件

待办（`items`：必填、单张、`buckyos.todo@1`）和日历（`events`：必填、`many`、`buckyos.calendar@1`；`todo`：`many`、`buckyos.todo@1`），正好落在默认规则里，不用自己写 on_bind。11-12 的专门实现全部删掉：

- `ContainerDefinition.source`；
- `CanvasView.insert` 里的分支；
- 日历 `create` 里的“绑上全部待办表”；
- 日历菜单的三项，改成一个入口“显示哪些表…”，即 `rebind`。

日历改读数组：`events`、`todo` 各是一组表；隐藏的表、颜色覆盖都按表的 id 记。`ContainerDefinition` 留可选的 `onBind(ctx)` 和 `onInitControl(ctx)`，给以后不符合默认规则的内置控件用。

### 2.6 组合控件：在声明里写

组合控件没有代码，`component.json` 加两项（manifest 升到 5，Q4）。

`on_bind`：一组按顺序执行的步骤（最多 16 步），每一步是下面之一：

- `{ "binding": "<name>", "pick": "one" | "many" | "all", "title"?, "allow_new"? }`：用默认对话框选；`all` 是不问、全部绑上。
- `{ "binding": "<name>", "from_parent": "<父控件的绑定名>" }`：沿用父控件的绑定。
- `{ "binding": "<name>", "create": { "standard": "<schema_id>" } | { "table": { "name", "title_field_id", "fields" } } | { "dir": { "name" } }, "when"?: "missing" | "always" }`：新建。默认只在没有可用数据时建（`missing`）。

步骤没写到的绑定，按默认规则处理。

`on_init_control`：

- `{ "require": "children" }`：子控件都要就绪，这也是默认做法；
- `{ "then": { "action": "<声明过的动作 id>" } }`：插入成功后跑这个动作。

照片投票改成：

- `on_bind`：
  1. `photos`：`pick: one`；
  2. `votes`：`create: table`，字段是 `voter`（submitter）和 `picks`（多选，`max_items` 3），`when: always`，每个投票有一张自己的表。
- `on_init_control`：`then: prepare`。

插入后就能直接投票。一起听改成 `playlist`：`pick: one`。

### 2.7 颜色（Q3 按建议）

宿主统一分配：`many` 绑定的每张表一个颜色，从统一的调色板按顺序取，接在前面的绑定之后。

- **覆盖颜色**：`config.colors` 原定按绑定名覆盖。改成数组后，一个绑定名对应多张表，所以覆盖改为按表的 id（`{ <entity_id>: "#rrggbb" }`）。这是按 Q2 做的调整，请过目。
- **投影**：组合控件投影出的每条记录带 `item.binding`（绑定名）、`item.source`（表的 id）、`item.binding_color`。
- **日历**：用宿主分配的颜色。

### 2.8 Agent 放置控件（Q6）

Agent 放置控件同样走 on_bind 和 on_init_control，固定 `no_ui`：

- Agent 给的绑定放在 `given`；
- 默认规则里不需要选择的步骤照常执行，比如没有表就新建、只有一份就直接绑；
- 需要人来选时失败，说明是哪个绑定要 Agent 给出。

落实方式（我的理解，请过目）：

- on_bind、on_init_control 的执行器写成共享模块，放在 websdk 的 `src/aiws_bind.ts`（和 `aiws_schedule.ts`、`aiws_ext.ts` 一样，Desktop 和 CLI 共用；websdk 引用不了 buckyos 的 `controls/`）。内容是默认规则、组合控件的声明步骤，以及两个事件的执行顺序。
- 内置控件的绑定声明：Desktop 读 `@controls/contracts`，CLI 读 `control.describe` 返回的契约。
- 新增命令 `buckyos aiws place <控件> --surface <画布> [--bind <name>=<id>[,<id>…]] [--config <json>]`，跑完后提交一个 Commit。
- Agent 直接写 `doc.commit` 时内核不会调用 on_bind，内核是 Rust，跑不了控件逻辑。手册里规定 Agent 放置控件要用 `place`，不手写插入操作。

### 2.9 第二步（2026-11-12 已实施，见 §8）

HTML 控件：

- `behavior.js` 里写 `aiws.on('bind', …)` 和 `aiws.on('init_control', …)`，用法同 2.1；
- 宿主插入前在不显示的沙箱里跑它，拿到结果后关掉；
- 需要 host_api 5；
- Agent 走 `place` 时怎么跑它，到时再定。

## 3. 裁决（owner 2026-11-12）

| # | 问题 | 裁决 |
| --- | --- | --- |
| Q1 | 改绑定是否也走 on_bind | 是，用 `ctx.reason` 区分（按建议） |
| Q2 | `many` 怎么存 | 改成数组（owner：“这个版本没有历史数据负担，改成数组吧（语义更清楚）”），见 2.3 |
| Q3 | 多张表各一种颜色归谁 | 宿主统一分配，投影带 `item.binding`、`item.binding_color`（按建议）。按 Q2 把覆盖改为按表的 id，见 2.7 |
| Q4 | 组合控件的声明是否升 manifest 5 | 是（按建议）。owner 补充：“按父控件->子控件调用 on_bind，这样子控件能直接拿到父控件完成的 binding。从事件的完备性来说，还要加一个反向调用顺序（子->父）的 on_init_control 事件，方便父控件最后检查是不是所有的子控件都就绪了”，见 2.1、2.6 |
| Q5 | 必填、没有可用数据、又建不了时 | 不插入，说明“工作区里还没有可用的 <绑定标题>”（按建议） |
| Q6 | Agent 直接写操作时 | 也调用（owner：“我觉得还是调用比较好，无非就是 ctx 里已经把数据准备好了必定不弹 UI（也可以在 ctx 里加一个 no_ui 的参数，一旦触发走 UI 的路径就失败）”），落实方式见 2.8 |

## 4. 任务（按顺序）

- **T1 声明与存储**：
  - 声明加 `schema_id`、`many`：改 `check_def_bindings`、`schemas/component.schema.json`；
  - 内核接受数组形式的绑定：`normalize_bindings`、引用边和 `owns` 边、大纲 `outline_extras`、`entity.set_props`；
  - 内置契约改用 `many`（去掉 `*`、`todo_*`），重新生成 `aiws/contracts`；
  - Desktop 本地大纲镜像和 WASM 重建；
  - 补 Rust 和 deno 测试。
- **T2 共享执行器**：websdk `src/aiws_bind.ts`。
  - 内容：两个事件按父到子、子到父的顺序执行；`ctx`（包括 `no_ui`、`given`、`parent`、`find`、对话框接口）；默认规则；组合控件声明步骤的解释器；把结果整理成一个 Commit 的操作。
  - 用 jest 测试。
- **T3 默认对话框**：`pickOne`、`pickMany`、`confirm`，由 `ui/controls/data/SourceChooser.tsx` 扩展而来，手机上用底部面板。
- **T4 Desktop 接入**：
  - 插入入口：`CanvasView.insert`、目录、工具栏、流式页、组合控件带出的子控件；
  - 未绑定占位，以及改绑定入口（属性面板、“显示哪些表…”）；
  - 目录插入扩展时，原来要求先选完必填绑定，改为插入时走 on_bind。
- **T5 内置控件迁移**：删掉 11-12 的专门实现（见 2.5）；绑定声明改为读契约；日历改读数组。
- **T6 组合控件**：
  - manifest 5，`ext.rs` 校验 `on_bind`、`on_init_control`；
  - websdk `aiws_ext` 预检同步；
  - 照片投票、一起听改用新声明，示例的 `aiws-manifest.json` 重算 digest。
- **T7 颜色**：宿主分配，投影带 `item.binding`、`item.source`、`item.binding_color`，日历改用宿主的颜色（见 2.7）。
- **T8 Agent 放置**：`buckyos aiws place`（websdk `cli/modules`），`no_ui` 下失败时说明缺哪个绑定。
- **T9 文档与手册**：
  - kernel 05、02（数组形式的绑定）；
  - 扩展实现思路；
  - 手册 `aiworkspace_template_dev`：`bindings` 字段、两个事件、manifest 5、`place` 命令；
  - `control-examples` 的 README 和 AGENT.md。
- **T10 测试**：
  - deno、Rust（声明和数组校验、事件声明校验、示例构建）；
  - websdk jest（`place`、预检）；
  - e2e：内置控件插入的各种情形、取消不插入、改绑定、照片投票插入后直接能投、父到子沿用绑定、子控件没就绪时整次取消、手机；
  - 本机 Zone DV，包括 Agent 用 `place`。

## 5. 验收场景

1. 插入待办：
   - 没有待办表就新建；
   - 有一张就绑上；
   - 有多张就选一张或新建；
   - 取消就不插入。
2. 插入日历：
   - 日历表按第 1 条处理；
   - 待办表全部带上，每张一个颜色；
   - 在“显示哪些表…”里增删日历表和待办表；
   - 取消就什么都不改。
3. 插入照片投票：选一个照片目录，就自动建好投票表、准备好选票，插入后两个人马上能投。
4. 插入一起听：选播放列表目录。
5. 组合控件的子控件声明了 `from_parent`，或按默认规则对得上父控件的绑定：直接沿用，不再弹框。有子控件没就绪时，父控件的 on_init_control 失败，整次插入取消并说明原因。
6. 必填、没有可用数据、又建不了：不插入，说明原因。
7. 手机上用底部面板完成以上操作。
8. Agent 用 `place` 放置：
   - 给齐绑定就直接成功；
   - 待办没有表时自动新建；
   - 有多张表又没给绑定时失败，并说明要给哪个绑定；
   - 照片投票只要给出 `photos`。
9. 只有容器、没有绑定的控件：显示“选择或新建”，走同一个 on_bind。

## 6. 关联

- [控件机制 TODO](aiworkspace-control-mechanism-todo.md) §7.5：这次要收回的专门实现。
- 文档：
  - `doc/workspace/kernel/05-Block控件与标准库.md`
  - `doc/workspace/kernel/02-数据模型与引用.md`
  - `doc/workspace/BuckyOS AI Workspace 扩展实现思路.md`
- 内核：
  - `src/frame/aiworkspace/schemas/component.schema.json`
  - `core/src/types.rs`（`check_def_bindings`、`normalize_bindings`）
  - `core/src/plan.rs`（`add_owns_edges`、`entity.set_props`）
  - `core/src/read.rs`（`outline_extras`）
  - `core/src/standard.rs`
  - `store/src/ext.rs`
- 共享控件：`src/frame/aiworkspace/controls/contracts.ts`。
- websdk：
  - `src/aiws_bind.ts`（新的执行器）
  - `src/aiws_ext.ts`
  - `cli/modules/aiws.ts`（`place`）
- Desktop：
  - `ui/controls/data/{source.ts,SourceChooser.tsx}`
  - `ui/canvas/{CanvasView.tsx,InsertCatalog.tsx}`
  - `ui/extensions/InstanceForm.tsx`
  - `ui/controls/composite/{Inspector.tsx,model.ts,Projections.tsx}`
  - `ui/controls/calendar/`
  - `state/outline.ts`
- 示例：`doc/workspace/control-examples/controls/{photo-vote,listen-together}`。

## 7. 实施记录（2026-11-12，已实施、已部署本机 Zone 做 DV，未提交）

已完成：

- **T1（内核部分）**：
  - 绑定声明认 `schema_id`、`many`（`check_def_bindings`），`component.schema.json` 同步；
  - 实例绑定可以是列表：`normalize_bindings` 逐项检查，每个绑定最多 32 项，重复的拒绝，空列表去掉；
  - 引用边和 `owns` 边逐项处理，`owns` 扩到组合控件，以及 Cell 的 `bindings`；复制时逐项换 id；大纲输出 `id | id[]`；
  - 扩展升级时，`many` 变单张、`schema_id` 变化算收窄，实例绑了多张而新声明只收一张算不兼容；
  - 新测试 `core/tests/bindings.rs`。
- **契约**：日历 `events`、`todo` 改为 `many`，去掉 `todo_*`、`*` 通配；颜色覆盖和隐藏改按表的 id；契约版本 calendar 2；Agent 自述同步。
- **T2**：websdk `src/aiws_bind.ts`：
  - 两个事件按父到子、子到父的顺序执行；
  - `ctx`（`no_ui`、`given`、`parent`、`find`、对话框接口）；
  - 默认规则、组合控件声明步骤的解释器、`createOps`、`bindingsPayload`、`checkBindSteps`；
  - 测试 `tests/aiws_bind.test.ts`，10 条。
- **T6（内核与 websdk 部分）**：
  - manifest 5：`ext.rs` 构建时检查并保存 `on_bind` / `on_init_control`（内核 `check_bind_steps`，定义实体直接写入时也查）；
  - `block-def` / `component` / `aiws-manifest` 三个 schema 同步；
  - websdk 预检 `aiws_ext` 同步，加了 v5 测试；
  - 照片投票改成 manifest 5，声明“选照片目录 → 新建投票表 → 插入后准备选票”，README、AGENT.md 同步，digest 重算；
  - 一起听是 HTML 控件，声明步骤不适用，按默认规则走（只有一个目录就绑，多个就问），未改。
- **Desktop 宿主清单版本**：`HOST_MANIFEST_VERSION` 升到 5。

验证：

- `cargo test -p aiworkspace-core -p aiworkspace-store -p aiworkspace` 全部通过，包括新的 `bindings.rs` 和 `ext.rs` 的 `control_examples_and_manifests`；
- deno 控件测试 34 条通过；
- websdk jest：`aiws_bind` 10 条，`aiws_ext` / `aiws_schedule` 共 40 条，通过。

Desktop 部分（owner 提交并说“继续”之后）：

- **T3 默认对话框**：
  - `ui/controls/data/BindDialogs.tsx`：`pickOne` / `pickMany` / `confirm`，“新建”可勾选，手机上从底部弹出，渲染在 `.aiws-root` 上；
  - `dialogBus.ts` 是请求总线，工作区页面挂一个 `BindDialogHost`；
  - 对话框的 test id 沿用 `aiws-source-chooser`。
- **T4 接入**：
  - `ui/controls/data/binding.ts` 的 `bindPlacement`：从插入的操作里找出控件树（数据控件按契约；组合控件、HTML / 声明式 Cell 按定义），按两个事件的顺序执行，新建的数据和绑定并进同一个 Commit，数据的排序键和名字避开同一次插入里的其他数据；
  - `CanvasView.insert` 所有插入都走它，插入后执行 `then` 动作；
  - `bindControl` 处理未绑定占位的“选择或新建”和改绑定（日历的“显示哪些表…”、组合控件属性面板的“更改绑定的数据…”）；
  - 目录插入扩展时不再要求先选完必填绑定；`InstanceForm` 不再显示列表绑定的下拉框。
- **T5 内置控件迁移**：
  - 删了 `ContainerDefinition.source`、`SourceChooser.tsx`、`CanvasView` 里的专门分支、日历菜单的三项、日历 `create` 里的“绑上全部待办表”；
  - 待办 / 日历的 `create` 只建容器；
  - 日历按列表读 `events` / `todo`，隐藏和颜色按表的 id；
  - 内置控件的绑定标题本地化（`待办表`、`日历表`……）；
  - 别处做的“标准表变体”（表格 Block 的 `config.standard`）保留原规则（工作区里已有这类表就问），改用同一个 `pickOne`；`source.ts` 只剩它要的 `standardTableOf` 和 `freeTableName`。
- **T7 颜色**：`ui/controls/data/colors.ts` 统一分配；组合控件投影的每条记录带 `item.binding`、`item.source`、`item.binding_color`；列表绑定的 records / counts 投影读所有表。
- **T8**：websdk `cli/modules/aiws_place.ts`：`buckyos aiws place <todo|calendar|定义> --workspace --surface [--bind "a=x;b=y,z"]`，`no_ui`，一个 Commit，输出 `then` 动作。
- **其他**：
  - 本地大纲镜像按列表读绑定；
  - WASM 已重建；
  - websdk 私人日程页的日历绑定改为列表；
  - `pickMany` 可返回“新建”（websdk 测试 11 条）。
- **T9 文档**：
  - kernel 05：新节“放置控件时的绑定：on_bind 与 on_init_control”，标准控件一节改写；
  - kernel 02：列表绑定；
  - 手册 `aiworkspace_template_dev`：`bindings` 的 `many` / `schema_id`、manifest 5 步骤、`place`；
  - 控件自述（`controls/schedule.ts`）改为先用 `place`。

验证：

- Desktop `tsc` / `eslint` 无报错；
- e2e：TD10（没有 / 一张 / 多张 / 取消 / 日历 / 未绑定就地绑定）、CA10（插入带上全部待办表；“显示哪些表…”多选、新建日历表、去掉一张、取消不改；每张表颜色不同；之后插入的日历带上全部表）、CE05（照片投票：选目录、建投票表、准备选票、第二个实例另建一张表、取消不插入）。

还没做：

- HTML 控件的 `aiws.on('bind')`（第二步）；
- `place` 只放容器，不建组合控件各 slot 的第一批 Block；
- `then` 动作 CLI 不执行，只列出。

回归（本地 e2e，串行跑）：

- todo / calendar / schedule / control-examples / extensions / mindmap / ui：全过；extensions 有两条用例在插入后加了等待，因为插入现在要先走 on_bind，是异步的；
- features / file-picker / canvas / connectors / flow / presentation：68 条通过；
- CN17（锚点）失败，组合控件复制粘贴 / 选择偶发失败：在 HEAD 上同样失败，与本次无关。

本机 Zone DV（`uv run buckyos-build.py -s aiworkspace desktop` + `uv run start.py`，`playwright.aiworkspace.zone.config.ts aiworkspace.controls`，6 条全过）：

- 原有 5 条（控件机制 TODO §7.4）在新契约下照常通过；
- 新增 `[ctl] on_bind`：
  - 服务给出的日历契约是 version 2，`events` 为 `many`；
  - Agent 用 `buckyos aiws place`：
    - 没有待办表时新建一张，并归控件所有；
    - 用 `--bind` 指定时按指定的绑；
    - 有两张待办表、又不给 `--bind` 时，失败为 `NEEDS_UI`，并说出是哪个绑定；
    - 日历新建自己的日历表，并带上全部待办表；
    - 照片投票有两个目录、不给 `--bind` 时失败，说出 `photos`；给了之后新建投票表（`voter` 是提交人字段），`then` 为 `["prepare"]`；
  - owner 在浏览器里从目录插入照片投票：弹出选目录对话框，选 `party`，新建投票表，选票准备好（选项是 `Cake.png`）。
- DV 发现并修了一处：`place` 原来默认都放在 (40,40)，Agent 连续放的控件会叠在一起。现在默认放在画布上已有 Block 的右边，DV 断言了依次右移。
- 注意：DV 调用的是 websdk 的 `cli/dist/cli.mjs`，改 `cli/modules` 后要 `npx vite build --config vite.cli.config.ts`。

没在 DV 里覆盖：手机上的选择对话框（底部弹出，没在真实 Zone 或真机上看过）、普通成员插入、日历“显示哪些表…”在真实 Zone 上的操作。

## 8. 第二步：HTML 控件的 `aiws.on('bind')`（2026-11-12，已实施、已部署本机 Zone 做 DV，未提交）

owner 裁决（11-12）：

- 脚本的 bind 处理不依赖页面：Desktop 在不显示的 Worker 里跑 `behavior.js`（`aiws.context.role === 'bind'`）。
- “坚持原则”：CLI 不跑扩展脚本。`buckyos aiws place` 把 Agent 给的绑定当作 ctx 原样使用；必填的没给，就以 `NEEDS_UI` 失败并说出名字。不拿声明里的默认规则替脚本做选择。

落点：

- **声明**：`component.json` 写 `"on_bind": "script"`（可选 `"on_init_control": "script"`），需要 `host_api` 5 和 `manifest_version` 5。
  - 内核：`HTML_API_LATEST` 升到 5，新增 `HTML_BIND_API`、`check_script_events`；`ext.rs` 构建时检查，定义实体直接写入时也查。
  - schema：`component` / `block-def` 的 `on_bind`、`on_init_control` 改为 `oneOf`（步骤列表，或 `"script"`）。
  - websdk 预检 `aiws_ext.ts` 同步，`AIWS_HTML_API_VERSION` 升到 5（新建 HTML 控件的骨架随之是 5）。
- **结果检查**（websdk `aiws_bind.ts` `checkBindResult`，所有 on_bind 都过一遍，不只脚本）：
  - 绑定名要已声明，单张只能一个 id，列表最多 32 个且不重复；
  - `create` 最多 16 项，`kind` 是 standard / table / dir，新建的都要绑上；
  - 每个控件结果里的 `ref` 改名成全局唯一，两个控件都用 `new` 也不会撞；
  - 不合规按 `INVALID` 失败。
  - 顺带的变化：改绑定时列表选成空，结果里不再保留 `[]`，直接省略。写入效果不变，`bindingsPayload` 本来就丢弃空列表。
- **查询**：`DataQuery` 加 `ids`，脚本可以只让对话框列出它挑出的几份。
- **Desktop**：
  - `ui/extensions/bindScript.ts`：Worker 从 blob 加载引导代码，`importScripts` 定义的 `js`。
  - 脚本可用的：`find` / `read` / `canCreate` / `defaults(names?)` / `dialogs.*`，都经消息转给宿主。
  - 超时：加载 8 秒；脚本 20 秒没有动静算超时，等人在对话框里选的时间不算。
  - 失败都是带原因的 `INVALID`：脚本抛错、碰页面、没注册处理函数、返回值不对，界面上显示“插入失败：……”。
  - `binding.ts` 的 `declaredOf` 对 `on_bind: "script"` 的定义给出脚本版 `onBind` / `onInit`，用到时才启动 Worker，结束后关掉。
  - 改绑定（实例属性面板的“更改绑定的数据…”）也由脚本决定（Q1）；组合控件的步骤仍只管插入。
  - `htmlRuntime.ts` 的版本表加 5（页面上没有新成员）。
- **CLI**：`place` 对 `on_bind: "script"` 的定义用 `givenOnly`；组合控件仍跑声明步骤；HTML 控件的 init 检查用默认的。
- **示例**：一起听改为 manifest 5、host API 5、`on_bind: "script"`。
  - 只列出装有音频的目录，只有一个就直接绑，一个都没有就新建空的 `Playlist` 目录。
  - 对话框标题按 `aiws.locale` 选中文或英文；README、AGENT.md、`aiws-manifest.json` 的 host_api 和 digest 同步。
- **文档**：kernel 05 新增“HTML 控件写在脚本里”一段，host API 列表加 5，并写明 Agent 走 `place` 不跑脚本；手册 `aiworkspace_template_dev` 同步。

验证：

- Rust：`cargo test -p aiworkspace-core -p aiworkspace-store -p aiworkspace`，58 个测试程序、289 条全过。`control_examples_and_manifests` 加了一起听的 `on_bind: "script"`，以及三条反例：HTML 写步骤、host_api 4 写 script、manifest 4 用 host_api 5。
- websdk jest：`aiws_bind` / `aiws_ext` / `aiws_schedule` 共 45 条通过（1 条原本就跳过）。新增 ref 不冲突、结果不合规、no_ui 下开对话框三条；HTML script 预检一条。
- Desktop：`tsc`、`eslint` 无报错；WASM 已重建。
- e2e：
  - CE06：一起听从目录插入。没有音频目录时不弹框，新建 Playlist；有两个音频目录和一个照片目录时只列出前两个，标题是脚本给的中文；取消不插入；改绑定时标出当前目录，改选另一个。
  - CE07：脚本抛错、用了页面（`document is not defined`）、没注册处理函数、`init_control` 返回未就绪原因，四种都不插入，并显示原因。
  - control-examples 7 条全过（CE04 一起听播放照常）。

- 回归（串行）：todo / calendar / extensions / blocks / schedule / wish 共 51 条。
  - Q33 原来用 api_version 5 充当“比宿主新的版本”，宿主升到 5 后改用 6，已过。
  - wish T3-4（本地执行器）在串行回归里超时；单独跑两次都过（18 秒对 1.2 分钟），和本次改动无关，是负载下的偶发。
- 本机 Zone DV（`buckyos-build.py -s aiworkspace desktop` + `start.py`，`aiworkspace.controls` 7 条全过）。新增 `[ctl] on_bind script`：
  - Agent 不给 `--bind` 时，`place` 以 `NEEDS_UI` 失败并说出 `playlist`；
  - owner 从目录插入一起听：没有音频目录时新建 Playlist；有两个音频目录和一个照片目录时，对话框只列出前两个；
  - Agent `--bind playlist=talks` 原样绑上，不新建任何数据。
- DV 截图里看到文件夹在对话框里显示表格图标，已改成文件夹图标（`BindDialogs.tsx`），重新部署 Desktop 后 DV 复跑通过。

还没做 / 没验：

- 20 秒无响应的超时没写用例；
- 服务端大纲不给 Cell 输出 `bindings`，只有数据控件和组合控件有。子控件拿父控件的绑定这条路不受影响，因为 HTML 控件不会是父控件；
- 真机、手机没验。

