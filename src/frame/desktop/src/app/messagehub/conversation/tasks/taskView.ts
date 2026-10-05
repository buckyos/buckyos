import type { TaskWatchNode } from '../../../../api/task_mgr'
import type { TaskChildren, TaskSnapshot } from './taskWatch'

export type TaskTone = 'active' | 'waiting' | 'paused' | 'success' | 'failed' | 'canceled' | 'unavailable'

/** An i18n key with its variables; the component resolves it. */
export interface TaskText {
  key: string
  vars?: Record<string, string | number>
}

export interface TaskStateView {
  tone: TaskTone
  label: TaskText
}

/** What the default two lines of a bubble's task area say. */
export interface TaskAreaView extends TaskStateView {
  /** Line 1 after the state: the task's one-line current activity. */
  activity?: string
  /** Line 2, joined with " · ": sub-task counts and the wait reason. */
  secondary: Array<TaskText | string>
  /** Line 2 ends with "updated …" for a task that is still working. */
  updatedAt?: number
}

/** How long a finished task whose final edit has not arrived is shown as "reply syncing". */
const REPLY_SYNC_WINDOW_MS = 10 * 60_000
/** A placeholder is posted well before its Turn ends; an ordinary reply is created as the Turn ends. */
const PLACEHOLDER_LEAD_MS = 2_000

/**
 * State of one task node. A pending control request is shown as "stopping…"
 * (never as canceled) until the runner reports the terminal state.
 */
export function taskNodeState(node: TaskWatchNode): TaskStateView {
  if (node.phase === 'Terminal') {
    if (node.outcome === 'Failed') return { tone: 'failed', label: { key: 'messagehub.task.state.failed' } }
    if (node.outcome === 'Canceled') return { tone: 'canceled', label: { key: 'messagehub.task.state.canceled' } }
    return { tone: 'success', label: { key: 'messagehub.task.state.succeeded' } }
  }
  if (node.pendingControl === 'Cancel') return { tone: 'waiting', label: { key: 'messagehub.task.state.stopping' } }
  if (node.pendingControl === 'Pause') return { tone: 'waiting', label: { key: 'messagehub.task.state.pausing' } }
  if (node.pendingControl === 'Resume') return { tone: 'waiting', label: { key: 'messagehub.task.state.resuming' } }
  if (node.phase === 'Paused') return { tone: 'paused', label: { key: 'messagehub.task.state.paused' } }
  if (node.phase === 'Waiting') {
    const kind = node.waitReason?.kind
    if (kind === 'ChildTask') return { tone: 'waiting', label: { key: 'messagehub.task.state.waitingChild' } }
    if (kind === 'Dependency' || kind === 'External') return { tone: 'waiting', label: { key: 'messagehub.task.state.waitingExternal' } }
    return { tone: 'waiting', label: { key: 'messagehub.task.state.waiting' } }
  }
  if (node.phase === 'Running') return { tone: 'active', label: { key: 'messagehub.task.state.running' } }
  return { tone: 'active', label: { key: 'messagehub.task.state.accepted' } }
}

function childCounts(children: TaskChildren | undefined): Array<TaskText> {
  const items = children?.items ?? []
  if (items.length === 0) return []
  const more = children?.nextCursor ? '+' : ''
  const count = (value: number) => `${value}${more}`
  const running = items.filter(item => item.phase === 'Running' || item.phase === 'Accepted' || item.phase === 'Promised').length
  const waiting = items.filter(item => item.phase === 'Waiting' || item.phase === 'Paused').length
  const failed = items.filter(item => item.phase === 'Terminal' && item.outcome !== 'Succeeded').length
  const lines: TaskText[] = []
  if (running > 0) lines.push({ key: 'messagehub.task.children.running', vars: { count: count(running) } })
  if (waiting > 0) lines.push({ key: 'messagehub.task.children.waiting', vars: { count: count(waiting) } })
  if (running === 0 && waiting === 0) lines.push({ key: 'messagehub.task.children.finished', vars: { count: count(items.length) } })
  if (failed > 0) lines.push({ key: 'messagehub.task.children.failed', vars: { count: count(failed) } })
  return lines
}

/**
 * The default task area of a bubble, or null when none is shown: the task
 * is unknown, unreadable or cleaned up, or it is an ordinary quick reply (a
 * successful task without sub-tasks). `message` is the anchor: whether its
 * final edit arrived and when it was created.
 */
export function taskAreaView(snapshot: TaskSnapshot | undefined, children: TaskChildren | undefined, message: { edited: boolean; createdAt: number }, now: number): TaskAreaView | null {
  const task = snapshot?.status === 'ready' ? snapshot.task : undefined
  if (!snapshot || !task) return null
  const terminal = task.phase === 'Terminal'
  // A failed read never keeps saying "running".
  if (snapshot.stale && !terminal) return { tone: 'unavailable', label: { key: 'messagehub.task.state.unavailable' }, secondary: [{ key: 'messagehub.task.staleHint' }] }
  const counts = childCounts(children)
  if (terminal && task.outcome === 'Succeeded') {
    const completedAt = task.completedAt ?? task.updatedAt
    if (!message.edited && message.createdAt + PLACEHOLDER_LEAD_MS < completedAt && now - completedAt < REPLY_SYNC_WINDOW_MS) return { tone: 'active', label: { key: 'messagehub.task.state.replySyncing' }, secondary: counts }
    if (counts.length === 0) return null
  }
  const state = taskNodeState(task)
  const activity = terminal && task.outcome === 'Failed' ? task.error?.message || task.message : task.message
  const secondary: Array<TaskText | string> = [...counts]
  if (!terminal && task.phase === 'Waiting' && task.waitReason?.message && task.waitReason.message !== activity) secondary.push(task.waitReason.message)
  return { ...state, activity: activity?.trim() || undefined, secondary, updatedAt: terminal ? undefined : task.updatedAt }
}
