# BuckyOS AI Workspace 许愿格实施记录与验收报告

> 对应：[《许愿格详细设计》](<BuckyOS AI  Workspace 许愿格详细设计.md>) v0.2（下称“设计”）。实施范围是设计 §16.1 的 W0–W4，W5 未做。实现与设计的差异已回写到设计 §19。
>
> 读者：审查许愿格交付、或在其上继续开发的人。
>
> 本文只记录**已经运行过的事实**：每项结论后面是产生它的命令或测试。没有运行过的写在 §6“未完成与未验证”。所有模型调用都来自脚本化的模型（OpenAI 兼容），**没有在真实 Zone 中经 AICC 运行过**。

## 1. 结论

- **W0–W4 已实施**：契约（`wish.analysis.v2`、结果约定、宿主工具、`aiws` v2、程序存储与 config digest、knowledge / refinements、运行状态与 RPC）、上下文地图与读取工具、Deno 程序运行器、结果收集与当场校验、Markdown 转换、xllm 两阶段（分析、执行、反馈、只重跑程序、修程序、`llm.map`）、服务端规划/预览/原子应用、派生列、html 结果与 Block 宿主 `aiws` v2、Desktop 面板。
- **后台**：aiworkspace 三个 package 共 **102 个测试通过，0 失败**（另有 1 个性能探测默认忽略），其中许愿格新增 23 个（core 6、store 6、server 11）；`aiws` 程序宿主 Deno 测试 3 个通过；共享类型所在的 `buckyos-api` 237 个通过，scheduler 编译通过。见 §3。
- **Desktop**：aiworkspace Playwright 全套 **68 个用例对真实后台进程通过**（许愿格 2 个新用例，原有 Mock 许愿格用例按服务端规划改写后继续通过）。见 §4。
- 设计 §15 的质量任务 Q01–Q11 与 §16.2 的 W 场景，在脚本化模型下的覆盖逐项列在 §5。**真实模型的任务集通过率尚无数据**。

## 2. 交付物

| 交付物 | 位置 |
| --- | --- |
| core：wish v2 契约与 digest（basis / config）、结果组与新鲜度维度（config_changed、direct_stale、external_data、model_judgment、analysis_sources_changed）、`table_view` / `table_query` 输入、`tree_children` / `tree_edge` 版本格、Cell `bindings`、Markdown ⇄ 富文本 | `src/frame/aiworkspace/core/src/{wish,markdown,types,plan,freshness,richtext}.rs` |
| 富文本 schema 新节点（blockquote、code_block、horizontal_rule、table / table_row / table_cell） | `src/frame/aiworkspace/schemas/richtext.basic.v1.json`、Desktop `richtext/schema.ts` |
| store：一致只读快照、短句柄、内容画像、画布方位、地图、物化、读取工具、读集、分析校验、结果收集、规划器、运行记录与幂等应用/对账 | `src/frame/aiworkspace/store/src/wish/` |
| server：运行编排、xllm 适配与宿主工具、Deno 运行器、`llm.map`、`proc.*` 分派、`/wish-host/<token>/llm_map` | `src/frame/aiworkspace/server/src/wish/`、`server/src/{lib,main}.rs` |
| `aiws` v2 程序宿主、给模型的接口文档、两阶段提示词、Renderer 目录 | `src/frame/aiworkspace/aiws/` |
| 服务设置 `AiWorkspaceSettings.wish`（模型、迭代、时限、内存、`llm.map` 上限、Deno） | `src/kernel/buckyos-api/src/aiworkspace_client.rs` |
| WASM 导出（`markdown_to_richtext`、`richtext_to_markdown`、`wish_config_digest`、`wish_needs_analysis`） | `src/frame/aiworkspace/wasm/src/lib.rs` → Desktop `src/app/aiworkspace/wasm/` |
| Desktop：服务端运行适配、面板（知识、修改意见、输出约定、检查、进度、候选预览、反馈、程序查看与编辑、只重跑/修程序）、`aiws` v2 Block 宿主、新鲜度标记 | `src/frame/desktop/src/app/aiworkspace/{api,ui/wish,ui/extensions,ui/sources,state}` |
| 测试 | `core/tests/phase2.rs`、`store/tests/wish.rs`、`server/tests/{wish,quality}.rs` + `server/tests/common/wishkit.rs`（脚本化模型）、`aiws/aiws_test.js`、Desktop `tests/aiworkspace/{wish.spec.ts,mock-llm.mjs,start-backend.mjs}` |
| 文档 | 后端 `README.md`“Wish runs”、Desktop `src/app/aiworkspace/README.md`、设计 §19 |

新增依赖：core 使用 `pulldown-cmark` 0.13（`default-features = false`，已在 `Cargo.lock` 中）；aiworkspace server 依赖仓库内的 `agent_tool`、`llm_context`。

## 3. 后台验收

命令在 `buckyos/src` 下执行（2026-10-06）。

```bash
cargo test -p aiworkspace-core -p aiworkspace-store -p aiworkspace
(cd frame/aiworkspace/aiws && deno test -A)          # Deno 2.9.4
cargo test -p buckyos-api -- --test-threads=1
cargo check -p scheduler
```

| 测试程序 | 通过 | 许愿格相关内容 |
| --- | --- | --- |
| core 单元测试 | 24 | 其中 5 个新增：wish 契约校验、digest、`record_id_for_key`、Markdown 转换 |
| core `phase2` | 6 | `wish_v2_basis_config_groups_bindings`：basis/config digest、结果组、bindings 引用 |
| core 其余（anchor、planner、schemas、vectors） | 18 | 回归 |
| store `wish` | 6 | 见下表 |
| store 其余（单元、collab、kernel、packages、phase2、replica） | 34 | 回归；perf 1 个默认忽略 |
| server `wish` | 3 | 见下表 |
| server `quality` | 8 | 见下表 |
| server 其余（单元、process） | 3 | 回归 |
| `aiws` Deno | 3 | 按名读取、结果/facts/检查输出、错误上报、`llm.map` 回调、外部请求记录 |
| `buckyos-api` | 237 | 回归（`AiWorkspaceSettings` 新增 `wish`，`serde(default)`，旧设置可读） |

server 测试启动真实服务进程，经 xllm SDK 的真实工具循环驱动脚本化模型，程序在真实 Deno 中运行。

| 测试 | 验证内容 |
| --- | --- |
| store `map_tools_analysis_and_writeback` | 地图（任务位置、方位、视图条件、框、批注、知识）；快照在 Workspace 继续变化时保持不变；句柄绑定真实数据与视图；写回一次 Commit；需求改动后需要重新分析；过期的分析计划被拒绝 |
| store `execute_collect_plan_apply_and_refresh` | 视图范围（筛选、排序、选项标签）；未声明读取自动追加输入；当场校验错误；预览 → 应用；移动 Block 不影响、改视图条件使结果过期；只重跑程序时身份不变、按键合并、文字结果标记需重新生成；人工修改后 keep / replace / new；删行需确认；旧预览不能应用 |
| store `derived_columns_are_owned_and_do_not_stale_themselves` | 派生列只写自有字段，不交给程序读取；写自身列不使自己过期；新增记录使其过期，重跑补齐且保留确认过的人工修改 |
| store `generation_cycles_are_refused_through_other_wishes_and_folders` | W07：经其他许愿格和文件夹形成的间接生成环被拒绝，遍历有界 |
| store `context_sources_require_reanalysis_when_they_change` | 写进任务说明的口径文档进入 `analysis.basis.reads`；执行时记入读集；文档改变后需要重新分析 |
| store `an_accepted_application_is_reconciled_after_a_crash` | W13：Commit 已接受而运行记录仍是 applying，经幂等记录对账为 succeeded；重发应用不产生新 Commit |
| server `analyze_execute_apply_rerun_feedback` | 未分析不能执行；分析 → 写回；执行（写程序、运行、交付、检查、finish）→ 候选与预览 → 应用；数据变化后过期，只重跑程序不调用模型；反馈轮从已有程序开始，应用后成为 refinement |
| server `llm_map_cache_mock_and_cancel` | 逐项判断结果走 `llm.map`，重跑时只对变化项调用模型（无变化则全部命中缓存）；Mock 结果由服务端规划；取消打断模型且不写文档；同一 start 请求是同一个 Run |
| server `restart_reports_interrupted_and_reconciles_applying` | W15：重启后运行中的 Run 为 interrupted；applying 且 Commit 未发生的回到 waiting_confirmation |
| server `quality` 8 个 | 见 §5 |

## 4. Desktop 验收

命令在 `src/frame/desktop` 下执行（2026-10-06）。

```bash
pnpm check
pnpm exec eslint src/app/aiworkspace tests/aiworkspace
pnpm build
AIWS_BIN=<cargo-target>/debug/aiworkspace pnpm exec playwright test --config=playwright.aiworkspace.config.ts
```

- `pnpm check`、eslint：无错误；`pnpm build` 成功。
- Playwright 全套：**68 个通过**。测试后台以 `--wish-config` 指向 `tests/aiworkspace/mock-llm.mjs`（脚本化 OpenAI 兼容模型：读地图、取画像、提交分析、写程序、运行、按 facts 写解读、finish）。
- 许愿格新增用例：
  - `许愿格 v0.2: the real executor …`：面板中分析（句柄绑定、写回一次 Commit）→ 执行（进度、候选、表格与正文预览、检查状态）→ 应用（结果、Block、柱状图视图、程序）→ 反馈轮（成为 refinement，不需要重新分析）→ 数据变化后过期 → 只重跑程序（未调用模型）→ 文字结果提示需要重新生成 → 查看程序。
  - `aiws v2: an HTML Block …`：按绑定名读取，类型化写入在一个 batch 中提交（一步撤销），数据变化时 watch 回调。
- 原有 Mock 许愿格用例（UI11–UI13、UI16、UI19、UI20）按“结果交服务端规划与应用”改写后通过。

本次实施中修过的回归：

- core 曾把 html 定义的 `api_version` 限定为 2，使 `blocks.spec.ts` 中“较新 API 版本的定义在本地降级”的用例无法写入定义。改为只校验格式（设计 §19.2），许愿格程序的 `api_version` 同样处理，运行器遇到不支持的版本视为没有可运行的程序。
- 新用例曾断言结果 Block 在可视区域内，它取决于相机位置，重复运行时约一半失败；改为断言 Block 已在画布上（应用后不自动移动相机，设计也未要求）。连续运行 3 次通过。

## 5. 设计 §15 / §16.2 对照

“通过”指在脚本化模型下机械判定项通过，不代表真实模型的质量。

| 编号 | 覆盖 | 结果 |
| --- | --- | --- |
| Q01 指代 | `q01_q04_q06_q07_…`：视图句柄绑定到视图与真实表 | 通过 |
| Q02 重名、缺失 | `q02_ambiguity_is_a_blocker`：重名给出真实 ID 候选，执行被拒绝（NEEDS_INPUT）；不存在的句柄、纯 UI Block 不会被绑定 | 通过 |
| Q03 大表 | `q03_large_table_through_the_program`：12,000 行经程序聚合，与独立计算一致，无截断 | 通过 |
| Q04 口径 | `q01_q04_…`：口径“排除 状态=未结算”进入给模型的材料；程序结果中订单数为 30 − 4 = 26 | 通过 |
| Q05 派生列 | `q05_derived_column_and_rerun` + store 派生列测试 | 通过 |
| Q06 数字可追溯 | `q01_…_q06_…`：解读中的数字都在 facts 或结果表中；未引用的数字列为 `uncited_numbers` | 通过 |
| Q07 图表与视图 | `q01_…_q07`：目录外的 Renderer 在分析时被拒绝；折线图 Block 的 config 以字段 ID 保存 | 通过 |
| Q08 反馈 | server `analyze_execute_apply_rerun_feedback`、Desktop 新用例 | 通过 |
| Q09 修程序 | `q09_repair_after_rename`：字段改名使重跑失败，修程序后通过检查 | 通过 |
| Q10 HTML 结果 | `q10_html_result_with_bindings` + Desktop `aiws v2` 用例 | 通过 |
| Q11 逐项判断 | server `llm_map_cache_mock_and_cancel` | 通过 |
| Q12 第二期 demo | 季度经营分析 demo 用真实执行器（脚本化模型）跑通；AI 短片 demo 只跑了 Mock | 部分 |
| W01 | Q01、Q02；“同一数据多个 Block 只取一次”没有单独断言 | 部分 |
| W02 | `w02_w18_w25_…`：零输入任务可执行；缺失输入是 blocker（Q02） | 通过 |
| W03 | Q03；store 视图范围测试（筛选、排序、选项） | 通过 |
| W04 | W5 | 未做 |
| W05 | `w05_w08_w12_guards`：分析等待确认时需求被修改，写回被拒绝并在预览中说明；分析后改需求、知识、修改意见、输入或口径文档，执行要求先重新分析（server、store 测试） | 通过 |
| W06 | store `execute_collect_plan_apply_and_refresh`：移动 Block 不报过期，改视图条件报过期 | 通过 |
| W07 | store `generation_cycles_…` | 通过 |
| W08 | `w05_w08_w12_guards` + store 收集测试：非法结果、重复名、路径越界、`direct` 提交表格当场拒绝 | 通过 |
| W09 | store 测试：身份按结果名与键稳定，缺失结果保留；新结果组模式（Desktop UI12/UI13，Mock） | 通过 |
| W10 | store 测试：按键保留记录 ID，删行需确认；撤销（Desktop） | 通过；历史恢复属 W5 |
| W11 | store 测试与 Desktop UI12/UI13：keep / replace / new；“确认后再被他人修改仍冲突”没有单独断言 | 部分 |
| W12 | `w05_w08_w12_guards`、Desktop UI20 | 通过 |
| W13 | store `an_accepted_application_is_reconciled_after_a_crash`、server 重启测试 | 通过 |
| W14 | 取消打断模型且不写文档；取消与应用的竞态没有单独测试 | 部分 |
| W15 | server 重启测试 | 通过；`proc.resume` 属 W5 |
| W16 | 运行、快照与读取工具按调用者授权读取，但没有受限用户的测试 | 未验证 |
| W17 | 结果组新鲜度（core phase2）、新结果组（Desktop UI12/UI13） | 部分 |
| W18 | `w02_w18_w25_…`、Desktop UI16：程序、结果与依赖随包导出/导入，过期不洗白 | 通过 |
| W19 | 离线时副本会话拒绝启动运行（Desktop offline 用例） | 通过 |
| W20 | server 与 Desktop 用例走真实 xllm 循环与 Deno；模型为脚本 | 部分（无真实模型） |
| W21 | server 与 Desktop 用例 | 通过 |
| W22 | Q05 | 通过 |
| W23 | store 执行测试 | 通过 |
| W24 | Q10、Desktop `aiws v2` 用例 | 通过 |
| W25 | `w02_w18_w25_…`：联网读取标记 external_data，该部分新鲜度为 unknown | 通过 |

## 6. 未完成与未验证

- **W5 全部**：fixed 输入读取历史对象、表格历史恢复与结果组回滚、`proc.resume`。
- **真实环境**：没有在真实 Zone 中运行；服务模式下 provider 为 `buckyos`（经 AICC，模型 `llm.plan` / `llm.code` / `llm.chat`），这一路径只编译通过，未运行。Deno 在 Zone 中按 `$BUCKYOS_ROOT/libexec/buckyos-tool/runtime/deno` 查找，未在部署环境中确认。
- **真实模型质量**：设计 §15 要求记录任务集通过率；本次只有脚本化模型的结果。
- **设计中未实施的 UI**：画布上的候选预览层、分析后在画布上高亮输入（改为面板中预览与列表，见设计 §19.2）。
- 反馈轮使用新快照、没有单独的轮次预算（设计 §19.2）。
- §5 中标为“部分”“未验证”的场景。

## 7. 已知风险（按“先能力后安全”接受）

- 程序与 html 结果以 Owner 的身份运行；程序可以联网，执行阶段开启 shell，命令运行在服务侧运行目录，不是操作系统隔离（设计 §18）。
- 程序回调 `llm.map` 用每个阶段的临时 token，阶段运行期间任何持有该 token 的本机进程都可以调用。
- `llm.map` 缓存没有保留期清理。

## 修订记录

| 日期 | 内容 |
| --- | --- |
| 2026-10-07 | 初版：W0–W4 实施记录与验收 |
