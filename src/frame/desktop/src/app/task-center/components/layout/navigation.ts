/* ── TaskCenter navigation types ── */

export type TaskCenterPage = 'home' | 'tasks' | 'schedules' | 'events'

export interface TaskCenterNav {
  page: TaskCenterPage
  taskId?: string
}
