import { z } from 'zod'
import { desktopUIStore } from '../../models/DesktopUIDataModel'
import { isSettingsPage, type SettingsPage } from './components/layout/navigation'

/** Launch parameters of Settings: the page it opens on. */
export const settingsLaunchSchema = z.object({ page: z.custom<SettingsPage>(isSettingsPage) })

export function openSettings(page: SettingsPage) {
  return desktopUIStore.openAppWindow('settings', {
    launch: { requestId: crypto.randomUUID(), payload: { page } },
  })
}
