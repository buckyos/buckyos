# TaskManager Runner Claim / Lease Plan for Beta2.2

## 1. Background

TaskManager is the shared state layer for distributed async work. Producers create tasks with:

- `status = Pending`
- `runner = <runner id>`
- task-specific payload in `data`

Consumers discover work through two channels:

- `task_ready` kevent wake-up
- database state queried through TaskManager APIs

The key rule is that kevent is only an acceleration signal. The database state is the source of truth.

The current branch has added the first strong consistency and recovery step:

- `claim_task(id, runner)` atomically changes a matching `Pending` task to `Running`.
- The update is guarded by `id + runner + Pending`.
- Successful claim returns an opaque `claim_token` and `lease_until`.
- `heartbeat_task_claim(id, claim_token, lease_ms?)` renews an active claim.
- `requeue_stale_task_claims` can explicitly move expired `Running` claims back to `Pending`.
- A losing runner receives `None` and must skip the task.
- `node_daemon` now uses leased claim before starting a thunk task and heartbeats while the child process is running.

This prevents the basic duplicate-start race from `list_tasks(Pending) -> update_task_status(Running)`.

## 2. Current Gap

The minimal lease path now solves the basic runner failure after claim case when reclaim is invoked.

Implemented behavior:

1. Producer creates a task.
2. Runner claims it: `Pending -> Running`.
3. TaskManager persists claim metadata: `claim_token`, `claimed_by`, `claimed_at`, `lease_until`, `last_heartbeat_at`.
4. Runner heartbeats before the lease expires.
5. If the runner process or host crashes, heartbeat stops.
6. `requeue_stale_task_claims` can set expired `Running` claims back to `Pending` and publish `task_ready`.

Remaining gap:

- No background sweep is enabled yet.
- No per-task-type reclaim policy is implemented yet.
- Requeue is currently explicit and conservative.

## 3. Event Semantics

TaskManager event semantics should be documented as:

- `task_ready` means "there may be runnable work for this runner".
- `task_ready` does not grant ownership.
- Consumers must call `claim_task` before starting new `Pending` work.
- Consumers must re-read TaskManager state after any event.
- Missed kevents are acceptable because polling / sweep remains the correctness fallback.
- Task changed events are notification and UI acceleration only; event payloads are not the authority.

Recommended consumer loop:

1. Subscribe `/task_mgr/runner/{runner}/task_ready` if kevent is available.
2. Periodically sweep `list_tasks(runner, status=Pending)`.
3. For every candidate, call `claim_task(id, runner)`.
4. Only start execution if `claim_task` returns a task.
5. Treat `None` as a normal race loss.

## 4. Claim Permission Model

Current compatibility behavior:

- General TaskManager read/write APIs still keep the existing empty-context compatibility path.
- Runner ownership APIs are tightened: `claim_task`, `heartbeat_task_claim`, and `requeue_stale_task_claims` require a non-empty token context.
- `claim_task` is no longer authorized by generic task write permission. It is authorized by runner ownership.

Beta2.2 runner ownership rule:

- `kernel` / `system` app context can manage any runner.
- A service whose `appid` equals the `runner` can manage that runner.
- `node-daemon` can manage runner `<runner>` when the token `sub` equals `<runner>`.
- A `node-daemon` token with `sub = kernel` is accepted for boot compatibility.
- `claim_task` still atomically guards on `task.runner == runner` and `task.status == Pending`.
- `heartbeat_task_claim` checks caller runner authority before accepting the claim token.
- `requeue_stale_task_claims(runner = None)` is system-only; runner-scoped requeue requires authority for that runner.

Remaining compatibility boundary:

- Empty context is still allowed for existing general task read/write paths.
- Tightening general task APIs is a separate change because Desktop, tests, and legacy service callers may still rely on that compatibility.

## 5. Lease / Heartbeat Design

The next reliability step should introduce a claim lease. A lease gives TaskManager enough information to detect a dead runner and either requeue or fail the task.

Implemented persisted fields:

- `claim_token`: opaque token returned by `claim_task`.
- `claimed_by`: runner id that claimed the task.
- `claimed_at`: claim timestamp.
- `lease_until`: timestamp after which the claim is considered stale.
- `last_heartbeat_at`: last successful heartbeat timestamp.

These should be database fields, not only JSON inside `Task.data`, because reclaim needs indexed, backend-portable queries across Sqlite and Postgres.

Implemented APIs:

- `claim_task(id, runner, lease_ms?) -> { task, claim_token, lease_until }`
- `heartbeat_task_claim(id, claim_token, lease_ms?) -> { lease_until }`
- `requeue_stale_task_claims(now?, runner?) -> { requeued_count }`

Not implemented yet:

- `complete_task` / `update_task` claim-token ownership checks.
- Background reclaim sweep.
- Per-task-type reclaim policy.

Recommended state rules:

- `Pending -> Running` only through `claim_task` for runner-owned work.
- `Running -> Pending` can happen only when `lease_until < now` and reclaim policy permits retry.
- `Running -> Failed` can happen when lease expired and reclaim policy says fail.
- Terminal tasks are never reclaimed.
- `Canceled` always wins over heartbeat and reclaim.

Recommended default timeout:

- Start with a conservative default such as 60 seconds.
- Runner can request a longer lease for long setup phases.
- Heartbeat interval should be less than half the lease duration.

## 6. Reclaim Policy

Not every task is safe to retry.

Recommended policy:

- `retry`: set stale `Running` task back to `Pending` and publish `task_ready`.
- `fail`: mark stale `Running` task as `Failed`.
- `manual`: leave task `Running` but add a note/event for operator intervention.

For Beta2.2, use conservative defaults:

- `scheduler.dispatch_thunk`: `retry` is reasonable because node_daemon writes execution output under task-specific work dirs and should be made idempotent.
- `agent.delegate`: do not auto-retry yet; this belongs to opendan session recovery design.
- Unknown task types: `manual` or `fail`, not automatic retry.

The policy can initially live in task `data`, but the reclaim query should still use real lease columns.

## 7. opendan Migration Boundary

Do not directly copy node_daemon's simple claim flow into opendan.

node_daemon flow:

- consumes only `Pending` thunk tasks
- starts one process
- reports final status

opendan task inbox flow:

- sweeps `Pending`, `WaitingForApproval`, `Running`, `Paused`, and `Canceled`
- can recover an existing bound work session
- can resume after human input
- can route a task into a session
- may need to wake an existing session instead of claiming new execution

Required opendan design before migration:

- Claim only the `Pending` path that starts a new session.
- Do not claim `WaitingForApproval`; it is waiting for a child human input task.
- Do not claim `Running`; it may represent an existing session that needs recovery/wake.
- Preserve `Paused` and `Canceled` control reflection behavior.
- Make `execution.session_id` the idempotency key for duplicate sweeps.

Until that design is complete, opendan should continue to use its current recovery-oriented logic.

## 8. Proposed Implementation Phases

### Phase A: Claim Semantics

Status: implemented in current branch.

- Add `claim_task`.
- Add DB conditional update.
- Use it in node_daemon.
- Document event semantics.
- Tighten runner ownership permission for claim-style APIs while keeping general task APIs compatible.

### Phase B: Lease Schema and Heartbeat

Status: implemented in current branch.

- Add lease fields to DB schema and migration.
- Extend API result types with `claim_token` and `lease_until`.
- Add `heartbeat_task_claim`.
- Add unit tests for stale claim detection and heartbeat extension.
- Keep `claim_task()` client compatibility by mapping the new result back to `Option<Task>`.
- node_daemon renews the claim while a thunk child process is running.

### Phase C: Reclaim Sweep

Status: minimal explicit reclaim API implemented; background sweep and policy are not implemented.

- Add TaskManager explicit reclaim API for stale claims.
- Requeue expired claims and republish `task_ready`.
- Leave automatic/background sweep for a separate change.
- Leave per-task-type reclaim policy for a separate change.

Still pending:

- Add background sweep for stale claims.
- Implement per-task-type reclaim policy.
- Mark unsafe stale tasks as `Failed` or leave for manual intervention.

### Phase D: opendan Migration

Separate task.

- Design opendan-specific idempotency around session binding.
- Add tests for `Pending`, `WaitingForApproval`, `Running`, `Paused`, and `Canceled`.
- Migrate only safe `Pending` startup to `claim_task`.

## 9. Current Recommendation

For this branch, keep the implemented lease path focused:

- Phase A is complete.
- Phase B is complete.
- Phase C has only explicit reclaim, no automatic sweep or policy engine.
- Phase D remains a separate opendan migration task.

Runner ownership permission tightening is already included for `claim_task`,
`heartbeat_task_claim`, and `requeue_stale_task_claims`. Do not expand this
branch further into general TaskManager API permission tightening, background
sweep, per-task policy, or opendan code migration before reviewing the current
lease behavior in isolation.
