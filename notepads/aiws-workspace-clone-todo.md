# AIWS 工作区 Clone TODO

状态：C1、C2、C5、C7 已实施（2026-11-13）；C3、C4、C6 随权限需求第 1 步一起实施（2026-11-13，见 [权限实施 TODO](aiws-permission-system-todo.md) §3.1、§3.3）。实施记录见 §5。

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

- [x] **C1 改名。**
  - `ws.fork` 改为 `ws.clone`，不保留旧名（允许 breaking change）。
  - 返回的 `forked_from` 改为 `cloned_from`；快照根的 `lineage.forked_from` 改为 `lineage.cloned_from`。
  - 物化种类 `fork_source` 改为 `clone_source`。
- [x] **C2 联动。**
  - 服务：[server/src/lib.rs](../src/frame/aiworkspace/server/src/lib.rs) 的方法表与分派；README 的方法表。
  - 内核文档：01（“`new` 创建 Fork”）、02（内部引用、名字）、09（“模板、Fork 与业务实例”一节）、13（决策索引）。
  - Desktop：`api/client.ts` 的 `wsFork`、`api/types.ts` 的 `forked_from`、`ui/WorkspaceList.tsx`、`ui/shell/MainMenu.tsx`、`i18n/aiworkspace/shell.ts` 的文案。本版本不调整 UI，这里只随改名同步名字和文案，不改交互。
  - 手册：`aiworkspace_template_dev` 里“`ws.fork` for a copy inside this Zone”一句。
  - 测试：store `tests/{sso,names,packages,guest}.rs`，server `tests/{contract,quality,process,sso}.rs`，Desktop `tests/aiworkspace/features.spec.ts`、`tests/dv/aiworkspace.integrity.spec.ts`（测试 ID `aiws-fork`）。
- [x] **C3 带上权限设计（权限需求 R12）。** 有 `manage` 时 Clone 带权限设计，被排除实体上的条目丢弃并在返回的 `permission_design.dropped` 里报告。
  - 有源工作区 `manage` 的人 Clone 时，带上权限设计：逻辑组的定义、授给逻辑组和固定组的条目（允许与拒绝）、宏的组绑定。
  - 不带：组成员、授给单个调用者的条目、公共匿名访问开关、分享入口。
  - 只有 `export` 的人 Clone，不带权限设计。
  - 条目指向的实体没有被复制过来（读不到而被排除）时，丢弃该条目，在返回里报告。
  - 依赖权限需求第 1 步（逻辑组模型）。
- [x] **C4 创建者。** 副本里实体的创建者（`created_by`）是发起 Clone 的人（权限需求 R6），由导入 Commit 的计划器记下。
- [x] **C5 来源记录。** 保留 `cloned_from` 作为出处，不建立上下游关系：没有同步、合并，来源被删除不影响副本。
- [x] **C6 新建导入。** 管理者导出的包在 manifest 带 `permission_design`，`ws.import new`（以及原工作区已不存在的 `restore`）按同一规则装入它（权限需求 R12）。
- [x] **C7 额度。** 核对副本的逻辑占用是否按完整内容计入新工作区。内核文档 11 规定“资源归属独立于底层是否去重”，即使对象在 named store 里共享，新工作区也按自己的逻辑占用计算。

## 3. 已确认（owner 2026-11-13）

| # | 问题 | 结论 |
| --- | --- | --- |
| Q1 | 方法名用 `ws.clone` 还是 `ws.copy` | `ws.clone` |
| Q2 | 是否保留来源记录 | 保留，只作出处（C5） |

## 4. 验收

- `ws.fork` 已不存在；`ws.clone` 的行为与原来的 fork 相同，返回 `cloned_from`。
- 有 `manage` 和只有 `export` 两种情况下，副本带与不带权限设计（C3）。
- 文档和手册里不再有工作区级的“Fork”；`tree.clone`、放映克隆、会话工作区的说法与工作区 Clone 区分清楚。
- `cargo test -p aiworkspace-store -p aiworkspace -- --test-threads=1` 通过；Desktop 相关 e2e 改名后通过。

## 5. 实施回写

2026-11-13，未提交。

### 5.1 已做

**C1 改名。**

- 服务：`ws.fork` → `ws.clone`，没有保留旧名。store 里的方法是 `Service::clone_workspace`（[store/src/service.rs](../src/frame/aiworkspace/store/src/service.rs)），不叫 `clone`，免得和 `Clone::clone` 混。
- 返回、`ws.get_info`、`workspace_meta` 的键都由 `forked_from` 改为 `cloned_from`；快照根改为 `lineage.cloned_from`（[core/src/materialize.rs](../src/frame/aiworkspace/core/src/materialize.rs) `snapshot_root`）；物化种类改为 `clone_source`；`LoadSpec.forked_from` 改为 `cloned_from`。
- `ws.import new` 原来也写 `forked_from`（包里的快照），一并改名；`restore` 读包里的 `lineage.cloned_from`。

**C2 联动。**

- 服务方法表、分派、README 方法表；契约 fixture 用 `AIWS_REGEN=1` 重录，diff 只有改名。
- 服务 README 新增“Workspace clone”一段：物化复制、对象共享与逻辑占用、只记出处、带与不带什么，以及与 `tree.clone` / `entity.clone`、放映克隆、会话运行工作区的区别。
- 内核文档：09 的一节改名为“模板、工作区 Clone 与业务实例”，写清语义并加了四种“克隆”的对照表；01、02、03、04、05、06、07、08、11、12、13 和目录 README 里工作区级的 Fork 都改为“工作区 Clone”；13 的锚点同步。
- 其他文档：`doc/workspace/BuckyOS AiWorkspace Durable Data Schema.md`（快照闭包里的 `lineage.cloned_from`）、`doc/testing/aiworkspace-zone.md`（选择器改为 `-g logic-clone`）、`schemas/block-def.schema.json` 一处说明文字。`doc/workspace/` 下其他历史设计稿和实施记录没有改。
- Desktop：`wsFork` → `wsClone`，`WorkspaceInfo.forked_from` → `cloned_from`（新类型 `ClonedFrom`），卡片按钮测试 ID `aiws-fork` → `aiws-clone`，主菜单项 ID `fork` → `clone`，i18n 键 `*.fork*` → `*.clone*`。文案用“复制为新工作区”/“Copy as new workspace”：这是原 Q1 建议里的说法，owner 只确认了方法名，文案待确认。交互没改：按钮仍只对有 `manage` 的人显示（服务只要 `export`，与原来一样）。
- 手册：`aiworkspace_template_dev` 那句改为 `ws.clone { workspace_id }`，并说明只记出处、不同步。`aiworkspace_run` 没有提到 Fork，不用改。
- 测试：store `packages.rs`（V05 改为 `v05_clone`）、`names.rs`、`sso.rs`、`guest.rs`；server `contract.rs`、`process.rs`、`quality.rs`、`sso.rs`；Desktop `features.spec.ts`、DV `aiworkspace.integrity.spec.ts`（`[logic-clone]`）。

**C5 来源记录。** 行为本来就是这样，补了用例（`v05_clone`）：删除来源后，副本照常读写，`ws.get_info.cloned_from` 不变，副本新快照的 `lineage.cloned_from` 仍指向来源快照，来源快照被副本的保留集留住。新用例 `import_new_records_cloned_from`：`ws.import new` 记下包里的快照，再导出、`restore` 后保留。

**C7 额度。** 核对结论：已经按完整内容计入新工作区，不用改代码。

- 装入时每个资产先 `stage_bound` 进新工作区，导入 Commit 再把它转为 `assets`；新工作区的 `assets` 表是空的，所以每个资产都按大小计一次，即使对象在 named store 里与来源共享。
- 结构化内容按导入 Commit 的增量计入 `content`。
- 不计到来源上：来源的 `clones` 只给放映克隆用。
- `v05_clone` 断言：副本 `usage.assets` 等于来源的，`content > 0`，`staged`、`clones` 为 0，来源的 `clones` 为 0。
- 已知差异（不改）：`content` 不会与来源相等，因为来源的 `content` 含富文本的历史 update，副本从新 lineage 开始。
- 另一点（不改，记下）：`load_workspace` 装入时不受额度检查，新工作区在 Service 打开它时才配上 `quota`。这与 `ws.import new`、模板实例化相同。

### 5.2 验证

- `cargo test -p aiworkspace-store -p aiworkspace -- --test-threads=1`：163 通过，0 失败，2 忽略。
- `pnpm exec playwright test --config=playwright.aiworkspace.config.ts features.spec.ts -g "packages: export"`（`AIWS_BIN` 指向新构建的服务）：通过，卡片上的“复制为新工作区”可用。
- `tsc -b` 通过。eslint 对改过的文件只报 DV 文件里未使用的 `QueryPage`，基线就有。
- `deno test tests/datamodel/aiworkspace-i18n.test.ts`：21 通过，1 失败。失败的是“backend speaks English”，命中 `core/src/naming.rs`、`search.rs`、`store/src/wish/outline.rs`、`aiws/prompts/analyze.md`，都不是本次改动；i18n 键的中英一致和使用检查通过。
- 没跑：真实 Zone 的 DV（`-g logic-clone`），没有部署到本机 Zone。

### 5.3 未做

- **C3、C4、C6**：代码里还没有逻辑组、固定组和实体的 `created_by`（权限需求第 1 步，R5–R7）。现在的授权行主体是用户、`group:`、`contact:`、`*`、`public`，按需求这些将来只作组成员，而成员不随 Clone 传播；在旧模型上先做一版会被第 1 步整体替换，所以没做。第 1 步落地后，在 `clone_workspace` 和 `import` 的 `new` 分支里接上：有 `manage` 带权限设计，只有 `export` 不带，被排除实体上的条目丢弃并在返回里报告，实体创建者记为发起者。
- **已有数据**：没有迁移。已经 Fork 出来的工作区，`workspace_meta` 里的 `forked_from` 不再被读取，`ws.get_info.cloned_from` 为 `null`，之后的快照不再带来源谱系；旧包 `restore` 时 `lineage.forked_from` 也不读。按 AGENTS.md 不做兼容。
- 权限需求文档 §2 现状部分仍写 Fork，没有改（那是基线 `aee37a21a` 的现状描述）。
