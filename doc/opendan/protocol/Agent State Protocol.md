# Agent State Protocol

版本 1 · 2026-09-29 · 由 `libopendan` 反写（`src/protocol/agent_state.rs`、`src/state/*`）

## 1. AgentRoot 布局

```text
<agent_root>/
  role.md self.md tools/ tool_plans/<name>.toml behaviors/ …   包层（OpenDAN 维护，libopendan 只读）
  sessions/<sid>/                           session 目录的默认位置（可选）
  state/
    sessions/<sid>.json                     登记表（全部 session，无论目录在哪）
    sessions/.unreachable/<sid>             巡检标记（目录不可达）
    perception/<sid>.jsonl                  感知
    perception/.cursor.json                 整理游标
    perception/.consolidations.jsonl        整理审计
    artifacts/<aid>/artifact.json           产物 head
    artifacts/<aid>/versions/v-<sid>.json   各 work session 的贡献
  memory/  notebook/  attention_signals/    认知（agent_tool 实现与其内部锁）
  workspace/<wid>/                          Agent 内部 workspace
  .locks/self_improve.lease  .locks/artifact/<aid>.lease
```

访问经 `AgentStateClient`：文件版直接读写上述文件（要求能看到 AgentRoot）；kRPC 版与 DFS 版必须对外表现一致（同一组 fixtures）。

## 2. 登记表 `state/sessions/<sid>.json`

```jsonc
{ "session_id": "work-…", "kind": "work", "class": "work",
  "created_by": { "principal": "app:app2@alice", "via": "app" }, "idempotency_key": null,
  "driver": { "principal": "app:app2@alice" },
  "location": "/abs/path/<sid>", "location_rev": 0,
  "input_queue": "app2::alice::opendan.session.<sid>", "wake_event": "/opendan/<agent_id>/session/<sid>/input",
  "agent_access": "full | status_only", "origin": null, "workspace": null, "artifact_id": null,
  "objective": "…", "scope": { "paths": [], "objects": [] },
  "status": { "rev": 17, "run_state": "…", "outcome": null, "acceptance": "…", "one_line_status": "…",
              "report_brief": "≤500 字", "pending_decision": null,
              "activity": { "summary": "…", "touching": [], "heartbeat_ms": 0 },
              "last_runner": { "runner_id": "…", "host": "…", "pid": 0, "lock_epoch": 0, "at_ms": 0 },
              "last_error": null, "updated_at_ms": 0 } }
```

- **注册**：创建者用不覆盖发布写入一次；已存在且 location 相同或同一身份（幂等键 + 创建者 + 驱动者 + kind）→ 返回已有条目；否则 SessionIdConflict。
- **回报**：驱动者持 session 锁、在每次提交之后写 `status`（整条原子替换）；`status.rev ≤` 已有值时忽略。回报失败由下次 drive 按 `state.reported_rev` 补发。`status` 只是缓存，真相是 session 的 state.json。
- **巡检**：location 不存在时只写 `.unreachable/<sid>` 标记，不删除、不改写条目（保持单写者）；驱动者下次回报时清除。
- **查询**是派生视图（扫描 + mtime 缓存），可以另建可删除重建的索引。

## 3. 活动 Session 视图

- 活跃 = `run_state ∈ {running, waiting}`。`running` 且 `max(heartbeat_ms, updated_at_ms)` 超过 5 分钟 → 标为“可能已中断”；`waiting` 不要求心跳。
- 声明的引用 = `scope.paths` + `scope.objects` + `status.activity.touching[].ref` + `artifact:<aid>`。两个引用相等或一方是另一方的路径前缀（`ws:snake/src/` 与 `ws:snake/src/collision.js`）即有交集。
- 关系排序：同一 workspace 或同一产物（`same_target`）> touching 有交集（`overlap`）> 其它；`agent_access = status_only` 的条目只给 one_line_status。
- activity 来源：创建时 scope（第一轮写入）、`control(activity)`、runner 从写类工具参数推断（`write_file` / `edit_file` 的 path）；run 结束清空 touching，finished 清空整个 activity。
- 它是**避让提示**，不是锁；冲突裁决属于 workspace。

## 4. 感知

```jsonc
// state/perception/<sid>.jsonl，单写者 = 该 session 的驱动者，seq 严格递增
{"seq":31,"at_ms":0,"session_id":"…","kind":"round_digest|observation|task_outcome|task_discarded",
 "source":"session","tags":[],"objects":[],"summary":"…","payload":{…},"refs":{"worklog_seq":316}}
```

- 追加幂等：跳过 `seq ≤` 文件最后一条的记录（反向读最后一行得到）。
- 自动记录：每次 run 结束 `round_digest`；finished 追加 `task_outcome`；discard 追加 `task_discarded`（保留来源）；`perception` 类型输入并入 `observation`。`state.perception_seq` 在提交中预留，追加在提交之后，缺失的由下次 drive 补发。
- **积压（backlog）**：按文件大小与 `.cursor.json.offsets[sid]` 求区间，跳过以 `.` 开头的文件和 kind 为 `self_improve` 的 session（防自我回声）。
- **self_improve**：session 的 `extensions.opendan.perception_window` 记录要整理的窗口（幂等键 `si:<窗口摘要>`）；drive 全程持 `self_improve` 锁（拿不到返回 Busy）；session 以 succeeded 结束后先写 `.consolidations.jsonl` 审计，再把游标推进到窗口末尾（不回退，at-least-once）；失败或 stopped 不推进。

## 5. 认知（边界）

Memory Graph（`memory/`）、Notebook（`notebook/`）、整理中间态（`attention_signals/`）由 `agent_tool` 实现；非 Rust 实现通过 `agent_tool agent-memory|agent-notebook …` CLI 访问，不重写。`recall_hints` 只读，任何 session 可调用；`notebook_append` 用于“记一下”；整理结果的提交只能在 `self_improve` 锁下进行。

## 6. 产物列表（只登记与指向）

```jsonc
// artifact.json
{ "aid": "snake-game", "workspace": { "kind": "external", "path": "…" }, "head": "v-work-A", "rev": 3, "updated_at_ms": 0 }
// versions/v-<sid>.json
{ "ver": "v-work-B", "session": "work-B", "base": "v-work-A", "state": "produced|accepted|discarded",
  "outputs": ["report.md", "snake.js"], "workspace_ref": null,
  "side_effects": [ { "call_id": "c-1", "tool": "exec", "note": "exec cannot be undone by the session" } ], "updated_at_ms": 0 }
```

- work session finished 时由驱动者（持 session 锁）登记自己的版本：`base` = 当时的 head（新工作从用户接受过的版本继续），`state = produced`；`artifact.json` 不存在时以不覆盖发布创建（head = null）。已 accepted / discarded 的版本不再被改回 produced。
- **decide**（驱动者在 drive 开头应用 `control(decide)`）：持 `artifact:<aid>` 锁，**在锁内重读** head 与版本：
  - accept：版本 → accepted，head → 该版本（discarded 的版本不能 accept）。
  - discard：版本 → discarded；只有 head 正指向它时才回退到 `nearest_valid_base`（沿 base 链找仍 accepted 的祖先，没有则 null；base 缺失或成环报错）；head 指向其它版本时保持不变。
  - 拿不到 artifact 锁：control 留在队列，下次再试。
- discard 的报告写入 `state.result.discard_report`：`workspace` = `none`（无 workspace）或 `unsupported`（workspace 回滚另行设计），`unsupported[]` 逐项列出不可撤销的副作用（worklog 中 effect 非 read_only 的动作），`head_moved_to` 记录 head 结果。
- 顺序：decide 条目先进 worklog，state.json 提交（acceptance 变化），之后追加 `task_discarded` 感知。
