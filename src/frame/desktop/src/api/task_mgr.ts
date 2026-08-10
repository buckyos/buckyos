import {
  buckyos,
  taskMgrErrorCode,
  HUMAN_APPROVAL_SCHEMA_ID,
  HUMAN_INPUT_TASK_SCHEMA_ID,
  TASK_ERR_REVISION_CONFLICT,
  TaskExecutorKind,
  TaskPhase,
  TaskOutcome,
  WORKFLOW_SCHEDULE_TASK_SCHEMA_ID,
} from 'buckyos'
import type {
  Task as TaskMgrTask,
  TaskControlAction,
  TaskManagerClient,
  TaskSummary,
  TaskWaitReason,
} from 'buckyos'
import { isMockRuntime } from '../runtime'
import { TaskCenterMockStore } from './task_mgr_mock.ts'

// The 2.0 protocol vocabulary is the websdk's; re-exported so Task Center
// code has one import site for both the view model and the wire enums.
export { TaskExecutorKind, TaskPhase, TaskOutcome }
export type { TaskControlAction, TaskSummary, TaskWaitReason }
export type { TaskMgrTask }

export type TaskStatus =
  | 'pending'
  | 'running'
  | 'paused'
  | 'completed'
  | 'failed'
  | 'cancelled'

export type TaskType = 'one-time' | 'scheduled' | 'download' | 'sync' | 'install' | 'workflow'

export type TaskSource = 'system' | 'user' | 'agent' | 'app'

export type WorkflowScheduleStatus = 'enabled' | 'paused' | 'archived' | 'error'

export type WorkflowScheduleSpec =
  | {
      kind: 'cron'
      expr: string
      timezone: string
      calendar?: string | null
      start_at?: string | number | null
      end_at?: string | number | null
    }
  | {
      kind: 'once'
      run_at: string | number
      timezone?: string | null
    }
  | {
      kind: 'run_every'
      every_sec: number
      timezone?: string | null
      start_at?: string | number | null
      end_at?: string | number | null
    }

export interface WorkflowScheduleTarget {
  task_type: string
  name_template?: string
  data_template?: Record<string, unknown>
}

export interface WorkflowScheduleTaskPayload extends Record<string, unknown> {
  request?: {
    schedule_id?: string
    name?: string
    status?: WorkflowScheduleStatus
    schedule?: WorkflowScheduleSpec
    target?: WorkflowScheduleTarget
  }
  result?: {
    next_fire_at?: string | number | null
    last_fire_at?: string | number | null
    last_task_id?: string | number | null
    last_run_id?: string | null
    consecutive_failures?: number
    last_error?: unknown
  }
}

export interface Task {
  rootTaskId: string
  taskId: string
  parentTaskId: string | null
  source: TaskSource
  type: TaskType
  status: TaskStatus
  title: string
  summary: string
  createdAt: string
  updatedAt: string
  startedAt: string | null
  endedAt: string | null
  progress: number | null
  schemaType: string | null
  payload: Record<string, unknown>
  children: Task[]
  /**
   * 2.0 composite state kept alongside the coarse `status`, so the UI can
   * render the Pausing / Canceling / Waiting projections of doc §5.4 without
   * widening the status union. Absent on mock tasks.
   */
  phase?: TaskPhase
  outcome?: TaskOutcome | null
  executorKind?: TaskExecutorKind
  pendingControlAction?: TaskControlAction | null
  waitReason?: TaskWaitReason | null
  error?: string | null
}

export type SystemNotificationAction = 'confirm' | 'dismiss' | 'approve' | 'reject'

export interface SystemNotification {
  id: string
  source: 'system'
  title: string
  summary: string
  severity: 'info' | 'warning' | 'critical'
  createdAt: string
  actions: SystemNotificationAction[]
  handled: boolean
  handledAction?: SystemNotificationAction
  handledAt?: string
}

export type SystemEventType =
  | 'task_created'
  | 'task_completed'
  | 'task_failed'
  | 'task_cancelled'
  | 'task_milestone'
  | 'notification_created'
  | 'notification_handled'

export interface SystemEvent {
  eventId: string
  eventType: SystemEventType
  source: string
  relatedRootTaskId: string | null
  relatedTaskId: string | null
  title: string
  summary: string
  occurredAt: string
  actionState: 'none' | 'handled'
  actionAt: string | null
  payload: Record<string, unknown>
}

export interface TaskCenterFilter {
  status?: TaskStatus
  type?: TaskType
  source?: TaskSource
  search?: string
}

export interface TaskCenterModel {
  getSnapshot(): number
  subscribe(listener: () => void): () => void
  refresh(): Promise<void>
  /**
   * Pull one task's Input/Result/progress. A refresh only lists metadata plus
   * the payloads the list views need, so a detail view asks for its own
   * (doc §14.3). No-op once the cached copy is current.
   */
  loadTaskDetail(taskId: string): Promise<void>
  getAllTasks(): Task[]
  getRunningTasks(): Task[]
  getRecentFinishedTasks(): Task[]
  getScheduledTasks(): Task[]
  getTaskById(taskId: string): Task | null
  filterTasks(opts: TaskCenterFilter): Task[]
  getPendingNotifications(): SystemNotification[]
  handleNotification(id: string, action: string): void
  getEvents(): SystemEvent[]
}

// ---------------------------------------------------------------------------
// TaskMgr 2.0 adapter
// ---------------------------------------------------------------------------
//
// Wire types and calls come from the websdk's `TaskManagerClient`; this file
// only owns the projection from the 2.0 protocol onto the Task Center's view
// model (coarse status, task type/source buckets, schedule payload).

/** One listed task plus its detail, when the detail read was permitted. */
interface TaskSnapshot {
  summary: TaskSummary
  detail: TaskMgrTask | null
}

interface TaskCenterRpcProvider {
  listTasks(): Promise<TaskSnapshot[]>
  loadDetail(taskId: string): Promise<TaskMgrTask | null>
  handleNotificationAction(task: Task, action: string): Promise<void>
}

/** Apps whose tasks are platform work rather than a user-installed app's. */
const KERNEL_APP_IDS = new Set([
  'task-manager',
  'task-dispatcher',
  'control-panel',
  'workflow',
  'scheduler',
  'repo-service',
  'node-daemon',
  'verify-hub',
  'kevent',
  'aicc',
  'system',
])

function isRecord(value: unknown): value is Record<string, unknown> {
  return value !== null && typeof value === 'object' && !Array.isArray(value)
}

function asRecord(value: unknown): Record<string, unknown> {
  return isRecord(value) ? value : {}
}

function asString(value: unknown): string | null {
  return typeof value === 'string' && value.trim() ? value : null
}

function asNumber(value: unknown): number | null {
  return typeof value === 'number' && Number.isFinite(value) ? value : null
}

function asStringOrNumber(value: unknown): string | number | null {
  return typeof value === 'string' || (typeof value === 'number' && Number.isFinite(value)) ? value : null
}

function normalizeTaskId(value: string | null | undefined): string | null {
  if (value === null || value === undefined) return null
  const text = String(value)
  return text.trim() ? text : null
}

function toIsoTime(value: unknown): string {
  const n = asNumber(value)
  if (n === null || n <= 0) return new Date(0).toISOString()
  const millis = n < 10_000_000_000 ? n * 1000 : n
  return new Date(millis).toISOString()
}

/**
 * Coarse list/badge status. The finer Pausing / Resuming / Canceling /
 * Waiting projections stay on `phase` + `pendingControlAction` so this union
 * (and every switch over it) keeps working.
 */
function toTaskStatus(
  phase: TaskPhase,
  outcome: TaskOutcome | null | undefined,
): TaskStatus {
  switch (phase) {
    case TaskPhase.Terminal:
      if (outcome === TaskOutcome.Failed) return 'failed'
      if (outcome === TaskOutcome.Canceled) return 'cancelled'
      return 'completed'
    case TaskPhase.Paused:
      return 'paused'
    // Waiting is still owned by its executor, but it needs the same "stalled,
    // may need you" affordance the 1.x WaitingForApproval status had.
    case TaskPhase.Waiting:
      return 'paused'
    case TaskPhase.Running:
    case TaskPhase.Accepted:
      return 'running'
    case TaskPhase.Promised:
    default:
      return 'pending'
  }
}

function toTaskType(schemaId: string): TaskType {
  if (schemaId === WORKFLOW_SCHEDULE_TASK_SCHEMA_ID) return 'scheduled'
  if (schemaId.startsWith('download/')) return 'download'
  if (schemaId.startsWith('app.')) return 'install'
  if (schemaId.includes('sync')) return 'sync'
  if (
    schemaId.startsWith('workflow.') ||
    schemaId.startsWith('agent.') ||
    schemaId.startsWith('opendan.')
  ) {
    return 'workflow'
  }
  return 'one-time'
}

function toTaskSource(summary: TaskSummary): TaskSource {
  const appId = (summary.creator.app_id ?? '').toLowerCase()
  if (appId.includes('opendan') || appId.includes('agent') || appId.includes('jarvis')) return 'agent'
  if (summary.schema_id.startsWith('agent.')) return 'agent'
  if (KERNEL_APP_IDS.has(appId)) return 'system'
  if (appId) return 'app'
  return 'user'
}

/**
 * 2.0 progress is free-form JSON owned by the runner, so pull a percentage
 * out of the shapes the platform runners actually write and give up rather
 * than guess.
 */
function toProgressPercent(progress: unknown): number | null {
  const clamp = (value: number) => Math.max(0, Math.min(100, Math.round(value)))
  const direct = asNumber(progress)
  if (direct !== null) return clamp(direct <= 1 ? direct * 100 : direct)
  if (!isRecord(progress)) return null

  const percent = asNumber(progress.percent) ?? asNumber(progress.percentage)
  if (percent !== null) return clamp(percent)

  const ratio = asNumber(progress.ratio)
  if (ratio !== null) return clamp(ratio * 100)

  const completed = asNumber(progress.completed) ?? asNumber(progress.completed_items)
  const total = asNumber(progress.total) ?? asNumber(progress.total_items)
  if (completed !== null && total !== null && total > 0) return clamp((completed / total) * 100)
  return null
}

/** Payload view of a task: result wins, then progress, then immutable input. */
function taskPayload(detail: TaskMgrTask | null): Record<string, unknown> {
  if (!detail) return {}
  if (detail.result !== undefined && detail.result !== null) return asRecord(detail.result)
  if (detail.progress !== undefined && detail.progress !== null) return asRecord(detail.progress)
  return asRecord(detail.input)
}

function waitReasonText(reason: TaskWaitReason | null | undefined): string | null {
  if (!reason) return null
  if (reason.message) return reason.message
  const detail = reason.code ? `${reason.kind}/${reason.code}` : reason.kind
  return `Waiting: ${detail}`
}

/** ScheduleStatus (the schedule's own lifecycle) -> the UI's friendly enum. */
function toScheduleStatus(value: unknown, fallback: TaskStatus): WorkflowScheduleStatus {
  switch (asString(value)) {
    case 'Running':
      return 'enabled'
    case 'Paused':
      return 'paused'
    case 'Failed':
      return 'error'
    case 'Canceled':
      return 'archived'
    default:
      break
  }
  switch (fallback) {
    case 'paused':
      return 'paused'
    case 'failed':
      return 'error'
    case 'completed':
    case 'cancelled':
      return 'archived'
    default:
      return 'enabled'
  }
}

/**
 * The schedule root task keeps its whole lifecycle inside the payload: the
 * task itself stays non-terminal, so `request.status` — not the task phase —
 * is the authority for enabled/paused/archived/error.
 */
function normalizeSchedulePayload(
  summary: TaskSummary,
  data: Record<string, unknown>,
  status: TaskStatus,
): WorkflowScheduleTaskPayload {
  const request = asRecord(data.request)
  const result = asRecord(data.result)
  const scheduleId = asString(request.schedule_id) ?? summary.root_id ?? summary.task_id
  const name = asString(request.name) ?? summary.name.replace(/^workflow\/schedule\//, '')
  const schedule = request.schedule as WorkflowScheduleSpec | undefined
  const target = request.target as WorkflowScheduleTarget | undefined

  return {
    ...data,
    request: {
      ...request,
      schedule_id: scheduleId,
      name,
      status: toScheduleStatus(request.status, status),
      schedule,
      target,
    },
    result: {
      ...result,
      next_fire_at: asStringOrNumber(result.next_fire_at),
      last_fire_at: asStringOrNumber(result.last_fire_at),
      last_task_id: asStringOrNumber(result.last_task_id),
      last_run_id: asString(result.last_run_id),
      consecutive_failures: asNumber(result.consecutive_failures) ?? 0,
      last_error: result.last_error,
    },
  }
}

export function toTaskCenterTask({ summary, detail }: TaskSnapshot): Task {
  const status = toTaskStatus(summary.phase, summary.outcome)
  const type = toTaskType(summary.schema_id)
  const createdAt = toIsoTime(summary.created_at)
  const updatedAt = toIsoTime(summary.updated_at)
  const waitReason = detail?.wait_reason ?? summary.wait_reason ?? null
  const errorMessage = detail?.error?.message ?? null
  const data = taskPayload(detail)
  const isSchedule = summary.schema_id === WORKFLOW_SCHEDULE_TASK_SCHEMA_ID

  return {
    rootTaskId: summary.root_id || summary.task_id,
    taskId: summary.task_id,
    parentTaskId: normalizeTaskId(summary.parent_id),
    source: toTaskSource(summary),
    type,
    status,
    title: summary.name || `Task ${summary.task_id}`,
    summary:
      asString(summary.message) ??
      asString(detail?.message) ??
      errorMessage ??
      waitReasonText(waitReason) ??
      '',
    createdAt,
    updatedAt,
    startedAt: summary.phase === TaskPhase.Promised ? null : createdAt,
    endedAt:
      summary.completed_at
        ? toIsoTime(summary.completed_at)
        : summary.phase === TaskPhase.Terminal
          ? updatedAt
          : null,
    progress: toProgressPercent(detail?.progress),
    schemaType: summary.schema_id,
    payload: isSchedule ? normalizeSchedulePayload(summary, data, status) : data,
    children: [],
    phase: summary.phase,
    outcome: summary.outcome ?? null,
    executorKind: summary.executor_kind,
    pendingControlAction: detail?.pending_control?.action ?? summary.pending_control_action ?? null,
    waitReason,
    error: errorMessage,
  }
}

function buildTaskTree(snapshots: TaskSnapshot[]): Task[] {
  const tasks = snapshots.map(toTaskCenterTask)
  const byId = new Map(tasks.map((task) => [task.taskId, task]))
  const roots: Task[] = []

  for (const task of tasks) {
    if (task.parentTaskId) {
      const parent = byId.get(task.parentTaskId)
      if (parent) {
        parent.children.push(task)
        continue
      }
    }
    roots.push(task)
  }

  for (const task of tasks) {
    task.children.sort((a, b) => new Date(a.createdAt).getTime() - new Date(b.createdAt).getTime())
  }

  return roots.sort((a, b) => new Date(b.updatedAt).getTime() - new Date(a.updatedAt).getTime())
}

function eventTypeFromTask(task: Task): SystemEventType {
  switch (task.status) {
    case 'completed':
      return 'task_completed'
    case 'failed':
      return 'task_failed'
    case 'cancelled':
      return 'task_cancelled'
    case 'running':
    case 'paused':
      return 'task_milestone'
    case 'pending':
    default:
      return 'task_created'
  }
}

function eventTitle(eventType: SystemEventType, task: Task): string {
  switch (eventType) {
    case 'task_completed':
      return `Task Completed: ${task.title}`
    case 'task_failed':
      return `Task Failed: ${task.title}`
    case 'task_cancelled':
      return `Task Cancelled: ${task.title}`
    case 'task_milestone':
      return `Task Updated: ${task.title}`
    default:
      return `Task Created: ${task.title}`
  }
}

function deriveEvents(tasks: Task[]): SystemEvent[] {
  const flattened = flattenTasks(tasks)
  return flattened
    .map((task) => {
      const eventType = eventTypeFromTask(task)
      return {
        eventId: `task-${task.taskId}-${eventType}`,
        eventType,
        source: task.source,
        relatedRootTaskId: task.rootTaskId,
        relatedTaskId: task.taskId,
        title: eventTitle(eventType, task),
        summary: task.summary,
        occurredAt: task.updatedAt,
        actionState: 'none' as const,
        actionAt: null,
        payload: { taskId: task.taskId },
      }
    })
    .sort((a, b) => new Date(b.occurredAt).getTime() - new Date(a.occurredAt).getTime())
}

/**
 * A pending human task is the 2.0 shape of "needs a decision from you": a
 * HumanSet executor that has not been committed yet (doc §10.2).
 */
function isPendingHumanTask(task: Task): boolean {
  return task.executorKind === TaskExecutorKind.HumanSet && task.phase !== TaskPhase.Terminal
}

function deriveNotifications(tasks: Task[]): SystemNotification[] {
  return flattenTasks(tasks)
    .filter(isPendingHumanTask)
    .map((task) => ({
      id: `task-approval-${task.taskId}`,
      source: 'system' as const,
      title:
        task.schemaType === HUMAN_APPROVAL_SCHEMA_ID ? 'Approval Required' : 'Input Required',
      summary: task.summary || task.title,
      severity: 'warning' as const,
      createdAt: task.updatedAt,
      actions: ['approve', 'reject'] as SystemNotificationAction[],
      handled: false,
    }))
}

function flattenTasks(tasks: Task[]): Task[] {
  const out: Task[] = []
  const visit = (task: Task) => {
    out.push(task)
    task.children.forEach(visit)
  }
  tasks.forEach(visit)
  return out
}

function toHumanActionKind(action: string): 'Approve' | 'Reject' | null {
  switch (action) {
    case 'approve':
    case 'confirm':
      return 'Approve'
    case 'reject':
    case 'dismiss':
      return 'Reject'
    default:
      return null
  }
}

/**
 * The one-shot Result a Task Center decision commits. Each schema owns its
 * own output contract, so shape the decision to the schema instead of
 * patching a shared `data` blob the way 1.x `updateTaskData` did.
 */
function humanCommitResult(
  task: TaskMgrTask,
  decision: 'Approve' | 'Reject',
): Record<string, unknown> {
  const actedAt = new Date().toISOString()
  if (task.schema_id === HUMAN_INPUT_TASK_SCHEMA_ID) {
    // human.input readers parse the whole TypedTaskData envelope, so the
    // immutable request has to travel with the answer.
    const input = asRecord(task.input)
    return {
      ...input,
      result: {
        response: { decision },
        answered_by: 'desktop',
        answered_at: Math.floor(Date.now() / 1000),
      },
    }
  }
  return { decision, comment: '', acted_at: actedAt, source: 'desktop' }
}

function matchesTaskFilter(task: Task, opts: TaskCenterFilter): boolean {
  if (opts.status && task.status !== opts.status) return false
  if (opts.type && task.type !== opts.type) return false
  if (opts.source && task.source !== opts.source) return false
  if (opts.search) {
    const q = opts.search.toLowerCase()
    return (
      task.title.toLowerCase().includes(q) ||
      task.taskId.toLowerCase().includes(q) ||
      task.summary.toLowerCase().includes(q)
    )
  }
  return true
}

class SubscribableModel {
  private version = 0
  private listeners = new Set<() => void>()

  getSnapshot(): number {
    return this.version
  }

  subscribe(listener: () => void): () => void {
    this.listeners.add(listener)
    return () => this.listeners.delete(listener)
  }

  emitChange(): void {
    this.version += 1
    this.listeners.forEach((listener) => listener())
  }
}

export class TaskCenterMockModel extends TaskCenterMockStore implements TaskCenterModel {
  private readonly subscription = new SubscribableModel()

  getSnapshot(): number {
    return this.subscription.getSnapshot()
  }

  subscribe(listener: () => void): () => void {
    return this.subscription.subscribe(listener)
  }

  async refresh(): Promise<void> {
    this.subscription.emitChange()
  }

  async loadTaskDetail(): Promise<void> {
    // Mock tasks already carry their payload inline.
  }

  override handleNotification(id: string, action: string): void {
    super.handleNotification(id, action)
    this.subscription.emitChange()
  }
}

export class TaskCenterRpcModel extends SubscribableModel implements TaskCenterModel {
  private snapshots: TaskSnapshot[] = []
  private tasks: Task[] = []
  private notifications: SystemNotification[] = []
  private events: SystemEvent[] = []
  private handledNotifications = new Map<string, Pick<SystemNotification, 'handledAction' | 'handledAt'>>()
  private readonly provider: TaskCenterRpcProvider

  constructor(provider: TaskCenterRpcProvider = new BuckyOSTaskMgrProvider()) {
    super()
    this.provider = provider
    void this.refresh()
  }

  async refresh(): Promise<void> {
    try {
      this.snapshots = await this.provider.listTasks()
    } catch (error) {
      console.error('task_mgr.list_tasks failed', error)
      return
    }
    this.rebuild()
  }

  async loadTaskDetail(taskId: string): Promise<void> {
    const snapshot = this.snapshots.find((item) => item.summary.task_id === taskId)
    if (!snapshot || snapshot.detail) return
    let detail: TaskMgrTask | null
    try {
      detail = await this.provider.loadDetail(taskId)
    } catch (error) {
      console.error('task_mgr.get_task failed', taskId, error)
      return
    }
    if (!detail) return
    snapshot.detail = detail
    this.rebuild()
  }

  private rebuild(): void {
    this.tasks = buildTaskTree(this.snapshots)
    this.notifications = deriveNotifications(this.tasks).map((notification) => {
      const handled = this.handledNotifications.get(notification.id)
      return handled
        ? { ...notification, handled: true, ...handled }
        : notification
    })
    this.events = deriveEvents(this.tasks)
    this.emitChange()
  }

  getAllTasks(): Task[] {
    return this.tasks
  }

  getRunningTasks(): Task[] {
    return this.tasks.filter(
      (task) =>
        task.schemaType !== WORKFLOW_SCHEDULE_TASK_SCHEMA_ID &&
        (task.status === 'running' || task.status === 'paused'),
    )
  }

  getRecentFinishedTasks(): Task[] {
    return this.tasks.filter(
      (task) =>
        task.schemaType !== WORKFLOW_SCHEDULE_TASK_SCHEMA_ID &&
        (task.status === 'completed' || task.status === 'failed' || task.status === 'cancelled'),
    )
  }

  getScheduledTasks(): Task[] {
    return this.tasks.filter(
      (task) => task.schemaType === WORKFLOW_SCHEDULE_TASK_SCHEMA_ID || task.type === 'scheduled',
    )
  }

  getTaskById(taskId: string): Task | null {
    return flattenTasks(this.tasks).find((task) => task.taskId === taskId) ?? null
  }

  filterTasks(opts: TaskCenterFilter): Task[] {
    return this.tasks.filter((task) => matchesTaskFilter(task, opts))
  }

  getPendingNotifications(): SystemNotification[] {
    return this.notifications.filter((notification) => !notification.handled)
  }

  handleNotification(id: string, action: string): void {
    const notification = this.notifications.find((item) => item.id === id)
    if (!notification) return
    const taskId = notificationTaskId(id)
    const task = taskId ? this.getTaskById(taskId) : null

    const handled = {
      handledAction: action as SystemNotificationAction,
      handledAt: new Date().toISOString(),
    }
    this.handledNotifications.set(id, handled)
    notification.handled = true
    notification.handledAction = handled.handledAction
    notification.handledAt = handled.handledAt
    this.emitChange()

    if (!task) return

    void this.provider.handleNotificationAction(task, action)
      .then(() => this.refresh())
      .catch((error) => {
        console.error('task_mgr.commit_result failed', error)
        this.handledNotifications.delete(id)
        notification.handled = false
        delete notification.handledAction
        delete notification.handledAt
        this.emitChange()
      })
  }

  getEvents(): SystemEvent[] {
    return this.events
  }
}

function notificationTaskId(id: string): string | null {
  const prefix = 'task-approval-'
  return id.startsWith(prefix) ? id.slice(prefix.length) : null
}

export function createTaskCenterModel(options: { useMock?: boolean } = {}): TaskCenterModel {
  return (options.useMock ?? isMockRuntime()) ? new TaskCenterMockModel() : new TaskCenterRpcModel()
}

/** Pages to walk before giving up; the Task Center is a human-scale list. */
const LIST_PAGE_LIMIT = 200
const MAX_LIST_PAGES = 10
/** Concurrent `get_task` reads while hydrating a refresh. */
const DETAIL_FETCH_CONCURRENCY = 8
/** Upper bound on details pulled per refresh, newest first. */
const REFRESH_HYDRATE_LIMIT = 300

class BuckyOSTaskMgrProvider implements TaskCenterRpcProvider {
  private client: TaskManagerClient | null = null
  /**
   * `list_tasks` only returns metadata (doc §14.3), so the payload-bearing
   * views read details separately. Cached by revision: a steady-state refresh
   * only re-reads the tasks that actually changed, and a detail fetched on
   * demand survives later refreshes for free.
   */
  private details = new Map<string, { revision: number; task: TaskMgrTask }>()

  async listTasks(): Promise<TaskSnapshot[]> {
    const summaries = await this.listSummaries()
    const live = new Set(summaries.map((summary) => summary.task_id))
    for (const taskId of [...this.details.keys()]) {
      if (!live.has(taskId)) this.details.delete(taskId)
    }
    return this.hydrate(summaries)
  }

  async loadDetail(taskId: string): Promise<TaskMgrTask | null> {
    const detail = await this.getTask(taskId)
    this.details.set(detail.task_id, { revision: detail.revision, task: detail })
    return detail
  }

  async handleNotificationAction(task: Task, action: string): Promise<void> {
    const decision = toHumanActionKind(action)
    if (!decision) return

    // Commit against a fresh snapshot: `expected_revision` is a CAS and the
    // cached copy may be several refreshes old.
    const attempted = await this.getTask(task.taskId)
    try {
      await this.commitResult(attempted, decision)
      return
    } catch (error) {
      if (taskMgrErrorCode(error) !== TASK_ERR_REVISION_CONFLICT) throw error
      // Someone else moved the task under us. If they committed the result,
      // the decision is already made (doc §10.2 first-CAS-wins).
      const current = await this.getTask(task.taskId)
      if (current.phase === TaskPhase.Terminal) return
      await this.commitResult(current, decision)
    }
  }

  private async commitResult(task: TaskMgrTask, decision: 'Approve' | 'Reject'): Promise<void> {
    await this.getClient().commitResult({
      task_id: task.task_id,
      result: humanCommitResult(task, decision),
      expected_revision: task.revision,
    })
  }

  private async listSummaries(): Promise<TaskSummary[]> {
    const summaries: TaskSummary[] = []
    let cursor: string | undefined
    for (let page = 0; page < MAX_LIST_PAGES; page += 1) {
      const result = await this.getClient().listTasks({ limit: LIST_PAGE_LIMIT, cursor })
      summaries.push(...(result.tasks ?? []))
      cursor = result.next_cursor
      if (!cursor) break
    }
    return summaries
  }

  /**
   * A refresh only pulls the details the list views actually render: the
   * schedule payloads and the progress of still-active tasks. Finished tasks
   * keep their metadata row until someone opens them, which is what
   * `loadDetail` is for.
   */
  private static needsRefreshDetail(summary: TaskSummary): boolean {
    return (
      summary.schema_id === WORKFLOW_SCHEDULE_TASK_SCHEMA_ID ||
      summary.phase !== TaskPhase.Terminal
    )
  }

  private async hydrate(summaries: TaskSummary[]): Promise<TaskSnapshot[]> {
    const snapshots: TaskSnapshot[] = summaries.map((summary) => {
      const cached = this.details.get(summary.task_id)
      return { summary, detail: cached?.revision === summary.revision ? cached.task : null }
    })
    const candidates = snapshots
      .filter(
        (snapshot) =>
          snapshot.detail === null && BuckyOSTaskMgrProvider.needsRefreshDetail(snapshot.summary),
      )
      .sort((a, b) => b.summary.updated_at - a.summary.updated_at)
    const stale = candidates.slice(0, REFRESH_HYDRATE_LIMIT)
    if (candidates.length > stale.length) {
      console.debug(
        `task_mgr: hydrated ${stale.length} of ${candidates.length} active tasks; ` +
          'the rest show metadata until opened',
      )
    }

    let next = 0
    const workers = Array.from(
      { length: Math.min(DETAIL_FETCH_CONCURRENCY, stale.length) },
      async () => {
        for (let index = next++; index < stale.length; index = next++) {
          const snapshot = stale[index]
          try {
            const detail = await this.getTask(snapshot.summary.task_id)
            this.details.set(detail.task_id, { revision: detail.revision, task: detail })
            snapshot.detail = detail
          } catch (error) {
            // Metadata-only visibility (doc §8.4 DataScope) and tasks that
            // vanished mid-refresh both land here: keep the summary row.
            console.debug('task_mgr.get_task failed', snapshot.summary.task_id, error)
          }
        }
      },
    )
    await Promise.all(workers)
    return snapshots
  }

  private async getTask(taskId: string): Promise<TaskMgrTask> {
    return this.getClient().getTask(taskId)
  }

  private getClient(): TaskManagerClient {
    if (!this.client) {
      this.client = buckyos.getTaskManagerClient()
    }
    return this.client
  }
}
