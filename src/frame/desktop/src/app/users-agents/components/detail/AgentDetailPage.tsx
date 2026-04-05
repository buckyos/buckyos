/* ── Agent detail page ── */

import { Chip } from '@mui/material'
import { useAgent } from '../../hooks/use-users-agents-store'
import { HeaderSection } from '../sections/HeaderSection'
import { BindingsSection } from '../sections/BindingsSection'
import { InfoFieldsSection } from '../sections/InfoFieldsSection'
import { DIDDocumentSection } from '../sections/DIDDocumentSection'
import { RuntimeInfoSection } from '../sections/RuntimeInfoSection'

export function AgentDetailPage() {
  const agent = useAgent()

  return (
    <div className="space-y-4">
      <HeaderSection
        name={agent.displayName}
        kind="agent"
        avatarUrl={agent.avatarUrl}
        did={agent.did}
        subtitle={`${agent.agentType} · v${agent.version}`}
        badges={
          <>
            {agent.capabilities.map((cap) => (
              <Chip key={cap} label={cap} size="small" variant="outlined" />
            ))}
          </>
        }
      />

      <BindingsSection bindings={agent.bindings} />

      <InfoFieldsSection title="Agent Info" fields={agent.info} />

      <RuntimeInfoSection runtime={agent.runtime} status={agent.status} />

      <DIDDocumentSection document={agent.didDocument} />
    </div>
  )
}
