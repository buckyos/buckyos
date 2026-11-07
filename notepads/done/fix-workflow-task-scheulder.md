# 修正 Workflow 计划任务执行模型 TODO

## 0. 目标模型

`dcrontab` 是 Agent 面向计划任务的 CLI facade，不直接创建普通任务，也不拥有调度系统。它负责把 CLI 参数翻译成 Workflow schedule schema，再调用 Workflow schedule API。

Workflow schedule 内核只负责两件事：

1. 管理触发器：`cron` / `once` / `run_every`、`next_fire_at`、`misfire`、`fire record`、`pause/resume/archive`。
2. 每次触发时，根据 schedule schema 在计划任务 root task 下创建一个 subtask。subtask 创建后依赖 `task_type` / `runner` / `data` 被对应 executor 自动拾取执行。

必须保持的结构：

```text
workflow/schedule root task
└── fire subtask
    ├── task_type = agent.delegate / workflow.run / workflow.send_message / ...
    ├── runner = schema 指定的 runner
    └── data = render(schema.data_template, fire context)
```

当前实现的问题是：`workflow.run` 路径接近这个模型，但 `remind` 和 `agent_task` 仍是 schedule manager 内部的 hardcoded 分支，没有统一走 schema-driven subtask creation。

## 1. 需要先同步的文档

- [ ] 更新 `notepads/workflow的定时触发器和计划任务管理.md`
  - 明确一个 schedule 必须对应一个 TaskMgr root task；root task 创建失败时，schedule 不应静默成功。
  - 把“每次触发创建 workflow/run child task”扩展为“每次触发按 schedule schema 创建 fire subtask”。
  - 增加 schedule schema 中 subtask template 的字段说明：`task_type`、`runner`、`name_template`、`data_template`、`parent_id/root_id` 绑定规则。
  - 明确 fire record 只记录调度事实与 subtask 关联，不承担业务执行结果总账；业务结果落在 subtask status/data/message 中。

- [ ] 更新 `doc/agent_tool/Agent 计划任务cli工具需求.md`
  - 纠正“remind 不创建 TaskMgr 任务”的旧表述。
  - 明确 `dcrontab remind` 转成 Workflow schedule schema，触发后创建可执行 subtask，由对应 executor 调 Message Center，并把结果写回 subtask。
  - 明确 `dcrontab task` 转成 `agent.delegate` subtask schema，而不是 CLI 或 schedule manager 直接创建普通 task。
  - 保留 CLI 的短形式设计，但把后端映射改成 `workflow schedule schema`，而不是 `ScheduleTarget::{Remind, AgentTask}` 的特殊语义。

- [ ] 更新 `doc/workflow/workflow service.md`
  - 在 Workflow schedule 章节补充 root task / fire subtask 的强约束。
  - 说明 schedule 内核不是业务 executor；它只按 schema 创建 subtask，执行由 TaskMgr runner 机制或 workflow run 机制接管。
  - 补充 `task_type` / `runner` 在计划任务 schema 中的职责边界。

- [ ] 如仍保留 `ScheduleTarget` 文档，统一命名
  - 避免把 `Remind` / `AgentTask` 描述成 schedule manager 的执行分支。
  - 推荐改成 `subtask_template` / `fire_task_template` 这类名字，表达“触发时创建什么任务”。

## 2. 需要修改的实现

### 2.1 数据模型与 API

- [ ] 重构 `src/kernel/workflow/src/scheduled_task_manager.rs`
  - 引入 schedule subtask schema，例如：

    ```rust
    pub struct ScheduleSubtaskTemplate {
        pub task_type: String,
        pub runner: Option<String>,
        pub name_template: String,
        pub data_template: serde_json::Value,
    }
    ```

  - `WorkflowSchedule` 中保存该 template，或把现有 `ScheduleTarget` 收敛为 template。
  - `ScheduleTaskMirror` 必须持有 root task id / root id；对 enabled schedule 来说应是强约束。
  - fire record 增加或明确 `task_id` 字段语义；不要复用 `run_id` 存 TaskMgr task id。

- [ ] 更新 `src/kernel/buckyos-api/src/workflow_service.rs`
  - 对外 API 类型同步 subtask template schema。
  - `run_scheduled_task_now` 返回 `fire_id` 与 `task_id`，必要时保留 `run_id` 仅给 `workflow.run` 类型使用。
  - list/show 返回 root task 与最近 fire subtask 状态，便于 dcrontab 展示。

### 2.2 Schedule 执行核心

- [ ] 修改 `src/kernel/workflow/src/server.rs::fire_schedule`
  - 统一流程改为：

    ```text
    load schedule
    ensure root task
    begin fire
    render subtask template with fire context
    create TaskMgr subtask(parent_id = root_task_id, root_id = root_id)
    complete fire with task_id
    update schedule state
    ```

  - 删除或降级 `ScheduleTarget::Remind` 的“直接成功”分支。
  - 删除 `ScheduleTarget::AgentTask` 中直接旁路 create_task 的特殊逻辑，改为 template 渲染。
  - 确保 manual `run-now` 不污染 `next_fire_at`。
  - root task 不可用时返回明确错误或把 schedule 置为 `error`，不能假成功。

- [ ] 修正 TaskMgr subtask 创建参数
  - 所有 fire subtask 必须设置：

    ```text
    parent_id = schedule.task_mirror.root_task_id
    root_id   = schedule.task_mirror.root_id
    ```

  - 当前 `agent.delegate` 只设置 `root_id = schedule_id`，需要改为挂到 root task 下。

- [ ] 并发控制改为基于 fire subtask
  - 当前 `active_schedule_runs` 只看 `RunStore`，对 `agent.delegate` / remind subtask 无效。
  - 应通过 TaskMgr 查询 root task 下未终态 fire subtask，或在 schedule state 中维护 active fire task。

### 2.3 dcrontab 映射

- [ ] 修改 `src/frame/agent_tool/src/dcrontab_tool.rs`
  - `remind` 映射为 send-message 类型 subtask template。
  - `task` 映射为 `agent.delegate` subtask template。
  - CLI 不直接表达 backend 特殊 target；只构造 workflow schedule schema。
  - `--agent` / `--owner` 缺失时仍按当前 runtime 推断，但最终必须体现在 template runner / owner 中。

- [ ] `dcrontab remind` template 建议

  ```json
  {
    "task_type": "workflow.send_message",
    "runner": "workflow",
    "data": {
      "send_message": {
        "to": "<owner|did|self>",
        "text": "<reminder text>",
        "trigger": {
          "schedule_id": "${schedule.schedule_id}",
          "fire_id": "${fire.fire_id}",
          "fire_time": "${fire.fire_time}",
          "manual": "${fire.manual}"
        }
      }
    }
  }
  ```

  实际 `task_type` / runner 名称以最终 workflow executor 约定为准。

- [ ] `dcrontab task` template 建议

  ```json
  {
    "task_type": "agent.delegate",
    "runner": "<target agent runtime id>",
    "data": {
      "agent_delegate": {
        "version": 1,
        "title": "<title>",
        "purpose": "<objective>",
        "workspace_hints": [{ "workspace_id": "<workspace_id>" }],
        "trigger": {
          "schedule_id": "${schedule.schedule_id}",
          "fire_id": "${fire.fire_id}",
          "fire_time": "${fire.fire_time}",
          "manual": "${fire.manual}"
        },
        "execution": {
          "workspace_id": "<workspace_id>",
          "runner": "<target agent runtime id>",
          "status": "pending"
        }
      }
    }
  }
  ```

### 2.4 Remind 执行器

- [ ] 明确 remind subtask 的 executor 所属模块
  - 方案 A：Workflow service 内置 `workflow.send_message` runner，监听对应 task type 并调用 Message Center。
  - 方案 B：把 remind 编译成标准 `workflow.run`，其中 step executor 为 `service::msg_center.*`。
  - 二选一后写入文档，避免 schedule manager 自己直接执行业务。

- [ ] 如选择 `service::msg_center.*`
  - 在 `src/kernel/workflow/src/adapters/` 增加 msg_center adapter。
  - 在 workflow service 启动时注册该 adapter。
  - 将 Message Center 调用结果写入 step task output / error。

- [ ] 如选择 `workflow.send_message` task type
  - 增加对应 runner 或执行循环。
  - 执行完成后更新 subtask status/data/message。
  - 失败时保留 Message Center 错误详情，便于 dcrontab history/show 展示。

### 2.5 校验与测试

- [ ] `validate_scheduled_task` 必须校验 template
  - `task_type` 非空。
  - runner 可解析或至少符合当前 runner 命名规则。
  - `agent.delegate` 必须有 `title` / `purpose` / 单一 `workspace_id`。
  - remind 的 `to` 可解析；无法解析时返回明确错误。

- [ ] 增加单元测试
  - 创建 schedule 时 root task 必须创建并写入 `task_mirror`。
  - fire `agent.delegate` 创建 root 下 subtask，`parent_id/root_id` 正确。
  - fire `remind` 创建 root 下 subtask，不再假成功。
  - `run-now` 返回 `task_id`，且不改变 `next_fire_at`。
  - 同一个 `fire_key` 重入不会重复创建 subtask。

- [ ] 增加 DV Test
  - 通过 `agent_tool dcrontab --every ... task ...` 创建计划任务。
  - 等待一次 fire。
  - 验证 TaskMgr 中存在 `workflow/schedule` root task 和 `agent.delegate` child task。
  - 验证 AgentTaskExecutor 能拾取 child task 并创建 WorkSession。
  - 增加 remind 端到端测试：fire subtask 执行后能看到 Message Center 调用结果。

## 3. 当前实现中需要重点回看的入口

- `src/kernel/workflow/src/scheduled_task_manager.rs`
  - `WorkflowSchedule`
  - `ScheduleTarget`
  - `ScheduleTaskMirrorClient::ensure_root_task`
  - `ScheduleTaskMirrorClient::create_agent_delegate_task`

- `src/kernel/workflow/src/server.rs`
  - `create_scheduled_task`
  - `run_scheduled_task_now`
  - `scan_due_schedules`
  - `fire_schedule`
  - `active_schedule_runs`

- `src/kernel/workflow/src/task_tracker.rs`
  - `run_task_options`
  - schedule root task 下的 `workflow/run` child 创建逻辑可复用其 parent/root 绑定模式。

- `src/frame/agent_tool/src/dcrontab_tool.rs`
  - CLI 短形式解析可以保留。
  - backend payload 需要从 `ScheduleTarget` 改为 schedule schema / subtask template。

## 4. 非目标

- 不让 `dcrontab` 直接写 TaskMgr 或 ScheduleStore 本地文件。
- 不把 TaskMgr 改造成 cron scheduler。
- 不在 schedule manager 中 hardcode remind / agent task 的业务执行逻辑。
- 暂不处理 TaskCenter UI，等 schedule/task schema 收敛后统一接入。
