import { z } from 'zod'
import { desktopUIStore } from '../../models/DesktopUIDataModel'

/** Launch parameters of My Network: the Requests view, one person's (or group's) details, or one of the two entry dialogs. */
export const myNetworkLaunchSchema = z.union([
  z.object({ view: z.literal('requests') }),
  z.object({ entity: z.string().startsWith('did:') }),
  z.object({ action: z.literal('add-friend'), input: z.string().max(2048).optional() }),
  z.object({ action: z.literal('join-group'), input: z.string().max(2048).optional() }),
])

export type MyNetworkLaunch = z.infer<typeof myNetworkLaunchSchema>

export function openMyNetwork(launch: MyNetworkLaunch) {
  return desktopUIStore.openAppWindow('my-network', {
    launch: { requestId: crypto.randomUUID(), payload: launch },
  })
}
