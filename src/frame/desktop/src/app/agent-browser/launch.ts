import { z } from 'zod'
import { desktopUIStore } from '../../models/DesktopUIDataModel'

export const AGENT_BROWSER_APP_ID = 'agent-browser'

export const browserModes = ['home', 'page', 'chat'] as const
export type BrowserMode = (typeof browserModes)[number]

/** The parameters of a launch, in the order a link spells them after the Agent. */
export const agentBrowserLaunchParams = ['mode', 'route', 'url', 'upgrade'] as const

/**
 * Launch parameters of the Agent browser: whose browser (the Agent's AgentId or DID) and where it opens. Without an
 * Agent it opens the last one used, or asks when the user has several. `mode` is `home` when left out. `route` is a
 * page of the personal site (its query string: `s=<source>`, `a=<article>`, `e=<event>`). `url` is an address: in
 * `page` mode the browser opens it, in `home` the site's page for it shows. `upgrade` is the id of an upgrade request
 * of the site, shown with the source it is about.
 */
export const agentBrowserLaunchSchema = z.object({
  agent: z.string().min(1).optional(),
  mode: z.enum(browserModes).optional(),
  route: z.string().max(256).optional(),
  url: z.string().min(1).max(4096).optional(),
  upgrade: z.string().min(1).max(64).optional(),
})

export type AgentBrowserLaunch = z.infer<typeof agentBrowserLaunchSchema>

export function openAgentBrowser(launch: AgentBrowserLaunch = {}) {
  return desktopUIStore.openAppWindow(AGENT_BROWSER_APP_ID, {
    launch: { requestId: crypto.randomUUID(), payload: launch },
  })
}

/**
 * Opens the App for a Desktop link to it (`/?agent_browser=…`, as a notification carries one) from where the link is
 * pressed, without loading the Desktop for it: a window of the Desktop, or the App's own page when the link is on
 * another App's page (a browser tab, the BuckyOS client). False when the link is not one of the App's.
 */
export function openAgentBrowserLink(link: string): boolean {
  let url: URL
  try {
    url = new URL(link, window.location.origin)
  } catch {
    return false
  }
  const launch = url.origin === window.location.origin && url.pathname === '/' ? agentBrowserLaunchFromParams(url.searchParams) : null
  if (!launch) return false
  openAgentBrowser(launch)
  return true
}

/** The launch of the Desktop link `?agent_browser=<agent>[&mode=<mode>][&route=…][&url=…][&upgrade=<id>]`, or null without one. */
export function agentBrowserLaunchFromParams(params: URLSearchParams): AgentBrowserLaunch | null {
  if (!params.has('agent_browser')) return null
  const parsed = agentBrowserLaunchSchema.safeParse({
    ...(params.get('agent_browser') ? { agent: params.get('agent_browser') } : {}),
    ...Object.fromEntries(agentBrowserLaunchParams.flatMap((name) => (params.get(name) ? [[name, params.get(name)]] : []))),
  })
  return parsed.success ? parsed.data : {}
}
