import { useState } from 'react'
import { Settings2 } from 'lucide-react'
import { useI18n } from '../../i18n/provider'
import { fetchAgentList } from '../../api/user_mgr'
import { desktopUIStore } from '../../models/DesktopUIDataModel'
import { openUsersAgents } from '../users-agents/launch'

/** Opens the Agent's details in Users and Agents, where "Allow this Agent to join group chats" lives. */
export function AgentGroupSettingsLink({ agentDid, name }: { agentDid: string; name: string }) {
  const { t } = useI18n()
  const [pending, setPending] = useState(false)
  // Only the desktop hosts Users and Agents; the standalone MessageHub page keeps the hint text alone.
  if (!desktopUIStore.getSnapshot().apps.some((app) => app.id === 'users-agents')) return null
  const open = async () => {
    setPending(true)
    const { data } = await fetchAgentList()
    setPending(false)
    const agent = data?.agents.find((entry) => entry.agent_did === agentDid)
    if (agent) openUsersAgents({ entityId: agent.agent_id })
    else desktopUIStore.openAppWindow('users-agents')
  }
  return (
    <button
      type="button"
      data-testid="agent-group-settings-link"
      className="inline-flex min-h-9 items-center gap-1.5 rounded-lg px-1 text-[13px] font-medium text-[color:var(--cp-accent)] disabled:opacity-50"
      disabled={pending}
      onClick={() => void open()}
    >
      <Settings2 size={14} aria-hidden />
      {t('messagehub.group.openAgentSettings', undefined, { name })}
    </button>
  )
}
