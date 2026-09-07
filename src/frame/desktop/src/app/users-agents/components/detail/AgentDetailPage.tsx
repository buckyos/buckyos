import { useLocation, useNavigate } from 'react-router-dom'
import { useI18n } from '../../../../i18n/provider'
import { desktopUIStore } from '../../../../models/DesktopUIDataModel'
import { getMessageHubStore } from '../../../messagehub/store'
/* ── Agent detail page ── */

import { Chip } from '@mui/material'
import { HeaderSection } from '../sections/HeaderSection'
import { SocialAccountsSection } from '../sections/SocialAccountsSection'
import { InfoFieldsSection } from '../sections/InfoFieldsSection'
import { DIDDocumentSection } from '../sections/DIDDocumentSection'
import { RuntimeInfoSection } from '../sections/RuntimeInfoSection'
import type { AgentEntity } from '../../datamodel/types'

export function AgentDetailPage({ agent }: { agent: AgentEntity }) {
  const { t } = useI18n()
  const location = useLocation(), navigate = useNavigate()
  const openMessages = async (observe: boolean) => {
    const store = getMessageHubStore()
    // The viewer is always the logged-in user; the store resolves it once.
    try { await store.initialize() } catch { /* the view reports load errors itself */ }
    const defaultContext = store.defaultContext()
    const ownerDid = agent.did ?? agent.id
    const context = observe ? { ...defaultContext, ownerDid, mode: 'observe' as const } : defaultContext
    const entityId = observe ? (store.isMock ? 'did:buckyos:person:alice' : null) : ownerDid
    const payload = { kind: 'messagehub', entityId, context }
    if (location.pathname === '/desktop' || location.pathname === '/') desktopUIStore.openAppWindow('messagehub', { launch: { requestId: crypto.randomUUID(), payload } })
    else navigate(`/messagehub?${new URLSearchParams({ ...(entityId ? { entityId } : {}), ownerDid: context.ownerDid, mode: context.mode })}`)
  }
  return (
    <div className="space-y-4">
      <HeaderSection
        name={agent.displayName}
        kind="agent"
        avatarUrl={agent.avatarUrl}
        did={agent.did}
        subtitle={`${agent.agentType} · Owner ${agent.settings.owner} · v${agent.version}`}
        previewUrl={`/profile/${encodeURIComponent(agent.did ?? agent.id)}`}
        badges={
          <>
            {agent.capabilities.map((cap) => (
              <Chip key={cap} label={cap} size="small" variant="outlined" />
            ))}
          </>
        }
      />

      <div className="flex flex-wrap gap-2">
        <button type="button" className="min-h-11 rounded-lg border border-[color:var(--cp-border)] px-4 text-sm" onClick={() => void openMessages(false)}>{t('messagehub.chatWithAgent')}</button>
        <button type="button" className="min-h-11 rounded-lg border border-[color:var(--cp-border)] px-4 text-sm" onClick={() => void openMessages(true)}>{t('messagehub.viewAgentSessions')}</button>
      </div>
      <RuntimeInfoSection runtime={agent.runtime} status={agent.status} />

      <InfoFieldsSection title="Profile" fields={agent.info} />

      <SocialAccountsSection entityId={agent.id} accounts={agent.socialAccounts} />

      <InfoFieldsSection title="Settings" fields={agent.settings} />

      <InfoFieldsSection
        title="Security & Account"
        editable={false}
        fields={{
          owner: agent.settings.owner,
          credential: 'Service key managed by owner',
          permissions: agent.settings.permissions,
        }}
      />

      <DIDDocumentSection document={agent.didDocument} />
    </div>
  )
}
