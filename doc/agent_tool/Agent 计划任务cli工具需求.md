# Agent 计划任务 CLI 工具需求

## 1. 背景与定位

OpenDAN 需要给 Agent 暴露一个计划任务工具，让 Agent 能用接近 crontab 的方式创建/管理"到点干一件事"。

本工具不拥有调度系统，它只是 CLI facade：

```text
Agent / Bash
  -> agent_tool dcrontab ...
  -> Workflow ScheduleStore (kernel/workflow/src/scheduled_task_manager.rs)
  -> Workflow 定时触发
  -> 按 Workflow schedule schema 创建 fire subtask：
       remind  -> workflow.send_message subtask，由 workflow/message executor 调 msg_center
       task    -> agent.delegate subtask，由 AgentTaskExecutor 拾起
```

底层数据模型见 [workflow的定时触发器和计划任务管理.md](../../notepads/workflow的定时触发器和计划任务管理.md)；本文件只定义 OpenDAN AgentTool CLI 的需求。

> 当前实现状态参考：
> - 时间触发已经有 `ScheduleSpec::{Cron, Once, RunEvery}` 三种形式 (`src/kernel/workflow/src/scheduled_task_manager.rs`)。
> - 任务派发已经有 `agent.delegate` task + `AgentTaskExecutor` 走 `create_worksession_by_task_id` 直接落 WorkSession 的路径 (`src/frame/opendan/src/agent_task_executor.rs`)。
> - CLI 短形式仍只暴露 `remind` 与 `task`，但后端 payload 统一映射为 Workflow schedule 的 fire subtask template，不再依赖 `ScheduleTarget::{Remind, AgentTask}` 的特殊执行分支。

## 2. 设计目标

1. CLI 把"crontab 的时间表达力"暴露给 Agent，三类时间触发都要支持：一次性 `run_at`、定时器 `every`、标准 crontab。
2. 计划任务的 CLI payload 在这一版只暴露两类：`remind` 和 `task`，后端统一转成 Workflow schedule schema。
3. 命中 `task` 时落地的是**标准 Agent Task Schema**（title / objective / workspace_id），TaskExecutor 不再需要从 `task.data` 里反解执行参数。
4. 所有 CLI 输出遵循 `AgentToolResult` JSON envelope。
5. 缺少 `OPENDAN_SESSION_ID` 也能跑（可作为普通终端工具），但必须能定位 owner/agent。
6. 所有写操作走后端 API，不直接改 ScheduleStore / TaskMgr 的文件。

## 3. 命名

主命令：

```text
agent_tool dcrontab
```

刻意叫 `dcrontab`（"d" = opendan / delegate），不沿用 `crontab` 这个名字。理由：

- 本工具的参数面和标准 crontab 差别已经不小：多了 `--run-at` / `--every` 两类时间触发；目标只暴露 `remind` 和 `task` 两种，不接收任意 shell 命令；输出是 `AgentToolResult` JSON 而不是行文本。
- 同名会误导用户期待 100% 兼容；不同名能让 LLM 和人类都立刻意识到这是 OpenDAN 自己的子集 + 扩展。

session tool dir 可挂软链接 `dcrontab` 同名即可（不再保留 `crontab` 别名以避免和系统命令冲突）；但 `agent_tool dcrontab` 必须始终可用。

## 4. 子命令

P0：

| 命令 | 说明 |
| --- | --- |
| `add` | 创建计划任务(默认可以不写) |
| `list` | 列出计划任务 |
| `show` | 查看详情 |
| `pause` | 暂停 |
| `resume` | 恢复并重算下一次触发时间 |
| `remove` | 软删除（archive） |
| `run-now` | 手动触发一次 |
| `validate` | 校验触发表达式与 target，不落库 |

P1：

| 命令 | 说明 |
| --- | --- |
| `next` | 预览后续触发时间 |
| `history` | 查看执行历史 |
| `import` / `export` | crontab 文本导入/导出 |

### 4.1 短形式约定（设计原则：越常用越短）

这是给 agent 用的 CLI，每一处使用说明都要进 prompt 消耗 token，所以**越常用的命令格式必须越简短**。CLI 解析层要支持下面的默认/省略规则：

1. **`add` 默认可省**：不带子命令时按 `add` 解析。仅在带其他子命令名（`list` / `show` / `pause` / `resume` / `remove` / `run-now` / `validate`）时才走那个分支。
2. **默认 target 是 `remind`**：trailing 位置参数若是普通字符串，按 `remind <text>` 解析；只有显式写 `task ...` 才走任务分支。
3. **`--name` 可省**：
   - `task` 模式：缺省取 `--title`（它已经是必填字段，没必要再写一遍）。
   - `remind` 模式：缺省取文案前几个字 + 短 hash（如 `喝水-1a2b`、`standup-time-3c4d`）。
   - 都需要时再显式传 `--name`，用于保证句柄稳定。
4. **`--to` 默认 `self`**：不传就是提醒 agent 自己。

得到的常用形态：

```bash
# 工作日 9 点提醒自己 standup
agent_tool dcrontab "0 9 * * 1-5" "standup"

# 每 5 分钟提醒自己喝水
agent_tool dcrontab --every 5m "喝水"

# 6 月 1 日 20:00 提醒主人打电话
agent_tool dcrontab --run-at "2026-06-01T20:00:00+08:00" --to owner "记得打电话"
```

完整形态仍然合法（验收/导出/排错时会用到）：

```bash
agent_tool dcrontab add "0 9 * * 1-5" --name standup \
  remind --to owner --text "standup time"
```

`task` 子命令因为有三个必填字段（title / objective / workspace），不享受短形式收缩 —— 这本来就是不常用、必须每个字段都明确写出来的命令。

LLM 使用说明（写进 tool description 的部分）应当只展示短形式 + 一两条提示，长形式只在 `--help` 和本文档里给出。

## 5. 时间触发：三种形式

`add` 必须支持下面三种互斥的时间触发，CLI 顶层用一组互斥选项表达，最终落到 `ScheduleSpec`。

### 5.1 `--run-at <ISO8601>`  → `ScheduleSpec::Once`

一次性触发，到点跑一次后自动 archived。

```bash
agent_tool dcrontab add --run-at "2026-06-01T03:00:00+08:00" \
  remind "记得把昨晚的下载清掉"
```

| 字段 | 必填 | 说明 |
| --- | --- | --- |
| `--run-at` | 是 | RFC3339 时间戳，CLI 端解析成 unix `run_at` |
| `--timezone` | 否 | 显示和提示用；解析以 `--run-at` 字面量为准 |

### 5.2 `--every <duration>`  → `ScheduleSpec::RunEvery`

定时器，间隔到点触发一次；最小粒度为 1 秒（与底层 `every_sec` 对齐）。

`<duration>` 支持人类可读后缀：`5s` / `30s` / `5m` / `2h` / `1d`，也支持纯秒数。

```bash
agent_tool dcrontab add --every 5m \
  remind "起来喝口水"
```

| 字段 | 必填 | 说明 |
| --- | --- | --- |
| `--every` | 是 | 间隔时长，解析为 `every_sec` |
| `--start-at` | 否 | 起点 unix/ISO 时间；不填取创建时刻 |
| `--end-at` | 否 | 自动停止时刻 |
| `--timezone` | 否 | 仅影响展示 |

### 5.3 位置参数 `<cron>`  → `ScheduleSpec::Cron`

标准 5 字段 crontab 或 `@daily` 等别名。

```bash
agent_tool dcrontab add "0 9 * * 1-5" \
  remind --to owner "standup time"
```

| 字段 | 必填 | 说明 |
| --- | --- | --- |
| `<cron>` | 是 | 5 字段 cron 或 `@hourly` / `@daily` / `@weekly` / `@monthly` / `@yearly` / `@annually` / `@reboot` |
| `--timezone` | 否 | 不填使用 agent/user 默认 |
| `--misfire` | 否 | `skip` / `run_once` / `catch_up` / `manual`，默认 `run_once` |

> P0 不支持秒级 cron。需要"每 N 秒"用 `--every`。

`--run-at` / `--every` / 位置 cron 三者**必须互斥**且至少出现一个。CLI 端要在最早一步检测并报错。

## 6. 触发目标：两种形式

CLI 在时间触发选项之后跟一个目标子命令：

```text
agent_tool dcrontab add <time-trigger> [common-opts] (remind|task) [target-opts]
```

`remind` 和 `task` 是子命令意义上的 target kind，不能混用。

### 6.1 `remind`：到点发提醒

最小语义：到点把一段文字送给"某个人"。

```bash
# 提醒 agent 自己（默认）
agent_tool dcrontab add --every 5m remind "喝水"

# 提醒主人 / 指定联系人
agent_tool dcrontab add "0 9 * * 1-5" remind --to owner --text "standup time"

agent_tool dcrontab add --run-at "2026-06-01T20:00:00+08:00" \
  remind --to did:bns:mom "记得打电话"
```

| 参数 | 必填 | 说明 |
| --- | --- | --- |
| `<text>` / `--text` | 是 | 提醒正文。无 `--text` 时使用 trailing 位置参数 |
| `--to <target>` | 否 | 收件人。不填则提醒 agent_tool（agent 自己 inbox）。可选值：`owner`（当前 agent 所属用户）/ DID / 联系人别名 |

落地语义：

- `--to` 缺省时：通过 agent_tool 的 inbox 通道，把 reminder 作为一条系统消息丢回 agent session，agent 在下一次唤醒会看到。
- `--to` 指定时：走 contact_manager 的发信路径，最终调用 msg_center 的 `post_send`，sender 为本 agent DID。如果指定 `--to` 但 contact 无法解析，schedule 入库前要在 validate 阶段失败，给出明确的 `UNKNOWN_RECIPIENT` 错误。

`remind` 会被编译成 `workflow.send_message` fire subtask，而不是由 CLI 或 schedule manager 直接发消息。该 subtask 的 executor 调用 Message Center，并把结果写回 subtask status / data / message。fire record 只保留 `fire_id` 与 `task_id` 的调度关联。

### 6.2 `task`：到点指派一份 Agent 任务

最小语义：到点用预填好的 Agent Task Schema 创建一个 `agent.delegate` fire subtask，由 AgentTaskExecutor 拾起、直接落 WorkSession。CLI 不直接创建普通 TaskMgr task，schedule manager 也不 hardcode agent task 业务逻辑。

这一版**故意收缩 schema**：调度侧只需要三个字段就能构成可执行的标准任务：

```text
title         # 任务标题
objective     # 给 agent 的目标描述（即 purpose）
workspace_id  # 在哪个 workspace 里做
```

CLI：

```bash
agent_tool dcrontab "0 3 * * *" \
  task --title "扫描新增图片" \
       --objective "找出 album=camera-roll 中上次扫描后新增的图片并归档" \
       --workspace ws-photos
```

`--name` 不必再写——schedule name 默认就是 `--title`，避免一份语义两处填。

| 参数 | 必填 | 说明 |
| --- | --- | --- |
| `--title` | 是 | Task / WorkSession 显示名 |
| `--objective` | 是 | 给 agent 看的目标；最终对应 `CreateWorkSessionParams.objective` |
| `--workspace` | 是 | 目标 workspace id；最终对应 `CreateWorkSessionParams.workspace_id` |
| `--behavior` | 否 | 指定 behavior id；不填走 agent 默认 |
| `--agent` | 否 | 指定执行 agent（写入 task data 的 `execution.runner` 业务字段）；不填取当前 agent |

落地语义：

1. Workflow 到点：
   - 在 ScheduleStore 写一条 fire record（保持现有 `fire_key` 幂等）。
   - **直接**用 schedule 自带的三字段构造标准 Agent Task Schema，调用 task_mgr 创建 `task_type = "agent.delegate"` 的任务。
2. AgentTaskExecutor 拾起后：
   - `task_data_supports_direct_worksession(data)` 命中（已有 purpose + 唯一 workspace_id）。
   - 走 `create_worksession_by_task_id`，直接落 WorkSession 并 auto-start。
3. 这一版不需要 `agent_delegate.workspace_hints`、不需要 `route.session_id`、不需要 `human_input`，schedule 侧填好的就是"已完成路由的标准 Agent Task Schema"。

> **重点**：触发后创建的 task **不走 task.data 推断 / 反解流程**。schedule 里就有 title/objective/workspace_id，executor 直接拿去用。`task.data.agent_delegate` 里写什么由 Workflow 服务在 mirror 时填入，与 schedule 本身的字段一一对应。

## 7. `list`

```bash
agent_tool dcrontab list
agent_tool dcrontab list --status enabled
agent_tool dcrontab list --target remind
agent_tool dcrontab list --target task
```

输出 summary 适合 LLM 阅读：

```json
{
  "agent_tool_protocol": "1",
  "status": "success",
  "cmd_name": "dcrontab",
  "summary": "2 schedules: 1 enabled, 1 paused",
  "detail": {
    "schedules": [
      {
        "schedule_id": "sch_1",
        "name": "scan-new-images",
        "status": "enabled",
        "trigger": { "kind": "cron", "expr": "0 3 * * *", "timezone": "Asia/Shanghai" },
        "target": { "kind": "task", "title": "扫描新增图片", "workspace_id": "ws-photos" },
        "next_fire_at": "2026-05-28T03:00:00+08:00"
      },
      {
        "schedule_id": "sch_2",
        "name": "water-reminder",
        "status": "enabled",
        "trigger": { "kind": "every", "every_sec": 300 },
        "target": { "kind": "remind", "to": "self", "text": "喝水" },
        "next_fire_at": "2026-05-27T22:35:00+08:00"
      }
    ]
  }
}
```

## 8. `show`

```bash
agent_tool dcrontab show sch_1
agent_tool dcrontab show scan-new-images
```

返回完整 schedule、后续触发时间、最近执行状态、对应的 TaskMgr root task id。所有 schedule 都有 root task；`remind` / `task` 的差异体现在 fire subtask 的 `task_type`。

## 9. `pause` / `resume` / `remove`

```bash
agent_tool dcrontab pause sch_1
agent_tool dcrontab resume sch_1
agent_tool dcrontab remove sch_1
```

语义：

- `pause`：状态置 `paused`，不再参与 due scan。
- `resume`：状态置 `enabled` 并基于 `now` 重算 `next_fire_at`。
- `remove`：状态置 `archived`，软删除，历史 fire/run 保留。

## 10. `run-now`

```bash
agent_tool dcrontab run-now sch_1
agent_tool dcrontab run-now scan-new-images --reason "manual test"
```

语义：

- 手动创建一次 fire record（`manual = true`），不改变 `next_fire_at`。
- 对 `task` 类目标：按 template 创建 `agent.delegate` fire subtask，返回 `task_id`。
- 对 `remind` 类目标：按 template 创建 `workflow.send_message` fire subtask，返回 `task_id`；消息投递结果由该 subtask 的 executor 写回。

返回字段：`fire_id`、（如适用）`task_id`。

## 11. `validate`

```bash
agent_tool dcrontab validate --run-at "2026-06-01T03:00:00+08:00"
agent_tool dcrontab validate --every 30s
agent_tool dcrontab validate "0 3 * * *" --timezone Asia/Shanghai
```

调用后端 `workflow.validate_schedule`，不允许 CLI 端复制完整解析逻辑（cron parser 在 `scheduled_task_manager.rs` 里）。

返回：

```json
{
  "valid": true,
  "trigger": { "kind": "cron", "normalized_expr": "0 3 * * *", "timezone": "Asia/Shanghai" },
  "next_fire_times": [
    "2026-05-28T03:00:00+08:00",
    "2026-05-29T03:00:00+08:00",
    "2026-05-30T03:00:00+08:00"
  ]
}
```

P0 至少返回 3 个后续触发时间（Once 类型只返 1 个）。

## 12. `import` / `export`（P1）

只支持 cron 形式（5 字段 / `@aliases`）和 `# remind:` / `# task:` 行内备注语义。CLI 端先做轻量解析后交给 `validate`，再批量 `add`。

约束：

- 支持 `TZ=` / `CRON_TZ=`。
- 忽略空行与 `#` 注释（除 `# remind:` / `# task:` 元数据外）。
- 不支持 system crontab username 字段。
- 不支持 `%` stdin 语义；遇到时报错。
- `--dry-run` 只返回将创建的 schedule 列表，不落库。

`--every` / `--run-at` 形式的计划任务无法用 crontab 文本表达，export 时跳过并在 summary 中提示。

## 13. crontab 兼容范围

P0 必须支持：

```text
* * * * *
*/15 * * * *
0 9 * * 1-5
@hourly
@daily
@weekly
@monthly
@yearly
@annually
@reboot
TZ=...
CRON_TZ=...
```

P0 不支持：

```text
秒级 cron（用 --every）
system crontab username 字段
% stdin 分隔语义
节假日 / 工作日历
```

## 14. 上下文与环境变量

CLI 在 OpenDAN session 中运行时读取：

| 环境变量 | 用途 |
| --- | --- |
| `OPENDAN_AGENT_ENV` | agent env root |
| `OPENDAN_AGENT_ID` | agent id / DID |
| `OPENDAN_SESSION_ID` | 当前 session |
| `OPENDAN_TRACE_ID` | 审计链路 |
| `OPENDAN_AGENT_TOOL` | 主 agent_tool 路径 |

但计划任务也可能在普通终端调用，因此 CLI 不能强依赖 `OPENDAN_SESSION_ID`：

- `--agent` 显式指定 agent。
- `--owner` 显式指定 owner。
- 都未指定时使用当前登录 runtime 的 user/app。

## 15. 输出协议

所有子命令默认输出单行 `AgentToolResult` JSON。

成功：

```json
{
  "agent_tool_protocol": "1",
  "status": "success",
  "cmd_name": "dcrontab",
  "cmd_args": "add ...",
  "title": "created schedule scan-new-images",
  "summary": "scan-new-images will run daily at 03:00 Asia/Shanghai",
  "detail": { }
}
```

错误：

```json
{
  "agent_tool_protocol": "1",
  "status": "error",
  "cmd_name": "dcrontab",
  "summary": "invalid cron expression: expected 5 fields",
  "detail": { "error_code": "INVALID_CRON" }
}
```

常见 `error_code`：`INVALID_CRON`、`INVALID_DURATION`、`INVALID_RUN_AT`、`UNKNOWN_RECIPIENT`、`UNKNOWN_WORKSPACE`、`MULTIPLE_TRIGGERS`、`MISSING_TRIGGER`。

## 16. 后端 API 映射

| CLI | Workflow / TaskMgr API |
| --- | --- |
| `add` | `workflow.create_scheduled_task`，payload 为 fire subtask template |
| `list` | `workflow.list_scheduled_tasks` |
| `show` | `workflow.get_scheduled_task` |
| `pause` | `workflow.pause_scheduled_task` |
| `resume` | `workflow.resume_scheduled_task` |
| `remove` | `workflow.archive_scheduled_task` |
| `run-now` | `workflow.run_scheduled_task_now` |
| `validate` | `workflow.validate_scheduled_task` |
| `history` | `workflow.get_scheduled_task_history` |

CLI 后端不再表达 `ScheduleTarget::{Remind, AgentTask}` 特殊语义，而是统一构造 `target` fire subtask template：

```rust
pub struct ScheduleSubtaskTemplate {
    pub task_type: String,          // workflow.send_message / agent.delegate
    pub name_template: String,
    pub data_template: serde_json::Value,
}
```

CLI 不直接调用 TaskMgr 创建 recurring task，也不直接写 ScheduleStore 本地文件。Workflow ScheduleStore 负责 root task mirror 与每次 fire subtask creation。

## 17. 实现位置建议

新增：

```text
src/frame/agent_tool/src/dcrontab_tool.rs         # TypedTool 实现
src/frame/agent_tool_cli_dev/src/lib.rs            # 注册 dcrontab 子命令
src/frame/opendan/src/agent_bash.rs                # session tool 软链
```

修改：

```text
src/kernel/workflow/src/scheduled_task_manager.rs  # ScheduleSubtaskTemplate / root task mirror / fire subtask creation
```

实现要点：

- 走 `TypedTool` + `parse_cli_args`。
- 三种时间触发用 clap 的 group 互斥校验。
- target 子命令用 clap subcommand 表达 `remind` / `task`。
- `task` 映射为 `agent.delegate` subtask template，data_template 至少包含：

  ```json
  {
    "agent_delegate": {
      "purpose": "<objective>",
      "workspace_hints": [{"workspace_id": "<workspace_id>"}],
      "title": "<title>",
      "trigger": {
        "schedule_id": "sch_...",
        "fire_id": "fire_...",
        "fire_time": 1779870000,
        "manual": false
      }
    }
  }
  ```

  目的是落到 `task_data_supports_direct_worksession` 命中分支：单一 workspace_id + 非空 purpose，直接 `create_worksession_by_task_id`，不再走 `task.data` 二次推断。

`remind` 映射为 `workflow.send_message` subtask template。该 task type 由 Workflow service 自己执行（按 task_type 扫描自己拥有的任务，beta2.2 起没有 TaskMgr runner 归属字段）：schedule manager 只创建 subtask，具体 Message Center 调用和结果写回由 `workflow.send_message` executor 完成。CLI 端**不要落地为本地文件**形成第二套真相源。

## 18. 典型用例

> §18.1–18.3 用短形式（agent 最常用的写法），§18.4 task 必须用完整形态。

### 18.1 工作日 9 点提醒 standup（cron + remind to owner）

```bash
agent_tool dcrontab "0 9 * * 1-5" --to owner "standup time"
```

等价的完整形态：

```bash
agent_tool dcrontab add "0 9 * * 1-5" --name standup \
  remind --to owner --text "standup time"
```

### 18.2 每 5 分钟提醒自己喝水（every + remind 默认 self）

```bash
agent_tool dcrontab --every 5m "喝水"
```

### 18.3 6 月 1 日 20:00 一次性提醒主人打电话

```bash
agent_tool dcrontab --run-at "2026-06-01T20:00:00+08:00" --to did:bns:mom "记得打电话"
```

### 18.4 每天 03:00 扫描新增图片（cron + task）

```bash
agent_tool dcrontab "0 3 * * *" \
  task --title "扫描新增图片" \
       --objective "找出 album=camera-roll 中上次扫描后新增的图片并归档" \
       --workspace ws-photos
```

返回（`name` 自动取自 `--title`）：

```json
{
  "status": "success",
  "summary": "扫描新增图片 will run daily at 03:00 Asia/Shanghai",
  "detail": {
    "schedule_id": "sch_...",
    "name": "扫描新增图片",
    "trigger": { "kind": "cron", "expr": "0 3 * * *" },
    "target": {
      "kind": "task",
      "title": "扫描新增图片",
      "workspace_id": "ws-photos"
    },
    "next_fire_at": "2026-05-28T03:00:00+08:00",
    "task_root_id": 123
  }
}
```

### 18.5 手动测试一次

```bash
agent_tool dcrontab run-now "扫描新增图片" --reason "test new schedule"
```

## 19. 验收标准

1. `add` 三种时间触发都可用，互斥校验明确报错。
2. `add ... remind` 触发后创建 `workflow.send_message` fire subtask；`add ... task` 触发后创建 `agent.delegate` fire subtask，二者都挂在 schedule root task 下。
3. `task` 触发产生的任务命中 `task_data_supports_direct_worksession`，AgentTaskExecutor 走 `create_worksession_by_task_id`，WorkSession 拿到的 `title` / `objective` / `workspace_id` 与 schedule 字段完全一致。
4. `remind --to owner` 能成功发出消息；目标 contact 不可解析时 schedule 在 validate 阶段失败。
5. `validate` 至少返回 3 个未来触发时间（Once 类型 1 个）。
6. `list` / `show` 能区分展示 `remind` 与 `task` 两类 target。
7. `pause` 后不再触发；`resume` 重新计算 next_fire_at。
8. `run-now` 返回新的 `fire_id` / `task_id`，不污染 `next_fire_at`。
9. CLI 输出满足 `AgentToolResult` 协议。
10. 缺少 `OPENDAN_SESSION_ID` 时，显式传 `--agent` / `--owner` 仍可工作。
11. CLI 不直接写 ScheduleStore 或 TaskMgr 的本地文件。
12. 短形式可用：`agent_tool dcrontab "0 9 * * 1-5" "standup"` 必须等价于完整形态的 `add ... remind --text ...`，包括 `add` 省略、`remind` 默认 target、`--name` 自动生成、`--to` 默认 `self`。
