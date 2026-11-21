import { buckyos, TaskExecutorKind, TaskOutcome, TaskPhase } from 'buckyos'
import type { ListTasksReq, Task, TaskManagerClient, TaskSummary } from 'buckyos'
import { TaskCenterRpcModel } from '../../../src/api/task_mgr.ts'

/** Exercise the production model with an older active task outside the newest list page. */
export async function pollOlderTask() {
  const summary = (id: string, when: number, phase: TaskPhase): TaskSummary => ({
    task_id: id, name: id, root_id: id, created_at: when, updated_at: when, revision: 1,
    phase, schema_id: 'test', schema_version: 1, executor_kind: TaskExecutorKind.App,
    creator: { user_id: 'test', app_id: 'test' }, storage_domain: 'User' as TaskSummary['storage_domain'],
  })
  const head = summary('recent', 2000, TaskPhase.Terminal)
  const old = summary('old', 1000, TaskPhase.Running)
  let snapshot = { ...old, executor: { kind: 'App', app_id: 'test' } } as Task
  let detailError = ''
  let detailReads = 0
  let hold: Promise<void> | undefined
  let started = () => {}
  const fakeClient = {
    async listTasks(req: ListTasksReq) {
      if (req.cursor) return { tasks: detailError === 'permission_denied' ? [] : [old] }
      return { tasks: [head], next_cursor: req.created_after ? undefined : 'older' }
    },
    async getTask() {
      detailReads++
      started()
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

    const gate = Promise.withResolvers<void>()
    const inFlight = Promise.withResolvers<void>()
    hold = gate.promise
    started = inFlight.resolve
    snapshot = { ...snapshot, revision: 3, phase: TaskPhase.Terminal, outcome: TaskOutcome.Succeeded }
    const before = detailReads
    const poll = model.refresh()
    await inFlight.promise
    const detail = model.loadTaskDetail('old')
    gate.resolve()
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
