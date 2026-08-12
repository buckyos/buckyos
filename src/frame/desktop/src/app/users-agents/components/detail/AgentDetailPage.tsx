/* ── Agent detail page ── */

import { Chip } from '@mui/material'
import { HeaderSection } from '../sections/HeaderSection'
import { SocialAccountsSection } from '../sections/SocialAccountsSection'
import { InfoFieldsSection } from '../sections/InfoFieldsSection'
import { DIDDocumentSection } from '../sections/DIDDocumentSection'
import { RuntimeInfoSection } from '../sections/RuntimeInfoSection'
import type { AgentEntity } from '../../datamodel/types'

export function AgentDetailPage({ agent }: { agent: AgentEntity }) {
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
