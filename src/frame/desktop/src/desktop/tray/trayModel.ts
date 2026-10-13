/**
 * The mobile status bar's two trays:
 *
 * - **Messages** (red count): unread conversations of MessageHub, without the
 *   system notification sessions and muted conversations. A row opens its
 *   session.
 * - **Notifications** (blue count): open System Notify notifications
 *   (unread notices, unanswered inputs) and the tasks whose Task Schema asks
 *   for system notifications (`presentation_schema.system_notify`): running
 *   ones with their progress, finished ones until dismissed. A notification
 *   opens its card in MessageHub, a task its page in the Task Center.
 *
 * This module is what the shell loads up front: the snapshot, the store and
 * the dismissed-task list. The readers (MessageHub store, msg-center,
 * TaskMgr) live in `trayRuntime.ts` and load only once the tray starts.
 */
import { useEffect, useSyncExternalStore } from 'react'

export type TrayLoadStatus = 'idle' | 'loading' | 'ready' | 'error'

export interface TrayMessageItem {
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

export interface TrayNotifyItem {
  notifyId: string
  title: string
  message?: string
  category: 'todo' | 'agent' | 'app'
  /** An `input` still waiting for the answer. */
  needsAnswer: boolean
  timestamp: number
}

export type TrayTaskState = 'running' | 'waiting' | 'paused' | 'completed' | 'failed' | 'canceled'

export interface TrayTaskItem {
  taskId: string
  title: string
  message?: string
  state: TrayTaskState
  /** 0–100 when the runner reports a total. */
  progress: number | null
  timestamp: number
}

export interface TraySnapshot {
  messages: { status: TrayLoadStatus; unread: number; items: TrayMessageItem[] }
  notifications: {
    status: TrayLoadStatus
    notifies: TrayNotifyItem[]
    /** Running tasks first, then finished ones the viewer has not dismissed. */
    tasks: TrayTaskItem[]
  }
}

export function isTaskFinished(state: TrayTaskState): boolean {
  return state === 'completed' || state === 'failed' || state === 'canceled'
}

/** What the bell counts: open notifications plus every task row. */
export function notificationCount(snapshot: TraySnapshot): number {
  return snapshot.notifications.notifies.length + snapshot.notifications.tasks.length
}

export function badgeLabel(count: number): string {
  return count > 99 ? '99+' : String(count)
}

/** The MessageHub page of one conversation. */
export function messageSessionPath(item: Pick<TrayMessageItem, 'entityId' | 'sessionId'>): string {
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

/* ── Dismissed finished tasks (per device, like a phone's notification shade) ── */

const DISMISSED_KEY = 'buckyos.desktop.tray.dismissed-tasks.v1'
/** Dismissals are kept a little longer than a finished task stays listed. */
const DISMISSED_RETENTION_MS = 10 * 24 * 60 * 60 * 1000

function readDismissed(): Record<string, number> {
  try {
    const parsed: unknown = JSON.parse(window.localStorage.getItem(DISMISSED_KEY) ?? '{}')
    return parsed && typeof parsed === 'object' && !Array.isArray(parsed) ? (parsed as Record<string, number>) : {}
  } catch {
    return {}
  }
}

function writeDismissed(dismissed: Record<string, number>) {
  const cutoff = Date.now() - DISMISSED_RETENTION_MS
  const kept = Object.fromEntries(Object.entries(dismissed).filter(([, at]) => at >= cutoff))
  try {
    window.localStorage.setItem(DISMISSED_KEY, JSON.stringify(kept))
  } catch {
    // Not persisted (private window): the dismissal lasts for this page.
  }
}

/* ── Store ── */

/** Started by `startTrayRuntime`; reports through `TrayRuntimeSink`. */
export interface TrayRuntimeSink {
  setMessages(messages: TraySnapshot['messages']): void
  setNotifies(status: TrayLoadStatus, notifies: TrayNotifyItem[]): void
  /** Every listed task; the store drops the dismissed finished ones. */
  setTasks(tasks: TrayTaskItem[]): void
  /** Ask the runtime to read everything again soon (a tray opened, the page came back). */
  onRefreshRequest(listener: () => void): () => void
}

const emptySnapshot: TraySnapshot = {
  messages: { status: 'idle', unread: 0, items: [] },
  notifications: { status: 'idle', notifies: [], tasks: [] },
}

export class TrayStore {
  private snapshot: TraySnapshot = emptySnapshot
  private listeners = new Set<() => void>()
  private refreshListeners = new Set<() => void>()
  private allTasks: TrayTaskItem[] = []
  private dismissed = readDismissed()
  private users = 0
  private stopRuntime: (() => void) | null = null
  private starting = false

  subscribe = (listener: () => void) => {
    this.listeners.add(listener)
    return () => {
      this.listeners.delete(listener)
    }
  }

  getSnapshot = () => this.snapshot

  private patch(next: Partial<TraySnapshot>) {
    this.snapshot = { ...this.snapshot, ...next }
    this.listeners.forEach((listener) => listener())
  }

  private publishTasks() {
    const tasks = this.allTasks.filter(
      (task) => !(isTaskFinished(task.state) && this.dismissed[task.taskId] !== undefined),
    )
    this.patch({ notifications: { ...this.snapshot.notifications, tasks } })
  }

  private readonly sink: TrayRuntimeSink = {
    setMessages: (messages) => this.patch({ messages }),
    setNotifies: (status, notifies) =>
      this.patch({ notifications: { ...this.snapshot.notifications, status, notifies } }),
    setTasks: (tasks) => {
      this.allTasks = tasks
      this.publishTasks()
    },
    onRefreshRequest: (listener) => {
      this.refreshListeners.add(listener)
      return () => {
        this.refreshListeners.delete(listener)
      }
    },
  }

  /** Reference-counted: the runtime runs while anyone uses the tray. */
  retain(): () => void {
    this.users += 1
    if (this.users === 1) void this.start()
    let released = false
    return () => {
      if (released) return
      released = true
      this.users -= 1
      if (this.users === 0) {
        this.stopRuntime?.()
        this.stopRuntime = null
      }
    }
  }

  private async start() {
    if (this.starting || this.stopRuntime) return
    this.starting = true
    if (this.snapshot.messages.status === 'idle') {
      this.patch({
        messages: { ...this.snapshot.messages, status: 'loading' },
        notifications: { ...this.snapshot.notifications, status: 'loading' },
      })
    }
    try {
      const { startTrayRuntime } = await import('./trayRuntime')
      if (this.users > 0 && !this.stopRuntime) this.stopRuntime = startTrayRuntime(this.sink)
    } catch (error) {
      console.error('status tray unavailable', error)
      this.patch({
        messages: { ...this.snapshot.messages, status: 'error' },
        notifications: { ...this.snapshot.notifications, status: 'error' },
      })
    } finally {
      this.starting = false
    }
  }

  refresh() {
    this.refreshListeners.forEach((listener) => listener())
  }

  /** Hides a finished task from the bell on this device. */
  dismissTask(taskId: string) {
    this.dismissed = { ...this.dismissed, [taskId]: Date.now() }
    writeDismissed(this.dismissed)
    this.publishTasks()
  }

  /** Hides every finished task currently listed. */
  dismissFinishedTasks() {
    const now = Date.now()
    const next = { ...this.dismissed }
    for (const task of this.allTasks) if (isTaskFinished(task.state)) next[task.taskId] = now
    this.dismissed = next
    writeDismissed(next)
    this.publishTasks()
  }
}

export const trayStore = new TrayStore()

/** The tray snapshot; runs the readers while `enabled` (the mobile status bar is mounted). */
export function useTray(enabled: boolean): TraySnapshot {
  useEffect(() => (enabled ? trayStore.retain() : undefined), [enabled])
  return useSyncExternalStore(trayStore.subscribe, trayStore.getSnapshot)
}
