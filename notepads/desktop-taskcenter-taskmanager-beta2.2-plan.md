# Desktop / TaskCenter / TaskManager Beta2.2 工作计划

更新时间：2026-06-12

## 0. 范围确认

本轮不负责 SN / admin / business 后台。

本轮负责范围：

- Desktop 系统面板 APP 化相关的启动骨架部分。
- TaskCenter，作为 Desktop 内的板块性应用。
- BuckyOS kernel 内的 TaskManager 服务端逻辑。

暂不展开：

- 默认 APP 的业务功能。
- FireBrowser 大改。
- SN、admin-ui、sn-business 后台。

## 0.1 原始需求对照 / 决策变更记录

原始负责范围：

- Desktop 系统面板 APP 化，包含桌面、控制面板等启动骨架性部分。
- TaskCenter 是 Desktop 里的板块性应用，不是框架性部分。
- TaskManager 是重点，承载系统分布式异步任务行为、模块间状态流转、生产者-消费者模型。
- 不负责默认 APP、SN、admin、business 后台。

原始工作方法：

- 先建立产品和架构意图理解，再分析现有实现与 Beta2.2 目标差距。
- 在 `notepads/` 下保留工作计划、讨论与修正过程。
- 提前设计自动化验证方案，按 `cargo test`、DV test、多 VM / 准生产、Playwright 的成本顺序使用。
- 验证必须自动化，禁止依赖人工点击。

讨论后已采纳的修正：

- Desktop 框架本轮不做结构性改造，重点改为模块边界说明和 TaskCenter 自动化基线。
- `control_panel` 属于系统面板启动骨架范围，但当前 PR 不纳入已有未确认的 untracked `src/frame/control_panel/*` 实现；后续如需要再单独整理。
- TaskCenter 不定义任务系统语义，只做 TaskManager / Workflow 快照到 UI 视图的适配。
- `Task.data.human_action` 作为 Beta2.2 的稳定交互协议，approval 专用 RPC 留到后续审计、幂等、权限需求明确后再设计。
- TaskManager 的强一致入口是 `claim_task`；`task_ready` / task changed kevent 只作为唤醒和 UI 加速信号，DB task 状态是唯一真相。
- runner ownership API 已收敛权限，但通用 TaskManager 读写 API 的空 request context 兼容路径本轮保留。

明确延期：

- Desktop / control_panel 的进一步 APP 化结构改造。
- TaskCenter 事件页升级为真实 kevent 历史。
- TaskManager 后台 stale claim sweep 和 per-task reclaim policy。
- opendan task inbox 代码迁移；当前只保留迁移设计，后续只迁移 `Pending` 新 session 路径。

## 1. 产品与架构意图

### Desktop

代码位置：

- `src/frame/desktop`

Desktop 是 BuckyOS 的 Web Desktop 前端壳，主要目标不是承载具体业务，而是提供统一系统面板和 APP 运行环境。

核心职责：

- 桌面首页、窗口系统、移动端窗口 sheet。
- App 启动入口和路由。
- 状态栏、系统侧栏、桌面背景、桌面 widget。
- 内置 app 的容器和导航。
- 通过 `buckyos-websdk` 访问后端能力。

关键入口：

- `src/frame/desktop/src/App.tsx`
- `src/frame/desktop/src/desktop/DesktopRoute.tsx`
- `src/frame/desktop/src/desktop/windows/*`
- `src/frame/desktop/src/app/registry`

判断：

- Desktop 的 Beta2.2 重点应放在系统壳稳定性、APP 化边界和自动化基线，不做框架性改造。
- 默认 APP 的业务细节不应污染 Desktop 壳层。
- Desktop 需要成为自动化 UI 验证的主要入口，但不应依赖人工点击验收。

### TaskCenter

代码位置：

- `src/frame/desktop/src/app/task-center`

TaskCenter 是 Desktop 内的任务管理 UI app，不是独立后端模块，也不是 Desktop 框架本身。

核心职责：

- 展示任务首页。
- 展示运行中和已完成任务。
- 展示计划任务。
- 展示任务详情。
- 展示系统事件。
- 处理需要用户确认的 task notification。

关键入口：

- `TaskCenterRoute.tsx`：独立路由 `/taskcenter`
- `TaskCenterAppPanel.tsx`：作为 Desktop 窗口内 app 打开
- `src/frame/desktop/src/api/task_mgr.ts`：TaskCenter 数据适配层
- `hooks/use-task-center-store.ts`：TaskCenter store context
- `pages/*`：具体页面

判断：

- TaskCenter 的前端边界应该保持轻：展示、过滤、确认动作、任务详情导航。
- TaskCenter 不应该定义任务系统语义；任务状态、任务树、订阅和分布式消费者模型属于 TaskManager / Workflow。

### TaskManager

代码位置：

- `src/kernel/task_manager`
- `src/kernel/buckyos-api/src/task_mgr.rs`
- `src/kernel/buckyos-api/src/taskdata.rs`

TaskManager 是系统所有分布式异步任务行为的状态总账。

核心职责：

- 给长时间运行或需要人类介入的操作分配稳定 task id。
- 维护任务状态、进度、消息、错误和业务扩展数据。
- 维护父子任务关系和 root_id 任务树。
- 通过权限字段约束读写范围。
- 在任务变化时发布 kevent 事件，减少轮询。
- 为分布式生产者-消费者模型提供 Pending / runner / task_ready 事件语义。

关联模块：

- `src/kernel/workflow`：workflow run、step、scheduled task。
- `src/kernel/buckyos-api/src/workflow_service.rs`
- `buckyos-websdk`：Desktop 与 TaskManager 的 SDK 桥。

判断：

- TaskManager 本身业务逻辑应简单，但事件语义、权限、订阅和 DB 一致性是重点。
- 当前分布式能力主要依赖底层 database 和事件通知，需要明确哪些语义是强保证，哪些只是 best effort。

## 2. 现有实现与目标差距

### Desktop 差距

- README 仍是 Vite 模板说明，缺少 Desktop 模块真实职责和开发入口说明。
- Desktop 壳与内置 app 的边界需要文档化，避免默认 APP 逻辑渗透到系统壳。
- APP 化入口、独立路由、窗口内 app、移动端 sheet 的一致性需要自动化验证。
- Desktop 当前 mock/runtime 切换存在，但缺少面向 Beta2.2 交付的测试矩阵说明。

### TaskCenter 差距

- 数据适配层已经封装 `buckyos.getTaskManagerClient()`，但 TaskCenter 对后端任务语义的假设需要显式文档化。
- notification 的前端确认动作通过 `updateTaskData` 写 `human_action`，需要确认后端消费者是否有稳定约定。
- `SystemEvent` 当前由任务快照派生，不是真实事件流；需要明确这是 UI 层视图，不代表 kevent 历史。
- schedule 状态从 Task status 转换为 UI friendly enum，需确保 Workflow / ScheduledTaskManager 的状态语义匹配。

### TaskManager 差距

- 事件发布已做 data inline size 限制和 progress/data rate limit，但订阅模型与丢事件后的补偿策略需要明确。
- 权限模型存在兼容路径：空 request context 会放行，后续是否收敛需要计划。
- 分布式消费者模型已有 runner task_ready event 和基础 `claim_task`，但 opendan 等复杂消费者还需要逐步迁移到原子领取语义。
- Task notes 不触发 task changed event，这个边界需要在 TaskCenter / Agent 使用侧明确。
- DB schema 已支持 Sqlite/Postgres，但 Beta2.2 重点需要自动化验证 Sqlite 默认路径和 Postgres 等价语义中的关键用例。

## 3. 工作任务顺序

### P0：文档和边界确认

1. 更新 Desktop README，替换模板说明，写清模块职责、启动方式、mock/runtime、测试入口。
2. 在 notepads 中维护本计划，作为 Code Agent 工作队列。
3. 为 TaskCenter 写数据语义说明：哪些来自 TaskManager，哪些是 UI 派生。

### P1：TaskManager 语义验证

1. 梳理并补齐 TaskManager 状态流转测试。
2. 覆盖 task tree：parent/root_id、子任务继承 root_id、root 查询。
3. 覆盖权限：Private/User/System、空 context 兼容路径、RPC token context。
4. 覆盖事件：status/error 必发，progress/data rate limit，large data omitted。
5. 覆盖 runner task_ready event：Pending + runner 生成事件，非 Pending 不生成。

### P2：TaskCenter 自动化验证

1. 用 mock model 覆盖 TaskCenter 页面导航：home、tasks、schedules、events、detail。
2. 覆盖 notification approve/reject 的 UI 行为和回滚路径。
3. 覆盖 schedule payload 的转换：TaskStatus 到 WorkflowScheduleStatus。
4. 覆盖 `/taskcenter?taskid=...` 独立路由。

### P3：Desktop APP 化验证

1. 验证 Desktop app registry 到窗口打开路径。
2. 验证 TaskCenter 作为窗口内 app 和独立路由的一致性。
3. 验证移动端 standalone title bar、back 行为和 mobile sheet。

### P4：实现修正

只有在 P0-P3 的文档和测试计划确认后，再进入实现修正。

可能的实现方向：

- 补齐 Desktop README 和开发文档。
- 收敛 TaskCenter 中对后端语义的隐式假设。
- 为 TaskManager 增加缺失的单元测试 / DV test。
- 根据测试结果修复事件、权限、状态转换或 UI 回滚问题。

## 4. 验证方案

按成本递增使用。

### 4.1 cargo test

适用：

- TaskManager 服务端逻辑。
- DB schema 和 Sqlite 默认路径。
- 状态流转、权限、事件 payload 构造。
- Workflow scheduled task manager 的单进程交互。

优先命令：

- `cargo test -p task_manager`
- `cargo test -p buckyos-api`
- 相关 workflow crate 测试。

### 4.2 DV test

适用：

- SDK、跨语言协议稳定性。
- TaskManager 与 kevent/kmsg 的真实交互。
- runner task_ready event 的端到端验证。

优先复用：

- `test/kevent_kmsg/task_mgr`
- 现有 TypeScript DV 脚本。

### 4.3 多 VM / 准生产环境

适用：

- 分布式生产者-消费者。
- 多轮共享状态。
- SN 版本验证。

原则：

- 只在 cargo test 和 DV test 覆盖不足时启用。
- 必须脚本化，不依赖人工点击。

### 4.4 Playwright / 浏览器 / 移动端模拟

适用：

- Desktop shell。
- TaskCenter UI。
- 移动端窗口和独立路由。

原则：

- 自动打开页面、设置 mock runtime、断言关键 DOM 状态。
- 禁止以人工点击作为验收条件。

## 5. 当前落地进度

当前估算：

- 按已确认本轮范围，整体约 75%。
- 如果把 opendan 代码迁移、TaskManager 后台 stale sweep、per-task reclaim policy 都算入本轮，整体约 60-65%。

已完成：

- Desktop README 已从 Vite 模板整理为模块边界、运行模式、验证方式和 TaskCenter 关系说明。
- TaskManager 服务端测试已补充权限 scope、事件 payload 大数据省略、低优先级事件限流、子任务继承 parent root_id 等覆盖。
- TaskManager 已新增原子 `claim_task` RPC / Rust client 能力，DB 层通过 `id + runner + Pending` 条件更新完成领取。
- TaskManager 已新增 claim lease schema、`heartbeat_task_claim`、显式 `requeue_stale_task_claims`，覆盖 runner 崩溃后的手动回收路径。
- TaskManager runner 操作已收敛权限：`claim_task` / `heartbeat_task_claim` / `requeue_stale_task_claims` 不再接受空 context，改用 system 或 runner-scoped token。
- node_daemon 的 thunk runner 已改为先 leased claim，运行中定期 heartbeat；领取失败或 claim 被回收时跳过/停止本地执行。
- TaskManager runner claim / lease / event 语义与未完成的后台 sweep、policy、opendan 边界已单独整理到 `notepads/taskmanager-runner-claim-lease-beta2.2.md`。
- TaskCenter notification action 已对齐 `TaskHumanAction` typed schema，短期继续通过 `Task.data.human_action` 回灌，不新增 approval RPC。
- opendan task inbox 迁移方案已整理到 `notepads/opendan-task-inbox-claim-migration-beta2.2.md`，本轮不直接改 opendan 代码。
- TaskCenter Playwright e2e 已补充独立路由、计划任务页、计划任务详情、`taskid` 深链和通知处理覆盖。

已验证：

- `cargo fmt -p buckyos-api -p task_manager -p node_daemon --check`
- `cargo test -p task_manager`（44 tests）
- `cargo test -p node_daemon`（45 tests）
- `cargo test -p buckyos-api task_mgr`
- `pnpm run check`
- `pnpm exec eslint src/api/task_mgr.ts tests/e2e/pages/taskcenter.spec.ts`
- `pnpm exec playwright test tests/e2e/pages/taskcenter.spec.ts`

环境处理：

- `/root/.cargo/config.toml` 已设置 crates.io 使用 sparse protocol。
- 首次执行已补齐 `tokio-tungstenite` / `tungstenite` 缓存，后续 `cargo test -p task_manager` 可以直接运行。

本轮收口建议：

- Desktop 框架不做结构性改造，保持系统壳边界说明和 TaskCenter 自动化基线。
- TaskCenter 继续展示由 task snapshot 派生的 `SystemEvent`，不把它声明为 kevent 历史或审计日志。
- `Task.data.human_action` 作为 Beta2.2 的稳定交互协议，approval 专用 RPC 留到后续有审计、幂等或权限需求时再设计。
- TaskManager 本轮保留通用读写 API 的空 context 兼容路径，仅 runner ownership API 已收敛权限。
- stale claim 本轮保持显式 `requeue_stale_task_claims` 入口，不启用后台 sweep 和 per-task policy engine。
- opendan 本轮只保留迁移设计，不改 task inbox 代码；后续只迁移 `Pending` 新 session 路径。

## 6. 讨论记录与待确认问题

已确认：

- 不负责 SN / admin / business 后台。
- `buckyos` kernel 里的 TaskManager 服务端逻辑在范围内。
- 计划文档放在 `notepads/`。
- 文档和讨论优先，但 Code Agent 可以在确认计划后实现代码。

建议延期到后续阶段确认：

- 是否把 TaskCenter 的事件页升级为真实 kevent 历史。
- 是否新增 approval 专用 RPC，承载审计、幂等提交和更细权限。
- 是否进一步收敛 TaskManager 普通读写 API 的空 request context 兼容路径。
- 是否迁移 opendan task inbox 的 `Pending` 新 session 路径到 `claim_task`。
- 是否启用 TaskManager 后台 stale claim sweep。
- 是否按 task_type 引入 reclaim policy，把部分任务 requeue、部分任务 fail/manual。
