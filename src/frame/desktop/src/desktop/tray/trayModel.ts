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
import {
  dismissTasks,
  isTaskFinished,
  readDismissedTasks,
  undismissedTasks,
  type ActivityMessageItem,
  type ActivityNotifyItem,
  type ActivityTaskItem,
  type DismissedTasks,
} from '../../shared/activity/model.ts'

export type TrayLoadStatus = 'idle' | 'loading' | 'ready' | 'error'

export interface TraySnapshot {
  messages: { status: TrayLoadStatus; unread: number; items: ActivityMessageItem[] }
  notifications: {
    status: TrayLoadStatus
    /** Open notifications, of which `notifies` lists the newest. */
    open: number
    notifies: ActivityNotifyItem[]
    /** Running tasks first, then finished ones the viewer has not dismissed. */
    tasks: ActivityTaskItem[]
  }
}

/** What the bell counts: open notifications plus every task row. */
export function notificationCount(snapshot: TraySnapshot): number {
  return snapshot.notifications.open + snapshot.notifications.tasks.length
}

/* ── Dismissed finished tasks (per device, like a phone's notification shade) ── */

const DISMISSED_KEY = 'buckyos.desktop.tray.dismissed-tasks.v1'

function readDismissed(): DismissedTasks {
  try {
    return readDismissedTasks(JSON.parse(window.localStorage.getItem(DISMISSED_KEY) ?? '{}'))
  } catch {
    return {}
  }
}

function writeDismissed(dismissed: DismissedTasks) {
  try {
    window.localStorage.setItem(DISMISSED_KEY, JSON.stringify(dismissed))
  } catch {
    // Not persisted (private window): the dismissal lasts for this page.
  }
}

/* ── Store ── */

/** Started by `startTrayRuntime`; reports through `TrayRuntimeSink`. */
export interface TrayRuntimeSink {
  setMessages(messages: TraySnapshot['messages']): void
  setNotifies(status: TrayLoadStatus, notifies: ActivityNotifyItem[], open: number): void
  /** Every listed task; the store drops the dismissed finished ones. */
  setTasks(tasks: ActivityTaskItem[]): void
  /** Ask the runtime to read everything again soon (a tray opened, the page came back). */
  onRefreshRequest(listener: () => void): () => void
}

const emptySnapshot: TraySnapshot = {
  messages: { status: 'idle', unread: 0, items: [] },
  notifications: { status: 'idle', open: 0, notifies: [], tasks: [] },
}

export class TrayStore {
  private snapshot: TraySnapshot = emptySnapshot
  private listeners = new Set<() => void>()
  private refreshListeners = new Set<() => void>()
  private allTasks: ActivityTaskItem[] = []
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
    const tasks = undismissedTasks(this.allTasks, this.dismissed)
    this.patch({ notifications: { ...this.snapshot.notifications, tasks } })
  }

  private readonly sink: TrayRuntimeSink = {
    setMessages: (messages) => this.patch({ messages }),
    setNotifies: (status, notifies, open) =>
      this.patch({ notifications: { ...this.snapshot.notifications, status, notifies, open } }),
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
    this.dismiss([taskId])
  }

  /** Hides every finished task currently listed. */
  dismissFinishedTasks() {
    this.dismiss(this.allTasks.filter((task) => isTaskFinished(task.state)).map((task) => task.taskId))
  }

  private dismiss(taskIds: string[]) {
    this.dismissed = dismissTasks(this.dismissed, taskIds)
    writeDismissed(this.dismissed)
    this.publishTasks()
  }
}

export const trayStore = new TrayStore()

/** The tray snapshot; runs the readers while `enabled` (the mobile status bar is mounted). */
export function useTray(enabled: boolean): TraySnapshot {
  useEffect(() => (enabled ? trayStore.retain() : undefined), [enabled])
  return useSyncExternalStore(trayStore.subscribe, trayStore.getSnapshot)
}
