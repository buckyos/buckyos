/* ── What the user is told about, from three sources ──
 *
 * - **Messages**: conversations with unread messages, one row each (no system notification sessions, no muted ones).
 * - **Notifications**: System Notify (`notify.list`).
 * - **Tasks**: those whose Task Schema asks for system notifications (`presentation_schema.system_notify`).
 *
 * The Shell's status-bar trays show the first two blocks apart; Activity is one list, newest first.
 */

import type { NotifyCategory, NotifyKind, NotifyLevel, NotifyStatus } from 'buckyos'

export type ActivitySource = 'message' | 'notify' | 'task'

export interface ActivityMessageItem {
  sessionId: string
  entityId: string
  /** The conversation: entity name, plus the session title when the entity has several. */
  title: string
  avatar?: string
  isGroup: boolean
  /** Who sent the latest message, when it is not obvious from the title (groups). */
  senderName?: string
  text: string
  unreadCount: number
  timestamp: number
}

export interface ActivityNotifyItem {
  notifyId: string
  title: string
  message?: string
  category: NotifyCategory
  kind: NotifyKind
  level: NotifyLevel
  status: NotifyStatus
  /** An `input` still waiting for the answer. */
  needsAnswer: boolean
  /** Who sent it: the Agent, or the App. */
  appId?: string
  agentDid?: string
  /** The sender's own page about it (`source.link`). */
  link?: string
  timestamp: number
}

export type ActivityTaskState = 'running' | 'waiting' | 'paused' | 'completed' | 'failed' | 'canceled'

export interface ActivityTaskItem {
  taskId: string
  title: string
  message?: string
  state: ActivityTaskState
  /** 0–100 when the runner reports a total. */
  progress: number | null
  timestamp: number
}

export type ActivityEntry =
  | { kind: 'message'; key: string; timestamp: number; item: ActivityMessageItem }
  | { kind: 'notify'; key: string; timestamp: number; item: ActivityNotifyItem }
  | { kind: 'task'; key: string; timestamp: number; item: ActivityTaskItem }

/** One list of the given rows, newest first; rows of the same time keep the order message, notification, task. */
export function mergeActivity(rows: {
  messages?: readonly ActivityMessageItem[]
  notifies?: readonly ActivityNotifyItem[]
  tasks?: readonly ActivityTaskItem[]
}): ActivityEntry[] {
  return [
    ...(rows.messages ?? []).map((item): ActivityEntry => ({ kind: 'message', key: `message-${item.sessionId}`, timestamp: item.timestamp, item })),
    ...(rows.notifies ?? []).map((item): ActivityEntry => ({ kind: 'notify', key: `notify-${item.notifyId}`, timestamp: item.timestamp, item })),
    ...(rows.tasks ?? []).map((item): ActivityEntry => ({ kind: 'task', key: `task-${item.taskId}`, timestamp: item.timestamp, item })),
  ].sort((a, b) => b.timestamp - a.timestamp)
}

export function isTaskFinished(state: ActivityTaskState): boolean {
  return state === 'completed' || state === 'failed' || state === 'canceled'
}

/** Running tasks first (newest first), then finished ones by when they finished. */
export function orderTasks(tasks: readonly ActivityTaskItem[]): ActivityTaskItem[] {
  const newestFirst = (a: ActivityTaskItem, b: ActivityTaskItem) => b.timestamp - a.timestamp
  return [
    ...tasks.filter((task) => !isTaskFinished(task.state)).sort(newestFirst),
    ...tasks.filter((task) => isTaskFinished(task.state)).sort(newestFirst),
  ]
}

export function badgeLabel(count: number): string {
  return count > 99 ? '99+' : String(count)
}

/* ── Where a row leads (pages of the Zone's root) ── */

/** The MessageHub page of one conversation. */
export function messageSessionPath(item: Pick<ActivityMessageItem, 'entityId' | 'sessionId'>): string {
  const params = new URLSearchParams({ entityId: item.entityId, sessionId: item.sessionId })
  return `/messagehub?${params.toString()}`
}

/** The MessageHub card of a notification. */
export function notifyPath(notifyId: string): string {
  return `/messagehub?notify=${encodeURIComponent(notifyId)}`
}

/** The Task Center page of a task. */
export function taskPath(taskId: string): string {
  return `/taskcenter?taskid=${encodeURIComponent(taskId)}`
}

/** The row's page in the Zone's own UI; a notification may also have one of its sender's (`item.link`). */
export function activityPath(entry: ActivityEntry): string {
  switch (entry.kind) {
    case 'message':
      return messageSessionPath(entry.item)
    case 'notify':
      return notifyPath(entry.item.notifyId)
    case 'task':
      return taskPath(entry.item.taskId)
  }
}

/* ── Labels ──
 * An i18n key of the Desktop's locale files, its English text and the values to fill in. A caller that has no such
 * key translates by its own table of these keys.
 */

export interface ActivityLabel {
  key: string
  text: string
  values?: Record<string, string | number>
}

export function notifyCategoryLabel(category: NotifyCategory): ActivityLabel {
  if (category === 'todo') return { key: 'tray.notify.todo', text: 'To-do' }
  if (category === 'agent') return { key: 'tray.notify.agent', text: 'Agent' }
  return { key: 'tray.notify.app', text: 'App' }
}

/** What the row says about the task's state: a label, or the runner's own words. */
export function taskStateLabel(task: Pick<ActivityTaskItem, 'state' | 'message'>): ActivityLabel | string {
  switch (task.state) {
    case 'completed':
      return { key: 'tray.task.completed', text: 'Completed' }
    case 'failed':
      return task.message
        ? { key: 'tray.task.failedWith', text: 'Failed: {{detail}}', values: { detail: task.message } }
        : { key: 'tray.task.failed', text: 'Failed' }
    case 'canceled':
      return { key: 'tray.task.canceled', text: 'Canceled' }
    case 'waiting':
      return task.message || { key: 'tray.task.waiting', text: 'Waiting' }
    case 'paused':
      return { key: 'tray.task.paused', text: 'Paused' }
    default:
      return task.message || { key: 'tray.task.running', text: 'In progress' }
  }
}

/** The row's text of a conversation and who wrote it (groups); without `showContent` only how many messages are new. */
export function messagePreview(
  item: Pick<ActivityMessageItem, 'text' | 'senderName' | 'unreadCount'>,
  showContent = true,
): { sender?: string; text: ActivityLabel | string } {
  if (!showContent) {
    return {
      text: item.unreadCount === 1
        ? { key: 'tray.messages.newOne', text: '1 new message' }
        : { key: 'tray.messages.newCount', text: '{{count}} new messages', values: { count: item.unreadCount } },
    }
  }
  return {
    ...(item.senderName ? { sender: item.senderName } : {}),
    text: item.text || { key: 'tray.messages.noPreview', text: 'New message' },
  }
}

/* ── Dismissed finished tasks: kept by the caller, per device, like a phone's notification shade ── */

/** Task id → when it was dismissed. */
export type DismissedTasks = Record<string, number>

/** Dismissals are kept a little longer than a finished task stays listed. */
const DISMISSED_RETENTION_MS = 10 * 24 * 60 * 60 * 1000

/** `dismissed` with `taskIds` added, without the ones old enough to be forgotten. */
export function dismissTasks(dismissed: DismissedTasks, taskIds: readonly string[], now = Date.now()): DismissedTasks {
  const next = { ...dismissed }
  for (const taskId of taskIds) next[taskId] = now
  return Object.fromEntries(Object.entries(next).filter(([, at]) => at >= now - DISMISSED_RETENTION_MS))
}

/** Dismissals read back from storage; anything else in them is dropped. */
export function readDismissedTasks(raw: unknown): DismissedTasks {
  if (!raw || typeof raw !== 'object' || Array.isArray(raw)) return {}
  return Object.fromEntries(Object.entries(raw).filter((entry): entry is [string, number] => typeof entry[1] === 'number'))
}

/** The tasks still listed: a dismissed one only goes once it is finished. */
export function undismissedTasks(tasks: readonly ActivityTaskItem[], dismissed: DismissedTasks): ActivityTaskItem[] {
  return tasks.filter((task) => !(isTaskFinished(task.state) && dismissed[task.taskId] !== undefined))
}
