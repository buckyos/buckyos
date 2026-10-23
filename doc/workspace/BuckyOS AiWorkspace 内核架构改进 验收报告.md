# BuckyOS AiWorkspace 内核架构改进 验收报告

> 对应：[《BuckyOS AiWorkspace 内核架构改进》](<BuckyOS AiWorkspace 内核架构改进.md>)（下称“设计”）§12.3 的实施 TODO（M0–M5，T0-1 至 T5-6）与 §11.2 的验证组合行为。每个 TODO 项的落点写在设计 §12.3 各里程碑标题下的进度注里，本文不重复。
>
> 读者：审查这轮内核改动、或在其上继续开发的人。
>
> 本文只记录**已经运行过的事实**：每项结论后面是产生它的测试名或命令。没有运行过的写在 §6。日期均为 2026-10-23；全部改动未提交。

## 1. 结论

- **M0–M5 共 51 项 TODO 中 49 项已实施**；T3-9（下线服务内执行路径）按其行内条件等托管执行的 DV 通过后再做；T4-8（独立域名）部分实施，Desktop 页面仍在 Zone 主机服务、SSO cookie 策略待定。各项状态见设计 §12.3 的表格行首标注。
- **后台**：aiworkspace 三个 package（core、store、server）**174 个测试通过，0 失败**（§3）。契约 fixtures 已按 PROTOCOL 1.0 重录；README 协议表与 `METHOD_LEVELS` 的镜像测试通过。
- **Desktop**：aiworkspace Playwright 全套对真实后台进程跑了一遍，**170 个用例中 161 通过、9 失败**；其中 7 个在本轮改动之前就失败或是壳层偶发（§4.2），2 个经复核：一个是本轮 D05 规则让旧断言过时（已改断言并通过），一个是壳层“最大化”偶发（单独重跑通过）。本轮新增与改写的用例（M3 本地执行器、M4 发布与引用、M5 四项）全部通过（§4.1）。
- **§11.2 的 16 条场景**：14 条有对应的自动化测试或 DV 证据，2 条部分覆盖，未覆盖的子项在 §5 明示。
- **真实 Zone（本机 DV）**：T0-6 的 AICC 归属修正、M3 本地执行协议全程、AICC 浏览器适配的消息形状、M4 发布与公开读取 HTTP 路由已在本机 Zone 验证（§4.3）；托管执行（Agent 领取 run）与 `aiws-*` 子域未在真实 Zone 验证。

## 2. 交付物

| 里程碑 | 交付物 | 位置 |
| --- | --- | --- |
| M0 | 错误码目录、不变量/决定性契约测试、`doc.get_submission`、执行上下文与 AICC 归属修正（T0-6） | `core/src/error.rs`、`schemas/error-codes.json`、`core/tests/`、`server/src/wish/llm.rs`、`agent_tool/src/xllm.rs` |
| M1 | FORMAT 1.0 / PROTOCOL 1.0 / DDL v4：对象实体与 `buckyos.dir`、自有数据（`owns`+`bind`、`entity.clone`）、引用对象、固定版本解析、版本矩阵 | `core/src/{model,types,plan,read,materialize}.rs`、`store/src/schema.rs`、`schemas/*.json`、`fixtures/` |
| M2 | 服务级 GC、历史保留期、配额准入与维护通道、崩溃恢复、放映克隆 | `store/src/service.rs`、`store/src/workspace.rs`、`store/src/show.rs` |
| M3 | 执行上下文、`proc.*` 执行器协议（claim / heartbeat / report / tool / program / files）、阶段宿主、本地（浏览器）执行器、Agent 执行器 CLI 与手册、托管配置 | `server/src/wish/{exec,stages,mod}.rs`、Desktop `executor/`、websdk `cli/modules/aiws.ts`、`cmd_skills/aiworkspace_run.md` |
| M4 | Source（`source{kind, mode, …}`）、服务侧刷新、按路径发布、引用登记、`/kapi/aiworkspace/pub/…` 与预渲染、子域路由 | `core/src/types.rs`、`store/src/source.rs`、`server/src/source.rs`、`rootfs/etc/boot_gateway.yaml`、scheduler |
| M5 | 草稿协议、进入编辑即取锁、公式显示与物化、复制画布与 `data.pin`、手册 `aiworkspace_template_dev`、本报告 | Desktop `state/drafts.ts`、`state/locks.ts`、`ui/formula.ts`、`ui/canvas/surfaceOps.ts`、`cmd_skills/aiworkspace_template_dev.md` |
| 联动 | `buckyos-api` `aiworkspace_client.rs`（设置项）、WASM 产物、Desktop 类型与 i18n、README 协议表、AGENTS.md 手册表 | 各处 |

## 3. 后台验收

命令在 `buckyos/src` 下执行：

```bash
cargo test -p aiworkspace-core -p aiworkspace-store -p aiworkspace -- --test-threads=1
```

| 范围 | 通过 | 本轮相关 |
| --- | --- | --- |
| core（单元 + anchor / planner / phase2 / schemas / vectors / contract） | 75 | 错误码目录、`entity.clone` 与自有数据、`buckyos.dir`、公式原样保存、引用对象校验、Source 校验与 `SOURCE_READ_ONLY`、物化公式的 derived 记录 |
| store（单元 + collab / exec / guest / kernel / m2 / packages / phase2 / replica / show / sso / wish） | 68 | GC 与保留、配额与维护通道、历史下限、崩溃恢复、run 领取与撤销、放映克隆随授权、发布表 |
| server（单元 + contract / guest / process / quality / source / wish） | 31 | 契约 fixtures、执行上下文与归属、外部执行器协议、`DELEGATION_STALE` / `RUN_REVOKED`、发布→HTTP→另一 Workspace 插入→刷新→撤回 |

另有 1 个性能探测默认忽略。websdk：`deno check --config cli/deno.json cli/modules/aiws.ts` 与 `deno test … cli/tests/aiws_test.ts` 通过。

## 4. Desktop 验收

命令在 `buckyos/src/frame/desktop` 下执行（先 `cargo build -p aiworkspace`；改过 core 读取/outline 后重建 WASM）：

```bash
AIWS_BIN=/tmp/dev-cache-root/cargo-target/debug/aiworkspace pnpm exec playwright test --config=playwright.aiworkspace.config.ts
pnpm check && pnpm exec eslint <改动文件>
```

### 4.1 本轮用例

| 用例 | 覆盖 |
| --- | --- |
| `wish.spec.ts` “内核架构改进 T3-4: the local executor …” | 浏览器托管模型循环与 Worker 程序，服务只规划；刷新后 run 变 interrupted 并可续跑 |
| `permissions.spec.ts` “AI Center Quota §3.3 …”（托管执行设置断言） | Owner 托管配置管理面、仅 `run` 权限者走托管 |
| `sources.spec.ts` “M4: publish a table, insert its link elsewhere …” | 发布、另一 Workspace 插入链接、跟随刷新、只读、引用者列表 |
| `m5.spec.ts` T5-1 | 草稿带 `idempotency_key`：已接受的提交在另一设备只清草稿不重发；未发出的草稿用同一 key 重发 |
| `m5.spec.ts` T5-2 | 进入 `lock_required` 编辑器即取锁（富文本与表格）；锁被强制解除后下一次编辑在编辑器处提示、修改保留、再进入取回后发出 |
| `m5.spec.ts` T5-3 | `$formula` 值求值显示不写回；物化按钮写入值并记录 `derived.program = formula` |
| `m5.spec.ts` T5-4 | 复制画布：自有数据克隆、共享数据引用；`data.pin` 把自有数据移入数据树 |
| `ui.spec.ts` UI-P07（断言改写） | 同画布复制 Block：自有数据（画布内容文件夹里的许愿）克隆，共享表引用（D05） |
| `features.spec.ts` “write locks …”、`offline.spec.ts` “undo removes an unsent submission …” | 取锁约定改动后的回归 |

### 4.2 全套结果

全套 170 个用例：161 通过，9 失败。失败项逐条：

| 用例 | 判断 |
| --- | --- |
| `connectors.spec.ts` CN17、`flow.spec.ts` FP02 / FP04、`i18n.spec.ts` film demo、`features.spec.ts` record properties、`offline-v16.spec.ts` V16、`presentation.spec.ts` P3-03 | 本轮改动前的基线已失败或为壳层“打开窗口/最大化”偶发；未在本轮处理 |
| `ui.spec.ts` UI-P07 | 本轮 D05 规则（同画布复制克隆自有数据）让旧断言过时；断言已改，单独重跑通过 |
| `ui.spec.ts` UI-P01b | 壳层“最大化”按钮超时；单独重跑通过 |

### 4.3 真实 Zone（本机 DV，`test.buckyos.io`）

| 项 | 做法 | 结果 |
| --- | --- | --- |
| T0-6 AICC 归属 | 同一 analyze 阶段前后对比 `aicc-usage-log.db` | 修复前 `user_id=ood1, caller=system:aiworkspace`；修复后 `user_id=devtest, app_id=aiworkspace, operator_id=devtest, mode=workspace_owner, run_ref{…}` |
| M3 本地执行协议 | `m3_dv.ts local <ws>`：`proc.start{execution.mode: local}` → claim → files / tool / program → report → apply | 全程通过，候选被规划并应用 |
| AICC 浏览器适配 | `m3_dv.ts chat`：`helper.llm_chat` 与 `chat.completions.create` 的返回形状 | 与适配器假设一致（assistant 消息原样保留） |
| M4 发布与公开读取 | 部署当前二进制后在新建工作区 `ws_nzzalmz6rmfcukeku35snc6ooi` 上 `m3_dv.ts publish <ws>`，再无会话 `curl /kapi/aiworkspace/pub/<ws>/orders`（JSON、`Accept: text/html`、`?fixed=1`、不存在的名字） | 通过：`ws.get_info.public_base = https://aiws-<id>.test.buckyos.io`、`traffic 0`；`ws.publish` 返回 `path /m3-dv/orders`、`url`、引用对象 `{workspace_id, entity_id, mode: live}`；`ws.list_publications` 列出该项；公开 JSON 含 5 个字段与全部记录；HTML 预渲染有 `<title>订单</title>` 与表格；`?fixed=1` 带 `content_root`；未发布的名字 404 |

## 5. §11.2 场景对照

| 场景 | 证据 | 状态 |
| --- | --- | --- |
| 对象改名后旧名称被复用，离线命令迟到 | `store/tests/replica.rs::unknown_results_revoked_permission_and_epoch_change`；引用对象按 ID 不按名（D10，`core/tests/phase2.rs` 引用校验） | 覆盖 |
| 同时创建数据源、Block 和绑定，中途失败 | `store/tests/kernel.rs::v10_mixed_batch_atomic_with_fault_injection`、`core/tests/phase2.rs::own_data_follows_its_block_and_shared_data_never_does` | 覆盖 |
| 两人分别修改 min 和 max；查询依赖期间新增记录 | `core/tests/planner.rs`（`unique` 与字段级约束）、`core/tests/phase2.rs::every_selector_of_the_frozen_catalogue_resolves_to_a_version_cell`（依赖版本格可识别变化） | 部分：跨字段不变量按 D-决策不由内核保证，没有“min ≤ max”的测试 |
| 提交成功但响应丢失，随后锁过期或权限变化 | `store/tests/kernel.rs::v12_idempotency_and_unknown_result`、`server/tests/process.rs::crash_recovery_at_three_positions`、`store/tests/replica.rs::unknown_results_revoked_permission_and_epoch_change` | 覆盖 |
| 去重历史已过期后收到很旧的提交 | `store/tests/m2.rs::the_history_floor_refuses_what_lies_below_it` | 覆盖 |
| 相同内容由完整备份或内容分享分别恢复 | `store/tests/packages.rs`、`store/tests/replica.rs`（epoch 变化）、`server/tests/process.rs::replaying_an_import_receipt_does_not_undo_a_later_restore` | 覆盖 |
| Source 初次下载失败、刷新失败、刷新时达到配额 | `server/tests/source.rs::publish_insert_refresh_and_unpublish`（初次失败不发布、刷新失败保留旧版、撤回后刷新失败不动旧数据）；配额由同一 commit 准入把关：`store/tests/m2.rs::quota_admission_and_the_maintenance_channel` | 部分：没有“刷新时恰好超额”的专门用例 |
| 演示期间原数据更新、资源替换或删除 | `store/tests/m2.rs::a_show_clone_follows_the_source_workspaces_current_grants`、`store/tests/show.rs` | 覆盖 |
| 解除最后一份历史保留后回收资源 | `store/tests/m2.rs::the_collector_keeps_what_a_root_reaches_and_reclaims_the_rest`、`purge_reclaims_at_once_and_closes_undo` | 覆盖 |
| Workspace 满额后执行回收、取消和恢复操作 | `store/tests/m2.rs::quota_admission_and_the_maintenance_channel` | 覆盖 |
| 多人多次读取同一公式，或缓存失效 | `core/tests/phase2.rs::opaque_entities_and_formulas_are_kept_verbatim_and_schema_checks_run_only_at_admission`（读取不推进版本）、`m5.spec.ts` T5-3（前端求值不写回） | 部分：“缓存不绕过权限”没有专门用例（前端无跨用户缓存） |
| 任务意图保存后服务崩溃，或执行结果提交时断线 | `store/tests/m2.rs::a_restart_marks_owned_runs_interrupted_before_anyone_asks`、`store/tests/wish.rs::an_accepted_application_is_reconciled_after_a_crash`、`server/tests/wish.rs::restart_reports_interrupted_and_reconciles_applying` | 覆盖 |
| 已授权任务被修改、运行中撤权、旧尝试迟到 | `server/tests/wish.rs::executors_outside_the_service_claim_report_and_the_service_plans_their_candidates`（`DELEGATION_STALE`、被取代的 attempt `RUN_REVOKED`）、`store/tests/exec.rs`（过期 attempt 的 heartbeat / check 得 `RUN_REVOKED`）、`server/tests/wish.rs::an_outside_executor_works_on_the_services_stage_through_proc_tool_and_proc_program` | 覆盖 |
| 用户任务缺失执行上下文或 AI 归属，宿主已有系统 runtime | `server/tests/wish.rs::wish_runs_use_the_owners_or_the_starters_aicc`（`AI_ATTRIBUTION_UNAVAILABLE`）、`exec.rs` `EXECUTION_CONTEXT_MISSING`（`server/tests/wish.rs` 外部执行器用例的上下文断言） | 覆盖 |
| 服务代理调用 AICC，AI 费用由 Owner 承担 | T0-6 DV（§4.3 用量记录四元组）、`server/tests/wish.rs::wish_runs_use_the_owners_or_the_starters_aicc` | 覆盖 |
| 表单提交成功后清理草稿失败，再从另一设备打开 | `m5.spec.ts` T5-1 | 覆盖 |

## 6. 未完成与未验证

- **T3-9**：服务内执行路径（进程内 xllm、Deno runner、`wish-host` 路由）仍在；按设计行内条件，待托管执行在真实 Zone 跑通后再下线。
- **托管执行 DV**：Agent 经 websdk `buckyos aiws run-claim …` 领取并完成 run，未在真实 Zone 验证（aios 容器尚未装本轮 websdk dist）。
- **T4-8**：`aiws-<id>.<zonehost>` 子域要重新部署 scheduler 与 `boot_gateway.yaml`，未做；Desktop SPA 仍在 Zone 主机服务；改名别名只对发布名生效。跨 Zone 的 Source 刷新与引用登记代码已写、未实测。
- **T5-5**：`aiworkspace_template_dev` 手册的示例按协议手写，未在 DV 中由 Agent 实跑。
- **§5 的两条“部分”**：跨字段不变量、Source 刷新恰好超额。
- 全套 e2e 中 7 个本轮之前已失败/偶发的用例未处理（§4.2）。

