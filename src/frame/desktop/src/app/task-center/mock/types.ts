/* ── TaskCenter mock types ── */

export type TaskStatus =
  | 'pending'
  | 'running'
  | 'paused'
  | 'completed'
  | 'failed'
  | 'cancelled'

export type TaskType = 'one-time' | 'scheduled' | 'download' | 'sync' | 'install' | 'workflow'

export type TaskSource = 'system' | 'user' | 'agent' | 'app'

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
