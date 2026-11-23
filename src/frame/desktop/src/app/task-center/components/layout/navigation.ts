/* ── TaskCenter navigation ── */

import { Activity, CalendarClock, Home, ListTodo } from 'lucide-react'

export type TaskCenterPage = 'home' | 'tasks' | 'schedules' | 'events'

export interface TaskCenterNav {
  page: TaskCenterPage
  /** The task whose detail is open: a side panel on a desktop, its own page on a phone. */
  taskId?: string
}

export const NAV_LABELS: Record<TaskCenterPage, { key: string; fallback: string }> = {
  home: { key: 'taskCenter.nav.home', fallback: 'Home' },
  tasks: { key: 'taskCenter.nav.tasks', fallback: 'Tasks' },
  schedules: { key: 'taskCenter.nav.schedules', fallback: 'Scheduled tasks' },
  events: { key: 'taskCenter.nav.events', fallback: 'System events' },
}

export const NAV_ITEMS: { key: TaskCenterPage; icon: typeof Home }[] = [
  { key: 'home', icon: Home },
  { key: 'tasks', icon: ListTodo },
  { key: 'schedules', icon: CalendarClock },
  { key: 'events', icon: Activity },
]
