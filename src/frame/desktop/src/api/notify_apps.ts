import { buckyos, type NotifyApp } from 'buckyos'
import { isMockRuntime } from '../runtime.ts'

export type { NotifyApp }

/** The Apps the signed-in user may let notify them or not. */
export async function fetchNotifyApps(): Promise<NotifyApp[]> {
  if (isMockRuntime()) return (await import('./notify_apps_mock.ts')).mockNotifyApps()
  return buckyos.getMsgCenterClient().notifyListApps()
}

export async function saveNotifyApp(appInstanceId: string, enabled: boolean): Promise<NotifyApp> {
  if (isMockRuntime()) return (await import('./notify_apps_mock.ts')).mockSetNotifyApp(appInstanceId, enabled)
  return buckyos.getMsgCenterClient().notifySetApp(appInstanceId, enabled)
}
