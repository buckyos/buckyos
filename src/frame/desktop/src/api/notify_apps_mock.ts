/* Mock-runtime notification settings (`?scenario=empty`: no App asked; `error`: the list fails). */

import { waitForMockLatency } from '../runtime.ts'
import type { NotifyApp } from './notify_apps.ts'

const HOUR_MS = 3_600_000

let apps: NotifyApp[] = [
  { app_instance_id: 'notes.example.com@alice', app_id: 'notes.example.com', owner_user_id: 'alice', show_name: 'Notes', installed: true, declared: true, enabled: true, last_sent_at_ms: Date.now() - 2 * HOUR_MS },
  { app_instance_id: 'photos.example.com@alice', app_id: 'photos.example.com', owner_user_id: 'alice', show_name: 'Photos', installed: true, declared: true, enabled: false },
  { app_instance_id: 'legacy.example.com@alice', app_id: 'legacy.example.com', owner_user_id: 'alice', show_name: 'Legacy Reminders', installed: true, declared: false, enabled: false, last_sent_at_ms: Date.now() - 26 * HOUR_MS },
  { app_instance_id: 'weather.example.com@alice', app_id: 'weather.example.com', owner_user_id: 'alice', installed: false, declared: false, enabled: false, last_sent_at_ms: Date.now() - 400 * HOUR_MS },
]

export async function mockNotifyApps(): Promise<NotifyApp[]> {
  await waitForMockLatency()
  const scenario = new URLSearchParams(window.location.search).get('scenario')
  if (scenario === 'error') throw new Error('mock notify.list_apps failure')
  return scenario === 'empty' ? [] : apps.map((app) => ({ ...app }))
}

export async function mockSetNotifyApp(appInstanceId: string, enabled: boolean): Promise<NotifyApp> {
  await waitForMockLatency()
  const app = apps.find((item) => item.app_instance_id === appInstanceId && item.declared)
  if (!app) throw new Error('NOTIFY_NOT_DECLARED')
  const next = { ...app, enabled }
  apps = apps.map((item) => (item === app ? next : item))
  return next
}
