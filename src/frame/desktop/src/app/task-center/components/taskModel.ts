/* ── TaskCenter: tones, labels, time formats and tree helpers shared by every view ── */

import { useI18n } from '../../../i18n/provider'
import { TaskPhase, type Task, type TaskSource, type TaskType } from '../../../api/task_mgr'

export type Tone = 'running' | 'waiting' | 'done' | 'failed' | 'pending' | 'idle'

export function taskTone(task: Pick<Task, 'status'>): Tone {
  switch (task.status) {
    case 'running':
      return 'running'
    case 'paused':
      return 'waiting'
    case 'completed':
      return 'done'
    case 'failed':
      return 'failed'
    case 'pending':
      return 'pending'
    default:
      return 'idle'
  }
}

/** Progress worth showing in a row: a live task that reports it. */
export function liveProgress(task: Task): number | null {
  return task.progress != null && (task.status === 'running' || task.status === 'paused') ? task.progress : null
}

function sameDay(a: Date, b: Date) {
  return a.getFullYear() === b.getFullYear() && a.getMonth() === b.getMonth() && a.getDate() === b.getDate()
}

function toDate(value: string | number | null | undefined): Date | null {
  if (value == null || value === '') return null
  const date = typeof value === 'number' ? new Date(value < 10_000_000_000 ? value * 1000 : value) : new Date(value)
  return Number.isNaN(date.getTime()) ? null : date
}

export function useTaskText() {
  const { t, locale } = useI18n()
  const year = new Date().getFullYear()
  const status = (task: Pick<Task, 'status' | 'phase'>) =>
    task.status === 'paused' && task.phase === TaskPhase.Waiting
      ? t('taskCenter.status.waiting', 'Waiting')
      : t(`taskCenter.status.${task.status}`, task.status)
  const type = (value: TaskType) => t(`taskCenter.type.${value}`, value)
  const source = (value: TaskSource) => t(`taskCenter.source.${value}`, value)
  return {
    t,
    locale,
    status,
    type,
    source,
    /** Second line of a row: the state, then what the task says about itself. */
    meta: (task: Task) => `${status(task)} · ${task.summary || `${type(task.type)} · ${source(task.source)}`}`,
    /** Rows: a time today, a date otherwise. */
    listTime(value: string) {
      const date = toDate(value)
      if (!date) return ''
      if (sameDay(date, new Date())) return date.toLocaleTimeString(locale, { hour: '2-digit', minute: '2-digit' })
      return date.toLocaleDateString(locale, date.getFullYear() === year
        ? { month: 'short', day: 'numeric' }
        : { year: 'numeric', month: 'short', day: 'numeric' })
    },
    /** Schedules: date and time, the year only when it is not this one. */
    dateTime(value: string | number | null | undefined) {
      const date = toDate(value)
      if (!date) return null
      return date.toLocaleString(locale, {
        ...(date.getFullYear() === year ? {} : { year: 'numeric' }),
        month: 'short',
        day: 'numeric',
        hour: '2-digit',
        minute: '2-digit',
      })
    },
    timeOfDay(value: string) {
      const date = toDate(value)
      return date ? date.toLocaleTimeString(locale, { hour: '2-digit', minute: '2-digit' }) : ''
    },
    fullTime(value: string | null) {
      const date = toDate(value)
      if (!date) return '—'
      return date.toLocaleString(locale, {
        year: 'numeric',
        month: 'short',
        day: 'numeric',
        hour: '2-digit',
        minute: '2-digit',
        second: '2-digit',
      })
    },
    day(value: string) {
      const date = toDate(value)
      return date ? date.toLocaleDateString(locale, { year: 'numeric', month: 'long', day: 'numeric' }) : ''
    },
  }
}

export type TaskText = ReturnType<typeof useTaskText>

/** Keeps the tasks that match and the ancestors leading to them; those ancestors open. */
export function pruneTree(tasks: Task[], match: (task: Task) => boolean): { roots: Task[]; open: Set<string> } {
  const open = new Set<string>()
  const visit = (task: Task): Task | null => {
    if (match(task)) return task
    const children = task.children.map(visit).filter((child): child is Task => child !== null)
    if (children.length === 0) return null
    open.add(task.taskId)
    return { ...task, children }
  }
  return { roots: tasks.map(visit).filter((task): task is Task => task !== null), open }
}

/** Ids of the tasks above `taskId`, root first. */
export function ancestorsOf(tasks: Task[], taskId: string | undefined): string[] {
  if (!taskId) return []
  const walk = (list: Task[], trail: string[]): string[] | null => {
    for (const task of list) {
      if (task.taskId === taskId) return trail
      const found = walk(task.children, [...trail, task.taskId])
      if (found) return found
    }
    return null
  }
  return walk(tasks, []) ?? []
}
