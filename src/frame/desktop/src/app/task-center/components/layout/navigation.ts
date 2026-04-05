/* ── TaskCenter navigation types ── */

export type TaskCenterPage = 'home' | 'tasks' | 'events'

export interface TaskCenterNav {
  page: TaskCenterPage
  taskId?: string
}
