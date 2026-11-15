/* APP8: from a conversation with one of the owner's Agents (MessageHub's Agent panel) to what the Agent does in its
 * browser for that conversation. Shown while the conversation's group has tabs or the Agent is at work in it. */

import { Globe } from 'lucide-react'
import { useI18n } from '../../../i18n/provider'
import { desktopUIStore } from '../../../models/DesktopUIDataModel'
import { usePanelResource } from '../../messagehub/agentpanel/hooks'
import { getBrowserBackend, type BrowserAgent } from '../api/backend'
import { DEFAULT_GROUP, type BrowserGroup, type BrowserStatus } from '../api/types'
import { AGENT_BROWSER_APP_ID, openAgentBrowser } from '../launch'
import { groupIdOfSession } from '../store/things'

const agents = new Map<string, Promise<BrowserAgent | null>>()

function agentOf(agentDid: string): Promise<BrowserAgent | null> {
  let agent = agents.get(agentDid)
  if (!agent) {
    agent = getBrowserBackend().then((backend) => backend.agents()).then((list) => list.find((item) => item.agentDid === agentDid) ?? null)
    agents.set(agentDid, agent)
    agent.then((found) => { if (!found) agents.delete(agentDid) }, () => agents.delete(agentDid))
  }
  return agent
}

/** The group of a MessageHub session (protocol §1) as `browser.status` reports it; null when the Agent is not one of the viewer's own. */
async function groupOfSession(agentDid: string, sessionId: string): Promise<{ agent: BrowserAgent; group: BrowserGroup } | null> {
  const agent = await agentOf(agentDid)
  if (!agent) return null
  const backend = await getBrowserBackend()
  let groupId = groupIdOfSession({ id: sessionId }, agentDid, null)
  let status = await backend.call<BrowserStatus>(agent, 'browser.status', { group_id: groupId })
  if (groupId !== DEFAULT_GROUP && status.default_session_id === sessionId) {
    groupId = DEFAULT_GROUP
    status = await backend.call<BrowserStatus>(agent, 'browser.status', { group_id: groupId })
  }
  const group = status.groups.find((item) => item.group_id === groupId)
  return group ? { agent, group } : null
}

const atWork = (group: BrowserGroup) => group.controller === 'agent' || group.agent_state !== 'idle'

/** `hintTaskId` (the Turn in front) and `signal` (the conversation's messages) say when to look again; while the Agent is at work the group is read every 15 seconds. */
export function AgentBrowserEntry({ agentDid, agentName, sessionId, hintTaskId, signal }: { agentDid: string; agentName: string; sessionId: string; hintTaskId?: string; signal?: unknown }) {
  const { t } = useI18n()
  const available = desktopUIStore.getSnapshot().apps.some((app) => app.id === AGENT_BROWSER_APP_ID)
  const browser = usePanelResource(available ? `browser:${agentDid}:${sessionId}` : null, () => groupOfSession(agentDid, sessionId), { live: (value) => !!value && atWork(value.group), hintTaskId, signal })
  const value = browser.value
  if (!value || (value.group.tabs.length === 0 && !atWork(value.group))) return null
  const { agent, group } = value
  const state = group.agent_state === 'waiting_takeover' ? 'waiting' : group.controller === 'agent' || group.agent_state === 'working' ? 'working' : 'tabs'
  const text = state === 'waiting' ? t('agentBrowser.entry.waiting', '{{name}} is waiting for you in the browser', { name: agentName })
    : state === 'working' ? t('agentBrowser.entry.working', '{{name}} is using the browser', { name: agentName })
    : t('agentBrowser.entry.tabs', 'This conversation has pages open in the browser')
  return (
    <div className="flex items-center gap-2 rounded-xl bg-[color:color-mix(in_srgb,var(--cp-accent)_10%,transparent)] px-3 py-2 text-[13px]" role="status" data-testid="agent-browser-entry" data-state={state} data-group={group.group_id}>
      <Globe size={15} className="shrink-0 text-[color:var(--cp-accent)]" aria-hidden />
      <span className="min-w-0 flex-1">{text}</span>
      <button type="button" className="mh-turn-action shrink-0" onClick={() => { openAgentBrowser({ agent: agent.agentId, group: sessionId }) }} data-testid="agent-browser-entry-open">{t('agentBrowser.entry.view', 'View')}</button>
    </div>
  )
}
