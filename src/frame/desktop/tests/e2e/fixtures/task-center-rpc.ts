import {
  baselineTaskControlProfile, buckyos, DEFAULT_CHILD_CONTROL_POLICY, StorageDomain,
  TASK_POLICY_PRESET_COLLABORATIVE_TREE_V1, TaskExecutorKind, TaskOutcome, TaskPhase,
} from 'buckyos'
import type { ListTasksReq, Task, TaskManagerClient, TaskSummary } from 'buckyos'
import { TaskCenterRpcModel } from '../../../src/api/task_mgr.ts'

/** Exercise the production model with an older active task outside the newest list page. */
export async function pollOlderTask() {
  const summary = (id: string, when: number, phase: TaskPhase) => ({
    task_id: id, name: id, root_id: id, created_at: when, updated_at: when, revision: 1,
    phase, schema_id: 'test', schema_version: 1, executor_kind: TaskExecutorKind.App,
    creator: { user_id: 'test', app_id: 'test' }, storage_domain: StorageDomain.User,
  } satisfies TaskSummary)
  const head = summary('recent', 2000, TaskPhase.Terminal)
  const old = summary('old', 1000, TaskPhase.Running)
  const { executor_kind, ...fields } = old
  let snapshot: Task = {
    ...fields,
    executor: { kind: executor_kind, app_id: old.creator.app_id },
    child_control_policy: DEFAULT_CHILD_CONTROL_POLICY,
    input: {},
    input_digest: 'test-digest',
    idempotency_key: old.task_id,
    runner_epoch: 1,
    control_profile: baselineTaskControlProfile(old.updated_at),
    policy_preset: TASK_POLICY_PRESET_COLLABORATIVE_TREE_V1,
    permission_boundary: false,
  }
  let detailError = ''
  let detailReads = 0
  let hold: Promise<void> | undefined
  let started: (() => void) | undefined
  const fakeClient = {
    async listTasks(req: ListTasksReq) {
      if (req.cursor) return { tasks: detailError === 'permission_denied' ? [] : [old] }
      return { tasks: [head], next_cursor: req.created_after ? undefined : 'older' }
    },
    async getTask() {
      detailReads++
      started?.()
      await hold
      if (detailError) throw new Error(detailError)
      return snapshot
    },
  }
  const original = buckyos.getTaskManagerClient
  buckyos.getTaskManagerClient = () => fakeClient as unknown as TaskManagerClient
  const model = new TaskCenterRpcModel()
  const states = []
  try {
    const first = model.refresh()
    const shared = first === model.refresh()
    await first
    states.push(model.getTaskById('old')?.phase)
    detailError = 'connection interrupted'
    await model.refresh()
    states.push(model.getTaskById('old')?.phase)
    detailError = ''
    snapshot = { ...snapshot, phase: TaskPhase.Waiting, revision: 2 }
    await model.refresh()
    states.push(model.getTaskById('old')?.phase)

    let release!: () => void
    hold = new Promise<void>((resolve) => { release = resolve })
    const inFlight = new Promise<void>((resolve) => { started = resolve })
    snapshot = { ...snapshot, revision: 3, phase: TaskPhase.Terminal, outcome: TaskOutcome.Succeeded }
    const before = detailReads
    const poll = model.refresh()
    await inFlight
    const detail = model.loadTaskDetail('old')
    release()
    await Promise.all([poll, detail])
    const sharedReads = detailReads - before
    states.push(model.getTaskById('old')?.phase)
    detailError = 'permission_denied'
    await model.loadTaskDetail('old')
    states.push(model.getTaskById('old')?.phase ?? null)
    return { states, shared, sharedReads }
  } finally {
    model.dispose()
    buckyos.getTaskManagerClient = original
  }
}
