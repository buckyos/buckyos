# BuckyOS AI Workspace UI 改进实施记录与验收报告

> 对应：[《BuckyOS AI Workspace UI 改进》](<BuckyOS AI Workspace UI改进.md>)（下称“设计”）。实施范围是设计 §13 的 P0 和 P1；P2（在线成员、演示场次与互动）未做。实施中的取舍和偏差已回写到设计 §15。
>
> 读者：审查这次界面改造，或在其上继续开发的人。
>
> 本文只记录**已经运行过的事实**，每项结论后面附有产生它的命令或测试。没有运行过的内容写在 §6。

## 1. 结论

- **P0、P1 已实施。** 画布铺满应用区域，主工具栏、演讲工具栏、竖排对象工具栏悬浮在其上；右下角没有导航区。完整功能索引在主菜单里。插入通过统一目录完成，不再弹出标题输入。新增对象剪贴板、画布图标、手形工具和一次性放置、访问链接，以及启动时恢复上次打开的画布和离开检查。
- **核心契约：** Surface 新增 `icon` 键，只做格式校验，并带出到 outline。`aiworkspace-core` 新增 1 个测试，核心与服务测试全部通过（§3）。
- **Desktop：** `pnpm check`、`eslint`、`pnpm build` 通过。AI Workspace 的 Playwright 全套 **80 个用例对真实后台进程全部通过**，其中新增 `ui.spec.ts` 11 个，原有 69 个按新入口改写后继续通过。完整结果见 §4。
- 改造中修正了两个原有缺陷：其一，用户工作状态在重新载入或关闭前约 0.8 秒内写入的值，会在下一次和服务端对齐时被当作“已在别处删除”而丢掉；其二，在画布上点击空白处结束编辑时，便签等编辑器在保存前就被卸载，正在输入的内容会丢失（§5）。

## 2. 交付物

| 交付物 | 位置 |
| --- | --- |
| 核心：Surface `icon`（预设 id，只限 Surface）的校验与 outline 投影 | `src/frame/aiworkspace/core/src/{types,read}.rs`、`core/tests/phase2.rs`；WASM 重新生成到 `src/frame/desktop/src/app/aiworkspace/wasm/` |
| Shell：顶层视图、当前画布、右侧面板、对话框、布局偏好、尺寸档 | `ui/shell/{WorkspaceShell,shellContext}.ts(x)` |
| 主工具栏（图标、改名、画布下拉、数据源、主菜单、协作）与主菜单 | `ui/shell/{MainToolbar,MainMenu}.tsx` |
| 演讲工具栏（批注、缩放与视图导航、身份、分享链接） | `ui/shell/PresenterToolbar.tsx` |
| 右下角状态区（保存/连接状态摘要、叠在其上的告警与提示）、“修改状态”面板 | `ui/shell/StatusSummary.tsx`（`StatusDock`） |
| 弹层与键盘菜单、右侧面板、对话框（新建、导出、导入、帮助、Mock、离开检查） | `ui/shell/{popover,SidePanel,dialogs}.tsx` |
| 画布：对象工具栏、插入目录、剪贴板、画布管理、预设图标；`CanvasView` 重写，`RenderHost` 增加手形工具与放置预览，`Camera` 增加安全区 | `ui/canvas/{ObjectToolbar,InsertCatalog,catalog,clipboard,surfaceManage,icons,CanvasView,tools}.tsx`、`ui/canvas/render/{RenderHost.tsx,camera.ts}` |
| Block 定义的目录元信息；图片/附件可以插入；扩展 Block 的 `def_id` 不再写进 `config` | `ui/blocks/{registry,builtin,samples}.ts(x)`、`ui/wish/wishBlock.tsx`、`ui/extensions/{HtmlBlockHost,declarative}.tsx` |
| 打开规则（链接、最近工作区、离开检查、关闭窗口与退出登录） | `AIWorkspaceAppPanel.tsx`、`state/recent.ts`、`state/store.ts`（`prepareLeave`、提示自动消失）、`api/transport.ts`（`target`、`principal()`） |
| Desktop 接入：访问链接 → 启动请求；应用请求退出登录 | `src/frame/desktop/src/desktop/DesktopRoute.tsx` |
| 工作区列表改用统一的新建对话框；导入、导出表单与 Shell 共用 | `ui/WorkspaceList.tsx`、`ui/shell/dialogs.tsx` |
| 用户工作状态对齐规则修正 | `state/userState.ts` |
| 端到端用例 | 新增 `tests/aiworkspace/ui.spec.ts`；`fixtures.ts` 新增主菜单、缩放、模式、面板、状态、返回列表等辅助函数；其余用例按新入口改写 |
| 文档回写 | 设计状态行与 §15、后端 `README.md`（`icon` 键）、Desktop `src/app/aiworkspace/README.md`（结构、“UI improvement”一节、未实现项） |

## 3. 后台验收

在 `buckyos/src` 下运行：

```bash
cargo test -p aiworkspace-core -p aiworkspace-store -p aiworkspace
```

| 测试程序 | 通过 |
| --- | --- |
| `aiworkspace-core`：单测 24、`anchor` 7、`phase2` 7（新增 1）、`planner` 7、`schemas` 3、`vectors` 1 | 49 |
| `aiworkspace-store`：单测 2、`collab` 6、`kernel` 12、`packages` 7、`phase2` 4、`replica` 3、`wish` 6（`perf` 1 个默认忽略） | 40 |
| `aiworkspace`（服务进程）：单测 1、`process` 2、`quality` 8、`wish` 3 | 14 |
| 合计 | **103 通过，0 失败** |

WASM 由 `PATH=/tmp/dev-cache-root/cargo/bin:$PATH CARGO_TARGET_DIR=/tmp/dev-cache-root/cargo-target bash frame/aiworkspace/wasm/build.sh` 重新生成。离线副本也调用同一份校验，因此离线修改图标同样受核心约束。

新增 `core/tests/phase2.rs::surface_icon_is_a_shared_preset_id`：`icon` 能写入、能清除，并出现在 outline 中；大写、含空格、空串、非字符串、超过 32 个字符都会被拒绝；`folder` 上不接受 `icon`。图标经导出、导入后保留，由 `ui.spec.ts` UI-P03 通过 kRPC 验证。

## 4. Desktop 验收

在 `src/frame/desktop` 下运行：

```bash
pnpm check
pnpm exec eslint src/app/aiworkspace src/desktop/DesktopRoute.tsx tests/aiworkspace
pnpm build
PATH=/tmp/dev-cache-root/cargo/bin:$PATH AIWS_BIN=<aiworkspace 调试构建> pnpm exec playwright test --config=playwright.aiworkspace.config.ts
```

| 检查 | 结果 |
| --- | --- |
| `pnpm check`（`tsc -b`） | 通过 |
| `eslint`（AI Workspace、`DesktopRoute.tsx`、e2e 用例） | 0 问题 |
| `pnpm build` | 通过（只有原有的 chunk 体积提示） |
| Playwright `playwright.aiworkspace.config.ts` | **80 通过，0 失败**（7.7 分钟）；`probe.spec.ts` 的渲染性能门槛不变，同样通过 |
| 状态区移到右下角之后 | 全套重跑 79 通过；唯一失败的 5,000 Block 渲染探针原因是拖动起点落在对象工具栏上（对象工具栏随之上移）。探针改为选一个中心确实在画布上的已选 Block 作起点后，`probe.spec.ts` 与 `ui.spec.ts` 共 15 个通过 |

改写原有用例时只改入口，不放宽断言：缩放和“适应全部”改从演讲工具栏的比例菜单操作；画布模式、受控加工、关闭工作区改从主菜单操作；准备离线、接管副本和处于“已保存到本设备”的修改改在“修改状态”面板中操作；“移动到画布…”改在就近工具栏的“更多”中操作；删除画布改在画布下拉每一项的“⋯”中操作；新建工作区和样例改用新建对话框；插入时不再弹出标题输入，需要标题时在插入目录里填写。原有的两处点击坐标 `(20,20)` 现在落在主工具栏上，已改为空白处。夹具 `openApp` 会把窗口最大化，并关闭启动时自动恢复的工作区；重新载入或冷启动后先调用 `backToList`，再从列表打开，原有断言保持不变。

| 编号 | 场景 | 用例 |
| --- | --- | --- |
| UI-P01 | 重新载入后恢复上次的工作区和画布；关闭后本次不再自动恢复；访问链接优先于最近记录，用后从地址栏清除；链接指向的画布已删除时回到第一张画布并提示；无权限的身份打开链接时说明原因并留在列表；最近记录按身份隔离 | `ui.spec.ts` UI-P01 |
| UI-P02 | 三个工具栏都存在，右下角没有导航区；在应用宽度不限、1024、768、390 下，主工具栏、演讲工具栏、对象工具栏、状态摘要互不重叠，且状态摘要位于工作区右下角；右侧面板宽档占布局宽度，中、窄档为抽屉，都能关闭；“适应全部”后点击对象能选中该对象 | UI-P02 |
| UI-P03 | 按 Esc 零提交；按 Enter 只产生一次提交，标题与名称同时更新；名称为空时恢复原名；他人并发改名时保留我的输入并说明原因；图标被另一客户端看到，只读客户端不能修改；导出、导入后图标保留 | UI-P03 |
| UI-P04 | 新建流式页带图标，画布和内容区在同一次提交中创建；模板标明含模拟内容，创建新工作区并打开，原工作区的提交数不变 | UI-P04 |
| UI-P05 | 选中对象后按 Esc 零写入；点击画布在点击处创建并直接进入便签输入，点击别处后内容已保存；主菜单插入的对象放在视图中央；右键插入的对象放在右键位置；“添加已有数据”只新建视图，不新建数据；样本渲染器缺少数据时不能插入 | UI-P05 |
| UI-P06 | 用手形工具拖动只移动视图、不选中对象；V/H 切换工具；面板中的比例与主菜单缩放作用于同一相机；隐藏演讲工具栏后主菜单仍能“适应全部”；恢复默认布局；全程零提交 | UI-P06 |
| UI-P07 | 复制分组后粘贴，新分组和成员都是新 id，数据仍是原来的；一次提交，可整体撤销；剪切后跨画布粘贴，一次提交，id 和绑定不变；在富文本里复制、粘贴由编辑器处理，不会新建 Block | UI-P07 |
| UI-P08 | 查看模式隐藏插入入口，主菜单的“插入”禁用；Esc 关闭菜单后焦点回到触发按钮；查看模式下可以批注；播放编辑是占位，“开始演示”禁用并说明原因；重开工作区时从播放编辑回到查看模式并提示；切换数据源时会话保持，按钮显示“返回画布” | UI-P08 |
| UI-P10/P11 | “协作”打开权限面板；分享链接指向当前画布，不含令牌，说明“不会授予权限”；只读成员用链接打开时落在同一画布，只看到自己的授权，没有插入入口；身份面板说明在线状态尚未接入，开发身份不能退出 | UI-P10/P11 |
| UI-P12 | 选中许愿格后点 AI 入口打开它的任务，不新建；未选中时放置新的许愿格并打开任务界面，不自动运行 | UI-P12 |
| UI-P13 | 菜单支持方向键、Enter、Esc，关闭后焦点回到触发按钮；可从主菜单打开快捷键帮助；在编辑器里输入 v、h 不会切换工具；画布获得焦点时 V/H 生效 | UI-P13 |
| 原有用例 | 第一期、第二期和许愿格的全部用例按新入口改写（主菜单、缩放面板、模式子菜单、修改状态面板、更多菜单、新建对话框、启动恢复），断言保持不变 | `basics / blocks / canvas / collab / features / annotations / permissions / wish / probe / offline*.spec.ts` |

## 5. 实测中发现并修正的问题

| 问题 | 后果 | 修正 |
| --- | --- | --- |
| 用户工作状态和服务端对齐时，把服务端没有的本地条目一律当作“别处已删除”而删除 | 切换画布、切换模式后约 0.8 秒内重新载入或关闭工作区，这次修改会丢失；离线测试中自动打开的“修改状态”面板随即又被关上 | 只删除服务端曾经确认过的键；服务端从未见过的本地键改为上传；关闭时把尚未发出的写入发出去（尽力而为） |
| 在画布上点击空白处结束编辑时，编辑器先被卸载，没有机会触发失焦保存 | 刚输入的便签内容丢失 | 结束编辑前先让焦点离开编辑器，让失焦时的保存先运行 |
| Node 端用例夹具导入 `demos.ts` 时，经 `sample.ts` 连带导入了没有 import attribute 的 JSON | 所有使用 `api.demo` 的用例在夹具阶段失败 | `TemplateFailure` 移到 `demos.ts` |
| 选中态、按下态直接使用 `--cp-accent-soft` 原色作底色，配 accent 蓝字，按钮上还叠加了第二层焦点描边 | 新建对话框、各处页签、数据树、插入目录、画布下拉、工具按钮等处颜色过重、不协调，深色主题下也一样 | 改为与 Desktop 自带应用相同的派生方式（`.aiws-root` 上的 `--aiws-*` 变量，见设计 15-16）；按钮使用 Desktop 的焦点光环；单选框和复选框不再套用输入框样式；“含模拟内容”标签不再被拉伸成整行宽；深色主题的主按钮改用深色文字 |
| 工作区告警区显示 Desktop 的 service worker 更新提示 | 用户把它看作多余的模拟提示 | 从 AI Workspace 中删除（设计 15-17） |
| 自动恢复工作区之前，列表会先渲染一下 | 列表一闪而过，用例看到列表时会和随后的自动打开发生竞态 | 检查最近记录期间显示“正在打开工作区…” |

## 6. 未完成与未验证

- **P2 未做：** presence（在线成员、头像、断线、跟随、远端指针）、演示场次（主讲、观众、公共屏）、互动按钮及其配置。相应入口不显示，或禁用并说明原因。UI-P14 没有用例。
- **设计中写明暂缓的内容：** 纯 UI 对象和画布空白点的批注锚点（点选这类目标时会提示）；跨工作区粘贴（会提示暂不支持）；画布图标不支持上传图片。
- **未自动化的检查：** 浏览器缩放到 200% 的布局；中文输入法组合输入期间的快捷键行为只在代码中通过 `isComposing` 判断，没有用例。窄宽度用例是在桌面窗口内限制应用容器宽度后测量的；Desktop 在浏览器宽度 ≤ 768 时切换为移动外壳，从移动外壳启动本应用不在本次范围内。
- **访问链接：** 只在开发直连身份下验证。真实 Zone 中“未登录 → 登录页 → 回到链接”依赖 Desktop 已有的 `redirect_url`，本次没有在 DV 环境运行。通过链接离线冷启动也没有验证。
- **没有在真实 Zone / DV 环境运行。** 所有结论都来自独立后台进程加 Desktop 的模拟运行时，浏览器只测了 Chromium。

## 7. 已知风险

| 风险 | 后果 | 可选的后续措施 |
| --- | --- | --- |
| 用户工作状态改为“服务端没见过就上传” | 另一设备删除的条目，如果本设备在删除前从未与服务端同步过，会被重新上传而恢复。影响范围仅限视口、面板等个人偏好，不涉及文档 | 给条目加时间戳，对齐时比较时间先后 |
| 剪贴板只保存在内存里 | 刷新页面或关闭窗口后剪贴板清空，不能在两个窗口之间复制 | 需要时改用 `BroadcastChannel` 或系统剪贴板中的自定义 MIME 类型 |
| 主菜单把编辑类命令收进二级菜单 | 和设计 §6.1 的平铺结构不同，多一步操作 | 菜单高度允许时恢复平铺 |
