# BuckyOS AI Workspace 第二期实施记录与验收报告

> 对应：[《第二期规划》](<BuckyOS AI Workspace 第二期规划.md>) v0.2.1（下称“规划”），实现与规划的差异已回写到规划 §16。
>
> 读者：审查第二期交付、或在其上继续开发的人。
>
> 本文只记录**已经运行过的事实**：每一项结论后面是产生它的命令或测试。没有运行过的，写在 §7“未完成与未验证”；本期接受而不处理的风险在 §8。
>
> §1–§6 保留初版验收记录；后续 Block Review 修复与回归结果见 §9。

## 1. 结论

- **M0–M4 全部实施**：两棵树与 Surface 结构、Block 定义与校验边界、许愿格两遍执行与依赖记录、版本历史与回滚、用户工作状态、HTML Block JS API、增量结构更新、渲染探针（M0）；数据源模式与画布壳（M1）；自由画布核心（M2）；声明式/HTML 扩展与许愿格 UI（M3）；两个 demo 与探针记录（M4）。
- **后台**：第一期回归与第二期新增测试共 **75 个通过，0 失败**（另 1 个性能探测默认忽略），见 §3。
- **Desktop 应用**：**51 个 Playwright 用例对真实后台进程全部通过**（第一期 34 个按新 UI 改写后继续通过，第二期新增 17 个），见 §4。
- **性能门槛已冻结并在生产构建上满足**（1,000 Block：首个可操作视图 0.43 s，平移/缩放 P95 16.8 ms，多选拖动 P95 16.8 ms，无 ≥ 200 ms 阻塞），5,000 规模已记录，见 §5。
- 规划 §13 列出的七项门槛逐项对照见 §6。**没有在真实 Zone 中运行**；播放编辑只有占位；执行器只有 Mock。

## 2. 交付物

| 交付物 | 位置 |
| --- | --- |
| 内核：两棵树、`placement` 去 z、`buckyos.wish` / `buckyos.block-def`、依赖记录、鲜度、系统节点保护 | `src/frame/aiworkspace/core/src/{model,plan,types,freshness,read,materialize,replica,anchor}.rs` |
| 存储与服务：schema v2、`entity_versions` 版本登记、用户工作状态、授权列表可见性、主体列表 | `src/frame/aiworkspace/store/src/{schema,docdb,workspace,reads,export,proc}.rs`、`server/src/{lib,auth}.rs` |
| 协议目录与样例 | `schemas/operations.json`（新增 `entity.set_derived`）、`fixtures/project-workspace/commits.json`（17 个实体的两棵树样例） |
| WASM 门面（大纲、鲜度、关系在浏览器副本中由同一份 core 计算） | `src/frame/aiworkspace/wasm/src/lib.rs` → `src/frame/desktop/src/app/aiworkspace/wasm/` |
| Desktop：增量大纲模型、用户工作状态、鲜度服务、Block 注册表与宿主、渲染宿主、许愿格服务、扩展运行时、数据源模式、画布壳 | `src/frame/desktop/src/app/aiworkspace/{state,ui/blocks,ui/canvas,ui/wish,ui/extensions,ui/sources,ui/shell,api/demos.ts}` |
| 两个正式 demo（季度经营分析、短片预演） | `src/frame/desktop/src/app/aiworkspace/api/demos.ts`，工作区列表中的“季度经营分析 demo”“短片预演 demo” |
| 端到端用例 | `src/frame/desktop/tests/aiworkspace/{canvas,wish,permissions,probe}.spec.ts`，第一期用例改写 |
| 文档回写 | 规划 §16、后端 `README.md`、Desktop `src/app/aiworkspace/README.md`、《分层架构》“计算归属”行 |

## 3. 后台验收

全部命令在 `buckyos/src` 下执行（2026-10-06）。

```bash
cargo test -p aiworkspace-core -p aiworkspace-store -p aiworkspace
```

| 测试程序 | 通过 |
| --- | --- |
| `aiworkspace-core`：单测 19、`anchor` 7、`phase2` 5、`planner` 7、`schemas` 3、`vectors` 1 | 42 |
| `aiworkspace-store`：`collab` 6、`kernel` 12、`packages` 6、`phase2` 4、`replica` 3（`perf` 1 个默认忽略） | 31 |
| `aiworkspace`（服务进程）：`process` 2 | 2 |

第二期新增测试覆盖的断言：

| 测试 | 断言 |
| --- | --- |
| `core/tests/phase2.rs` | 系统节点不能创建/删除/改名/移动；`data` 树只收数据类型，`surfaces` 只收 Surface，Surface/分组只收 Block 与分组；`placement` 不接受 `z`；`buckyos.cell` 的 `view.type` 可以是任意渲染器 id，非内置渲染器需 `source_ref` 或 `def_ref`，`config` 超 64 KiB 拒绝；`buckyos.wish` / `buckyos.block-def` 校验；`entity.set_derived` 产生 `derived` / `produced` / `input` 引用且不阻止删除；鲜度 `current → stale → upstream_stale`；导入时依赖记录按包内版本重定基，导出前已过期的结果导入后仍过期 |
| `store/tests/phase2.rs` | 创建 Workspace 即有 `root` / `data` / `canvas-content` / `surfaces`；每次 `set_derived` 登记一个可寻址版本，`doc.list_versions` / `doc.restore_version` 回滚为普通 Commit；`ws.list_grants` 对非管理者只返回自己的行；删除预检只列出调用者可读的引用方并以 `hidden_referrers` 标出其余；用户工作状态按主体隔离、不进历史 |
| 第一期用例的改写 | 原 `page-main` 流式页改为 `surface-main` Surface + `surface-main-content` 内容区；大纲实体数 13 → 17；路径 `/data/任务` |

## 4. Desktop 验收

在 `buckyos/src/frame/desktop` 下执行（2026-10-06）：

```bash
pnpm check
pnpm exec eslint src/app/aiworkspace tests/aiworkspace
pnpm build
PATH=/tmp/dev-cache-root/cargo/bin:$PATH AIWS_BIN=<cargo 构建的 aiworkspace> \
  pnpm exec playwright test --config=playwright.aiworkspace.config.ts
```

结果：类型检查与 ESLint 无错误；生产构建成功；**51 passed (5.3m)**，Chromium，独立后台进程 + Desktop 壳的模拟运行时。

规划 §13 验收矩阵对照（UI 编号沿用规划）：

| 编号 | 用例 | 文件 |
| --- | --- | --- |
| UI01 | 打开 demo：默认画布模式、Surface 列表、模式/子模式状态是用户工作状态（刷新后保持，不进历史） | `canvas.spec.ts` |
| UI02 / UI03 | 数据源模式三栏；无 Block 的数据可直接编辑；同一数据在两张 Surface 上的三个 Block 同步 | `canvas.spec.ts` |
| UI04 | 新建 Surface 自带内容区；Block 跨 Surface 移动保持 id 与绑定；删除 Surface 先预检；被引用数据拒绝删除且只列出可见引用方 | `canvas.spec.ts` |
| UI05 | 拖动 = 一次提交一个撤销步，Esc 取消零提交；手柄缩放一次提交；缩放不写文档 | `canvas.spec.ts` |
| UI06 / UI07 | 布局并发自动合并，“位置已被修改”通知与重新应用；插入 = 数据 + Block 一次提交 | `canvas.spec.ts` |
| UI08 / UI09 | 编辑/查看子模式分派渲染、工具与写入；未知渲染器、类型不符本地回退 | `canvas.spec.ts` |
| UI10 | 数据被删与渲染器抛错都局部化，画布仍可编辑 | `canvas.spec.ts` |
| UI11 / UI20 | 分析 → 执行 → 应用：一次可撤销提交写入结果、Block 与依赖记录；只有输入变化才过期；重开后状态来自持久记录 | `wish.spec.ts` |
| UI12 / UI13 | 失败保留旧结果；人工修改被保护（保留/替换/新建）；版本回滚；新建方式产生可比较的结果组；成环被拒 | `wish.spec.ts` |
| UI14 | 预设角色、画布级授权、并集语义；受限主体看不到名单、引用方与依赖细节 | `permissions.spec.ts` |
| UI16 | 导出/导入携带 Surface、Block、定义、许愿格与依赖记录，过期状态随包 | `wish.spec.ts` |
| UI17 | 两个 demo 经正常 UI 打开，所有 Block 由注册表渲染 | `canvas.spec.ts` |
| UI19 | HTML 扩展 Block 通过 `window.aiws` 读数据、经 store 提交（可撤销）、上传；崩溃本地回退 | `wish.spec.ts` |
| UI20 | 两个客户端应用同一许愿格只接受一个 | `wish.spec.ts` |
| UI21 | 渲染探针（§5） | `probe.spec.ts` |
| 第一期 V 系列 | 34 个用例按新 UI 改写后继续通过（富文本、锚点、锁、离线、包） | `basics / collab / features / annotations / offline*.spec.ts` |

实施中由用例暴露并修掉的缺陷（都有对应用例守住）：画布容器被浏览器滚动导致 DOM 与空间索引错位（改 `overflow: clip`）；Esc 取消拖动后残留文字选区使下一次拖动被原生拖放取消；快速手势结束前未处理的移动被当成点击；自由便签（无 target 的批注）让整个 Desktop 崩溃；记录 Block 的编辑器没有写锁条；属性面板丢失 `def_ref` 使声明式 Inspector 为空；画布内静态表格没有受限高度（1 万行全部渲染）。

## 5. 渲染探针与冻结的性能门槛

探针 `tests/aiworkspace/probe.spec.ts` 按规划 §9.5 在真实 RenderHost 上测量，对 **`vite preview` 的生产构建**运行（同一后台），结果写入 `test-results/aiworkspace-probe-{1000,5000,multi}.json`。环境：headless Chromium 151（无 GPU 光栅化），视口 1600×1000，2026-10-06。

**冻结的门槛（1,000 Block 规模）**：首个可操作视图 < 2 s；平移/缩放帧耗时 P95 ≤ 16.7 ms；多选拖动 P95 ≤ 32 ms；交互期间无 ≥ 200 ms 的连续主线程阻塞。自动化门禁对帧耗时取门槛的 2 倍（headless 无 GPU），实测值如下。

| 指标 | 1,000 Block（含 1 万行表、300 个表格 Block） | 5,000 Block | 门槛 |
| --- | --- | --- | --- |
| 首个可操作视图（从打开 Workspace 起） | 430 ms | 1,502 ms（记录） | < 2,000 ms |
| 连续平移帧耗时 P50 / P95 / 最大 | 16.7 / 16.8 / 33.4 ms，长任务 0 | 16.7 / 33.3 / 33.4 ms，长任务 0 | P95 ≤ 16.7 ms |
| 跨 LOD 阈值缩放 P50 / P95 / 最大 | 16.7 / 16.8 / 16.8 ms | 16.7 / 16.7 / 16.8 ms | P95 ≤ 16.7 ms |
| 框选后拖动 P50 / P95 / 最大 | 329 个选中：16.7 / 16.8 / 100 ms | 1,765 个选中：16.7 / 33.4 / 600 ms | P95 ≤ 32 ms |
| 挂载 Block / 占位 / 隐藏（打开时，缩放 1） | 52 / 0 / 149 | 51 / 0 / 149 | 远小于总数 |
| 适应全部后 挂载 + 占位 | 0 + 400 | 0 + 400 | ≤ 450（MAX_MOUNTED 400） |
| 挂载的编辑器 / HTML 运行时 | 0 / 0 | 0 / 0 | 受预算约束 |
| DOM 节点（打开 / 结束） | 2,492 / 5,613 | 2,383 / 19,239 | 有界 |
| 堆（MB） | 18 | 28 | 记录 |
| 手势期间网络写入 | 拖动只产生 1 次提交 | 同左 | 1 |
| 三张 1,000 Block Surface 切换（ms） | 272 / 184 / 180 / 202 | — | < 2,000 |

探针确认的规则（规划 §9.3）：相机是 CSS 变换，不是 React 状态；拖动期间不做 React 渲染（选区轮廓随 SVG 组变换，大选区只隐藏一次再恢复，Block 宿主已 memo）；三级裁剪带滞回；LOD 占位；编辑器与 HTML 运行时按需挂载并有预算。§9.4 的 Canvas 2D 退路本期不需要启用。5,000 规模的拖动最大值 600 ms 来自一次性处理 1,765 个选中项的提交，未触发退路。

## 6. 规划 §1 判据对照

| 判据 | 状态 | 证据 |
| --- | --- | --- |
| 同一数据能在数据源模式中编辑，在不同 Surface 上以不同 Block 展现；数据树与 BlockTree 分开管理 | 满足 | UI02/UI03/UI04；`core/tests/phase2.rs` 子类型规则 |
| 编辑与查看子模式驱动各自的渲染、交互与工具策略；播放编辑占位可进入且不写文档 | 满足（播放编辑只有占位） | UI08/UI09；`canvas.spec.ts` 中播放编辑占位用例 |
| 数据变化、引用失效、权限、离线和撤销在两种界面中保持同一语义；新增 Block 类型只需注册扩展 | 满足 | UI10/UI14、第一期离线用例、`samples.tsx` 的三个样例渲染器与两个文档内定义 |
| 许愿格以两遍执行产生结果，并能显示上游变化 | 满足（Mock 执行器） | UI11–UI13、UI20 |
| 画布在约定规模下满足 M0 冻结的性能门槛 | 满足 | §5 |
| 用旧 ai-canvas 的两个 demo 验证边界 | 满足 | UI17；`api/demos.ts` |
| 回写 README、验收报告、分层架构文档 | 满足 | 本文；规划 §16；两份 README；《分层架构》“计算归属” |

## 7. 未完成与未验证

**只做契约预留 / 占位**

- 播放模式、播放编辑：只有入口与“尚未实现”占位，“开始演示”禁用；不注册编排专用的 Renderer、动作或 Inspector。
- 执行器：只有 Mock（`def-mock-wish` 文档实体，提示词 `#fail` / `#invalid` / `#slow` 触发失败路径）；xllm、agent-work-session 执行器未接。
- 连接器、Notion 式布局容器与画布模板：未实现。
- `ws.list_subjects` 系统模式读 control-panel 用户列表，Agent 主体未列出。

**未验证**

- 真实 Zone / DV：所有验证都在独立后台进程 + Desktop 壳模拟运行时下完成；系统模式传输、真实 SSO、kevent 唤醒未运行。
- 只在 Chromium 上验证；探针数据来自无 GPU 的 headless 环境，桌面浏览器的绝对值会不同。
- 浏览器离线副本对第二期新接口（版本、用户工作状态、主体列表）只做在线转发，离线时这些面板显示“需要后台”，没有离线用例。

**明确延后**

- 表格类结果的版本恢复（只有富文本、记录、资产）；从数据树拖到画布（用“添加已有数据…”代替）；多用户光标；坐标型评论钉；扩展市场或 AI 生成扩展的流水线（HTML 定义手工编写）；流式 Surface 的拖放排序。
- 版本行的保留期限与随包导出（规划 §14.5）：未设期限，不随包。

## 8. 已知风险（规划 §15，本期接受）

| 编号 | 风险 | 现状 |
| --- | --- | --- |
| R1 | 非 Owner 引入的 HTML 定义（有编辑权限的 Zone 用户添加的，或导入包携带的）在 Owner 打开时以 Owner 的会话运行，`window.aiws` 可读写 Owner 可读写的一切 | 按 D16 不设沙盒、不做启用确认；UI19 验证的是 API 与回退，不是隔离 |

另记两项实现层面的已知弱点：`doc.restore_version` 的回滚对富文本按块级差分生成操作，长文档一次回滚可能是很多块操作；`kept_manual` 结果的内容没有重新生成，鲜度“最新”表示用户的决定而不是内容与输入一致。

## 9. Block Review 修复

本次修复覆盖 Review 提出的七项问题：

| 问题 | 修复 | 回归证据 |
| --- | --- | --- |
| HTML 快照只有配置中的 object id，未纳入资产链路 | `config.snapshot` 保留 `{ object_id, media_type, size }`，通过已有资产校验归一化并建立 Cell → asset 引用；物化、导出闭包、权限、离线 bootstrap、保留与撤销都处理该引用 | `store/tests/packages.rs` 的 `block_snapshots_are_assets_in_grants_packages_and_replicas`；`blocks.spec.ts` 协作者读取及包导入；`offline.spec.ts` 快照断网冷启动 |
| 富文本嵌入每层从深度 0 开始 | `RenderContext.depth` 逐层传递，超过 3 层停止展开 | `blocks.spec.ts` 六层嵌入只展开到上限 |
| 自定义 Inspector 抛错导致工作区崩溃 | 属性面板使用现有 `BlockBoundary`，错误局限在面板内，可重试 | `blocks.spec.ts` 实际注册抛错 Inspector，并继续操作其他 Block |
| HTML 启动超时后原 Promise 不结束 | 超时、崩溃和 dispose 都拒绝原始 mount 与等待中的请求、释放 iframe；重试创建新 runtime；许愿执行器同步管理生命周期 | `blocks.spec.ts` 启动超时、就绪前卸载、崩溃后重新运行 |
| 文档内 BlockDef 约束被通用 HTML/声明式注册覆盖 | 解析 `def_ref` 后按 kind、accepts、allow_no_source、config_schema、HTML api_version 校验；不满足时通用只读回退 | `blocks.spec.ts` 五组不合法定义/绑定用例；原声明式 demo 用例 |
| 就近动作拿到伪造的不完整上下文 | 提取 `useBlockContext`，宿主、Inspector 和动作共享真实 payload、keyRevs、定义与模式/权限状态 | `blocks.spec.ts` “打开定义”、携带配置及 expect rev 的动作、查看模式隐藏写动作 |
| 注册表更新没有使已挂载 Block 重新解析 | 注册表 revision 纳入解析依赖与错误边界重置条件 | `blocks.spec.ts` 注销后回退、重新注册后恢复 |

`config_schema` 使用现有 Zod 的 `fromJSONSchema`，依赖下限更新为 4.4.3；只支持该转换器支持的 JSON Schema 子集，转换失败会显示 `invalid_definition`。快照是宿主保留字段，不参与扩展配置 Schema 校验。后台操作协议与 SQLite 表结构没有增加；已重建 WASM 产物，使浏览器副本使用同一份资产引用逻辑。共享 Rust 客户端和 Web SDK 未定义 Cell 配置结构，无对应类型变更。

原 UI10 测试改为实际注册抛错 Renderer；HTML API 测试补齐 `aiws.ready()` 握手；协作测试使用源码的调试接口类型。锁的获取和续约继续由既有编辑器负责，保留未持锁时的“开始编辑”入口。

本轮实际验证结果：

| 检查 | 结果 |
| --- | --- |
| `cargo test -p aiworkspace-core -p aiworkspace-store -p aiworkspace` | 76 passed，0 failed，1 个已有性能探测 ignored |
| `PATH=/tmp/dev-cache-root/cargo/bin:$PATH bash frame/aiworkspace/wasm/build.sh`（在 `src/`） | 重建成功 |
| `pnpm check`、`pnpm exec eslint src/app/aiworkspace tests/aiworkspace`、`pnpm build` | 全部成功；Vite 保留大 chunk 提示 |
| 全套 `pnpm exec playwright test --config=playwright.aiworkspace.config.ts` | 66 项中 64 passed；两个写锁入口用例失败，原因是统一只读判断隐藏了编辑器的取得写锁入口；1,000 / 5,000 Block 和多 Surface 探针均通过 |
| 修复锁入口后的针对性复测（下列命令） | **17 passed (1.4m)**：全部 14 项 Block 回归、上述两项失败用例、快照离线冷启动 |

```bash
# 在 src/frame/desktop
pnpm exec playwright test --config=playwright.aiworkspace.config.ts \
  blocks.spec.ts features.spec.ts:17 offline.spec.ts:5 offline.spec.ts:189
```

验证环境仍为独立 aiworkspace 后台 + Desktop 壳模拟运行时、Chromium；未运行完整 `buckyos-build.py`、全 Rust workspace 测试或真实 Zone / DV。

## 修订记录

| 日期 | 内容 |
| --- | --- |
| 2026-10-06 | 初版：M0–M4 实施记录、后台与 Desktop 验收结果、冻结的性能门槛与探针数据、未完成项与风险 |
| 2026-10-06 | Block Review 七项修复、扩展契约与快照资产说明、针对性回归与原测试修正 |
