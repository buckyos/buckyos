/* APP8: from a conversation with one of the owner's Agents (MessageHub's Agent panel) to the Agent's browser. The
 * browser conversation always has the way in; another conversation shows it while the Agent operates the browser. */

import { Globe } from 'lucide-react'
import { useI18n } from '../../../i18n/provider'
import { desktopUIStore } from '../../../models/DesktopUIDataModel'
import type { AgentSessionEntryProps } from '../../messagehub/agentpanel/entries'
import { usePanelResource } from '../../messagehub/agentpanel/hooks'
import { getBrowserBackend, type BrowserAgent } from '../api/backend'
import type { BrowserStatus } from '../api/types'
import { AGENT_BROWSER_APP_ID, openAgentBrowser } from '../launch'

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

/** The Agent's browser as `browser.status` reports it; null when the Agent is not one of the viewer's own. */
async function browserOf(agentDid: string): Promise<{ agent: BrowserAgent; status: BrowserStatus } | null> {
  const agent = await agentOf(agentDid)
  if (!agent) return null
  const backend = await getBrowserBackend()
  return { agent, status: await backend.call<BrowserStatus>(agent, 'browser.status', {}) }
}

const operating = (status: BrowserStatus) => status.groups[0]?.controller === 'agent'

/** `hintTaskId` (the Turn in front) and `signal` (the conversation's messages) say when to look again; while the Agent operates the browser it is read every 15 seconds. */
export function AgentBrowserEntry({ agentDid, agentName, sessionId, hintTaskId, signal }: AgentSessionEntryProps) {
  const { t } = useI18n()
  const available = desktopUIStore.getSnapshot().apps.some((app) => app.id === AGENT_BROWSER_APP_ID)
  const browser = usePanelResource(available ? `browser:${agentDid}` : null, () => browserOf(agentDid), { live: (value) => !!value && operating(value.status), hintTaskId, signal })
  const value = browser.value
  const own = value?.status.default_session_id === sessionId
  if (!value || (!own && !operating(value.status))) return null
  const open = () => { openAgentBrowser({ agent: value.agent.agentId }) }
  return (
    <div className="flex items-center gap-2 rounded-xl bg-[color:color-mix(in_srgb,var(--cp-accent)_10%,transparent)] px-3 py-2 text-[13px]" role="status" data-testid="agent-browser-entry" data-state={own ? 'session' : 'working'}>
      <Globe size={15} className="shrink-0 text-[color:var(--cp-accent)]" aria-hidden />
      <span className="min-w-0 flex-1">{own ? t('agentBrowser.entry.session', 'This is the conversation of {{name}}’s browser', { name: agentName }) : t('agentBrowser.entry.working', '{{name}} is using the browser', { name: agentName })}</span>
      <button type="button" className="mh-turn-action shrink-0" onClick={open} data-testid="agent-browser-entry-open">{own ? t('agentBrowser.entry.open', 'Open browser') : t('agentBrowser.entry.view', 'View')}</button>
    </div>
  )
}
