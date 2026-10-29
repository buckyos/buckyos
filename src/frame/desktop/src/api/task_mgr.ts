import {
  buckyos,
  DOWNLOAD_TASK_SCHEMA_ID,
  taskMgrErrorCode,
  taskMgrTaskEventPath,
  taskMgrTreeEventPath,
  HUMAN_APPROVAL_SCHEMA_ID,
  HUMAN_INPUT_TASK_SCHEMA_ID,
  TASK_ERR_NOT_FOUND,
  TASK_ERR_PERMISSION_DENIED,
  TASK_ERR_REVISION_CONFLICT,
  TaskControlAction as TaskControlActionKind,
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
import { fetchAgentList } from './user_mgr'
import { getSharedAppServiceStore } from '../app/app-service/hooks/use-app-service-store'
import type { InstallTask } from '../app/app-service/types'
import { TaskCenterMockStore } from './task_mgr_mock.ts'
import { createDownloadTask, type CreateDownloadInput } from './downloads.ts'
import { toProgressPercent } from './task_progress.ts'
import {
  cancelMockDownload,
  listMockDownloads,
  mockDownloadState,
  subscribeMockDownloads,
  type MockDownload,
} from './downloads_mock.ts'

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
  aiccRequest?: { method: string; params: Record<string, unknown> }
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
  /**
   * The System Notify this task answers (`input.notify.notify_id` of a
   * `human.input/v1` task). Such a task is handled on its MessageHub card
   * (`/messagehub?notify=<id>`), never by an approve / reject card here.
   */
  notifyId?: string
  /** The runner accepts a cancel request now (and none is pending). */
  cancellable?: boolean
  /** A URL download (`download/v1`): where from and, once done, where it was saved. */
  download?: TaskDownloadInfo
}

export interface TaskDownloadInfo {
  url: string
  /** Path in the user's files (`/home/<user>/Downloads/<name>`) once complete. */
  localPath?: string
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
  /**
   * Release upstream subscriptions/timers. Safe to call more than once; a
   * later `subscribe` re-attaches, which StrictMode's simulated
   * unmount/remount relies on.
   */
  dispose(): void
  getAllTasks(): Task[]
  getRunningTasks(): Task[]
  getRecentFinishedTasks(): Task[]
  getScheduledTasks(): Task[]
  getTaskById(taskId: string): Task | null
  filterTasks(opts: TaskCenterFilter): Task[]
  getPendingNotifications(): SystemNotification[]
  handleNotification(id: string, action: string): void
  getEvents(): SystemEvent[]
  /** Starts a URL download into `~/Downloads`; resolves to its task id. */
  createDownload(input: CreateDownloadInput): Promise<string>
  /** Asks the task's runner to cancel it (a `Cancel` control request). */
  cancelTask(taskId: string): Promise<void>
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
  createDownload(input: CreateDownloadInput): Promise<string>
  cancelTask(taskId: string): Promise<void>
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

/** Each Agent runs as its own constructed App whose AppId is the AgentId. */
function toTaskSource(summary: TaskSummary, agentAppIds: ReadonlySet<string>): TaskSource {
  const appId = (summary.creator.app_id ?? '').toLowerCase()
  if (agentAppIds.has(appId)) return 'agent'
  if (summary.schema_id.startsWith('agent.')) return 'agent'
  if (KERNEL_APP_IDS.has(appId)) return 'system'
  if (appId) return 'app'
  return 'user'
}

/** Source URL and saved path of a `download/v1` task, when its detail is loaded. */
function downloadInfoOf(summary: TaskSummary, detail: TaskMgrTask | null): TaskDownloadInfo | undefined {
  if (summary.schema_id !== DOWNLOAD_TASK_SCHEMA_ID || !detail) return undefined
  const request = asRecord(asRecord(detail.input).request)
  const url = asString(request.download_url) ?? (Array.isArray(request.urls) ? asString(request.urls[0]) : null)
  if (!url) return undefined
  const localPath = asString(asRecord(detail.result).local_path)
  return { url, ...(localPath ? { localPath } : {}) }
}

/** A live task whose runner declared it can be canceled, with no cancel already requested. */
function isCancellable(summary: TaskSummary, detail: TaskMgrTask | null): boolean {
  if (!detail || summary.phase === TaskPhase.Terminal) return false
  if (detail.pending_control?.action === TaskControlActionKind.Cancel) return false
  const cancel = detail.control_profile?.cancel?.kind
  return cancel === 'Safe' || cancel === 'Interrupt'
}

/** The notification an input task answers (`input.notify.notify_id`), when it is one. */
function notifyIdOf(detail: TaskMgrTask | null): string | undefined {
  const notifyId = asRecord(asRecord(detail?.input).notify).notify_id
  return typeof notifyId === 'string' && notifyId ? notifyId : undefined
}

/** Where the MessageHub card of a notification opens. */
export function notifyCardPath(notifyId: string): string {
  return `/messagehub?notify=${encodeURIComponent(notifyId)}`
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

export function toTaskCenterTask({ summary, detail }: TaskSnapshot, agentAppIds: ReadonlySet<string> = new Set()): Task {
  const status = toTaskStatus(summary.phase, summary.outcome)
  const type = toTaskType(summary.schema_id)
  const createdAt = toIsoTime(summary.created_at)
  const updatedAt = toIsoTime(summary.updated_at)
  const waitReason = detail?.wait_reason ?? summary.wait_reason ?? null
  const errorMessage = detail?.error?.message ?? null
  const data = taskPayload(detail)
  const notifyId = notifyIdOf(detail)
  const download = downloadInfoOf(summary, detail)
  const isSchedule = summary.schema_id === WORKFLOW_SCHEDULE_TASK_SCHEMA_ID

  return {
    rootTaskId: summary.root_id || summary.task_id,
    taskId: summary.task_id,
    parentTaskId: normalizeTaskId(summary.parent_id),
    source: toTaskSource(summary, agentAppIds),
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
    ...(summary.schema_id === 'aicc.compute/v1' && detail ? { aiccRequest: {
      method: summary.name.replace(/^AICC /, ''),
      params: asRecord(asRecord(asRecord(detail.input).request).request),
    } } : {}),
    children: [],
    phase: summary.phase,
    outcome: summary.outcome ?? null,
    executorKind: summary.executor_kind,
    pendingControlAction: detail?.pending_control?.action ?? summary.pending_control_action ?? null,
    waitReason,
    error: errorMessage,
    ...(notifyId ? { notifyId } : {}),
    cancellable: isCancellable(summary, detail),
    ...(download ? { download } : {}),
  }
}

function buildTaskTree(snapshots: TaskSnapshot[], agentAppIds: ReadonlySet<string>): Task[] {
  const tasks = snapshots.map((snapshot) => toTaskCenterTask(snapshot, agentAppIds))
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
 * HumanSet executor that has not been committed yet (doc §10.2). The task of
 * a System Notify is answered on its MessageHub card, so it gets no card here.
 */
function isPendingHumanTask(task: Task): boolean {
  return task.executorKind === TaskExecutorKind.HumanSet && task.phase !== TaskPhase.Terminal && !task.notifyId
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

  protected hasListeners(): boolean {
    return this.listeners.size > 0
  }

  emitChange(): void {
    this.version += 1
    this.listeners.forEach((listener) => listener())
  }
}

/** A mock download as the Task Center shows a `download/v1` task. */
function mockDownloadTaskView(download: MockDownload, now: number): Task {
  const state = mockDownloadState(download, now)
  const status: TaskStatus =
    state.outcome === 'Succeeded'
      ? 'completed'
      : state.outcome === 'Failed'
        ? 'failed'
        : state.outcome === 'Canceled'
          ? 'cancelled'
          : 'running'
  const created = new Date(download.createdAt).toISOString()
  const updated = new Date(state.updatedAt).toISOString()
  const localPath = state.outcome === 'Succeeded' ? `/home/self/Downloads/${download.filename}` : undefined
  return {
    rootTaskId: download.taskId,
    taskId: download.taskId,
    parentTaskId: null,
    source: 'user',
    type: 'download',
    status,
    title: `Download ${download.filename}`,
    summary: state.error ?? `${formatMockBytes(state.completedBytes)} / ${formatMockBytes(download.totalBytes)}`,
    createdAt: created,
    updatedAt: updated,
    startedAt: created,
    endedAt: state.completedAt ? new Date(state.completedAt).toISOString() : null,
    progress: Math.round((state.completedBytes / download.totalBytes) * 100),
    schemaType: DOWNLOAD_TASK_SCHEMA_ID,
    payload: localPath ? { state: 'completed', local_path: localPath } : { download_url: download.url },
    children: [],
    phase: state.phase === 'Terminal' ? TaskPhase.Terminal : TaskPhase.Running,
    outcome: (state.outcome as TaskOutcome | undefined) ?? null,
    error: state.error ?? null,
    cancellable: state.phase === 'Running',
    download: { url: download.url, ...(localPath ? { localPath } : {}) },
  }
}

function formatMockBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`
  const units = ['KB', 'MB', 'GB']
  let value = bytes / 1024
  let unit = 0
  while (value >= 1024 && unit < units.length - 1) {
    value /= 1024
    unit += 1
  }
  return `${value.toFixed(1)} ${units[unit]}`
}

function installationTaskView(task: InstallTask): Task {
  const time = new Date(task.updated_at).toISOString()
  return {
    rootTaskId: task.task_id,
    taskId: task.task_id,
    parentTaskId: null,
    source: 'system',
    type: 'install',
    status:
      task.outcome === 'Succeeded'
        ? 'completed'
        : task.outcome === 'Failed'
          ? 'failed'
          : task.outcome === 'Canceled'
            ? 'cancelled'
            : task.phase === 'Waiting'
              ? 'paused'
              : task.phase === 'Unknown'
                ? 'pending'
                : 'running',
    title: task.app.show_name,
    summary: `${task.stage} · ${task.owner_user_id}`,
    createdAt: new Date(task.plan.created_at).toISOString(),
    updatedAt: time,
    startedAt: time,
    endedAt: task.phase === 'Terminal' ? time : null,
    progress: task.progress,
    schemaType: task.schema_id,
    children: [],
    payload: {
      task_id: task.task_id,
      app_instance_id: task.app_instance_id,
      retry_of: task.retry_of,
      phase: task.phase,
      outcome: task.outcome,
      desired_state_committed: task.desired_state_committed,
    },
  }
}

export class TaskCenterMockModel
  extends TaskCenterMockStore
  implements TaskCenterModel
{
  private readonly subscription = new SubscribableModel()
  private readonly appServiceStore = getSharedAppServiceStore()
  /**
   * Upstream unsubscribe. Attached lazily on the first `subscribe` (so a
   * model StrictMode discards from a double-invoked `useState` initializer
   * never listens) and released only by `dispose`.
   */
  private detachAppService: (() => void) | null = null
  private detachDownloads: (() => void) | null = null
  /** Re-renders running mock downloads, whose progress follows the clock. */
  private downloadTimer: ReturnType<typeof setInterval> | null = null

  constructor() {
    super()
    this.syncInstallTasks()
    this.syncDownloadTasks()
  }

  private syncDownloadTasks(): void {
    const now = Date.now()
    const downloads = listMockDownloads().map((download) => mockDownloadTaskView(download, now))
    this.tasks = [
      ...downloads.sort((a, b) => b.createdAt.localeCompare(a.createdAt)),
      ...this.tasks.filter((task) => !task.taskId.startsWith('mock-dl-')),
    ]
    this.subscription.emitChange()
  }

  private syncInstallTasks(): void {
    this.tasks = [
      ...this.tasks.filter(
        (task) =>
          !['app.install/v1', 'app.update/v1'].includes(
            task.schemaType ?? '',
          ),
      ),
      ...this.appServiceStore.getTasks().map(installationTaskView),
    ]
    this.subscription.emitChange()
  }

  /** Idempotent, so re-subscribing on every render cannot cause churn. */
  private attach(): void {
    if (this.detachAppService) return
    this.detachAppService = this.appServiceStore.subscribe(() =>
      this.syncInstallTasks(),
    )
    this.detachDownloads = subscribeMockDownloads(() => this.syncDownloadTasks())
    this.downloadTimer = setInterval(() => {
      if (this.tasks.some((task) => task.taskId.startsWith('mock-dl-') && task.status === 'running')) {
        this.syncDownloadTasks()
      }
    }, 500)
    // Catch up on anything that changed while detached.
    this.syncInstallTasks()
    this.syncDownloadTasks()
  }

  getSnapshot(): number {
    return this.subscription.getSnapshot()
  }

  /** The scripted notifications plus a card for each pending human task, as the RPC model derives them. */
  override getPendingNotifications(): SystemNotification[] {
    return [...super.getPendingNotifications(), ...deriveNotifications(this.tasks).filter((notification) => !this.handledTasks.has(notification.id))]
  }

  private readonly handledTasks = new Set<string>()

  subscribe(listener: () => void): () => void {
    this.attach()
    return this.subscription.subscribe(listener)
  }

  dispose(): void {
    this.detachAppService?.()
    this.detachAppService = null
    this.detachDownloads?.()
    this.detachDownloads = null
    if (this.downloadTimer) clearInterval(this.downloadTimer)
    this.downloadTimer = null
  }

  async refresh(): Promise<void> {
    this.syncDownloadTasks()
  }

  async createDownload(input: CreateDownloadInput): Promise<string> {
    const taskId = await createDownloadTask(input)
    this.syncDownloadTasks()
    return taskId
  }

  async cancelTask(taskId: string): Promise<void> {
    cancelMockDownload(taskId)
    this.syncDownloadTasks()
  }

  async loadTaskDetail(): Promise<void> {
    // Mock tasks already carry their payload inline.
  }

  override handleNotification(id: string, action: string): void {
    if (notificationTaskId(id)) this.handledTasks.add(id)
    else super.handleNotification(id, action)
    this.subscription.emitChange()
  }
}

export class TaskCenterRpcModel extends SubscribableModel implements TaskCenterModel {
  private snapshots: TaskSnapshot[] = []
  private agentAppIds: ReadonlySet<string> = new Set()
  private tasks: Task[] = []
  private notifications: SystemNotification[] = []
  private events: SystemEvent[] = []
  private handledNotifications = new Map<string, Pick<SystemNotification, 'handledAction' | 'handledAt'>>()
  private readonly provider: TaskCenterRpcProvider
  /** The list fetch in flight, so detail loads can wait for it. */
  private pendingRefresh: Promise<void> | null = null
  /** Keeps an open Task Center current: often while a task runs, rarely when idle. */
  private pollTimer: ReturnType<typeof setTimeout> | null = null
  private lastRefreshAt = 0

  constructor(provider: TaskCenterRpcProvider = new BuckyOSTaskMgrProvider()) {
    super()
    this.provider = provider
    void this.refresh()
  }

  refresh(): Promise<void> {
    const pending = this.fetchTasks().finally(() => {
      if (this.pendingRefresh === pending) this.pendingRefresh = null
    })
    this.pendingRefresh = pending
    return pending
  }

  private async fetchTasks(): Promise<void> {
    this.lastRefreshAt = Date.now()
    const agents = fetchAgentList().then(({ data }) => data?.agents ?? null)
    try {
      this.snapshots = await this.provider.listTasks()
    } catch (error) {
      console.error('task_mgr.list_tasks failed', error)
      return
    }
    const list = await agents
    // AppId == AgentId for each Agent's constructed App; keep the last known set when the list fails.
    if (list) this.agentAppIds = new Set(list.map((agent) => agent.agent_id.toLowerCase()))
    this.rebuild()
  }

  /** Polling runs only while someone is subscribed, and a later `subscribe` restarts it. */
  override subscribe(listener: () => void): () => void {
    const unsubscribe = super.subscribe(listener)
    this.schedulePoll()
    return () => {
      unsubscribe()
      if (!this.hasListeners()) this.stopPoll()
    }
  }

  private schedulePoll(): void {
    if (this.pollTimer || !this.hasListeners()) return
    this.pollTimer = setTimeout(() => {
      this.pollTimer = null
      if (!this.hasListeners()) return
      const active = flattenTasks(this.tasks).some(
        (task) =>
          task.schemaType !== WORKFLOW_SCHEDULE_TASK_SCHEMA_ID &&
          (task.status === 'running' || task.status === 'pending'),
      )
      const due = active || Date.now() - this.lastRefreshAt >= IDLE_POLL_MS
      if (!due || (typeof document !== 'undefined' && document.hidden)) {
        this.schedulePoll()
        return
      }
      void this.refresh().finally(() => this.schedulePoll())
    }, ACTIVE_POLL_MS)
  }

  private stopPoll(): void {
    if (this.pollTimer) clearTimeout(this.pollTimer)
    this.pollTimer = null
  }

  dispose(): void {
    this.stopPoll()
  }

  async createDownload(input: CreateDownloadInput): Promise<string> {
    const taskId = await this.provider.createDownload(input)
    await this.refresh()
    return taskId
  }

  async cancelTask(taskId: string): Promise<void> {
    await this.provider.cancelTask(taskId)
    await this.refresh()
  }

  async loadTaskDetail(taskId: string): Promise<void> {
    // A deep link (`?taskid=`) can open before the initial list resolves.
    if (this.pendingRefresh) await this.pendingRefresh
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
    this.tasks = buildTaskTree(this.snapshots, this.agentAppIds)
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

/** Refresh cadence of an open Task Center while a task runs / when nothing does. */
const ACTIVE_POLL_MS = 3_000
const IDLE_POLL_MS = 30_000

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

  createDownload(input: CreateDownloadInput): Promise<string> {
    return createDownloadTask(input)
  }

  async cancelTask(taskId: string): Promise<void> {
    await this.getClient().requestControl({
      task_id: taskId,
      action: TaskControlActionKind.Cancel,
      request_id: crypto.randomUUID(),
    })
    this.details.delete(taskId)
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
      // An input task names its notification only in its input: the list links it to MessageHub.
      summary.schema_id === HUMAN_INPUT_TASK_SCHEMA_ID ||
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

// ---------------------------------------------------------------------------
// Per-task reads (MessageHub task area)
// ---------------------------------------------------------------------------
//
// A consumer that follows a handful of known task ids reads them one by one
// with the logged-in user's own TaskMgr permissions, instead of listing every
// task the way the Task Center does.

/** One node of a task tree: what `get_subtasks` returns, with millisecond times. */
export interface TaskWatchNode {
  taskId: string
  name: string
  parentId?: string
  rootId: string
  phase: 'Promised' | 'Accepted' | 'Running' | 'Waiting' | 'Paused' | 'Terminal'
  waitReason?: { kind: string; code?: string; relatedTaskId?: string; message?: string }
  outcome?: 'Succeeded' | 'Failed' | 'Canceled'
  /** A control request that has not been applied yet. */
  pendingControl?: 'Pause' | 'Resume' | 'Cancel'
  message?: string
  revision: number
  createdAt: number
  updatedAt: number
  completedAt?: number
}

/** `get_task`: the node plus its payload. */
export interface TaskWatchDetail extends TaskWatchNode {
  schemaId: string
  input?: unknown
  progress?: unknown
  result?: unknown
  error?: { code: string; message: string }
  originRef?: { kind: string; id: string }
}

export interface TaskWatchEvent {
  eventId: string
  taskId: string
  type: string
  revision: number
  at: number
}

export type TaskReadFailure = 'denied' | 'missing' | 'error'

export interface TaskWatchSource {
  getTask(taskId: string): Promise<TaskWatchDetail>
  getSubtasks(taskId: string, cursor: string | undefined, limit: number): Promise<{ tasks: TaskWatchNode[]; nextCursor?: string }>
  /** Persisted events of the whole tree under `rootId` the reader may see, oldest first. */
  listEvents(rootId: string, limit: number): Promise<TaskWatchEvent[]>
  /** `task`: `/obj/taskmgr/<id>/changed`; `tree`: `/obj/taskmgr/<root_id>/tree`. An event only says "read again". */
  subscribe(scope: 'task' | 'tree', id: string, onHint: () => void): () => void
  failure(error: unknown): TaskReadFailure
}

function toMillis(value: number): number {
  return value > 0 && value < 10_000_000_000 ? value * 1000 : value
}

function toWatchNode(task: TaskSummary | TaskMgrTask): TaskWatchNode {
  const pending = 'pending_control' in task ? task.pending_control?.action : (task as TaskSummary).pending_control_action
  return {
    taskId: task.task_id,
    name: task.name,
    parentId: task.parent_id,
    rootId: task.root_id,
    phase: task.phase,
    waitReason: task.wait_reason ? { kind: task.wait_reason.kind, code: task.wait_reason.code, relatedTaskId: task.wait_reason.related_task_id, message: task.wait_reason.message } : undefined,
    outcome: task.outcome,
    pendingControl: pending,
    message: task.message,
    revision: task.revision,
    createdAt: toMillis(task.created_at),
    updatedAt: toMillis(task.updated_at),
    completedAt: task.completed_at ? toMillis(task.completed_at) : undefined,
  }
}

export function createTaskWatchSource(): TaskWatchSource {
  let client: TaskManagerClient | null = null
  const getClient = () => client ??= buckyos.getTaskManagerClient()
  // One event stream for every watched task: a stream per task would use up
  // the browser's connections to the zone host and starve the reads.
  const listeners = new Map<string, Set<() => void>>()
  let stream: { close(): Promise<void> } | null = null
  let generation = 0
  let timer: ReturnType<typeof setTimeout> | null = null
  const reopen = () => {
    timer = null
    const mine = ++generation
    const previous = stream
    stream = null
    if (previous) void previous.close()
    const paths = [...listeners.keys()]
    if (paths.length === 0) return
    void buckyos.subscribeKEvent(paths, event => {
      if (mine !== generation) return
      for (const [path, set] of listeners) {
        if (event.eventid === path || event.eventid.startsWith(`${path}/`)) set.forEach(hint => hint())
      }
    })
      .then(subscription => { if (mine === generation) stream = subscription; else void subscription.close() })
      .catch(() => { /* polling remains the baseline */ })
  }
  const scheduleStream = () => { timer ??= setTimeout(reopen, 250) }
  return {
    async getTask(taskId) {
      const task = await getClient().getTask(taskId)
      return { ...toWatchNode(task), schemaId: task.schema_id, input: task.input, progress: task.progress, result: task.result, error: task.error ? { code: task.error.code, message: task.error.message } : undefined, originRef: task.origin_ref }
    },
    async getSubtasks(taskId, cursor, limit) {
      const page = await getClient().getSubtasks({ task_id: taskId, cursor, limit })
      return { tasks: (page.tasks ?? []).map(toWatchNode), nextCursor: page.next_cursor }
    },
    async listEvents(rootId, limit) {
      const result = await getClient().listTaskEvents({ root_id: rootId, limit })
      return (result.events ?? []).map(event => ({ eventId: event.event_id, taskId: event.task_id, type: event.event_type, revision: event.task_revision, at: toMillis(event.created_at) }))
    },
    subscribe(scope, id, onHint) {
      const path = scope === 'tree' ? taskMgrTreeEventPath(id) : taskMgrTaskEventPath(id)
      const set = listeners.get(path) ?? new Set<() => void>()
      listeners.set(path, set)
      set.add(onHint)
      if (set.size === 1) scheduleStream()
      return () => {
        set.delete(onHint)
        if (set.size === 0 && listeners.get(path) === set) {
          listeners.delete(path)
          scheduleStream()
        }
      }
    },
    failure(error) {
      const code = taskMgrErrorCode(error)
      return code === TASK_ERR_PERMISSION_DENIED ? 'denied' : code === TASK_ERR_NOT_FOUND ? 'missing' : 'error'
    },
  }
}
