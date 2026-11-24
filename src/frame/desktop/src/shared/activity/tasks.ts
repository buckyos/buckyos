/* ── Tasks whose schema asks for system notifications ──
 *
 * TaskMgr tasks of the schemas marked `presentation_schema.system_notify`, created by the viewer: running ones with
 * their progress, finished ones for a while.
 */

import { TaskOutcome, TaskPhase, type Task, type TaskManagerClient, type TaskSchemaDefinition, type TaskSummary } from 'buckyos'
import { toProgressPercent } from './taskProgress.ts'
import type { ActivityTaskItem, ActivityTaskState } from './model.ts'

const DAY_MS = 24 * 60 * 60 * 1000
/** What the tray lists; Activity passes its own, longer window. */
export const TRAY_TASK_WINDOW: TaskWindow = { lookbackMs: 7 * DAY_MS, finishedTtlMs: 3 * DAY_MS }
export const ACTIVITY_TASK_WINDOW: TaskWindow = { lookbackMs: 30 * DAY_MS, finishedTtlMs: 30 * DAY_MS }
/** `presentation_schema` key of a schema whose tasks are listed (buckyos-api `TASK_PRESENTATION_SYSTEM_NOTIFY`). */
const SYSTEM_NOTIFY_KEY = 'system_notify'
const MAX_PAGES = 3

type SystemNotifyMode = 'progress' | 'completion'

export interface TaskWindow {
  /** Tasks created longer ago are no longer looked at. */
  lookbackMs: number
  /** A finished task goes after this long, dismissed or not. */
  finishedTtlMs: number
}

export interface TaskReader {
  /** Every task to list, running ones included. */
  read(): Promise<ActivityTaskItem[]>
}

export interface TaskReaderOptions {
  client: Pick<TaskManagerClient, 'listTaskSchemas' | 'listTasks' | 'getTask'>
  /** The viewer's user id; an empty one lists the tasks of everyone the session may see. */
  username: () => Promise<string>
  window?: TaskWindow
  now?: () => number
}

function toMillis(value: number | undefined): number {
  if (!value) return 0
  return value < 10_000_000_000 ? value * 1000 : value
}

export function taskState(phase: TaskPhase, outcome: TaskOutcome | undefined): ActivityTaskState {
  switch (phase) {
    case TaskPhase.Terminal:
      return outcome === TaskOutcome.Failed ? 'failed' : outcome === TaskOutcome.Canceled ? 'canceled' : 'completed'
    case TaskPhase.Waiting:
      return 'waiting'
    case TaskPhase.Paused:
      return 'paused'
    default:
      return 'running'
  }
}

function systemNotifyMode(schema: TaskSchemaDefinition): SystemNotifyMode | null {
  const presentation = schema.presentation_schema
  if (!presentation || typeof presentation !== 'object') return null
  const mode = (presentation as Record<string, unknown>)[SYSTEM_NOTIFY_KEY]
  return mode === 'progress' || mode === 'completion' ? mode : null
}

export function createTaskReader({ client, username: readUsername, window = TRAY_TASK_WINDOW, now: clock = Date.now }: TaskReaderOptions): TaskReader {
  /** schema_id → schema_version → mode, read once per reader. */
  let marked: Promise<Map<string, Map<number, SystemNotifyMode>>> | null = null
  let username: Promise<string> | null = null
  const details = new Map<string, { revision: number; task: Task }>()

  const loadMarked = () => {
    marked ??= client.listTaskSchemas({ include_disabled: true }).then((schemas) => {
      const map = new Map<string, Map<number, SystemNotifyMode>>()
      for (const schema of schemas) {
        const mode = systemNotifyMode(schema)
        if (!mode) continue
        const versions = map.get(schema.schema_id) ?? new Map<number, SystemNotifyMode>()
        versions.set(schema.schema_version, mode)
        map.set(schema.schema_id, versions)
      }
      return map
    }).catch((error: unknown) => { marked = null; throw error })
    return marked
  }

  const detailOf = async (summary: TaskSummary): Promise<Task | null> => {
    const cached = details.get(summary.task_id)
    if (cached && cached.revision === summary.revision) return cached.task
    try {
      const task = await client.getTask(summary.task_id)
      details.set(task.task_id, { revision: task.revision, task })
      return task
    } catch {
      return cached?.task ?? null
    }
  }

  return {
    async read() {
      const schemas = await loadMarked()
      username ??= readUsername().catch((error: unknown) => { username = null; throw error })
      const user = await username
      const now = clock()
      const items: ActivityTaskItem[] = []
      const live = new Set<string>()
      for (const [schemaId, versions] of schemas) {
        let cursor: string | undefined
        for (let page = 0; page < MAX_PAGES; page += 1) {
          const result = await client.listTasks({
            schema_id: schemaId,
            ...(user ? { creator_user_id: user } : {}),
            created_after: now - window.lookbackMs,
            limit: 200,
            cursor,
          })
          for (const summary of result.tasks ?? []) {
            const mode = versions.get(summary.schema_version)
            if (!mode) continue
            const terminal = summary.phase === TaskPhase.Terminal
            if (!terminal && mode === 'completion') continue
            const finishedAt = toMillis(summary.completed_at ?? summary.updated_at)
            if (terminal && now - finishedAt > window.finishedTtlMs) continue
            live.add(summary.task_id)
            const detail = terminal ? null : await detailOf(summary)
            items.push({
              taskId: summary.task_id,
              title: summary.name || summary.task_id,
              message: summary.message ?? detail?.message ?? detail?.error?.message,
              state: taskState(summary.phase, summary.outcome),
              progress: terminal ? null : toProgressPercent(detail?.progress),
              timestamp: terminal ? finishedAt : toMillis(summary.created_at),
            })
          }
          cursor = result.next_cursor
          if (!cursor) break
        }
      }
      for (const taskId of [...details.keys()]) if (!live.has(taskId)) details.delete(taskId)
      return items
    },
  }
}
