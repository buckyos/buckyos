import { z } from 'zod'
import { desktopUIStore } from '../../models/DesktopUIDataModel'

export const AGENT_BROWSER_APP_ID = 'agent-browser'

export const browserModes = ['page', 'reflow', 'chat'] as const
export type BrowserMode = (typeof browserModes)[number]

/**
 * Launch parameters of the Agent browser: whose browser (the Agent's AgentId or DID) and the mode it opens in.
 * Without an Agent it opens the last one used, or asks when the user has several.
 */
export const agentBrowserLaunchSchema = z.object({
  agent: z.string().min(1).optional(),
  mode: z.enum(browserModes).optional(),
})

export type AgentBrowserLaunch = z.infer<typeof agentBrowserLaunchSchema>

export function openAgentBrowser(launch: AgentBrowserLaunch = {}) {
  return desktopUIStore.openAppWindow(AGENT_BROWSER_APP_ID, {
    launch: { requestId: crypto.randomUUID(), payload: launch },
  })
}

/** The launch of the Desktop link `?agent_browser=<agent>[&mode=<mode>]`, or null without one. */
export function agentBrowserLaunchFromParams(params: URLSearchParams): AgentBrowserLaunch | null {
  if (!params.has('agent_browser')) return null
  const parsed = agentBrowserLaunchSchema.safeParse({
    ...(params.get('agent_browser') ? { agent: params.get('agent_browser') } : {}),
    ...(params.get('mode') ? { mode: params.get('mode') } : {}),
  })
  return parsed.success ? parsed.data : {}
}
