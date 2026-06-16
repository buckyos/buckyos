# TaskCenter 数据语义说明

更新时间：2026-06-16

## 1. 定位

TaskCenter 是 Desktop 内的任务管理 UI app，代码位置：

- `src/frame/desktop/src/app/task-center`

TaskCenter 不定义 TaskManager 的系统语义。它负责把 TaskManager / Workflow 暴露的任务快照转换成适合桌面用户查看和操作的 UI 视图。

系统事件的目标真相源是 `slog`，不是 TaskManager snapshot。当前 UI 中仍可使用 mock / task snapshot 派生事件作为 fallback，但不能把 fallback 当成正式事件源。

## 2. 数据入口

TaskCenter 的数据适配层：

- `src/frame/desktop/src/api/task_mgr.ts`

真实运行时通过：

```ts
buckyos.getTaskManagerClient()
```

访问 `buckyos-websdk` 的 Task Manager client。

mock 运行时通过：

- `src/frame/desktop/src/api/task_mgr_mock.ts`
- `VITE_CP_USE_MOCK`

提供本地 UI 数据。

## 3. TaskCenter 视图模型

TaskCenter 视图模型定义在 `api/task_mgr.ts`：

- `Task`
- `TaskStatus`
- `TaskType`
- `TaskSource`
- `SystemNotification`
- `SystemEvent`
- `WorkflowScheduleTaskPayload`

这些是 UI 视图类型，不是后端协议的唯一真相。

后端原始任务结构在适配层中用 `RawTask` 表示，字段来自 TaskManager SDK：

- `id`
- `user_id`
- `app_id`
- `session_id`
- `parent_id`
- `root_id`
- `name`
- `task_type`
- `runner`
- `status`
- `progress`
- `message`
- `data`
- `permissions`
- `created_at`
- `updated_at`

## 4. 状态映射

后端状态到 UI 状态的映射：

| 后端状态 | UI 状态 |
| --- | --- |
| `Pending` | `pending` |
| `Running` | `running` |
| `Paused` | `paused` |
| `WaitingForApproval` | `paused` |
| `Completed` | `completed` |
| `Failed` | `failed` |
| `Canceled` / `Cancelled` | `cancelled` |

注意：

- `WaitingForApproval` 在任务列表里表现为 `paused`，同时会派生 notification。
- TaskCenter 不应自行创造新的后端状态词汇。

## 5. 任务类型映射

UI `TaskType` 是展示分类：

- `one-time`
- `scheduled`
- `download`
- `sync`
- `install`
- `workflow`

映射依据包括：

- `task_type`
- `data.schema_type`
- `data.schemaType`

关键约定：

- `task_type === "workflow/schedule"` 或 `schema_type === "workflow/schedule"` 会映射为 `scheduled`。
- 包含 `workflow` 或 `agent.` 的任务会映射为 `workflow`。
- 包含 `download`、`install`、`sync` 的任务映射到对应 UI 分类。

## 6. 计划任务语义

计划任务页面展示的是 `workflow/schedule` 任务。

TaskCenter 会把 Task status 转成 UI 友好的 `WorkflowScheduleStatus`：

| Task status | Schedule status |
| --- | --- |
| `running` | `enabled` |
| `paused` | `paused` |
| `failed` | `error` |
| `completed` / `cancelled` | `archived` |
| 其他 | `enabled` |

`WorkflowScheduleTaskPayload` 中的 `request.status` 不是后端状态真相。当前适配层会重新写入由 Task status 推导出的 schedule status。

需要保持的边界：

- Workflow / ScheduledTaskManager 负责定义计划任务执行语义。
- TaskCenter 只负责把任务快照转换成可读视图。

## 7. Notification 语义

TaskCenter 会从任务快照派生 system notification。

当前规则：

- 当 `task.payload.backendStatus === "WaitingForApproval"` 时，生成 approval notification。
- notification id 格式为 `task-approval-${taskId}`。
- action 包括 `approve` / `reject`。

用户操作 notification 时，TaskCenter 调用：

```ts
updateTaskData(id, {
  ...taskData,
  human_action: {
    kind,
    actor: "desktop",
    submitted_at,
    payload: {
      source: "desktop",
      acted_at
    }
  }
})
```

其中：

- `approve` / `confirm` 归一为 `human_action.kind = "approve"`。
- `reject` / `dismiss` 归一为 `human_action.kind = "reject"`。
- `human_action` 采用 `buckyos-api` 中的 `TaskHumanAction` 结构：`kind`、`payload`、`actor`、`submitted_at`。
- `payload.source = "desktop"` 用于标记动作来源；`payload.acted_at` 保留 ISO 时间，方便 UI/日志读取。
- 兼容期内可以同时写入顶层 `source` / `acted_at`，但后端消费者应以 `TaskHumanAction` typed 字段为准。

结论：

- Beta2.2 先把 `human_action` 写入 `Task.data` 作为稳定交互协议。
- Workflow 已通过 TaskManager data 事件订阅回灌 `human_action`，短期不新增 TaskManager approval 专用 RPC。
- 如果后续需要审计、幂等提交或更强权限边界，再新增 `submit_human_action` / approval RPC，并让它写入同一个 typed `TaskHumanAction`。

## 8. Task Schema 交互语义

TaskCenter 详情页已支持 schema 驱动交互 UI 的最小闭环。

当前支持：

- `human/approval` 或 `interaction.kind = "approval"`：渲染 Approve / Reject 两个动作。
- `human/comment`、`human/suggestion` 或 `interaction.kind = "comment" / "suggestion"`：渲染输入框和提交按钮。

数据入口：

- `task.schemaType`
- `task.payload.schema_type`
- `task.payload.schemaType`
- `task.payload.interaction`

提交入口：

```ts
store.submitTaskHumanAction(taskId, action, payload?)
```

写回规则：

- Approve / Confirm 归一为 `human_action.kind = "approve"`。
- Reject / Dismiss 归一为 `human_action.kind = "reject"`。
- 评论 / 建议类交互写为 `human_action.kind = "submit_output"`，评论内容放在 `human_action.payload` 中。
- 未识别 schema 时不渲染专用交互区，继续展示 Raw payload 兜底。

当前边界：

- 这不是完整 schema registry，只是内置最小渲染器。
- Workflow 当前会消费 `approve` / `reject` / `submit_output` 等已知 `human_action.kind`。
- 更多 schema 类型、schema 注册机制、字段校验和错误展示留到后续阶段。

## 9. SystemEvent 语义

TaskCenter 的 `SystemEvent` 目标真相源是 `slog`，不是 TaskManager snapshot 或 kevent 历史。

当前 fallback 规则：

- 从任务快照的状态推导事件类型。
- `completed` -> `task_completed`
- `failed` -> `task_failed`
- `cancelled` -> `task_cancelled`
- `running` / `paused` -> `task_milestone`
- 其他 -> `task_created`

边界：

- 该事件列表不能作为审计日志。
- 丢失真实 kevent 后，TaskCenter 仍可通过任务快照展示当前状态。
- 如果 Beta2.2 需要事件历史，应接入 `slog` 事件流，而不是继续扩展 UI 派生逻辑。

## 10. Store 语义

TaskCenter store 通过 `useSyncExternalStore` 暴露快照版本。

真实运行时：

- `TaskCenterRpcModel` 初始化时自动 `refresh()`。
- `refresh()` 调用 `listTasks()`。
- 每次刷新后重建 task tree、notifications、events。

mock 运行时：

- `TaskCenterMockModel` 使用本地 mock store。
- 用于 UI 开发和 Playwright 测试。

## 11. 验证重点

自动化验证应覆盖：

- `RawTask` 到 `Task` 的状态、类型、来源映射。
- parent/root 任务树构建。
- scheduled task payload 解析。
- notification 派生和 approve/reject 回滚。
- 详情页 schema renderer：approval 按钮、comment 输入、写回后的 `human_action` 展示。
- `SystemEvent` fallback 视图的排序和字段；后续补 `slog` 接入验证。
- `/taskcenter?taskid=...` 直接进入详情。
- mock runtime 与真实 runtime 的分支不混淆。

## 12. 待确认问题

1. TaskCenter 何时接入真实 `slog` 事件流，以及 slog 与 task id 的关联接口形态。
2. 是否需要在 Beta2.2 之后提供 approval 专用 API，用于审计、幂等和更细权限。
3. 是否需要让 TaskCenter 显示 `human_action.actor` / `submitted_at` 的处理记录。
4. schedule 状态是否应由 Workflow 暴露专门字段，而不是 UI 从 Task status 推导。
5. schema renderer registry 的注册方式、schema 命名规范和字段校验策略。

## 13. Beta2.2 当前建议

- 事件页短期继续使用 mock / task snapshot fallback，不作为正式事件源、kevent 历史或审计日志；正式事件源转向 `slog`。
- approval 和 schema 交互短期继续通过 `Task.data.human_action` 回灌，不新增 TaskManager 专用 RPC。
- `human_action.actor` / `submitted_at` 先作为数据字段保留，UI 是否展示由产品体验另行决定。
- schedule 页面继续从 Task status 推导展示状态；如果 Workflow 后续暴露专门状态字段，再由适配层切换来源。
