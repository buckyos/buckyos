import { z } from 'zod'
import { desktopUIStore } from '../../models/DesktopUIDataModel'

/** Launch parameters of Users and Agents: open one entity's details, or the signed-in user's own page. */
export const usersAgentsLaunchSchema = z.union([
  z.object({ entityId: z.string().min(1) }),
  z.object({ view: z.literal('self') }),
])

export type UsersAgentsLaunch = z.infer<typeof usersAgentsLaunchSchema>

export function openUsersAgents(launch: UsersAgentsLaunch) {
  return desktopUIStore.openAppWindow('users-agents', {
    launch: { requestId: crypto.randomUUID(), payload: launch },
  })
}
