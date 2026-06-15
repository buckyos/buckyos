import { buckyos } from 'buckyos'
import { isMockRuntime } from '../runtime'
import { TaskCenterMockStore } from './task_mgr_mock.ts'

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
  runner?: string | null
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

type RawTaskStatus =
  | 'Pending'
  | 'Running'
  | 'Paused'
  | 'Completed'
  | 'Failed'
  | 'Canceled'
  | 'Cancelled'
  | 'WaitingForApproval'
  | string

interface RawTaskPermissions {
  read?: string
  write?: string
}

interface RawTask {
  id: number | string
  user_id?: string
  app_id?: string
  session_id?: string
  parent_id?: number | string | null
  root_id?: string
  name?: string
  task_type?: string
  runner?: string
  status?: RawTaskStatus
  progress?: number
  message?: string | null
  data?: unknown
  permissions?: RawTaskPermissions
  created_at?: number
  updated_at?: number
}

interface TaskMgrSdkClient {
  listTasks(params?: Record<string, unknown>): Promise<RawTask[]>
  updateTaskData(id: number, data: unknown): Promise<void>
}

interface TaskCenterRpcProvider {
  listTasks(): Promise<RawTask[]>
  handleNotificationAction(task: Task, action: string): Promise<void>
}

const terminalStatuses = new Set<TaskStatus>(['completed', 'failed', 'cancelled'])

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

function normalizeTaskId(value: number | string | null | undefined): string | null {
  if (value === null || value === undefined) return null
  const text = String(value)
  return text.trim() ? text : null
}

function normalizeNumericTaskId(value: string): number | null {
  const numeric = Number(value)
  return Number.isInteger(numeric) && numeric > 0 ? numeric : null
}

function toIsoTime(value: unknown): string {
  const n = asNumber(value)
  if (n === null || n <= 0) return new Date(0).toISOString()
  const millis = n < 10_000_000_000 ? n * 1000 : n
  return new Date(millis).toISOString()
}

function toTaskStatus(status: RawTaskStatus | undefined): TaskStatus {
  switch (status) {
    case 'Running':
      return 'running'
    case 'Paused':
    case 'WaitingForApproval':
      return 'paused'
    case 'Completed':
      return 'completed'
    case 'Failed':
      return 'failed'
    case 'Canceled':
    case 'Cancelled':
      return 'cancelled'
    case 'Pending':
    default:
      return 'pending'
  }
}

function toTaskType(rawType: string | undefined, data: Record<string, unknown>): TaskType {
  const taskType = (rawType ?? '').toLowerCase()
  const schemaType = asString(data.schema_type) ?? asString(data.schemaType)

  if (taskType === 'workflow/schedule' || schemaType === 'workflow/schedule') return 'scheduled'
  if (taskType.includes('download')) return 'download'
  if (taskType.includes('install') || taskType.includes('app.')) return 'install'
  if (taskType.includes('sync')) return 'sync'
  if (taskType.includes('workflow') || taskType.includes('agent.')) return 'workflow'
  return 'one-time'
}

function toTaskSource(task: RawTask, data: Record<string, unknown>): TaskSource {
  const dataSource = asString(data.source)
  if (dataSource === 'system' || dataSource === 'user' || dataSource === 'agent' || dataSource === 'app') {
    return dataSource
  }

  const runner = (task.runner ?? '').toLowerCase()
  const appId = (task.app_id ?? '').toLowerCase()
  const taskType = (task.task_type ?? '').toLowerCase()
  if (runner.includes('agent') || runner.includes('jarvis') || taskType.includes('agent.')) return 'agent'
  if (appId && appId !== 'system' && appId !== 'task-manager') return 'app'
  if (task.user_id) return 'user'
  return 'system'
}

function clampProgress(value: unknown, status: TaskStatus): number | null {
  const n = asNumber(value)
  if (n === null) return null
  const progress = Math.max(0, Math.min(100, Math.round(n)))
  if (progress > 0 || status === 'running' || status === 'paused') return progress
  return null
}

function toScheduleStatus(status: TaskStatus): WorkflowScheduleStatus {
  switch (status) {
    case 'running':
      return 'enabled'
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

function normalizeSchedulePayload(
  raw: RawTask,
  data: Record<string, unknown>,
  status: TaskStatus,
): WorkflowScheduleTaskPayload {
  const request = asRecord(data.request)
  const result = asRecord(data.result)
  const scheduleId = asString(request.schedule_id) ?? raw.root_id ?? normalizeTaskId(raw.id) ?? ''
  const name = asString(request.name) ?? (raw.name ?? '').replace(/^workflow\/schedule\//, '')
  const schedule = request.schedule as WorkflowScheduleSpec | undefined
  const target = request.target as WorkflowScheduleTarget | undefined

  return {
    ...data,
    backendStatus: raw.status,
    request: {
      ...request,
      schedule_id: scheduleId,
      name,
      // schedule 状态的唯一真相是 Task 的 status 列（后端已统一用 TaskStatus）。
      // 不再读 request.status 字符串（它现在是 TaskStatus 词汇，非本 UI 的友好枚举）。
      status: toScheduleStatus(status),
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

export function toTaskCenterTask(raw: RawTask): Task {
  const taskId = normalizeTaskId(raw.id) ?? ''
  const data = asRecord(raw.data)
  const status = toTaskStatus(raw.status)
  const type = toTaskType(raw.task_type, data)
  const source = toTaskSource(raw, data)
  const createdAt = toIsoTime(raw.created_at)
  const updatedAt = toIsoTime(raw.updated_at ?? raw.created_at)
  const isSchedule = raw.task_type === 'workflow/schedule' || type === 'scheduled'
  const title = asString(data.title) ?? raw.name ?? `Task ${taskId}`
  const summary =
    raw.message ??
    asString(data.summary) ??
    asString(data.message) ??
    (raw.status === 'WaitingForApproval' ? 'Waiting for user approval' : '')
  const payload = isSchedule ? normalizeSchedulePayload(raw, data, status) : { ...data, backendStatus: raw.status }

  return {
    rootTaskId: raw.root_id || taskId,
    taskId,
    parentTaskId: normalizeTaskId(raw.parent_id),
    source,
    type,
    status,
    title,
    summary,
    createdAt,
    updatedAt,
    startedAt: status === 'pending' ? null : createdAt,
    endedAt: terminalStatuses.has(status) ? updatedAt : null,
    progress: clampProgress(raw.progress, status),
    schemaType: isSchedule ? 'workflow/schedule' : asString(data.schema_type) ?? asString(data.schemaType),
    payload,
    children: [],
  }
}

function buildTaskTree(rawTasks: RawTask[]): Task[] {
  const tasks = rawTasks.map(toTaskCenterTask)
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

function deriveNotifications(tasks: Task[]): SystemNotification[] {
  return flattenTasks(tasks)
    .filter((task) => task.payload.backendStatus === 'WaitingForApproval')
    .map((task) => ({
      id: `task-approval-${task.taskId}`,
      source: 'system' as const,
      title: 'Approval Required',
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

function taskDataForUpdate(task: Task): Record<string, unknown> {
  const data = { ...task.payload }
  delete data.backendStatus
  return data
}

function toHumanActionKind(action: string): 'approve' | 'reject' | null {
  switch (action) {
    case 'approve':
    case 'confirm':
      return 'approve'
    case 'reject':
    case 'dismiss':
      return 'reject'
    default:
      return null
  }
}

function notificationTaskId(id: string): string | null {
  const prefix = 'task-approval-'
  return id.startsWith(prefix) ? id.slice(prefix.length) : null
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

  override handleNotification(id: string, action: string): void {
    super.handleNotification(id, action)
    this.subscription.emitChange()
  }
}

export class TaskCenterRpcModel extends SubscribableModel implements TaskCenterModel {
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
    let rawTasks: RawTask[]
    try {
      rawTasks = await this.provider.listTasks()
    } catch (error) {
      console.error('task_mgr.listTasks failed', error)
      return
    }

    this.tasks = buildTaskTree(rawTasks)
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
      (task) => task.schemaType !== 'workflow/schedule' && (task.status === 'running' || task.status === 'paused'),
    )
  }

  getRecentFinishedTasks(): Task[] {
    return this.tasks.filter(
      (task) =>
        task.schemaType !== 'workflow/schedule' &&
        (task.status === 'completed' || task.status === 'failed' || task.status === 'cancelled'),
    )
  }

  getScheduledTasks(): Task[] {
    return this.tasks.filter((task) => task.schemaType === 'workflow/schedule' || task.type === 'scheduled')
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
        console.error('task_mgr.handleNotification failed', error)
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

export function createTaskCenterModel(options: { useMock?: boolean } = {}): TaskCenterModel {
  return (options.useMock ?? isMockRuntime()) ? new TaskCenterMockModel() : new TaskCenterRpcModel()
}

class BuckyOSTaskMgrProvider implements TaskCenterRpcProvider {
  private client: TaskMgrSdkClient | null = null

  async listTasks(): Promise<RawTask[]> {
    return this.getClient().listTasks()
  }

  async handleNotificationAction(task: Task, action: string): Promise<void> {
    const kind = toHumanActionKind(action)
    const id = normalizeNumericTaskId(task.taskId)
    if (!kind || id === null) return

    const actedAt = new Date()
    await this.getClient().updateTaskData(id, {
      ...taskDataForUpdate(task),
      human_action: {
        kind,
        actor: 'desktop',
        submitted_at: Math.floor(actedAt.getTime() / 1000),
        payload: {
          source: 'desktop',
          acted_at: actedAt.toISOString(),
        },
        // Legacy aliases kept while older consumers still read raw Task.data.
        acted_at: actedAt.toISOString(),
        source: 'desktop',
      },
    })
  }

  private getClient(): TaskMgrSdkClient {
    if (!this.client) {
      this.client = buckyos.getTaskManagerClient() as unknown as TaskMgrSdkClient
    }
    return this.client
  }
}
