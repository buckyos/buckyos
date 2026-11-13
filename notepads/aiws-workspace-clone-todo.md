# AIWS 工作区 Clone TODO

状态：待实施。

日期：2026-11-13。源码基线：buckyos `aee37a21a`。

来源：[权限系统整理需求](aiws-permission-system-prd.md) Q4 的 owner 裁决：“现在的 fork 估计要改名叫 clone 了……named store 已经接住了大头，没必要在逻辑数据这里坚持 Copy-On-Write”。

## 0. 结论

现在的 `ws.fork` 已经是物化复制，不是写时复制：

- 它先把源工作区物化成一份快照，再装入成一个新工作区。
- 不可变对象在 named store 里共享，逻辑数据（`doc.sqlite`）是独立的一份。
- 新旧工作区之间没有同步、合并或上下游关系，只记了一条来源。

所以这个 TODO 不改复制的方式，只做三件事：

1. 改名。
2. 把语义写清楚。
3. 按权限需求带上权限设计。

## 1. 现状

`ws.fork`（stable），代码在 [store/src/service.rs](../src/frame/aiworkspace/store/src/service.rs) `fork`：

| 环节 | 现在的实现 |
| --- | --- |
| 前提 | 调用者可以拥有工作区（不受限账号或服务），并且在源工作区有工作区级 `export` |
| 物化 | `materialize_for(&access, "fork_source", …)`：只物化调用者读得到的内容；读不到的实体在返回的 `excluded_entities` 里列出 |
| 装入 | `load_workspace`：新的工作区 ID，调用者成为 owner；不带个人范围的实体，协作状态从头开始 |
| 来源 | 返回 `forked_from: { workspace_id, snapshot }`，同样写进快照根的 `lineage`（[core/src/materialize.rs](../src/frame/aiworkspace/core/src/materialize.rs) `snapshot_root`） |
| 不带的东西 | `local.sqlite` 里的一切：授权、分享入口、发布、运行、上传、托管配置；工作区的名字（内核文档 02） |

`ws.import new` 和 `ext.create_from_template` 走同一个 `load_workspace`。

“clone”这个词在 AIWS 里已经有三种用法，改名后要在文档里分清：

| 说法 | 是什么 |
| --- | --- |
| `tree.clone` / `entity.clone` | 工作区内复制一棵子图的 Commit 操作 |
| 放映克隆（`clone_for_show`） | 放映时冻结的只读基线 |
| 会话的运行工作区（`clone_for_session`） | 演示会话复制出的工作区 |

## 2. 要做的

- [ ] **C1 改名。**
  - `ws.fork` 改为 `ws.clone`，不保留旧名（允许 breaking change）。
  - 返回的 `forked_from` 改为 `cloned_from`；快照根的 `lineage.forked_from` 改为 `lineage.cloned_from`。
  - 物化种类 `fork_source` 改为 `clone_source`。
- [ ] **C2 联动。**
  - 服务：[server/src/lib.rs](../src/frame/aiworkspace/server/src/lib.rs) 的方法表与分派；README 的方法表。
  - 内核文档：01（“`new` 创建 Fork”）、02（内部引用、名字）、09（“模板、Fork 与业务实例”一节）、13（决策索引）。
  - Desktop：`api/client.ts` 的 `wsFork`、`api/types.ts` 的 `forked_from`、`ui/WorkspaceList.tsx`、`ui/shell/MainMenu.tsx`、`i18n/aiworkspace/shell.ts` 的文案。本版本不调整 UI，这里只随改名同步名字和文案，不改交互。
  - 手册：`aiworkspace_template_dev` 里“`ws.fork` for a copy inside this Zone”一句。
  - 测试：store `tests/{sso,names,packages,guest}.rs`，server `tests/{contract,quality,process,sso}.rs`，Desktop `tests/aiworkspace/features.spec.ts`、`tests/dv/aiworkspace.integrity.spec.ts`（测试 ID `aiws-fork`）。
- [ ] **C3 带上权限设计（权限需求 R12）。**
  - 有源工作区 `manage` 的人 Clone 时，带上权限设计：逻辑组的定义、授给逻辑组和固定组的条目（允许与拒绝）、宏的组绑定。
  - 不带：组成员、授给单个调用者的条目、公共匿名访问开关、分享入口。
  - 只有 `export` 的人 Clone，不带权限设计。
  - 条目指向的实体没有被复制过来（读不到而被排除）时，丢弃该条目，在返回里报告。
  - 依赖权限需求第 1 步（逻辑组模型）。
- [ ] **C4 创建者。** 副本里实体的创建者（`created_by`）是发起 Clone 的人（权限需求 R6）。
- [ ] **C5 来源记录。** 保留 `cloned_from` 作为出处，不建立上下游关系：没有同步、合并，来源被删除不影响副本。
- [ ] **C6 新建导入。** `ws.import new` 按同一规则处理权限设计（权限需求 R12）。
- [ ] **C7 额度。** 核对副本的逻辑占用是否按完整内容计入新工作区。内核文档 11 规定“资源归属独立于底层是否去重”，即使对象在 named store 里共享，新工作区也按自己的逻辑占用计算。

## 3. 待确认

| # | 问题 | 建议 |
| --- | --- | --- |
| Q1 | 方法名用 `ws.clone` 还是 `ws.copy` | `ws.clone`，与 owner 的说法一致；界面文案用“复制为新工作区” |
> 用ws.clone
| Q2 | 是否保留来源记录 | 保留，只作出处（C5） |
> 保留

## 4. 验收

- `ws.fork` 已不存在；`ws.clone` 的行为与原来的 fork 相同，返回 `cloned_from`。
- 有 `manage` 和只有 `export` 两种情况下，副本带与不带权限设计（C3）。
- 文档和手册里不再有工作区级的“Fork”；`tree.clone`、放映克隆、会话工作区的说法与工作区 Clone 区分清楚。
- `cargo test -p aiworkspace-store -p aiworkspace -- --test-threads=1` 通过；Desktop 相关 e2e 改名后通过。

## 5. 实施回写

- 待实施。
