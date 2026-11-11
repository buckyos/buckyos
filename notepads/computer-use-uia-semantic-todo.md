# ComputerUse 语义观察：Windows UIA 实施 TODO

日期：2026-11-08

状态：设计已 Review（2026-11-08，结论见 §6）；第一阶段已实施（2026-11-08，记录见 §7），§4.6 的 Windows 实机场景验收尚未执行。macOS AX 后端已按同一协议实施并在本机 DV（2026-11-09，记录见 §8），其后 Windows 回归通过（2026-11-11，见 §8.4）。D6 第二阶段的 `toggle`、`select`、`expand`、`collapse` 已在 Windows 实施并实机 DV（2026-11-11，见 §9），macOS 一侧随后实施并在本机 DV（2026-11-11，见 §9.6）。§2 是实施前的源码事实；旧文档里的规划不代表已有实现。

来源与相关设计：

- [buckyos-agent-daemon TODO](buckyos-agent-daemon-todo.md)：§1 第 3 条“辅助功能优先，按平台交付：macOS AX、Windows UI Automation 提供结构化观察和语义操作，截图、坐标输入按需补充；普通表单、菜单、文本操作不要求每一步截图”；§6.1 的工具面与 opaque `element_id`；§6.2 的动态状态；§6.3 的“语义观察足够时不截图”；§7.2 的 Windows UIA 条目。
- [ComputerUse 技术草案](../doc/llm_context/BuckyOS%20Agent-Runtime%20集成ComputerUse%20技术草案.md)：§3.3 的观察模式（visual / semantic / hybrid / auto），§6.1 的语义观察载荷，§6.2 的 `ElementTarget`，§6.3 的有效性规则，§7 的动作语义（click 的 element 目标、`semantic.fill`、`element_state` 等待），§9.3 的图片结果。
- [Agent Daemon](../doc/arch/agent_daemon.md)：当前写明 semantic/hybrid、element target、text/element_state 等待均不支持。

## 1. 目标与范围

Agent 通过控件树观察和操作普通应用：按钮、菜单、标签页、列表、输入框、开关。普通表单、菜单和文本操作不必每一步都截图。截图和坐标输入作为补充，用于自绘界面、canvas，以及需要视觉确认的场景。

语义树计划覆盖 Windows（UIA）、macOS（AX）和浏览器三个平台，三者共用一套通用语义（D13）。

本期实现：

- 通用语义协议：`buckyos-api` 中的共享类型、node_executor、agent_tool 的 `computer` tool，以及 websdk 的类型。
- Windows UIA 后端：位于 `computer_driver`，由 node_daemon 桌面模式和 `buckyos_agent_daemon` 共用。

macOS AX 后端和浏览器语义后端按同一协议另列实施；macOS 已于 2026-11-09 完成（§8），浏览器语义后端实施前如实报告 semantic 不支持。Linux GUI 不在范围内。

验收场景以今天跑通的那类任务为准：在 Chrome 中切到 Gmail，进入垃圾邮件文件夹，选中一封移到垃圾箱，全程以语义观察为主。

## 2. 当前实现事实（2026-11-08）

| 位置 | 事实 | 缺口 |
| --- | --- | --- |
| [computer.rs](../src/kernel/buckyos-api/src/computer.rs) | `ComputerObservationMode` 已有 `Semantic` / `Hybrid` 两个枚举值；`ComputerTarget` 只有 `Point` / `Focus`；`ComputerWaitCondition` 只有 `Delay`；`ComputerObservation.screenshot` 是必填字段，没有放语义载荷的字段 | element 目标、可选截图、语义载荷，以及语义动作和等待 |
| [desktop.rs](../src/kernel/computer_driver/src/desktop.rs) | `native_capabilities` 固定报告 `accessibility: unsupported (semantic_observation_unsupported)`；`validate_mode` 只接受 Visual / Auto，所以 auto 实际总是 visual；观察 TTL 30 秒，最多保留 64 条；有效性检查是 `observation_current`（过期、epoch、revision、surface、焦点），今天已取消人工接管检测 | 能力探测、观察记录中的元素映射、元素目标校验 |
| [native_windows.cpp](../src/kernel/computer_driver/src/native_windows.cpp) | 只有 GDI 截图、`SendInput` 和 LL 钩子，没有 COM / UIA；附带一个原生自测入口（`--input`） | UIA 工作线程、树快照、元素解析与动作接口 |
| [agent_tool computer tool](../src/frame/agent_tool/src/runtime/computer.rs) | 结果 JSON 以文本形式给模型，截图作为图片附在 tool result（§9.3，最多保留 3 张）；工具说明只讲 visual | 说明 semantic，展示语义文本；没有截图时不附图 |
| [node_executor_proto.rs](../src/kernel/buckyos-api/src/node_executor_proto.rs) | 控制帧上限 `EXECUTOR_MAX_CONTROL_FRAME` = 1 MiB；PNG 走 content 通道 | 语义载荷放在控制帧里时需要自己的上限 |
| [websdk executor_client.ts](../../buckyos-websdk/src/executor_client.ts) | 有 `computer_*` 操作类型 | 同步新的 target、观察和命令类型 |

## 3. 设计决策

**D1 观察范围。** semantic 观察返回两部分：

- 当前显示器上的顶层窗口列表，每项含 ref、标题、进程名、pid、bounds、是否前台、是否可访问；
- 一棵窗口的控件子树，默认是前台窗口。

`observe` 增加可选参数 `scope`：

- `{"kind":"foreground"}`，默认；
- `{"kind":"window","observation_id":…,"ref":…}`，取上一次观察窗口列表里的另一个窗口。

不做整桌面全树。observe 本身不改变焦点，是否把窗口带到前台由动作决定。浏览器后端的“窗口”对应标签页。

**D2 节点内容与裁剪。**

- **视图**：Windows 采用 UIA 的 Control View。
- **节点字段**：ref、role、name、可选 id、value（截断）、状态、bounds（截图坐标空间，与 point 目标同一坐标系）、可用动作。role、id、状态和动作都用 D13 的通用名。
- **裁剪规则**：
  - offscreen 节点不返回，只计入 coverage；
  - 无名称、无动作的纯容器节点把子节点上提。
- **默认上限**：`max_nodes` 400，`max_depth` 32，`max_text_bytes` 32 KiB，单次快照 3 秒。首期用固定默认值，不开放给请求调整。超时只返回已遍历的部分，并标记 truncated。
- **coverage**：返回 `visited`、`returned`、`truncated`、`reason`。

**D3 给模型的格式。** 语义载荷用紧凑大纲文本，每行一个节点，只用 D13 的通用 role、状态和动作名，例如：

```text
[e12] tab "Gmail" selected (select)
  [e40] link "垃圾邮件 3" (invoke)
  [e77] button "删除" (invoke)
```

JSON 只保留 `format`（`outline-v1`）、`text`、`windows`、`coverage`，不再重复一份逐节点 JSON，以节省 token。结构化节点和平台原生信息只留在 driver 的观察记录里。

**D4 元素引用。**

- **ref 的形态**：观察内的短 id（e1、e2…），只对该 `observation_id` 有效。
- **driver 侧保存**：观察记录保存 ref → 后端原生身份。Windows 是 RuntimeId、pid、顶层 hwnd、缓存的 UIA 元素和 bounds。
- **动作前重新解析**：元素仍存在、原生身份和进程一致、未 offscreen、bounds 在显示器内。
- **解析失败**：返回 `stale_element`。不按名称或下标找替代节点，也不退化为坐标（草案 §6.2）。

**D5 现有命令的 element 目标。**

- `click(element)`：校验后在元素的可点击点做物理点击，不等同于 `invoke`（草案 §7）。
- `type(element)`、`keypress(element)`：先通过通用 `focus` 聚焦元素，回读焦点确认后再输入。
- `scroll(element)`：在元素中心滚动。

**D6 语义动作命令，分两阶段。**

- 第一阶段：D5，加上 `invoke` 和 `set_value`。
- 第二阶段：看实测效果，再加 `toggle`、`select`、`expand`、`collapse`、`scroll_into_view`。

每个命令只对节点声明了的动作开放，执行后回读状态。命令名和语义都是三个平台通用的，平台映射见 D13。`set_value` 替换整个值，就是草案里的 `semantic.fill`。`invoke` 不移动鼠标，可用于部分被遮挡或不在可视区的元素。

**D7 auto 与 hybrid。**

- **auto**：semantic 可用，且前台窗口树中有可操作节点并达到最小规模时，选 semantic；否则选 visual。`selection_reason` 写明原因，例如 `semantic_available`、`tree_sparse`、`accessibility_unavailable`。
- **hybrid**：同一观察同时返回树和截图，截图仍经 §9.3 的图片结果给模型，坐标和 ref 都可用；返回两者各自的采样时间。
- **动作反馈**：`feedback_mode` 默认 `auto`，不在 binding 上记住“上次用的模式”。草案 §3.3 要求减少跨调用的隐式状态，auto 每次按当前窗口重新选择。

**D8 线程与超时（Windows）。**

- **线程**：native_windows.cpp 新开一个 UIA 工作线程（`CoInitializeEx` MTA），所有 `IUIAutomation` 调用都放在这个线程，不在 Tokio 线程上做 COM 调用。
- **调用方式**：Rust 侧走同步 FFI，在 `desktop.rs` 用阻塞任务包住并设截止时间。
- **防卡死**：用 `IUIAutomation2` 的 ConnectionTimeout / TransactionTimeout，防止 provider 不响应时卡死。
- **结果状态**：超时返回 `backend_timeout`；元素动作超时的执行状态记为 `unknown`。

**D9 Chromium / Electron（经 UIA 观察桌面浏览器）。**

- **无障碍延迟启用**：Chromium 检测到 UIA 客户端后才启用渲染进程的无障碍。第一次观察可能树为空或不完整，需要实测；必要时做一次有界重采样。
- **不改浏览器**：不默认修改浏览器启动参数。
- **Gmail 树很深**：要靠 D2 的裁剪，并实测在 400 个节点内是否能覆盖收件箱列表、左侧文件夹和工具栏。

**D10 权限边界。**

- **高完整性窗口**：普通用户进程受 UIPI 限制，读不到更高完整性的窗口（任务管理器、管理员终端）。这类窗口在列表中标 `inaccessible`，不返回子树。
- **安全桌面与锁屏**：UAC 安全桌面、锁屏按现有 `desktop_unavailable` 处理。
- **不绕过限制**：不使用 UIAccess，也不提权。

**D11 隐私：先能力再安全。** Personal Agent 是用户的全权代理，语义文本不做额外的过滤、脱敏或单独留存策略。授权沿用 `computer_observe`，留存沿用现有 worklog。密码框的 value 按平台实际返回的内容处理，UIA 通常拒绝读取。

**D12 与今天的决定一致。** 语义动作同样不检测人工接管；观察有效性只看 D4 的元素解析和 `observation_current` 的既有规则。

**D13 跨平台通用语义。** Windows UIA、macOS AX 和浏览器三个后端共用一套 role、状态和动作名。协议、大纲文本和命令里只出现通用名，平台原生名只留在 driver 的观察记录里，供诊断使用。

- **role**：参照 WAI-ARIA 角色名，三个平台都能映射：
  - 窗口与结构：`window`、`dialog`、`document`、`group`、`toolbar`、`heading`；
  - 交互控件：`button`、`link`、`tab`、`tablist`、`menu`、`menubar`、`menuitem`、`checkbox`、`radio`、`switch`、`textbox`、`combobox`、`slider`；
  - 列表与表格：`list`、`listitem`、`tree`、`treeitem`、`table`、`row`、`cell`；
  - 其他：`image`、`text`；
  - 映射不到的一律记为 `generic`。
- **状态**：`disabled`、`focused`、`selected`、`checked`、`mixed`、`expanded`、`collapsed`、`readonly`。
- **id**：Windows 是 AutomationId，macOS 是 AXIdentifier，浏览器是 DOM `id`。
- **动作映射**：

| 通用动作 | Windows UIA | macOS AX | 浏览器 |
| --- | --- | --- | --- |
| `focus` | `SetFocus` | 设置 `AXFocused` | `element.focus()` |
| `invoke` | InvokePattern.Invoke | `AXPress` | DOM 激活（`element.click()`） |
| `set_value` | ValuePattern.SetValue | 设置 `AXValue` | 设置 value 并派发 input / change 事件 |
| `toggle` | TogglePattern.Toggle | checkbox 上的 `AXPress` | `element.click()` |
| `select` | SelectionItemPattern.Select | 设置 `AXSelected` | 选中 option / 激活 tab |
| `expand`、`collapse` | ExpandCollapsePattern | 设置 `AXExpanded` / `AXDisclosing` | 激活带 `aria-expanded` 的控件 |
| `scroll_into_view` | ScrollItemPattern.ScrollIntoView | `AXScrollToVisible` | `element.scrollIntoView()` |

映射表随第一个后端（UIA）一起落成代码和单元测试。macOS 和浏览器后端实施时复用这张表，表中的映射需要在实施时实测确认。

## 4. 实施 TODO

### 4.1 共享协议（buckyos-api）

- [x] 定义 D13 的通用 role、状态、动作枚举，以及它们的序列化名；这些名字同时用于大纲文本、命令和 websdk 类型。
- [x] `ComputerTarget` 增加 `Element { observation_id, ref }`，同步 `observation_id()` 和 `validate()`。
- [x] `ComputerObservation.screenshot` 改为 `Option`；新增 `semantic: Option<ComputerSemantic>`，包含 `format`、`text`、`windows`、`coverage`。
- [x] `Observe` 增加可选 `scope`（D1）；上限用固定默认值（D2）。
- [x] 新增 D6 第一阶段的 `invoke`、`set_value` 命令及其校验；归入 `computer_act`，`is_mutating` 为真，只读授权不能调用。
- [x] 语义载荷单独设上限（建议 256 KiB），远低于 1 MiB 的控制帧上限。
- [x] schema 变化通过 schemars 进入 agent tool 的参数说明；补序列化和校验测试。不加旧版本兼容分支。

### 4.2 Windows 原生层（native_windows.cpp）

- [x] 新建 UIA 工作线程与请求队列：初始化 `CUIAutomation8`，设置超时，进程退出时清理。
- [x] 快照接口：给定根（前台窗口或指定 hwnd），用 CacheRequest 批量取属性，遍历 Control View 并按 D2 裁剪，输出节点数组和顶层窗口列表；同时返回 RuntimeId 和缓存元素句柄，供观察记录使用。
- [x] 元素接口：按 RuntimeId 重新解析和校验；取可点击点；`SetFocus` 并回读焦点；执行 Invoke / SetValue 并回读状态。
- [x] bounds 转成截图坐标：复用截图的 DPI 和显示器偏移映射，多显示器只支持主屏（与现状一致）。
- [x] 扩展原生自测：在自测窗口里放按钮、输入框、复选框，检查树中能找到它们，Invoke / SetValue 能生效，超时路径能返回。

### 4.3 Driver（desktop.rs）

- [x] UIA ControlType、pattern、状态到 D13 通用名的映射，以及映射的单元测试。
- [x] 能力探测：UIA 可用时 `accessibility` 报 supported / ready，`observation_modes` 加入 semantic、hybrid；不可用时给出具体原因。
- [x] `validate_mode` 放开 semantic / hybrid；`observe` 生成树（以及 hybrid 下的截图）和大纲文本；观察记录保存 ref 映射；实现 auto 的选择逻辑（D7）。
- [x] `target()` 校验 element 目标（D4），point / focus 的规则不变。
- [x] `act()` 实现 D5 的 element 目标，以及 `invoke` / `set_value`；仍走现有桌面锁、输入串行和 finish 释放路径。
- [x] macOS 后端在 AX 完成前继续报告 semantic 不支持（2026-11-09 已由 AX 后端取代，见 §8）。

### 4.4 node_executor 与 agent_tool

- [x] 动作反馈支持 semantic / hybrid，`feedback_mode` 默认 auto；只有截图时才使用 content 通道。
- [x] `computer` tool 的说明补上 semantic、element 目标、`invoke` / `set_value`；结果给模型展示大纲文本，没有截图时不附图片；补 computer_result_tests。
- [x] 验证上下文成本：一棵 400 节点的树大约多少 token，与一张截图对比。

### 4.5 websdk 与文档手册

- [x] 同步 websdk `executor_client.ts` 的类型（含 D13 的通用名）；同一变更内检查 websdk CLI 的 computer 相关代码。
- [x] 文档：
  - 在草案 §3.3 / §6 / §7 写入 D13 的通用语义和实现状态；
  - 更新 [agent_daemon.md](../doc/arch/agent_daemon.md) 中写“语义树、semantic、element target 不支持”的三处（开头的现状说明，以及 visual/auto 和 Windows 两段）；
  - 更新 [node_exector.md](../doc/arch/node_exector.md) 的 ComputerUse 段落。
- [x] 手册：`xllm.md`（computer tool 用法）、`buckyos_admin.md`（平台能力）、`opendan_agent_dev.md`（实现入口），按 AGENTS.md 的 cmd_skills 联动表检查。

### 4.6 测试与验收

- [x] 单元测试：协议校验、通用名映射、大纲生成与裁剪、ref 失效、auto 选择。
- [x] 原生自测：见 4.2。
- [ ] Windows x86_64 实机 DV（2026-11-08 执行，结果见 §7.5；未完成：Gmail 移到垃圾箱、RDP 会话、150% DPI）：

| 场景 | 判定 |
| --- | --- |
| Notepad 输入与菜单保存 | 全程语义观察；`type(element)` 与菜单 `invoke` 生效；回读文本一致 |
| 设置页开关 | `click(element)` 后，回读的 `checked` 状态改变 |
| 文件资源管理器选中、重命名 | `click(element)` 选中；进入重命名后 `set_value` 生效 |
| Chrome + Gmail：切换标签、进入垃圾邮件、移一封到垃圾箱 | 以语义观察为主；记录截图次数、token 和是否需要 hybrid |
| 高完整性窗口（任务管理器） | 标 `inaccessible`，不崩溃、不卡死 |
| 不响应的窗口 | 3 秒内返回 truncated 或 timeout |
| RDP 会话与控制台会话、150% DPI | bounds 与截图坐标一致 |

- [ ] 指标：每个任务的截图次数、模型 token、成功率，分别与纯 visual 对比（§7.5 只记录了语义一侧的截图次数与大纲体量，纯 visual 对照未做）。

## 5. 后续

- D6 第二阶段命令：`toggle`、`select`、`expand`、`collapse` 见 §9；`scroll_into_view` 推迟，要先定 offscreen 节点的返回策略（§9.1）。
- 浏览器语义后端：按 D13 实施，另列 TODO。
- `wait_for` 的 `element_state` / `text` 条件：依赖 D4 的元素跟踪。
- move / drag、多显示器、Linux AT-SPI、UIAccess。

## 6. Review 记录（2026-11-08）

- D1–D5、D7–D10、D12：按建议执行。
- D7 的动作反馈默认值：Review 时写了“按你的建议”，但原文给了两个选项。现定为默认 `auto`，不记住上一次的模式，理由是草案 §3.3 要求减少跨调用的隐式状态。
- D6：同意两阶段。补充要求：语义树支持 Windows、macOS、浏览器三个平台，这些语义最好是通用的。据此新增 D13。
- D11：隐私问题不必在意，先能力再安全。Personal Agent 基本是用户的全权代理。

## 7. 实施记录（2026-11-08）

### 7.1 改动入口

| 位置 | 内容 |
| --- | --- |
| [computer.rs](../src/kernel/buckyos-api/src/computer.rs) | D13 枚举 `ComputerRole` / `ComputerElementState` / `ComputerElementAction`（`as_str`、`ALL`）；`ComputerTarget::Element { observation_id, ref }`；`Observe.scope`（`ComputerObserveScope`）；`Invoke`、`SetValue` 命令；`ComputerObservation.screenshot` 改为可选，新增 `semantic`（`format`、`text`、`windows`、`coverage`、`captured_at_ms`）；`ComputerScreenshot.captured_at_ms`；固定上限常量（载荷 256 KiB、400 节点、深度 32、文本 32 KiB、3 秒、窗口 32）；动作 `feedback_mode` 缺省为 auto |
| [native_windows.cpp](../src/kernel/computer_driver/src/native_windows.cpp) | UIA 工作线程（MTA、`CUIAutomation8`，ConnectionTimeout 1 s / TransactionTimeout 1.5 s，`atexit` 清理）与请求队列；窗口列表；广度优先快照（Element / Children 两个 CacheRequest、Control View）；元素解析、`SetFocus`、Invoke、SetValue 与回读；`--uia` 自测。`build.rs` 增加 `oleaut32`、`dwmapi` |
| [semantic.rs](../src/kernel/computer_driver/src/semantic.rs)（新增） | 平台无关的裁剪、预算与 `outline-v1` 渲染；UIA ControlType / AriaRole / pattern / 状态到 D13 名的映射及单元测试 |
| [desktop.rs](../src/kernel/computer_driver/src/desktop.rs) | 能力探测；observe 的 scope、auto/hybrid 选择与 Chromium 重采样；观察记录中的 ref 映射；element 目标解析；element 目标的 click/type/keypress/scroll 与 `invoke` / `set_value`；动作反馈的窗口 scope |
| [ops.rs](../src/kernel/node_executor/src/ops.rs) | 截图可选；语义载荷超限则 feedback 失败；只有截图写 content 通道 |
| [agent_tool computer.rs](../src/frame/agent_tool/src/runtime/computer.rs) | 工具说明；大纲移出 JSON，作为纯文本行附在结果 JSON 之后；无截图不附图 |
| [executor_client.ts](../../buckyos-websdk/src/executor_client.ts) | Computer 请求、目标、观察类型与 D13 名常量；`ExecutorCommandStream.computer()`。websdk CLI 没有 computer 相关代码，无需修改 |
| 文档与手册 | 草案 §3.3 / §6 / §7、[agent_daemon.md](../doc/arch/agent_daemon.md)、[node_exector.md](../doc/arch/node_exector.md)；`xllm.md`、`buckyos_admin.md`、`opendan_agent_dev.md` |

### 7.2 实施中定下的细节

- **大纲行**：`[ref] role "name" #id value="…" 状态… (动作, …) @x,y wxh`。name 最多 160 字符；value 只对 textbox / combobox / slider 显示，最多 120 字符，与 name 相同时省略；id 只显示不超过 64 字节、无空白的可打印 ASCII。
- **裁剪**：在 D2 两条规则之外，重复最近保留祖先名称的 text / image 标签也省略（Chrome 的链接、按钮各带一个同名 text 子节点）。`scroll_into_view` 几乎每个 Chromium 元素都有，大纲不打印，裁剪判断也不算它；记录中保留。预算按广度优先计，输出按树序；`reason` 取 `max_nodes`、`max_depth`、`max_text_bytes`、`timeout`、`max_visited`（原生层最多访问 3000 个节点）、`backend_timeout`、`window_inaccessible`。
- **原生遍历**：offscreen 节点只计数、不返回；有几何的不再展开，没有几何的（WinUI 3 弹出菜单外层的 `Popup`）继续展开（DV 中发现，见 §7.5）；同一 RuntimeId 只收一次；Text、Image、Separator、Thumb、ScrollBar、ProgressBar 不展开子节点。
- **前台 scope 含弹出窗口**：Win32 菜单（`#32768`）和被目标窗口拥有的弹出窗口是独立的顶层窗口，与目标窗口作为并列根一起遍历，否则 Notepad 的菜单项不在树里。
- **窗口列表**：可见、未 cloaked、未最小化、与主屏相交、有标题（前台窗口除外）、非 tool window，最多 32 个；`accessible` 为同会话、同用户且完整性不高于本进程。
- **auto 阈值**：返回节点 ≥ 10 且可操作节点 ≥ 5 才选 semantic。
- **Chromium 重采样（D9）**：实测 Chrome 首次被 UIA 查询时树里只有浏览器外壳，没有网页 `document`，且外壳本身已足够“丰富”，单看稀疏度不会重采样。现规则：根窗口类名为 `Chrome_WidgetWin*` 且树中没有 `document` 时，等待 500 ms 重采样一次。
- **element 解析**：click / scroll 要求元素不在 offscreen、点在主屏内、该点处的顶层窗口仍是元素所在窗口（否则 `element_obscured`）；invoke / set_value / focus 按 D6 不要求在可视区，但要求未禁用且 pattern 仍可用；set_value 另拒绝只读值。
- **type / keypress(element)**：`SetFocus` 后最多回读 10 次（间隔 20 ms）`HasKeyboardFocus`，再重新 probe，以新的焦点作为后续输入的焦点保护；聚焦之后的失败记为 `partial`。
- **set_value 回读**：回读值与请求不一致时返回 `value_mismatch`，执行状态 `completed`。
- **超时**：UIA 任务已开始而调用方超时，执行状态为 `unknown`，node_executor 按既有规则暂停 Driver；任务尚未开始就超时则丢弃，返回 `backend_timeout` / `not_started`。
- **反馈 scope**：element 动作的语义反馈观察元素所在窗口，窗口已不存在时回到前台窗口。
- **元素句柄生命周期**：观察记录持有原生快照；生成大纲后立即释放未进入大纲的元素，记录被清除或过期时释放其余元素。

### 7.3 验证结果

- 单元测试：`cargo test -p buckyos-api --lib computer`（协议 7 项）、`cargo test -p computer_driver`（25 通过，2 项 ignored 的桌面 smoke；只读 smoke 当时名为 `windows_semantic_smoke`，2026-11-09 起改为跨平台的 `semantic_smoke`）、`cargo test -p node_executor --lib --test computer`（6 + 11，含语义载荷不走 content、超限 feedback 失败）、`cargo test -p agent_tool --lib runtime::computer`（11，含大纲纯文本、无图）；websdk `jest tests/executor_client.test.ts`（5）与 `tsc -p tsconfig.build.json`。
- 原生自测：MSVC `/W4 /WX` 编译通过。默认 probe 与截图通过；`--uia` 通过：快照 15 ms，找到按钮、输入框、复选框，Invoke 触发按钮，SetValue 写入中文，控件销毁后返回 stale，挂起窗口约 1 秒返回 `backend_timeout`。
- 只读 smoke（`windows_semantic_smoke`，本机 Windows 11、5120×1440 主屏、前台 Chrome）：首次观察 47 节点、3.5 KB 大纲、29 ms，没有网页 `document`；Chrome 启用网页无障碍后，同一窗口 149 节点、10.8 KB 大纲（JSON 载荷 12.6 KB）、约 38 ms，含 `document`。
- 上下文成本（4.4，估算）：按仓库 `ByteHeuristicTokenizer`（4 字节 ≈ 1 token），149 节点大纲约 2.7k token，400 节点 / 32 KiB 上限约 8.2k token。截图成本取决于 provider 的缩放；按 Anthropic 公开的公式（缩放到长边 1568 px、约 1.15 MP 以内后 宽×高/750）估算，本机 5120×1440 截图约 0.9k token、1920×1080 约 1.5k token。即富页面上单次语义观察并不比截图省 token，收益在于精确目标、无需逐步截图核对；每个任务的实际对比要等 DV。
- `cargo check --workspace --tests --keep-going` 只剩三处与本改动无关的既有错误（文件未改动）：`control_panel` 与 `opendan/tests/loader.rs` 测试里的 Unix 专用代码在 Windows 上不能编译，`node_executor/tests/windows.rs` 缺少 `expected_size`。

### 7.4 未完成与风险

- §4.6 中 Gmail 移到垃圾箱、RDP 会话与 150% DPI 未执行，原因见 §7.5。
- 浏览器语义后端未实施，继续报告 semantic 不支持（macOS AX 见 §8）。
- D6 第二阶段命令与 `wait_for` 的 `element_state` / `text` 未实施。
- 有的 provider 在 Invoke 打开模态对话框时会阻塞直到对话框关闭，调用会超时并记为 `unknown`，随后 Driver 被暂停、需本地 resume。DV 中需要关注其频率。

### 7.5 实机 DV（2026-11-08，Windows 11，5120×1440 主屏）

方法：新增 `src/kernel/computer_driver/examples/computer_dv.rs`，在本机以真实 NodeExecutor（账本、content 通道、语义上限）加真实 Driver 运行，经 `127.0.0.1:<port>` 每次接收一个 Computer 请求，返回与 agent `computer` 工具给模型的相同文本（结果 JSON 加纯文本大纲）。由 Claude 充当 Agent：每一步先观察、读大纲选 ref，再动作。启动：`cargo run -p computer_driver --example computer_dv -- 47800 <输出目录>`。

| 场景 | 结果 |
| --- | --- |
| Notepad 输入与菜单保存 | 通过。auto 全程选 semantic（34–53 节点，约 3 KB 大纲），只在保存对话框关闭的瞬间回落 visual（`tree_sparse`）。`type(element)` 写入编辑区；File 菜单 `invoke` 后反馈树列出 13 个菜单项（修复见下）；“Save as” `invoke`、文件名框 `set_value`（回读一致）、Save `invoke`；保存的文件与编辑器文本一致。另验证了复用旧 observation 被拒绝（`stale_observation`）。异常：第一次 `type` 把 “UIA” 写成 “IIA”，同样请求再试两次均正确，Notepad 的自动更正处于开启状态，怀疑与其有关 |
| 设置页开关 | 以 Notepad 设置页的 ToggleSwitch 代替系统设置页（不修改系统设置）。映射为 `switch`；`click(element)` 两次，回读 checked → 未选中 → checked，已复原。顺带验证：`scope` 观察后台窗口可用；元素被其他窗口遮挡时 `click(element)` 返回 `element_obscured` / `not_started` |
| 资源管理器选中与重命名 | 通过（91–97 节点）。`click(element)` 选中（`selected`），`keypress(element)` F2 出现重命名框，`set_value` 回读一致，Enter 后磁盘上文件已改名。异常：窗口刚打开时的第一次 click 没有选中，第二次正常 |
| Chrome + Gmail | 部分完成。网页树 299 节点（`max_text_bytes` 截断，访问 538 个），覆盖左侧文件夹、工具栏和 26 行邮件。`click(element)` 切到 Gmail 标签页（第一次点击丢失，第二次生效）；选中一封垃圾邮件（全选框变为 `mixed`，工具栏出现 Delete forever / Not spam / Move to）；打开 Move to 菜单并用 Escape 关闭。未执行“移到垃圾箱”：Spam 视图只有永久删除（Delete forever）和 Not spam，Move to 列表里没有系统 Trash（`[Imap]/Trash` 是用户标签）。同时发现本机 Jarvis 的工作会话 work-20261108-10 正在执行同一个“删除一封垃圾邮件”任务（node_daemon、opendan 在运行，Chrome 标签页被切回 test.buckyos.io），为避免两个 Agent 争用桌面而停止 DV。该邮件仍处于选中状态，未做任何移动或删除 |
| 高完整性窗口 | 通过。没有另开需要提权的任务管理器（普通完整性的进程关不掉它），改用本机已在运行的高完整性窗口（MotionAssistant.exe，窗口列表中 `accessible:false`），经 `scope` 观察：semantic 0.05 s 返回空树、`reason: window_inaccessible`；auto 回落 visual，`selection_reason: window_inaccessible`；hybrid 返回截图加空树。不崩溃、不卡住 |
| 不响应窗口 | 通过。用一个不再处理消息的测试窗口（不抢焦点）经 `scope` 观察：semantic 约 1.1 s 返回 `truncated: true`、`reason: backend_timeout`。auto 回落 visual 时截图超过 1 MiB，报 `capture_size_limit`（见下） |
| bounds 与截图坐标 | 100%（96 DPI）下通过：hybrid 观察中任取 6 个元素按大纲 bounds 从截图裁剪，均与控件像素对齐。150% DPI 需要修改系统缩放、RDP 需要远程会话，均未执行 |

第二轮（同日 22:30 起）开始前核对了 node_daemon 执行器账本：Jarvis 最后一次桌面操作在 20:26，此后到 DV 结束都没有新操作。

截图次数：Notepad、设置开关、资源管理器三个场景完成路径上 0 次；Gmail 中 3 次，均用于排查点击丢失和意外窗口。大纲体量：Notepad 约 3 KB（约 0.8k token），资源管理器约 7 KB，Chrome 外壳约 11 KB，Gmail 32 KB 上限（约 8k token）。

DV 中发现并已修复：

- **WinUI 3 弹出菜单缺失**：菜单链路为 `PopupHost → pane → Window "Popup"（IsOffscreen=true、空矩形）→ Menu → 菜单项`，原实现不展开 offscreen 节点，整个菜单丢失。现在 offscreen 且没有几何的节点视为逻辑容器：不返回，但继续展开。
- **弹出窗口重复**：`PopupHost` 既是窗口树的子节点，又作为被拥有的弹出窗口根出现两次。现按 RuntimeId 去重。
- **不可见格式字符**：垃圾邮件标题夹满零宽字符（U+200B–U+200D、U+FEFF），资源管理器日期带 bidi 标记，白占字节和 token。大纲与窗口标题现去掉这类格式字符（不动 emoji 变体选择符），有单元测试。
- **harness 本身**：Executor 空闲连接会被关闭、授权快照最长一个租约，harness 改为每个请求重新登录并刷新配置。

仍待处理：

- **点击偶尔不生效（未定位）**：第一轮约 8 次 `click(element)` 中有 2 次没有生效。第二轮受控复现：指针先停在其他窗口再点击资源管理器 6/6 生效；先切走再用程序把资源管理器带到前台后点击 4/4 生效；同样方式点击 Chrome 标签页 4 次丢 1 次，随后不切窗口的连续点击中又丢 1 次。丢点击的时段里有人在用鼠标（前台被切到 VS Code、指针停在远离最后点击点的位置、最近一次输入晚于驱动的最后一次点击），Driver 按设计不检测人工接管，因此无法区分是 Driver 问题还是人工输入干扰，未改代码。需要在无人操作的桌面上，专门对 Chrome 标签栏重复点击来定位。
- **截图实际上限是 1 MiB**：scheduler 与 control_panel 给 Agent profile 用的是 `ExecutorLimits::default()`，其中 `max_file_bytes` 为 1 MiB，ops.rs 取它和 16 MiB 的较小值。本机 5120×1440 桌面内容稍多时 PNG 就超过 1 MiB，visual 观察（包括 auto 的回落）报 `capture_size_limit`。这与本次语义改动无关，但会让大屏上的 visual 模式不可靠；语义模式不受影响。可考虑为 Computer 截图单独设上限，而不是借用文件上限。
- **动作反馈时机**：反馈在动作后 60 ms 观察，`invoke` 打开的“另存为”对话框没赶上，下一次 observe 才看到。等第二阶段的 `wait_for element_state` 或增加有界的稳定等待。
- **Gmail 行的体量**：每行展开约 10 个 cell，且不少 cell 名称重复行名，26 行就用满 32 KiB。可考虑省略与所在行同名的 cell。
- **不属于本任务、但在 DV 中看到的问题**：Jarvis 在 Windows 原生 runtime 中执行无扩展名的 `agent-session` 时，系统用 Notepad 打开它并报 “Cannot find …\.runtime\bin\agent-session file”；Jarvis 的工作日志里还有 `local computer IPC: the executor has no valid authorization snapshot`（node_daemon 的执行器授权快照过期）。

## 8. macOS AX 实施记录（2026-11-09）

协议、通用名、大纲与 Driver 流程与 Windows 共用；本节只记录 macOS 后端及为共用而做的调整。

### 8.1 改动入口

| 位置 | 内容 |
| --- | --- |
| [native.m](../src/kernel/computer_driver/src/native.m) | AX 后端：`bucky_semantic_status/snapshot/element/retain/release`；窗口列表、根选择、广度优先快照、元素解析与 focus / invoke / set_value；按快照保存 AXUIElement。probe 的 `focus_on_display` 改为焦点元素被其窗口裁剪后的可见部分在主屏内（见 §8.2） |
| [native_windows.cpp](../src/kernel/computer_driver/src/native_windows.cpp) | 五个入口由 `bucky_uia_*` 改名为平台中立的 `bucky_semantic_*`，行为不变 |
| [semantic.rs](../src/kernel/computer_driver/src/semantic.rs) | pattern / state 位与 `states()`、`actions()` 移出 `uia`，两个后端共用；`RawWindow` / `RawSnapshot` / `RawNode`，`Snapshot` 按平台取别名；新增 `ax` 模块（AX role / subrole 到 D13 名的映射）及单元测试 |
| [desktop.rs](../src/kernel/computer_driver/src/desktop.rs) | `native_uia` 改为两平台共用的 `native_semantic`；macOS 报告 `accessibility`（未授权时原因为 `accessibility_permission_required`）；`build_tree`、`semantic_tree`、`resolve`、`element_op` 去掉 Windows 条件编译；只读 smoke 改名 `semantic_smoke`，两平台共用 |
| [ax_macos.m](../test/agent_daemon/ax_macos.m)（新增） | 原生自测：子进程显示测试窗口，父进程只经 AX 观察和操作；[capture_macos.m](../test/agent_daemon/capture_macos.m) 修正 `computer_driver` 拆分后失效的 include 路径 |
| 文档与手册 | [agent_daemon.md](../doc/arch/agent_daemon.md)、[node_exector.md](../doc/arch/node_exector.md)、草案 §1 / §3.3 / §6 / §7、`buckyos_admin.md`、`opendan_agent_dev.md`、[buckyos-agent-daemon TODO](buckyos-agent-daemon-todo.md) §7.1 |

共享协议（`buckyos-api`）、node_executor、agent_tool 与 websdk 没有改动：macOS 只是让同一协议在另一个平台可用。`xllm.md` 中 computer tool 的用法与平台无关，无需修改。

### 8.2 实施中定下的细节

- **线程与超时**：AX 是对目标进程的同步 IPC，可在任意线程调用，因此没有像 UIA 那样设工作线程，Rust 侧仍在阻塞任务里同步调用。每个元素设 messaging timeout（不超过 1 s，且不超过快照剩余预算），快照整体 3 s；某次调用返回 `kAXErrorCannotComplete` 即视为目标进程无响应，停止遍历并报 `backend_timeout`。
- **窗口列表**：`CGWindowListCopyWindowInfo` 取主屏上 on-screen、layer 0 的窗口，按前后次序，最多 32 个；标题优先用 `kCGWindowName`，拿不到（无屏幕录制权限）时用 AX 窗口标题，所以语义观察只需要辅助功能权限。`process` 为 owner 名，`accessible` 为窗口属主 uid 与当前用户相同，`foreground` 为 AX 焦点应用的焦点窗口。窗口身份是 CGWindowID，用 `_AXUIElementGetWindow` 对应到 AX 窗口，失败时按 pid 与窗口 frame 匹配。
- **根**：scope 指定的窗口；默认取 AX 焦点应用的 `AXFocusedWindow`，依次退到 `AXMainWindow`、第一个窗口。对前台应用另加两类根：应用下打开的上下文菜单（`AXMenu`）和菜单栏（菜单栏是 macOS 的主要命令入口；未展开的子菜单没有几何，不展开）。遍历结束后，若应用的焦点元素不在树中（Finder 改名时的 field editor 直接挂在 `AXApplication` 下），把它作为一个额外的根。
- **offscreen**：AX 没有 IsOffscreen。以主屏为初始可见区，经过 `AXScrollArea`、`AXWebArea` 时与其 frame 求交，进入 `AXMenu` 时重置为主屏（菜单是独立窗口）。可见部分为空且有几何的节点不返回也不展开；没有几何的节点视为逻辑容器，不返回但继续展开（`AXMenu` 与叶子角色除外）。`AXColumn` 不返回（其单元格已在行下）；同一元素按 `CFEqual` 只收一次。
- **pattern / state 位**（与 UIA 同一布局；2026-11-11 起 toggle / select / expand 的声明改为要求能执行，见 §9.6）：
  - `invoke`：动作列表含 `AXPress`；
  - `set_value`：文本类角色的 `AXValue` 可写，否则标 `readonly`；
  - `focus`：文本、列表、表格、大纲、网页区域和滑块上 `AXFocused` 可写；
  - `toggle` 与 `checked` / `mixed`：`AXCheckBox` 的 `AXValue` 为 1 / 2，菜单项看 `AXMenuItemMarkChar`；
  - `select`：只给行、单选、标签页；`AXRadioButton`（含标签页）`AXValue` 为 1 时 `selected`；
  - `expand` / `collapse`：只给 combobox、popup、菜单按钮和行（`AXExpanded`），以及 `AXDisclosureTriangle`。
  Chromium 在每个节点上都报 `AXSelected` / `AXExpanded`（值为 false），不限定角色时 Gmail 等网页的每个 group 都会带 `select` 和 `expand`，无法裁剪；其他角色只在值为 true 时显示 `selected` / `expanded`。`focused` 不读各元素的 `AXFocused`（SwiftUI 列表在每个单元格上都报 true），改为与应用的 `AXFocusedUIElement` 比较。
- **名称与 id**：依次取 `AXTitle`、`AXDescription`、静态文本的值、`AXTitleUIElement` 的值、文本框的 placeholder；带 subrole 的可 invoke 控件或文本框再取 `AXRoleDescription`（窗口的关闭 / 最小化 / 全屏按钮、搜索框），最后是可操作元素的 `AXHelp`。id 取 `AXIdentifier`（AppKit 自动生成的 `_NS:` 除外），否则取网页的 `AXDOMIdentifier`。value 只取文本类、combobox、popup 和滑块。
- **映射**（Rust `semantic::ax`）：
  - 角色：`AXSheet`、`AXPopover`、对话框 subrole 映射为 `dialog`；`AXWebArea` 为 `document`；`AXPopUpButton` 为 `combobox`；`AXRadioButton` + `AXTabButton` 为 `tab`；`AXCheckBox` + `AXSwitch` 为 `switch`，+ `AXToggle` 为 `button`；`AXRow` + `AXOutlineRow` 为 `treeitem`；`AXMenuBarItem` 为 `menuitem`。其余见代码与单元测试，原生侧的计数容器表与之由测试对齐。
  - 裁剪：无名称、无动作的菜单项（分隔线；原生侧对无名称且禁用的菜单项也不报 `AXPress`）记为 `generic`；无名称、无状态、无动作的单元格记为 `group`，与其他纯容器一起被裁剪。
- **Chromium / Electron（D9）**：快照时对目标应用设置 `AXManualAccessibility = true`，成功即视为 Chromium 系（`web`）；树中没有 `document` 时由 Driver 等 500 ms 重采样一次（与 Windows 同一逻辑）。这是运行时的 AX 属性，不改浏览器启动参数。Chrome 的网页节点不在动作列表里声明 `AXPress`（链接和按钮只有 `AXShowMenu`、`AXScrollToVisible`），因此大纲中网页链接和按钮没有 `invoke`，用 `click(element)`；Chrome 自身的标签页、工具栏和 Electron 的网页按钮都声明 `AXPress`。
- **元素解析**：重新读取属性，`kAXErrorInvalidUIElement` 或角色变化即 `stale_element`。可见部分沿祖先求得（scroll area、web area，到菜单为止），点击点是其在主屏内部分的中心；用系统范围的 AX 命中测试判断遮挡，命中其他进程或同进程的其他窗口即 `element_obscured`。
- **focus**：目标应用不在前台时先设置 `AXFrontmost`，再对其窗口 `AXRaise` 并设 `AXMain`，然后设 `AXFocused`，回读系统焦点元素等于目标或其后代（最多 10 次、每次间隔 20 ms；激活过应用时 30 次）。
- **invoke**：`AXPress` 放在后台队列执行。0.5 s 内没有返回时，再向该应用发一个请求（读 `AXFocusedUIElement`）：若有应答，说明按下已在嵌套循环（模态对话框、菜单跟踪）中执行，按已送达处理，返回 `pending`、执行状态 `completed`，不记为 `unknown`，也不会因此暂停 Driver。若该请求也无应答，就等到超时，返回 `backend_timeout` / `unknown`。
- **set_value**：设置 `AXValue` 后回读，与 Windows 一样不一致时报 `value_mismatch`。
- **probe**：长文档的 `AXTextArea` frame 是整篇文档，原先“焦点元素整体在主屏内”的判断会让 `type(element)` 与 focus 目标返回 `focus_unverifiable`。现改为焦点元素被其窗口裁剪后的部分非空且在主屏内，与 Windows 按焦点 HWND（只含可见控件）判断的含义一致。
- **对本进程的 AX 请求**：AX 请求若发给调用者自己的进程，会在调用线程上直接处理；在后台线程设置自己窗口里文本框的值会与主线程布局死锁。因此原生自测把测试窗口放在子进程里。Driver 不会以自身为目标（Runtime.app 不显示窗口）。

### 8.3 验证结果

- 单元测试：`cargo test -p computer_driver`（20 通过，1 项 ignored 的 `semantic_smoke`，含新增的 AX 角色映射与节点映射）、`cargo test -p buckyos_agent_daemon`（通过）；`cargo check -p node_daemon -p buckyos_agent_daemon` 通过（node_daemon 有一条与本改动无关的既有 dead code 警告）。`native.m` 按 build.rs 的 `-Wall -Werror` 编译。
- 原生自测 `ax_macos.m`：17 项通过。快照 12 节点约 88 ms；invoke 计数、Unicode `set_value` 回读、复选框 invoke 后树中为 `checked`；focus 激活窗口并聚焦文本框；打开模态面板的按钮 invoke 约 511 ms 返回 `pending`；控件移除后同一 ref 为 stale、快照释放后为 stale；主线程挂起的应用约 1.26 s 返回 `backend_timeout`。
- 只读 smoke（`semantic_smoke`，前台或 `BUCKY_SEMANTIC_TITLE`）：

| 窗口 | 访问 / 返回 | 大纲 | 耗时 |
| --- | --- | --- | --- |
| 系统设置（辅助功能页） | 181 / 123 | 7.2 KB | 35–52 ms |
| Chrome 知乎问答页（后台窗口） | 578 / 137 | 9.8 KB | 52–91 ms |
| Claude 桌面应用（Electron） | 1357 / 337 | 32 KiB 上限，`max_text_bytes` | 113–130 ms |

  Chrome 页面在按角色限定 `select` / `expand` 之前为 324 行、29.7 KB。两个 Chromium 系应用在首次采样时就已有网页 `document`，本次没有触发重采样。

- 实机 DV：`computer_dv` harness（真实 NodeExecutor 加 Driver），macOS 15.8.1、Apple silicon，主屏 1620×1080、1× 缩放，display id 3（harness 默认 binding 为 `display:1`，请求需显式给出 `binding_id`）。Accessibility 与屏幕录制授予 Claude Code 的进程身份。期间确认本机 Jarvis 自执行器打开后没有桌面操作。

| 场景 | 结果 |
| --- | --- |
| TextEdit 输入与菜单保存 | 通过。auto 选 semantic（15 行）。`keypress(element)` Cmd+↓ 后 `type(element)` 写入中文；菜单栏 File `invoke` 0.41 s，反馈树列出菜单项（未展开的 Open Recent 不展开）；Save `invoke` 后磁盘内容与编辑器一致；`set_value` 替换整个文本并回读一致 |
| 设置复选框 | 经菜单 TextEdit › Settings… 打开设置窗口：标签页、单选为 `selected`，复选框为 `checked`，文本框以标签命名。`click(element)` 两次切换 “Smart links”，回读 checked → 未选中，已复原 |
| 后台窗口与遮挡 | `scope` 观察被设置窗口部分遮住的 note.txt 窗口；对被遮住的文本区 `click(element)` 返回 `element_obscured` / `not_started`；`invoke` 设置窗口的关闭按钮；Chrome 在前台时对 TextEdit 窗口 `type(element)`，先激活 TextEdit 再输入 |
| 长文档输入 | 300 行文档的 `type(element)` 起初返回 `focus_unverifiable`（执行状态 `partial`）；改 probe 后通过 |
| Finder 选中与改名 | 列表视图（行、单元格、可写文件名框）。`click(element)` 选中行，`keypress(element)` Enter 进入改名；改名框起初不在树中，加入“孤立焦点元素作为根”后可见；`set_value` 写入 “gamma 改名.txt”，Enter 后磁盘上已改名 |
| Chrome 标签页 | 对 Gmail 标签页 `invoke`，0.27 s 切换且不移动鼠标，反馈树含 Gmail 的标签列表与 Archive / Report spam / Delete / Move to 工具栏；再 `click(element)` 原标签页复原 |
| hybrid 与坐标 | Chrome 窗口 hybrid 观察，按大纲 bounds 从截图裁剪 5 个元素（刷新按钮、书签星标、标签页、知乎链接、新标签页），均与控件像素对齐；树与截图分别带采样时间（相差 154 ms） |
| 菜单分隔线 | 原生侧不再对无名称、禁用的菜单项报 `AXPress` 后，File 菜单从 38 行降到 34 行，分隔线不再出现 |

截图次数：除 hybrid 场景外为 0；另有一次 visual 观察用于查看意外出现的系统对话框（DV 中误用 osascript 触发的自动化授权提示，由用户关闭，与 Driver 无关）。

### 8.4 未完成与风险

- **Windows 回归（2026-11-11 已执行，通过，未改代码）**：本次在 macOS 上实施，`native_windows.cpp` 的入口改名与 Rust 侧 `native_semantic` 的共用随后在 Windows 11（5120×1440 主屏）上重新编译和测试：
  - `cargo test -p computer_driver`：27 通过，2 项 ignored，无警告。
  - `--ignored` 的 `semantic_smoke`、`windows_desktop_capture_smoke` 通过。`semantic_smoke` 的前台为 VS Code（Electron），走了 Chromium 重采样：首次 6 节点（`tree_sparse`），重采样后 371 节点、`max_text_bytes` 截断，约 78 ms。
  - MSVC `/W4 /WX /DBUCKY_DESKTOP_SELF_TEST` 自测编译通过。`--uia` 通过：快照 32 ms，三类控件均在树中，Invoke、SetValue 生效，控件销毁后 stale，挂起窗口 1000 ms 返回 `backend_timeout`。
  - 自测中全屏 PNG 为 1,086,950 字节，超过 1 MiB，§7.5 的截图上限问题仍在。
- **未执行的场景**：Retina 或缩放显示（本机为 1×，bounds 换算只有单元测试覆盖）、多显示器、锁屏、其他用户的窗口（`accessible:false`）、真实应用上 auto 回落 visual、Chromium 的重采样路径、真实应用中打开模态对话框的 invoke（只有自测覆盖）、Gmail 移到垃圾箱（涉及用户邮件，未执行）。
- **私有 API**：`_AXUIElementGetWindow` 是私有但长期稳定的符号，SDK 中有导出；不可用时按 frame 匹配，同一应用两个 frame 完全相同的窗口可能对应错。
- **Chromium 无障碍开销**：`AXManualAccessibility` 打开后，在该应用进程的生命周期内都保持开启，带来渲染进程的额外开销，与 Windows 上 UIA 客户端出现时的行为相同。
- **Chrome 网页没有 invoke**：网页链接和按钮只能 `click(element)`，因此需要可见且未被遮挡。
- **隐藏的备选菜单项**：AppKit 会暴露按住 Option 才显示的备选项（如 Close / Close All 位置相同），大纲中两者都会出现。
- **pending 判断依赖应用的 AX 实现**：应用若在主线程以外处理 AX 请求，嵌套循环判断可能把尚未执行的按下当成已送达；本次测试的 AppKit、Chrome、Electron 都在主线程处理 AX 请求。

## 9. D6 第二阶段命令（2026-11-11）

### 9.1 范围与决定

- **本期命令**：`toggle`、`select`、`expand`、`collapse`。
  - 形态与 `invoke` 相同：只接受 element 目标，字段为 `binding_id`、`target`、`feedback`、`feedback_mode`、`execution_timeout_ms`，归入 `computer_act`。
  - 命令名就是大纲里的动作名，只对观察中声明了该动作的元素开放。`expand` 只在元素折叠或部分展开时声明，`collapse` 只在展开或部分展开时声明。
  - 执行前重新解析，要求元素未禁用、pattern 仍可用；与 `invoke` 一样不要求在可视区、不移动鼠标。
- **`scroll_into_view` 推迟，D2 不变。**
  - D2 不返回 offscreen 节点，滚出可视区的元素没有 ref，这个命令只能作用于已部分可见的元素，价值很小。
  - 要让它有用，就要返回 offscreen 节点。这会让每次观察多花 token：Chromium 长页面上几乎每个元素都有 ScrollItem。需要先定上限再实测。
  - 在此之前，滚动用 `scroll(element)` 加重新观察。D6 原文“`invoke` 可用于……不在可视区的元素”同样受 D2 限制，只对部分可见的元素成立。
- **回读**：动作后轮询状态直到达到目标，最多 300 ms，且不超过元素超时的一半。
  - 目标：`toggle` 是 checked / mixed 与动作前不同；`select` 是 `selected`；`expand` 是 `expanded`（含部分展开）；`collapse` 是 `collapsed`。
  - 未达到目标返回 `state_mismatch`，执行状态 `completed`，与 `set_value` 的 `value_mismatch` 一致。元素在动作后消失则不判定。
- **macOS AX 后补**：先在 Windows 上实施，AX 一侧同日在 Mac 上实施（§9.6）。AX 按 D13 映射：`toggle` 用 `AXPress`；`select` 对单选和标签页用 `AXPress`，对行设置 `AXSelected`；`expand` / `collapse` 设置 `AXExpanded`（没有该属性的大纲行设置 `AXDisclosing`），展开三角用 `AXPress`。

### 9.2 TODO

- [x] 协议（buckyos-api）：四个命令及其校验，单元测试。
- [x] UIA（native_windows.cpp）：TogglePattern、SelectionItemPattern、ExpandCollapsePattern 的执行与回读；`--uia` 自测增加复选框 `toggle`、树节点 `select` 与 `expand` / `collapse`。
- [x] Driver（desktop.rs）：element 目标与 pattern 校验、回读判定与 `state_mismatch`、capabilities 按平台列出命令，单元测试。
- [x] agent_tool 工具说明；websdk 类型与测试。
- [x] 文档与手册：草案 §7、[agent_daemon.md](../doc/arch/agent_daemon.md)、[node_exector.md](../doc/arch/node_exector.md)；手册 `xllm`、`buckyos_admin`、`opendan_agent_dev`。
- [x] Windows 单元测试与原生自测（结果见 §9.3）。
- [x] Windows 实机 DV：Notepad 设置页开关 `toggle`、资源管理器文件 `select` 与导航树 `expand` / `collapse`、Chrome 标签页 `select`（结果见 §9.4）。
- [x] macOS AX 实施、`ax_macos.m` 自测与 DV（在 Mac 上，结果见 §9.6）。

### 9.3 实施记录（2026-11-11）

| 位置 | 内容 |
| --- | --- |
| [computer.rs](../src/kernel/buckyos-api/src/computer.rs) | `Toggle`、`Select`、`Expand`、`Collapse` 四个命令（字段同 `Invoke`，只接受 element 目标）；`element_action()` 给出命令对应的通用动作 |
| [native_windows.cpp](../src/kernel/computer_driver/src/native_windows.cpp) | element 操作 4–7：调用对应 pattern 后轮询回读，回复 `before`、`st`（状态位）与 `gone`；`--uia` 自测增加树视图（comctl32，只在自测入口链接） |
| [desktop.rs](../src/kernel/computer_driver/src/desktop.rs) | 声明动作与 pattern 校验；`state_mismatch` 判定；`STATE_COMMANDS`（只在 Windows 为真）控制 capabilities 与解析，macOS 返回 `command_unsupported` |
| [agent_tool computer.rs](../src/frame/agent_tool/src/runtime/computer.rs) | 工具说明 |
| [executor_client.ts](../../buckyos-websdk/src/executor_client.ts) | 四个命令的类型与测试 |
| 文档与手册 | 草案 §7、agent_daemon.md、node_exector.md；`xllm`、`buckyos_admin`、`opendan_agent_dev` |

验证（Windows 11，5120×1440 主屏）：

- `cargo test -p buckyos-api --lib computer` 10 项、`cargo test -p computer_driver` 28 项（2 项 ignored）、`cargo test -p agent_tool --lib runtime::computer` 11 项、`cargo test -p node_executor --lib --test computer` 6 + 12 项，均通过；websdk `jest tests/executor_client.test.ts` 5 项通过，`tsc -p tsconfig.build.json` 通过（输出到临时目录，未改 `dist`）。
- MSVC `/W4 /WX` 自测编译通过；`--uia` 通过：复选框 toggle 后为 checked，树节点 select 后为当前选中项，expand / collapse 后树视图的展开状态与回读一致，对没有 Toggle pattern 的输入框 toggle 返回失败（code 3，不是超时）。
- `cargo check -p buckyos_agent_daemon -p node_daemon -p computer_driver --tests --examples` 通过。

### 9.4 实机 DV（2026-11-11，Windows 11，5120×1440 主屏）

方法同 §7.5：`computer_dv` harness，每步先观察、按大纲选 ref 再动作，动作反馈为默认 auto。开始前确认本机没有 node_daemon、opendan 或其他 Agent 进程。inspect 的 `commands` 列出四个新命令。全程没有截图，也没有出现 `state_mismatch`。

| 场景 | 结果 |
| --- | --- |
| Notepad 菜单与设置页 | 通过。菜单栏 File `expand` 后为 `expanded`，反馈树出现 PopupHost 与菜单项；`collapse` 后弹出窗口消失。设置页 “Word wrap” 开关（`switch`）`toggle` 两次：checked → 未选中 → checked，设置已复原。“Opening files” combobox `expand` 列出两个可 `select` 的选项，未选择即 `collapse`；“App theme” 展开器 `expand` 后列出主题单选，`collapse` 复原 |
| 资源管理器 | 通过。在临时目录中对 `beta.txt` `select`，反馈中为 `selected`。导航树 “DevDrive (D:)” `expand` 后为 `expanded`，`collapse` 后为 `collapsed`。注意：资源管理器在 `expand` 导航树节点时同时选中该节点，并把窗口导航到 D:，这是应用的行为 |
| Chrome 标签页 | 通过。经 `scope` 观察后台 Chrome 窗口（261 节点），对另一标签页 `select`，反馈中该标签页为 `selected`；再 `select` 原标签页复原。注意：Chrome 在标签页被 `select` 时会把窗口带到前台 |
| 未声明的动作 | 对标签页 `toggle` 返回 `element_action_unsupported` / `not_started` |

DV 打开的 Notepad 与资源管理器窗口最后用 `invoke` 关闭按钮关掉。

### 9.5 未完成与风险

- `state_mismatch` 只有单元测试覆盖，DV 中没有出现过；Chromium 等异步 provider 在 300 ms 内仍未更新状态时会误报，需在实际使用中观察频率。
- `select` / `expand` 可能带来应用自己的副作用（Chrome 激活窗口、资源管理器导航），Agent 需要看反馈观察，而不是假定只有状态改变。
- `scroll_into_view` 仍推迟（§9.1）。

### 9.6 macOS AX 实施记录（2026-11-11）

| 位置 | 内容 |
| --- | --- |
| [native.m](../src/kernel/computer_driver/src/native.m) | element 操作 4–7：执行后轮询回读，回复 `before`、`st`、`gone`，与 Windows 同一格式；`ax_press` 推广为 `ax_deliver`（按下或设置属性，嵌套循环判断相同）；`ax_read` 改为只对能执行的元素声明 toggle / select / expand，读取 `AXDisclosing` |
| [desktop.rs](../src/kernel/computer_driver/src/desktop.rs) | 去掉 `STATE_COMMANDS`，capabilities 与元素解析在两个平台相同 |
| [ax_macos.m](../test/agent_daemon/ax_macos.m) | 测试窗口增加单选按钮、popup、combobox 和可展开、允许多选的大纲；新增 23 项检查 |
| 文档与手册 | 草案 §7、[agent_daemon.md](../doc/arch/agent_daemon.md)、[node_exector.md](../doc/arch/node_exector.md)；手册 `xllm`、`buckyos_admin`、`opendan_agent_dev` 删除“macOS 未实施”。agent_tool 工具说明与 websdk 类型本就与平台无关，无需修改 |

实施中定下的细节：

- **执行**：
  - `toggle`：复选框上的 `AXPress`。
  - `select`：单选按钮与标签页上的 `AXPress`；行上设置 `AXSelected = true`。AppKit 大纲允许多选时，设置行的 `AXSelected` 会替换原有选择，与 UIA `Select` 一致（自测确认）。
  - `expand` / `collapse`：设置 `AXExpanded`；没有该属性的大纲行设置 `AXDisclosing`。展开三角用 `AXPress`，因为按下是翻转，所以已处于目标状态时不按。
  - 按下和设置属性都经 `ax_deliver` 在后台队列执行，0.5 s 未返回时用与 `invoke` 相同的判断返回 `pending`。combobox 的展开实测不阻塞。
- **声明规则（取代 §8.2 的按角色声明）**：实测（自测窗口、Finder、System Settings、Chrome）发现：
  - AppKit 大纲行没有 `AXExpanded`，只有 `AXDisclosing`：可展开的行可写，叶子行只读；Finder 列表视图的表头行与 System Settings（SwiftUI）侧栏行报只读的 `AXDisclosing = 1`。
  - `NSPopUpButton` 没有 `AXExpanded`（打开它用 `invoke`），`NSComboBox` 的 `AXExpanded` 可写。
  - Chromium 网页节点的 `AXSelected` / `AXExpanded` 只读（GitHub 文件表的行、带弹出菜单的网页按钮）；Chrome 自身的标签页声明 `AXPress`。

  因此：`toggle`、单选与标签页的 `select`、展开三角的 `expand` / `collapse` 要求元素声明 `AXPress`；行的 `select` 要求 `AXSelected` 可写；其余 `expand` / `collapse` 要求 `AXExpanded` 或 `AXDisclosing` 可写，可写时才给出 expanded / collapsed 状态。不可写的 `AXExpanded` 为 true 时只显示 expanded；不可写的 `AXDisclosing` 不显示。可写性查询只对行、combobox、popup、菜单按钮各多一次 IPC。效果：Chrome 网页表格的行原先因只读属性显示 `collapsed (expand, select)`，现在只是 `row`。
- **回读**：与 Windows 相同，动作后轮询直到达到目标，最多 300 ms 且不超过元素超时的一半；读取失败但元素未失效时不给 `st`，不判定 `state_mismatch`。
- **不激活应用**：这些命令可直接作用于后台窗口，不把应用带到前台；Chrome 标签页 `select` 也不激活 Chrome（Windows 上会）。

验证（macOS 15.8.1、Apple silicon、主屏 1620×1080）：

- 原生自测 `ax_macos.m`（`-Wall -Werror`）40 项通过，其中新增 23 项：
  - 声明：单选、两类大纲行、展开三角、combobox 的动作声明正确；只读 `AXDisclosing` 的叶子行和没有 `AXExpanded` 的 popup 不声明 expand。
  - 执行与回读：复选框 toggle；单选与行 select（行 select 替换多选）；大纲行与展开三角 expand / collapse（已展开时再 expand 不翻转）；combobox expand / collapse。回读状态与之后的快照一致。
  - 未声明的动作：对文本框 toggle、对 popup expand 直接失败（code 3）。
- `cargo test -p computer_driver`：21 通过，1 项 ignored（`semantic_smoke`）。对自测窗口运行 `semantic_smoke`，大纲为 `treeitem collapsed (select, expand)`、`treeitem (select)`、`combobox "Color" collapsed (focus, set_value, expand)`、`checkbox "Remember" (invoke, toggle)`。
- `cargo check -p node_daemon -p buckyos_agent_daemon -p computer_driver --tests --examples` 通过，只有一条与本改动无关的既有 dead code 警告（`node_daemon/src/paios_images.rs` 的 `short_id`）。
- 实机 DV（`computer_dv` harness，方法同 §8.3，binding `desktop:display:3`）：开始前确认本机 Jarvis 自 11-08 起没有桌面操作。inspect 的 `commands` 列出四个命令。全程没有截图，没有出现 `state_mismatch`，前台始终是 Claude 桌面应用。单次动作的总耗时（含 60 ms 后的语义反馈）为 170–290 ms。

| 场景 | 结果 |
| --- | --- |
| Finder 侧栏（后台窗口，经 `scope`） | 分区标题行 “Locations” 为 `expanded (select, collapse)`；`collapse` 后为 collapsed，Macintosh HD、liu-nas 从反馈树中消失；`expand` 复原。叶子行只有 `select` |
| Finder 列表视图 | `select` 文件行 node_daemon，行与单元格为 selected。在 §8.3 的 DV 夹具目录中临时建了一个子文件夹：文件夹行为 `collapsed (select, expand)`，`expand` 后为 expanded，下一次观察出现子文件行；`collapse` 复原。临时文件夹已删除 |
| Chrome 标签页（后台窗口） | 对“知乎”标签页 `select`，反馈中为 selected，Chrome 仍在后台；再 `select` 原标签页复原。GitHub 网页的 16 行不再带 expand / select |
| 复选框 | Xcode 组件选择窗口的 “watchOS 26.2” `toggle` 两次：未选 → checked → 未选，已复原（未点 Download & Install）。对禁用的 “macOS 26.2” `toggle` 返回 `element_disabled` / `not_started` |
| 未声明的动作 | 对 Finder 标签页 `toggle` 返回 `element_action_unsupported` / `not_started` |
| System Settings（只读观察） | 侧栏行只有 `select`，不再因只读的 `AXDisclosing` 显示 expanded |

未完成与风险：

- **反馈时机**：Finder 展开文件夹后，60 ms 时的反馈中还没有子行（异步加载），要重新观察才能看到，与 §7.5 的问题相同。
- **popup 没有 expand**：AppKit popup 用 `invoke` 打开菜单（菜单跟踪中返回 `pending`），再 `invoke` 菜单项。网页上的弹出按钮（`AXExpanded` 只读、不声明 `AXPress`）只能 `click(element)`。
- **行没有名称**：AppKit 大纲行的名称在单元格上，大纲中 treeitem 本身无名，模型需要看子节点；Windows 的树节点有名称。展开三角与所在行重复声明 expand。
- **未实测**：真实应用中的 NSComboBox 展开（只有自测覆盖）；SwiftUI 开关的 toggle（不改系统设置）；Chromium 网页复选框与单选（不声明 `AXPress` 时不会有 toggle / select）；`state_mismatch` 在实际使用中的频率。
- **Windows**：本次只去掉了 Rust 侧在 Windows 上恒为真的 `STATE_COMMANDS` 条件，Windows 行为不变，未在 Windows 上重新编译和测试。
