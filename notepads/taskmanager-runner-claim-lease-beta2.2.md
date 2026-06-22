# TaskManager Assigned Executor Start / Timeout Plan for Beta2.2

## 1. Background

TaskManager is the shared state layer for distributed async work. Producers and schedulers create tasks with:

- `status = Pending`
- `runner = <assigned executor id>`
- task-specific payload in `data`

Consumers discover work through two channels:

- `task_ready` kevent wake-up
- database state queried through TaskManager APIs

The key rule is that kevent is only an acceleration signal. The database state is the source of truth.

The Beta2.2 model is not an open preemptive producer-consumer queue. Scheduler assignment is deterministic. A runner may start only tasks already assigned to that runner, and TaskManager's atomic update is the duplicate-start guard.

## 2. Implemented Behavior

The current branch implements the assigned executor baseline:

- `start_assigned_task(id, runner, timeout_ms?)` atomically changes a matching `Pending` task to `Running`.
- The update is guarded by `id + runner + Pending`.
- Successful start returns an opaque `execution_token` and `timeout_at`.
- `extend_task_execution(id, execution_token, timeout_ms?)` can extend the execution timeout before it expires.
- `fail_timed_out_task_executions(now?, runner?)` marks expired `Running` executions as `Failed`.
- A losing runner receives `None` and must skip the task.
- `node_daemon` starts only after `start_assigned_task` succeeds and no longer depends on periodic renewal or Running-to-Pending recovery.

This prevents the duplicate-start race from `list_tasks(Pending) -> update_task_status(Running)` while keeping task rerun semantics explicit.

## 3. Timeout And Rerun Semantics

Timeout is a terminal failure for the current task instance:

- `Pending -> Running` happens through `start_assigned_task` for runner-owned work.
- `Running -> Failed` happens when `timeout_at < now` and `fail_timed_out_task_executions` is invoked.
- `Running -> Pending` is not part of this branch.
- Re-running work should create a new task instance, not reuse the same task id.
- Terminal tasks are not restarted by TaskManager.
- `Canceled` always wins over local executor completion.

This matches the product direction that the scheduler assigns executors, while TaskManager guards state transitions and records timeout failure.

## 4. Event Semantics

TaskManager event semantics:

- `task_ready` means "there may be runnable work for this runner".
- `task_ready` does not grant ownership.
- Consumers must call `start_assigned_task` before starting new `Pending` work.
- Consumers must re-read TaskManager state after any event.
- Missed kevents are acceptable because polling remains the correctness fallback.
- Task changed events are notification and UI acceleration only; event payloads are not the authority.
- `fail_timed_out_task_executions` publishes task status change events, not `task_ready`.

Recommended consumer loop:

1. Subscribe `/task_mgr/runner/{runner}/task_ready` if kevent is available.
2. Periodically sweep `list_tasks(runner, status=Pending)`.
3. For every candidate, call `start_assigned_task(id, runner)`.
4. Only start execution if `start_assigned_task` returns a task.
5. Treat `None` as a normal race loss.

## 5. Runner Permission Model

Current compatibility behavior:

- General TaskManager read/write APIs still keep the existing empty-context compatibility path.
- Runner ownership APIs are tightened: `start_assigned_task`, `extend_task_execution`, and `fail_timed_out_task_executions` require a non-empty token context.
- `start_assigned_task` is not authorized by generic task write permission. It is authorized by runner ownership.

Beta2.2 runner ownership rule:

- `kernel` / `system` app context can manage any runner.
- A service whose `appid` equals the `runner` can manage that runner.
- `node-daemon` can manage runner `<runner>` when the token `sub` equals `<runner>`.
- A `node-daemon` token with `sub = kernel` is accepted for boot compatibility.
- `start_assigned_task` still atomically guards on `task.runner == runner` and `task.status == Pending`.
- `extend_task_execution` checks caller runner authority before accepting the execution token.
- `fail_timed_out_task_executions(runner = None)` is system-only; runner-scoped timeout failure requires authority for that runner.

Remaining compatibility boundary:

- Empty context is still allowed for existing general task read/write paths.
- Tightening general task APIs is a separate change because Desktop, tests, and legacy service callers may still rely on that compatibility.

## 6. Persisted Execution Fields

Implemented DB fields:

- `execution_token`: opaque token returned by `start_assigned_task`.
- `assigned_executor`: runner id that started the task.
- `execution_started_at`: start timestamp.
- `timeout_at`: timestamp after which the running execution should fail.
- `last_execution_update_at`: last accepted execution timeout update timestamp.

These are database fields, not only JSON inside `Task.data`, because timeout checks need indexed, backend-portable queries across Sqlite and Postgres.

Implemented APIs:

- `start_assigned_task(id, runner, timeout_ms?) -> { task, execution_token, timeout_at }`
- `extend_task_execution(id, execution_token, timeout_ms?) -> { timeout_at }`
- `fail_timed_out_task_executions(now?, runner?) -> { failed_count }`

Not implemented:

- `complete_task` / `update_task` execution-token ownership checks.
- Background timeout sweep.
- Automatic retry/rerun policy.

## 7. opendan Migration Boundary

Do not directly copy node_daemon's simple start flow into opendan.

node_daemon flow:

- consumes only `Pending` thunk tasks
- starts one process
- reports final status

opendan task inbox flow:

- sweeps `Pending`, `WaitingForApproval`, `Running`, `Paused`, and `Canceled`
- can recover an existing bound work session
- can resume after human input
- can route a task into a session
- may need to wake an existing session instead of starting new execution

Required opendan design before migration:

- Use `start_assigned_task` only for the `Pending` path that starts a new session.
- Do not start `WaitingForApproval`; it is waiting for a child human input task.
- Do not start `Running`; it may represent an existing session that needs recovery/wake.
- Preserve `Paused` and `Canceled` control reflection behavior.
- Make `execution.session_id` the idempotency key for duplicate sweeps.

Until that design is complete, opendan should continue to use its current recovery-oriented logic.

## 8. Implementation Phases

### Phase A: Assigned Executor Start Guard

Status: implemented in current branch.

- Add `start_assigned_task`.
- Add DB conditional update.
- Use it in node_daemon.
- Document event semantics.
- Tighten runner ownership permission for runner APIs while keeping general task APIs compatible.

### Phase B: Execution Timeout Fields

Status: implemented in current branch.

- Add execution timeout fields to DB schema and migration.
- Extend API result types with `execution_token` and `timeout_at`.
- Add `extend_task_execution`.
- Add unit tests for duplicate-start prevention and timeout extension.
- Keep `start_assigned_task()` client compatibility by mapping the result back to `Option<Task>`.

### Phase C: Timed-Out Execution Failure

Status: explicit API implemented; background sweep is not implemented.

- Add TaskManager explicit timeout failure API.
- Mark expired running executions as `Failed`.
- Publish task status changed events only.
- Do not move `Running` tasks back to `Pending`.
- Leave automatic/background sweep for a separate change.
- Leave rerun policy for a separate scheduler-level change that creates new tasks.

### Phase D: opendan Migration

Separate task.

- Design opendan-specific idempotency around session binding.
- Add tests for `Pending`, `WaitingForApproval`, `Running`, `Paused`, and `Canceled`.
- Migrate only safe `Pending` startup to `start_assigned_task`.

## 9. Current Recommendation

For this branch, keep the implemented path focused:

- Phase A is complete.
- Phase B is complete.
- Phase C has explicit timeout failure, no automatic sweep.
- Phase D remains a separate opendan migration task.

Runner ownership permission tightening is included for `start_assigned_task`, `extend_task_execution`, and `fail_timed_out_task_executions`. Do not expand this branch into general TaskManager API permission tightening, background timeout sweep, retry policy, or opendan code migration before reviewing the current assigned-executor behavior in isolation.
