/**
 * Opens the app a share was handed to (design §3.10). MessageHub needs its
 * own viewer context in the launch payload; HomeStation takes `compose`.
 * Both apps report the outcome back with `reportShareDelivery`.
 */

import type { ShareHandoff } from '../api/shares'
import { desktopUIStore } from '../models/DesktopUIDataModel'

export async function openShareTarget(handoff: ShareHandoff) {
  const requestId = crypto.randomUUID()
  if (handoff.launch.appId === 'messagehub') {
    const { getMessageHubStore } = await import('../app/messagehub/store')
    const hub = getMessageHubStore()
    try {
      await hub.initialize()
    } catch {
      // the view reports load errors itself
    }
    const payload = { kind: 'messagehub', entityId: null, context: hub.defaultContext(), share: handoff.launch.payload.share }
    desktopUIStore.openAppWindow('messagehub', { launch: { requestId, payload } })
    return
  }
  desktopUIStore.openAppWindow('homestation', { launch: { requestId, payload: handoff.launch.payload } })
}
