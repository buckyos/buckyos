/* ── Desktop Jarvis guide entry (PRD §9) ──
 *
 * The `agent-guide` desktop item stays a guide until the signed-in user's own
 * Agent created from it (settings.desktop_entry == "jarvis_guide") is ready;
 * then it shows that Agent's name and avatar and opens its home. State comes
 * from `agent.list` and is refreshed on focus, after the wizard closes, when
 * an Agent changes, and every 30 seconds.
 */

import { fetchCurrentAccount, isLimitedUserType } from '../../api/account'
import { fetchAgentList, type AgentEntry } from '../../api/user_mgr'
import { desktopUIStore } from '../../models/DesktopUIDataModel'
import { isMockRuntime } from '../../runtime'
import { openUsersAgents } from '../users-agents/launch'
import { onAgentsChanged } from './events'
import { openAgentSetup } from './launch'
import {
  AGENT_GUIDE_APP_ID,
  AGENT_SETUP_APP_ID,
  agentDisplayName,
  agentHomeTarget,
  computeAgentGuideState,
  usableImageUrl,
  type AgentGuideState,
} from './model'

const REFRESH_INTERVAL_MS = 30_000

/** Opens an Agent's home: its constructed App's Web entry, else its Users and Agents details. */
export async function openAgentHome(entry: AgentEntry) {
  const target = agentHomeTarget(entry)
  if (target.kind === 'app') {
    const known = () => desktopUIStore.getSnapshot().apps.some((app) => app.id === target.appId)
    if (!known() && !isMockRuntime()) await desktopUIStore.refreshApps().catch(() => undefined)
    if (known() && desktopUIStore.openAppWindow(target.appId)) return
  }
  openUsersAgents({ entityId: entry.agent_id })
}

class AgentGuideController {
  private state: AgentGuideState | null = null
  private started = false
  private pending: Promise<AgentGuideState | null> | null = null

  start() {
    if (this.started) return
    this.started = true
    let setupOpen = this.setupWindowOpen()
    desktopUIStore.subscribe(() => {
      const open = this.setupWindowOpen()
      if (setupOpen && !open) void this.refresh()
      setupOpen = open
    })
    window.addEventListener('focus', () => void this.refresh())
    document.addEventListener('visibilitychange', () => {
      if (document.visibilityState === 'visible') void this.refresh()
    })
    onAgentsChanged(() => void this.refresh())
    window.setInterval(() => void this.refresh(), REFRESH_INTERVAL_MS)
    void this.refresh()
  }

  private setupWindowOpen() {
    return desktopUIStore.getSnapshot().runtime.windows.some((window) => window.appId === AGENT_SETUP_APP_ID)
  }

  private guideShown() {
    return desktopUIStore.getSnapshot().apps.some((app) => app.id === AGENT_GUIDE_APP_ID)
  }

  refresh(): Promise<AgentGuideState | null> {
    this.pending ??= this.load().finally(() => { this.pending = null })
    return this.pending
  }

  private async load(): Promise<AgentGuideState | null> {
    if (!this.guideShown()) return this.state
    const account = await fetchCurrentAccount().catch(() => null)
    if (!account || isLimitedUserType(account.user_type)) return this.state
    const { data } = await fetchAgentList()
    // A failed read keeps the last known entry rather than flipping it back to the guide.
    if (!data) return this.state
    const state = computeAgentGuideState(data.agents, account.user_id)
    this.state = state
    desktopUIStore.setAgentGuidePresentation(
      state.kind === 'linked'
        ? { label: agentDisplayName(state.agent), iconUrl: usableImageUrl(state.agent.profile?.avatar) }
        : { badge: state.kind === 'creating' ? 'busy' : state.kind === 'failed' ? 'error' : undefined },
    )
    return state
  }

  async open() {
    const state = (await this.refresh().catch(() => null)) ?? this.state ?? { kind: 'none' as const }
    switch (state.kind) {
      case 'none':
        openAgentSetup({ source: 'jarvis_guide' })
        return
      case 'creating':
      case 'failed':
        openAgentSetup({ source: 'jarvis_guide', agent_id: state.agent.agent_id })
        return
      case 'linked':
        await openAgentHome(state.agent)
    }
  }
}

export const agentGuide = new AgentGuideController()
