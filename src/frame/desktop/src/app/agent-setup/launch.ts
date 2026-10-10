import { z } from 'zod'
import { desktopUIStore } from '../../models/DesktopUIDataModel'
import { AGENT_SETUP_APP_ID, type AgentSetupSource } from './model'

/** Launch parameters of the Agent setup wizard; with `agent_id` it shows that creation's status. */
export const agentSetupLaunchSchema = z.object({
  source: z.enum(['jarvis_guide', 'users_agents']),
  agent_id: z.string().min(1).optional(),
})

export type AgentSetupLaunch = z.infer<typeof agentSetupLaunchSchema>

export function openAgentSetup(launch: { source: AgentSetupSource; agent_id?: string }) {
  return desktopUIStore.openAppWindow(AGENT_SETUP_APP_ID, {
    launch: { requestId: crypto.randomUUID(), payload: launch },
  })
}
