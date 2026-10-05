import type { TaskWatchDetail, TaskWatchEvent, TaskWatchNode, TaskWatchSource } from '../../../api/task_mgr'
import { MOCK_AGENT_TASKS as MOCK_TASKS, MOCK_SELF_DID } from './data'

interface MockTask extends TaskWatchDetail {
  /** Who TaskMgr lets read the task: the Agent's owner (D6). */
  grantedTo: string[]
}

type TaskSeed = Partial<MockTask> & Pick<MockTask, 'taskId' | 'name' | 'phase'>

function seedTasks(now: number): MockTask[] {
  const task = (seed: TaskSeed, ageMs: number): MockTask => ({ rootId: seed.parentId ? '' : seed.taskId, revision: 1, createdAt: now - ageMs, updatedAt: now - ageMs, schemaId: 'opendan.turn/v1', grantedTo: [MOCK_SELF_DID], ...seed })
  const minutes = (value: number) => value * 60_000
  const tasks = [
    task({ taskId: MOCK_TASKS.done, name: 'Summarize yesterday\'s build failures', phase: 'Terminal', outcome: 'Succeeded', message: 'Report ready', revision: 9, completedAt: now - minutes(45), updatedAt: now - minutes(45), result: { summary: '3 failures, 2 flaky', artifacts: ['build-failures.md'] }, originRef: { kind: 'opendan.session', id: 'ui-session-1/turn-12' } }, minutes(49)),
    task({ taskId: `${MOCK_TASKS.done}.logs`, parentId: MOCK_TASKS.done, name: 'Explorer: collect CI logs', phase: 'Terminal', outcome: 'Succeeded', message: 'Collected 3 failing jobs', completedAt: now - minutes(47), updatedAt: now - minutes(47) }, minutes(48)),
    task({ taskId: `${MOCK_TASKS.done}.triage`, parentId: MOCK_TASKS.done, name: 'Work session: triage failures', phase: 'Terminal', outcome: 'Succeeded', message: '2 flaky, 1 real regression', completedAt: now - minutes(46), updatedAt: now - minutes(46) }, minutes(47)),
    task({ taskId: MOCK_TASKS.fast, name: 'Answer: current UTC time', phase: 'Terminal', outcome: 'Succeeded', revision: 3, completedAt: now - minutes(40), updatedAt: now - minutes(40) }, minutes(40)),
    task({ taskId: MOCK_TASKS.failed, name: 'Deploy the docs preview', phase: 'Terminal', outcome: 'Failed', message: 'Deploy step failed', error: { code: 'deploy_failed', message: 'Preview bucket quota exceeded' }, revision: 6, completedAt: now - minutes(30), updatedAt: now - minutes(30) }, minutes(34)),
    task({ taskId: MOCK_TASKS.foreign, name: 'Someone else\'s turn', phase: 'Running', grantedTo: [] }, minutes(20)),
    task({ taskId: MOCK_TASKS.running, name: 'Check the MessageHub release checklist', phase: 'Waiting', waitReason: { kind: 'ChildTask', message: 'Waiting for the UI test run' }, message: 'Checking the message projection', revision: 4, updatedAt: now - 20_000, originRef: { kind: 'opendan.session', id: 'ui-session-1/turn-16' } }, minutes(3)),
    task({ taskId: `${MOCK_TASKS.running}.explore`, parentId: MOCK_TASKS.running, name: 'Explorer: read relations.ts', phase: 'Terminal', outcome: 'Succeeded', message: 'Edit folding replaces the whole content', completedAt: now - 90_000, updatedAt: now - 90_000 }, 150_000),
    task({ taskId: `${MOCK_TASKS.running}.tests`, parentId: MOCK_TASKS.running, name: 'Work session: run UI tests', phase: 'Running', message: 'Running 42 Playwright specs', updatedAt: now - 15_000 }, 120_000),
    task({ taskId: `${MOCK_TASKS.running}.tests.shard`, parentId: `${MOCK_TASKS.running}.tests`, name: 'exec_bash: playwright test', phase: 'Running', message: 'messagehub.spec.ts', schemaId: 'tool.exec_bash/v1', updatedAt: now - 10_000 }, 60_000),
    task({ taskId: `${MOCK_TASKS.running}.docs`, parentId: MOCK_TASKS.running, name: 'Work session: update the protocol doc', phase: 'Waiting', waitReason: { kind: 'External', message: 'Waiting for the doc lock' }, message: 'Queued behind another edit', updatedAt: now - 40_000 }, 110_000),
  ]
  const byId = new Map(tasks.map(item => [item.taskId, item]))
  for (const item of tasks) {
    let root = item
    while (root.parentId && byId.get(root.parentId)) root = byId.get(root.parentId)!
    item.rootId = root.taskId
  }
  return tasks
}

/**
 * Scripted TaskMgr for mock mode. Reads go through the same permission check
 * a real `get_task` does: a task not granted to the logged-in viewer is
 * denied, whatever mailbox the viewer is looking at.
 */
export class MockTaskSource implements TaskWatchSource {
  private readonly tasks = new Map<string, MockTask>()
  private readonly hints = new Map<string, Set<() => void>>()
  private readonly events: TaskWatchEvent[] = []
  private offline = false
  /** Reads served so far, by method (tests assert that nothing is fetched needlessly). */
  readonly reads = { getTask: 0, getSubtasks: 0, listEvents: 0 }
  viewerDid = MOCK_SELF_DID

  constructor(now = Date.now()) {
    for (const task of seedTasks(now)) {
      this.tasks.set(task.taskId, task)
      this.events.push({ eventId: `${task.taskId}:1`, taskId: task.taskId, type: 'TaskCreated', revision: 1, at: task.createdAt })
      if (task.phase === 'Terminal') this.events.push({ eventId: `${task.taskId}:${task.revision}`, taskId: task.taskId, type: task.outcome === 'Failed' ? 'TaskFailed' : 'ResultCommitted', revision: task.revision, at: task.completedAt ?? task.updatedAt })
    }
  }

  private readable(taskId: string): MockTask {
    if (this.offline) throw Error('offline')
    const task = this.tasks.get(taskId)
    if (!task) throw Object.assign(Error('task not found'), { code: 'task_not_found' })
    if (!task.grantedTo.includes(this.viewerDid)) throw Object.assign(Error('permission denied'), { code: 'permission_denied' })
    return task
  }

  private node({ taskId, name, parentId, rootId, phase, waitReason, outcome, pendingControl, message, revision, createdAt, updatedAt, completedAt }: MockTask): TaskWatchNode {
    return { taskId, name, parentId, rootId, phase, waitReason, outcome, pendingControl, message, revision, createdAt, updatedAt, completedAt }
  }

  async getTask(taskId: string): Promise<TaskWatchDetail> {
    this.reads.getTask++
    const task: Partial<MockTask> = structuredClone(this.readable(taskId))
    delete task.grantedTo
    return task as TaskWatchDetail
  }

  async getSubtasks(taskId: string, cursor: string | undefined, limit: number) {
    this.reads.getSubtasks++
    this.readable(taskId)
    const children = [...this.tasks.values()].filter(task => task.parentId === taskId && task.grantedTo.includes(this.viewerDid)).sort((a, b) => a.createdAt - b.createdAt)
    const start = cursor ? Number(cursor) : 0
    const end = start + limit
    return { tasks: children.slice(start, end).map(task => this.node(task)), nextCursor: end < children.length ? String(end) : undefined }
  }

  async listEvents(rootId: string, limit: number): Promise<TaskWatchEvent[]> {
    this.reads.listEvents++
    this.readable(rootId)
    return this.events.filter(event => this.tasks.get(event.taskId)?.rootId === rootId).sort((a, b) => a.at - b.at).slice(-limit)
  }

  subscribe(scope: 'task' | 'tree', id: string, onHint: () => void) {
    const path = `${scope}:${id}`
    const listeners = this.hints.get(path) ?? new Set()
    this.hints.set(path, listeners)
    listeners.add(onHint)
    return () => { listeners.delete(onHint); if (listeners.size === 0) this.hints.delete(path) }
  }

  failure(error: unknown) {
    const code = (error as { code?: unknown } | null)?.code
    return code === 'permission_denied' ? 'denied' as const : code === 'task_not_found' ? 'missing' as const : 'error' as const
  }

  /** Open subscriptions, as `task:<id>` / `tree:<root>` paths. */
  subscriptions(): string[] { return [...this.hints.keys()].sort() }

  private emit(task: MockTask) {
    for (const path of [`task:${task.taskId}`, `tree:${task.rootId}`]) this.hints.get(path)?.forEach(listener => listener())
  }

  /** The runner reports a change: a new revision, its event, and the kevent hints. `silent` drops the hints (a lost event). */
  update(taskId: string, patch: Partial<MockTask>, silent = false) {
    const task = this.tasks.get(taskId)
    if (!task) return
    const now = Date.now()
    Object.assign(task, patch, { revision: task.revision + 1, updatedAt: now })
    if (task.phase === 'Terminal') task.completedAt ??= now
    this.events.push({ eventId: `${taskId}:${task.revision}`, taskId, type: task.phase === 'Terminal' ? (task.outcome === 'Failed' ? 'TaskFailed' : task.outcome === 'Canceled' ? 'TaskCanceled' : 'ResultCommitted') : patch.pendingControl ? 'ControlRequested' : patch.phase ? 'PhaseChanged' : 'ProgressReported', revision: task.revision, at: now })
    if (!silent) this.emit(task)
  }

  /** TaskMgr cleaned the task up. */
  remove(taskId: string) {
    const task = this.tasks.get(taskId)
    if (!task) return
    this.tasks.delete(taskId)
    this.emit(task)
  }

  /** Every read fails (a dropped connection) until it is switched back. */
  setOffline(offline: boolean) { this.offline = offline }
}
