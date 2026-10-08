# BuckyOS AI Workspace 第三期：实施记录与验收报告

> 日期：2026-10-08。对应设计：[第三期规划](<BuckyOS AI Workspace 第三期规划.md>) v0.2（实施后为 v0.3，实施回写见其 §18）。
>
> 环境：独立运行的 `aiworkspace` 服务（调试构建；性能数字用 release 构建）＋ Vite 开发服务器，Desktop 外壳使用模拟运行时，Chromium（Playwright，无 GPU 光栅化）。离线与性能探针用例跑在生产构建（`vite preview`）上，离线用例通过可切断的 TCP 中继断网。没有在 DV 或真实 Zone 上运行。

## 1. 结论

第三期 M0–M4 已实现：

- **数据模型**：Frame 的 `presentation`（背景、讲解备注、说明，任意 Block 的“放映时可操作”），新实体 Viewport 与演讲路径，放在新的系统文件夹 `shows` 中；步骤引用和 Viewport 的画布引用都不阻止删除，目标删除后步骤成为失效项。FORMAT 0.4、PROTOCOL 0.5，旧工作区和旧包不迁移。
- **路径编辑**：画布子模式“路径编辑”取代占位：路径与引导的列表和属性、舞台尺寸（改尺寸时一次提交调整所有 Frame 的比例）、可拖拽排序的步骤列表、步骤与目标属性、失效与比例诊断、画布上的步骤取景框（Viewport 可直接拖动和缩放）、“添加当前视角”的取景框。所有修改都是一次提交、一次撤销，步骤修改在冲突时按步骤命令自动重放。
- **放映**：独立的舞台视图：fly（van Wijk–Nuij）、fade、cut 三种转场和 §9.2 的默认规则，到达时目标 Block 已挂载并就绪，转场可被打断，自由浏览与“回到本步骤”，黑屏、激光笔、板书，§10.4 的按键。舞台复用画布的 `RenderHost`。
- **数据归属**：有写权限的放映给工作区加放映锁，其他所有会话的写入被拒（`SHOW_LOCKED`），其他窗口随即只读并显示放映者，离线副本的写入排队等放映结束；路径涉及的画布上有“放映时可操作”的 Block 时，服务端用 `VACUUM INTO` 建临时克隆，现场写入只进克隆，放映结束、舞台失联或服务重启后克隆被删除。
- **提示器链接**：任何浏览器打开即进入演讲者控制面板，不需要登录；备注、下一步、目录、计时、翻页、黑屏；同机已登录的大屏在面板后渲染当前画面。中继在服务内存中，舞台刷新后继续本场。
- **使用引导**：网站式导览，取景外变暗，画布不接受操作，进度按用户保存，打开作品时提示一次；手机和离线可用。
- **导出**：分享包默认去掉讲解备注，个人备份默认保留，导出对话框可选；说明总是保留。

与规划不同或规划未写明的细节都记在规划 §18（18-1 至 18-22）。需要注意的三点：

- **named store 没有接入**（18-9）。现有对象存储本来就是服务级、按内容寻址、所有工作区共用的，克隆不复制任何对象字节，所以本期目标“克隆只增加引用”已经成立；接入系统 named store 仍是第一期留下的替换边界。
- **放映锁只在内存中**（18-6）。和中继一样，服务重启即结束放映、释放锁、删除残留克隆。
- **版本**：FORMAT 0.4 让升级前的工作区（包括 DV 上的测试工作区）无法打开，需要重建测试数据；本次没有部署到 DV。

## 2. 交付物

| 交付物 | 位置 |
| --- | --- |
| 内核：`TYPE_VIEWPORT`、`TYPE_SHOW_PATH`、系统文件夹 `shows`（只收这两类），Cell `presentation` 校验，Viewport 与路径的校验（目标在写入时判定，未改动的失效步骤保留），`show_target` / `show_surface` 引用，读模型投影（`viewport`、`show_path`、`live`），去掉备注的物化 `materialize_with` / `without_notes`，错误码 `SHOW_LOCKED`，FORMAT 0.4 / PROTOCOL 0.5 | `src/frame/aiworkspace/core/src/{model,types,plan,read,materialize,error}.rs`；`schemas/*.json`；新增 `core/tests/presentation.rs` |
| 共用 fixture | `src/frame/aiworkspace/fixtures/presentation/commits.json` |
| 存储：放映锁（服务级、内存）、提交前的锁检查、`ws.get_info.show_lock`、临时克隆与删除、启动时清理残留克隆、克隆不出现在列表也不能准备离线，导出 `include_notes`，许愿格快照排除 `shows` | `store/src/{show,workspace,service,export}.rs`、`store/src/wish/{snapshot,context}.rs`；新增 `store/tests/show.rs`；`store/tests/perf.rs` 增加克隆计时并修复已失效的第三个探针 |
| 服务：`show.start / heartbeat / publish / command / watch / notes / end`，提示器令牌认证，清扫器，`doc.wait_changes` 携带 `show_lock`，`doc.export` 的 `include_notes` | `server/src/{show,lib}.rs`；新增 `server/tests/show.rs` |
| WASM | `src/frame/desktop/src/app/aiworkspace/wasm/`（`wasm/build.sh` 重新生成） |
| 前端模型与写入：取景换算、舞台适配、默认转场、飞行纯函数、步骤命令与重放、路径／Viewport／Frame 的创建、舞台尺寸、备注／说明／背景、`live` | `presentation/{model,pathOps}.ts` |
| 路径编辑 | `presentation/PathEditor.tsx`；`ui/canvas/CanvasView.tsx`（子模式、左侧留白、Block 属性中的“放映时可操作”） |
| 放映：开始对话框、会话与克隆 store、舞台、控制器、中继客户端、标记 | `presentation/{StartShowDialog.tsx,showSession.ts,StageView.tsx,StageCanvas.tsx,controller.ts,relay.ts,overlays.tsx}` |
| 画布复用：`RenderHost` 的 `prefetch` / `hidden`、`show` 模式；`useSurfaceLayout` 从 CanvasView 抽出；`show` 模式策略与 live 判定；`show` 模式用 View 实现 | `ui/canvas/render/RenderHost.tsx`、`ui/canvas/useSurfaceLayout.ts`、`ui/blocks/{registry.ts,useBlockContext.ts,BlockHost.tsx}` |
| 使用引导 | `presentation/GuideOverlay.tsx` |
| 提示器 | `presentation/PrompterRoute.tsx`；`src/App.tsx`（路由）、`src/publicRoutes.ts`（免登录）、`api/transport.ts`（`prompterCall`） |
| 外壳：放映接管窗口、恢复本标签页的放映、引导状态、提示一次；放映锁提示与强制结束；只读原因；改名 `ViewToolbar` 并加引导与放映按钮；主菜单“开始放映…”“使用引导”“路径编辑”；导出“包含讲解备注”；快捷键说明 | `ui/shell/{WorkspaceShell,shellContext,ViewToolbar,MainMenu,StatusSummary,dialogs}.tsx`、`state/{hooks,store,outline}.ts`、`api/{session,client,types}.ts`、`offline/replicaSession.ts` |
| 数据源视图：`shows` 默认折叠、标为系统、不作为新建或移动目标；新类型标签；系统文件夹排序 | `ui/sources/{DataTree,DataDetail,PropertiesPanel,dataOps}.tsx` 等 |
| 文档 | 第三期规划 v0.3（§18）、第四期规划 v0.1.1（按 §17 同步）、UI 改进（§4 规则 3、模式表、改名说明）、后台与前端 README |

## 3. 后台验收

在 `buckyos/src` 运行：

```bash
cargo test -p aiworkspace-core -p aiworkspace-store -p aiworkspace     # 125 个用例通过（core 66、store 43、server 16），另有 1 个性能探针默认忽略
cargo test -p aiworkspace-store --release --test perf -- --ignored --nocapture
(cd frame/aiworkspace/aiws && deno test -A)                           # 3 个通过
PATH=/tmp/dev-cache-root/cargo/bin:$PATH bash frame/aiworkspace/wasm/build.sh
```

| 范围 | 用例 | 结果 |
| --- | --- | --- |
| M0 契约（`core/tests/presentation.rs`，9 个） | fixture 重放出五步混排路径和引导路径；`shows` 是 `data` 下的系统文件夹；每步一条不阻止删除的 `show_target` 引用，Viewport 一条 `show_surface`；读模型投影；`presentation` 按 Block 类型校验（备注超长为 `LIMIT_EXCEEDED`，`null` 删除整个键）；`shows` 只收路径和 Viewport；Viewport 的形状与画布（不存在、不是 Surface、流式页）；步骤只在写入时判定，同一 Frame 两步，目标删除后路径仍可编辑，重绑失效步骤按新步骤判定；舞台尺寸与用途；撤销恢复步骤及其引用；去掉备注的物化保留说明，路径内容不变 | 通过 |
| 放映锁与克隆（`store/tests/show.rs`，3 个） | 锁住后本人和协作者的写入都是 `SHOW_LOCKED`（可重试，带放映者和 show_id），用户状态照常写；第二场放映被拒；续期则保持、不续期则过期；释放只对本场生效；克隆是完整副本（seq、epoch、内容、授权、资产可读，用户状态为空），对象目录没有新增文件，克隆可写而原作品不变，不出现在列表，不能准备离线；只能删除克隆；服务重启后锁消失、残留克隆被删除 | 通过 |
| 服务进程（`server/tests/show.rs`，2 个，租期缩短为 1.5 s） | 可操作放映：加锁、克隆、协作者写入被拒、`ws.get_info.show_lock`、克隆可写；提示器令牌：读备注、发命令（重复的 command_id 被去重）、长轮询被 publish 唤醒、旧 seq 不覆盖新状态、超时返回；令牌不能 publish / heartbeat / end，错误令牌和不带会话的普通方法都被拒；非放映者不能控制；结束后锁释放、克隆目录消失、令牌失效、原作品没有现场写入。只读用户放映不加锁不克隆；持续续期保持锁，停止续期后锁和克隆被清扫；他人放映时第二场被拒，只读用户不能强制结束，Owner 可以 | 通过 |
| 回归 | 已有测试中写死的大纲数量因新增 `shows` 改为 18；`doc.wait_changes` 的应答多了 `show_lock`；导出接口多一个参数 | 已更新，全部通过 |

性能（release 构建，`store/tests/perf.rs`，1 万行 × 20 字段的表格加资源）：克隆（两个数据库，对象共享）43 ms，删除克隆 2 ms，目标是 ≤ 1 s。该探针的第三组（1,000 个实体）自第二期起已因树规则变化无法运行，本次一并修正。

## 4. Desktop 验收

在 `src/frame/desktop` 运行：

```bash
pnpm check && pnpm exec eslint src/app/aiworkspace tests/aiworkspace src/App.tsx src/publicRoutes.ts && pnpm build
PATH=/tmp/dev-cache-root/cargo/bin:$PATH AIWS_BIN=/tmp/dev-cache-root/cargo-target/debug/aiworkspace \
  pnpm exec playwright test --config=playwright.aiworkspace.config.ts
```

全量运行 136 个用例：129 个通过，7 个失败，失败的都是旧用例对已删除的“播放编辑”占位、旧的实体数量（新增 `shows` 后多一个）的断言，以及一处真实问题（主菜单变长后，最下面的子菜单被状态区的提示挡住，见 §6）。修正后重跑这 7 个和全部第三期用例，全部通过；之后又加了 1 个深色主题截图用例（通过）。第三期用例：

| 编号 | 用例 | 结果 |
| --- | --- | --- |
| P3-01 | 从 UI 放映 `frame1 → frame2 → viewport1 → viewport2 → frame3`，逐步截图；P3-01b 在路径编辑中新建路径、加入两个 Frame 和当前视角，每个动作一次提交、一次撤销，重新打开后子模式和路径还在 | 通过 |
| P3-02 | 舞台改为 4:3：确认后一次提交，三个 Frame 宽度不变、高度变为 1200、中心不变；共享 Frame 显示诊断；Ctrl+Z 恢复 | 通过 |
| P3-03 | 在 1600×1000 和 1100×950 的窗口中，舞台显示的世界矩形与 Viewport 的取景矩形误差 ≤ 1 px；16:9 的 Frame 正好铺满舞台 | 通过 |
| P3-04 | 近距离飞行全程不比两端更远；远距离飞行中途缩放降到两端较小值的一半以下；到达后取景误差 ≤ 1 px | 通过 |
| P3-05 | 转场中连按三次 → 停在第四步、取景正确，1.5 s 后没有回跳；拖动画布进入自由浏览，步骤不变；“回到本步骤”飞回；滚轮再进入自由浏览，Esc 回到本步骤而不退出 | 通过 |
| P3-06 | 实际转场依次为 fly（Frame→Viewport）、fade（步骤指定）、fade（跨画布）；跨画布后遮罩挖空正好是舞台、遮罩不透明度 1、过渡层不透明度 0；`hide` 的 Block 和 Frame 外框不显示 | 通过 |
| P3-07 | 同一 Frame 第二次加入（插在选中步骤之后），拖到第二位，再用 Alt+↑ 移到第一位；两个步骤 id 不同，Frame 的 `order_key` 不变 | 通过 |
| P3-08 | 删除 viewport2 和 frame3：两步标为失效、“2 个失效项”，开始对话框显示 3 步，舞台只有 3 步；移除失效步骤；其他目标未被删除 | 通过 |
| P3-09 | 拦截编辑器的提交，期间另一写入者把步骤倒序：编辑器重放后两处修改都在；另一写入者删掉锚点步骤时，提示“无法应用到最新的路径上” | 通过 |
| P3-10 | 导出对话框：分享包默认不含备注，个人备份默认含；分享包导入为新作品后路径 5 步、说明保留、备注不存在、没有原作者的引导进度 | 通过 |
| P3-11 | 打开时提示一次；引导第一步取景外变暗（挖空为 16:9）；拖动、滚轮、Delete 都不改变画布也不提交；Esc 结束后进度为 1，再开从第 2 步继续，完成后不再提示 | 通过 |
| P3-12 | 放映中他人写入为 `SHOW_LOCKED` 并带放映者；他人的窗口出现“alice 正在放映”并只读；舞台标签页直接关闭后，租期内锁仍在，过期后锁释放、克隆删除、他人窗口恢复；他人放映时 Owner 在提示中“结束放映” | 通过 |
| P3-12（离线副本） | 持有离线副本的窗口在他人放映期间移动对象：保存到本设备、队列 1 条、不进入“需要处理”、后台没有新提交；放映结束后自动发送，后台恰好多 1 条提交 | 通过 |
| P3-13 | 双击可操作的便签输入，方向键不翻页，Esc 交还；写入只在克隆中，原作品不变；退出后克隆不存在，原作品 head 不变 | 通过 |
| P3-14 | 没有可操作 Block 时为“只读放映 · 已关闭写入”；翻页、滚轮缩放、回到本步骤、激光笔、一笔板书和撤销；退出后原作品没有提交，`canvas:mode`、`surface:active`、`viewport:*` 不变 | 通过 |
| P3-15 | 提示器链接带 `#k=pt_…`；未登录的手机上打开显示封面和备注、下一步；手机翻页舞台跟随；第二个提示器（已登录的大屏）在面板后显示当前画面，黑屏开关；关闭它不影响放映；舞台标签页重新载入后回到同一步并继续接收提示器命令；退出放映后提示器显示“放映已结束” | 通过 |
| P3-16 | PageDown、PageUp、B、`.`、End、Home | 通过 |
| P3-17 | 减少动态效果：Frame→Viewport 的飞行变为淡入淡出 | 通过 |
| P3-18 | 断网（中继关闭且浏览器离线）：引导可用；开始对话框说明后台不可达，放映为“本地放映”，提示器链接按钮禁用，不是可操作放映，可以翻页 | 通过 |
| UI-M03 | Pixel 7 仿真：工具栏有“引导”没有“开始放映”，点按翻页和结束 | 通过 |
| 深色主题 | 路径编辑、开始对话框、舞台控制与备注、引导、提示器的截图核对 | 通过（截图见 `test-results/aiworkspace-presentation/`） |
| P3-19 | 性能探针，见 §5 | 通过 |

## 5. 性能

`probe.spec.ts` P3-19，生产构建（`vite preview`），无 GPU 光栅化的无头 Chromium，1600×1000。画布：1,000 个 Block（便签与外框）、20 个表格 Block（其中一半是条形图样本）、一个 1 万行表格 Block、12 个 HTML Block（其中 1 个可操作）、6 个富文本 Block；路径 8 步（4 个 Frame、3 个 Viewport，含远距离移动，最后回到第一步）。

| 指标 | 结果 |
| --- | --- |
| 开始放映（加锁、克隆、打开克隆的会话并读取大纲） | 487 ms |
| 每步到达（目标 Block 已挂载、没有“载入中”） | fly 408–1204 ms，即飞行本身的时长（0.4–1.2 s 按距离）；fade 165 ms |
| 飞行中的帧间隔 | p95 16.7–49.9 ms，最大 49.9 ms |
| 长任务（> 50 ms） | 0 |
| 结束时画布 DOM 节点 / JS 堆 | 3,002 / 87 MB |

后台克隆耗时见 §3。飞行中的帧时间在无 GPU 的无头环境下测得，偶有 33–50 ms 的帧；真实 GPU 上没有测量。结果写在 `test-results/aiworkspace-probe-show.json`。

## 6. 实测中发现并修正的问题

| 问题 | 修正 |
| --- | --- |
| 引导的变暗遮罩没有出现：观察尺寸的 effect 在引导图层挂载前就执行过了 | 改为回调 ref，图层出现后再观察；用例检查遮罩的挖空 |
| 从窗口派发的按键（目标不是元素）让舞台的按键处理抛错 | 舞台、引导、提示器的按键处理都先判断目标是否为元素 |
| 主菜单多了“使用引导”一项后，“开发工具”子菜单落在状态区提示的下面，点不到 | 浮动工具栏的菜单或弹出层打开时，整个工具栏层提到状态区之上 |
| 舞台刷新后用本标签页记下的 seq 和命令游标继续，可能落后于中继：新状态被当作旧的、已处理的命令再处理一次 | 继续时以中继返回的 seq、游标和当前步骤为准 |
| 引导取景铺满画布时，说明气泡盖住主工具栏 | 气泡依次尝试取景下方、上方、取景内底部 |
| 开始引导后，“这个作品带有使用引导”的提示仍然留着 | 引导开始时关闭这条提示 |
| 打开备注面板时，舞台状态说明被面板压住一半 | 面板打开时隐藏状态说明 |
| 路径面板中“应用”按钮被挤成竖排 | 按钮不换行 |

## 7. 未完成与风险

| 项目 | 说明 |
| --- | --- |
| named store 接入 | 未做（规划 18-9）。克隆不复制对象字节的目标已由服务级的内容寻址存储满足。 |
| DV／真实 Zone | 没有部署和验证。FORMAT 0.4 会让 DV 上已有的测试工作区无法打开，需要重建；部署会影响共享环境，需要先确认。提示器在真实 Zone 中的免登录路由、已登录大屏的背景画面、网关对 `/workspace/<id>/show/<show>` 的回退都只在开发环境验证。 |
| 真实设备 | 没有在真实手机、平板和双屏上验证；Window Management API（“放到其他屏幕”）没有实测。二维码没有做（需要新依赖）。 |
| 可操作 Block 的覆盖 | 端到端只覆盖了便签；HTML Block 激活运行、许愿格在舞台抽屉中运行与应用、媒体都只有代码路径，没有专门用例。 |
| 锁的感知延迟 | 其他窗口最迟在下一次长轮询（≤ 25 s）知道放映锁；在此之前的写入会被拒并立即提示。 |
| 已知风险（按规划接受） | 提示器链接即能力：链接泄露期间他人可以翻页、读备注。中继、锁和克隆都在内存中：服务重启即结束放映。讲解备注是 Frame／Viewport 的属性，能读该对象的人都能读到（只在提示器显示，分享导出时去掉）。HTML 扩展仍是同源运行。 |
| 舞台刷新 | 独立标签页已验证；Desktop 窗口内重新载入后的接回没有单独验证。 |
| 性能 | 只在无 GPU 的无头 Chromium 中测量。 |
